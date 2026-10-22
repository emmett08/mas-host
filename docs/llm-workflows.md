# LLM agents and composable workflows

The host gives native functions, LLM adapters and recursive workflow plans the same MAS
request/result path. An actor's public operation stays independent of its
implementation, so changing provider preserves the application's communication contract.
The host constructs MAS envelopes and derives the authenticated sender. The model
supplies a decision through the selected provider API.

## Configuration and provider selection

A `mas-agents/3` document contains `agents`, optional named `connections` and
optional protocol `limits`. An actor declares `name`, optional `operation`
(defaults to its name), and `implementation`:

```toml
version = "mas-agents/3"

[connections.review_provider]
kind = "openai"
api_key_env = "OPENAI_API_KEY"

[[agents]]
name = "reviewer"
operation = "review/1"
input_schema = "schemas/input.json"
output_schema = "schemas/output.json"
[agents.implementation]
kind = "llm"
connection = "review_provider"
model = "<explicit-model-id>"
instructions = "prompts/review.md"
max_output_tokens = 4096
```

`kind = "native"` selects `language`, `source` and `handler`, using the supported
language build profiles. `kind = "workflow"` selects a relative `plan` file. `kind =
"command"` adapts an ordinary CLI through the [command contract](commands.md).
Input/output schemas belong to the agent operation for every implementation kind.
Instructions, schemas and plans must remain inside the application project. The builder
embeds their contents in the immutable generated bundle. Each actor exposes one public
operation in this profile. Separate actors can expose separate operations.

Connections support `mock`, `openai`, `openai-compatible`, `cursor` and `command`.
See the [provider contract](../crates/mas-host-providers/README.md) for exact fields,
HTTP dependencies, endpoint rules and the versioned command extension port.
External providers need no changes to the scheduler, protocol or application SDKs.

`build` and `run --project` accept `--agents FILE` and `--connections FILE`. Relative
paths are resolved against the project. A `mas-connections/1` profile replaces named
connections, optionally declaring a model override for each. Unknown aliases are
rejected. Profiles let one set of actors and workflow plans run against different
providers. An unavailable provider fails explicitly. Mock output requires an explicit
mock connection.

Credentials are environment references such as `api_key_env: OPENAI_API_KEY`.
Workers receive that variable and names explicitly listed in connection `env`,
alongside a fixed minimal PATH. Secret values are excluded from bundles and
journals. Inspection and cancellation do not need provider credentials.

OpenAI Responses and compatible HTTP adapters require `curl`. Cursor uses the official
SDK pinned in `assets/providers/package-lock.json`. Install it with `npm ci --prefix
assets/providers` and Node 22.13 or later. A Cursor command can refer to
`@sdk/assets/providers/cursor.mjs`. The builder resolves this distribution asset and
binds its digest. The worker checks that digest before execution. Cursor disables tools,
MCP, subagents, ambient settings and hidden retries in a fresh workspace. The adapter
records `token_limit_enforced:false` because the SDK provides no documented output-token
cap. Live calls require credentials and provider-available model identifiers.

## Composition and model-directed requests

The [workflow contract](../crates/mas-host-workflow/README.md) defines recursive
sequence/parallel groups, branches, repeat, foreach, named definitions and exact
JSON Pointer bindings. Nested parallel groups submit ready leaves in deterministic
batches of eight. All actor requests enter the existing MAS authority.

Star, tree and mesh describe the permitted edges in `mas-topology/1`. Plans and native
code determine execution order and joins. A reciprocal reply edge remains an initiation
permission too in this topology profile. One local owner enforces the execution bounds:
at most 31 application actors, 32 concurrent workers, request depth 16 and 1024 journal
steps. Configured protocol budgets apply as well. Arbitrary combinations of the
supported control structures work within these explicit resource bounds.

