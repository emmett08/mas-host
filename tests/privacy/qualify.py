"""Qualify the local neural CLI on an explicitly synthetic, labelled corpus."""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import platform
import resource
import signal
import subprocess
import time


class QualificationFailure(Exception):
    """A fixed failure code that cannot contain source content."""


def require(condition, code):
    if not condition:
        raise QualificationFailure(code)


def digest(path):
    result = hashlib.sha256()
    with Path(path).open("rb") as stream:
        while block := stream.read(1048576):
            result.update(block)
    return result.hexdigest()


def unique_pairs(pairs):
    value = {}
    for key, item in pairs:
        require(key not in value, "duplicate-json-field")
        value[key] = item
    return value


def parse(raw):
    try:
        return json.loads(raw, object_pairs_hook=unique_pairs,
                          parse_constant=lambda _: (_ for _ in ()).throw(
                              QualificationFailure("nonfinite-json")))
    except (UnicodeError, ValueError, RecursionError):
        raise QualificationFailure("invalid-json") from None


def strings(value):
    if isinstance(value, str):
        yield value
    elif isinstance(value, dict):
        for key, item in value.items():
            yield key
            yield from strings(item)
    elif isinstance(value, list):
        for item in value:
            yield from strings(item)


def validate_report(raw, errors, expected_detector, case):
    require(len(raw) <= 262144 and len(errors) <= 65536, "output-limit")
    require(not errors, "unexpected-stderr")
    value = parse(raw)
    text = case["text"]
    source = text.encode("utf-8")
    require(value["version"] == "mas-privacy-inspection/1"
            and value["status"] == "assessed", "inspection-state")
    require(value["source_bytes"] == len(source), "source-length")
    require(value["completion"] == "complete", "incomplete-assessment")
    require(value["release_permitted"] is False
            and value["quality_qualified"] is False
            and value["disposition"] in {"hold", "withhold"}, "release-authority")
    require(value["detector"] == expected_detector, "detector-binding")
    restricted = value["restricted_assessment"]
    require(restricted["completion"] == "complete"
            and restricted["release_qualified"] is False, "combined-state")
    require(type(value["finding_count"]) is int
            and value["finding_count"] == len(restricted["findings"]), "finding-count")
    neural = restricted["neural"]
    require(neural["detector"] == expected_detector
            and neural["input_bytes"] == len(source), "neural-source-binding")
    require(neural["completion"] == "complete" and neural["omissions"] == []
            and neural["offset_unit"] == "utf8-bytes"
            and neural["score_semantics"] == "not-exposed", "neural-state")
    expected_ranges = [{"start": 0, "end": len(source)}] if source else []
    require(neural["checked_ranges"] == expected_ranges, "coverage-gap")
    findings = neural["findings"]
    for finding in findings:
        start, end = finding["start"], finding["end"]
        require(type(start) is int and type(end) is int
                and 0 <= start < end <= len(source), "invalid-byte-range")
        try:
            source[:start].decode("utf-8")
            source[:end].decode("utf-8")
        except UnicodeError:
            raise QualificationFailure("split-utf8-codepoint") from None
    checks = []
    forbidden = [text] if text else []
    for entity in case["expected_entities"]:
        start, end, label = entity["start"], entity["end"], entity["label"]
        require(type(start) is int and type(end) is int
                and 0 <= start < end <= len(source), "invalid-corpus-annotation")
        forbidden.append(source[start:end].decode("utf-8"))
        covered = any(item["label"] == label and item["start"] <= start
                      and item["end"] >= end for item in findings)
        checks.append({"label": label, "start": start, "end": end, "covered": covered})
    leaves = tuple(strings(value))
    require(not any(secret in leaf for secret in forbidden for leaf in leaves), "source-leaked")
    require(all(item["covered"] for item in checks), "expected-entity-missed")
    require(len(findings) >= case["minimum_findings"], "too-few-findings")
    if case["expect_no_findings"]:
        require(not findings, "unexpected-finding")
    return {"completion": "complete", "disposition": value["disposition"],
            "release_permitted": False, "neural_finding_count": len(findings),
            "combined_finding_count": value["finding_count"],
            "expected_entity_checks": checks, "findings": findings}


def run_case(command, text, timeout_seconds):
    # The host's guardian owns the worker deadline and descendant cleanup. This
    # outer process-group deadline additionally bounds a stuck CLI invocation.
    process = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                               stderr=subprocess.PIPE, start_new_session=True)
    try:
        stdout, stderr = process.communicate(text.encode("utf-8"), timeout=timeout_seconds)
    except subprocess.TimeoutExpired:
        os.killpg(process.pid, signal.SIGTERM)
        try:
            process.communicate(timeout=2)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            process.communicate()
        raise QualificationFailure("cli-deadline") from None
    require(process.returncode == 0, "cli-failed")
    return stdout, stderr


def receipt_state(expected_ids, cases):
    if any(case["status"] == "FAIL" for case in cases):
        return "FAIL"
    completed = [case["id"] for case in cases if case["status"] == "PASS"]
    if expected_ids and len(completed) == len(expected_ids) and set(completed) == set(expected_ids):
        return "PASS"
    return "RUNNING"


