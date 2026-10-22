"""Project a privacy case from two authorised retained-effect views."""
from __future__ import annotations

import re

from .contracts import CaseEvent, Notification, ResponseError, canonical


_VIEW_FIELDS = frozenset(("version", "namespace_ref", "actor", "parent_invocation", "key",
                          "effect_id", "state", "host_settlement", "external_execution",
                          "payload_disposition", "descriptor", "output", "evidence"))
_STATES = frozenset(("not-recorded", "prepared", "executing", "unknown", "completed",
                     "resolved", "blocked", "failed", "rejected"))
_SETTLEMENTS = frozenset(("pending", "completed", "failed", "unknown"))
_CERTAINTIES = frozenset(("completed", "not-sent", "refused", "unknown"))
_DISPOSITIONS = frozenset(("available", "withheld", "unavailable"))
_SETTLEMENT_BY_STATE = {"not-recorded": "unknown", "prepared": "pending", "executing": "pending",
                        "unknown": "unknown", "blocked": "unknown", "failed": "failed", "rejected": "failed",
                        "completed": "completed", "resolved": "completed"}


def _require(condition):
    if not condition:
        raise ResponseError("privacy-case-view-invalid")


def _record(value, fields):
    _require(type(value) is dict and set(value) == set(fields))


def _choice(value, choices):
    _require(type(value) is str and value in choices)


def _hex(value, prefix=""):
    _require(type(value) is str and re.fullmatch(re.escape(prefix) + r"[0-9a-f]{64}", value) is not None)


def _view(value, key):
    _record(value, _VIEW_FIELDS)
    _require(value["version"] == "mas-governed-effect-view/1" and value["key"] == key)
    _hex(value["namespace_ref"])
    _hex(value["parent_invocation"], "task-")
    actor = value["actor"]
    _require(type(actor) is str and re.fullmatch(r"[A-Za-z0-9._:/-]{1,128}", actor) is not None)
    _choice(value["state"], _STATES)
    _choice(value["host_settlement"], _SETTLEMENTS)
    _choice(value["external_execution"], _CERTAINTIES)
    _choice(value["payload_disposition"], _DISPOSITIONS)
    _require(value["host_settlement"] == _SETTLEMENT_BY_STATE[value["state"]])
    if value["state"] == "not-recorded":
        _require(value["effect_id"] is None and value["external_execution"] == "unknown"
                 and value["host_settlement"] != "completed" and value["payload_disposition"] == "unavailable")
    else:
        _hex(value["effect_id"], "eff-")
    if value["state"] in ("completed", "resolved"):
        _require(value["payload_disposition"] in ("available", "withheld"))
    else:
        _require(value["external_execution"] == "unknown" and value["payload_disposition"] == "unavailable")
    if value["payload_disposition"] == "available":
        _require(type(value["descriptor"]) is dict)
        for field in ("output", "evidence"):
            _require(value[field] is None or type(value[field]) is dict)
        _require(type(value["output"]) is dict and type(value["evidence"]) is dict)
    else:
        _require(all(value[field] is None for field in ("descriptor", "output", "evidence")))
    try:
        _require(len(canonical(value)) <= 65536)
    except (TypeError, ValueError, UnicodeError, RecursionError):
        raise ResponseError("privacy-case-view-invalid") from None
    return value


def _event(value):
    _record(value, ("version", "event_version", "reason", "disposition", "exposure",
                    "categories", "finding_count", "route"))
    _require(value["version"] == "mas-privacy-case-event/1" and type(value["categories"]) is list)
    event = CaseEvent(value["reason"], value["disposition"], value["exposure"],
                      value["event_version"], tuple(value["categories"]), value["finding_count"])
    _require(event.safe_record() == value)
    return event


def _descriptor(value, kind):
    _record(value, ("format", "event", "target_binding"))
    _require(value["format"] == kind)
    _hex(value["target_binding"])
    return _event(value["event"])


