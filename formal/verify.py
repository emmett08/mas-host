#!/usr/bin/env python3
"""Execute bounded SPIN controls, mutations and reachability witnesses."""
from __future__ import annotations

import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import resource
import shutil
import subprocess
import tempfile
import zipfile

ROOT = Path(__file__).resolve().parent
DEPTH = 100_000
SECONDS = 30
MEMORY_BYTES = 512 * 1024 * 1024
CASES = []
for model, claim in (("revision", "settles"), ("disclosure", "context_finishes"),
                     ("delegation", "delegation_finishes"),
                     ("provider-results", "provider_results_settle")):
    for ltl in (False, True):
        for no_reduce in (False, True):
            CASES.append((model, f"{'progress' if ltl else 'safety'}{'-no-reduction' if no_reduce else ''}",
                          None, claim if ltl else None, no_reduce, None))
for name, macro, assertion in (
    ("unknown-retry", "BUG_UNKNOWN_RETRY", "effect_dispatches"),
    ("completed-replay", "BUG_REPLAY", "effect_dispatches"),
    ("assessment-replay", "BUG_ASSESSMENT_REPLAY", "decision"),
    ("error-pass", "BUG_ERROR_PASS", "terminal!=Completed"),
    ("revision-budget", "BUG_BUDGET", "revisions<=1"),
    ("permission-expansion", "BUG_PERMISSION", "permitted_effects==1"),
    ("deadline-expansion", "BUG_DEADLINE", "active_deadline==frozen_deadline"),
    ("remediation-effect", "BUG_REMEDIATION", "remediation_effects==0"),
    ("complete-witness", "WITNESS_COMPLETE", "terminal!=Completed"),
    ("replay-witness", "WITNESS_REPLAY", "!(replayed)"),
    ("unknown-witness", "WITNESS_UNKNOWN", "!(unknown_seen)"),
    ("exhausted-witness", "WITNESS_EXHAUSTED", "terminal!=Exhausted"),
    ("error-witness", "WITNESS_ERROR", "!(retained_error)"),
    ("assessment-replay-witness", "WITNESS_ASSESSMENT_REPLAY", "!(assessment_replayed)"),
    ("invocation-budget-witness", "WITNESS_INVOCATION_BUDGET", "!(budget_exhausted)"),
):
    CASES.append(("revision", name, macro, None, False, assertion))
CASES.append(("revision", "stalled-controller", "BUG_STALL", "settles", False, "acceptance cycle"))
for name, macro, assertion in (
    ("hidden-peer", "BUG_HIDDEN_PEER", "reviewer_input_a==reviewer_input_b"),
    ("reassemble-replay", "BUG_REASSEMBLE", "reviewer_input_a==snapshot_a"),
    ("policy-rebind", "BUG_POLICY_REBIND", "rejected_rebind"),
    ("derived-reader", "BUG_DERIVED_READER", "!(output_reader)"),
    ("restricted-witness", "WITNESS_RESTRICTED", "!(supplied_feedback)"),
    ("delegation-disclosure", "BUG_DELEGATION_DISCLOSURE", "!(delegated)"),
    ("delegation-inheritance", "BUG_DELEGATION_INHERITANCE", "!(inherited_reader)"),
    ("evidence-namespace", "BUG_EVIDENCE_NAMESPACE", "!(draft_dispatched)"),
    ("evidence-readers", "BUG_EVIDENCE_READERS", "!(candidate_evidence_reader)"),
):
    CASES.append(("disclosure", name, macro, None, False, assertion))
