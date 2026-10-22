"""Evaluate exact-source Privacy Filter CLI records against a frozen labelled corpus.

This command owns benchmark evidence only. It grants no handling permission.
Corpus and replay files contain restricted annotation/source-binding metadata.
"""
import argparse
from collections import Counter, defaultdict
import hashlib
import json
import math
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import time

MAX_SOURCE = 16384
MAX_RECORD = 65536
MAX_CORPUS = 268435456
MAX_DOCUMENTS = 100000
CATEGORIES = frozenset(("person", "location", "organisation", "email-address", "phone-number",
    "account-identifier", "credential", "sensitive-context", "postal-address", "url", "date"))
DETECTOR_FIELDS = {"bundle_sha256", "code_revision", "model_revision", "backend", "adapter"}


class QualificationError(ValueError):
    """Fixed error code without source values or underlying process output."""


def require(condition, code="invalid-qualification-record"):
    if not condition:
        raise QualificationError(code)


def digest(raw):
    return hashlib.sha256(raw).hexdigest()


def pairs(items):
    result = {}
    for key, value in items:
        require(key not in result, "duplicate-json-key")
        result[key] = value
    return result


def parse(raw):
    try:
        return json.loads(raw, object_pairs_hook=pairs,
            parse_constant=lambda _: (_ for _ in ()).throw(QualificationError("nonfinite-json")))
    except (ValueError, UnicodeError, RecursionError):
        raise QualificationError("invalid-json") from None


def closed(value, fields):
    require(isinstance(value, dict) and set(value) == set(fields))


def identifier(value):
    require(isinstance(value, str) and re.fullmatch(r"[A-Za-z0-9_.:/-]{1,128}", value) is not None)


def hex64(value):
    require(isinstance(value, str) and re.fullmatch(r"[a-f0-9]{64}", value) is not None)


def rate(value):
    require(type(value) in (int, float) and math.isfinite(value) and 0 <= value <= 1)


def read_bytes(path, limit):
    with Path(path).open("rb") as source:
        raw = source.read(limit + 1)
    require(len(raw) <= limit, "file-limit")
    return raw


def manifest(raw):
    value = parse(raw)
    closed(value, ("version", "corpus_sha256", "detector", "taxonomy", "offset_unit", "confidence",
        "evidence_scope", "annotation_evidence", "sampling_evidence", "independent_units_attested",
        "strata"))
    require(value["version"] == "mas-privacy-qualification-manifest/1")
    hex64(value["corpus_sha256"])
    closed(value["detector"], DETECTOR_FIELDS)
    hex64(value["detector"]["bundle_sha256"])
    for field in ("code_revision", "model_revision"):
        require(isinstance(value["detector"][field], str)
            and re.fullmatch(r"[a-f0-9]{40}", value["detector"][field]) is not None)
    for field in ("backend", "adapter"):
        identifier(value["detector"][field])
    require(value["taxonomy"] == "mas-privacy-taxonomy/2" and value["offset_unit"] == "utf8-bytes")
    rate(value["confidence"])
    require(0 < value["confidence"] < 1)
    require(value["evidence_scope"] in ("synthetic", "deployment-domain"))
    require(type(value["independent_units_attested"]) is bool)
    for field in ("annotation_evidence", "sampling_evidence"):
        require(isinstance(value[field], str) and len(value[field]) <= 256)
    if value["evidence_scope"] == "deployment-domain":
        require(value["annotation_evidence"] and value["sampling_evidence"]
            and value["independent_units_attested"], "missing-domain-evidence")
    require(isinstance(value["strata"], dict) and 1 <= len(value["strata"]) <= 128)
    for name, thresholds in value["strata"].items():
        identifier(name)
        closed(thresholds, ("min_units", "min_negative_units", "max_residual_upper",
            "max_false_block_upper", "min_complete_fraction", "min_required_preserved_fraction"))
        for key in ("min_units", "min_negative_units"):
            require(type(thresholds[key]) is int and 1 <= thresholds[key] <= MAX_DOCUMENTS)
        for key in set(thresholds) - {"min_units", "min_negative_units"}:
            rate(thresholds[key])
    return value


def byte_boundaries(text):
    try:
        raw = text.encode("utf-8")
    except (UnicodeError, AttributeError):
        raise QualificationError("invalid-source-encoding") from None
    boundaries, offset = {0}, 0
    for char in text:
        offset += len(char.encode("utf-8"))
        boundaries.add(offset)
    return raw, boundaries