def _evidence(view, scope):
    value = view["evidence"]
    fields = {"version", "external_execution", "payload_disposition", "reported_evidence"}
    _require(type(value) is dict and fields <= set(value) <= fields | {"inspection"})
    _require(value["version"] == "mas-governed-effect-evidence/1"
             and value["external_execution"] == view["external_execution"]
             and value["payload_disposition"] == "available")
    reported = value["reported_evidence"]
    _record(reported, ("execution", "execution_scope"))
    _require(reported["execution"] == view["external_execution"] and reported["execution_scope"] == scope)
    if "inspection" in value:
        receipt = value["inspection"]
        _record(receipt, ("version", "contract", "original_binding", "projection_binding", "completion",
                          "coverage", "original_findings_count", "projected_findings_count", "text_units",
                          "inspected_sources", "boundary_bytes"))
        _require(receipt["version"] == "mas-governed-inspection-receipt/1"
                 and receipt["completion"] == "complete"
                 and receipt["coverage"] == "complete-within-json-text-components/1"
                 and type(receipt["contract"]) is dict
                 and receipt["contract"].get("version") == "mas-governed-inspection/1")
        _hex(receipt["original_binding"])
        _hex(receipt["projection_binding"])
        for field, maximum in (("original_findings_count", 16384), ("projected_findings_count", 0),
                               ("text_units", 128), ("boundary_bytes", 16384)):
            _require(type(receipt[field]) is int and 0 <= receipt[field] <= maximum)
        _require(type(receipt["inspected_sources"]) is int
                 and 0 <= receipt["inspected_sources"] <= receipt["text_units"])
        # The authenticated host validates the current detector contract and
        # source/projection HMAC. Those keys are deliberately absent here.


def case_status(intent_view, notification_view):
    """Validate one immutable case intent and its selected notification record.

    Callers obtain both records from the authenticated host resolver. Structural
    validation supplies no authentication of a supplied JSON object. The view
    preserves the restrictions and snapshot limits of those reads. Lookup
    failures must remain failures rather than fabricated missing records.
    """
    intent = _view(intent_view, "privacy-case-intent")
    notification = _view(notification_view, "privacy-notification")
    for field in ("namespace_ref", "actor", "parent_invocation"):
        _require(intent[field] == notification[field])
    _require(notification["effect_id"] is None or intent["effect_id"] != notification["effect_id"])
    if intent["host_settlement"] != "completed" or intent["payload_disposition"] != "available":
        raise ResponseError("privacy-case-intent-unavailable")
    event = _descriptor(intent["descriptor"], "mas-privacy-case-intent/1")
    _require(intent["external_execution"] == "completed")
    _record(intent["output"], ("case_retained", "notification_pending"))
    _require(intent["output"]["case_retained"] is True and intent["output"]["notification_pending"] is True)
    _evidence(intent, "host-case-intent")
    identity = Notification.for_invocation(intent["parent_invocation"], event)
    result = {"version": "mas-privacy-case-status/1", "case_id": identity.case_id,
              "event": event.safe_record(), "target_binding": intent["descriptor"]["target_binding"],
              "intent_effect_id": intent["effect_id"], "notification": {
                  "effect_id": notification["effect_id"], "record_state": notification["state"],
                  "host_settlement": notification["host_settlement"],
                  "external_execution": notification["external_execution"],
                  "payload_disposition": notification["payload_disposition"],
                  "binding": "unavailable", "action_id": None, "submission": "unknown",
                  "recipient_delivery": "unknown", "retry_after_seconds": None}}
    if notification["payload_disposition"] != "available":
        return result
    notified_event = _descriptor(notification["descriptor"], "mas-privacy-notification-submit/1")
    _require(notified_event == event and
             notification["descriptor"]["target_binding"] == intent["descriptor"]["target_binding"])
    result["notification"]["binding"] = "matched"
    result["notification"]["action_id"] = identity.action_id
    if notification["host_settlement"] != "completed":
        return result
    _evidence(notification, "notification-endpoint-submission")
    output = notification["output"]
    _record(output, ("submission", "recipient_delivery", "retry_after_seconds", "response_metadata"))
    _choice(output["submission"], ("accepted", "refused", "throttled", "not-sent"))
    _require(output["recipient_delivery"] == "unknown")
    after = output["retry_after_seconds"]
    _require(type(after) is int and 0 <= after <= 3600)
    _choice(output["response_metadata"], ("available", "withheld"))
    certainty = {"accepted": "completed", "throttled": "refused", "refused": "refused", "not-sent": "not-sent"}
    # A local routing/concurrency refusal can be proved unsent. Both facts stay
    # distinct from an endpoint's known refusal and from an uncertain outcome.
    expected = certainty[output["submission"]]
    _require(notification["external_execution"] == expected or
             output["submission"] == "refused" and notification["external_execution"] == "not-sent")
    _require(output["submission"] == "throttled" or after == 0)
    result["notification"]["submission"] = output["submission"]
    result["notification"]["retry_after_seconds"] = after
    return result
