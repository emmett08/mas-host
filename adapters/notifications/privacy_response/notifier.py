"""Submit one fixed notification to a configured endpoint without redispatch."""
from __future__ import annotations

import re
import socket
import ssl
import threading
import time
from urllib.parse import urlsplit

from mas_host import EffectResult

from .contracts import Notification, ResponseError, ROUTES, canonical
import hashlib


class SubmissionUncertain(RuntimeError):
    """The admitted submission needs reconciliation under its original identity."""


def _result(state, execution, retry_after=0, metadata="available"):
    return EffectResult(
        {"submission": state, "recipient_delivery": "unknown",
         "retry_after_seconds": retry_after, "response_metadata": metadata},
        {"execution": execution, "execution_scope": "notification-endpoint-submission"},
    )


def _remaining(deadline):
    available = deadline - time.monotonic()
    if available <= 0:
        raise TimeoutError
    return available


class MemorySink:
    """Deterministic bounded sink; process memory is not durable delivery evidence."""
    def __init__(self, max_actions=128):
        if type(max_actions) is not int or not 1 <= max_actions <= 1024:
            raise ResponseError("privacy-notifier-config-invalid")
        self.max_actions = max_actions
        self.actions = {}
        self._lock = threading.Lock()

    def configuration_binding(self):
        return hashlib.sha256(canonical(["mas-privacy-notifier-target/1", "memory", self.max_actions])).hexdigest()

    def submit(self, notification):
        if type(notification) is not Notification:
            raise ResponseError("privacy-notification-invalid")
        raw = notification.encoded()
        with self._lock:
            prior = self.actions.get(notification.action_id)
            if prior is not None:
                if prior != raw:
                    raise ResponseError("privacy-notification-identity-conflict")
                return _result("accepted", "completed")
            if len(self.actions) >= self.max_actions:
                return _result("refused", "refused")
            self.actions[notification.action_id] = raw
        return _result("accepted", "completed")


