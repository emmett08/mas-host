"""Run paced local host work and independently check its source history."""

from __future__ import annotations

import argparse
import concurrent.futures
import hashlib
import json
import math
from pathlib import Path
import shutil
import sys
import tempfile
import time

from baseline import ROOT, classify, command, environment, latency_summary, totals
from history import check_history
from workload import MANIFEST, encoded, generate, load_manifest


def parameters(count: int, rate: float, occupancy_ms: int, concurrency: int,
               fault_every: int, fault_mode: str) -> None:
    if type(count) is not int or not 1 <= count <= 10000:
        raise ValueError("count must be 1..10000")
    if not isinstance(rate, (float, int)) or isinstance(rate, bool) or not math.isfinite(rate) or not .01 <= rate <= 100:
        raise ValueError("offered rate must be finite and 0.01..100 per second")
    if type(occupancy_ms) is not int or not 0 <= occupancy_ms <= 30000:
        raise ValueError("occupancy_ms must be 0..30000")
    if type(concurrency) is not int or not 1 <= concurrency <= 16:
        raise ValueError("local harness concurrency must be 1..16")
    if type(fault_every) is not int or not 0 <= fault_every <= count:
        raise ValueError("fault_every must be 0..count")
    if fault_mode not in ("refuse-before-effect", "lost-completion"):
        raise ValueError("unsupported fault mode")
    if max((count - 1) / rate, count * occupancy_ms / 1000 / concurrency) > 3600:
        raise ValueError("scheduled demand or minimum occupancy exceeds one hour")


def source_events(path: Path) -> list[dict]:
    if not path.exists():
        return []
    if path.stat().st_size > 65536:
        raise RuntimeError("synthetic per-request ledger exceeded 64 KiB")
    events = [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines()]
    if not all(isinstance(event, dict) for event in events):
        raise RuntimeError("invalid synthetic source event")
    return events


def occupancy_summary(events: list[dict]) -> dict:
    boundaries = []
    starts = {(event.get("request_id"), event.get("effect_id")): event.get("at_ms")
              for event in events if event.get("kind") == "started"}
    for event in events:
        start = starts.get((event.get("request_id"), event.get("effect_id")))
        end = event.get("at_ms")
        if event.get("kind") == "completed" and type(start) is type(end) is int and start < end:
            boundaries.extend(((start, 1), (end, -1)))
    # End events sort first at the same millisecond. This is a lower-resolution
    # source occupancy observation, not a CPU, scheduler slot or provider count.
    occupied = peak = 0
    area = 0
    previous = boundaries[0][0] if boundaries else 0
    for timestamp, delta in sorted(boundaries):
        area += occupied * (timestamp - previous)
        occupied += delta
        peak = max(peak, occupied)
        previous = timestamp
    return {"peak_observed_source_occupancy": peak,
            "source_occupancy_seconds": area / 1000,
            "timestamp_resolution_ms": 1,
            "assignment_seconds": None, "cpu_seconds": None}


