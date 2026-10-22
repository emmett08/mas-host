"""Generate one explicit trusted-local fixture package and its demonstration approval."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil


ROOT = Path(__file__).resolve().parent


def digest(raw):
    return hashlib.sha256(raw).hexdigest()


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()


def require(condition):
    if not condition:
        raise ValueError("fixture-package-rejected")


def prepare(language, destination, runtime=None):
    require(language in {"python", "typescript"})
    selected = runtime or shutil.which("python3" if language == "python" else "node")
    require(selected is not None)
    executable = Path(selected).resolve(strict=True)
    require(executable.is_file())
    executable_hash = digest(executable.read_bytes())
    agent = "evidence_" + language
    files = ["fixture.json", "schemas/input.json", "schemas/output.json"]
    if language == "python":
        files += ["python/collector.py"]
        arguments = [{"literal": "-I"}, {"literal": "-B"}, {"file": "python/collector.py"}]
    else:
        files += ["typescript/cli.mjs", "typescript/dist/capability.mjs", "typescript/capability.mts",
                  "typescript/package.json", "typescript/package-lock.json", "typescript/tsconfig.json"]
        arguments = [{"file": "typescript/cli.mjs"}, {"literal": "--module"}, {"file": "typescript/dist/capability.mjs"}]
    arguments += [{"literal": "--fixture"}, {"file": "fixture.json"},
                  {"literal": "--invocation"}, {"invocation_id": True}]
    require(all((ROOT / name).is_file() for name in files))
    implementation = {"kind": "command", "executable": str(executable), "arguments": arguments,
                      "stdin": "json", "stdout": "json-object", "success_exit_codes": [0],
                      "timeout_seconds": 30, "max_output_bytes": 8192}
    package = {
        "version": "mas-capability/1", "package_id": "structured-deployment-" + language,
        "agent": agent, "operation": "kubernetes_deployment_snapshot/1", "working_directory": ".",
        "implementation_sha256": digest(canonical(implementation)), "executable_sha256": executable_hash,
        "files": [{"path": name, "sha256": digest((ROOT / name).read_bytes())} for name in sorted(files)],
        "runtime": {"id": language + "-fixture-" + executable_hash[:16],
                    "files": [{"path": str(executable), "sha256": executable_hash}]},
        "input_schema_sha256": digest(canonical(json.loads((ROOT / "schemas/input.json").read_bytes()))),
        "output_schema_sha256": digest(canonical(json.loads((ROOT / "schemas/output.json").read_bytes()))),
        "requires": {"grants": ["synthetic-deployment-read"], "credential_references": [], "destinations": ["host-local"]},
        "limits": {"timeout_seconds": 30, "max_output_bytes": 8192},
        "recovery": {"effect": "read", "replay": "retained-observation", "unknown": "operator-reconciliation"},
    }
    policy = {"version": "mas-capability-approvals/1", "profile": "trusted-local",
              "approved_packages": [digest(canonical(package))], "require_attestation": False,
              "requirements": {**package["requires"], "runtime_ids": [package["runtime"]["id"]]}}
    destination = Path(destination).resolve()
    destination.mkdir(parents=True, exist_ok=False)
    for name in files + ["input.json"]:
        target = destination / name
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(ROOT / name, target)
    quoted = lambda value: json.dumps(value, ensure_ascii=False)
    argv = ",\n".join("  { " + next(iter(item)) + " = " + ("true" if item.get("invocation_id") else quoted(next(iter(item.values())))) + " }" for item in arguments)
    agents = f'''version = "mas-agents/3"
[[agents]]
name = {quoted(agent)}
operation = "kubernetes_deployment_snapshot/1"
input_schema = "schemas/input.json"
output_schema = "schemas/output.json"
environment = []
working_directory = "."
[agents.implementation]
kind = "command"
executable = {quoted(str(executable))}
arguments = [
{argv}
]
stdin = "json"
stdout = "json-object"
success_exit_codes = [0]
timeout_seconds = 30
max_output_bytes = 8192
[limits]
wall_seconds = 45
max_calls = 1
max_messages = 8
max_bytes = 131072
mailbox_capacity = 4
'''
    (destination / "agents.toml").write_text(agents)
    (destination / "topology.json").write_text(json.dumps({"version": "mas-topology/1", "edges": [["client", agent], [agent, "client"]]}, indent=2) + "\n")
    for name, value in [("capability.json", package), ("capabilities.json", {"version": "mas-capabilities/1", "packages": [package]}), ("approvals.json", policy)]:
        (destination / name).write_text(json.dumps(value, indent=2) + "\n")
    return destination


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--language", choices=("python", "typescript"), required=True)
    parser.add_argument("--destination", required=True)
    parser.add_argument("--runtime")
    args = parser.parse_args()
    print(prepare(args.language, args.destination, args.runtime))


if __name__ == "__main__":
    main()
