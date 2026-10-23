# Host architecture

## Purpose and boundary

The application owns the solution and declares which agents may communicate. The host
turns those declarations into transport bindings, process launchers and registration
manifests. Application repositories therefore keep their domain code, configuration and
ordinary language build files and tests. Python is an optional application language. The
shared host is Rust, and each other language profile runs with its own toolchain.

`mas` and `mas-host` have independent responsibilities and release histories. This host
consumes the public Rust `mas-protocol-codec` crate at commit
`fe644bbf3acb3ccc2abaffd54bcc4b367053fe4b`. The host consumes that source as a pinned
dependency while independently published protocol releases remain unavailable.
`Cargo.lock` records the complete Rust graph.

```mermaid
flowchart TD
    App["Application code + agent/topology configuration"] --> Build["Language build tooling"]
    Build --> Bundle["Generated executables + bundle"]
    Bundle --> Host["Rust execution core"]
    Host --> Protocol["MAS protocol authority"]
    Host --> Journal["Execution journal"]
    Host --> Guardian["Worker guardian"]
    Guardian --> Workers["Go, Rust, C++ and Python SDKs"]
    Workers --> Host
```

| Owner | Responsibility |
|---|---|
| MAS protocol | Message grammar, task transitions, permitted routes, mailboxes, call reservation and expiry |
| Host core | Bound application identity, execution records, replay, uncertain effects and administrative resolution |
| Worker supervisor | Private channel, original deadline and descendant cleanup |
| SDK | Language-native context, codecs and error conversion |
| Build integration | Generated entry point, handler binding, executable build and bundle |
| Application | Business decisions, durable step names, joins, acceptance and effect meaning |
| Operator | Trusted code, credentials, deployment environment, evidence for reconciliation and retention |

Workers receive neither protocol database handles nor authority to choose a
sender identity. The host derives the sender from the invocation that owns the
private channel. MAS performs route admission on actual Request/Result exchanges.
The host also rejects a permitted task request whose frozen topology forbids
its reply, before preparing or dispatching that request. This preflight protects
the host's reply-requiring execution contract without changing MAS wire rules.

## Design choices

One shared Rust core gives every language the same admission and recovery semantics.
Separate full hosts would multiply the persistence and recovery implementations that
must agree. A Python factory layer would retain the integration dependency that
motivated this project. The shared process contract provides the language boundary.
Embedding would require its own lifecycle and ABI contract, justified by measured
requirements.

This local implementation uses the earlier host as a behavioural reference. Its state
belongs to a separate storage contract, so migration from the earlier databases requires
explicit support. Protocol, host IPC, application configuration and generated bundles
have independent versions. Resuming retained work requires the original executable,
bundle and root input.

Application bindings use `mas-agents/3`. Generated worker records use `mas-bundle/3` and
always carry a public operation, environment scope and working directory. Input/output
schemas belong to operations and are enforced for every implementation kind. Only
source-function workers carry a handler binding. TOML/YAML authoring and typed SDK
builders feed the same resolved configuration. Strict JSON remains the generated bundle
and worker-frame representation.

The first profile deliberately uses the existing public protocol store API. The protocol
database rejects unrelated tables, so the host maintains a separate execution journal.
The host records intent before reserving a call, then records admission before granting
worker permission. Recovery preserves uncertainty when interruption leaves a gap between
those steps. One process owns the run. A later shared-storage implementation must
preserve atomic admission, ownership fencing and original deadlines before claiming
distributed support.

The optional [live observer](observation.md) combines the protocol's public read-only
`Observer` snapshot with the host's execution journal. The protocol owns storage
compatibility and payload filtering, while the host owns its checkpoints. Both reads
leave expiry reconciliation and protocol transitions to execution. The two databases
retain separate commit and observation boundaries.

## Application execution

A typed Go or Python function, or a supported Rust/C++ JSON-object function, becomes an
agent handler through its agent configuration binding. The build tool writes its worker
entry point and bundle under `.mas/build/`. Keep these generated build outputs out of
source control.

Topology is an explicit permission graph. The application calls another actor
through its context, names the durable step, and decides how the result affects
the solution. A `parallel` request joins a bounded group in input order. A loop
belongs in application code, with distinct stable keys per iteration and a
business stopping condition. Runtime message and time limits bound execution.

The replay unit is a named request or admitted effect. After interruption, orchestration
may restart and reuse recorded child results. Code between those checkpoints must
therefore be deterministic and free of external effects. Put external writes and network
calls inside admitted effects, and use named child requests for durable concurrent work.

Request admission uses the protocol's typed outcome. A known mailbox-capacity refusal
permits up to eight attempts per dispatch, with the same message and step identities,
under the original run deadline. Delays start at 25 ms and double to a 250 ms ceiling.
Cancellation interrupts further attempts. Database access ends before each delay.
Exhaustion retains the request as prepared and keeps its ancestors replayable. Explicit
resume starts another bounded dispatch under the same durable request identity and
original deadline.

Permanent refusal is retained as rejected. An uncertain admission result blocks that
dispatch even if a later observation finds no message. Explicit resume may resubmit the
identical message: protocol duplicate detection establishes whether that request was
admitted before any worker starts. This permission applies to message admission only. An
admitted external effect with an uncertain outcome continues to require evidence-backed
resolution before execution can resume. Step evidence retains the structured admission
code, disposition and attempt count. Worker IPC keeps its existing error string
contract.

