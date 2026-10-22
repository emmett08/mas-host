# Governed local structured execution

`mas-host-a2a/2` adds authenticated, current data-use admission to the existing
durable task service. It supports one bounded **trusted-local** path: an
operator declares closed structured inputs, outputs and effect evidence, and
the host projects those values before writing new payload records or delivering
them. A prohibited result can be withheld while its known effect completion
remains retained for replay.

This path implements part of E2/N2. It does not establish the whole N2 runtime,
free-text PII release, managed or shared deployment, hostile-worker isolation or
payload deletion. The existing [A2A wire profile](a2a.md) still applies. The
version 2 configuration and retention rules below replace that document's
version 1 inline authentication and complete-submission retention rules.

## Operator-owned configuration

The service consumes five separate authorities. Requests and worker output
cannot supply or widen them.

| File | Contract and responsibility |
|---|---|
| Service configuration | `mas-host-a2a/2`, loopback listener, published exports, absolute authentication and governance paths, task capacity |
| Authentication table | `mas-host-authentication/1`, bearer-token digests bound to tenant, principal, purpose, grant reference and exported actors |
| Governance configuration | `mas-governed-local/1`, absolute authority/key paths, key ID, location, and each actor's structured schemas and declared effects |
| Current authority | `mas-governed-authority/1`, active grants and matching policy templates, with validity times in Unix milliseconds |
| Commitment key | Exactly 32 private bytes in a regular file, with no group or other permission bits and no symlink |

The configuration objects have closed fields and explicit versions. An
authentication table has at most 128 records, with distinct token digests and
distinct `(tenant, principal)` pairs. Bearers are 32–512 printable non-space ASCII
bytes. Authentication identity fields use bounded ASCII identifiers.
Authentication checks the SHA-256 token digest, then constructs a session from
the record's identities.
It does not trust tenant, purpose or grant fields from the submitted data.

Each governed actor declares its worker destination, input/output schemas and
up to 16 effect contracts. Each effect declares a format, destination,
descriptor schema, output schema and evidence schema. Policy templates omit
`evidence_refs`: the host associates them with the exact boundary source.
Applicable policies are conjunctive, so an added template can restrict an
existing permission. Every rule must explicitly require
`inline-retention-approved`.

The supported schema subset has closed objects with at most 32 properties,
strings with at most 64 enumerated values of at most 128 bytes each, integers
with explicit minimum and maximum, arrays with `maxItems` at most 32, booleans
and null. Nesting is bounded. Arbitrary strings, floating-point numbers and
`anyOf` alternatives are rejected. A string `maxLength` alone does not qualify
as a governed string contract.

## Runnable synthetic example

Run these commands from the repository root on a supported Unix host with
Python 3.11 or later. The example uses a trusted native Python worker and local
synthetic evidence. Its effect appends one byte to `calls` so that replay is
observable. It makes no network or provider calls.

