"""Extend the existing privacy-response application's capability declarations."""
import importlib.util
import hashlib
import json
from pathlib import Path
import shutil

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parents[1]
_spec = importlib.util.spec_from_file_location("response_prepare", ROOT.parent / "privacy-response/prepare.py")
_base = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(_base)


def prepare(destination, fixture_python="/usr/bin/python3", *, composed_inspection=False):
    destination = _base.prepare(destination, fixture_python)
    shutil.copyfile(destination / "application.py", destination / "response_application.py")
    shutil.copyfile(ROOT / "application.py", destination / "application.py")
    if composed_inspection:
        # End-to-end fixture allows its actual inspector launch work inside the
        # queue lifetime. Production timing targets require separate measurement.
        application = (destination / "application.py").read_text().replace("max_age_seconds=60", "max_age_seconds=1800")
        (destination / "application.py").write_text(application)
    modes = _base.enum("retry-once", "reconcile-accepted", "reconcile-absent", "reconcile-unknown", "acknowledged", "escalate")
    governance = json.loads((destination / "governance.json").read_text())
    output = governance["agents"][0]["output_schema"]
    output["properties"]["acknowledgement"] = _base.enum("acknowledged", "pending", "unknown")
    output["properties"]["escalation"] = _base.enum("not-required", "accepted", "refused", "not-sent", "throttled", "uncertain", "expired", "exhausted")
    output["properties"]["submission"] = _base.enum("accepted", "refused", "throttled", "not-sent", "uncertain", "expired", "exhausted")
    for actor in governance["agents"]:
        actor["input_schema"]["properties"]["mode"] = modes
    effects = governance["agents"][0]["effects"]
    event = effects[0]["descriptor_schema"]["properties"]["event"]
    target = effects[0]["descriptor_schema"]["properties"]["target_binding"]
    policy = {"version": "mas-privacy-retry-policy/1", "max_attempts": 2, "backoff_seconds": 1, "max_age_seconds": 1800 if composed_inspection else 60}
    digest = hashlib.sha256(json.dumps(policy, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
    common = {"event": event, "target_binding": target, "policy_binding": _base.enum(digest), "notification_kind": _base.enum("review", "escalation")}
    integer = {"type": "integer", "minimum": 0, "maximum": 4102444800000}
    scopes = {"mas-privacy-queue-intent/1": "host-case-intent", "mas-privacy-queue-timer/1": "host-notification-timer",
              "mas-privacy-queue-submit/1": "notification-endpoint-submission", "mas-privacy-queue-reconcile/1": "notification-provider-lookup"}
    for name, scope in scopes.items():
        descriptor = {"format": _base.enum(name), **common}
        if "intent" in name:
            result = _base.object_schema({"queued_at_ms": integer, "expires_at_ms": integer})
        else:
            descriptor["attempt"] = {"type": "integer", "minimum": 0, "maximum": 7}
            if "timer" in name:
                descriptor["delay_seconds"] = {"type": "integer", "minimum": 0, "maximum": 3600}
                result = _base.object_schema({"due_at_ms": integer})
            elif "submit" in name:
                result = output
            else:
                result = _base.object_schema({"acceptance": _base.enum("accepted", "not-accepted", "unknown")})
        effects.append({"format": name, "destination": "host-case-journal" if "intent" in name or "timer" in name else "fixture-incident-endpoint",
                        "descriptor_schema": _base.object_schema(descriptor), "output_schema": result,
                        "evidence_schema": _base.object_schema({"execution": _base.enum("completed", "refused", "not-sent", "unknown"), "execution_scope": _base.enum(scope)})})
    ack_common = {"event": event, "target_binding": target, "acknowledgement_policy": _base.object_schema({
        "wait_seconds": {"type":"integer","minimum":1,"maximum":1},
        "poll_seconds": {"type":"integer","minimum":1,"maximum":1}})}
    for name, result, scope in (("mas-privacy-acknowledgement-timer/1", _base.object_schema({"started_at_ms":integer,"deadline_ms":integer}), "host-acknowledgement-timer"),
                               ("mas-privacy-acknowledgement-read/1", _base.object_schema({"acknowledgement":_base.enum("acknowledged","pending","unknown")}), "notification-acknowledgement-lookup")):
        descriptor = {"format":_base.enum(name),**ack_common}
        if "read" in name: descriptor["poll"]={"type":"integer","minimum":0,"maximum":32}
        effects.append({"format":name,"destination":"host-case-journal" if "timer" in name else "fixture-incident-endpoint",
            "descriptor_schema":_base.object_schema(descriptor),"output_schema":result,
            "evidence_schema":_base.object_schema({"execution":_base.enum("completed"),"execution_scope":_base.enum(scope)})})
    authority = json.loads((destination / "authority.json").read_text())
    rules = authority["policies"][0]["rules"]
    template = next(rule for rule in rules if rule["operation"] == "effect" and rule["actor"] == "responder")
    for effect in effects[2:]:
        rules.append({**template, "recipient": "operation:" + effect["format"], "destination": effect["destination"]})
    if composed_inspection:
        for actor in ("collector","responder"):
            rule = next(rule for rule in rules if rule["actor"] == actor)
            rules.append({**rule,"operation":"retain-assessment","recipient":"host-assessment-journal","destination":"host-journal"})
        for operation, recipient, target_name in (("resolve-assessment","synthetic-user","client"),
                                                  ("adjudicate-assessment","host-assessment-journal","host-journal")):
            rules.append({**template,"operation":operation,"recipient":recipient,"destination":target_name})
        inspector = json.loads((destination / "privacy-inspector.json").read_text())
        inspector["version"] = "mas-governed-privacy-inspector/3"
        (destination / "privacy-inspector.json").write_text(json.dumps(inspector))
    for name, content in (("governance.json", governance), ("authority.json", authority),
                          ("input-schema.json", governance["agents"][0]["input_schema"]),
                          ("output-schema.json", output), ("input.json", {"mode": "retry-once"})):
        (destination / name).write_text(json.dumps(content, indent=2) + "\n")
    manifest = (destination / "agents.toml").read_text().replace("max_calls = 4", "max_calls = 16").replace("max_messages = 16", "max_messages = 64")
    if composed_inspection:
        manifest = manifest.replace("wall_seconds = 300", "wall_seconds = 1800")
    (destination / "agents.toml").write_text(manifest)
    return destination