def span(value, boundaries, fields):
    closed(value, fields)
    require(type(value["start"]) is int and type(value["end"]) is int
        and value["start"] < value["end"] and value["start"] in boundaries and value["end"] in boundaries,
        "invalid-byte-span")


def corpus(raw, config):
    require(digest(raw) == config["corpus_sha256"], "corpus-digest-mismatch")
    result, identifiers = [], set()
    for line in raw.splitlines():
        require(line and len(line) <= MAX_RECORD, "corpus-line-limit")
        item = parse(line)
        closed(item, ("version", "id", "unit_id", "stratum", "text", "prohibited", "required"))
        require(item["version"] == "mas-privacy-labelled-source/1")
        for key in ("id", "unit_id", "stratum"):
            identifier(item[key])
        require(item["id"] not in identifiers, "duplicate-source")
        identifiers.add(item["id"])
        require(item["stratum"] in config["strata"], "undeclared-stratum")
        source, boundaries = byte_boundaries(item["text"])
        require(len(source) <= MAX_SOURCE, "source-limit")
        for key in ("prohibited", "required"):
            require(isinstance(item[key], list) and len(item[key]) <= 128)
        for annotation in item["prohibited"]:
            span(annotation, boundaries, ("start", "end", "category", "entity_id"))
            require(isinstance(annotation["category"], str) and annotation["category"] in CATEGORIES, "unknown-category")
            identifier(annotation["entity_id"])
        require(len({(a["start"], a["end"], a["category"])
            for a in item["prohibited"]}) == len(item["prohibited"]), "duplicate-annotation")
        prohibited_bytes = {b for annotation in item["prohibited"]
            for b in range(annotation["start"], annotation["end"])}
        for annotation in item["required"]:
            span(annotation, boundaries, ("start", "end"))
            require(not prohibited_bytes.intersection(range(annotation["start"], annotation["end"])),
                "contradictory-required-annotation")
        require(len({(a["start"], a["end"]) for a in item["required"]}) == len(item["required"]),
            "duplicate-required-annotation")
        result.append(item)
        require(len(result) <= MAX_DOCUMENTS, "document-limit")
    require(result, "empty-corpus")
    return result


def validate_report(report, item, config):
    require(isinstance(report, dict) and report.get("version") == "mas-privacy-inspection/1"
        and report.get("status") == "assessed", "invalid-inspection-report")
    source, boundaries = byte_boundaries(item["text"])
    require(type(report.get("source_bytes")) is int and report["source_bytes"] == len(source),
        "source-length-mismatch")
    require(report.get("detector") == config["detector"], "detector-mismatch")
    assessment = report.get("restricted_assessment")
    require(isinstance(assessment, dict)
        and assessment.get("version") == "mas-combined-privacy-assessment/1"
        and assessment.get("taxonomy") == config["taxonomy"], "missing-restricted-assessment")
    completion = report.get("completion")
    require(completion in ("complete", "partial", "unsupported", "failed")
        and completion == assessment.get("completion"), "completion-mismatch")
    neural = assessment.get("neural")
    require(isinstance(neural, dict) and neural.get("version") == "mas-neural-assessment/1"
        and neural.get("input_bytes") == len(source) and neural.get("detector") == config["detector"]
        and neural.get("completion") == completion and neural.get("offset_unit") == "utf8-bytes"
        and neural.get("extraction") == "utf8-text/1" and neural.get("normalisation") == "none/1",
        "neural-binding-mismatch")
    ranges = neural.get("checked_ranges")
    require(isinstance(ranges, list) and len(ranges) <= 128)
    checked = set()
    for region in ranges:
        span(region, boundaries, ("start", "end"))
        checked.update(range(region["start"], region["end"]))
    omissions = neural.get("omissions")
    require(isinstance(omissions, list) and all(isinstance(v, str) for v in omissions))
    if completion == "complete":
        require(not omissions and checked == set(range(len(source))), "silent-truncation")
    else:
        require(omissions, "incomplete-without-reason")
    findings = assessment.get("findings")
    require(isinstance(findings, list) and len(findings) <= 128
        and type(report.get("finding_count")) is int and report["finding_count"] == len(findings))
    for finding in findings:
        span(finding, boundaries, ("start", "end", "categories", "recognisers"))
        categories = finding["categories"]
        require(isinstance(categories, list) and categories and all(isinstance(c, str) for c in categories) and len(categories) == len(set(categories))
            and all(isinstance(c, str) and c in CATEGORIES for c in categories), "unknown-category")
        require(isinstance(finding["recognisers"], list) and finding["recognisers"])
        for recogniser in finding["recognisers"]:
            identifier(recogniser)
    return findings, completion


