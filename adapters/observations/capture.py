#!/usr/bin/env python3
"""Bounded ordinary-local GetTaskStatus capture. No worker or source operations."""
from __future__ import annotations

import argparse
from collections import deque
import copy
from dataclasses import asdict, dataclass
import fcntl
import hashlib
import hmac
import json
import os
from pathlib import Path
import re
import sqlite3
import stat
import sys
import time
import weakref

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "acquisition"))
from coalescing import BASE, HTTPRPC

PROFILE = "ordinary-trusted-local-metadata/1"
VERSION = "mas-status-trigger-capture/1"
MAX_INT = 2**63 - 1
COUNTERS = ("buffer_evicted", "byte_budget", "sample_failed", "revision_gap",
            "capture_refused", "restart_interrupted", "sampling_gap")
REASONS = {"operator", "status-unknown", "revision-gap"}
GAPS = {"restart-interrupted", "byte-budget", "pre-window-incomplete", "revision-gap",
        "sampling-gap", "sample-failed", "post-window-incomplete", "buffer-evicted"}
STATES = {"submitted", "working", "completed", "failed", "unknown"}
STATUS_FIELDS = {"version", "id", "agent", "bundleDigest", "serviceState", "cancelRequested",
                 "hostSettlement", "externalExecution", "payloadAvailability", "evidenceCoverage",
                 "applicationAssessment", "sourceEvidenceFreshness", "materialisation",
                 "executionPermission", "captureProfile"}


def require(condition):
    if not condition:
        raise ValueError("status-capture-contract-rejected")


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":"), allow_nan=False).encode()


def integer(value, minimum=0, maximum=MAX_INT):
    require(type(value) is int and minimum <= value <= maximum)
    return value


def private_file(path, maximum):
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
    try:
        meta = os.fstat(fd)
        require(stat.S_ISREG(meta.st_mode) and meta.st_mode & 0o077 == 0)
        with os.fdopen(fd, "rb", closefd=False) as stream:
            value = stream.read(maximum + 1)
        require(len(value) <= maximum)
        return value
    finally:
        os.close(fd)


@dataclass(frozen=True)
class Limits:
    pre_ms: int = 5000
    post_ms: int = 5000
    retention_ms: int = 60000
    max_bytes: int = 65536
    max_captures: int = 16
    buffer_bytes: int = 65536
    sample_ms: int = 1000

    def validate(self):
        integer(self.pre_ms, 0, 60000)
        integer(self.post_ms, 0, 60000)
        integer(self.retention_ms, max(self.pre_ms, self.post_ms) + 1, 86400000)
        integer(self.max_bytes, 4096, 65536)
        integer(self.max_captures, 1, 16)
        integer(self.buffer_bytes, 1024, 65536)
        integer(self.sample_ms, 100, 60000)
        return self


def project_status(value, registration, token, at_ms):
    """Validate the authenticated host's explicit ordinary-only classification."""
    require(type(value) is dict and set(value) == STATUS_FIELDS)
    require(value["captureProfile"] == PROFILE and value["version"] == "mas-host-task-status/1")
    require(value["id"] == registration["id"] and value["agent"] == registration["agent"]
            and value["bundleDigest"] == registration["bundleDigest"])
    require(value["serviceState"] in STATES and type(value["cancelRequested"]) is bool)
    require(value["hostSettlement"] == ("completed" if value["serviceState"] == "completed"
                                         else "not-established"))
    require(value["executionPermission"] is False)
    require(all(value[name] == "not-assessed" for name in
                ("externalExecution", "evidenceCoverage", "applicationAssessment", "sourceEvidenceFreshness")))
    require(value["payloadAvailability"] in
            {"retained-requires-authorised-read", "not-retained-in-service"})
    material = value["materialisation"]
    require(type(material) is dict and set(material) ==
            {"revision", "updatedAtMs", "observedAtMs", "consistency", "incomplete"})
    require(material["consistency"] == "committed-service-snapshot" and material["incomplete"] is False)
    integer(material["observedAtMs"])
    return {"at_ms": integer(at_ms), "task_ref": token("task", value["id"]),
            "agent_ref": token("agent", value["agent"]), "bundle_ref": token("bundle", value["bundleDigest"]),
            "revision": integer(material["revision"], 1), "updated_at_ms": integer(material["updatedAtMs"]),
            "state": value["serviceState"], "cancel_requested": value["cancelRequested"],
            "host_settlement": value["hostSettlement"], "consistency": "committed-service-snapshot"}