for name, macro, assertion in (
    ("early-complete", "BUG_EARLY_COMPLETE", "successful>=minimum"),
    ("early-complete-schema", "BUG_SCHEMA", "complete_allowed=="),
    ("partial-coverage-complete", "BUG_PARTIAL_COMPLETE", "covered&required"),
    ("required-target-schema", "BUG_TARGET_SCHEMA", "permitted_requests=="),
    ("duplicate-target-schema", "BUG_DUPLICATE_TARGET_SCHEMA", "permitted_requests=="),
    ("final-turn-request", "BUG_FINAL_REQUEST", "request_allowed=="),
    ("replay-count", "BUG_REPLAY_COUNT", "successful==retained_successful"),
    ("replay-coverage", "BUG_REPLAY_COVERAGE", "covered==retained_covered"),
    ("replay-dispatch", "BUG_REPLAY_DISPATCH", "peer_dispatches==saved_peer_dispatches"),
    ("complete-witness", "WITNESS_COMPLETE", "terminal==Completed"),
    ("zero-minimum-witness", "WITNESS_ZERO", "minimum==0"),
    ("replay-witness", "WITNESS_REPLAY", "replayed"),
    ("early-invalid-witness", "WITNESS_EARLY_INVALID", "terminal==Invalid"),
    ("partial-coverage-invalid-witness", "WITNESS_PARTIAL_INVALID", "successful==1"),
    ("duplicate-target-invalid-witness", "WITNESS_DUPLICATE_INVALID", "duplicate_rejected"),
    ("reverse-order-witness", "WITNESS_REVERSE_ORDER", "first_peer==2"),
):
    CASES.append(("delegation", name, macro, None, False, assertion))
CASES.append(("delegation", "stalled-inference", "BUG_STALL", "delegation_finishes",
              False, "acceptance cycle"))
for name, macro, assertion in (
    ("first-output", "BUG_FIRST_OUTPUT", "selected=="),
    ("ignore-malformed-sibling", "BUG_MALFORMED_SIBLING", "selected=="),
    ("commentary-promotion", "BUG_COMMENTARY", "selected=="),
    ("ignore-refusal", "BUG_REFUSAL", "selected=="),
    ("reject-equivalent-messages", "BUG_REJECT_REPEATED", "selected=="),
    ("skip-schema", "BUG_SCHEMA", "peer_calls==prior_peer_calls"),
    ("skip-request-admission", "BUG_REQUEST_ADMISSION", "peer_calls==prior_peer_calls"),
    ("known-invalid-becomes-unknown", "BUG_INVALID_UNKNOWN", "inference_state==Known"),
    ("inference-replay", "BUG_REPLAY", "inference_calls"),
    ("peer-replay", "BUG_PEER_REPLAY", "peer_calls"),
    ("invalid-retry", "BUG_INVALID_RETRY", "inference_calls"),
    ("unknown-retry", "BUG_UNKNOWN_RETRY", "inference_calls"),
    ("reclassify-retained-invalid", "BUG_RECLASSIFY", "retained_status==sealed_status"),
    ("descriptor-rebind", "BUG_DESCRIPTOR_REBIND", "frozen_descriptor"),
    ("single-witness", "WITNESS_SINGLE", "response_kind==Single"),
    ("repeated-witness", "WITNESS_REPEATED", "response_kind==Repeated"),
    ("triplicated-witness", "WITNESS_TRIPLICATED", "response_kind==Triplicated"),
    ("conflict-after-peer-witness", "WITNESS_CONFLICT_AFTER_PEER", "response_kind==Conflicting"),
    ("malformed-witness", "WITNESS_MALFORMED", "response_kind==MalformedSibling"),
    ("schema-invalid-witness", "WITNESS_SCHEMA_INVALID", "response_kind==RepeatedSchemaInvalid"),
    ("request-invalid-witness", "WITNESS_REQUEST_INVALID", "response_kind==RequestInvalid"),
    ("refusal-witness", "WITNESS_REFUSAL", "response_kind==Refused"),
    ("unknown-witness", "WITNESS_UNKNOWN", "terminal==Blocked"),
    ("lost-known-witness", "WITNESS_LOST_KNOWN", "transient_known_lost"),
    ("cached-inference-witness", "WITNESS_CACHED_INFERENCE", "cached_inference"),
    ("delegate-crash-witness", "WITNESS_DELEGATE_CRASH", "delegate_state==Uncertain"),
    ("descriptor-rebind-witness", "WITNESS_REBIND", "!(rebind_rejected)"),
):
    CASES.append(("provider-results", name, macro, None, False, assertion))
CASES.append(("provider-results", "stalled-provider", "BUG_STALL", "provider_results_settle",
              False, "acceptance cycle"))


def bounded() -> None:
    resource.setrlimit(resource.RLIMIT_AS, (MEMORY_BYTES, MEMORY_BYTES))


