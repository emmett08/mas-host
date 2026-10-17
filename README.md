# MAS Host for Nehemiah

A shared Rust host for native functions, CLI tools, LLMs and composable workflows using the Nehemiah communication protocol (existing `mas-*` identifiers remain stable). Application
repositories supply **functions or executable tools, agent configuration and topology
configuration**, plus their normal build metadata and tests. The host tooling
generates executable entry points and registration bundles. Go, Rust, C++ and
Python handlers share the same routing, admission and recovery rules.

This is a **trusted local Linux profile**, built from source. It
implements native execution, bounded parallel requests, durable child results,
effect admission, inspection and evidence-based recovery. It consumes a pinned
revision of [`emmett08/mas`](https://github.com/emmett08/mas) through its public
Rust crate; neither the protocol repository nor existing demos need changes.

[`mas-agents/3`](docs/llm-workflows.md) configures maintained OpenAI Responses,
OpenAI-compatible, Cursor SDK, command and explicit mock providers. Workflow
agents recursively compose sequences, parallel groups, branches, bounded loops,
foreach and named subworkflows. Star, tree and mesh routes can be combined in one
application. Every cross-agent request still passes through MAS admission.
The [composable examples](https://github.com/emmett08/mas-host-examples/tree/main/examples/composable-agents)
exercise these shapes with one actor definition and selectable provider profiles.

## Build and try an application

Prerequisites: Linux, Git, Rust **1.99.0**, a C compiler and Make. Rust is pinned
in `rust-toolchain.toml`; the protocol dependency builds bundled SQLite. Native
Go applications require Go **1.24.0 or later**, C++ applications require C++17,
and Python applications require Python **3.11 or later**. Full verification uses
all four language toolchains.

```sh
git clone https://github.com/emmett08/mas-host.git
cd mas-host
make build
```

Maintained runnable applications live in
[`mas-host-examples`](https://github.com/emmett08/mas-host-examples): Go, Rust,
C++, Python, ordinary CLI tools and composed LLM workflows. That repository pins
its host revision and documents setup, execution and recovery.

This repository keeps small compiler and recovery inputs under `tests/fixtures`.
They are exercised by host CI and are not a second application catalogue.
SDK documentation retains concise API snippets beside the implementation.

## An application repository

For Go, an exported handler has a typed signature:

```go
func Assess(ctx *mas.Context, proposal Proposal) (Assessment, error)
```

Its `agents.toml` selects the function:

```toml
version = "mas-agents/3"

[[agents]]
name = "assessor"
operation = "assess/1"
[agents.implementation]
kind = "native"
language = "go"
source = "."
handler = "Assess"
```

Its `topology.yaml` permits requests and replies:

```yaml
version: mas-topology/1
edges:
  - [client, assessor]
  - [assessor, client]
```

The application also has its ordinary `go.mod`, SDK dependency, source and tests.
It does not contain a worker `main`, registration factory or protocol codec.
The CLI writes generated files under `.mas/build/`; add `.mas/` to the application's
`.gitignore`. Select `go`, `rust`, `cpp` or `python` in each native implementation
binding; routing and admission follow the shared host contract.

| Language | Supported application signature/build profile |
|---|---|
| [Go](sdk/go/README.md) | Typed exported functions in an ordinary module; requirements, replacements and checksums preserved |
| [Rust](sdk/rust/README.md) | Standalone source functions using SDK `Context` and `Json`; generated Cargo entry point |
| [C++](sdk/cpp/README.md) | Standalone source functions using SDK `Context` and `Json`; generated C++17 entry point |
| [Python](sdk/python/README.md) | Typed functions using dataclasses or dictionaries; generated archive containing application modules and the SDK |

Python handlers receive ordinary application types:

```python
from dataclasses import dataclass
from mas_host import Context

@dataclass
class Proposal:
    units: int

@dataclass
class Assessment:
    accepted: bool

def assess(ctx: Context, proposal: Proposal) -> Assessment:
    return Assessment(accepted=proposal.units <= 100)
```

Bind this function with `language = "python"`, `source = "application.py"`,
and `handler = "assess"`. The [Python examples](https://github.com/emmett08/mas-host-examples/tree/main/examples/python-review)
include peer requests, durable effects and a typed caller that supplies input
without creating an input file.

TOML settings, YAML topology/workflows and typed SDK inputs share one validation
path. JSON remains available for generated configuration, schemas, stdin and
interchange. See [authoring formats](docs/authoring.md) for discovery, conversion
rules and recovery identity.

Full Cargo/CMake project discovery and arbitrary native type-to-schema generation
are future profiles. JSON values use exact integers; decimal quantities should
use scaled integers or strings. The SDK documentation gives each language's
numeric bounds and rejection rules.

The source distribution includes its generators and SDKs. For an application
outside this checkout, set `MAS_HOST_SDK_ROOT` to this repository's absolute path,
or supply `--sdk-root`. Copying only the host executable does not install those
build assets. A future packaged release should install them together.

## Ordinary command-line tools

A `command` implementation declares an executable, separate argument bindings,
stdin and stdout formats, accepted exit codes and bounded execution limits.
Its operation declares input/output schemas independently of the implementation.
The host adapts that tool to MAS and journals each invocation as an admitted effect.
The tool needs no SDK or protocol implementation.

See the [command contract](docs/commands.md) and the
[CLI example](https://github.com/emmett08/mas-host-examples/tree/main/examples/command-policy).

## Execution and recovery

Application code expresses sequencing, joins, bounded loops and acceptance.
Topology specifies who may communicate. The SDK supplies three operations:

- **Request:** run a named child step and reuse its recorded result on replay.
- **Parallel:** join up to eight independent child requests in input order.
- **Effect:** obtain durable admission before an external action and record its
  result with evidence. A previously completed effect returns its stored result.

Code outside effect boundaries must be deterministic and free of external
effects. An unfinished handler may restart; ordinary threads or writes do not
automatically acquire checkpoints. After an admitted action loses its completion
response, the host retains uncertainty and refuses automatic repetition.

```sh
target/debug/mas-host inspect --state .state --run release-001
target/debug/mas-host observe --state .state --run release-001
target/debug/mas-host resolve --state .state --run release-001 \
  --effect EFFECT_ID --output confirmed-output.json --evidence evidence.json
```

Resolution records the operator's assertion after checking external evidence;
the host does not establish that assertion's truth. Empty evidence is rejected.
Run the original command again to resume. The original deadline remains in force.
`cancel --state DIR --run ID` records cancellation intent; a worker observes it
at its next host interaction or is stopped at the original deadline.

`observe` exposes committed message envelopes, task states and effect checkpoints
while a run is active. It opens both databases read-only, takes no execution-owner
lock and performs no protocol action. Payloads, inputs, outputs and evidence are
omitted by default; `--include-payloads true` explicitly includes them. See the
[observation contract](docs/observation.md) for live displays and polling limits.

The host binds a run to its host binary, generated bundle, worker digests and
input. Rebuilding changed code cannot silently resume it. Old generated bundles
and their content-addressed worker executables remain available under `.mas/build/`.
Use `run --bundle FILE ...` to select a retained bundle explicitly. Shared-library
and environment pinning remain deployment responsibilities.

## Verify

```sh
make test        # Rust/Go checks, Python unit tests and C++ conformance
make acceptance  # native applications and real effect recovery
```

The implementation uses one process owner per run, a separate execution journal,
the unchanged MAS protocol store, and MAS's Linux worker guardian. Workers are
trusted code with the operator's filesystem permissions. The guardian clears the
environment, discards worker stdout/stderr and applies fixed local resource
limits; diagnostics travel in returned errors and evidence. It is not a sandbox.
Workers and tools must remain in the foreground and keep descendants in the
inherited process group. Daemonising children can delay guardian cleanup and
ownership-lock release beyond the deadline; they are outside this profile.
All workers receive only their declared environment bindings; LLM connections add
their explicitly named credentials. CLI tool I/O is captured by its adapter;
retained application logs are not implemented.

The optional [A2A service](docs/a2a.md) publishes explicitly granted operations with durable request identities, scoped Agent Cards and conservative recovery. `describe --bundle FILE` generates operation contracts from the validated bundle.

This release does not claim distributed ownership or compatibility
with the transitional Python host's state. Those extensions and
their acceptance criteria are set out in [architecture](docs/architecture.md).
The precise framing, execution and recovery rules are in the
[native contract](docs/native-contract.md).

## Licence

GPL-3.0-only, matching the linked MAS protocol crate. The vendored nlohmann JSON
header retains its upstream MIT licence and provenance in `sdk/cpp/include/nlohmann/`.

For auditable evaluator findings, controlled feedback and bounded draft–assess–revise
execution, see [assessment and revision](docs/assessment-revision.md).
