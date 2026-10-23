"""One credential-free, registered GET inside a fresh native dispatch.

The existing host controller consumes the original native and provider permits
before launching this source function. This helper grants neither permission.
Request registration is compiled into the pinned worker source, never accepted
from an application input or provider response. No request is retried here.
"""

from __future__ import annotations

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
except ImportError:
    import provider


_FIELDS = {"version", "provider", "source_ref", "quota_scope", "endpoint", "operation",
           "if_none_match", "projection", "allow_loopback_http", "timeout_ms", "max_body_bytes"}


def _require(condition):
    if not condition:
        raise ValueError("registered-provider-read-rejected")


def _clock_ms():
    return time.time_ns() // 1_000_000


def _identity(value):
    _require(type(value) is str and len(value) <= 160
             and re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._:/-]*", value) is not None)
    return value


def _json(raw):
    def pairs(values):
        result = {}
        for key, value in values:
            _require(key not in result)
            result[key] = value
        return result
    def reject(_):
        raise ValueError("registered-provider-read-rejected")
    return json.loads(raw, object_pairs_hook=pairs, parse_constant=reject)


def _project(value, projection):
    result = {}
    for name, selection in projection.items():
        selected = value
        for component in selection["pointer"].split("/")[1:]:
            component = component.replace("~1", "/").replace("~0", "~")
            if type(selected) is dict:
                selected = selected[component]
            elif type(selected) is list and re.fullmatch(r"0|[1-9][0-9]{0,3}", component):
                selected = selected[int(component)]
            else:
                raise ValueError("registered-provider-read-rejected")
        kind = selection["type"]
        if kind == "string":
            _require(type(selected) is str and len(selected.encode()) <= 1024
                     and not any(ord(c) < 32 for c in selected))
        elif kind == "integer":
            _require(type(selected) is int and -(2 ** 63) <= selected < 2 ** 63)
        elif kind == "boolean":
            _require(type(selected) is bool)
        else:
            raise ValueError("registered-provider-read-rejected")
        result[name] = selected
    _require(len(json.dumps(result, ensure_ascii=False).encode()) <= 16384)
    return result


