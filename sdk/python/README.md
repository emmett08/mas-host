# Python SDK

Python 3.11 or later, with no third-party runtime dependencies. Application
functions run through the shared Rust host and its `mas-native/1` contract.
The generated launcher and frozen application archive are build outputs.

For source-checkout use, add `sdk/python` to `PYTHONPATH`; an installed host
distribution includes the SDK assets. The generated worker bundles its own SDK
copy and application `.py` files, so it does not depend on your current
`PYTHONPATH`. The selected Python interpreter and installed external packages
remain deployment dependencies.

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

Dataclass payloads support nested dataclasses, `list[T]`, `dict[str, T]`, strict
`str`, `int` and `bool`, `None`, `Any`, `Literal` and unions with exactly one
matching variant. Required fields remain required even when their annotation
allows `None`; defaults and default factories are supported. Unknown fields,
boolean-to-integer coercion and unsupported annotations are rejected.
Dataclass fields must participate in `__init__`; `InitVar`, custom field aliases,
tuple/set payloads, arbitrary Python objects and asynchronous handlers are outside
this profile. A synchronous handler has two positional arguments and returns
an object or dataclass instance. Postponed annotations are resolved with
`typing.get_type_hints` in the application's module.

Inputs, outputs, effect descriptors and effect evidence use exact integer JSON:
no floating-point values, at most 1024 decimal digits per integer, valid Unicode,
unique keys, at most 32 nested containers and a 65536-byte native frame. Python
integers retain their exact value. Encode decimal quantities as scaled integers
or strings. Dataclass constructors and `__post_init__` are application code;
they must obey the deterministic orchestration rule.

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

`request` returns a dictionary unless `output_type` is supplied. `parallel`
accepts 1..8 requests and returns dictionaries in request order. Step keys must
be stable and distinct within their parent invocation, contain 1..128 UTF-8
bytes and exclude control characters. The host owns topology admission, request replay,
parallelism, cancellation and the original deadline. `ctx.actor`,
`ctx.invocation_id`, `ctx.deadline_ms` and `ctx.remaining_seconds` identify the
current invocation; a context cannot be reused after its handler ends.

An effect callback receives its stable effect invocation ID and runs only after
host admission. A cached effect skips the callback. Return `EffectResult` with
object output and non-empty object evidence. A callback error, missing completion
acknowledgement or uncertain admission raises `EffectUncertain` and blocks further
context operations. The host retains uncertainty even if application code catches
the exception. The SDK never retries an external operation automatically.

Code outside effect boundaries must be deterministic and free of external
effects. Threads and subprocesses do not create durable checkpoints. Worker
stdout/stderr are application streams, not the framed private channel; the local
guardian currently discards them.

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

`Client.run` sends input on stdin using `--input -`; an input file is optional.
Repeated calls with the same run ID and retained identity reuse recorded work.
When omitted, the client creates a run ID once per client instance; retain
`client.run_id` to resume later. `bundle=...` selects a retained bundle instead
of rebuilding a project. `host`, `sdk_root` and `connections` accept explicit
host executable, SDK distribution and connection-profile paths.

The typed authoring API also supplies `Workflow(plan)` and
`Llm(connection, model, instructions, ...)` with schemas on `Agent`. Export is an explicit
application action that produces ordinary configuration documents. The host
validates operation identities, paths, schemas, provider settings and topology.
It never imports Python configuration code while inspecting or resuming a run.

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