The opt-in `mas-candidate-quorum/1` parallel join counts approvals from distinct
requested actors only when they identify the same caller-bound candidate and
frozen review context. It retains conflicting reviews and a deterministic
selection receipt. Ordinary parallel plans retain every result without an
agreement policy. See the [candidate agreement contract](../crates/mas-host-workflow/README.md#agreement-on-one-candidate)
for configuration, reference validation and the limits of an accepted quorum.

Before preparing an executable request, the host checks that a permitted forward route
also permits its correlated reply. An impossible reply therefore fails before admitting
the request, starting the worker or admitting its effects. The MAS protocol still owns
actual route admission. Its generic transport may carry one-way messages, while the host
task contract requires a reply route.

An LLM actor can optionally declare `delegates: ["peer"]` and `max_turns` greater than
one. Each inference returns `decision`, either `{kind: complete, output: ...}` or
`{kind: request, target: peer, input_json: "{...}"}`. The executor validates the action,
parses `input_json` through MAS's exact JSON codec and requests the peer through the
host. The string carries arbitrary task JSON while preserving exact integers and a
strict provider response schema. Subsequent turns receive the original `task` and prior
request/result `history`. Each permitted peer's public operation and available schemas
are included in the instructions. Command actors use the same delegation and topology
permissions as native or LLM actors. An application can expose a custom MCP tool through
such a delegate. The [MCP adapter integration](mcp-tools.md) defines ownership, effect
and recovery rules. `client` is reserved for ingress and cannot be a workflow request
target or an LLM delegate. The builder checks that targets are executable application
agents. The protocol then checks communication permissions at request admission.

Set optional `min_delegate_calls = 1` when an actor must obtain a peer observation
before completing. The non-negative integer defaults to zero, must be smaller than
`max_turns`, and requires a non-empty `delegates` list when positive. Until that many
peer requests have returned successfully through the host, each turn's response schema
permits only `request`. The runtime checks the same condition before accepting
completion. The count measures completed requests, including schema-valid application
error observations, rather than distinct tools or useful evidence. Applications must
still validate which observations satisfy their task. A provider that ignores the schema
produces a retained invalid outcome and stops under the existing turn budget. Further
execution follows the ordinary recovery contract. Omitting the field preserves the
previous resolved configuration and its digest.

Set optional `required_delegates = ["pod", "events", "workload", "logs"]` when
completion requires observations from particular peers. Entries must be unique members
of `delegates`, and their count must be smaller than `max_turns`, leaving one turn for
completion. Until every required peer has returned, the response schema permits requests
only to the remaining required peers, in an order chosen by the model. Completion
additionally respects `min_delegate_calls` when set. After coverage, the model can
request any permitted peer within the remaining turns. On the final turn, a non-empty
required list permits only completion. Runtime checks also enforce the selected target
and completion conditions when using retained outcomes. Omitting the field or supplying
an empty list preserves the previous decision schemas and behaviour. Omission also
preserves configuration identity. Returned application errors count as observations, so
applications must still check whether the collected results justify assessment or
action.

## Outcomes, recovery and validation

Every inference turn is a separate admitted effect. Valid output, refusal, invalid
output, rejected requests and confirmed non-dispatch are recorded as known observations.
Lost provider outcomes remain uncertain and block automatic repetition. A completed
response survives cleanup failure, but that invocation does not report task success.
Replaying uses the retained provider response and peer results. Cancellation interrupts
local reads and prevents further dispatch or durable task success. Remote inference may
still be running, so its outcome can remain uncertain.

Internal effect descriptors use `mas-inference/2` and `mas-command/2`. They retain the
per-turn input and a SHA-256 digest of the compact JSON resolved worker configuration.
The execution journal retains that full configuration in its immutable bundle, allowing
each effect frame to reference the binding through the digest. Adapter evidence formats
retain their separate `/1` versions. Complete native frames, including input, history
and framing fields, still have the 65536-byte bound. Hashing configuration does not
enlarge that limit.

The schema validator supports `type`, object properties/required/additionalProperties,
array items/length, string length, exact integer minimum/maximum, enum and anyOf, plus
title/description. Unsupported keywords fail before provider execution. Each provider
checks its own schema support. The host also validates every result locally.
Schema-valid output remains a candidate requiring application acceptance. The host
limits admitted call counts. Enforcing monetary reservations or a supplier billing cap
requires a separate spending policy and implementation.

`make test` covers Rust/Go/native contracts. `make provider-test` verifies the Cursor
SDK boundary offline. CLI integration checks execute actual MAS tasks, provider outcome
recovery and replay. The examples repository adds full topology and composition
acceptance. Offline tests establish the exercised adapter contracts. Live account/model
availability and answer quality require separate observations.

## SOLID and selected patterns

| Boundary | Principle/pattern | Verification |
| --- | --- | --- |
| Provider-specific HTTP/SDK mapping | Adapter and Provider Strategy, with one external boundary per adapter | Shared request/outcome contract plus recorded response fixtures |
| Provider construction | Factory at worker composition root | Explicit selection, with unavailable providers failing explicitly |
| Recursive workflow nodes | Composite with a small Runtime interface | Nested combinations and independent arithmetic oracle |
| Native SDK and built-in worker | Facade over host admission | Real cross-agent requests and unchanged native acceptance |
| Durable requests and inference turns | Commands with stable identities | Replay adds no provider calls or protocol messages |

These boundaries separate changes in provider APIs, workflow dataflow, process
supervision and protocol authority. The interpreter depends on its small Runtime
interface. Inference policy depends on Provider, which keeps vendor SDK details inside
their adapters. Provider and runtime substitutions preserve ordered results, explicit
failures and budgets. Adding a service bus, inheritance hierarchy or provider server
would add ownership and failure semantics without a demonstrated need in the current
local profile.
