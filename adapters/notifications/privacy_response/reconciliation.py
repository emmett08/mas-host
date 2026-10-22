"""Optional exact-action lookup for an explicitly qualified incident endpoint."""
from __future__ import annotations

import hashlib
import re
import socket
import ssl
import time

from .contracts import Notification, ResponseError, canonical
from .notifier import HttpsIncidentSink, SubmissionUncertain, _remaining


class HttpsReconciledIncidentSink(HttpsIncidentSink):
    """Require same-origin, authoritative GET lookup and idempotent POST.

    The configured endpoint contract must bind 200/204 to accepted submission
    and 404 to authoritative absence for this exact Idempotency-Key. Other
    statuses and transport failures preserve uncertainty. The adapter never
    derives recipient acknowledgement from submission lookup.
    """
    def __init__(self, *, lookup_path, delivery_contract, acknowledgement_path=None, **arguments):
        qualified = delivery_contract in ("idempotency-key-authoritative-lookup/1", "idempotency-key-authoritative-lookup-and-ack/1")
        if (not qualified
                or type(lookup_path) is not str or not lookup_path.startswith("/")
                or not lookup_path.endswith("/") or len(lookup_path) > 512
                or re.fullmatch(r"/[A-Za-z0-9_./-]*/", lookup_path) is None
                or ".." in lookup_path.split("/")):
            raise ResponseError("privacy-notifier-lookup-config-invalid")
        if acknowledgement_path is not None:
            if (delivery_contract != "idempotency-key-authoritative-lookup-and-ack/1"
                    or type(acknowledgement_path) is not str
                    or len(acknowledgement_path) > 512
                    or re.fullmatch(r"/[A-Za-z0-9_./-]*/", acknowledgement_path) is None
                    or ".." in acknowledgement_path.split("/")):
                raise ResponseError("privacy-notifier-acknowledgement-config-invalid")
        elif delivery_contract == "idempotency-key-authoritative-lookup-and-ack/1":
            raise ResponseError("privacy-notifier-acknowledgement-config-invalid")
        super().__init__(**arguments)
        self.lookup_path = lookup_path
        self.acknowledgement_path = acknowledgement_path
        self.delivery_contract = delivery_contract

    def configuration_binding(self):
        return hashlib.sha256(canonical(["mas-privacy-reconciled-target/1",
                                         super().configuration_binding(), self.lookup_path, self.acknowledgement_path])).hexdigest()

    def reconcile(self, notification):
        return self._lookup(notification, self.lookup_path, {200:"accepted",204:"accepted",404:"not-accepted"})

    def acknowledged(self, notification):
        if self.acknowledgement_path is None:
            raise ResponseError("privacy-acknowledgement-capability-required")
        return self._lookup(notification, self.acknowledgement_path, {200:"acknowledged",204:"acknowledged",404:"pending"})

    def _lookup(self, notification, lookup_path, states):
        if type(notification) is not Notification or notification.event.route != self.route:
            raise ResponseError("privacy-reconciliation-contract-invalid")
        if not self._active.acquire(blocking=False):
            return "unknown"
        connection = None
        try:
            deadline = time.monotonic() + self.timeout_seconds
            raw = socket.create_connection((self.host, 443), timeout=self.timeout_seconds)
            try:
                raw.settimeout(_remaining(deadline))
                connection = ssl.create_default_context().wrap_socket(raw, server_hostname=self.host)
            except BaseException:
                raw.close()
                raise
            request = (f"GET {lookup_path}{notification.action_id} HTTP/1.1\r\n"
                       f"Host: {self.authority}\r\nAuthorization: Bearer {self._bearer}\r\n"
                       "Connection: close\r\n\r\n").encode("ascii")
            connection.settimeout(_remaining(deadline))
            connection.sendall(request)
            line = self._read_line(connection, deadline, 512)
            match = re.fullmatch(rb"HTTP/1\.[01] ([1-5][0-9]{2}) [\x20-\x7e]*\r\n", line)
            if match is None:
                return "unknown"
            status = int(match[1])
            return states.get(status, "unknown")
        except (OSError, TimeoutError, EOFError, ValueError, SubmissionUncertain):
            return "unknown"
        finally:
            if connection is not None:
                connection.close()
            self._active.release()