def command(argv: list[str], cwd: Path, logs: list[dict]) -> str:
    result = subprocess.run(argv, cwd=cwd, text=True, capture_output=True,
                            timeout=SECONDS, preexec_fn=bounded)
    output = result.stdout + result.stderr
    logs.append({"argv": argv, "exit_code": result.returncode, "output": output})
    if result.returncode:
        raise RuntimeError(f"{argv[0]} exited {result.returncode}")
    return output


def sha(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, default=ROOT / "evidence")
    args = parser.parse_args()
    output = args.out.resolve()
    output.mkdir(parents=True, exist_ok=True)
    output_lock = (output / ".verification.lock").open("a")
    try:
        fcntl.flock(output_lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError:
        parser.error("another verifier owns this output directory; choose another --out")
    summary = {"schema": "mas-host-assessment-formal/1", "status": "RUNNING_OR_INTERRUPTED",
               "model_sha256": {p.name: sha(p) for p in sorted(ROOT.glob("*.pml"))},
               "runner_sha256": sha(Path(__file__)),
               "limits": {"seconds_per_command": SECONDS, "memory_bytes": MEMORY_BYTES,
                          "search_depth": DEPTH},
               "bounds": {"runs": 1, "candidates": 2, "invocations": 4,
                          "revisions": 1, "disruptions": 1, "disclosure_worlds": 2,
                          "delegation": {"inference_turns": 3, "permitted_peers": 2,
                                         "required_peer_subsets": 4,
                                         "minimum_calls": [0, 1, 2], "replays": 1},
                          "provider_results": {"inference_effects": 1, "messages": 3,
                                               "downstream_effects": 1,
                                               "retained_earlier_peers": [0, 1],
                                               "crashes": 1, "terminal_replays": 1}},
               "assumptions": [
                   "One host owner serialises this run and immutable journal writes are atomic",
                   "A provider invocation terminates or reaches its fixed external deadline",
                   "At most one crash, cancellation or expiry observation; storage survives",
                   "An interrupted started external effect is unresolved and not retried; pure handler computation may restart",
                   "Deadline values are abstract observations; no wall-clock or scheduling bound is proved",
                   "Two disclosure worlds differ only in withheld peer findings",
                   "Disclosure applies only to host-managed inputs under the trusted-worker boundary",
                   "Provider result values abstract exact parsed JSON; schema and request admissibility are separate inputs",
                   "Provider result admission covers one current logical decision, not semantic deduplication across turns",
                   "No unmediated files, provider memory, traffic timing, hash collision or storage adversary",
                   "No fairness options, approximate hashing or disabled bounds/assertion checks",
                   "This is finite control checking plus implementation correspondence, not a refinement proof",
               ], "checks": []}
    summary_path = output / "summary.json"
    summary_path.write_text(json.dumps(summary, indent=2) + "\n")
    spin = shutil.which("spin")
    compiler = shutil.which(os.environ.get("CC", "cc"))
    if not spin or not compiler:
        summary.update(status="NOT_EXECUTED", reason="SPIN and a C compiler are required")
        summary_path.write_text(json.dumps(summary, indent=2) + "\n")
        return 1
    logs = {"versions": []}
    summary["spin"] = command([spin, "-V"], ROOT, logs["versions"]).strip()
    summary["compiler"] = command([compiler, "--version"], ROOT, logs["versions"]).splitlines()[0]
    archive_workspace = tempfile.TemporaryDirectory(prefix="mas-host-spin-evidence-")
    archive_path = Path(archive_workspace.name) / "counterexamples.zip"
    with zipfile.ZipFile(archive_path, "w", zipfile.ZIP_DEFLATED):
        pass
    for model, name, macro, claim, no_reduce, expected in CASES:
        key = f"{model}-{name}"
        entries = logs[key] = []
        check = {"name": key, "model": f"{model}.pml", "macro": macro,
                 "claim": claim, "partial_order_reduction": not no_reduce,
                 "expected": "counterexample" if expected else "zero_errors"}
        try:
            with tempfile.TemporaryDirectory(prefix="mas-host-spin-") as directory:
                work = Path(directory)
                source = f"{model}.pml"
                shutil.copyfile(ROOT / source, work / source)
                defines = [f"-D{macro}"] if macro else []
                if name == "safety":
                    for seed in (17, 29):
                        command([spin, f"-n{seed}", "-u1000", "-p", *defines, source], work, entries)
                command([spin, *defines, "-a", source], work, entries)
                flags = ["-O2", "-DMEMLIM=512"]
                if not claim:
                    flags += ["-DSAFETY", "-DNOCLAIM"]
                if no_reduce:
                    flags += ["-DNOREDUCE"]
                command([compiler, *flags, "-o", "pan", "pan.c"], work, entries)
                options = ["-a", "-N", claim] if claim else []
                options += ["-b", f"-m{DEPTH}", "-w20"]
                result = command([str(work / "pan"), *options], work, entries)
                errors = re.search(r"errors:\s*(\d+)", result)
                if not errors:
                    raise RuntimeError("verifier supplied no error count")
                if any(t in result for t in ("max search depth too small", "out of memory")):
                    raise RuntimeError("search depth or memory exhausted")
                if not expected and "Search not completed" in result:
                    raise RuntimeError("positive search was incomplete")
                if expected:
                    if int(errors[1]) == 0:
                        raise RuntimeError("mutation/witness did not produce a counterexample")
                    replay = command([spin, *defines, "-t", "-p", "-g", "-l", source], work, entries)
                    if claim:
                        if "acceptance cycle" not in result or "START OF CYCLE" not in replay:
                            raise RuntimeError("expected liveness cycle did not replay")
                    else:
                        failed = re.search(r"text of failed assertion: (.+)", replay)
                        if not failed or expected not in re.sub(r"\s+", "", failed[1]):
                            raise RuntimeError(f"counterexample missed intended assertion: {expected}")
                    trail = work / f"{source}.trail"
                    if not trail.exists():
                        raise RuntimeError("no replayable trail")
                    with zipfile.ZipFile(archive_path, "a", zipfile.ZIP_DEFLATED) as archive:
                        for file in [work / source, trail, *sorted(work.glob("pan.*"))]:
                            archive.write(file, f"{key}/{file.name}")
                        archive.writestr(f"{key}/configuration.json", json.dumps({
                            "defines": defines, "compiler_flags": flags,
                            "verifier_options": options,
                        }, indent=2) + "\n")
                    check["result"] = "EXPECTED_COUNTEREXAMPLE"
                else:
                    if int(errors[1]):
                        raise RuntimeError("positive model produced a counterexample")
                    check["result"] = "COMPLETE_ZERO_ERRORS"
                check["errors"] = int(errors[1])
                for label, pattern in (("stored_states", r"([\d.e+]+) states, stored"),
                                       ("transitions", r"([\d.e+]+) transitions"),
                                       ("depth_reached", r"depth reached (\d+)")):
                    match = re.search(pattern, result)
                    check[label] = match[1] if match else None
        except (RuntimeError, OSError, subprocess.TimeoutExpired) as error:
            check.update(result="FAILED_OR_INCOMPLETE", reason=str(error))
        summary["checks"].append(check)
        print(f"{key}: {check['result']}", flush=True)
    changed = [name for name, digest in summary["model_sha256"].items()
               if sha(ROOT / name) != digest]
    if sha(Path(__file__)) != summary["runner_sha256"]:
        changed.append("verify.py")
    summary["source_changed_during_verification"] = changed
    summary["status"] = "PASS" if not changed and all(
        item["result"] != "FAILED_OR_INCOMPLETE" for item in summary["checks"]) else "FAIL"
    with zipfile.ZipFile(archive_path) as archive:
        names = archive.namelist()
        if len(names) != len(set(names)):
            summary["status"] = "FAIL"
            summary["archive_error"] = "duplicate evidence names"
    shutil.copyfile(archive_path, output / "counterexamples.zip")
    archive_workspace.cleanup()
    (output / "commands.json").write_text(json.dumps(logs, indent=2) + "\n")
    summary_path.write_text(json.dumps(summary, indent=2) + "\n")
    return 0 if summary["status"] == "PASS" else 1


if __name__ == "__main__":
    raise SystemExit(main())
