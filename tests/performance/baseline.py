"""Measure a bounded local native-execution and observation smoke baseline."""

from __future__ import annotations

import argparse
import collections
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import subprocess
import sys
import tempfile
import time

from workload import MANIFEST, encoded, generate, load_manifest


ROOT = Path(__file__).resolve().parents[2]


def command(args: list[str], timeout: int = 60) -> tuple[dict | None, int | None, float]:
    start = time.perf_counter_ns()
    try:
        result = subprocess.run(args, capture_output=True, timeout=timeout, check=False)
        try:
            document = json.loads(result.stdout)
        except (UnicodeDecodeError, json.JSONDecodeError):
            document = None
        return document, result.returncode, (time.perf_counter_ns() - start) / 1e6
    except subprocess.TimeoutExpired:
        # Terminating a CLI does not establish the outcome of its worker/effect.
        return None, None, (time.perf_counter_ns() - start) / 1e6


def text_command(args: list[str]) -> str | None:
    try:
        result = subprocess.run(args, capture_output=True, text=True, timeout=10, check=False)
    except (OSError, subprocess.TimeoutExpired):
        return None
    return result.stdout.strip() if result.returncode == 0 else None


def classify(snapshot: dict | None, cli_result: dict | None, returncode: int | None) -> dict:
    """Preserve separate admission, execution and start knowledge.

    A returned error alone proves neither non-admission nor execution failure.
    Polling misses intermediate states, so absent start evidence stays unknown.
    """
    result = {"admission": "unknown", "execution": "unknown", "started": None}
    if snapshot is not None:
        roots = [s for s in snapshot.get("steps", [])
                 if s.get("kind") == "request" and s.get("parent") == "" and s.get("key") == "root"]
        if len(roots) == 1:
            root = roots[0]
            state = root.get("state")
            admitted = any(m.get("kind") == "Request" and m.get("task_id") == root.get("id")
                           for m in snapshot.get("messages", []))
            if admitted:
                result["admission"] = "admitted"
            if state == "rejected":
                result.update(admission="rejected", execution="not_started", started=False)
            elif state == "completed":
                result.update(admission="admitted", execution="completed", started=True)
            elif state == "running":
                result.update(admission="admitted", execution="started", started=True)
            elif state == "failed":
                result["execution"] = "failed"
            # Prepared can precede admission or follow a retryable refusal.
            # Payload-free observation cannot distinguish those histories.
    if returncode == 0 and isinstance(cli_result, dict) and cli_result.get("status") == "completed":
        # Stable-but-empty and contradictory post-completion projections fail.
        if snapshot is not None and (len(roots) != 1 or roots[0].get("state") != "completed"):
            return {"admission": "unknown", "execution": "unknown", "started": None}
        result.update(admission="admitted", execution="completed", started=True)
    return result


def totals(records: list[dict]) -> dict:
    admissions = collections.Counter(r["admission"] for r in records)
    executions = collections.Counter(r["execution"] for r in records)
    return {"offered": len(records),
            "admission": {key: admissions[key] for key in ("admitted", "deferred", "rejected", "unknown")},
            "execution": {key: executions[key] for key in ("not_started", "started", "completed", "failed", "unknown")},
            "started": sum(r["started"] is True for r in records),
            "start_unknown": sum(r["started"] is None for r in records)}


def latency_summary(values: list[float]) -> dict:
    ordered = sorted(values)
    return {"count": len(values), **{
        name: ordered[max(0, math.ceil(len(ordered) * fraction) - 1)] if ordered else None
        for name, fraction in (("p50_ms", .50), ("p95_ms", .95), ("p99_ms", .99))}}


def environment(host: Path) -> dict:
    manifest = ROOT / "Cargo.toml"
    revision = text_command(["git", "-C", str(ROOT), "rev-parse", "HEAD"])
    untracked = text_command(["git", "-C", str(ROOT), "ls-files", "--others", "--exclude-standard"])
    return {"checkout_revision": revision, "host_source_revision": None,
            "host_source_binding": "BINARY_HASH_ONLY_BUILD_PROVENANCE_REQUIRED",
            "working_tree_diff_sha256": hashlib.sha256(subprocess.run(
                ["git", "-C", str(ROOT), "diff", "--binary", "HEAD"],
                capture_output=True, check=True).stdout).hexdigest(),
            "source_tree_identity_complete": revision is not None and untracked == "",
            "host_sha256": hashlib.sha256(host.read_bytes()).hexdigest(),
            "cargo_lock_sha256": hashlib.sha256((ROOT / "Cargo.lock").read_bytes()).hexdigest(),
            "workspace_manifest_sha256": hashlib.sha256(manifest.read_bytes()).hexdigest(),
            "harness_sha256": {path.name: hashlib.sha256(path.read_bytes()).hexdigest()
                               for path in (Path(__file__), MANIFEST.with_name("workload.py"))},
            "python": platform.python_version(), "rustc": text_command(["rustc", "--version"]),
            "os": platform.system(), "kernel": platform.release(),
            "architecture": platform.machine(), "logical_cpus": os.cpu_count(),
            "physical_memory_bytes": os.sysconf("SC_PAGE_SIZE") * os.sysconf("SC_PHYS_PAGES"),
            "storage": "temporary directory, local SQLite journals; filesystem/device unqualified",
            "database_version": None, "cost_ceiling": None,
            "provider_requests": 0, "provider_tokens": None, "provider_billing": None}


