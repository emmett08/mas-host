"""Actual governed-host acceptance for the registered acquisition client.

Uses the existing synthetic governed acquisition fixture without its neural
inspector. No external provider is contacted. Requires status and publication.
"""

from __future__ import annotations

import argparse
import copy
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import time

from coalescing import Acquisition, HTTPRPC, RegisteredRead, Reply, Subscription


REPO = Path(__file__).resolve().parents[2]
TOKEN = "synthetic-acquisition-local-token-0123456789"


def digest_file(path):
    value = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            value.update(block)
    return value.hexdigest()


def write(path, value):
    temporary = path.with_name(path.name + ".next")
    temporary.write_text(json.dumps(value), encoding="utf-8")
    temporary.replace(path)


def checked(host, *args):
    result = subprocess.run([str(host), *map(str, args)], capture_output=True, timeout=180)
    if result.returncode:
        raise AssertionError("fixture-host-command-failed")
    return json.loads(result.stdout)


class Service:
    def __init__(self, host, project, port):
        self.process = subprocess.Popen([str(host), "serve-a2a", "--bundle",
            str(project / ".mas/build/bundle.json"), "--state", str(project / "state"),
            "--config", str(project / "gateway.json")], stdout=subprocess.DEVNULL,
            stderr=subprocess.PIPE)
        deadline = time.monotonic() + 30
        try:
            while time.monotonic() < deadline:
                if self.process.poll() is not None:
                    raise AssertionError("fixture-service-exited")
                try:
                    with socket.create_connection(("127.0.0.1", port), timeout=.1):
                        return
                except OSError:
                    time.sleep(.02)
            raise AssertionError("fixture-service-startup-timeout")
        except BaseException:
            self.close()
            raise

    def close(self):
        self.process.terminate()
        try:
            self.process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait(timeout=5)
        # No raw diagnostics or provider/worker bytes are emitted by this test.
        self.process.stderr.close()


class LoseSubmissionAcknowledgement:
    """Drop an actual successful reply after the real host accepted the task."""
    def __init__(self, rpc):
        self.authority, self.rpc = rpc.authority, rpc
        self.lost = False
        self.submissions = []

    def call(self, method, params, *args, **kwargs):
        reply = self.rpc.call(method, params, *args, **kwargs)
        if method == "SendMessage":
            self.submissions.append(copy.deepcopy(params))
            if reply.state == "ok" and not self.lost:
                self.lost = True
                return Reply("transport-unknown")
        return reply


