# Optional Nehemiah A2A service

The Rust host can publish selected operations through an **A2A 1.0 JSON-RPC
profile**. Nehemiah is the protocol's public name; existing `mas-*` package,
configuration, envelope and profile identifiers remain unchanged. The gateway
runs the same immutable bundle and `Engine` as the local CLI. It adds service
admission and task records around that execution authority.

```sh
mas-host describe --bundle application-bundle.json
mas-host serve-a2a --bundle application-bundle.json --state .service --config service.json
```

`describe` validates worker bindings using the execution core and projects only
actor names, operations, input/output schemas, a bundle digest and the numeric
and message-size profile. It excludes executable paths, working directories,
environment bindings, provider settings and prompts. This local description is
not an authorisation grant or a quality assessment.

## Explicit publication and grants

A service configuration has this form. Replace the example token hash with
SHA-256 of a securely generated bearer token of at least 32 printable bytes;
only the client receives the original token.

```json
{
  "version": "mas-host-a2a/1",
  "listen": "127.0.0.1:8765",
  "public_url": "http://127.0.0.1:8765/a2a",
  "name": "Nehemiah release review",
  "exports": [
    {"agent": "review", "name": "Release review", "description": "Review a proposed release.", "public": true}
  ],
  "principals": [
    {"name": "alice", "token_sha256": "REPLACE_WITH_64_LOWERCASE_HEX_DIGITS", "agents": ["review"]}
  ],
  "max_active": 4,
  "max_tasks": 1000
}
```

Each exported actor must have explicit input and output object schemas and both
`client -> actor` and `actor -> client` routes. Its implementation may be native,
command, LLM or workflow. Internal actors need not be exported. A `public` export
appears in the public card but still requires an authenticated execution grant.
An authenticated extended card lists only that principal's permitted exports.
Actor descriptions and schemas are approved for disclosure by this export policy.

The listener accepts only a literal loopback address with a nonzero port.
`public_url` must identify `/a2a` using HTTPS, or literal loopback HTTP for local
use. Put an HTTPS reverse proxy in front of the listener for remote access;
preserve the bearer, version and extension headers and protect access to the
loopback port. This profile does not implement OAuth or accept proxy-supplied
principal names. Browser `Origin` requests are rejected. Public cards contain no
credentials, and HTTP responses disable caching.

## Wire profile

| Operation | Behaviour |
|---|---|
| `GET /.well-known/agent-card.json` | Public card and explicitly public contracts; no token required |
| `GetExtendedAgentCard` | Authenticated card with the principal's permitted contracts |
| `SendMessage` | Submit one JSON data part to a pinned exported actor; return a durable task |
| `GetTask` | Observe only a task owned by the authenticated principal |
| `CancelTask` | Persist cancellation intent; return `-32002` while cancellation remains unconfirmed |

RPCs use `POST /a2a`, `Content-Type: application/json`, `Authorization: Bearer ...`
and `A2A-Version: 1.0`. Task methods additionally require:

```text
A2A-Extensions: urn:mas:host-task-service:1
```

The required extension is advertised in `capabilities.extensions`. Its
`params.descriptor` contains the published operation contracts and
`bundle_digest`. The response acknowledges the supported activated extension;
unknown optional extension URIs are not echoed. Missing required extension
returns `-32008`, unsupported version `-32009`, and unsupported push-notification
methods `-32003`.

```json
{
  "jsonrpc": "2.0", "id": "rpc-1", "method": "SendMessage",
  "params": {
    "message": {
      "messageId": "release-42", "role": "ROLE_USER",
      "parts": [{"mediaType": "application/json", "data": {
        "agent": "review", "input": {"release": "release-42"},
        "descriptorDigest": "DIGEST_FROM_THE_AUTHORISED_CARD"
      }}]
    },
    "configuration": {"returnImmediately": true}
  }
}
```