def binomial_upper(failures, trials, confidence=0.95):
    """Exact one-sided Clopper-Pearson upper bound, evaluated in log space."""
    require(type(failures) is int and type(trials) is int and 0 <= failures <= trials)
    require(0 < confidence < 1)
    if trials == 0:
        return None
    if failures == trials:
        return 1.0
    if failures == 0:
        return -math.expm1(math.log1p(-confidence) / trials)
    alpha_log = math.log1p(-confidence)
    def log_cdf(p):
        terms = [math.lgamma(trials + 1) - math.lgamma(k + 1) - math.lgamma(trials - k + 1)
            + k * math.log(p) + (trials - k) * math.log1p(-p) for k in range(failures + 1)]
        largest = max(terms)
        return largest + math.log(sum(math.exp(term - largest) for term in terms))
    lo, hi = failures / trials, 1.0
    for _ in range(64):
        mid = (lo + hi) / 2
        if mid >= 1:
            break
        if log_cdf(mid) > alpha_log:
            lo = mid
        else:
            hi = mid
    return (lo + hi) / 2


def ratio(numerator, denominator):
    return numerator / denominator if denominator else None


def metric_counts(expected, predicted):
    return {"true_positive": len(expected & predicted), "false_positive": len(predicted - expected),
        "false_negative": len(expected - predicted)}


def metric_result(counts):
    tp = counts["true_positive"]
    return {**counts, "precision": ratio(tp, tp + counts["false_positive"]),
        "recall": ratio(tp, tp + counts["false_negative"])}


