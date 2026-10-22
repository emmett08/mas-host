"""Exact registered read sharing through the existing governed A2A service.

The host owns task admission, original-message deduplication and effect history.
The application owns durable registration/checkpoint and subscription lifecycle.
This module neither schedules workers nor stores credentials or provider bodies.
"""

from __future__ import annotations

from dataclasses import dataclass, field
import hashlib
import hmac
import http.client
import ipaddress
import json
import re
import socket
import ssl
import threading
import time
from urllib.parse import urlsplit

try:
    from . import provider
except ImportError:  # Direct use from this adapter directory.
    import provider


BASE = "urn:mas:host-task-service:1"
PUBLICATION = "urn:mas:host-governed-source-publication:1"
SHARED_READ = "urn:mas:host-governed-shared-source-read:1"
MAX_FRAME = 65_536
_HASH = re.compile(r"[0-9a-f]{64}")
_TASK = re.compile(r"a2a-[0-9a-f]{56}")
_REFERENCE = re.compile(r"shared-[0-9a-f]{64}")
_CONTRACT = {"version", "authority", "tenant", "acquisition_grant", "purpose",
             "credential_epoch", "policy_epoch", "source", "query", "generation",
             "projection", "schema_version", "transform_version", "agent",
             "descriptor_digest", "effect_key", "submission_deadline_ms",
             "transport_quota_scope"}


def _require(condition: bool) -> None:
    if not condition:
        raise ValueError("acquisition-contract-rejected")


def _identity(value: object, maximum: int = 128) -> str:
    _require(type(value) is str and 0 < len(value) <= maximum
             and re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._:/-]*", value) is not None)
    return value


def _timestamp(value: object) -> int:
    _require(type(value) is int and 0 <= value <= provider.MAX_TIMESTAMP_MS)
    return value


def _shape(value: object, depth: int = 0) -> None:
    _require(depth <= 24)
    if type(value) is dict:
        _require(len(value) <= 256)
        for key, child in value.items():
            _require(type(key) is str and len(key) <= 512)
            _shape(child, depth + 1)
    elif type(value) is list:
        _require(len(value) <= 4096)
        for child in value:
            _shape(child, depth + 1)
    elif type(value) is int:
        _require(-(2 ** 63) <= value < 2 ** 63)
    else:
        _require(value is None or type(value) in (str, bool))


def _canonical(value: object) -> bytes:
    try:
        _shape(value)
        encoded = json.dumps(value, sort_keys=True, separators=(",", ":"),
                             ensure_ascii=False, allow_nan=False).encode("utf-8")
        _require(len(encoded) <= MAX_FRAME)
        return encoded
    except (TypeError, UnicodeError, ValueError, RecursionError):
        raise ValueError("acquisition-contract-rejected") from None


def _decode(raw: bytes) -> object:
    def pairs(values):
        result = {}
        for key, value in values:
            _require(key not in result)
            result[key] = value
        return result
    try:
        _require(len(raw) <= MAX_FRAME)
        value = json.loads(raw, object_pairs_hook=pairs,
                           parse_constant=lambda _: _require(False))
        _shape(value)
        return value
    except (TypeError, UnicodeError, ValueError, RecursionError):
        raise ValueError("acquisition-contract-rejected") from None


def _hash(value: object) -> str:
    return hashlib.sha256(_canonical(value)).hexdigest()


def _clock_ms() -> int:
    return time.time_ns() // 1_000_000


