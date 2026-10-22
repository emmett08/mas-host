"""Generate bounded synthetic requests for the canonical Nehemiah workloads."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path


MANIFEST = Path(__file__).with_name("workloads.json")
MAX_REQUESTS = 10000
MAX_INPUT_BYTES = 32768
MAX_FIXTURE_INPUT_BYTES = 64 * 1024 * 1024


def encoded(value: object) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()


def load_manifest() -> dict:
    return json.loads(MANIFEST.read_text(encoding="utf-8"))


def generate(count: int, seed: int = 20261022, distribution: str = "uniform",
             tenants: int = 100, clusters: int = 20,
             payload_bytes: int = 500) -> list[dict]:
    """Tenant labels describe synthetic demand, and supply no host authority.

    The skew has exactly 80% hot-tenant demand in every complete block of ten.
    A partial block retains its deterministic prefix. Payload size is the exact
    encoded input size, separate from the larger request record's encoded size.
    """
    if type(count) is not int or not 1 <= count <= MAX_REQUESTS:
        raise ValueError("count must be 1..10000")
    if type(seed) is not int:
        raise ValueError("seed must be an integer")
    if type(tenants) is not int or not 1 <= tenants <= 10000:
        raise ValueError("tenants must be 1..10000")
    if type(clusters) is not int or not 1 <= clusters <= 10000:
        raise ValueError("clusters must be 1..10000")
    if distribution not in ("uniform", "skewed-80-percent"):
        raise ValueError("unsupported distribution")
    if distribution == "skewed-80-percent" and tenants < 2:
        raise ValueError("skew requires at least two tenants")
    if type(payload_bytes) is not int or not 128 <= payload_bytes <= MAX_INPUT_BYTES:
        raise ValueError("payload_bytes must be 128..32768")
    if count * payload_bytes > MAX_FIXTURE_INPUT_BYTES:
        raise ValueError("aggregate fixture input exceeds 64 MiB")
    identity = {"seed": seed, "distribution": distribution,
                "tenants": tenants, "clusters": clusters, "payload_bytes": payload_bytes}
    prefix = hashlib.sha256(encoded(identity)).hexdigest()[:16]
    records = []
    for index in range(count):
        tenant = index % tenants
        if distribution == "skewed-80-percent":
            tenant = 0 if index % 10 < 8 else 1 + (index // 10 * 2 + index % 10 - 8) % (tenants - 1)
        payload = {"amount_minor": 12500 + index, "limit_minor": 20000, "padding": ""}
        padding = payload_bytes - len(encoded(payload))
        if padding < 0:
            raise ValueError("input does not fit requested payload size")
        payload["padding"] = "x" * padding
        records.append({"request_id": f"p0-{prefix}-{index:05d}",
                        "tenant": f"tenant-{tenant:04d}",
                        "cluster": f"cluster-{index % clusters:04d}",
                        "permission_scope": f"synthetic-scope-{tenant}-{index % 4}",
                        "source_object": f"object-{index % 10}",
                        "input_bytes": len(encoded(payload)), "input": payload})
    return records