def evaluate(items, records, config, manifest_sha):
    require(len(items) == len(records), "assessment-count-mismatch")
    strata = {}
    by_stratum = defaultdict(list)
    for item, record in zip(items, records):
        closed(record, ("version", "id", "source_sha256", "manifest_sha256", "latency_ms", "report"))
        require(record["version"] == "mas-privacy-qualified-assessment/1" and record["id"] == item["id"]
            and record["source_sha256"] == digest(item["text"].encode())
            and record["manifest_sha256"] == manifest_sha, "assessment-source-mismatch")
        latency = record["latency_ms"]
        require(type(latency) in (float, int) and math.isfinite(latency) and latency >= 0)
        findings, completion = validate_report(record["report"], item, config)
        by_stratum[item["stratum"]].append((item, findings, completion, latency))
    all_pass = True
    for name, limits in config["strata"].items():
        rows = by_stratum[name]
        units, negative_units, complete = {}, {}, 0
        entities = defaultdict(lambda: [0, 0])
        counts = Counter()
        metrics = defaultdict(lambda: {"utf8_bytes": Counter(), "unicode_codepoints": Counter(), "exact_entities": Counter()})
        for item, findings, completion, _latency in rows:
            local_metrics = defaultdict(lambda: {"expected_bytes": set(), "predicted_bytes": set(),
                "expected_entities": set(), "predicted_entities": set(),
                "expected_characters": set(), "predicted_characters": set()})
            complete += completion == "complete"
            _, character_boundaries = byte_boundaries(item["text"])
            character_starts = sorted(character_boundaries)[:-1]
            detected = {b for f in findings for b in range(f["start"], f["end"])}
            expected = {b for f in item["prohibited"] for b in range(f["start"], f["end"])}
            # The evaluated transformation withholds incomplete input, and removes
            # complete-pass finding spans. This is a benchmark rule, not host release.
            residual = completion == "complete" and bool(expected - detected)
            false_block = completion != "complete"
            units[item["unit_id"]] = units.get(item["unit_id"], False) or residual
            if not expected:
                negative_units[item["unit_id"]] = negative_units.get(item["unit_id"], False) or false_block
            counts["residual_documents"] += residual
            counts["negative_documents"] += not expected
            counts["false_block_documents"] += not expected and false_block
            counts["negative_redaction_documents"] += not expected and completion == "complete" and bool(detected)
            counts["excess_redacted_bytes"] += len(detected - expected)
            counts["source_bytes"] += len(item["text"].encode())
            counts["required_facts"] += len(item["required"])
            for fact in item["required"]:
                counts["required_preserved"] += completion == "complete" and not (detected & set(range(fact["start"], fact["end"])))
            for annotation in item["prohibited"]:
                covered = set(range(annotation["start"], annotation["end"])) <= detected
                key = (item["unit_id"], annotation["category"], annotation["entity_id"])
                entities[key][0] += 1
                entities[key][1] += covered
                category = local_metrics[annotation["category"]]
                category["expected_bytes"].update(b for b in range(annotation["start"], annotation["end"]))
                category["expected_entities"].add((annotation["start"], annotation["end"]))
                category["expected_characters"].update(i for i, b in enumerate(character_starts)
                    if annotation["start"] <= b < annotation["end"])
            for finding in findings:
                for label in finding["categories"]:
                    category = local_metrics[label]
                    category["predicted_bytes"].update(b for b in range(finding["start"], finding["end"]))
                    category["predicted_entities"].add((finding["start"], finding["end"]))
                    category["predicted_characters"].update(i for i, b in enumerate(character_starts)
                        if finding["start"] <= b < finding["end"])
            for label, values in local_metrics.items():
                for dimension, suffix in (("utf8_bytes", "bytes"), ("unicode_codepoints", "characters"), ("exact_entities", "entities")):
                    metrics[label][dimension].update(metric_counts(values["expected_" + suffix], values["predicted_" + suffix]))
        residual_upper = binomial_upper(sum(units.values()), len(units), config["confidence"])
        false_upper = binomial_upper(sum(negative_units.values()), len(negative_units), config["confidence"])
        complete_fraction = ratio(complete, len(rows))
        preserved = ratio(counts["required_preserved"], counts["required_facts"])
        checks = {"minimum_units": len(units) >= limits["min_units"],
            "minimum_negative_units": len(negative_units) >= limits["min_negative_units"],
            "residual_disclosure_bound": residual_upper is not None and residual_upper <= limits["max_residual_upper"],
            "false_block_bound": false_upper is not None and false_upper <= limits["max_false_block_upper"],
            "complete_fraction": complete_fraction is not None and complete_fraction >= limits["min_complete_fraction"],
            "required_fact_preservation": preserved is not None and preserved >= limits["min_required_preserved_fraction"]}
        passed = all(checks.values())
        all_pass &= passed
        latencies = sorted(row[3] for row in rows)
        percentiles = {f"p{p}": latencies[max(0, math.ceil(p / 100 * len(latencies)) - 1)] if latencies else None for p in (50, 95, 99)}
        strata[name] = {"documents": len(rows), "independent_units": len(units), "negative_units": len(negative_units),
            "counts": dict(counts), "completion": dict(Counter(row[2] for row in rows)),
            "residual_unit_upper": residual_upper, "false_block_unit_upper": false_upper,
            "complete_fraction": complete_fraction, "required_preserved_fraction": preserved,
            "repeated_entities": {"groups": sum(a > 1 for a, _ in entities.values()),
                "fully_detected": sum(a > 1 and a == b for a, b in entities.values())},
            "categories": {category: {dimension: metric_result(counts) for dimension, counts in values.items()}
                for category, values in sorted(metrics.items())},
            "latency_ms": percentiles, "summed_inspection_ms": sum(row[3] for row in rows),
            "source_bytes_per_second": counts["source_bytes"] / (sum(row[3] for row in rows) / 1000)
                if rows and sum(row[3] for row in rows) > 0 else None,
            "checks": checks, "thresholds_met": passed}
    status = "THRESHOLDS_NOT_MET" if not all_pass else (
        "SYNTHETIC_CONFORMANCE_ONLY" if config["evidence_scope"] == "synthetic" else "DECLARED_EVALUATION_THRESHOLDS_MET")
    return {"version": "mas-privacy-qualification-result/1", "manifest_sha256": manifest_sha,
        "corpus_sha256": config["corpus_sha256"], "detector": config["detector"],
        "evidence_scope": config["evidence_scope"], "status": status, "thresholds_met": all_pass,
        "release_authority_granted": False, "confidence": config["confidence"],
        "inference_unit": "declared-unit-with-any-document-failure",
        "sampling_and_annotation_evidence_independently_verified": False,
        "assessment_records_sha256": digest(json.dumps(records, sort_keys=True, separators=(",", ":"), allow_nan=False).encode()),
        "runtime_memory_and_cost": "not-measured",
        "strata": strata}


