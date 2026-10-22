"""Check request identities against independent synthetic source histories."""

from __future__ import annotations

import collections


def check_history(offered: list[dict], records: list[dict], source: list[dict]) -> dict:
    """Keep source execution truth separate from retained host settlement.

    The fixture emits source events before returning its effect result. A source
    completion paired with an unknown host result is valid after a lost reply.
    Repeated source starts establish duplicate execution even if totals match.
    Missing or malformed events make coverage incomplete instead of proving
    non-execution. This checker never grants recovery or execution permission.
    """
    errors: list[str] = []
    gaps: list[str] = []
    ids = [offer["request_id"] for offer in offered]
    if len(set(ids)) != len(ids):
        errors.append("duplicate_offered_identity")
    results = collections.defaultdict(list)
    events = collections.defaultdict(list)
    for record in records:
        results[record.get("request_id")].append(record)
    for event in source:
        events[event.get("request_id")].append(event)
    if set(results) - set(ids):
        errors.append("unoffered_result_identity")
    if set(events) - set(ids):
        errors.append("unoffered_source_identity")
    uncertain_completed = 0
    for offer in offered:
        request = offer["request_id"]
        matching = results[request]
        if len(matching) != 1:
            errors.append(f"result_identity_count:{request}:{len(matching)}")
            continue
        result = matching[0]
        history = events[request]
        starts = [event for event in history if event.get("kind") == "started"]
        completions = [event for event in history if event.get("kind") == "completed"]
        if any(event.get("kind") not in ("started", "completed") for event in history):
            gaps.append(f"unknown_source_event:{request}")
        if len(starts) > 1 or len(completions) > 1:
            errors.append(f"duplicate_source_execution:{request}")
        if completions and not starts:
            gaps.append(f"source_start_missing:{request}")
        if len(starts) == len(completions) == 1:
            start, complete = starts[0], completions[0]
            if (not isinstance(start.get("effect_id"), str)
                    or not start["effect_id"]
                    or complete.get("effect_id") != start["effect_id"]):
                errors.append(f"source_effect_identity_mismatch:{request}")
            if (type(start.get("at_ms")) is not int or type(complete.get("at_ms")) is not int
                    or complete["at_ms"] < start["at_ms"]):
                errors.append(f"source_order_invalid:{request}")
            if result.get("execution") == "completed":
                if result.get("effect_id") != complete.get("effect_id"):
                    errors.append(f"host_source_effect_mismatch:{request}")
            else:
                uncertain_completed += 1
            if ("retained_effect_id" in result
                    and result["retained_effect_id"] != complete.get("effect_id")):
                errors.append(f"retained_source_effect_mismatch:{request}")
        mode = offer.get("fault_mode", "success")
        if mode == "refuse-before-effect":
            if history:
                errors.append(f"refused_source_dispatched:{request}")
            if result.get("execution") == "completed":
                errors.append(f"refused_host_completed:{request}")
            elif result.get("execution") != "failed":
                gaps.append(f"refusal_not_observed:{request}")
        elif not (len(starts) == len(completions) == 1):
            gaps.append(f"source_coverage_incomplete:{request}")
        if mode == "success" and result.get("execution") != "completed":
            gaps.append(f"host_completion_unavailable:{request}")
        if (mode == "success" and "application_check" in result
                and result["application_check"] != "pass"):
            errors.append(f"application_outcome_mismatch:{request}")
        if mode == "lost-completion" and result.get("execution") == "completed":
            errors.append(f"lost_completion_claimed_completed:{request}")
        if (mode == "lost-completion" and "retained_effect_state" in result
                and result["retained_effect_state"] != "unknown"):
            gaps.append(f"uncertain_effect_not_observed:{request}")
        if result.get("replay_source_unchanged") is False:
            errors.append(f"replay_redispatched:{request}")
        if mode == "lost-completion" and result.get("replay_source_unchanged") is not True:
            gaps.append(f"replay_source_check_unavailable:{request}")
    return {"status": "FAIL" if errors else "INCONCLUSIVE" if gaps else "PASS",
            "errors": errors, "coverage_gaps": gaps,
            "offered_identities": len(ids), "result_identities": len(results),
            "source_completed_with_host_unsettled": uncertain_completed}
