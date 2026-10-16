# Live observation

`mas-host observe --state DIR --run ID` returns a JSON snapshot suitable for a
terminal, local server or message-flow display. It works while the execution
owner is running. The command opens the existing SQLite files read-only and
does not create state, acquire the execution-owner lock, migrate a database,
reconcile deadlines, receive messages or acknowledge deliveries.

```sh
mas-host observe --state .state --run demo-001
mas-host observe --state .state --run demo-001 --include-payloads true
```

The second command explicitly exposes retained application content. The default
omits message payloads, step input/output/evidence and call evidence. Environment
values, the retained bundle, provider credentials and worker logs are never read
by the observer. Application authors must still avoid secrets in actor names,
operation names, task identifiers and step keys, because these identifiers are
part of the default observation.

## JSON contract

The response version is `mas-host-observation/1`.

| Field | Meaning |
| --- | --- |
| `run_id` | Retained run identity. |
| `observed_at_ms` | Snapshot completion time in Unix milliseconds; not a message send time. |
| `payloads_included` | Whether application content was explicitly requested. |
| `cancelled` | Committed cancellation intent in the execution journal. |
| `run` | Frozen protocol `config`, `active_protocol`, `deadline` in Unix seconds, recorded `expired` flag and `usage` counters. |
| `messages` | Accepted protocol envelopes ordered by durable insertion sequence. |
| `steps` | Execution journal checkpoints in creation order. |
| `calls` | Protocol effect reservations in creation order. |
| `tasks` | Recorded task state in creation order. |

Each message contains `sequence`, `message_id`, `run_id`, `sender`, `recipient`,
`version`, `kind`, `task_id`, `reply_to`, `operation`, `received` and `acked`.
The three correlation/operation fields are strings or null. Request envelopes
identify their `task_id` and `operation`; Result and Failure envelopes identify
the same task and their original request in `reply_to`. With opt-in, `payload`
contains the complete retained payload object.

Each step contains `id`, `parent`, `key`, `kind`, `actor`, `target` and `state`.
`kind` distinguishes `request` from `effect`; `parent` links child operations to
the requesting checkpoint. The root request has an empty parent and key `root`.
Effect state can remain `executing` or `unknown` after a lost response; a display
must preserve that uncertainty. With opt-in, `input`, `output` and `evidence`
contain the retained application objects or null where no result exists.

Each call contains `id`, `actor` and `state`. Its identity matches its effect
checkpoint. Protocol states are `reserved` and `recorded`; they do not replace
the finer host effect states. With opt-in, `evidence` contains the retained
protocol evidence or null.

Each task contains `task_id`, `request_id`, `state`, `terminal_message_id` and
`closed_at`. The final two fields can be null; `closed_at` is recorded Unix
seconds. A task's result is a participant's recorded claim, not independent
proof of an external action.

## Polling and consistency

Consumers can poll every 250–1000 ms and deduplicate message animations by
`message_id` or `sequence`. Reconnecting consumers obtain all retained messages;
they should distinguish historical replay from newly observed traffic. Sequence
is insertion order in this run's local protocol database. It is not a clock,
latency measurement or distributed causal order. The schema retains no message
admission timestamp, so the observer does not invent one.

Each database is read in a short transaction, with a 250 ms busy timeout. The
journal and protocol snapshots are separate because execution commits them
separately. A checkpoint and its corresponding message can therefore appear in
adjacent polls. Intermediate checkpoint states can be missed between polls;
this API is a snapshot, not a complete history of state changes or token stream.
The recorded `expired` flag changes only when execution or a protocol operation
reconciles the deadline. Observation alone never changes it.

Unknown runs, partially initialised runs and busy databases return the ordinary
CLI error object on stderr and a nonzero exit code. A polling server may retry
these transient conditions while its run process remains active. This adapter
accepts execution journal version 1. Protocol storage compatibility is checked
by the dependency's public `observation::Observer` API; the host does not query
protocol tables or reproduce their schema. Unsupported versions are rejected without migration.
No wire message, payload rule or protocol transition is added by observation.

Request admission failures retain `evidence.admission` with stable protocol
`code`, `disposition` and diagnostic `message` fields. `admission_attempts`
records attempts within the latest dispatch. A `prepared` request following
`retryable` admission exhausted that bounded dispatch without admission; a
`blocked` request with `unknown` admission needs explicit resume or investigation.
These fields require payload opt-in because they are retained execution evidence.
They describe message admission, not the outcome of an external effect.