def run_request(host: Path, bundle: str, state: Path, scratch: Path,
                fixture: dict, epoch_ns: int, due_ms: float, occupancy_ms: int) -> tuple[dict, list[dict]]:
    started_ms = (time.perf_counter_ns() - epoch_ns) / 1e6
    request = fixture["request_id"]
    ledger = scratch / f"{request}.jsonl"
    payload = {"request_id": request, "source_ledger": str(ledger),
               "occupancy_ms": occupancy_ms, "fault_mode": fixture["fault_mode"]}
    input_path = scratch / f"{request}.json"
    input_path.write_bytes(encoded(payload))
    args = [str(host), "run", "--bundle", bundle, "--input", str(input_path),
            "--state", str(state), "--run", request, "--agent", "occupancy"]
    result, code, elapsed = command(args, 60)
    bound = result if isinstance(result, dict) and result.get("run_id") == request else None
    snapshot, observer_code, observer_ms = command(
        [str(host), "observe", "--state", str(state), "--run", request], 10)
    valid = (observer_code == 0 and isinstance(snapshot, dict)
             and snapshot.get("version") == "mas-host-observation/1"
             and snapshot.get("run_id") == request and snapshot.get("payloads_included") is False)
    outcome = classify(snapshot if valid else None, bound, code)
    retained_effects = ([step for step in snapshot.get("steps", []) if step.get("kind") == "effect"]
                        if valid else [])
    retained_effect = retained_effects[0] if len(retained_effects) == 1 else {}
    outcome.update({key: fixture[key] for key in ("request_id", "tenant", "cluster", "fault_mode")})
    outcome.update(input_bytes=len(encoded(payload)), scheduled_offer_ms=due_ms,
                   harness_started_ms=started_ms, harness_queue_ms=max(0, started_ms - due_ms),
                   elapsed_ms=elapsed, cli_returncode=code,
                   observer_success=valid, observer_ms=observer_ms,
                   host_admission_latency_ms=None, host_eligible_queue_ms=None,
                   retained_effect_id=retained_effect.get("id"),
                   retained_effect_state=retained_effect.get("state"),
                   effect_id=bound.get("output", {}).get("effect_id") if bound else None,
                   application_check=("pass" if bound and bound.get("output", {}).get("request_id") == request
                                      and bound.get("output", {}).get("done") is True else "unavailable"))
    before = source_events(ledger)
    if fixture["fault_mode"] == "lost-completion":
        replay, replay_code, replay_ms = command(args, 60)
        after = source_events(ledger)
        outcome.update(replay_returncode=replay_code, replay_elapsed_ms=replay_ms,
                       replay_source_unchanged=before == after,
                       replay_claimed_completed=(isinstance(replay, dict) and replay.get("status") == "completed"))
        if outcome["replay_claimed_completed"]:
            outcome["execution"] = "completed"
        return outcome, after
    return outcome, before