def smoke(host: Path, count: int, polls: int, seed: int) -> dict:
    if not 1 <= count <= 100 or not 1 <= polls <= 100:
        raise ValueError("smoke count and polls must be 1..100")
    host = host.resolve(strict=True)
    manifest = load_manifest()
    report = {"version": "mas-host-baseline/1", "status": "SMOKE_ONLY",
              "workload_manifest_sha256": hashlib.sha256(MANIFEST.read_bytes()).hexdigest(),
              "environment": environment(host), "records": [], "observation_samples": [],
              "qualification": {key: {"status": "NOT_RUN", "reason": value["missing"]}
                                for key, value in manifest["scenarios"].items()},
              "limits": {"active_cli_processes": 1, "count": count, "polls_per_run": polls,
                         "run_timeout_seconds": 60, "observation_timeout_seconds": 10},
              "assignment_to_investigation_mapping": "UNESTABLISHED",
              "deferral_observation_supported": False}
    with tempfile.TemporaryDirectory(prefix="mas-p0-") as scratch:
        state = Path(scratch) / "state"
        bundle, code, build_ms = command([str(host), "build", "--project",
                                        str(ROOT / "tests/fixtures/rust-invoice"),
                                        "--sdk-root", str(ROOT)])
        report["build_ms"] = build_ms
        if code != 0 or not isinstance(bundle, dict) or not isinstance(bundle.get("bundle"), str):
            raise RuntimeError("native fixture build failed; no workload qualification established")
        for fixture in generate(count, seed=seed, tenants=1, clusters=1):
            input_path = Path(scratch) / "input.json"
            input_path.write_bytes(encoded(fixture["input"]))
            result, code, elapsed = command([str(host), "run", "--bundle", bundle["bundle"],
                                             "--input", str(input_path), "--state", str(state),
                                             "--run", fixture["request_id"], "--agent", "invoice"])
            bound_result = (result if isinstance(result, dict)
                            and result.get("run_id") == fixture["request_id"] else None)
            expected_output = {"approved": fixture["input"]["amount_minor"] <= fixture["input"]["limit_minor"],
                               "amount_minor": fixture["input"]["amount_minor"],
                               "limit_minor": fixture["input"]["limit_minor"]}
            application_check = "unknown"
            if bound_result is not None:
                application_check = "pass" if bound_result.get("output") == expected_output else "fail"
            snapshots = []
            for _ in range(polls):
                snapshot, observer_code, observer_ms = command([
                    str(host), "observe", "--state", str(state), "--run", fixture["request_id"]], 10)
                valid = (observer_code == 0 and isinstance(snapshot, dict)
                         and snapshot.get("version") == "mas-host-observation/1"
                         and snapshot.get("run_id") == fixture["request_id"]
                         and snapshot.get("payloads_included") is False)
                if valid:
                    snapshots.append(snapshot)
                report["observation_samples"].append({"request_id": fixture["request_id"],
                    "success": valid, "latency_ms": observer_ms,
                    "encoded_projection_bytes": len(encoded(snapshot)) if valid else None})
            outcome = classify(snapshots[-1] if snapshots else None, bound_result, code)
            stable = [dict(s, observed_at_ms=None) for s in snapshots]
            outcome.update({key: fixture[key] for key in ("request_id", "tenant", "cluster", "input_bytes")})
            outcome.update(cli_returncode=code, elapsed_ms=elapsed, application_check=application_check)
            report["records"].append(outcome)
            if len(stable) != polls or any(s != stable[0] for s in stable[1:]):
                report["status"] = "SMOKE_FAILED"
        report["retained_state_bytes"] = sum(p.stat().st_size for p in state.rglob("*") if p.is_file())
    report["totals"] = totals(report["records"])
    report["native_cli_latency"] = latency_summary([r["elapsed_ms"] for r in report["records"]])
    report["observation_cli_latency"] = latency_summary([
        s["latency_ms"] for s in report["observation_samples"] if s["success"]])
    if any(r["execution"] != "completed" or r["application_check"] != "pass" for r in report["records"]):
        report["status"] = "SMOKE_FAILED"
    return report


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=("generate", "smoke"))
    parser.add_argument("--host", type=Path, default=ROOT / "target/debug/mas-host")
    parser.add_argument("--count", type=int, default=3)
    parser.add_argument("--polls", type=int, default=3)
    parser.add_argument("--seed", type=int, default=20261022)
    parser.add_argument("--distribution", choices=("uniform", "skewed-80-percent"), default="uniform")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    result = ({"version": "mas-host-fixtures/1", "qualification_status": "NOT_RUN",
               "workload_manifest_sha256": hashlib.sha256(MANIFEST.read_bytes()).hexdigest(),
               "parameters": {"count": args.count, "seed": args.seed,
                              "distribution": args.distribution},
               "records": generate(args.count, args.seed, args.distribution)}
              if args.mode == "generate" else smoke(args.host, args.count, args.polls, args.seed))
    args.output.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    return int(result.get("status") == "SMOKE_FAILED")


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (ValueError, OSError, RuntimeError) as error:
        print(f"baseline failed: {error}", file=sys.stderr)
        raise SystemExit(1)
