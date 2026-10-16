# Native execution contract

Status: initial local Linux profile. The worker is trusted application code, not a
sandbox. This contract is independent of the MAS message protocol. MAS remains
the authority for routing, task transitions and call reservations.

Application repositories supply agent and topology configuration, application
functions and their normal build metadata. The CLI generates worker launchers
and an immutable bundle in `.mas/build/`. Configuration never comes from task
input. Generated files are disposable build outputs.

## Application configuration

The agent document contains `version: mas-agents/3`, `agents`, and optional
`connections` and `limits`. Each agent has `name`, an `implementation` object,
and an optional public `operation` that defaults to its name. Optional
`input_schema` and `output_schema` paths describe that operation for every
implementation kind. Commands and LLMs require an output schema.
`environment` lists granted environment-variable names; `working_directory`
is project-relative, defaults to `.`, and must remain inside the project. `client` is
reserved for host ingress. Unknown fields are errors.

A native implementation has `kind: native`, `language` (`go`, `rust`, `cpp` or
`python`), `source` (a project-relative directory for Go; a source file for
Rust/C++/Python), and `handler` (a function name). Commands have a [CLI adapter contract](commands.md). Workflow and LLM implementations
have their own bindings in the [LLM and workflow contract](llm-workflows.md).

`limits` may set `wall_seconds` (default 60, maximum 3600), `max_calls` (default
32), `max_messages` (default 1000), `max_bytes` (default 16777216), and
`mailbox_capacity` (default 32). These are application requests under the local
operator's authority, not grants for a multi-tenant service. The CLI enforces
bounded maxima. Remote deployment policy is a later profile.

The topology document has exactly `version` (`mas-topology/1`) and `edges`: explicit
directed pairs of actor names. Both request and reply edges are required.
Topology permits communication; application code supplies order and decisions.

JSON, TOML and restricted YAML are alternative authoring syntaxes for these
same versioned documents. The [authoring contract](authoring.md) defines
discovery, conversion and format-independent validation. Bundles and native
frames retain their strict JSON contract.

## Generated bundle and implementation identity

The bundle contains `version: mas-bundle/3`, an `agents` array of
`{name, executable, sha256, operation, environment, working_directory}`, and `topology` in MAS's native
run format (including `agents`, `edges`, `application: agent-task/1` and the
above limits). Every generated worker record includes its public operation;
optional `arguments` and `configuration` bind maintained command, LLM and workflow
workers. A `handler` is present only for function workers. Optional operation
schemas are embedded and validated by the host before dispatch and completion.
The build retains executables under their content digests and a bundle snapshot
for each build. The host hashes its own executable and the canonical bundle,
and rechecks each worker digest before dispatch. Absolute executable locations
are part of this initial local identity. A changed bundle cannot resume an
existing run. Contract versions and compiled SDK code are part of the binding.
Native runtime/library dependencies must be pinned by
the deployment; an executable digest alone does not identify shared libraries.
Python workers retain application Python sources and SDK modules in a generated
executable archive. Changes to those sources change the executable digest.
The selected interpreter and installed third-party dependencies must remain
pinned for the lifetime of retained runs, just like native runtime libraries.

## Framing

Workers communicate through the private inherited `MAS_WORKER_CHANNEL_FD` socket.
Standard output/error are not the protocol. Each frame is a four-byte big-endian
length followed by UTF-8 JSON (1..65536 bytes). MAS strict JSON validation applies:
integer numbers only, unique keys, bounded depth, and valid Unicode. Inputs,
outputs, effect descriptors and evidence are JSON objects. Applications can
represent decimals with scaled integers or strings. No automatic floating-point
rounding is permitted.

The host sends:

```json
{"type":"start","version":"mas-native/1","actor":"planner","handler":"Plan","input":{},"configuration":{},"invocation_id":"stable-id","deadline_ms":1792112400000}
```

Function-worker start frames include `handler`; configured built-in workers omit
it and dispatch from their immutable configuration.

Only one request may be outstanding on a worker channel. The SDK serialises
access. A handler may send these commands:

| Command | Fields | Reply |
|---|---|---|
| `request` | `key`, `target`, `input` | `response`, `ok`, `output` or `error` |
| `parallel` | `requests`: array of `{key,target,input}` | `response`, `ok`, `outputs` in input order or `error` |
| `effect_begin` | `key`, `descriptor` | `effect`, `state: execute`, `invocation_id`; or `state: cached`, `output`; or `response`, `ok:false`, `error` |
| `effect_complete` | `key`, `output`, `evidence` | `response`, `ok:true`; or `ok:false`, `error` |
| `complete` | `output` | Ends worker execution |
| `error` | `message` | Ends worker execution as a failure |

Keys are stable application step names, unique within their parent invocation.
Repeating a key with different input, target or descriptor is an error. Completed
child requests and effects are replayed from durable records. Incomplete pure
handler code may execute again. Code outside explicit effect boundaries must be
deterministic and free of external effects. Threads do not acquire durability;
use the SDK's bounded `parallel` request for concurrent durable child work.

## External effects and recovery

The host records an effect intent and reserves a MAS call before replying
`execute`. The application may then perform the external operation and return
both output and evidence. An interrupted admitted effect remains uncertain and
prevents successful completion, even if application code catches its error.
There is no automatic retry of an uncertain effect and no exactly-once claim.

An operator may resolve an uncertain effect with a confirmed output and
non-empty evidence using the administrative CLI. This records an operator
assertion; the host cannot establish the truth of external evidence by itself.
SDK integrations may supply domain-specific verification. Resolution does not
extend the original run deadline or create another call reservation.

The host uses separate protocol and execution journals, under one process-owner
lock. Prepared intent is durable before protocol reservation, and permission is
sent only after durable admission. Crash windows are resolved conservatively;
this initial profile does not claim a distributed atomic transaction. Workers
and their process groups are supervised through MAS's existing Linux guardian.
The guardian retains the ownership lock until descendants are reaped.

The guardian clears the inherited environment, discards worker
standard output/error, limits address space to 2 GiB and files to 64 MiB, and
bounds CPU time by the remaining deadline. Workers receive a fixed minimal PATH
and their declared environment references; LLM connections contribute their
credential names. Retained application logs remain a later profile. Cancellation records durable intent,
interrupts local worker reads and is checked before committing successful task
completion. Already-observed effect outcomes remain recordable after cancellation.
The original deadline remains the hard lifetime bound; neither local termination
nor cancellation establishes that a remote provider stopped work.

The worker frame version is `mas-native/1`; application configuration and bundle
versions describe separate contracts. The Rust SDK's configured entry point
receives task input and immutable configuration separately. The public operation
remains independent of the implementation handler.

## API boundary

The host owns execution identity, deadlines, admission, journal state and
reconciliation. SDKs own framing and language error conversion. Application
functions own decisions, step keys, acceptance conditions and the meaning of
external effects. SDKs cannot declare success while the host retains uncertainty.