def paced(host: Path, count: int, rate: float, occupancy_ms: int, concurrency: int,
          seed: int, distribution: str, fault_every: int = 0,
          fault_mode: str = "lost-completion") -> dict:
    parameters(count, rate, occupancy_ms, concurrency, fault_every, fault_mode)
    host = host.resolve(strict=True)
    offers = generate(count, seed=seed, distribution=distribution)
    workload_identity = hashlib.sha256(encoded({"version": "mas-host-paced-baseline/1",
        "count": count, "rate": rate, "occupancy_ms": occupancy_ms,
        "concurrency": concurrency, "seed": seed, "distribution": distribution,
        "fault_every": fault_every, "fault_mode": fault_mode})).hexdigest()[:16]
    for index, offer in enumerate(offers):
        offer["request_id"] = f"paced-{workload_identity}-{index:05d}"
        offer["fault_mode"] = fault_mode if fault_every and (index + 1) % fault_every == 0 else "success"
    manifest = load_manifest()
    report = {"version": "mas-host-paced-baseline/1", "status": "LOCAL_EXPERIMENT_ONLY",
              "environment": environment(host),
              "workload_manifest_sha256": hashlib.sha256(MANIFEST.read_bytes()).hexdigest(),
              "parameters": {"count": count, "offered_rate_per_second": rate,
                             "occupancy_ms": occupancy_ms, "concurrency": concurrency,
                             "seed": seed, "distribution": distribution,
                             "fault_every": fault_every, "fault_mode": fault_mode},
              "limits": {"maximum_requests": 10000, "maximum_active_cli": concurrency,
                         "maximum_harness_pending": count, "maximum_cli_seconds": 60,
                         "maximum_source_ledger_bytes_per_request": 65536,
                         "maximum_aggregate_source_ledger_bytes": count * 65536},
              "assignment_to_investigation_mapping": "UNESTABLISHED",
              "fixture_mapping": "one original request, one root worker, one admitted synthetic effect",
              "host_queued_service_exercised": False, "records": [], "source_history": [],
              "qualification": {key: {"status": "NOT_RUN", "reason": value["missing"]}
                                for key, value in manifest["scenarios"].items()}}
    report["experiment_source_sha256"] = {
        str(path.relative_to(ROOT)): hashlib.sha256(path.read_bytes()).hexdigest()
        for path in (Path(__file__), Path(__file__).with_name("history.py"),
                     *sorted(Path(__file__).with_name("fixture").glob("*")))
        if path.is_file()}
    with tempfile.TemporaryDirectory(prefix="mas-p0-paced-") as directory:
        scratch = Path(directory)
        state = scratch / "state"
        pinned_host = scratch / "mas-host"
        shutil.copyfile(host, pinned_host)
        pinned_host.chmod(0o700)
        report["host_snapshot_sha256"] = hashlib.sha256(pinned_host.read_bytes()).hexdigest()
        if report["host_snapshot_sha256"] != report["environment"]["host_sha256"]:
            raise RuntimeError("host executable changed while pinning the experiment")
        host = pinned_host
        built, code, build_ms = command([str(host), "build", "--project",
            str(Path(__file__).with_name("fixture")), "--sdk-root", str(ROOT)])
        report["build_ms"] = build_ms
        if code != 0 or not isinstance(built, dict) or not isinstance(built.get("bundle"), str):
            raise RuntimeError("occupancy fixture build failed")
        report["fixture_bundle_sha256"] = hashlib.sha256(Path(built["bundle"]).read_bytes()).hexdigest()
        epoch_ns = time.perf_counter_ns()
        with concurrent.futures.ThreadPoolExecutor(max_workers=concurrency) as executor:
            futures = []
            for index, fixture in enumerate(offers):
                due_ms = index * 1000 / rate
                remaining = due_ms / 1000 - (time.perf_counter_ns() - epoch_ns) / 1e9
                # Keep each wait bounded so interruption remains responsive.
                while remaining > 0:
                    time.sleep(min(remaining, .25))
                    remaining = due_ms / 1000 - (time.perf_counter_ns() - epoch_ns) / 1e9
                futures.append(executor.submit(run_request, host, built["bundle"], state,
                    scratch, fixture, epoch_ns, due_ms, occupancy_ms))
            for future in futures:
                record, history = future.result()
                report["records"].append(record)
                report["source_history"].extend(history)
        report["elapsed_ms"] = (time.perf_counter_ns() - epoch_ns) / 1e6
        report["retained_state_bytes"] = sum(path.stat().st_size for path in state.rglob("*") if path.is_file())
        report["history_check"] = check_history(offers, report["records"], report["source_history"])
    report["totals"] = totals(report["records"])
    report["observed_source_occupancy"] = occupancy_summary(report["source_history"])
    report["native_cli_latency"] = latency_summary([record["elapsed_ms"] for record in report["records"]])
    report["harness_queue_latency"] = latency_summary([record["harness_queue_ms"] for record in report["records"]])
    report["successful_completion_rate_per_second"] = report["totals"]["execution"]["completed"] / (report["elapsed_ms"] / 1000)
    if report["history_check"]["status"] != "PASS":
        report["status"] = "LOCAL_EXPERIMENT_FAILED"
    return report


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", type=Path, default=ROOT / "target/debug/mas-host")
    parser.add_argument("--count", type=int, default=3)
    parser.add_argument("--rate", type=float, default=.25)
    parser.add_argument("--occupancy-ms", type=int, default=30000)
    parser.add_argument("--concurrency", type=int, default=16)
    parser.add_argument("--seed", type=int, default=20261022)
    parser.add_argument("--distribution", choices=("uniform", "skewed-80-percent"), default="uniform")
    parser.add_argument("--fault-every", type=int, default=0)
    parser.add_argument("--fault-mode", choices=("refuse-before-effect", "lost-completion"), default="lost-completion")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    result = paced(args.host, args.count, args.rate, args.occupancy_ms, args.concurrency,
                   args.seed, args.distribution, args.fault_every, args.fault_mode)
    args.output.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    return int(result["status"] == "LOCAL_EXPERIMENT_FAILED")


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (ValueError, OSError, RuntimeError) as error:
        print(f"paced baseline failed: {error}", file=sys.stderr)
        raise SystemExit(1)