class HttpsIncidentSink:
    """Bounded HTTP/1.1 endpoint acceptance under a deployment-approved contract.

    The endpoint must implement `Idempotency-Key` deduplication, reject changed
    content under the same key, and guarantee that a 4xx refusal has not accepted an incident. Deployment qualification
    must verify those promises. No HTTP library redirect, retry, proxy or
    provider fallback is enabled. Network ambiguity raises, preserving the
    existing host effect's uncertainty. A 2xx status establishes submission
    acceptance, including when subsequent response metadata is malformed.
    """
    def __init__(self, *, route, endpoint, bearer_token, timeout_seconds=5,
                 delivery_contract="idempotency-key-acceptance/1"):
        if type(endpoint) is not str:
            raise ResponseError("privacy-notifier-config-invalid")
        try:
            selected = urlsplit(endpoint)
            valid = (type(endpoint) is str and len(endpoint) <= 2048
                     and not any(ord(c) < 33 or ord(c) > 126 for c in endpoint)
                     and selected.scheme == "https" and selected.hostname
                     and selected.username is None and selected.password is None
                     and selected.query == "" and selected.fragment == ""
                     and selected.port in (None, 443)
                     and len(selected.path) <= 1024
                     and route in ROUTES
                     and type(bearer_token) is str and 1 <= len(bearer_token) <= 4096
                     and all(33 <= ord(c) <= 126 for c in bearer_token)
                     and type(timeout_seconds) in (int, float)
                     and 0.1 <= timeout_seconds <= 30
                     and delivery_contract == "idempotency-key-acceptance/1")
        except (ValueError, TypeError):
            valid = False
        if not valid:
            raise ResponseError("privacy-notifier-config-invalid")
        self.route = route
        self.endpoint = endpoint
        self.delivery_contract = delivery_contract
        self.host = selected.hostname
        self.authority = selected.netloc
        self.path = selected.path or "/"
        self._bearer = bearer_token
        self.timeout_seconds = timeout_seconds
        # One request per adapter instance. Independent native invocations are
        # additionally bounded by the host's existing admission limits.
        self._active = threading.Lock()

    def configuration_binding(self):
        return hashlib.sha256(canonical(["mas-privacy-notifier-target/1", self.route,
                                         self.host, self.authority, self.path,
                                         self.delivery_contract, self.timeout_seconds])).hexdigest()

    def _read_line(self, connection, deadline, remaining):
        data = bytearray()
        while len(data) < remaining:
            connection.settimeout(_remaining(deadline))
            part = connection.recv(1)
            if not part:
                raise EOFError
            data += part
            if data.endswith(b"\r\n"):
                return bytes(data)
        raise ResponseError("privacy-notifier-response-limit")

    def _read_response(self, connection, deadline):
        line = self._read_line(connection, deadline, 512)
        match = re.fullmatch(rb"HTTP/1\.[01] ([1-5][0-9]{2}) [\x20-\x7e]*\r\n", line)
        if match is None:
            raise SubmissionUncertain("privacy-notification-submission-uncertain")
        status = int(match[1])
        known_accepted = 200 <= status < 300
        known_refused = 400 <= status < 500 and status not in (408, 425)
        used = len(line)
        headers = {}
        try:
            for _ in range(33):
                line = self._read_line(connection, deadline, 8192 - used)
                used += len(line)
                if line == b"\r\n":
                    break
                if len(headers) == 32 or b":" not in line:
                    raise ValueError
                key, value = line[:-2].split(b":", 1)
                if (not re.fullmatch(rb"[!#$%&'*+.^_`|~0-9A-Za-z-]+", key)
                        or key.lower() in headers
                        or any(c < 32 and c != 9 or c > 126 for c in value)):
                    raise ValueError
                headers[key.lower()] = value.strip()
            else:
                raise ValueError
        except (OSError, EOFError, TimeoutError, ResponseError, ValueError):
            if known_accepted:
                return _result("accepted", "completed", metadata="withheld")
            if known_refused:
                return _result("throttled" if status == 429 else "refused", "refused", metadata="withheld")
            raise SubmissionUncertain("privacy-notification-submission-uncertain") from None
        # Response bodies are never read, retained or echoed. This adapter
        # reports endpoint acceptance and cannot claim recipient delivery.
        if known_accepted:
            return _result("accepted", "completed", metadata="withheld")
        if status == 429:
            value = headers.get(b"retry-after", b"")
            retry_after = int(value) if re.fullmatch(rb"[0-9]{1,4}", value) else 0
            return _result("throttled", "refused", min(retry_after, 3600))
        if 400 <= status < 500 and status not in (408, 425):
            return _result("refused", "refused")
        # Redirects, transient/server failures and timeout responses have no
        # acceptance guarantee under this contract. No follow-up send occurs.
        raise SubmissionUncertain("privacy-notification-submission-uncertain")

    def submit(self, notification):
        if type(notification) is not Notification:
            raise ResponseError("privacy-notification-invalid")
        if notification.event.route != self.route:
            return _result("refused", "not-sent")
        body = notification.encoded()
        if not self._active.acquire(blocking=False):
            return _result("refused", "not-sent")
        connection = None
        sent = False
        try:
            deadline = time.monotonic() + self.timeout_seconds
            raw = socket.create_connection((self.host, 443), timeout=self.timeout_seconds)
            try:
                raw.settimeout(_remaining(deadline))
                connection = ssl.create_default_context().wrap_socket(raw, server_hostname=self.host)
            except BaseException:
                raw.close()
                raise
            request = (f"POST {self.path} HTTP/1.1\r\nHost: {self.authority}\r\n"
                       f"Authorization: Bearer {self._bearer}\r\n"
                       f"Idempotency-Key: {notification.action_id}\r\n"
                       "Content-Type: application/json\r\nConnection: close\r\n"
                       f"Content-Length: {len(body)}\r\n\r\n").encode("ascii") + body
            connection.settimeout(_remaining(deadline))
            # Once send starts, even a partial failure cannot prove unsent.
            sent = True
            connection.sendall(request)
            return self._read_response(connection, deadline)
        except (OSError, EOFError, TimeoutError, ValueError) as error:
            if not sent:
                return _result("not-sent", "not-sent")
            raise SubmissionUncertain("privacy-notification-submission-uncertain") from None
        finally:
            if connection is not None:
                connection.close()
            self._active.release()
