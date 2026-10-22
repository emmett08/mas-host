# Native execution contract

This contract defines native execution in the initial trusted local Linux profile.
Workers run with the operator's process privileges. MAS governs routing, task
transitions and call reservations, while this separate contract governs the host–worker
exchange and its durable execution checkpoints.

Application repositories supply agent and topology configuration, application
functions and their normal build metadata. The CLI generates worker launchers
and an immutable bundle in `.mas/build/`. Configuration never comes from task
input. Generated files are disposable build outputs.

## Application configuration

The agent document contains `version: mas-agents/3`, `agents`, and optional
`connections` and `limits`. Each agent has `name`, an `implementation` object, and an
optional public `operation` that defaults to its name. Optional `input_schema` and
`output_schema` paths describe that operation for every implementation kind. Commands
and LLMs require an output schema. `environment` lists granted environment-variable
names. `working_directory` is project-relative, defaults to `.`, and must remain inside
the project. `client` is reserved for host ingress. Unknown fields are errors.

A native implementation has `kind: native`, `language` (`go`, `rust`, `cpp` or
`python`), `source` (a project-relative directory for Go or a source file for
Rust/C++/Python), and `handler` (a function name). Commands have a [CLI adapter
contract](commands.md). Workflow and LLM implementations have their own bindings in the
[LLM and workflow contract](llm-workflows.md).

`limits` may set `wall_seconds` (default 60, maximum 3600), `max_calls` (default 32),
`max_messages` (default 1000), `max_bytes` (default 16777216), and `mailbox_capacity`
(default 32). These are application requests under the local operator's authority. The
CLI enforces bounded maxima. A multi-tenant service requires a later profile with its
own remote deployment policy and grants.

The topology document has exactly `version` (`mas-topology/1`) and `edges`: explicit
directed pairs of actor names. Both request and reply edges are required. Topology
permits communication. Application code supplies order and decisions.

JSON, TOML and restricted YAML are alternative authoring syntaxes for these
same versioned documents. The [authoring contract](authoring.md) defines
discovery, conversion and format-independent validation. Bundles and native
frames retain their strict JSON contract.

## Generated bundle and implementation identity

The bundle contains `version: mas-bundle/3`, an `agents` array of `{name, executable,
sha256, operation, environment, working_directory}`, and `topology` in MAS's native run
format (including `agents`, `edges`, `application: agent-task/1` and the above limits).
Every generated worker record includes its public operation. Optional `arguments` and
`configuration` bind maintained command, LLM and workflow workers. A `handler` is
present only for function workers. Optional operation schemas are embedded and validated
by the host before dispatch and completion. The complete generated bundle, including
embedded schemas, prompts, plans and delegate contracts, must fit the strict JSON limit
of 65536 UTF-8 bytes and its nesting bounds. The builder uses compact JSON and validates
the assembled bundle before publishing it. A rejected rebuild preserves the previous
bundle and its snapshot. Check the assembled representation: embedded files and delegate
contracts together consume the bundle's byte allowance. The build retains executables
under their content digests and a bundle snapshot for each build. The host hashes its
own executable and the canonical bundle, and rechecks each worker digest before
dispatch. Absolute executable locations are part of this initial local identity. A
changed bundle cannot resume an existing run. Contract versions and compiled SDK code
are part of the binding. The deployment must pin native runtime and library dependencies
alongside the executable digest. Python workers retain application Python sources and
SDK modules in a generated executable archive. Changes to those sources change the
executable digest. The selected interpreter and installed third-party dependencies must
remain pinned for the lifetime of retained runs, just like native runtime libraries.

## Framing

Workers communicate through the private inherited `MAS_WORKER_CHANNEL_FD` socket. The
guardian discards standard output/error. Each channel frame is a four-byte big-endian
length followed by UTF-8 JSON (1..65536 bytes). MAS strict JSON validation applies:
integer numbers only, unique keys, bounded depth, and valid Unicode. Inputs, outputs,
effect descriptors and evidence are JSON objects. Applications must represent decimals
explicitly with scaled integers or strings.

The host sends:

```json
{"type":"start","version":"mas-native/1","actor":"planner","handler":"Plan","input":{},"configuration":{},"invocation_id":"stable-id","deadline_ms":1792112400000}
```

Function-worker start frames include `handler`. Built-in workers dispatch from their
immutable configuration and omit that field.

Only one request may be outstanding on a worker channel. The SDK serialises
access. A handler may send these commands:

| Command | Fields | Reply |
|---|---|---|
| `request` | `key`, `target`, `input` | `response`, `ok`, `output` or `error` |
| `parallel` | `requests`: array of `{key,target,input}` | `response`, `ok`, `outputs` in input order or `error` |
| `effect_begin` | `key`, `descriptor` | `effect`, `state: execute`, `invocation_id`, or `state: cached`, `output`, or `response`, `ok:false`, `error` |
| `effect_complete` | `key`, `output`, `evidence` | `response`, `ok:true`, optional governed `completion`, or `ok:false`, `error` |
| `complete` | `output` | Ends worker execution |
| `error` | `message` | Ends worker execution as a failure |

Keys are stable application step names, unique within their parent invocation. Repeating
a key with different input, target or descriptor is an error. Completed child requests
and effects are replayed from durable records. Incomplete pure handler code may execute
again. Code outside explicit effect boundaries must be deterministic and free of
external effects. Use the SDK's bounded `parallel` request for concurrent durable child
work. Ordinary threads remain part of the handler computation that may restart.

## External effects and recovery

The host records an effect intent and reserves a MAS call before replying `execute`. The
application may then perform the external operation and return both output and evidence.
An interrupted admitted effect remains uncertain and prevents successful completion,
even if application code catches its error. Resuming that effect requires evidence-based
resolution under its original identity. The contract provides conservative recovery
after uncertainty, with exactly-once external execution remaining outside its
guarantees.

A governed host may replace the callback's output with an authoritative safe
projection in its successful completion acknowledgement:

```json
{"type":"response","ok":true,"completion":{"version":"mas-governed-effect-completion/1","effect_id":"effect-1","host_settlement":"completed","external_execution":"completed","payload_disposition":"available","output":{"redacted":true}}}
```

`completion` is a closed object with exactly the six fields shown. Its version
is `mas-governed-effect-completion/1`. `effect_id` must equal the identity granted
by `effect_begin`, and `host_settlement` must be `completed`.
`external_execution` is one of `completed`, `not-sent`, `refused` or `unknown`.
This records external certainty separately from the host's completed settlement.
`payload_disposition` is `available` or `withheld`, and `output` is an object.
The SDK returns this host output immediately. Cached admission returns the same
host-retained output without invoking the callback. When `completion` is absent,
the SDK retains the ordinary behaviour of returning the callback's output.

For `withheld`, `output` has exactly these three fields, with
`external_execution` matching the completion envelope:

```json
{"version":"mas-governed-unavailable/1","external_execution":"completed","payload_disposition":"withheld"}
```

This marker is reserved and cannot be used as an `available` projection. The
SDK returns it as an ordinary object. The host effect remains completed, and
replay never grants another external execution. Built-in command and inference
workers recognise the marker before outcome or schema processing and return
the static error `effect completed; result unavailable under current handling policy`.
This delivery failure does not convert known host settlement into uncertainty.
An unknown version, extra or missing field, mismatched identity, invalid enum,
non-object output or malformed withheld marker fails with
`invalid governed effect completion`. SDKs neither expose the callback's raw
output as a fallback nor retry the operation. Python raises `HostError` for
this post-settlement validation failure without setting `EffectUncertain`.
Loss of the acknowledgement itself still requires reconciliation.

An operator may resolve an uncertain effect with a confirmed output and non-empty
evidence using the administrative CLI. This records an operator assertion whose truth
the operator must verify against the outside world. SDK integrations may supply
domain-specific verification. Resolution does not extend the original run deadline or
create another call reservation.

The host uses separate protocol and execution journals, under one process-owner lock.
Prepared intent is durable before protocol reservation, and permission is sent only
after durable admission. Recovery preserves uncertainty across crash windows between the
two journals under this single-owner profile. Workers and their process groups are
supervised through MAS's existing Linux guardian. The guardian retains the ownership
lock until descendants are reaped. Supported workers and tools run in the foreground and
preserve the inherited process group. The guardian terminates that group and waits for
adopted children. Descendants that detach into another session can delay cleanup beyond
the deadline.

The guardian clears the inherited environment, discards worker standard output/error,
limits address space to 2 GiB and files to 64 MiB, and bounds CPU time by the remaining
deadline. Workers receive a fixed minimal PATH and their declared environment
references. LLM connections contribute their credential names. Retained application logs
remain a later profile. Cancellation records durable intent, interrupts local worker
reads and is checked before committing successful task completion. Already-observed
effect outcomes remain recordable after cancellation. The original deadline prevents
further admission and durable task success. After local termination or cancellation,
remote provider work may still be in progress and its outcome still requires
reconciliation.

The worker frame version is `mas-native/1`. Application configuration and bundle
versions describe separate contracts. The Rust SDK's configured entry point receives
task input and immutable configuration separately. The public operation remains
independent of the implementation handler.

## API boundary

The host owns execution identity, deadlines, admission, journal state and
reconciliation. SDKs own framing and language error conversion. Application
functions own decisions, step keys, acceptance conditions and the meaning of
external effects. SDKs cannot declare success while the host retains uncertainty.