class PublicGET:
    """Immutable operator-owned read registration for a credential-free worker.

    Production GitHub reads use api.github.com with verified TLS. Kubernetes
    support here is explicitly anonymous loopback qualification only. The host's
    current shared native profile deliberately grants no provider credentials.
    The approved projection is ordinary data, not a managed privacy guarantee.
    """

    def __init__(self, contract: dict):
        _require(type(contract) is dict and set(contract) == _FIELDS
                 and contract["version"] == "mas-provider-public-get/1"
                 and contract["provider"] in {"github", "kubernetes"}
                 and contract["operation"] in {"read", "list", "watch"})
        _identity(contract["source_ref"])
        _identity(contract["quota_scope"])
        _require(type(contract["allow_loopback_http"]) is bool
                 and type(contract["timeout_ms"]) is int and 1 <= contract["timeout_ms"] <= 10000
                 and type(contract["max_body_bytes"]) is int and 1 <= contract["max_body_bytes"] <= 32768)
        etag = contract["if_none_match"]
        _require(etag is None or (type(etag) is str and 2 <= len(etag) <= 256
                 and re.fullmatch(r'(W/)?"[!#-~]+"', etag) is not None))
        projection = contract["projection"]
        _require(type(projection) is dict and 1 <= len(projection) <= 32)
        for name, selection in projection.items():
            _identity(name)
            _require(type(selection) is dict and set(selection) == {"pointer", "type"}
                     and selection["type"] in {"string", "integer", "boolean"}
                     and type(selection["pointer"]) is str and len(selection["pointer"]) <= 512
                     and selection["pointer"].startswith("/")
                     and len(selection["pointer"].split("/")) <= 17
                     and not any(ord(c) < 32 for c in selection["pointer"]))
        try:
            endpoint = contract["endpoint"]
            _require(type(endpoint) is str and len(endpoint) <= 2048
                     and all(33 <= ord(c) <= 126 for c in endpoint))
            parsed = urlsplit(endpoint)
            _require(parsed.hostname is not None and parsed.username is None
                     and parsed.password is None and not parsed.fragment)
            local = False
            if parsed.scheme == "http" and contract["allow_loopback_http"]:
                local = ipaddress.ip_address(parsed.hostname).is_loopback
            _require(local or (parsed.scheme == "https" and parsed.hostname == "api.github.com"
                     and parsed.port in {None, 443} and contract["provider"] == "github"))
            self._host, self._port, self._scheme = parsed.hostname, parsed.port, parsed.scheme
            self._path = (parsed.path or "/") + ("?" + parsed.query if parsed.query else "")
            # Deep snapshot without retaining caller-owned mutable dictionaries.
            self._contract = json.dumps(contract, sort_keys=True, separators=(",", ":"), allow_nan=False)
            _require(len(self._contract.encode()) <= 16384)
        except Exception:
            raise ValueError("registered-provider-read-rejected") from None

    def __repr__(self):
        return "PublicGET(registered=True)"

    def read(self, *, deadline_ms: int, clock=_clock_ms) -> dict:
        """Execute once after consuming host permission, before its deadline."""
        _require(type(deadline_ms) is int and 0 <= deadline_ms <= provider.MAX_TIMESTAMP_MS)
        contract = json.loads(self._contract)
        now_ms = clock()
        _require(type(now_ms) is int and 0 <= now_ms <= provider.MAX_TIMESTAMP_MS)
        connection = watchdog = None
        projection = None
        disposition = "unavailable"
        dispatched = False
        response_status = None
        if now_ms >= deadline_ms:
            observation = provider.observe(contract["provider"], contract["quota_scope"], contract["operation"],
                now_ms=now_ms, transport="not-dispatched")
        else:
            try:
                timeout = min(contract["timeout_ms"], deadline_ms - now_ms) / 1000
                if self._scheme == "https":
                    connection = http.client.HTTPSConnection(self._host, self._port, timeout=timeout,
                                                            context=ssl.create_default_context())
                else:
                    connection = http.client.HTTPConnection(self._host, self._port, timeout=timeout)
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
                headers = {"Accept": "application/json", "User-Agent": "nehemiah-registered-read/1"}
                if contract["if_none_match"] is not None:
                    headers["If-None-Match"] = contract["if_none_match"]
                dispatched = True
                connection.request("GET", self._path, headers=headers)
                response = connection.getresponse()
                response_status = response.status
                now_ms = max(now_ms, clock())
                observation = provider.observe(contract["provider"], contract["quota_scope"], contract["operation"],
                    now_ms=now_ms, http_status=response.status, headers=response.getheaders())
                if response.status == 304:
                    disposition = "not-modified"
                elif response.status == 200:
                    # Only an approved deterministic scalar projection leaves
                    # this boundary. Raw response/error bodies are never stored.
                    raw = response.read(contract["max_body_bytes"] + 1)
                    _require(len(raw) <= contract["max_body_bytes"])
                    projection = _project(_json(raw), contract["projection"])
                    if clock() < deadline_ms:
                        disposition = "available"
                    else:
                        projection = None
            except Exception:
                now_ms = max(now_ms, clock())
                observation = provider.observe(contract["provider"], contract["quota_scope"], contract["operation"],
                    now_ms=now_ms, transport="outcome-unknown")
                projection = None
            finally:
                if watchdog is not None:
                    watchdog.cancel()
                if connection is not None:
                    connection.close()
        now_ms = max(now_ms, clock())
        decision = provider.schedule(observation, now_ms=now_ms, deadline_ms=deadline_ms)
        result = {"version": "mas-provider-public-read-result/1", "source_ref": contract["source_ref"],
            "request_dispatched": dispatched, "response_http_status": response_status,
            "provider_observation": observation,
            "provider_schedule": decision, "payload_disposition": disposition, "projection": projection,
            "raw_source_disposition": "not-retained", "execution_permission": False}
        _require(len(json.dumps(result, ensure_ascii=False).encode()) <= 32768)
        return result