```sh
cargo build --locked -p mas-host
export NEHEMIAH_HOST="$(pwd)/target/debug/mas-host"
export NEHEMIAH_DEMO="$(mktemp -d)"
chmod 700 "$NEHEMIAH_DEMO"
python3 - <<'PY'
import hashlib
import json
import os
from pathlib import Path
import secrets
import time

root = Path(os.environ["NEHEMIAH_DEMO"]).resolve()

def write(name, value):
    path = root / name
    path.write_text(json.dumps(value, indent=2), encoding="utf-8")
    path.chmod(0o600)

def obj(properties, required):
    return {"type": "object", "properties": properties, "required": required,
            "additionalProperties": False}

input_schema = obj({"mode": {"type": "string", "enum": ["available", "withheld"]}}, ["mode"])
output_schema = obj({"approved": {"type": "boolean"}}, ["approved"])
descriptor_schema = obj({
    "format": {"type": "string", "enum": ["fixture-effect/1"]},
    "destination": {"type": "string", "enum": ["fixture"]},
}, ["format", "destination"])
evidence_schema = obj({"execution": {"type": "string", "enum": ["completed"]}}, ["execution"])
write("input.json", input_schema)
write("output.json", output_schema)
(root / "application.py").write_text('''from mas_host import Context, EffectResult

def collect(ctx: Context, value: dict) -> dict:
    def action(invocation_id):
        with open("calls", "a", encoding="ascii") as stream:
            stream.write("x")
        approved = True if value["mode"] == "available" else "synthetic-prohibited-value"
        return EffectResult(
            {"approved": approved, "discarded": "synthetic-private-note"},
            {"execution": "completed", "discarded": "synthetic-private-note"},
        )
    return ctx.effect("collect", {"format": "fixture-effect/1", "destination": "fixture"}, action)
''', encoding="utf-8")
(root / "agents.toml").write_text('''version = "mas-agents/3"
[[agents]]
name = "collector"
operation = "collect/1"
input_schema = "input.json"
output_schema = "output.json"
[agents.implementation]
kind = "native"
language = "python"
source = "application.py"
handler = "collect"
''', encoding="utf-8")
write("topology.json", {"version": "mas-topology/1", "edges": [
    ["client", "collector"], ["collector", "client"],
]})

key = root / "commitment.key"
key.write_bytes(secrets.token_bytes(32))
key.chmod(0o600)
token = secrets.token_hex(32)
(root / "client-token").write_text(token, encoding="ascii")
(root / "client-token").chmod(0o600)
write("authentication.json", {
    "version": "mas-host-authentication/1",
    "principals": [{"tenant": "tenant-a", "principal": "alice", "purpose": "diagnosis",
                    "grant_ref": "grant-a", "token_sha256": hashlib.sha256(token.encode()).hexdigest(),
                    "agents": ["collector"]}],
})
write("governance.json", {
    "version": "mas-governed-local/1",
    "authority_file": str(root / "authority.json"),
    "commitment_key_file": str(key), "commitment_key_id": "demo-key-1", "location": "local",
    "agents": [{"actor": "collector", "destination": "local-worker",
                "input_schema": input_schema, "output_schema": output_schema,
                "effects": [{"format": "fixture-effect/1", "destination": "fixture",
                             "descriptor_schema": descriptor_schema, "output_schema": output_schema,
                             "evidence_schema": evidence_schema}]}],
})
expires = int(time.time() * 1000) + 3_600_000
rules = []
for operation, recipient, destination in [
    ("invoke", "worker:collector", "local-worker"),
    ("effect", "operation:fixture-effect/1", "fixture"),
    ("deliver", "alice", "client"),
]:
    rules.append({"operation": operation, "principal": "alice", "actor": "collector",
                  "producer": "host", "recipient": recipient, "purpose": "diagnosis",
                  "destination": destination, "location": "local", "classifications": ["restricted"],
                  "obligations": ["inline-retention-approved"], "max_bytes": 16384})
write("authority.json", {
    "version": "mas-governed-authority/1",
    "grants": [{"version": "mas-current-authority/1", "status": "active", "tenant": "tenant-a",
                "principal": "alice", "grant_ref": "grant-a", "grant_epoch": 1,
                "policy_version": "pv1", "valid_from_ms": 0, "expires_at_ms": expires}],
    "policies": [{"version": "mas-data-use-policy/1", "id": "policy-a", "tenant": "tenant-a",
                  "policy_version": "pv1", "valid_from_ms": 0, "expires_at_ms": expires, "rules": rules}],
})
write("gateway.json", {
    "version": "mas-host-a2a/2", "listen": "127.0.0.1:8765",
    "public_url": "http://127.0.0.1:8765/a2a", "name": "Governed synthetic evidence",
    "authentication_file": str(root / "authentication.json"),
    "governance_file": str(root / "governance.json"),
    "exports": [{"agent": "collector", "name": "Collector",
                 "description": "Synthetic structured evidence", "public": False}],
    "max_active": 4, "max_tasks": 1000,
})
print(root)
PY
"$NEHEMIAH_HOST" build --project "$NEHEMIAH_DEMO"
"$NEHEMIAH_HOST" describe --bundle "$NEHEMIAH_DEMO/.mas/build/bundle.json" > "$NEHEMIAH_DEMO/descriptor.json"
"$NEHEMIAH_HOST" serve-a2a --bundle "$NEHEMIAH_DEMO/.mas/build/bundle.json" \
  --state "$NEHEMIAH_DEMO/state" --config "$NEHEMIAH_DEMO/gateway.json" \
  > "$NEHEMIAH_DEMO/service.log" 2>&1 &
NEHEMIAH_DEMO_PID=$!
```

Port 8765 must be available. If necessary, change both `listen` and `public_url`
before starting. The generated grant and policy expire after one hour. The
plaintext token file belongs to the demonstration client, not the service's
authentication table.

Submit and repeat a message using the same shell:

