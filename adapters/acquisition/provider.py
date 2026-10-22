"""Bounded provider observations and conservative scheduling decisions.

The registered collector owns endpoint authentication and quota-scope identity.
These functions receive established transport facts, suppress response bodies
and arbitrary headers, and grant no execution permission or provider credit.
"""

from __future__ import annotations

from datetime import timezone
from email.utils import parsedate_to_datetime
import hashlib
import json
import re


MAX_TIMESTAMP_MS = 253402300799999
MAX_HINT_SECONDS = 315360000
OPERATIONS = {"read", "list", "watch", "mutation"}
TRANSPORTS = {"response", "not-dispatched", "outcome-unknown"}
_HEADER_NAMES = {"retry-after", "x-ratelimit-reset", "x-ratelimit-remaining"}
_FIELDS = {"version", "provider", "quota_scope", "operation", "transport", "http_status",
           "response_class", "mutation_disposition", "retry_not_before_ms", "retry_hint_state",
           "retry_after_not_before_ms", "rate_limit_reset_ms",
           "throttle_kind", "source_time_ms", "history_coverage", "watch_cursor_sha256",
           "relist_required", "body_retained"}


def _require(condition: bool) -> None:
    if not condition:
        raise ValueError("provider-contract-rejected")


def _timestamp(value: object) -> int:
    _require(type(value) is int and 0 <= value <= MAX_TIMESTAMP_MS)
    return value


def _scope(value: object) -> str:
    _require(type(value) is str and len(value) <= 160
             and re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._:/-]*", value) is not None)
    return value


def _number(value: str, maximum: int) -> int:
    _require(bool(re.fullmatch(r"[0-9]{1,12}", value)))
    parsed = int(value)
    _require(parsed <= maximum)
    return parsed


def _headers(values: object) -> tuple[dict[str, str], bool]:
    """Copy only approved scheduling fields, with no arbitrary header output."""
    _require(type(values) in (list, tuple) and len(values) <= 64)
    selected: dict[str, str] = {}
    size = 0
    invalid = False
    for pair in values:
        _require(type(pair) in (list, tuple) and len(pair) == 2)
        name, value = pair
        _require(type(name) is str and type(value) is str)
        try:
            size += len(name.encode("utf-8")) + len(value.encode("utf-8"))
        except UnicodeError:
            raise ValueError("provider-contract-rejected") from None
        _require(size <= 16384 and len(name) <= 128 and len(value) <= 4096)
        if (not re.fullmatch(r"[!#$%&'*+.^_`|~0-9A-Za-z-]+", name)
                or any(ord(character) < 32 or ord(character) > 126 for character in value)):
            invalid = True
            continue
        name = name.lower()
        if name in _HEADER_NAMES:
            if name in selected:
                invalid = True
            else:
                selected[name] = value.strip()
    return selected, invalid


def _retry_after(value: str, now_ms: int) -> int:
    if re.fullmatch(r"[0-9]+", value):
        result = now_ms + _number(value, MAX_HINT_SECONDS) * 1000
    else:
        try:
            date = parsedate_to_datetime(value)
            _require(date is not None and date.tzinfo is not None)
            result = int(date.astimezone(timezone.utc).timestamp() * 1000)
        except (TypeError, ValueError, OverflowError):
            raise ValueError("provider-contract-rejected") from None
    return max(now_ms, _timestamp(result))