def check_capture(value):
    """Check snapshot revision invariants; never infer external execution truth."""
    require(type(value) is dict and set(value) == {"version", "capture_ref", "scope_ref", "task_ref",
            "trigger_reason", "triggered_at_ms", "start_ms", "end_ms", "expires_at_ms", "occurrences",
            "records", "coverage_gaps", "closed", "loss"} and value["version"] == VERSION)
    require(len(canonical(value)) <= 65536 and value["trigger_reason"] in REASONS)
    for name in ("capture_ref", "scope_ref", "task_ref"):
        require(type(value[name]) is str and re.fullmatch(r"[0-9a-f]{64}", value[name]))
    for name in ("triggered_at_ms", "start_ms", "end_ms", "expires_at_ms"):
        integer(value[name])
    require(value["start_ms"] <= value["triggered_at_ms"] <= value["end_ms"] < value["expires_at_ms"])
    integer(value["occurrences"], 1)
    require(type(value["closed"]) is bool and type(value["coverage_gaps"]) is list
            and len(value["coverage_gaps"]) <= len(GAPS) and set(value["coverage_gaps"]) <= GAPS)
    require(type(value["loss"]) is dict and set(value["loss"]) == set(COUNTERS))
    for count in value["loss"].values():
        integer(count)
    require(type(value["records"]) is list and len(value["records"]) <= 256)
    for record in value["records"]:
        require(type(record) is dict and set(record) == {"at_ms", "task_ref", "agent_ref", "bundle_ref",
                "revision", "updated_at_ms", "state", "cancel_requested", "host_settlement", "consistency"})
        require(record["task_ref"] == value["task_ref"] and record["state"] in STATES
                and type(record["cancel_requested"]) is bool
                and record["consistency"] == "committed-service-snapshot"
                and record["host_settlement"] == ("completed" if record["state"] == "completed" else "not-established"))
        integer(record["revision"], 1)
        integer(record["updated_at_ms"])
        integer(record["at_ms"], value["start_ms"], value["end_ms"])
        for name in ("agent_ref", "bundle_ref"):
            require(type(record[name]) is str and re.fullmatch(r"[0-9a-f]{64}", record[name]))
    errors, gaps, prior = [], list(value["coverage_gaps"]), {}
    for record in value["records"]:
        old = prior.get(record["task_ref"])
        if old:
            if record["revision"] < old["revision"]:
                errors.append("status-revision-regressed")
            elif record["revision"] == old["revision"]:
                if {k: v for k, v in record.items() if k != "at_ms"} != {
                        k: v for k, v in old.items() if k != "at_ms"}:
                    errors.append("status-revision-conflict")
            elif record["revision"] > old["revision"] + 1:
                gaps.append("revision-gap")
        prior[record["task_ref"]] = record
    if not value["closed"]:
        gaps.append("post-window-incomplete")
    gaps.extend(name.replace("_", "-") for name, count in value["loss"].items() if count)
    # A service snapshot cannot establish no lost work, duplicate execution or
    # competing owners. Independent authoritative histories remain necessary.
    gaps.append("independent-source-history-unavailable")
    return {"status": "FAIL" if errors else "INCONCLUSIVE", "errors": sorted(set(errors)),
            "coverage_gaps": sorted(set(gaps)), "execution_permission": False}