```sh
python3 - <<'PY'
import json
import os
from pathlib import Path
import time
import urllib.request

root = Path(os.environ["NEHEMIAH_DEMO"])
gateway = json.loads((root / "gateway.json").read_text())
descriptor = json.loads((root / "descriptor.json").read_text())
token = (root / "client-token").read_text()

def rpc(method, params):
    body = json.dumps({"jsonrpc": "2.0", "id": "demo-rpc-1", "method": method, "params": params}).encode()
    request = urllib.request.Request(gateway["public_url"], data=body, headers={
        "Content-Type": "application/json", "Authorization": "Bearer " + token,
        "A2A-Version": "1.0", "A2A-Extensions": "urn:mas:host-task-service:1",
    })
    with urllib.request.urlopen(request, timeout=12) as response:
        value = json.load(response)
    assert "error" not in value, value
    return value["result"]

for _ in range(100):
    try:
        with urllib.request.urlopen(gateway["public_url"].replace("/a2a", "/.well-known/agent-card.json"), timeout=1):
            break
    except OSError:
        time.sleep(0.05)
else:
    raise RuntimeError("service did not start")

params = {"message": {"messageId": "demo-withheld-1", "role": "ROLE_USER",
          "parts": [{"mediaType": "application/json", "data": {
              "agent": "collector", "descriptorDigest": descriptor["bundle_digest"],
              "input": {"mode": "withheld", "discarded": "synthetic-private-input"},
          }}]}, "configuration": {"returnImmediately": True}}
task = rpc("SendMessage", params)["task"]
deadline = time.monotonic() + 15
while task["metadata"]["mas"]["state"] != "completed":
    assert task["metadata"]["mas"]["state"] not in ["failed", "unknown"], task
    assert time.monotonic() < deadline, task
    time.sleep(0.05)
    task = rpc("GetTask", {"id": task["id"]})
output = task["artifacts"][0]["parts"][0]["data"]
assert output == {"version": "mas-governed-unavailable/1", "external_execution": "completed",
                  "payload_disposition": "withheld"}, output
assert rpc("SendMessage", params)["task"]["id"] == task["id"]
assert (root / "calls").read_text() == "x"
print(json.dumps(output))
PY
```

Repeating the client block returns the retained task and leaves `calls` at one
byte. Restart the same service with the same immutable host, bundle,
configuration and key, then repeat it again to check retained completion.
`kill "$NEHEMIAH_DEMO_PID"` stops the demonstration listener. A separate message
with mode `available` returns `{"approved":true}` after dropping the extra
input, output and evidence properties. Changing the existing message's mode or
even its discarded input field produces `request_identity_conflict`.

## Projection, identity and current authority

The host commits the original submission with domain-separated HMAC-SHA-256
before retaining its projected input. The commitment covers the original
`SendMessage` parameters, including properties removed by projection. It is
bound to the private key ID, tenant, principal, purpose and grant reference.
Keyed commitments retain conflict detection without saving the original
payload or an unkeyed digest vulnerable to simple value enumeration.

Task ownership and message identities use a separate commitment domain bound
to tenant and principal. Changing a purpose or grant reference therefore
cannot reset a message identity and silently dispatch it again. Supplied
context IDs are validated but replaced by generated opaque context IDs. RPC IDs
are independent correlation values: version 2 accepts signed/unsigned 64-bit
integers or nonempty ASCII identifiers of at most 128 bytes using letters,
digits, `_`, `.` and `-`. Invalid IDs are returned as null.

At authentication, the service reads the current table and key, constructs the
session, then verifies the exact key consumed by that session. Its retained
authentication lease rechecks the exact selected table record and key at new
admissions and deliveries, including later worker and effect admissions. A
session also rereads the current grant and selected policies, requiring the
original grant epoch, policy version and policy-template digest. Missing,
expired, revoked or changed authority denies the boundary.

Updating an operator file should use a complete replacement followed by an
atomic rename. Removing a principal, replacing its token digest or narrowing
its actor list invalidates existing leases. A newly authenticated replacement
token for the same tenant/principal and unchanged policy can retrieve the same
retained tasks. A changed grant epoch invalidates an old session, while a fresh
session can preserve retained identity under the same policy. Policy or key
changes require explicit migration rather than reinterpretation of old state.
There is no migration command. Key bytes are bound into the service identity,
so hot replacement fails current use and restarting with a different key
rejects reuse of that service state.

`GetTask`, identical `SendMessage` replay and `CancelTask` require current
authentication, task ownership, actor eligibility and the task's retained
governance fingerprint. Both pending metadata and completed artifacts require
current delivery admission. Authorisation failures use fixed errors such as
`task_not_found` without disclosing whether another tenant's task exists.
Polling recovery identifiers are disclosed only when current task observation
and the recovery representation are admitted. Raw worker diagnostics are
replaced with fixed governed errors.

Revocation is checked at each admission and delivery. It prevents later work
or disclosure after that check observes the change. It does not terminate an
already admitted callback, recall an already admitted transmission or prove
that an external operation did not happen. Separate file reads are local
snapshots, not a shared atomic revocation/admission transaction.

## Completion, withholding and retention

Effect descriptors must already equal their declared projection. Extra fields
are rejected before admission. Effect output and reported evidence are
projected before new completion records are written. A schema or current
authority failure produces a safe unavailable payload while retaining the
known settlement separately:

```json
{
  "version": "mas-governed-effect-completion/1",
  "effect_id": "HOST_EFFECT_ID",
  "host_settlement": "completed",
  "external_execution": "completed",
  "payload_disposition": "withheld",
  "output": {
    "version": "mas-governed-unavailable/1",
    "external_execution": "completed",
    "payload_disposition": "withheld"
  }
}
```

