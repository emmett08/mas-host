"""Run a bounded durable notification queue through the host's existing effects.

The host journal owns every intent, timer, submission and lookup. This module
adds no database or execution owner. A pending timer occupies its trusted worker;
shared scheduling and admission remain separately qualified host capabilities.
"""
from __future__ import annotations

from dataclasses import dataclass
import hashlib
import re
import time

from mas_host import EffectResult
from .contracts import CaseEvent, Notification, ResponseError, canonical
from .notifier import SubmissionUncertain


@dataclass(frozen=True)
class RetryPolicy:
    max_attempts: int = 3
    backoff_seconds: int = 1
    max_age_seconds: int = 600

    def __post_init__(self):
        for value, lower, upper in ((self.max_attempts, 1, 8),
                                    (self.backoff_seconds, 1, 300),
                                    (self.max_age_seconds, 1, 3600)):
            if type(value) is not int or not lower <= value <= upper:
                raise ResponseError("privacy-lifecycle-policy-invalid")

    def value(self):
        return {"version": "mas-privacy-retry-policy/1", "max_attempts": self.max_attempts,
                "backoff_seconds": self.backoff_seconds, "max_age_seconds": self.max_age_seconds}


def _safe_submission(state, execution, retry_after=0):
    return EffectResult({"submission": state, "recipient_delivery": "unknown",
                         "retry_after_seconds": retry_after, "response_metadata": "withheld"},
                        {"execution": execution,
                         "execution_scope": "notification-endpoint-submission"})


def _millis(clock):
    now = clock()
    if type(now) not in (int, float) or not 0 <= now <= 4_102_444_800:
        raise ResponseError("privacy-lifecycle-clock-invalid")
    return int(now * 1000)


