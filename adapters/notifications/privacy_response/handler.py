"""Retain safe case intent before admitting a separate notification effect."""
from __future__ import annotations

import re

from mas_host import EffectResult

from .contracts import CaseEvent, Notification, ResponseError


def respond(ctx, event: CaseEvent, sink):
    """Run an explicitly registered trusted response, with no automatic retry.

    The first effect appends the safe case/intent to the existing host journal.
    It does not create an external case service. A crash after that commit can
    resume the same notification admission. Each effect is checked by the host
    at fresh admission, independently of this application's route selection.
    """
    notification = Notification.for_invocation(ctx.invocation_id, event)
    target_binding = sink.configuration_binding()
    if type(target_binding) is not str or re.fullmatch(r"[0-9a-f]{64}", target_binding) is None:
        raise ResponseError("privacy-notification-target-invalid")
    event_record = event.safe_record()
    intent = ctx.effect(
        "privacy-case-intent",
        {"format": "mas-privacy-case-intent/1", "event": event_record, "target_binding": target_binding},
        lambda _effect_id: EffectResult(
            {"case_retained": True, "notification_pending": True},
            {"execution": "completed", "execution_scope": "host-case-intent"}),
    )
    if intent != {"case_retained": True, "notification_pending": True}:
        # A withheld intent must never accidentally authorise a notification.
        raise ResponseError("privacy-case-intent-unavailable")
    def submit(_effect_id):
        if sink.configuration_binding() != target_binding:
            # This callback may already have an admitted host effect. A changed
            # local target is proved unsent, so retain that fact instead of
            # turning a pre-send configuration refusal into uncertainty.
            return EffectResult(
                {"submission": "not-sent", "recipient_delivery": "unknown",
                 "retry_after_seconds": 0, "response_metadata": "withheld"},
                {"execution": "not-sent", "execution_scope": "notification-endpoint-submission"})
        return sink.submit(notification)

    result = ctx.effect(
        "privacy-notification",
        {"format": "mas-privacy-notification-submit/1", "event": event_record, "target_binding": target_binding},
        submit,
    )
    # Exact host unavailable markers remain authoritative on fresh completion
    # and replay. The caller does not reinterpret them as permission to retry.
    return result