def prepare(project, fixture_python):
    spec = importlib.util.spec_from_file_location("acquisition_fixture_prepare",
        REPO / "examples/governed-acquisition/prepare.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    module.prepare(project, fixture_python)
    gateway = json.loads((project / "gateway.json").read_text())
    gateway["source_publication"] = True
    # This qualification isolates sharing from model/inspector deployment. The
    # structured schema and original source restrictions still govern the read.
    gateway.pop("privacy_inspector_file", None)
    authentication = json.loads((project / "authentication.json").read_text())
    authority = json.loads((project / "authority.json").read_text())
    original_grant = copy.deepcopy(authority["grants"][0])
    tokens = {}
    for principal in ("reader-a", "reader-b", "reader-denied"):
        token = "synthetic-coalescing-" + principal + "-token-0123456789"
        tokens[principal] = token
        authentication["principals"].append({"tenant": "synthetic-tenant", "principal": principal,
            "purpose": "synthetic-acquisition", "grant_ref": principal + "-grant",
            "agents": ["collector"], "token_sha256": hashlib.sha256(token.encode()).hexdigest()})
        grant = copy.deepcopy(original_grant)
        grant.update(principal=principal, grant_ref=principal + "-grant")
        authority["grants"].append(grant)
    for policy in authority["policies"]:
        resolve = next(rule for rule in policy["rules"]
            if rule["operation"] == "resolve" and rule["actor"] == "collector")
        publish = copy.deepcopy(resolve)
        publish["operation"] = "publish-source"
        policy["rules"].append(publish)
        for principal in ("reader-a", "reader-b"):
            reader = copy.deepcopy(resolve)
            reader.update(principal=principal, recipient=principal)
            policy["rules"].append(reader)
    write(project / "authentication.json", authentication)
    write(project / "authority.json", authority)
    return gateway, authority, tokens


def run(host, fixture_python, sdk_root=REPO):
    host = Path(host).resolve(strict=True)
    host_digest = digest_file(host)
    with tempfile.TemporaryDirectory(prefix="mas-coalesced-read-") as temporary:
        project = Path(temporary) / "application"
        gateway, authority, tokens = prepare(project, fixture_python)
        checked(host, "build", "--project", project, "--sdk-root", Path(sdk_root).resolve(strict=True))
        descriptor = checked(host, "describe", "--bundle", project / ".mas/build/bundle.json")["bundle_digest"]
        with socket.socket() as listener:
            listener.bind(("127.0.0.1", 0))
            port = listener.getsockname()[1]
        endpoint = f"http://127.0.0.1:{port}/a2a"
        gateway.update(listen=f"127.0.0.1:{port}", public_url=endpoint)
        write(project / "gateway.json", gateway)
        now_ms = time.time_ns() // 1_000_000
        registration = RegisteredRead({"version": "mas-coalesced-read/1", "authority": "fixture-host/1",
            "tenant": "synthetic-tenant", "acquisition_grant": "synthetic-grant",
            "purpose": "synthetic-acquisition", "credential_epoch": "credential-1", "policy_epoch": "policy-1",
            "source": "synthetic-deployment-fixture/1", "query": {"namespace_uid": "fixture-namespace-001",
                "resource_uid": "fixture-deployment-001"}, "generation": "original-acquisition-1",
            "projection": "governed-deployment-point/1", "schema_version": "point/1",
            "transform_version": "collector/1", "agent": "collector", "descriptor_digest": descriptor,
            "effect_key": "acquire", "submission_deadline_ms": now_ms + 120000,
            "transport_quota_scope": "fixture-host/task-service"}, {"mode": "acquire"}, identity_key=os.urandom(32))
        def rpc(token):
            return HTTPRPC(endpoint, "fixture-host/1", "fixture-host/task-service", lambda: token,
                allow_loopback_http=True, timeout_ms=10000)
        producer = LoseSubmissionAcknowledgement(rpc(TOKEN))
        service = Service(host, project, port)
        try:
            acquisition = Acquisition(registration, producer)
            first = acquisition.advance(now_ms=now_ms, resolution_deadline_ms=now_ms + 120000)
            assert first["state"] == "submission-unresolved" and producer.lost, "lost-ack-not-exercised"
            # Restore the trusted producer owner checkpoint after the lost reply.
            acquisition = Acquisition(registration, producer, trusted_checkpoint=acquisition.checkpoint())
            progress = first
            until = time.monotonic() + 120
            while time.monotonic() < until:
                progress = acquisition.advance(now_ms=time.time_ns() // 1_000_000,
                    resolution_deadline_ms=now_ms + 120000)
                if progress["state"] == "published":
                    break
                assert progress["state"] != "terminal-without-source", "host-task-did-not-complete"
                time.sleep(.1)
            assert progress["state"] == "published", "publication-timeout"
            assert len(producer.submissions) >= 2 and all(item == producer.submissions[0]
                for item in producer.submissions), "changed-original-submission"
            publication, checkpoint = progress["publication"], acquisition.checkpoint()
            deliveries = {}
            for principal in ("reader-a", "reader-b", "reader-denied"):
                subscriber = Subscription(principal, registration, rpc(tokens[principal]),
                    deadline_ms=now_ms + 120000, max_age_ms=120000)
                deliveries[principal] = subscriber.deliver(publication, now_ms=time.time_ns() // 1_000_000)
            assert deliveries["reader-a"]["state"] == deliveries["reader-b"]["state"] == "available", "permitted-reader-denied"
            assert deliveries["reader-denied"]["state"] == "unavailable", "independent-reader-policy-bypassed"
            original_source = deliveries["reader-a"]["source"]
            assert original_source == deliveries["reader-b"]["source"], "retained-source-changed"
            stale = Subscription("stale-reader", registration, rpc(tokens["reader-a"]),
                deadline_ms=now_ms + 120000,
                min_acquired_at_ms=original_source["source_metadata"]["acquired_at_ms"] + 1)
            assert stale.deliver(publication, now_ms=time.time_ns() // 1_000_000)["state"] == "stale"
            cancelled = Subscription("cancelled-reader", registration, rpc(tokens["reader-a"]),
                deadline_ms=now_ms + 120000)
            cancelled.cancel()
            assert cancelled.deliver(publication, now_ms=time.time_ns() // 1_000_000)["state"] == "cancelled"
            # Revoke one independent current grant; the other reader still works.
            revoked = copy.deepcopy(authority)
            revoked["grants"] = [grant for grant in revoked["grants"] if grant["principal"] != "reader-a"]
            write(project / "authority.json", revoked)
            for principal, expected in (("reader-a", "unavailable"), ("reader-b", "available")):
                current = Subscription("current-" + principal, registration, rpc(tokens[principal]),
                    deadline_ms=now_ms + 120000)
                assert current.deliver(publication, now_ms=time.time_ns() // 1_000_000)["state"] == expected
            write(project / "authority.json", authority)
            assert (project / "collector-calls").read_bytes() == b"x", "physical-acquisition-repeated"
        finally:
            service.close()
        restarted = Service(host, project, port)
        try:
            recovered = Acquisition(registration, rpc(TOKEN), trusted_checkpoint=checkpoint)
            assert recovered.publication() == publication
            reader = Subscription("restart-reader", registration, rpc(tokens["reader-b"]),
                deadline_ms=now_ms + 120000)
            result = reader.deliver(recovered.publication(), now_ms=time.time_ns() // 1_000_000)
            assert result["state"] == "available" and result["source"] == original_source, "restart-source-changed"
            assert (project / "collector-calls").read_bytes() == b"x", "restart-refetched-source"
        finally:
            restarted.close()
        assert digest_file(host) == host_digest, "host-binary-changed-during-acceptance"
        return {"version": "mas-coalesced-host-acceptance/1", "status": "PASS",
            "host_sha256": host_digest, "descriptor_digest": descriptor,
            "physical_acquisitions": 1, "permitted_subscribers": 2,
            "lost_ack_original_identity_replay": "PASS", "independent_reader_denial": "PASS",
            "current_grant_revocation": "PASS", "stale_refetch_suppression": "PASS",
            "subscriber_cancellation": "PASS", "restart_identity_and_source_times": "PASS",
            "external_provider": "NOT_RUN", "durable_logical_subscriptions": "NOT_RUN",
            "multi_replica_coordination": "NOT_RUN"}


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--host", required=True)
    parser.add_argument("--fixture-python", default="/usr/bin/python3")
    parser.add_argument("--sdk-root", default=str(REPO))
    args = parser.parse_args()
    print(json.dumps(run(args.host, args.fixture_python, args.sdk_root), sort_keys=True))