def respond_queued(ctx, event: CaseEvent, sink, policy=RetryPolicy(), *, clock=time.time,
                   sleep=time.sleep, notification_kind="review"):
    """Retain a queue intent and bounded retry schedule before each admission.

    Stable host step keys replay completed attempts. Every fresh submission and
    read-only provider reconciliation is a distinct registered effect requiring
    current authority. All attempts retain the same endpoint idempotency key.
    An ambiguous send is settled as unknown, then a separately admitted lookup
    may supply later evidence. Only a contractually authoritative `not-accepted`
    permits another attempt. Unavailable/withheld evidence stops the queue.
    """
    if type(event) is not CaseEvent or type(policy) is not RetryPolicy:
        raise ResponseError("privacy-lifecycle-contract-invalid")
    notification = Notification.for_invocation(ctx.invocation_id, event, kind=notification_kind)
    target = sink.configuration_binding()
    if type(target) is not str or re.fullmatch(r"[0-9a-f]{64}", target) is None:
        raise ResponseError("privacy-notification-target-invalid")
    policy_binding = hashlib.sha256(canonical(policy.value())).hexdigest()
    common = {"event": event.safe_record(), "target_binding": target,
              "policy_binding": policy_binding, "notification_kind": notification_kind}
    def intent(_effect_id):
        now = _millis(clock)
        return EffectResult({"queued_at_ms": now, "expires_at_ms": now + policy.max_age_seconds * 1000},
                            {"execution": "completed", "execution_scope": "host-case-intent"})
    queued = ctx.effect("privacy-queue-intent", {"format": "mas-privacy-queue-intent/1", **common}, intent)
    if (type(queued) is not dict or set(queued) != {"queued_at_ms", "expires_at_ms"}
            or any(type(queued[key]) is not int for key in queued)
            or queued["expires_at_ms"] - queued["queued_at_ms"] != policy.max_age_seconds * 1000):
        raise ResponseError("privacy-queue-intent-unavailable")
    delay = 0
    for attempt in range(policy.max_attempts):
        def schedule(_effect_id):
            return EffectResult({"due_at_ms": _millis(clock) + delay * 1000},
                                {"execution": "completed", "execution_scope": "host-notification-timer"})
        scheduled = ctx.effect(f"privacy-queue-timer-{attempt}",
                               {"format": "mas-privacy-queue-timer/1", "attempt": attempt,
                                "delay_seconds": delay, **common}, schedule)
        if (type(scheduled) is not dict or set(scheduled) != {"due_at_ms"}
                or type(scheduled["due_at_ms"]) is not int
                or scheduled["due_at_ms"] < queued["queued_at_ms"]):
            raise ResponseError("privacy-queue-timer-unavailable")
        while True:
            now = _millis(clock)
            if now >= queued["expires_at_ms"] or scheduled["due_at_ms"] >= queued["expires_at_ms"]:
                break
            remaining = scheduled["due_at_ms"] - now
            if remaining <= 0:
                break
            # This bounded timer does not release a worker or claim distributed
            # scheduling. The host's invocation deadline remains authoritative.
            sleep(min(remaining / 1000, 1))
        def submit(_effect_id):
            # A cached completed attempt bypasses this callback on recovery.
            # Only a fresh attempt can be classified expired by today's clock.
            if _millis(clock) >= queued["expires_at_ms"] or scheduled["due_at_ms"] >= queued["expires_at_ms"]:
                return _safe_submission("expired", "not-sent")
            if sink.configuration_binding() != target:
                return _safe_submission("not-sent", "not-sent")
            try:
                return sink.submit(notification)
            except SubmissionUncertain:
                # Complete the original observation without pretending its
                # external acceptance is known. No automatic resend follows.
                return _safe_submission("uncertain", "unknown")
        result = ctx.effect(f"privacy-queue-submit-{attempt}",
                            {"format": "mas-privacy-queue-submit/1", "attempt": attempt, **common}, submit)
        if (type(result) is not dict or set(result) != {"submission", "recipient_delivery",
                                                     "retry_after_seconds", "response_metadata"}
                or result.get("recipient_delivery") != "unknown"
                or result.get("response_metadata") not in ("available", "withheld")
                or type(result.get("retry_after_seconds")) is not int
                or not 0 <= result["retry_after_seconds"] <= 3600):
            raise ResponseError("privacy-queue-submission-unavailable")
        state = result["submission"]
        if state in ("accepted", "refused", "expired"):
            return result
        if state == "uncertain":
            lookup = getattr(sink, "reconcile", None)
            if not callable(lookup):
                return result
            def reconcile(_effect_id):
                if sink.configuration_binding() != target:
                    state = "unknown"
                else:
                    try:
                        state = lookup(notification)
                    except (OSError, TimeoutError, SubmissionUncertain):
                        state = "unknown"
                if state not in ("accepted", "not-accepted", "unknown"):
                    raise ResponseError("privacy-reconciliation-invalid")
                return EffectResult({"acceptance": state},
                                    {"execution": "completed", "execution_scope": "notification-provider-lookup"})
            observation = ctx.effect(f"privacy-queue-reconcile-{attempt}",
                                     {"format": "mas-privacy-queue-reconcile/1", "attempt": attempt, **common}, reconcile)
            if (type(observation) is not dict or set(observation) != {"acceptance"}
                    or observation["acceptance"] not in ("accepted", "not-accepted", "unknown")):
                raise ResponseError("privacy-reconciliation-unavailable")
            if observation["acceptance"] == "accepted":
                return _safe_submission("accepted", "completed").output
            if observation["acceptance"] == "unknown":
                return result
        elif state not in ("throttled", "not-sent"):
            raise ResponseError("privacy-queue-submission-invalid")
        delay = max(policy.backoff_seconds * 2 ** attempt, result["retry_after_seconds"])
    return _safe_submission("exhausted", "not-sent").output


