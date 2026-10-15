# MAS Host

A shared Rust host for applications written in Go, Rust and C++. Application
repositories supply **application functions, agent configuration and topology
configuration**, plus their normal build metadata and tests. The host tooling
generates executable entry points and registration bundles. No Python, handwritten
transport or application-owned launcher is required.

This is the initial **trusted local Linux profile**, built from source. It
implements native execution, bounded parallel requests, durable child results,
effect admission, inspection and evidence-based recovery. It consumes a pinned
revision of [`emmett08/mas`](https://github.com/emmett08/mas) through its public
Rust crate; neither the protocol repository nor existing demos need changes.

## Run the release example

Prerequisites: Linux, Git, Rust **1.99.0**, Go **1.24.0 or later**, a C/C++ compiler
supporting C++17, and Make. Rust is pinned in `rust-toolchain.toml`; C++ is needed
for the C++ example and full verification. The protocol's bundled SQLite build
also requires a C compiler.

```sh
git clone https://github.com/emmett08/mas-host.git
cd mas-host
make build

target/debug/mas-host run \
  --project examples/go-release --agent planner \
  --input examples/go-release/input.json \
  --state .state --run release-001
```

The Go example checks capacity and security concurrently, adjusts capacity within
a bounded loop, and obtains an independent acceptance check. Its optional audit
receipt demonstrates an external effect. `make acceptance` exercises an actual
worker exit after the receipt is durable, resolves the uncertain result from the
receipt, and resumes with one admitted call and no second write.

The native examples use the same host:

```sh
target/debug/mas-host run --project examples/rust-invoice --agent invoice \
  --input examples/rust-invoice/input.json --state .state --run rust-001
target/debug/mas-host run --project examples/cpp-invoice --agent invoice \
  --input examples/cpp-invoice/input.json --state .state --run cpp-001
```

## An application repository

For Go, an exported handler has a typed signature:

```go
func Assess(ctx *mas.Context, proposal Proposal) (Assessment, error)
```

Its `agents.json` selects the function:

```json
{
  "version": "mas-agents/1",
  "agents": [
    {"name": "assessor", "language": "go", "source": ".", "handler": "Assess"}
  ]
}
```

Its `topology.json` permits requests and replies:

```json
{
  "version": "mas-topology/1",
  "edges": [["client", "assessor"], ["assessor", "client"]]
}
```

The application also has its ordinary `go.mod`, SDK dependency, source and tests.
It does not contain a worker `main`, registration factory or protocol codec.
The CLI writes generated files under `.mas/build/`; add `.mas/` to the application's
`.gitignore`. A mixed-language configuration selects `rust` or `cpp` for other
actors without changing routing or admission rules.

| Language | Supported application signature/build profile |
|---|---|
| [Go](sdk/go/README.md) | Typed exported functions in an ordinary module; requirements, replacements and checksums preserved |
| [Rust](sdk/rust/README.md) | Standalone source functions using SDK `Context` and `Json`; generated Cargo entry point |
| [C++](sdk/cpp/README.md) | Standalone source functions using SDK `Context` and `Json`; generated C++17 entry point |

Full Cargo/CMake project discovery and arbitrary native type-to-schema generation
are future profiles. JSON values use exact integers; decimal quantities should
use scaled integers or strings. The SDK documentation gives each language's
numeric bounds and rejection rules.

The source distribution includes its generators and SDKs. For an application
outside this checkout, set `MAS_HOST_SDK_ROOT` to this repository's absolute path,
or supply `--sdk-root`. Copying only the host executable does not install those
build assets. A future packaged release should install them together.

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
target/debug/mas-host resolve --state .state --run release-001 \
  --effect EFFECT_ID --output confirmed-output.json --evidence evidence.json
```

Resolution records the operator's assertion after checking external evidence;
the host does not establish that assertion's truth. Empty evidence is rejected.
Run the original command again to resume. The original deadline remains in force.
`cancel --state DIR --run ID` records cancellation intent; a worker observes it
at its next host interaction or is stopped at the original deadline.

The host binds a run to its host binary, generated bundle, worker digests and
input. Rebuilding changed code cannot silently resume it. Old generated bundles
and their content-addressed worker executables remain available under `.mas/build/`.
Use `run --bundle FILE ...` to select a retained bundle explicitly. Shared-library
and environment pinning remain deployment responsibilities.

## Verify

```sh
make test        # Rust/Go checks, unit tests and C++ conformance
make acceptance  # all three languages, mixed-language routing and real recovery
```

The implementation uses one process owner per run, a separate execution journal,
the unchanged MAS protocol store, and MAS's Linux worker guardian. Workers are
trusted code with the operator's filesystem permissions. The guardian clears the
environment, discards worker stdout/stderr and applies fixed local resource
limits; diagnostics travel in returned errors and evidence. It is not a sandbox.
Credential injection and retained application logs are not implemented yet.

This release does not claim distributed ownership, remote ingress, built-in model
providers or compatibility with the Python host's state. Those extensions and
their acceptance criteria are set out in [architecture](docs/architecture.md).
The precise framing, execution and recovery rules are in the
[native contract](docs/native-contract.md).

## Licence

GPL-3.0-only, matching the linked MAS protocol crate. The vendored nlohmann JSON
header retains its upstream MIT licence and provenance in `sdk/cpp/include/nlohmann/`.