def write_receipt(path, receipt):
    temporary = path.with_name(path.name + f".{os.getpid()}.tmp")
    try:
        temporary.write_text(json.dumps(receipt, indent=2) + "\n")
        temporary.replace(path)
    finally:
        temporary.unlink(missing_ok=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", required=True, type=Path)
    parser.add_argument("--python", required=True, type=Path)
    parser.add_argument("--manifest", required=True, type=Path)
    parser.add_argument("--assets", required=True, type=Path)
    parser.add_argument("--corpus", type=Path, default=Path(__file__).with_name("corpus.json"))
    parser.add_argument("--receipt", required=True, type=Path)
    parser.add_argument("--case-timeout-seconds", type=int, default=180, choices=range(1, 601), metavar="1..600")
    parser.add_argument("--memory-mib", type=int, default=12288)
    args = parser.parse_args()
    receipt = {"version": "mas-privacy-local-qualification/1", "status": "RUNNING",
               "scope": "synthetic-cli-conformance", "population_quality_qualified": False,
               "recorded_at": datetime.now(timezone.utc).isoformat(),
               "expected_case_count": None, "completed_case_count": 0, "cases": []}
    write_receipt(args.receipt, receipt)
    try:
        return qualify(args, receipt)
    except QualificationFailure as failure:
        receipt.update(status="FAIL", reason=str(failure))
    except KeyboardInterrupt:
        receipt.update(status="FAIL", reason="qualification-interrupted")
    except (KeyError, TypeError, ValueError, OSError):
        receipt.update(status="FAIL", reason="qualification-incomplete")
    write_receipt(args.receipt, receipt)
    return 1


def qualify(args, receipt):
    require(256 <= args.memory_mib <= 65536, "invalid-memory-limit")
    host, manifest_path, assets = args.host.resolve(), args.manifest.resolve(), args.assets.resolve()
    # Preserve the venv path. Resolving this symlink would select the base Python.
    python = args.python.absolute()
    corpus = parse(args.corpus.read_bytes())
    require(corpus["version"] == "mas-privacy-synthetic-corpus/1"
            and 1 <= len(corpus["cases"]) <= 32, "invalid-corpus")
    expected_ids = [case["id"] for case in corpus["cases"]]
    require(all(isinstance(name, str) and 0 < len(name) <= 64 for name in expected_ids)
            and len(set(expected_ids)) == len(expected_ids), "invalid-corpus-id")
    manifest = parse(manifest_path.read_bytes())
    expected_detector = {"bundle_sha256": digest(manifest_path), **{
        name: manifest[name] for name in ("code_revision", "model_revision", "backend", "adapter")}}
    receipt.update({"expected_case_count": len(expected_ids), "expected_case_ids": expected_ids,
               "bindings": {"harness_sha256": digest(__file__), "corpus_sha256": digest(args.corpus),
                            "host_sha256": digest(host), "python_executable_sha256": digest(python),
                            "worker_sha256": manifest["worker_sha256"], "detector": expected_detector},
               "environment": {"platform": platform.platform(), "harness_python": platform.python_version()},
               "limits": {"worker_timeout_ms": args.case_timeout_seconds * 1000,
                          "cli_grace_seconds": 10, "memory_mib": args.memory_mib}})
    write_receipt(args.receipt, receipt)
    command = [str(host), "privacy-inspect", "--input", "-", "--python", str(python),
               "--manifest", str(manifest_path), "--assets", str(assets), "--include-details", "true",
               "--timeout-ms", str(args.case_timeout_seconds * 1000), "--memory-mib", str(args.memory_mib)]
    for case in corpus["cases"]:
        case_id, text = case["id"], case["text"]
        require(len(text.encode("utf-8")) <= 16384, "corpus-byte-limit")
        before = time.monotonic()
        result = {"id": case_id, "input_sha256": hashlib.sha256(text.encode()).hexdigest(),
                  "input_bytes": len(text.encode()), "status": "PASS"}
        try:
            stdout, stderr = run_case(command, text, args.case_timeout_seconds + 10)
            result.update(validate_report(stdout, stderr, expected_detector, case))
        except QualificationFailure as failure:
            result["status"] = "FAIL"
            result["reason"] = str(failure)
        except (KeyError, TypeError, ValueError):
            result["status"] = "FAIL"
            result["reason"] = "invalid-inspection-record"
        result["seconds"] = round(time.monotonic() - before, 4)
        receipt["cases"].append(result)
        receipt["completed_case_count"] = len(receipt["cases"])
        receipt["status"] = receipt_state(expected_ids, receipt["cases"])
        receipt["max_child_rss_kib"] = resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss
        # Verify identity again before retaining observations against these bytes.
        require(digest(__file__) == receipt["bindings"]["harness_sha256"]
                and digest(args.corpus) == receipt["bindings"]["corpus_sha256"]
                and digest(host) == receipt["bindings"]["host_sha256"]
                and digest(manifest_path) == expected_detector["bundle_sha256"], "qualification-input-changed")
        write_receipt(args.receipt, receipt)
        print(json.dumps({"id": case_id, "status": result["status"], "seconds": result["seconds"]}), flush=True)
        if result["status"] != "PASS":
            break
    return 0 if receipt["status"] == "PASS" else 1


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except QualificationFailure:
        print("privacy qualification rejected its inputs", file=__import__("sys").stderr)
        raise SystemExit(1) from None