@dataclass(frozen=True)
class AcknowledgementPolicy:
    wait_seconds: int = 900
    poll_seconds: int = 30

    def __post_init__(self):
        if (type(self.wait_seconds) is not int or not 1 <= self.wait_seconds <= 900
                or type(self.poll_seconds) is not int or not 1 <= self.poll_seconds <= self.wait_seconds
                or (self.wait_seconds + self.poll_seconds - 1) // self.poll_seconds > 32):
            raise ResponseError("privacy-acknowledgement-policy-invalid")


class _PrefixContext:
    def __init__(self, ctx, prefix):
        self.ctx, self.prefix, self.invocation_id = ctx, prefix, ctx.invocation_id
    def effect(self, key, descriptor, execute):
        return self.ctx.effect(self.prefix + key, descriptor, execute)


def respond_until_acknowledged(ctx, event, sink, retry_policy=RetryPolicy(),
                               acknowledgement_policy=AcknowledgementPolicy(), *,
                               clock=time.time, sleep=time.sleep):
    """Retain bounded acknowledgement polls and a separately admitted escalation.

    Endpoint acceptance and recipient acknowledgement remain separate. A pending
    or unknown acknowledgement at the retained deadline permits the configured
    escalation event only through its own fresh host admission. The fixed
    escalation template keeps the original case and a distinct action identity.
    """
    if type(acknowledgement_policy) is not AcknowledgementPolicy:
        raise ResponseError("privacy-acknowledgement-policy-invalid")
    result = respond_queued(_PrefixContext(ctx, "initial-"), event, sink, retry_policy,
                            clock=clock, sleep=sleep)
    if result["submission"] != "accepted":
        return {"notification": result, "acknowledgement": "unknown", "escalation": "not-required"}
    acknowledgement = getattr(sink, "acknowledged", None)
    if not callable(acknowledgement):
        raise ResponseError("privacy-acknowledgement-capability-required")
    notification = Notification.for_invocation(ctx.invocation_id, event)
    target = sink.configuration_binding()
    config = {"wait_seconds": acknowledgement_policy.wait_seconds,
              "poll_seconds": acknowledgement_policy.poll_seconds}
    common = {"event": event.safe_record(), "target_binding": target,
              "acknowledgement_policy": config}
    def timer(_effect_id):
        now = _millis(clock)
        return EffectResult({"started_at_ms": now,
                             "deadline_ms": now + acknowledgement_policy.wait_seconds * 1000},
                            {"execution": "completed", "execution_scope": "host-acknowledgement-timer"})
    scheduled = ctx.effect("privacy-acknowledgement-timer",
                           {"format": "mas-privacy-acknowledgement-timer/1", **common}, timer)
    if (type(scheduled) is not dict or set(scheduled) != {"started_at_ms", "deadline_ms"}
            or any(type(value) is not int for value in scheduled.values())
            or scheduled["deadline_ms"] - scheduled["started_at_ms"] != acknowledgement_policy.wait_seconds * 1000):
        raise ResponseError("privacy-acknowledgement-timer-unavailable")
    polls = (acknowledgement_policy.wait_seconds + acknowledgement_policy.poll_seconds - 1) // acknowledgement_policy.poll_seconds
    last = "unknown"
    for index in range(polls + 1):
        due = min(scheduled["started_at_ms"] + index * acknowledgement_policy.poll_seconds * 1000,
                  scheduled["deadline_ms"])
        while _millis(clock) < due:
            sleep(min((due - _millis(clock)) / 1000, 1))
        def lookup(_effect_id):
            if sink.configuration_binding() != target:
                state = "unknown"
            else:
                try:
                    state = acknowledgement(notification)
                except (OSError, TimeoutError, SubmissionUncertain):
                    state = "unknown"
            if state not in ("acknowledged", "pending", "unknown"):
                raise ResponseError("privacy-acknowledgement-invalid")
            return EffectResult({"acknowledgement": state},
                                {"execution": "completed", "execution_scope": "notification-acknowledgement-lookup"})
        observed = ctx.effect(f"privacy-acknowledgement-read-{index}",
                              {"format": "mas-privacy-acknowledgement-read/1", "poll": index, **common}, lookup)
        if (type(observed) is not dict or set(observed) != {"acknowledgement"}
                or observed["acknowledgement"] not in ("acknowledged", "pending", "unknown")):
            raise ResponseError("privacy-acknowledgement-unavailable")
        last = observed["acknowledgement"]
        if last == "acknowledged":
            return {"notification": result, "acknowledgement": last, "escalation": "not-required"}
    escalation = respond_queued(_PrefixContext(ctx, "escalation-"), event, sink, retry_policy,
                                clock=clock, sleep=sleep, notification_kind="escalation")
    return {"notification": result, "acknowledgement": last, "escalation": escalation["submission"]}