class RegisteredRead:
    """An operator-registered immutable exact acquisition, including generation.

    Contract values and input must already have passed the application's input,
    authorisation and privacy checks. Hashing never authenticates these values.
    `authority` is bound to a registered endpoint by trusted application config.
    """

    def __init__(self, contract: dict, input_value: object, *, identity_key: bytes):
        _require(type(contract) is dict and set(contract) == _CONTRACT
                 and contract["version"] == "mas-coalesced-read/1")
        for name in _CONTRACT - {"version", "query", "submission_deadline_ms", "descriptor_digest"}:
            _identity(contract[name], 160 if name == "transport_quota_scope" else 128)
        _require(type(contract["descriptor_digest"]) is str
                 and _HASH.fullmatch(contract["descriptor_digest"]) is not None)
        _timestamp(contract["submission_deadline_ms"])
        _require(type(contract["query"]) is dict)
        # A distinct private application identity key prevents publication of
        # dictionary-testable commitments to predictable restricted inputs.
        # It is not a bearer, credential epoch or a public salt.
        _require(type(identity_key) is bytes and 32 <= len(identity_key) <= 64)
        # Snapshot both values: a caller mutating input afterwards cannot change
        # a replay's parameters while retaining the original message identity.
        self._contract = _canonical(contract)
        self._input = _canonical(input_value)
        identity = _canonical({"contract": _decode(self._contract),
                              "input_sha256": hashlib.sha256(self._input).hexdigest()})
        self._query_id = hmac.new(identity_key, b"mas-coalesced-read-identity/1\x00" + identity,
                                  hashlib.sha256).hexdigest()
        self._original_message_id = "coalesced-" + self.query_id
        # An acquisition-specific immutable alias, never a mutable 'latest' key.
        self._publication_alias = _hash(["mas-coalesced-publication/1", self.query_id,
                                        self.original_message_id])

    @property
    def query_id(self):
        return self._query_id

    @property
    def original_message_id(self):
        return self._original_message_id

    @property
    def publication_alias(self):
        return self._publication_alias

    def __repr__(self):
        return f"RegisteredRead(query_id={self.query_id!r})"

    def contract(self) -> dict:
        return _decode(self._contract)

    def submission(self) -> dict:
        contract = self.contract()
        return {"message": {"messageId": self.original_message_id, "role": "ROLE_USER",
                "parts": [{"data": {"agent": contract["agent"],
                    "descriptorDigest": contract["descriptor_digest"],
                    "input": _decode(self._input)}, "mediaType": "application/json"}]},
                "configuration": {"returnImmediately": True}}


@dataclass(repr=False)
class Reply:
    """Private RPC value plus bounded safe scheduling metadata."""
    state: str
    value: object = field(default=None, repr=False)
    observation: dict | None = None
    decision: dict | None = None

    def __repr__(self):
        return f"Reply(state={self.state!r})"

    def safe(self) -> dict:
        return {"state": self.state, "observation": self.observation,
                "decision": self.decision}