Authoring readers are format adapters selected by extension or an explicit CLI option.
They produce the same MAS value domain before schema, topology and workflow validation.
This keeps file syntax out of admission and recovery. JSON parsing for native messages
and persisted bundles remains a separate strict boundary. Native builders select the Go,
Rust, C++ or Python backend. The Python backend packages its sources in an archive for
the shared host. The [authoring contract](authoring.md) specifies the conversion and
identity rules.

An effect descriptor identifies all behaviourally relevant parameters without
credentials. The context requests admission, executes only when authorised, and records
output with evidence. An interrupted admitted effect remains uncertain. The host blocks
successful completion until evidence resolves that uncertainty. An operator-supplied
resolution records an assertion about the outside world. The operator must verify that
assertion against external evidence. Resolution preserves the original effect identity
and deadline, with repetition still subject to the effect recovery contract.

## Initial supported domain

The implemented target is a trusted local Linux deployment, with native executables and
a bounded single-run process owner. Go uses typed application functions and standard
modules. Rust and C++ initially use standalone source functions with the SDK's JSON
value type. Whole-workspace Rust dependencies, CMake target discovery and arbitrary
native type-to-schema derivation are later build profiles with their own implementation
and acceptance work.

Python supports module files inside an application project, dataclass input and
output adapters, and explicit configuration builders. The generator freezes
application Python sources and the SDK into a deterministic executable archive,
and binds the interpreter selected at build time. Third-party packages remain
installed in that interpreter's environment and require deployment pinning.

The worker guardian supplies process supervision and resource ceilings. Code runs with
the local operator's filesystem permissions, so hostile-code isolation requires
additional deployment controls. Generated binaries and their dependencies must be
treated as immutable for a retained run. Shared libraries and runtime environment need
deployment pinning in addition to the executable digest.

This supervision profile requires foreground workers and tools whose descendants remain
in the inherited process group. The pinned guardian terminates that group and waits for
adopted children. A child that creates another session or process group can therefore
delay cleanup and ownership-lock release beyond the deadline. Supporting daemonising
tools requires a stronger upstream guardian contract. Keeping cleanup in that guardian
preserves one owner for process lifetime.

Maintained LLM and workflow implementations are described in [LLM agents and
composition](llm-workflows.md). The optional [A2A service](a2a.md) adds authenticated
ingress. Explicit exports and principal grants determine which operations it publishes,
and a separate service journal retains task identity. Model credentials use scoped
environment references. Credential management, human approval queues, distributed
scheduling and Python-host state migration remain extension work. Unsupported languages
and fields fail configuration validation before execution.

## Qualification and extension sequence

The acceptance suite must exercise an application repository without handwritten
transport or registration code in each supported language. It must also verify route
admission, result reuse, changed identity rejection, a lost effect response,
evidence-based resolution, call-budget enforcement, expiry and descendant cleanup.
Recovery claims require those interruption and replay checks alongside successful sample
output.

Further releases should extend these boundaries in this order:

1. Publish signed host distributions containing generators and SDK assets, then consume released protocol packages with a tested compatibility matrix.
2. Add full Cargo/CMake build profiles and type-derived schemas while retaining
   the same worker contract. Explicitly version any wider numeric profile.
3. Add maintained effect integrations with documented provider idempotency,
   evidence and credential binding. Generalise agent implementation kinds only
   alongside their executable contracts.
4. Extend the implemented bounded A2A ingress only with explicit migration and
   recovery contracts. Qualify shared storage, leases and ownership fencing before
   allowing multiple host owners.
5. Add richer durable workflow/human-input APIs if application evidence shows the
   named-request model is insufficient. Preserve explicit business control flow.

The repository split is already real: this project builds from a pinned remote
dependency rather than a sibling MAS checkout. Existing `mas`, `mas-demo` and
examples remain independent consumers or references.

## Command adapters and repository ownership

The configuration resolver produces typed native, command, LLM and workflow variants.
Native builders own source/function binding. The command builder owns retained tool
files and executable identity. Every variant resolves to the same supervised worker
contract. The core reads launch properties without inspecting provider-specific
configuration.

Function wrappers let application code coordinate requests, parallel work and
domain-specific effects. Ordinary CLI tools use the maintained command adapter, which
supplies their transport and admission duties. A generic plugin system would require a
wider lifecycle and compatibility contract than these four supported implementations
need. The explicit variants keep selection inspectable while each implementation owns
its boundary behaviour.

A maintained command adapter owns argument binding, stdin/stdout conversion and
external-outcome classification. Provider adapters and the command adapter share bounded
process I/O through `mas-host-process`. The existing Linux guardian owns all descendant
cleanup. Operation schemas share `mas-host-schema` across the resolver, core and
runtime. These shared components have no protocol-store or workflow dependencies.

Application demonstrations belong in `mas-host-examples`. Local `tests/fixtures` retain
only inputs required to verify this host's compiler integration and recovery. Changes to
the current authoring and bundle contracts migrate those fixtures and the examples
together. Validation accepts the current schemas and rejects prior versions.

## Assessment and revision

The optional [assessment/revision controller](assessment-revision.md) uses existing
request and effect execution. The core owns immutable invocation contexts, disclosure
inheritance, evaluator binding and bounded control. The protocol owns the versioned
assessment payload types and reference validation. The public observer applies reader
restrictions to derived records, messages and effect evidence. The local operator
inspection remains an administrative interface.