The SDK consumes the host's completion acknowledgement. A repeated known
completion returns its retained acknowledgement and does not dispatch another
effect. Withholding the payload therefore does not convert a known completed
action into permission to retry. `external_execution` records the trusted
adapter's reported certainty, separately from host settlement and payload
availability. It may instead be `not-sent`, `refused` or `unknown`.

An A2A restart marks unfinished submitted/working tasks `unknown` and does not
redispatch them. The task's state is a service observation, not proof that its
effect was absent. Cancellation records intent. Preserve original identities
and reconcile uncertainty rather than creating another task to force a retry.

Permitted projections, evidence and safe receipts remain inline in local
SQLite journals and protocol state. `inline-retention-approved` is a mandatory
operator policy decision for this path. It provides no deletion, expiry
purging, cryptographic erasure or retroactive cleaning of historical journals.
Governed runs use a separate namespace. Ordinary `run`, `inspect`, `observe`,
`resolve` and revision paths cannot open or bypass that namespace. Authenticated
core API status and A2A retrieval are the supported observation paths.

## API, transport and execution limits

All byte counts below measure UTF-8 bytes of the stated JSON representation,
not Unicode characters, tokens or HTTP-compressed bytes. `max_bytes` is a
per-boundary data-use limit, distinct from the transport limits.

| Boundary | Representation checked |
|---|---|
| Structured projection | Compact serialised projected JSON, at most 16,384 bytes |
| Original commitment | Canonical original JSON, at most 65,536 bytes |
| Available effect completion | Current policy checks the combined projected output, evidence and receipt, then the exact native acknowledgement frame must fit 65,536 bytes |
| `GovernedSession::check_delivery` | Exact retained output JSON and current delivery policy |
| `Engine::status()` | Complete core status JSON, checked against delivery rules for the represented actors |
| A2A task retrieval | Output admission, then exact complete task/status/artifact JSON against delivery policy |
| HTTP envelope | Request body at most 65,536 bytes and serialised response at most 262,144 bytes |

Core status and A2A task status are different representations. A policy that
permits a small output may still reject the larger task or core snapshot.
Measure the intended API representation when setting `max_bytes`. The outer
JSON-RPC response wrapper has its own HTTP response limit. A fresh governed
core run with no bound actor/step refuses status observation. An accepted A2A
task already has a declared actor against which its pending status is checked.

The HTTP service allows 32 connections and 32 headers, with a 16,384-byte parse
buffer. Header and body reads each have a three-second deadline. A whole
connection, including response writing, has a ten-second deadline. Synchronous
`SendMessage` observation waits at most five seconds. These response deadlines
do not cancel accepted execution. The bundle's original run budget and deadline
remain authoritative. `max_active` defaults to 4 and is at most 16.
`max_tasks` defaults to 1,000 and is at most 10,000 lifetime accepted tasks.
Grant and policy timestamps use Unix milliseconds, while command
`timeout_seconds` and topology `wall_seconds` use seconds.

## Trust and verification limits

The local operator, registered implementations, installed runtimes and state
directory are trusted. The public listener accepts literal loopback addresses.
Remote access requires an operator-controlled HTTPS proxy preserving bearer,
version and extension headers. Browser `Origin` requests are rejected.
Published schemas and descriptions remain explicit operator-approved public
metadata, not worker-supplied authority.

Command workers continue through the maintained command adapter. Their
executable and declared retained `assets` are digest-checked before external
command admission. [Capability packages](capabilities.md) add explicit approval,
fixed argument and declared runtime/file bindings. The operator owns the
completeness and immutability of that dependency closure. A native callback can
perform direct filesystem or network operations with its process privileges.
Only operations routed through `Context.effect` receive the host's effect
admission and settlement contract. Process supervision and immutable byte
bindings do not establish a hostile-code sandbox.

The standalone privacy detector and its unqualified neural assessment do not
grant this path permission to release arbitrary text. Its release mechanism is
the configured structured allowlist. `check-profile` continues to reject
`managed-cluster`, `shared-enterprise`, mandatory worker isolation and the
broader unimplemented governance capabilities. This local path does not change
those profile acceptance results.

The focused regression suites exercise projection before persistence,
discarded-field conflicts, tenant separation, current-authority retrieval,
token/epoch rotation, later effect admission after revocation, withheld known
completion, restart replay and ordinary-path bypass rejection:

```sh
cargo test --locked -p mas-host-core governance::runtime
cargo test --locked -p mas-host --test a2a_governed
```

These synthetic checks support the stated local structured contract. Live
provider quality, complete PII detection, shared scheduling, payload lifecycle
and hostile-worker isolation require their own implementation and evidence.
