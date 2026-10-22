"""Synthetic governed handling, safe intent and registered notification example."""
import hashlib
from pathlib import Path
import time

from mas_host import Context, EffectResult
from privacy_response import CaseEvent, respond as submit_response, SubmissionUncertain


PRIVATE = "private-response-person-8731@example.invalid"


def collect(ctx: Context, value: dict) -> dict:
    def execute(_effect_id):
        with open("collector-calls", "a", encoding="ascii") as stream:
            stream.write("x")
        return EffectResult({"approved": PRIVATE}, {"execution": "completed"})
    return ctx.effect("collect", {"format": "synthetic-private-collection/1"}, execute)


class FixtureSink:
    def __init__(self, mode):
        self.mode = mode

    def configuration_binding(self):
        return hashlib.sha256(b'mas-privacy-fixture-incident-endpoint/1').hexdigest()

    def submit(self, notification):
        # This synthetic sink records only the fixed safe projection. No
        # external endpoint is contacted. Real deployments can substitute the
        # explicit HttpsIncidentSink under their approved endpoint contract.
        with open("notification-calls", "a", encoding="ascii") as stream:
            stream.write("x")
        Path("notification.json").write_bytes(notification.encoded())
        if self.mode == "unknown":
            raise SubmissionUncertain("synthetic-lost-acknowledgement")
        state = "throttled" if self.mode == "throttled" else "accepted"
        return EffectResult(
            {"submission": state, "recipient_delivery": "unknown",
             "retry_after_seconds": 60 if state == "throttled" else 0,
             "response_metadata": "withheld"},
            {"execution": "refused" if state == "throttled" else "completed",
             "execution_scope": "notification-endpoint-submission"})


class PausingContext:
    """Give the external test controller a pre-admission revocation point."""
    def __init__(self, ctx):
        self.ctx = ctx
        self.invocation_id = ctx.invocation_id

    def effect(self, key, descriptor, execute):
        result = self.ctx.effect(key, descriptor, execute)
        if key == "privacy-case-intent":
            Path("case-ready").write_text("ready", encoding="ascii")
            deadline = time.monotonic() + 10
            while not Path("continue").exists():
                if time.monotonic() >= deadline:
                    raise RuntimeError("synthetic-controller-timeout")
                time.sleep(0.01)
        return result


def respond(ctx: Context, value: dict) -> dict:
    observed = ctx.request("collect", "collector", {"mode": value["mode"]})
    expected = {"version": "mas-governed-unavailable/1", "external_execution": "completed",
                "payload_disposition": "withheld"}
    if observed != expected:
        raise RuntimeError("synthetic-expected-known-completed-withheld")
    # A withheld marker alone supplies no category findings or evidence of
    # exposure. It supports this bounded handling-review event only.
    event = CaseEvent("handling-withheld", "withheld", "blocked")
    selected = PausingContext(ctx) if value["mode"] == "revoke-before-notify" else ctx
    return submit_response(selected, event, FixtureSink(value["mode"]))
