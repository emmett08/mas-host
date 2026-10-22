"""Exercise queued notifications with exact-action reconciliation on the host."""
from pathlib import Path
import time

from mas_host import EffectResult
from privacy_response import (CaseEvent, RetryPolicy, SubmissionUncertain, respond_queued,
                              AcknowledgementPolicy, respond_until_acknowledged)
from response_application import collect, FixtureSink


class LifecycleSink(FixtureSink):
    def submit(self, notification):
        prior = Path("notification-calls")
        attempt = len(prior.read_bytes()) if prior.exists() else 0
        prior.write_bytes(b"x" * (attempt + 1))
        Path("notification.json").write_bytes(notification.encoded())
        if self.mode.startswith("reconcile") and attempt == 0:
            raise SubmissionUncertain("synthetic-lost-acknowledgement")
        state = "throttled" if self.mode == "retry-once" and attempt == 0 else "accepted"
        return EffectResult({"submission": state, "recipient_delivery": "unknown", "retry_after_seconds": 1 if state == "throttled" else 0,
                             "response_metadata": "withheld"},
                            {"execution": "refused" if state == "throttled" else "completed",
                             "execution_scope": "notification-endpoint-submission"})

    def acknowledged(self, _notification):
        return "acknowledged" if self.mode == "acknowledged" else "pending"

    def reconcile(self, notification):
        prior = Path("reconciliation-calls")
        prior.write_bytes((prior.read_bytes() if prior.exists() else b"") + b"x")
        assert Path("notification.json").read_bytes() == notification.encoded()
        return {"reconcile-accepted": "accepted", "reconcile-absent": "not-accepted", "reconcile-unknown": "unknown"}[self.mode]


def respond(ctx, value):
    observed = ctx.request("collect", "collector", {"mode": value["mode"]})
    if observed != {"version": "mas-governed-unavailable/1", "external_execution": "completed", "payload_disposition": "withheld"}:
        raise RuntimeError("synthetic-expected-withheld-result")
    event = CaseEvent("handling-withheld", "withheld", "blocked")
    policy = RetryPolicy(max_attempts=2, backoff_seconds=1, max_age_seconds=60)
    sink = LifecycleSink(value["mode"])
    if value["mode"] in ("acknowledged", "escalate"):
        result = respond_until_acknowledged(ctx, event, sink, policy,
                         AcknowledgementPolicy(wait_seconds=1, poll_seconds=1))
        return {**result["notification"], "acknowledgement": result["acknowledgement"], "escalation": result["escalation"]}
    return respond_queued(ctx, event, sink, policy)