class HTTPRPC:
    """One bounded request; TLS verification, no proxies or redirects/retries.

    `bearer` is a callable resolving this caller's current credential per request.
    HTTP error bodies, arbitrary response headers and error text are discarded.
    Loopback HTTP is opt-in for local governed-host fixtures/deployments only.
    """

    def __init__(self, endpoint: str, authority: str, quota_scope: str, bearer, *,
                 allow_loopback_http: bool = False, timeout_ms: int = 5000, clock=_clock_ms):
        self.authority = _identity(authority)
        self.quota_scope = _identity(quota_scope, 160)
        _require(callable(bearer) and type(timeout_ms) is int and 1 <= timeout_ms <= 10000)
        try:
            parsed = urlsplit(endpoint)
            _require(parsed.scheme in {"https", "http"} and parsed.hostname is not None
                     and parsed.username is None and parsed.password is None
                     and not parsed.query and not parsed.fragment
                     and not any(ord(c) <= 32 or ord(c) >= 127 for c in endpoint))
            if parsed.scheme == "http":
                _require(allow_loopback_http and ipaddress.ip_address(parsed.hostname).is_loopback)
            self._port = parsed.port
        except (TypeError, ValueError):
            raise ValueError("acquisition-contract-rejected") from None
        self._scheme, self._host = parsed.scheme, parsed.hostname
        self._path = parsed.path or "/"
        self._bearer, self._timeout_ms, self._clock = bearer, timeout_ms, clock
        self._lock = threading.Lock()
        self._read_wait = None
        self._read_attempt = 0

    def __repr__(self):
        return f"HTTPRPC(authority={self.authority!r})"

    def call(self, method: str, params: dict, extensions: tuple, request_id: str, *,
             now_ms: int, deadline_ms: int) -> Reply:
        if not self._lock.acquire(blocking=False):
            return Reply("client-busy")
        try:
            return self._call(method, params, extensions, request_id,
                              now_ms=now_ms, deadline_ms=deadline_ms)
        finally:
            self._lock.release()

    def _call(self, method, params, extensions, request_id, *, now_ms, deadline_ms):
        operations = {"SendMessage": "mutation", "GetTaskStatus": "read",
                      "PublishRetainedSource": "mutation", "GetSharedSource": "read"}
        profiles = {"SendMessage": (BASE,), "GetTaskStatus": (BASE,),
                    "PublishRetainedSource": (BASE, PUBLICATION),
                    "GetSharedSource": (BASE, SHARED_READ)}
        _require(method in operations and extensions == profiles[method])
        _identity(request_id)
        now_ms, deadline_ms = _timestamp(now_ms), _timestamp(deadline_ms)
        now_ms = max(now_ms, _timestamp(self._clock()))
        if now_ms >= deadline_ms:
            return Reply("deadline-expired")
        if operations[method] == "read" and self._read_wait is not None:
            decision = provider.schedule(self._read_wait, now_ms=now_ms, deadline_ms=deadline_ms,
                                         attempt=self._read_attempt)
            if decision["action"] != "defer" or decision["not_before_ms"] > now_ms:
                return Reply("locally-" + decision["action"], observation=self._read_wait,
                             decision=decision)
        body = _canonical({"jsonrpc": "2.0", "id": request_id, "method": method, "params": params})
        connection = None
        watchdog = None
        observation = None
        try:
            token = self._bearer()
            _require(type(token) is str and 1 <= len(token) <= 4096
                     and all(33 <= ord(c) <= 126 for c in token))
            timeout = min(self._timeout_ms, deadline_ms - now_ms) / 1000
            if self._scheme == "https":
                connection = http.client.HTTPSConnection(self._host, self._port, timeout=timeout,
                                                        context=ssl.create_default_context())
            else:
                connection = http.client.HTTPConnection(self._host, self._port, timeout=timeout)
            # After bounded connection establishment, an absolute socket cutoff
            # also stops a peer drip-feeding response headers/chunks indefinitely.
            started = time.monotonic()
            connection.connect()
            remaining = timeout - (time.monotonic() - started)
            if remaining <= 0:
                raise TimeoutError()
            active_socket = connection.sock
            def stop_io():
                try:
                    active_socket.shutdown(socket.SHUT_RDWR)
                except OSError:
                    pass
            watchdog = threading.Timer(remaining, stop_io)
            watchdog.daemon = True
            watchdog.start()
            connection.request("POST", self._path, body, {"Content-Type": "application/json",
                "Authorization": "Bearer " + token, "A2A-Version": "1.0",
                "A2A-Extensions": ", ".join(extensions)})
            del token
            response = connection.getresponse()
            observed_ms = max(now_ms, _timestamp(self._clock()))
            observation = provider.observe("a2a", self.quota_scope, operations[method],
                now_ms=observed_ms, http_status=response.status, headers=response.getheaders())
            decision = provider.schedule(observation, now_ms=observed_ms, deadline_ms=deadline_ms)
            if response.status != 200:
                self._retain_read_wait(operations[method], observation)
                return Reply("http-" + decision["action"], observation=observation, decision=decision)
            # Read one bounded successful RPC envelope; raw text is never logged.
            raw = response.read(MAX_FRAME + 1)
            value = _decode(raw)
            _require(type(value) is dict and value.get("jsonrpc") == "2.0"
                     and value.get("id") == request_id and set(value) in
                     ({"jsonrpc", "id", "result"}, {"jsonrpc", "id", "error"}))
            if _timestamp(self._clock()) >= deadline_ms:
                return Reply("deadline-expired", observation=observation, decision=decision)
            if "error" in value:
                # Includes not-yet-materialised status. Neither message nor data
                # nor a response body crosses this client's error boundary.
                return Reply("rpc-unavailable", observation=observation, decision=decision)
            if operations[method] == "read":
                self._read_wait, self._read_attempt = None, 0
            return Reply("ok", value["result"], observation, decision)
        except Exception:
            observed_ms = max(now_ms, _timestamp(self._clock()))
            unknown = provider.observe("a2a", self.quota_scope, operations[method],
                now_ms=observed_ms, transport="outcome-unknown")
            decision = provider.schedule(unknown, now_ms=observed_ms, deadline_ms=deadline_ms)
            self._retain_read_wait(operations[method], unknown)
            return Reply("transport-unknown", observation=unknown, decision=decision)
        finally:
            if watchdog is not None:
                watchdog.cancel()
            if connection is not None:
                connection.close()

    def _retain_read_wait(self, operation, observation):
        if operation == "read" and observation["response_class"] in {
                "rate-limited", "transport-unknown", "server-error", "not-dispatched"}:
            if self._read_wait is not None:
                self._read_attempt = min(self._read_attempt + 1, 8)
            self._read_wait = observation


