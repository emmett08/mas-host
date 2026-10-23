#!/usr/bin/env python3
"""Read-only cluster precondition checks; runtime claims stay explicitly open."""
from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import subprocess
import sys

from prepare import load_json, validate


def run(kubectl: str, arguments: list[str]) -> tuple[int, str]:
    completed = subprocess.run([kubectl, *arguments], capture_output=True, timeout=30, check=False)
    return completed.returncode, completed.stdout.decode("utf-8", errors="strict").strip()


def qualify(config: dict, kubectl: str) -> dict:
    validate(config)
    namespace, name = config["namespace"], config["name"]
    checks = []

    def rbac(actor, verb, resource, scope, expected):
        code, output = run(kubectl, ["auth", "can-i", verb, resource, "--namespace", scope,
                                    "--as", f"system:serviceaccount:{namespace}:{actor}"])
        # can-i returns 1 for a known denial; API/impersonation errors must not
        # count as successful negative authorisation tests.
        observed = output if (code, output) in ((0, "yes"), (1, "no"), (0, "no")) else "unavailable"
        checks.append({"actor": actor, "verb": verb, "resource": resource, "namespace": scope,
                       "expected": "yes" if expected else "no", "observed": observed,
                       "status": "PASS" if observed == ("yes" if expected else "no") else "FAIL"})

    for actor in (name + "-host", name + "-action"):
        for verb, resource in (("get", "secrets"), ("patch", "deployments.apps"), ("create", "pods/exec"), ("get", "pods/log")):
            rbac(actor, verb, resource, namespace, False)
    collector = config["collector"]
    if collector:
        actor, observed_namespace = name + "-collector", collector["observation_namespace"]
        rbac(actor, "list", "deployments.apps", observed_namespace, True)
        rbac(actor, "watch", "deployments.apps", observed_namespace, True)
        for verb, resource in (("patch", "deployments.apps"), ("get", "secrets"), ("get", "pods/log"), ("create", "pods/exec")):
            rbac(actor, verb, resource, observed_namespace, False)
        if namespace != observed_namespace:
            rbac(actor, "list", "deployments.apps", namespace, False)
    code, version = run(kubectl, ["version", "--output", "json"])
    try:
        cluster_version = json.loads(version).get("serverVersion") if code == 0 else None
    except ValueError:
        cluster_version = None
    code, claim = run(kubectl, ["get", "pvc", config["state_claim"], "--namespace", namespace, "--output", "json"])
    try:
        value = json.loads(claim)
        rwop = value.get("spec", {}).get("accessModes") == ["ReadWriteOncePod"]
    except ValueError:
        rwop = False
    checks.append({"check": "state-claim-single-pod-access", "status": "PASS" if code == 0 and rwop else "FAIL"})
    return {"version": "mas-host-deployment-cluster-check/1", "configuration_sha256": hashlib.sha256(
        json.dumps(config, sort_keys=True, separators=(",", ":")).encode()).hexdigest(),
        "server_version": cluster_version, "checks": checks,
        "status": "PASS" if all(check["status"] == "PASS" for check in checks) and cluster_version else "FAIL",
        "egress_enforcement": "NOT_RUN", "collector_overflow": "NOT_RUN", "duplicate_job": "NOT_RUN",
        "volume_lock": "NOT_RUN", "restore": "NOT_RUN", "worker_isolation": "UNSUPPORTED"}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("configuration", type=pathlib.Path)
    parser.add_argument("--kubectl", default="kubectl")
    parser.add_argument("--output", type=pathlib.Path, required=True)
    args = parser.parse_args()
    try:
        raw = args.configuration.read_bytes()
        if len(raw) > 65536:
            raise ValueError("deployment configuration exceeds byte bound")
        receipt = qualify(load_json(raw), args.kubectl)
        with args.output.open("x") as output:
            json.dump(receipt, output, indent=2)
            output.write("\n")
        return 0 if receipt["status"] == "PASS" else 1
    except (OSError, ValueError, subprocess.SubprocessError):
        print("cluster precondition check could not complete", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
