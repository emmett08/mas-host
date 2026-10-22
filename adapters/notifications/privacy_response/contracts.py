"""Construct bounded case metadata and fixed notifications from trusted handling events."""
from __future__ import annotations

from dataclasses import dataclass
import hashlib
import json
import re


class ResponseError(ValueError):
    """An invalid response contract; messages contain no supplied content."""


CATEGORIES = frozenset(("name", "address", "email", "phone", "account", "identifier",
                        "credential", "location"))
REASONS = frozenset(("handling-withheld", "coverage-failure", "prohibited-finding",
                     "suspected-exposure"))
ROUTES = frozenset(("data-owner", "security-response", "service-operations"))


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":"),
                      ensure_ascii=False, allow_nan=False).encode("utf-8")


def _require(value):
    if not value:
        raise ResponseError("privacy-response-contract-invalid")


def _identity(kind, *values):
    return kind + "-" + hashlib.sha256(canonical([kind + "/1", *values])).hexdigest()


@dataclass(frozen=True)
class CaseEvent:
    """A trusted handling outcome, excluding text, spans and arbitrary metadata.

    Construction does not authenticate or authorise the event. The trusted
    application must obtain it from its handling contract. The host separately
    admits every case/notification operation under its configured scope.
    """
    reason: str
    disposition: str
    exposure: str
    event_version: int = 1
    categories: tuple[str, ...] = ()
    finding_count: int = 0

    def __post_init__(self):
        _require(type(self.reason) is str and self.reason in REASONS)
        _require(type(self.disposition) is str and self.disposition in ("withheld", "hold", "rejected"))
        _require(type(self.exposure) is str and self.exposure in ("blocked", "possible", "confirmed"))
        _require(type(self.event_version) is int and 1 <= self.event_version <= 1_000_000)
        _require(type(self.finding_count) is int and 0 <= self.finding_count <= 128)
        _require(type(self.categories) is tuple and len(self.categories) <= 8)
        _require(all(type(value) is str and value in CATEGORIES for value in self.categories))
        _require(tuple(sorted(set(self.categories))) == self.categories)
        _require(bool(self.categories) == bool(self.finding_count))
        _require(self.reason != "prohibited-finding" or self.finding_count > 0)
        _require(self.reason != "suspected-exposure" or self.exposure != "blocked")

    @property
    def route(self):
        if self.reason == "coverage-failure":
            return "service-operations"
        if self.reason == "suspected-exposure" or "credential" in self.categories:
            return "security-response"
        return "data-owner"

    def safe_record(self):
        return {"version": "mas-privacy-case-event/1", "event_version": self.event_version,
                "reason": self.reason, "disposition": self.disposition,
                "exposure": self.exposure, "categories": list(self.categories),
                "finding_count": self.finding_count, "route": self.route}


@dataclass(frozen=True)
class Notification:
    """A fixed projection whose identities derive from a host invocation.

    Host invocation identities include the journal's random nonce and retained
    operation identity. Replacing this value with a supplied request identifier
    would invalidate the scope-separation contract.
    """
    case_id: str
    action_id: str
    event: CaseEvent
    kind: str = "review"

    def __post_init__(self):
        _require(type(self.event) is CaseEvent)
        _require(type(self.kind) is str and self.kind in ("review", "escalation"))
        _require(type(self.case_id) is str and
                 re.fullmatch(r"privacy-case-[0-9a-f]{64}", self.case_id) is not None)
        _require(type(self.action_id) is str and
                 re.fullmatch(r"privacy-notification-[0-9a-f]{64}", self.action_id) is not None)
        _require(self.action_id == _identity("privacy-notification", self.case_id,
                                            self.event.event_version, self.event.route, *(() if self.kind == "review" else ("escalation",))))

    @classmethod
    def for_invocation(cls, invocation_id, event, *, kind="review"):
        _require(type(invocation_id) is str and
                 re.fullmatch(r"task-[0-9a-f]{64}", invocation_id) is not None)
        _require(type(event) is CaseEvent)
        case_id = _identity("privacy-case", invocation_id)
        _require(type(kind) is str and kind in ("review", "escalation"))
        action_id = _identity("privacy-notification", case_id, event.event_version, event.route,
                              *(() if kind == "review" else ("escalation",)))
        return cls(case_id, action_id, event, kind)

    def payload(self):
        # No supplied endpoint, title, prose, source reference or detected value
        # can enter this projection. A case link is deferred until its resolver
        # has a separately qualified authorisation/existence-metadata contract.
        return {"template": "privacy-handling-review/1" if self.kind == "review" else "privacy-handling-escalation/1",
                "case_id": self.case_id, "action_id": self.action_id,
                **self.event.safe_record(), "version": "mas-privacy-notification/1"}

    def encoded(self):
        raw = canonical(self.payload())
        _require(len(raw) <= 2048)
        return raw