Use the published input schema. A successful submission returns `result.task`;
its `id` is used in `GetTask` with `params: {"id": "..."}`. Completed output is
in the task's `result` artefact as a JSON data part. A supplied `contextId` groups
messages but confers no authority. Task IDs and data remain principal-scoped.

With `returnImmediately: false` (the default), the service waits for at most
five seconds. A deadline or unknown outcome returns a JSON-RPC error whose
`data` contains `google.rpc.ErrorInfo` metadata with the retained `taskId` and
`contextId`. Poll that task or repeat the exact original message. Blocking
timeouts do not cancel work or create another execution.

The profile accepts exact-integer JSON only; duplicate keys, fractional numbers,
exponent notation and malformed Unicode are rejected by the existing MAS codec.
Clients must preserve these values. SDKs that convert JSON numbers through
floating-point representations need application-defined decimal strings or
rational records; the gateway never rounds data to accommodate a client.

Streaming, push notifications, `ListTasks`, task continuations, arbitrary file
parts, remote effect resolution and dynamic membership are outside this profile.
A card is descriptive metadata: discovery cannot add an actor or route to a run.

## Durability, cancellation and uncertainty

A private SQLite journal records the authenticated principal, message identity,
complete canonical submission, task identity and state before any worker starts.
Repeating the same principal and `messageId` with the same content returns that
task. Changed content is rejected. The response RPC ID is independent of task
identity. Accepted identities are retained after completion and cancellation;
`max_tasks` limits lifetime accepted submissions, not just active work.

The service binds its state to the complete bundle, configuration and host
executable. An executable or configuration change, including a token hash or
grant change, rejects reuse of that state. There is currently **no migration
command**. Keep the serving executable and generated bundle immutable. Rebuilding
or replacing an in-use binary can prevent later execution identity checks.
Plan credential rotation and upgrades as explicit deployment migrations that
retain the original task records and outstanding-effect obligations.

On restart, unfinished `submitted` and `working` records become `unknown`.
They are never dispatched automatically, even if the process might have crashed
before starting a worker. A repeated original message observes that same record.
Completed records retain their outputs. Every execution error is conservatively
reported as `unknown`; a private bounded diagnostic is retained in the service
journal. The remote response does not expose internal error text.

Task `metadata.mas.state` preserves these distinctions; A2A `UNSPECIFIED` alone
is insufficient for a recovery decision. Cancellation persists intent and
forwards it to the existing core journal. It does not establish that remote
provider work or external effects stopped. Cancellation observed before any
worker dispatch closes the service task as `failed`; after dispatch, uncertain
outcomes remain `unknown` rather than being reported as confirmed cancellation.

The local core's `inspect`, `observe` and evidence-backed `resolve` commands
remain available under `.service/runs`. Resolving a core effect does **not**
automatically settle or resume a gateway task. This first profile has no remote
or automatic reconciliation path; retain unknown records for operator review.
Creating another state directory or message identity does not establish that
repeating its external action is safe.

## Bounds and verification

Hyper parses HTTP. The service allows 32 simultaneous connections, 32 headers,
a 16 KiB HTTP parsing buffer, a 65,536-byte request body and a 262,144-byte response.
Header and body reads each have a three-second deadline, and the complete
connection has a ten-second deadline. Connection loss does not cancel accepted
work. `max_active` defaults to four (maximum 16); `max_tasks` defaults to 1,000
(maximum 10,000). The bundle's original core run budgets apply once its run is
created. There is one process owner per service state directory.

`cargo test -p mas-host --test a2a` exercises real HTTP, cards and grants, exact
retries, stale descriptors, a deterministic workflow with a fixture LLM provider,
completed restart retention, cancellation and unknown restart without duplicate
provider calls. These checks establish this bounded adapter contract, not live
model quality or universal interoperability with every A2A implementation.
The separate `mas-host-examples/examples/a2a-review` demonstration also checks an
independently maintained, pinned official A2A SDK.