def observe(provider: str, quota_scope: str, operation: str, *, now_ms: int,
            http_status: int | None = None, headers: list | tuple = (),
            transport: str = "response", watch_cursor: str | None = None) -> dict:
    """Project one response or established transport failure into safe metadata.

    `not-dispatched` requires evidence that request transmission never began.
    A timeout, connection loss after sending, or unclassified transport failure
    is `outcome-unknown`. The scope comes from registration, never a token or an
    untrusted response header. List/watch 410 records a lost interval; a later
    successful snapshot establishes current state without repairing that history.
    """
    _require(provider in {"github", "kubernetes", "a2a"} and operation in OPERATIONS
             and transport in TRANSPORTS)
    now_ms = _timestamp(now_ms)
    quota_scope = _scope(quota_scope)
    _require(watch_cursor is None or (type(watch_cursor) is str and 0 < len(watch_cursor) <= 512))
    _require(watch_cursor is None or operation in {"list", "watch"})
    try:
        cursor_digest = hashlib.sha256(watch_cursor.encode()).hexdigest() if watch_cursor is not None else None
    except UnicodeError:
        raise ValueError("provider-contract-rejected") from None
    _require(http_status is None or (type(http_status) is int and 100 <= http_status <= 599))
    _require((transport == "response") == (http_status is not None))
    _require(transport == "response" or not headers)
    selected, invalid = _headers(headers)
    retry = reset = remaining = None
    hint_state = "absent"
    try:
        if "retry-after" in selected:
            retry = _retry_after(selected["retry-after"], now_ms)
            hint_state = "valid"
        if provider == "github" and "x-ratelimit-reset" in selected:
            reset = _number(selected["x-ratelimit-reset"], MAX_TIMESTAMP_MS // 1000) * 1000
        if provider == "github" and "x-ratelimit-remaining" in selected:
            remaining = _number(selected["x-ratelimit-remaining"], 1_000_000_000)
    except ValueError:
        invalid = True
    retry_after = retry
    response = "unknown"
    throttle = "none"
    mutation = "not-applicable" if operation != "mutation" else "unknown"
    relist = False
    if transport == "not-dispatched":
        response = "not-dispatched"
        mutation = "not-sent" if operation == "mutation" else "not-applicable"
    elif transport == "outcome-unknown":
        response = "transport-unknown"
    elif (http_status == 429 or (provider == "github" and http_status == 403
                               and ("retry-after" in selected or remaining == 0))):
        response = "rate-limited"
        throttle = ("provider" if provider != "github" else
                    "primary" if remaining == 0 else "secondary-or-unspecified")
        # Generic host HTTP admission is distinct from the upstream provider's
        # documented throttle refusal. Reconcile an A2A mutation by its original
        # idempotent protocol identity; this observation cannot authorise retry.
        if operation == "mutation" and provider != "a2a":
            mutation = "known-throttle-refusal"
        if throttle == "primary":
            if reset is None:
                invalid = True
            else:
                retry = max(now_ms, retry or now_ms, reset)
                hint_state = "valid"
    elif provider == "kubernetes" and http_status == 410 and operation in {"list", "watch"}:
        response = "expired-history"
        relist = True
    elif http_status == 304 and operation != "mutation":
        response = "not-modified"
    elif http_status is not None and 200 <= http_status <= 299:
        response = "accepted"
        if operation == "mutation":
            mutation = "accepted"
    elif http_status is not None and 300 <= http_status <= 399:
        response = "redirect-denied"
    elif http_status is not None and 400 <= http_status <= 499:
        response = "http-refusal"
    elif http_status is not None and 500 <= http_status <= 599:
        response = "server-error"
    if invalid:
        hint_state = "invalid"
    result = {"version": "mas-provider-observation/1", "provider": provider,
              "quota_scope": quota_scope, "operation": operation, "transport": transport,
              "http_status": http_status, "response_class": response,
              "mutation_disposition": mutation, "retry_not_before_ms": retry,
              "retry_after_not_before_ms": retry_after, "rate_limit_reset_ms": reset,
              "retry_hint_state": hint_state, "throttle_kind": throttle,
              "source_time_ms": now_ms,
              "history_coverage": "lost-interval" if relist else "point-response",
              "watch_cursor_sha256": cursor_digest,
              "relist_required": relist, "body_retained": False}
    _require(len(json.dumps(result, separators=(",", ":")).encode()) <= 2048)
    return result


def schedule(observation: dict, *, now_ms: int, deadline_ms: int, attempt: int = 0,
             jitter_ms: int = 0, max_delay_ms: int = 3600000,
             max_attempts: int = 8) -> dict:
    """Choose a bounded next step. Every retry still needs current admission.

    Provider hints establish a lower bound. A hint beyond the permitted waiting
    budget causes HOLD or expiry rather than an earlier capped retry. Jitter is
    supplied explicitly for reproducible tests and can only increase that bound.
    Provider allowance reservation and durable cooldown belong to the existing
    authority owner; this decision supplies neither one.
    """
    _require(type(observation) is dict and set(observation) == _FIELDS
             and observation["version"] == "mas-provider-observation/1"
             and observation["provider"] in {"github", "kubernetes", "a2a"}
             and observation["operation"] in OPERATIONS
             and observation["transport"] in TRANSPORTS
             and observation["body_retained"] is False)
    _scope(observation["quota_scope"])
    now_ms = _timestamp(now_ms)
    deadline_ms = _timestamp(deadline_ms)
    _require(type(attempt) is int and 0 <= attempt <= 64
             and type(jitter_ms) is int and 0 <= jitter_ms <= 1000
             and type(max_delay_ms) is int and 1 <= max_delay_ms <= 3600000
             and type(max_attempts) is int and 1 <= max_attempts <= 16)
    source_time = _timestamp(observation["source_time_ms"])
    _require(source_time <= now_ms)
    hint = observation["retry_not_before_ms"]
    _require(hint is None or type(hint) is int)
    if hint is not None:
        _timestamp(hint)
    for name in ("retry_after_not_before_ms", "rate_limit_reset_ms"):
        if observation[name] is not None:
            _timestamp(observation[name])
    _require(observation["rate_limit_reset_ms"] is None or observation["provider"] == "github")
    _require(observation["retry_after_not_before_ms"] is None
             or (hint is not None and hint >= observation["retry_after_not_before_ms"]))
    _require(observation["retry_hint_state"] in {"absent", "valid", "invalid"}
             and observation["throttle_kind"] in {"none", "primary", "secondary-or-unspecified", "provider"}
             and observation["mutation_disposition"] in {"not-applicable", "not-sent", "unknown", "accepted", "known-throttle-refusal"}
             and observation["response_class"] in {"unknown", "not-dispatched", "transport-unknown", "rate-limited",
                 "expired-history", "not-modified", "accepted", "redirect-denied", "http-refusal", "server-error"}
             and type(observation["relist_required"]) is bool
             and observation["history_coverage"] in {"point-response", "lost-interval"})
    operation, transport = observation["operation"], observation["transport"]
    response, disposition = observation["response_class"], observation["mutation_disposition"]
    status = observation["http_status"]
    _require((transport == "response") == (type(status) is int and 100 <= status <= 599))
    _require(transport == "response" or status is None)
    _require(transport != "outcome-unknown" or response == "transport-unknown")
    _require(transport != "not-dispatched" or response == "not-dispatched")
    _require(operation == "mutation" or disposition == "not-applicable")
    if operation == "mutation":
        _require(disposition != "not-applicable")
        _require((disposition != "not-sent" or transport == "not-dispatched")
                 and (transport != "not-dispatched" or disposition == "not-sent"))
        _require(disposition != "accepted" or (response == "accepted" and transport == "response" and 200 <= status <= 299))
        _require(disposition != "known-throttle-refusal" or (response == "rate-limited"
            and observation["provider"] != "a2a" and transport == "response"
            and (status == 429 or (observation["provider"] == "github" and status == 403))))
        _require(transport != "outcome-unknown" or disposition == "unknown")
    _require((response != "accepted" or (transport == "response" and 200 <= status <= 299))
             and (response != "not-modified" or (transport == "response" and status == 304 and operation != "mutation")))
    if response == "rate-limited":
        _require(transport == "response" and (status == 429 or (observation["provider"] == "github" and status == 403)))
        _require(observation["throttle_kind"] in ({"primary", "secondary-or-unspecified"}
            if observation["provider"] == "github" else {"provider"}))
    else:
        _require(observation["throttle_kind"] == "none")
    relist = (response == "expired-history")
    _require(observation["relist_required"] == relist
             and (observation["history_coverage"] == "lost-interval") == relist)
    _require(not relist or (observation["provider"] == "kubernetes" and status == 410 and operation in {"list", "watch"}))
    cursor = observation["watch_cursor_sha256"]
    _require(cursor is None or (type(cursor) is str and re.fullmatch(r"[0-9a-f]{64}", cursor) is not None))
    _require(observation["retry_hint_state"] != "valid" or hint is not None)
    mutation_unknown = (observation["operation"] == "mutation"
                        and observation["mutation_disposition"] == "unknown")
    result = {"version": "mas-provider-schedule/1", "quota_scope": observation["quota_scope"],
              "action": "hold", "reason": "unsupported-response", "not_before_ms": None,
              "requires_fresh_admission": False, "execution_permission": False,
              "reconciliation_required": mutation_unknown or disposition == "accepted",
              "deadline_expired": now_ms >= deadline_ms,
              "coverage_gap": observation["history_coverage"] == "lost-interval",
              "new_snapshot_repairs_history": False}
    if mutation_unknown:
        result.update(action="reconcile", reason="mutation-execution-unknown")
        return result
    if now_ms >= deadline_ms:
        result.update(action="expired", reason="original-deadline")
        return result
    response = observation["response_class"]
    if response in {"accepted", "not-modified"}:
        result.update(action="finish", reason="http-response-accepted",
                      reconciliation_required=observation["mutation_disposition"] == "accepted")
        return result
    if response in {"redirect-denied", "http-refusal"}:
        result.update(action="refused", reason=response)
        return result
    if observation["retry_hint_state"] == "invalid":
        result.update(reason="retry-metadata-invalid")
        return result
    if response == "expired-history" and observation["relist_required"]:
        not_before = max(now_ms, max(source_time, hint or source_time) + jitter_ms)
        action, reason = "relist", "snapshot-required-after-lost-interval"
    elif response in {"rate-limited", "transport-unknown", "server-error", "not-dispatched"}:
        if attempt >= max_attempts:
            result.update(reason="retry-attempt-budget")
            return result
        base = 60000 if (observation["provider"] == "github"
                         and observation["throttle_kind"] == "secondary-or-unspecified") else 1000
        not_before = max(now_ms, max(source_time + base * (2 ** attempt),
                                    hint or source_time) + jitter_ms)
        action, reason = "defer", "provider-wait" if response == "rate-limited" else "bounded-retry"
    else:
        return result
    if not_before >= deadline_ms:
        result.update(action="expired", reason="retry-would-exceed-original-deadline")
    elif not_before - now_ms > max_delay_ms:
        result.update(reason="provider-wait-exceeds-local-budget")
    else:
        result.update(action=action, reason=reason, not_before_ms=not_before,
                      requires_fresh_admission=True)
    return result