@dataclass(frozen=True)
class Publication:
    authority: str
    query_id: str
    reference: str


class Acquisition:
    """Producer-side progression over one original idempotent host submission.

    A trusted application must durably persist registration and checkpoint before
    relying on crash recovery. A checkpoint is private producer state, never a
    subscriber capability. It cannot grant authority at the host.
    """

    def __init__(self, registration: RegisteredRead, producer_rpc, *, trusted_checkpoint=None):
        self.registration, self._rpc = registration, producer_rpc
        _require(producer_rpc.authority == registration.contract()["authority"])
        self._task_id = self._reference = None
        self._lock = threading.Lock()
        if trusted_checkpoint is not None:
            state = _decode(_canonical(trusted_checkpoint))
            _require(type(state) is dict and set(state) == {"version", "query_id", "task_id", "reference"}
                     and state["version"] == "mas-coalesced-checkpoint/1"
                     and state["query_id"] == registration.query_id)
            _require(state["task_id"] is None or (type(state["task_id"]) is str
                     and _TASK.fullmatch(state["task_id"]) is not None))
            _require(state["reference"] is None or (state["task_id"] is not None
                     and type(state["reference"]) is str and _REFERENCE.fullmatch(state["reference"]) is not None))
            self._task_id, self._reference = state["task_id"], state["reference"]

    def checkpoint(self) -> dict:
        with self._lock:
            return {"version": "mas-coalesced-checkpoint/1", "query_id": self.registration.query_id,
                    "task_id": self._task_id, "reference": self._reference}

    def publication(self) -> Publication | None:
        with self._lock:
            return self._publication()

    def _publication(self):
        if self._reference is None:
            return None
        return Publication(self.registration.contract()["authority"], self.registration.query_id,
                           self._reference)

    def advance(self, *, now_ms: int, resolution_deadline_ms: int) -> dict:
        now_ms, deadline_ms = _timestamp(now_ms), _timestamp(resolution_deadline_ms)
        with self._lock:
            registration, contract = self.registration, self.registration.contract()
            result = {"version": "mas-coalesced-progress/1", "query_id": registration.query_id,
                      "state": "pending", "execution_permission": False}
            if self._reference is not None:
                result.update(state="published", publication=self._publication())
                return result
            if now_ms >= deadline_ms:
                result.update(state="resolution-deadline-expired")
                return result
            if self._task_id is None:
                # A follower cannot extend the registered original admission
                # cutoff. Lost acknowledgements retain the same original ID.
                submission_deadline = min(deadline_ms, contract["submission_deadline_ms"])
                if now_ms >= submission_deadline:
                    result.update(state="submission-expired-unresolved")
                    return result
                reply = self._rpc.call("SendMessage", registration.submission(), (BASE,),
                    "submit-" + registration.query_id, now_ms=now_ms, deadline_ms=submission_deadline)
                if reply.state != "ok":
                    result.update(state="submission-unresolved", transport=reply.safe())
                    return result
                value = reply.value
                task_id = value.get("task", {}).get("id") if type(value) is dict and type(value.get("task")) is dict else None
                if type(task_id) is not str or _TASK.fullmatch(task_id) is None:
                    result.update(state="submission-unresolved")
                    return result
                self._task_id = task_id
                # A SendMessage replay may include cached producer artifacts.
                # Only its private task identity is retained here.
            reply = self._rpc.call("GetTaskStatus", {"id": self._task_id}, (BASE,),
                "status-" + registration.query_id, now_ms=now_ms, deadline_ms=deadline_ms)
            if reply.state != "ok":
                result.update(state="status-unresolved", transport=reply.safe())
                return result
            status = reply.value
            if (type(status) is not dict or status.get("version") != "mas-host-task-status/1"
                    or status.get("id") != self._task_id or status.get("agent") != contract["agent"]
                    or status.get("bundleDigest") != contract["descriptor_digest"]
                    or status.get("executionPermission") is not False):
                result.update(state="status-unresolved")
                return result
            if status.get("serviceState") != "completed" or status.get("hostSettlement") != "completed":
                state = status.get("serviceState")
                result.update(state="terminal-without-source" if state in {"failed", "unknown", "cancelled"} else "pending")
                return result
            reply = self._rpc.call("PublishRetainedSource", {"id": self._task_id,
                "key": contract["effect_key"], "reference": registration.publication_alias},
                (BASE, PUBLICATION), "publish-" + registration.query_id,
                now_ms=now_ms, deadline_ms=deadline_ms)
            value = reply.value
            if (reply.state != "ok" or type(value) is not dict
                    or set(value) != {"version", "reference", "status"}
                    or value.get("version") != "mas-governed-source-publication/1"
                    or value.get("status") != "published" or type(value.get("reference")) is not str
                    or _REFERENCE.fullmatch(value["reference"]) is None):
                result.update(state="publication-unresolved", transport=reply.safe())
                return result
            self._reference = value["reference"]
            result.update(state="published", publication=self._publication())
            return result


