#!/usr/bin/env python3
"""Retain an exact-source local command receipt and reject source drift."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time


def sources(repo):
    paths = subprocess.check_output(
        ["git", "ls-files", "--cached", "--others", "--exclude-standard", "-z"], cwd=repo
    ).decode().split("\0")
    return {name: hashlib.sha256((repo / name).read_bytes()).hexdigest()
            for name in sorted(set(paths)) if name and not name.startswith("evidence/")
            and (repo / name).is_file()}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo", type=Path, required=True)
    parser.add_argument("--label", required=True)
    parser.add_argument("--log", type=Path, required=True)
    parser.add_argument("--result", type=Path, required=True)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command[1:] if args.command[:1] == ["--"] else args.command
    before = sources(args.repo)
    began = time.time()
    receipt = {"version": "mas-privacy-mediation-validation/1", "label": args.label,
               "command": command, "base": subprocess.check_output(
                   ["git", "rev-parse", "HEAD"], cwd=args.repo).decode().strip(),
               "source_sha256": before, "started_unix": began,
               "environment": {key: os.environ.get(key) for key in
                   ("CARGO_INCREMENTAL", "GOFLAGS", "RUSTUP_HOME", "CARGO_HOME")},
               "status": "RUNNING_OR_INTERRUPTED"}
    args.result.write_text(json.dumps(receipt, indent=2) + "\n")
    with args.log.open("wb") as stream:
        result = subprocess.run(command, cwd=args.repo, stdout=stream, stderr=subprocess.STDOUT)
    after = sources(args.repo)
    changed = [name for name in sorted(before.keys() | after.keys())
               if before.get(name) != after.get(name)]
    receipt.update(exit_code=result.returncode, seconds=round(time.time() - began, 3),
                   source_changes_during_checks=changed,
                   log_sha256=hashlib.sha256(args.log.read_bytes()).hexdigest(),
                   status="PASS" if result.returncode == 0 and not changed else "FAIL")
    args.result.write_text(json.dumps(receipt, indent=2) + "\n")
    print(json.dumps({key: receipt[key] for key in
                     ("label", "status", "exit_code", "seconds", "source_changes_during_checks")}))
    raise SystemExit(0 if receipt["status"] == "PASS" else 1)


if __name__ == "__main__":
    main()
