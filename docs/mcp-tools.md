# LLM-directed MCP tools

An LLM actor can select a tool exposed by a custom MCP server through an ordinary
MAS delegate. The application supplies an MCP client adapter as a `command` or
`native` implementation; the host admits, executes and records that operation
using its existing request and effect contracts. This arrangement requires no
new Nehemiah message or host implementation kind.

The Kubernetes-specific server belongs with the application in
[`mas-host-examples/examples/k3s-live`](https://github.com/emmett08/mas-host-examples/tree/main/examples/k3s-live).
It owns tool definitions, Kubernetes access, resource restrictions and evidence
redaction. The host remains responsible for execution identity, permitted routes,
budgets and recovery. Another application can supply a different MCP client and
server under the same contracts.

## Request path

1. An LLM actor declares `delegates` naming executable application actors and a
   `max_turns` budget. The host includes each delegate's operation and schemas in
   the model instructions.
2. The model returns a request decision containing the delegate name and
   `input_json`. The host validates the decision, parses exact JSON, checks the
   operation input and admits the request through Nehemiah.
3. A command actor obtains effect admission before starting its MCP client
   adapter. The adapter establishes an MCP connection and invokes its configured
   tool once. MCP protocol messages use that connection; MAS request and result
   messages use the host's existing channel.
4. The adapter converts the MCP response to the actor's bounded output schema.
   The host records the command outcome and supplies the result in the next
   inference turn's `history`.
5. The model requests another permitted delegate or returns its final output.

Use one registered actor per approved tool so tool permissions, input contracts
and observations remain individually inspectable. The adapter fixes the MCP
server and tool name in application configuration. Neither value should come
from the model's arguments. MCP discovery is not permission to execute every
advertised tool: the registered delegates and server-side policy define the
available operations.

See the [LLM decision contract](llm-workflows.md#composition-and-model-directed-requests)
and [command configuration](commands.md). The existing
[`command-policy` example](https://github.com/emmett08/mas-host-examples/tree/main/examples/command-policy)
demonstrates the same model-request, command-result and subsequent-inference
sequence without an MCP connection.

## Adapter contract and deployment

An ordinary command receives its operation input through `stdin = "json"` or
explicit argument bindings, and returns one JSON object on stdout. An MCP server
does not implement this CLI contract. A client adapter must translate between
that single invocation and the MCP session. Protocol diagnostics belong on
stderr; stdout contains only the operation result.

For a stdio server, the adapter starts and owns a foreground child process and
its protocol streams. For a separately deployed server, the adapter connects to
the configured endpoint. Each command invocation performs at most one tool call;
session initialisation is connection setup. The adapter must not include hidden
model turns, tool retries, parallel tool calls or additional mutation operations
inside that one admitted operation.

Retain the client implementation and its resources using the command builder's
project-relative executable and `file` argument bindings. A retained script does
not include neighbouring imports automatically. Package dependencies into one
executable or pass each immutable resource as a declared file. Interpreters,
third-party package installations and separately deployed MCP servers remain
pinned deployment dependencies. Record the MCP protocol and server version in
tool results when the application needs that provenance; the host cannot infer
remote implementation identity from the client executable digest.

The k3s example retains its single-file `project/mcp-client.mjs` bridge and fixes
the tool name in each command actor. Its MCP SDK and compiled server module graph
are supplied by the deployed application image. Keep that image and its installed
dependencies immutable for retained runs; retaining the bridge alone does not
retain those modules. Environment references identify permitted variable names,
not immutable values. The application must bind and check its selected context
and server artefacts when they supply part of a run's identity.

Grant credentials through the command actor's `environment` list. Model-provider
credentials belong to the LLM connection; Kubernetes credentials belong to the
tool service or its adapter. Server endpoints, namespace/resource grants and
credential references are deployment configuration. A model-supplied resource
name must be checked against that grant before any Kubernetes request.

Tool arguments and results use the host's exact integer JSON subset. Convert
decimal measurements explicitly to strings or scaled integers and reject
unsupported values; do not silently round them. Bound the entire encoded result,
including escaped strings and metadata. The accumulated request/result history
and embedded configuration must still fit the 65536-byte worker frame limit.

This is a trusted-code deployment profile. A command shares the operator's
filesystem permissions; the host's environment filtering and process supervision
do not sandbox an untrusted MCP adapter or server.

## Outcomes, limits and replay

`max_turns = 5` permits at most four delegate requests followed by one completion
turn. Five model calls and four command calls require nine admitted effects,
before accounting for later assessment or other application operations. Set the
application's `max_calls`, message, byte and wall-clock limits for the complete
workflow. The host rejects a delegate request on the final turn before dispatch.

Use `min_delegate_calls = 1` for an investigation that must request a tool before
completion. The host excludes `complete` from the response schema until that many
delegated requests return, and rejects premature completion locally. The setting
defaults to zero and must be less than `max_turns`. It counts completed delegate
requests, including returned tool error observations; it does not require distinct
tools or establish evidence quality. Application acceptance must check the
recorded results. A model that violates the required decision schema fails with
a retained invalid outcome rather than an automatic retry.

Command `timeout_seconds` is capped by the original run deadline. The host stops
the local process group on interruption; neither process termination nor MCP
cancellation proves that a remote tool stopped. A stdio server and its descendants
must preserve the inherited process group and remain in the foreground.

A validated successful MCP result is a completed command outcome. A known tool
error can also be a completed observation: the adapter returns a schema-valid
object with an explicit failure status and an accepted exit code, allowing the
model to use that observation. It must not convert an unknown remote outcome
into a claimed successful tool execution. Lost responses, process failures and
timeouts follow the command contract's uncertain-outcome rules and block
automatic repetition.

Completed tool calls replay from their recorded result, including reads of
changing external state. Replaying an investigation does not collect newer
Kubernetes evidence. A new investigation requires a new run identity. Forward
the command's `invocation_id` binding when the server supports correlation or
idempotency; the identifier alone does not establish exactly-once execution.

## Assessment and provider boundaries

For an assessed investigation, trusted application code builds the assessment
input from actual recorded tool results and freezes that evidence before draft,
assessment and revision. The LLM's concluding prose is a candidate interpretation
of the observations. It cannot create evidence by claiming that an unrequested
tool returned a particular result. Independent assessment can then check
citations against the frozen evidence used for that candidate.

Read tools can support investigation while the application's existing remediation
controller owns changes. If mutation tools are added later, retain a separate
admitted operation and verification policy for each external action.

The provider connection `kind = "command"` is a separate extension point: it
adapts a model provider to the inference request/response contract. It is not an
application tool actor or an MCP registration mechanism. Enabling a provider's
own hidden tools inside that adapter would place their effects outside the
host's per-tool admission and recovery records. Use declared delegates for this
integration; provider-native tool calling would require its own host contract.