class Subscription:
    """One caller's bounded delivery, with independent bearer/deadline/cancel.

    The application must durably own logical subscription admission, identity,
    accounting and cancellation. This in-memory object supplies no durable lease.
    Cancellation never sends CancelTask or cancels another subscriber's read.
    """

    def __init__(self, request_id: str, registration: RegisteredRead, subscriber_rpc, *,
                 deadline_ms: int, min_acquired_at_ms: int = 0, max_age_ms: int | None = None,
                 clock=_clock_ms):
        self.request_id = _identity(request_id)
        self.registration, self._rpc = registration, subscriber_rpc
        _require(subscriber_rpc.authority == registration.contract()["authority"])
        self.deadline_ms, self.min_acquired_at_ms = _timestamp(deadline_ms), _timestamp(min_acquired_at_ms)
        _require(max_age_ms is None or (type(max_age_ms) is int and 0 <= max_age_ms <= 315360000000))
        self.max_age_ms, self._clock, self._cancelled = max_age_ms, clock, threading.Event()

    def cancel(self) -> None:
        self._cancelled.set()

    def deliver(self, publication: Publication, *, now_ms: int) -> dict:
        now_ms = _timestamp(now_ms)
        result = {"version": "mas-coalesced-delivery/1", "request_id": self.request_id,
                  "query_id": self.registration.query_id, "state": "unavailable",
                  "execution_permission": False, "fresh_acquisition_started": False}
        if self._cancelled.is_set():
            result.update(state="cancelled")
            return result
        if now_ms >= self.deadline_ms:
            result.update(state="deadline-expired")
            return result
        contract = self.registration.contract()
        _require(type(publication) is Publication and publication.authority == contract["authority"]
                 and publication.query_id == self.registration.query_id
                 and _REFERENCE.fullmatch(publication.reference) is not None)
        reply = self._rpc.call("GetSharedSource", {"reference": publication.reference},
            (BASE, SHARED_READ), self.request_id, now_ms=now_ms, deadline_ms=self.deadline_ms)
        # Delivery is independently withdrawable even while its HTTP read runs.
        delivered_ms = max(now_ms, _timestamp(self._clock()))
        if self._cancelled.is_set():
            result.update(state="cancelled")
            return result
        if delivered_ms >= self.deadline_ms:
            result.update(state="deadline-expired")
            return result
        if reply.state != "ok":
            result.update(transport=reply.safe())
            return result
        view = reply.value
        if (type(view) is not dict or view.get("version") not in {"mas-governed-source-view/1", "mas-governed-source-view/2"}
                or view.get("actor") != contract["agent"] or view.get("key") != contract["effect_key"]
                or view.get("host_settlement") != "completed" or "projection" not in view):
            return result
        if view.get("payload_disposition") != "available":
            result.update(state="withheld")
            return result
        metadata = view.get("source_metadata")
        acquired = metadata.get("acquired_at_ms") if type(metadata) is dict else None
        expires = metadata.get("expires_at_ms") if type(metadata) is dict else None
        if (type(view.get("source_id")) is not str or type(view.get("effect_id")) is not str
                or type(metadata) is not dict or metadata.get("id") != view["source_id"]
                or metadata.get("step_id") != view["effect_id"]
                or type(acquired) is not int or acquired < 0 or acquired > delivered_ms
                or (expires is not None and (type(expires) is not int or expires < acquired))):
            return result
        if (acquired < self.min_acquired_at_ms or (expires is not None and delivered_ms >= expires)
                or (self.max_age_ms is not None and delivered_ms - acquired > self.max_age_ms)):
            result.update(state="stale", acquired_at_ms=acquired, expires_at_ms=expires)
            return result
        # Only the independently authorised shared-source view crosses delivery.
        # Preserve its original times/lineage; never substitute delivery time.
        result.update(state="available", source=view)
        return result
