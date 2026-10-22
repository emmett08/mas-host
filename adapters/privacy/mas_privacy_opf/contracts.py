"""Validate the bounded source-correlated worker wire contract."""
import json
import re

MAX_FRAME = 65536
MAX_INPUT = 16384
MAX_FINDINGS = 128
CODE_REVISION = "f7f00ca7fb869683eb732c010299d901457f19c3"
MODEL_REVISION = "7ffa9a043d54d1be65afb281eddf0ffbe629385b"
BACKEND = "opf-cpu-bfloat16/1"
ADAPTER = "mas-privacy-opf/1"
LABELS = frozenset(("account_number", "private_address", "private_email", "private_person",
                    "private_phone", "private_url", "private_date", "secret"))
OMISSIONS = frozenset(("input-token-limit", "unsupported-language", "unsupported-domain", "model-unavailable",
    "model-inference-failed", "incomplete-token-coverage", "decoder-incomplete", "source-reconstruction-failed",
    "invalid-model-output", "detector-limit"))


class AdapterError(Exception):
    """A fixed public reason code, never an underlying exception message."""
    def __init__(self, code):
        self.code = code
        super().__init__(code)


def require(condition, code="invalid-request"):
    if not condition:
        raise AdapterError(code)


def pairs(items):
    result = {}
    for key, value in items:
        require(key not in result)
        result[key] = value
    return result


def parse(raw, limit=MAX_FRAME):
    require(len(raw) <= limit)
    try:
        return json.loads(raw, object_pairs_hook=pairs,
                          parse_constant=lambda _: (_ for _ in ()).throw(AdapterError("invalid-request")))
    except (UnicodeError, ValueError, RecursionError):
        raise AdapterError("invalid-request") from None


def closed(value, fields, code="invalid-request"):
    require(isinstance(value, dict) and set(value) == set(fields), code)


def identifier(value):
    require(isinstance(value, str) and re.fullmatch(r"[A-Za-z0-9_.:/-]{1,128}", value) is not None)


def lower_hex(value, length):
    return isinstance(value, str) and re.fullmatch(r"[0-9a-f]{%d}" % length, value) is not None


def detector(value):
    closed(value, ("bundle_sha256", "code_revision", "model_revision", "backend", "adapter"))
    require(lower_hex(value["bundle_sha256"], 64)
            and value["code_revision"] == CODE_REVISION
            and value["model_revision"] == MODEL_REVISION
            and value["backend"] == BACKEND and value["adapter"] == ADAPTER)


def request(value):
    closed(value, ("version", "source_id", "source_token", "input_bytes", "text", "detector"))
    require(value["version"] == "mas-privacy-worker-request/1")
    identifier(value["source_id"])
    require(lower_hex(value["source_token"], 64))
    require(isinstance(value["text"], str) and type(value["input_bytes"]) is int)
    try:
        require(len(value["text"].encode("utf-8")) == value["input_bytes"] <= MAX_INPUT)
    except UnicodeError:
        raise AdapterError("invalid-request") from None
    detector(value["detector"])
    return value


def response(value, completion, findings=(), omissions=()):
    require(completion in {"complete", "partial", "unsupported", "failed"}, "invalid-response")
    text = value["text"]
    require(len(findings) <= MAX_FINDINGS and len(omissions) <= len(OMISSIONS)
            and len(set(omissions)) == len(omissions) and all(code in OMISSIONS for code in omissions), "invalid-response")
    require((completion == "complete") == (not omissions), "invalid-response")
    if completion in {"failed", "unsupported"}:
        require(not findings, "invalid-response")
    for finding in findings:
        closed(finding, ("start", "end", "label"), "invalid-response")
        require(type(finding["start"]) is int and type(finding["end"]) is int
                and 0 <= finding["start"] < finding["end"] <= len(text)
                and finding["label"] in LABELS, "invalid-response")
    result = {"version": "mas-privacy-worker-response/1", "source_id": value["source_id"],
              "source_token": value["source_token"], "input_bytes": value["input_bytes"],
              "detector": value["detector"], "offset_unit": "unicode-codepoints",
              "completion": completion, "checked_ranges": [], "omissions": list(omissions),
              "findings": list(findings)}
    if completion in {"complete", "partial"} and text:
        result["checked_ranges"] = [{"start": 0, "end": len(text)}]
    encoded = json.dumps(result, ensure_ascii=True, separators=(",", ":")).encode("utf-8")
    require(len(encoded) <= MAX_FRAME, "response-limit")
    return result