def collect(items, config, manifest_sha, args):
    command = [str(Path(args.host).resolve()), "privacy-inspect", "--input", "-", "--include-details", "true",
        "--python", str(Path(args.python).absolute()), "--manifest", str(Path(args.bundle_manifest).resolve()),
        "--assets", str(Path(args.assets).resolve()), "--worker", str(Path(args.worker).resolve()),
        "--timeout-ms", str(args.timeout_ms)]
    require(digest(read_bytes(args.bundle_manifest, MAX_RECORD)) == config["detector"]["bundle_sha256"], "bundle-digest-mismatch")
    records, total_record_bytes = [], 0
    for item in items:
        started = time.monotonic()
        with tempfile.TemporaryFile() as output:
            try:
                process = subprocess.run(command, input=item["text"].encode(), stdout=output,
                    stderr=subprocess.DEVNULL, timeout=args.timeout_ms / 1000 + 10, check=False)
            except (subprocess.TimeoutExpired, OSError):
                raise QualificationError("inspection-process-unavailable") from None
            require(process.returncode == 0, "inspection-process-failed")
            output.seek(0)
            raw = output.read(MAX_RECORD + 1)
        require(len(raw) <= MAX_RECORD, "inspection-report-limit")
        total_record_bytes += len(raw) + 512
        require(total_record_bytes <= MAX_CORPUS, "aggregate-assessment-limit")
        report = parse(raw)
        validate_report(report, item, config)
        records.append({"version": "mas-privacy-qualified-assessment/1", "id": item["id"],
            "source_sha256": digest(item["text"].encode()), "manifest_sha256": manifest_sha,
            "latency_ms": (time.monotonic() - started) * 1000, "report": report})
    return records


def write_private(path, content):
    # Existing benchmark outputs are also narrowed before writing restricted metadata.
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, "w", encoding="utf-8") as output:
        os.fchmod(output.fileno(), 0o600)
        output.write(content)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", required=True)
    parser.add_argument("--corpus", required=True)
    parser.add_argument("--output", required=True)
    source = parser.add_mutually_exclusive_group(required=True)
    source.add_argument("--replay", help="Restricted JSONL assessment envelope file")
    source.add_argument("--host", help="Built mas-host executable for fresh collection")
    parser.add_argument("--python")
    parser.add_argument("--bundle-manifest")
    parser.add_argument("--assets")
    parser.add_argument("--worker", default=str(Path(__file__).with_name("worker.py")))
    parser.add_argument("--timeout-ms", type=int, default=120000)
    parser.add_argument("--assessments-output", help="Restricted output for later exact-source replay")
    args = parser.parse_args(argv)
    try:
        inputs = {Path(path).resolve() for path in (args.manifest, args.corpus, args.replay, args.bundle_manifest) if path}
        outputs = [Path(path).resolve() for path in (args.output, args.assessments_output) if path]
        require(not inputs.intersection(outputs) and len(set(outputs)) == len(outputs), "output-overlaps-input")
        raw = read_bytes(args.manifest, MAX_RECORD)
        config, manifest_sha = manifest(raw), digest(raw)
        items = corpus(read_bytes(args.corpus, MAX_CORPUS), config)
        if args.replay:
            records = [parse(line) for line in read_bytes(args.replay, MAX_CORPUS).splitlines()]
        else:
            require(args.python and args.bundle_manifest and args.assets, "missing-inspector-configuration")
            require(1 <= args.timeout_ms <= 600000, "invalid-timeout")
            records = collect(items, config, manifest_sha, args)
        result = evaluate(items, records, config, manifest_sha)
        if args.assessments_output:
            # Source texts are never copied. Span and plain source-digest metadata
            # remains restricted benchmark data and needs caller-owned retention.
            write_private(args.assessments_output, "".join(json.dumps(r, separators=(",", ":")) + "\n" for r in records))
        write_private(args.output, json.dumps(result, indent=2, allow_nan=False) + "\n")
        print(json.dumps({"status": result["status"], "thresholds_met": result["thresholds_met"]}))
        return 0 if result["thresholds_met"] else 2
    except (QualificationError, OSError, TypeError, KeyError, OverflowError, RecursionError):
        print(json.dumps({"status": "QUALIFICATION_FAILED", "reason": "invalid-or-unavailable-qualification-evidence"}), file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
