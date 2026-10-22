"""Exercise real journal/effect admission, replay and exact-action lookup."""
import argparse
import importlib.util
import json
from pathlib import Path
import socket
import sqlite3
import tempfile
import sys

ROOT = Path(__file__).resolve().parent
_spec = importlib.util.spec_from_file_location("response_verify", ROOT.parent / "privacy-response/verify.py")
_base = importlib.util.module_from_spec(_spec)
# The source verifier imports its adjacent prepare module. Retain this module's
# own preparation explicitly rather than depending on sys.path module order.
sys.path.insert(0, str(ROOT.parent / "privacy-response"))
_spec.loader.exec_module(_base)
_spec2 = importlib.util.spec_from_file_location("lifecycle_prepare", ROOT / "prepare.py")
_local = importlib.util.module_from_spec(_spec2)
_spec2.loader.exec_module(_local)


def run(host, mode, composed_inspection=False):
    if composed_inspection:
        _base.COMPLETION_SECONDS = 1500
    with tempfile.TemporaryDirectory(prefix="mas-privacy-lifecycle-") as temporary:
        project = _local.prepare(Path(temporary) / "application", composed_inspection=composed_inspection)
        _base.checked(host, "build", "--project", project)
        digest = _base.checked(host, "describe", "--bundle", project / ".mas/build/bundle.json")["bundle_digest"]
        with socket.socket() as listener:
            listener.bind(("127.0.0.1", 0))
            port = listener.getsockname()[1]
        gateway = json.loads((project / "gateway.json").read_text())
        # N3 conformance is independently bounded from per-component model
        # launches. N2 inspector behaviour has its own actual-host regression.
        if not composed_inspection:
            gateway.pop("privacy_inspector_file", None)
        gateway.update(listen=f"127.0.0.1:{port}", public_url=f"http://127.0.0.1:{port}/a2a")
        (project / "gateway.json").write_text(json.dumps(gateway))
        request = {"message": {"messageId": "synthetic-queued-event", "role": "ROLE_USER", "parts": [
            {"data": {"agent": "responder", "descriptorDigest": digest, "input": {"mode": mode, "discarded_private": _base.PRIVATE.decode()}}, "mediaType": "application/json"}]},
            "configuration": {"returnImmediately": True}}
        service = _base.Service(host, project, port)
        try:
            response = service.rpc("SendMessage", request)
            assert "error" not in response, response
            identity = response["result"]["task"]["id"]
            task = service.terminal(identity, "completed")
            output = task["artifacts"][0]["parts"][0]["data"]
            assert output["submission"] == ("uncertain" if mode == "reconcile-unknown" else "accepted"), task
            attempts = 2 if mode in ("retry-once", "reconcile-absent", "escalate") else 1
            assert _base.count(project, "notification-calls") == attempts
            assert _base.count(project, "reconciliation-calls") == (0 if mode in ("retry-once","acknowledged","escalate") else 1)
            with sqlite3.connect(_base.journal(project)) as db:
                effects = [(json.loads(body)["format"], state) for body, state in db.execute("SELECT input,state FROM steps WHERE kind='effect' ORDER BY rowid")]
            assert all(state == "completed" for _, state in effects), effects
            assert [name for name, _ in effects].count("mas-privacy-queue-intent/1") == (2 if mode == "escalate" else 1)
            assert [name for name, _ in effects].count("mas-privacy-queue-submit/1") == attempts
            for path in (project / "state").rglob("*"):
                if path.is_file():
                    assert _base.PRIVATE not in path.read_bytes()
            if mode in ("acknowledged","escalate"):
                assert output["acknowledgement"] == ("acknowledged" if mode == "acknowledged" else "pending")
                assert output["escalation"] == ("not-required" if mode == "acknowledged" else "accepted")
            if composed_inspection:
                assessment = service.rpc("GetRetainedAssessment", {"id":identity,"key":"privacy-queue-intent"},
                                         extensions=_base.TASK_EXTENSION+",urn:mas:host-governed-assessment-review:1")
                assert "error" not in assessment, assessment
                selected = assessment["result"]
                assert selected["inspection"]["contract"]["version"] == "mas-governed-inspection/3"
                assert selected["component_count"] > 0
                review = {"id":identity,"key":"privacy-queue-intent","review":{
                    "review_key":"composed-review-1","assessment_binding":selected["inspection"]["component_assessments"][0]["binding"],
                    "verdict":"inconclusive","reason":"insufficient-evidence"}}
                extensions = _base.TASK_EXTENSION+",urn:mas:host-governed-assessment-review:1"
                recorded = service.rpc("AdjudicateAssessment",review,extensions=extensions)
                assert "error" not in recorded, recorded
                assert recorded["result"]["record"]["release_permission"] == "requires-fresh-policy-decision"
                assert service.rpc("AdjudicateAssessment",review,extensions=extensions) == recorded
                changed_review = json.loads(json.dumps(review))
                changed_review["review"].update(verdict="false-positive",reason="domain-context")
                assert "error" in service.rpc("AdjudicateAssessment",changed_review,extensions=extensions)
                for path in (project / "state").rglob("*"):
                    if path.is_file():
                        assert _base.PRIVATE not in path.read_bytes()
            notification = (project / "notification.json").read_bytes()
            if mode == "escalate":
                assert json.loads(notification)["template"] == "privacy-handling-escalation/1"
        finally:
            service.close()
        service = _base.Service(host, project, port)
        try:
            replay = service.rpc("SendMessage", request)
            assert replay["result"]["task"]["artifacts"] == task["artifacts"], replay
            assert _base.count(project, "notification-calls") == attempts
            assert (project / "notification.json").read_bytes() == notification
        finally:
            service.close()


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--host", required=True)
    parser.add_argument("--scenario", choices=("retry-once", "reconcile-accepted", "reconcile-absent", "reconcile-unknown", "acknowledged", "escalate"), required=True)
    parser.add_argument("--with-inspector", action="store_true")
    args = parser.parse_args()
    run(str(Path(args.host).resolve()), args.scenario, args.with_inspector)
    print(json.dumps({"scenario": args.scenario, "status": "PASS"}))
