# Python SDK

Write typed Python handlers and let the shared Rust host supply execution and recovery
through `mas-native/1`. The SDK requires Python 3.11 or later and uses only the standard
library. Building an application produces a generated launcher and a frozen archive of
its sources.

For source-checkout use, add `sdk/python` to `PYTHONPATH`. An installed host
distribution includes the SDK assets. The generated worker bundles its own SDK copy and
application `.py` files, so it does not depend on your current `PYTHONPATH`. The
selected Python interpreter and installed external packages remain deployment
dependencies.

## Typed handlers

```python
from dataclasses import dataclass
from mas_host import Context

@dataclass(frozen=True)
class Proposal:
    amount_minor: int

@dataclass(frozen=True)
class Assessment:
    approved: bool

def assess(ctx: Context, proposal: Proposal) -> Assessment:
    return Assessment(approved=proposal.amount_minor <= 20_000)
```

Bind the function with a version 3 agent implementation:

```toml
version = "mas-agents/3"

[[agents]]
name = "assessor"
operation = "assess/1"

[agents.implementation]
kind = "native"
language = "python"
source = "assessment.py"
handler = "assess"
```

The host builds the importable module without importing application code at
build time. The generated entry point revokes private-socket inheritance before
importing the SDK, including any application modules reached through those
imports. It then calls `serve_module("assess", "assessment")`, which claims the
socket and validates the start message before importing the handler module.
The interpreter's earlier site initialisation, including installed `.pth` and
`sitecustomize` code, belongs to the trusted deployment environment. Applications
do not supply launchers or protocol codecs. `serve(name, function)` is also
available for an already imported function.

Dataclass payloads support nested dataclasses, `list[T]`, `dict[str, T]`, strict `str`,
`int` and `bool`, `None`, `Any`, `Literal` and unions with exactly one matching variant.
An annotation allowing `None` still requires the field to be present unless it has a
default. Defaults and default factories are supported. Unknown fields,
boolean-to-integer coercion and unsupported annotations are rejected. Dataclass fields
must participate in `__init__`. Supporting `InitVar`, custom field aliases, tuple/set
payloads, arbitrary Python objects or asynchronous handlers requires a wider adapter
profile. A synchronous handler has two positional arguments and returns an object or
dataclass instance. Postponed annotations are resolved with `typing.get_type_hints` in
the application's module.

Inputs, outputs, effect descriptors and effect evidence use exact integer JSON: no
floating-point values, at most 1024 decimal digits per integer, valid Unicode, unique
keys, at most 32 nested containers and a 65536-byte native frame. Python integers retain
their exact value. Encode decimal quantities as scaled integers or strings. Dataclass
constructors and `__post_init__` execute as application code and must obey the
deterministic orchestration rule.

## Requests and effects

```python
from mas_host import EffectResult, Request

peer = ctx.request("review", "reviewer", proposal, output_type=Assessment)
results = ctx.parallel([
    Request("capacity", "capacity_checker", {"replicas": 3}),
    Request("security", "security_checker", {"critical_findings": 0}),
])

receipt = ctx.effect(
    "publish",
    {"resource": "receipt", "revision": "v1"},
    lambda invocation_id: EffectResult(
        output={"published": True},
        evidence={"external_receipt": publish_receipt(invocation_id)},
    ),
)
```

`request` returns a dictionary unless `output_type` is supplied. `parallel` accepts 1..8
requests and returns dictionaries in request order. Step keys must be stable and
distinct within their parent invocation, contain 1..128 UTF-8 bytes and exclude control
characters. The host owns topology admission, request replay, parallelism, cancellation
and the original deadline. `ctx.actor`, `ctx.invocation_id`, `ctx.deadline_ms` and
`ctx.remaining_seconds` identify the current invocation. The context remains valid only
for that handler's lifetime.

An effect callback receives its stable effect invocation ID and runs only after host
admission. A cached effect skips the callback. Return `EffectResult` with object output
and non-empty object evidence. A callback error, missing completion acknowledgement or
uncertain admission raises `EffectUncertain` and blocks further context operations. The
host retains uncertainty even if application code catches the exception. Another
external attempt requires the host's explicit admission and recovery contract.

A governed acknowledgement replaces the callback's local output with the host's
safe projection. Fresh and cached calls return that same dictionary. A withheld
payload returns the reserved `mas-governed-unavailable/1` marker and keeps the
host effect completed. Malformed replacement metadata raises
`HostError("invalid governed effect completion")` without exposing local raw
output or setting `EffectUncertain`. The SDK does not retry. An acknowledgement
without `completion` preserves the ordinary callback output. See the exact
closed version, effect identity, external certainty and disposition fields in
the [native contract](../../docs/native-contract.md#external-effects-and-recovery).

Keep code outside effect boundaries deterministic and free of external effects because
the handler may restart between durable checkpoints. Threads and subprocesses remain
ordinary application computation. The SDK exchanges frames through the private channel,
while the local guardian discards worker stdout/stderr.

## Typed root input and configuration

```python
from pathlib import Path
from mas_host import Agent, Client, Configuration, Native

project = Path("/path/to/application")
authored = Configuration(
    agents=[Agent("assessor", Native("python", "assessment.py", "assess"), "assess/1")],
    edges=[("client", "assessor"), ("assessor", "client")],
).export(project / ".mas" / "authored")

client = Client(
    project=project,
    agents=authored.agents,
    topology=authored.topology,
    state=project / ".mas" / "state",
    run_id="assessment-001",
)
result = client.run("assessor", Proposal(12_500), output_type=Assessment)
```

`Client.run` sends input on stdin using `--input -`, allowing callers to supply typed
input directly. Repeated calls with the same run ID and retained identity reuse recorded
work. When the run ID is omitted, the client creates one per instance. Retain
`client.run_id` to resume that run later. `bundle=...` selects a retained bundle instead
of rebuilding a project. `host`, `sdk_root` and `connections` accept explicit host
executable, SDK distribution and connection-profile paths.

The typed authoring API also supplies `Workflow(plan)` and `Llm(connection, model,
instructions, ...)` with schemas on `Agent`. Export is an explicit application action
that produces ordinary configuration documents. The host validates the exported
operation identities, paths, schemas, provider settings and topology. Inspection and
resume consume those documents, keeping Python configuration execution an explicit
application action.

## Verify

```sh
PYTHONPATH=sdk/python python3 -m unittest discover -s sdk/python/tests -v
```

Tests exercise exact JSON boundaries, nested typed data, actual socket framing,
durable effect outcomes, descriptor inheritance and the CLI client contract.

## CLI configuration

`Command` adapts an ordinary executable without requiring a Python handler:

```python
from mas_host import Agent, Command, FromInput

policy = Agent(
    "policy_check",
    Command("tools/policy-check", arguments=("--service", FromInput("/service"))),
    "check_policy/1",
    input_schema="schemas/input.json", output_schema="schemas/output.json",
    environment=("POLICY_TOKEN",), working_directory=".",
)
```

`FileArgument("tool.py")` retains a project file and supplies its frozen path.
`InvocationId()` supplies the admitted effect ID to tools that produce receipts.
Schemas, environment references and working directory are properties of `Agent`
for all implementation kinds. See the [command contract](../../docs/commands.md).