class Monitor:
    """One trusted local owner; retained rows and in-memory work are bounded."""

    def __init__(self, rpc, registrations, *, key, spool, limits=Limits(), now_ms,
                 clock=lambda: time.time_ns() // 1000000):
        self.limits = limits.validate()
        require(type(key) is bytes and 32 <= len(key) <= 64)
        require(type(registrations) is list and 1 <= len(registrations) <= 16)
        self.rpc, self.key, self.authority = rpc, key, rpc.authority
        self._clock = clock
        self.registrations = {}
        for registration in registrations:
            require(type(registration) is dict and set(registration) == {"id", "agent", "bundleDigest"})
            require(type(registration["id"]) is str and re.fullmatch(r"a2a-[0-9a-f]{56}", registration["id"]))
            require(type(registration["agent"]) is str and
                    re.fullmatch(r"[A-Za-z0-9._:/-]{1,128}", registration["agent"]))
            require(type(registration["bundleDigest"]) is str and
                    re.fullmatch(r"[0-9a-f]{64}", registration["bundleDigest"]))
            require(registration["id"] not in self.registrations)
            self.registrations[registration["id"]] = copy.deepcopy(registration)
        require(type(rpc.authority) is str and 1 <= len(rpc.authority) <= 128)
        self.scope = self.token("scope", [rpc.authority, registrations, asdict(limits)])
        self.clock = integer(now_ms)
        self.next_poll_ms = self.clock
        self.buffer, self.previous, self.classified = deque(), {}, set()
        self.loss = dict.fromkeys(COUNTERS, 0)
        self.sequence = 0
        self.cases = {}
        path = Path(spool)
        require(path.is_absolute() and path.parent.is_dir() and
                path.parent.stat().st_mode & 0o077 == 0)
        try:
            fd = os.open(path, os.O_CREAT | os.O_EXCL | os.O_WRONLY | os.O_NOFOLLOW, 0o600)
            os.close(fd)
        except FileExistsError:
            private_file(path, 2 * 1024 * 1024)
        self._owner_fd = os.open(path, os.O_RDWR | os.O_NOFOLLOW)
        self._owner_finalizer = weakref.finalize(self, os.close, self._owner_fd)
        try:
            fcntl.flock(self._owner_fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except OSError:
            self._owner_finalizer()
            raise ValueError("status-capture-owner-unavailable") from None
        self.db = sqlite3.connect(path.as_uri() + "?mode=rw", uri=True, timeout=1)
        self.db.execute("PRAGMA secure_delete=ON")
        self.db.execute("PRAGMA journal_mode=DELETE")
        self.db.execute("PRAGMA synchronous=EXTRA")
        with self.db:
            self.db.execute("CREATE TABLE IF NOT EXISTS capture_meta(scope TEXT PRIMARY KEY)")
            self.db.execute("CREATE TABLE IF NOT EXISTS captures(id TEXT PRIMARY KEY,expires INTEGER,body TEXT,mac TEXT)")
            self.db.execute("CREATE TABLE IF NOT EXISTS capture_counters(name TEXT PRIMARY KEY,value INTEGER)")
            rows = self.db.execute("SELECT scope FROM capture_meta LIMIT 2").fetchall()
            if rows:
                require(rows == [(self.scope,)])
            else:
                self.db.execute("INSERT INTO capture_meta VALUES(?)", (self.scope,))
            counters = self.db.execute("SELECT name,value FROM capture_counters LIMIT 8").fetchall()
            require(len(counters) <= len(COUNTERS))
            for name, count in counters:
                require(name in COUNTERS)
                self.loss[name] = integer(count)
            self.db.execute("DELETE FROM captures WHERE expires<=?", (self.clock,))
            rows = self.db.execute("SELECT id,body,mac FROM captures LIMIT 17").fetchall()
            require(len(rows) <= self.limits.max_captures)
            for reference, body, mac in rows:
                require(type(body) is str and len(body.encode()) <= limits.max_bytes
                        and hmac.compare_digest(self.token("stored", body), mac))
                case = json.loads(body)
                check_capture(case)
                require(case["scope_ref"] == self.scope and case["capture_ref"] == reference)
                self.cases[reference] = case
                if not case["closed"]:
                    case["closed"] = True
                    self.gap(case, "restart-interrupted")
                    self.bump("restart_interrupted")
                    self.save(case)
            self.flush()

    def token(self, kind, value):
        return hmac.new(self.key, canonical(["mas-status-capture/1", self.authority, kind, value]),
                        hashlib.sha256).hexdigest()

    def bump(self, name, amount=1):
        self.loss[name] = min(MAX_INT, self.loss[name] + amount)

    @staticmethod
    def gap(case, name):
        if name not in case["coverage_gaps"]:
            case["coverage_gaps"].append(name)

    def save(self, case):
        case["loss"] = dict(self.loss)
        body = canonical(case).decode()
        require(len(body.encode()) <= self.limits.max_bytes)
        self.db.execute("INSERT INTO captures VALUES(?,?,?,?) ON CONFLICT(id) DO UPDATE SET body=excluded.body,mac=excluded.mac",
                        (case["capture_ref"], case["expires_at_ms"], body, self.token("stored", body)))

    def flush(self):
        for name, count in self.loss.items():
            self.db.execute("INSERT INTO capture_counters VALUES(?,?) ON CONFLICT(name) DO UPDATE SET value=excluded.value",
                            (name, count))
        for case in self.cases.values():
            self.save(case)

    def tick(self, now_ms):
        now_ms = integer(now_ms)
        require(self.clock <= now_ms <= MAX_INT - self.limits.retention_ms)
        self.clock = now_ms
        with self.db:
            self.db.execute("DELETE FROM captures WHERE expires<=?", (now_ms,))
            for reference, case in list(self.cases.items()):
                if now_ms >= case["expires_at_ms"]:
                    del self.cases[reference]
                elif now_ms > case["end_ms"] and not case["closed"]:
                    case["closed"] = True
                    if not case["records"] or case["records"][-1]["at_ms"] < case["end_ms"] - self.limits.sample_ms:
                        self.gap(case, "post-window-incomplete")
                    self.save(case)
        while self.buffer and self.buffer[0]["at_ms"] < now_ms - self.limits.pre_ms:
            self.buffer.popleft()

    def append(self, case, record):
        if len(case["records"]) >= 256 or sum(len(canonical(r)) for r in case["records"]) + len(canonical(record)) > self.limits.max_bytes - 2048:
            self.bump("byte_budget")
            self.gap(case, "byte-budget")
        else:
            case["records"].append(copy.deepcopy(record))

    def trigger(self, task_id, reason, *, now_ms):
        self.tick(now_ms)
        require(task_id in self.classified and reason in REASONS)
        task_ref = self.token("task", task_id)
        for case in self.cases.values():
            if not case["closed"] and case["task_ref"] == task_ref and case["trigger_reason"] == reason:
                case["occurrences"] = min(MAX_INT, case["occurrences"] + 1)
                with self.db:
                    self.save(case)
                return case["capture_ref"]
        if len(self.cases) >= self.limits.max_captures:
            self.bump("capture_refused")
            with self.db:
                self.flush()
            return None
        self.sequence += 1
        reference = self.token("capture", [task_ref, now_ms, self.sequence, os.urandom(16).hex()])
        case = {"version": VERSION, "capture_ref": reference, "scope_ref": self.scope,
                "task_ref": task_ref, "trigger_reason": reason, "triggered_at_ms": now_ms,
                "start_ms": max(0, now_ms - self.limits.pre_ms), "end_ms": now_ms + self.limits.post_ms,
                "expires_at_ms": now_ms + self.limits.retention_ms, "occurrences": 1,
                "records": [], "coverage_gaps": [], "closed": False, "loss": dict(self.loss)}
        if reason == "revision-gap":
            self.gap(case, "revision-gap")
        for record in self.buffer:
            if record["task_ref"] == task_ref and record["at_ms"] >= case["start_ms"]:
                self.append(case, record)
        if not case["records"] or (self.limits.pre_ms and case["records"][0]["at_ms"] > case["start_ms"] + self.limits.sample_ms):
            self.gap(case, "pre-window-incomplete")
        self.cases[reference] = case
        with self.db:
            self.save(case)
        return reference

    def poll(self, *, now_ms, deadline_ms):
        self.tick(now_ms)
        require(self.rpc.authority == self.authority)
        integer(deadline_ms, now_ms + 1)
        if now_ms < self.next_poll_ms:
            return self.summary(now_ms=now_ms)
        self.next_poll_ms = now_ms + self.limits.sample_ms
        for task_id, registration in self.registrations.items():
            reply = self.rpc.call("GetTaskStatus", {"id": task_id}, (BASE,),
                                  "capture-" + self.token("attempt", [task_id, now_ms]),
                                  now_ms=now_ms, deadline_ms=deadline_ms)
            received_ms = max(self.clock, now_ms, integer(self._clock()))
            self.tick(received_ms)
            record = None
            if reply.state == "ok":
                # An authenticated source must advertise ordinary capture. A
                # caller-supplied boolean or read grant cannot activate it.
                require(type(reply.value) is dict and reply.value.get("captureProfile") == PROFILE)
                try:
                    record = project_status(reply.value, registration, self.token, received_ms)
                    self.classified.add(task_id)
                except (ValueError, TypeError, KeyError):
                    pass
            if record is None:
                self.classified.discard(task_id)
                self.bump("sample_failed")
                for case in self.cases.values():
                    if not case["closed"] and case["task_ref"] == self.token("task", task_id):
                        self.gap(case, "sample-failed")
                continue
            old = self.previous.get(task_id)
            if old and record["at_ms"] - old["at_ms"] > 2 * self.limits.sample_ms:
                self.bump("sampling_gap")
            revision_gap = old and record["revision"] > old["revision"] + 1
            if revision_gap:
                self.bump("revision_gap", record["revision"] - old["revision"] - 1)
            self.previous[task_id] = record
            self.buffer.append(record)
            while len(self.buffer) > 128 or sum(len(canonical(r)) for r in self.buffer) > self.limits.buffer_bytes:
                self.buffer.popleft()
                self.bump("buffer_evicted")
            for case in self.cases.values():
                if not case["closed"] and case["task_ref"] == record["task_ref"]:
                    self.append(case, record)
                    if revision_gap:
                        self.gap(case, "revision-gap")
                    if old and record["at_ms"] - old["at_ms"] > 2 * self.limits.sample_ms:
                        self.gap(case, "sampling-gap")
            if record["state"] == "unknown":
                self.trigger(task_id, "status-unknown", now_ms=received_ms)
            if revision_gap:
                self.trigger(task_id, "revision-gap", now_ms=received_ms)
        with self.db:
            self.flush()
        return self.summary(now_ms=self.clock)

    def read(self, reference, *, now_ms):
        self.tick(now_ms)
        require(reference in self.cases)
        return copy.deepcopy(self.cases[reference])

    def summary(self, *, now_ms):
        self.tick(now_ms)
        return {"version": "mas-status-capture-summary/1", "scope_ref": self.scope,
                "captures": len(self.cases), "loss": dict(self.loss),
                "next_poll_ms": self.next_poll_ms,
                "execution_permission": False, "source_history": "not-assessed"}

    def close(self):
        self.db.close()
        self._owner_finalizer()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("endpoint", "authority", "quota-scope", "bearer-file", "identity-key-file", "registrations", "spool"):
        parser.add_argument("--" + name, required=True)
    parser.add_argument("--duration-ms", type=int, default=30000)
    parser.add_argument("--allow-loopback-http", action="store_true")
    args = parser.parse_args()
    try:
        integer(args.duration_ms, 1, 3600000)
        key = private_file(args.identity_key_file, 64)
        registrations = json.loads(private_file(args.registrations, 8192))
        bearer = lambda: private_file(args.bearer_file, 4096).decode().strip()
        rpc = HTTPRPC(args.endpoint, args.authority, args.quota_scope, bearer,
                      allow_loopback_http=args.allow_loopback_http)
        start = time.monotonic_ns() // 1000000
        origin = time.time_ns() // 1000000
        monitor = Monitor(rpc, registrations, key=key, spool=args.spool, now_ms=origin)
        while (elapsed := time.monotonic_ns() // 1000000 - start) < args.duration_ms:
            now = max(monitor.clock, origin + elapsed)
            monitor.poll(now_ms=now, deadline_ms=now + min(5000, args.duration_ms - elapsed))
            time.sleep(min(monitor.limits.sample_ms, args.duration_ms - elapsed) / 1000)
        print(json.dumps(monitor.summary(now_ms=max(monitor.clock, origin + args.duration_ms)), sort_keys=True))
        monitor.close()
        return 0
    except Exception:
        print("status-capture-unavailable", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
