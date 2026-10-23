"""Qualify a closed revision contract through the actual CLI and guardian."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time
import unittest

BINARY = Path(os.environ.get("MAS_HOST_BINARY", "target/debug/mas-host")).resolve()


def digest(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":")).encode()).hexdigest()


def schema(values):
    first = values[0] if values else None
    if first is None:
        return {"type": "null"}
    if isinstance(first, bool):
        return {"type": "boolean"}
    if isinstance(first, int):
        return {"type": "integer", "minimum": min(values), "maximum": max(values)}
    if isinstance(first, str):
        return {"type": "string", "enum": sorted(set(values))}
    if isinstance(first, list):
        return {"type": "array", "maxItems": max(map(len, values)),
                "items": schema([item for value in values for item in value])}
    names = set().union(*(value.keys() for value in values))
    return {"type": "object", "additionalProperties": False,
            "required": sorted(set.intersection(*(set(value) for value in values))),
            "properties": {name: schema([value[name] for value in values if name in value]) for name in sorted(names)}}


class GovernedRevision(unittest.TestCase):
    def test_actual_closed_revision_and_restart(self):
        self.assertTrue(BINARY.is_file(), f"Build the actual host first: {BINARY}")
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            output = {"ready": True}
            candidate = {"schema_version": 1, "id": "candidate-0", "output": output,
                         "output_sha256": digest(output), "parent": None, "feedback": []}
            evidence = {"schema_version": 1, "id": "candidate-0-output", "content": output,
                        "content_sha256": digest(output)}
            result = {"status": "completed", "findings": [{"criterion_id": "safe", "verdict": "pass",
                      "score": 1, "evidence": [{"id": evidence["id"], "sha256": digest(evidence)}],
                      "explanation": "checked exact output"}]}
            worker = root / "worker.py"
            worker.write_text("#!/usr/bin/python3\n" + '''import json,os,socket,struct
s=socket.socket(fileno=int(os.environ['MAS_WORKER_CHANNEL_FD']))
def read(n):
 b=bytearray()
 while len(b)<n:
  c=s.recv(n-len(b))
  if not c: raise EOFError()
  b.extend(c)
 return bytes(b)
a=json.loads(read(struct.unpack('>I',read(4))[0]))
''' + f"with open({str(root / 'launches')!r},'a') as f: f.write(a['actor']+'\\n')\n" +
                              f"outputs=json.loads({json.dumps({'drafter': output, 'assessor': result})!r})\n" +
                              "b=json.dumps({'type':'complete','output':outputs[a['actor']]}).encode()\n" +
                              "s.sendall(struct.pack('>I',len(b))+b)\n")
            worker.chmod(0o700)
            actors = ["drafter", "assessor"]
            bundle = {"version": "mas-bundle/3", "agents": [{"name": actor, "executable": str(worker),
                      "sha256": hashlib.sha256(worker.read_bytes()).hexdigest(), "handler": "run", "operation": "run",
                      "environment": [], "working_directory": str(root)} for actor in actors],
                      "topology": {"agents": ["client"] + actors, "edges": [[a, b] for actor in actors for a, b in [("client", actor), (actor, "client")]],
                      "application": "agent-task/1", "max_calls": 8, "wall_seconds": 300}}
            rubric = {"schema_version": 1, "id": "r", "criteria": [{"id": "safe", "measurement": "model", "minimum": 0, "maximum": 1}]}
            evaluator = {"id": "checker", "version": "1", "kind": "model"}
            plan = {"version": "mas-host-revision/1", "draft_agent": "drafter", "revision_agent": "drafter", "max_revisions": 0,
                    "max_invocations": 2, "draft_inputs": [], "revision_inputs": [], "allowed_agents": [], "feedback_evaluators": [],
                    "rubric": rubric, "evaluators": [{"actor": "assessor", "identity": evaluator, "inputs": []}]}
            original = {"sources": []}
            for name, value in [("bundle", bundle), ("plan", plan), ("input", original)]:
                (root / f"{name}.json").write_text(json.dumps(value))

            def run(identity, governed=False):
                args = [str(BINARY), "run", "--bundle", str(root / "bundle.json"), "--agent", "drafter", "--input", str(root / "input.json"),
                        "--state", str(root), "--run", identity, "--revision-plan", str(root / "plan.json")]
                if governed:
                    args += ["--governance", str(root / "governance.json"), "--tenant", "tenant-a", "--principal", "principal-a", "--purpose", "diagnosis", "--grant", "grant-a"]
                invocation = subprocess.run(args, capture_output=True, text=True, timeout=60)
                self.assertEqual(invocation.returncode, 0, invocation.stderr)
                return json.loads(invocation.stdout)

            ordinary = run("reference")
            self.assertEqual(ordinary["revision"]["status"], "completed")
            audit = ordinary["revision"]
            contexts = [{key: value for key, value in context.items() if key != "id"} for context in audit["contexts"]]
            schemas = {"plan": schema([{"plan": plan, "input": original}]), "context": [schema([context]) for context in contexts],
                       "terminal": schema([{"status": "completed", "selected": {"id": candidate["id"], "sha256": digest(candidate)}}]),
                       "delivery": schema([output])}
            for kind in ["candidate", "rubric", "evidence", "assessment"]:
                choices = [schema([record["record"]]) for record in audit["records"] if record["kind"] == kind]
                schemas[kind] = choices or schema([{}])
            expires = int(time.time() * 1000) + 300000
            rules = []
            for actor in actors:
                for operation, recipient, destination in [("invoke", f"worker:{actor}", f"local-{actor}"), ("deliver", "principal-a", "client"),
                                                           ("revision-retain", f"controller:{actor}", f"local-{actor}")]:
                    rules.append({"operation": operation, "principal": "principal-a", "actor": actor, "producer": "host", "recipient": recipient,
                                  "purpose": "diagnosis", "destination": destination, "location": "uk", "classifications": ["restricted"],
                                  "obligations": ["inline-retention-approved"], "max_bytes": 65536})
            authority = {"version": "mas-governed-authority/1", "grants": [{"version": "mas-current-authority/1", "tenant": "tenant-a", "principal": "principal-a",
                         "grant_ref": "grant-a", "grant_epoch": 1, "policy_version": "policy-v1", "status": "active", "valid_from_ms": 0, "expires_at_ms": expires}],
                         "policies": [{"version": "mas-data-use-policy/1", "id": "policy-a", "tenant": "tenant-a", "policy_version": "policy-v1", "valid_from_ms": 0,
                                       "expires_at_ms": expires, "rules": rules}]}
            (root / "authority.json").write_text(json.dumps(authority))
            (root / "key").write_bytes(bytes([19]) * 32)
            (root / "key").chmod(0o600)
            policies = [{"actor": context["actor"], "destination": f"local-{context['actor']}", "input_schema": schema([context["input"]]),
                         "output_schema": schema([output if context["actor"] == "drafter" else result]), "effects": []} for context in contexts]
            config = {"version": "mas-governed-local/2", "authority_file": str(root / "authority.json"), "commitment_key_file": str(root / "key"),
                      "commitment_key_id": "key-v1", "location": "uk", "baseline_policy_refs": ["policy-a"],
                      "revision": {"actor": "drafter", "schemas": schemas}, "agents": policies}
            (root / "governance.json").write_text(json.dumps(config))
            governed = run("protected", True)
            self.assertEqual(governed["output"], output)
            self.assertEqual(governed["revision"]["status"], "completed")
            self.assertEqual(governed["revision"]["candidates"], [])
            launches = (root / "launches").read_bytes()
            self.assertEqual(run("protected", True), governed)
            self.assertEqual((root / "launches").read_bytes(), launches)


if __name__ == "__main__":
    unittest.main()
