# Optional Nehemiah A2A service

Publish selected host operations through the **A2A 1.0 JSON-RPC profile**. The gateway
uses the local CLI's immutable bundle and `Engine`, then adds authenticated service
admission and durable task records. Nehemiah is the protocol's public name. Existing
`mas-*` package, configuration, envelope and profile identifiers remain stable.

The wire version remains `1.0`. The [source pin](a2a-source-pin.json) identifies
the official `v1.0.0` proto, specification and extension guidance at commit
`173695755607e884aa9acf8ce4feed90e32727a1`, with verified Git blob and SHA-256
digests. The retained-effect extension follows that version's extension method,
dependency, activation and response acknowledgement rules. These source checks
bound the extension review. Independent SDK interoperability retains its own
validation requirement.

```sh
mas-host describe --bundle application-bundle.json
mas-host serve-a2a --bundle application-bundle.json --state .service --config service.json
```

`describe` validates worker bindings using the execution core and projects only actor
names, operations, input/output schemas, a bundle digest and the numeric and
message-size profile. It excludes executable paths, working directories, environment
bindings, provider settings and prompts. The description lets clients inspect a
contract. Execution still requires an authorisation grant, and quality requires separate
assessment.

## Explicit publication and grants

A service configuration has this form. Replace the example token hash with SHA-256 of a
securely generated bearer token of at least 32 printable bytes. Give the original token
only to the client.

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

The listener accepts only a literal loopback address with a nonzero port. `public_url`
must identify `/a2a` using HTTPS, or literal loopback HTTP for local use. For remote
access, put an HTTPS reverse proxy in front of the listener. Preserve the bearer,
version and extension headers, and protect access to the loopback port. This profile
does not implement OAuth or accept proxy-supplied principal names. Browser `Origin`
requests are rejected. Public cards contain no credentials, and HTTP responses disable
caching.

## Wire profile

| Operation | Behaviour |
|---|---|
| `GET /.well-known/agent-card.json` | Public card and explicitly public contracts, available without a token |
| `GetExtendedAgentCard` | Authenticated card with the principal's permitted contracts |
| `SendMessage` | Submit one JSON data part to a pinned exported actor and return a durable task |
| `GetTask` | Observe only a task owned by the authenticated principal |
| `CancelTask` | Persist cancellation intent and return `-32002` while cancellation remains unconfirmed |
| `GetRetainedEffect` | Governed A2A2 extension: read one root-workflow effect under current resolve authority |
| `GetRetainedSource` | Governed local/2 extension: resolve one exact retained source projection and its full ancestry |

RPCs use `POST /a2a`, `Content-Type: application/json`, `Authorization: Bearer ...`
and `A2A-Version: 1.0`. Task methods additionally require:

```text
A2A-Extensions: urn:mas:host-task-service:1
```

The required extension is advertised in `capabilities.extensions`. Its
`params.descriptor` contains the published operation contracts and `bundle_digest`. The
response acknowledges the supported activated extension and omits unknown optional
extension URIs. Missing required extension returns `-32008`, unsupported version
`-32009`, and unsupported push-notification methods `-32003`.

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

Use the published input schema. A successful submission returns `result.task`. Use its
`id` in `GetTask` with `params: {"id": "..."}`. Completed output is in the task's
`result` artefact as a JSON data part. A supplied `contextId` groups messages but
confers no authority. Task IDs and data remain principal-scoped.

With `returnImmediately: false` (the default), the service waits for at most
five seconds. A deadline or unknown outcome returns a JSON-RPC error whose
`data` contains `google.rpc.ErrorInfo` metadata with the retained `taskId` and
`contextId`. Poll that task or repeat the exact original message. Blocking
timeouts do not cancel work or create another execution.

The profile accepts exact-integer JSON only. The MAS codec rejects duplicate keys,
fractional numbers, exponent notation and malformed Unicode. Clients must preserve these
values. SDKs that convert JSON numbers through floating-point representations need
application-defined decimal strings or rational records to preserve exact values at the
gateway.

## Governed retained-effect reads

A `mas-host-a2a/2` service advertises the optional extension
`urn:mas:host-governed-effect-read:1`. Its `params.requires` names the existing
task-service extension. `params.methods`, `requestFields`,
`additionalRequestFields`, `maxKeyUtf8Bytes`, `requestExample`, `resultVersion`
and `scope` describe the method and its bounded request. The example field is
illustrative. Requests must supply an actual task ID and exact logical effect
key.

Activate both extensions to call the method:

```text
A2A-Extensions: urn:mas:host-task-service:1, urn:mas:host-governed-effect-read:1
```

```json
{
  "jsonrpc": "2.0",
  "id": "read-case-1",
  "method": "GetRetainedEffect",
  "params": {
    "id": "a2a-REPLACE_WITH_THE_OWNED_TASK_ID_SUFFIX",
    "key": "privacy-case-intent"
  }
}
```

The response acknowledges both activated URIs. The additional URI is optional
for existing task operations and mandatory for `GetRetainedEffect`. Omission
returns `-32008` with `required_effect_extension_missing`. Ordinary A2A1 services
keep their existing extension set and unknown-method behaviour. Unsupported
optional URIs are ignored and excluded from the acknowledgement.

The request has exactly `id` and `key`. A key contains 1–128 UTF-8 bytes without
control characters. The task must belong to the authenticated owner and its
actor must remain an authorised export. The service validates the retained
scope fingerprint before deriving a run directory. The core then selects that
namespace's exact root invocation. Clients supply neither filesystem paths nor
actor, tenant or parent-invocation identities.

Current policy must explicitly permit operation `resolve`, recipient equal to
the authenticated principal, and destination `client`, for the task's workflow
actor, purpose and location. Every applicable policy participates. A configured
inspector also needs its current `inspect` permission for the query and any
returned application payload. Existing `deliver` permission alone grants no
effect-read authority.

The result is the closed `mas-governed-effect-view/1` object:

| Field | Meaning |
|---|---|
| `version` | `mas-governed-effect-view/1` |
| `namespace_ref` | Opaque keyed reference to the admitted namespace |
| `actor`, `parent_invocation`, `key` | Exact workflow, root invocation and logical effect key |
| `effect_id` | Original durable effect identity, or null when no effect is recorded |
| `state` | Recorded step state, or `not-recorded` in this snapshot |
| `host_settlement` | `completed`, `pending`, `failed` or `unknown` |
| `external_execution` | `completed`, `not-sent`, `refused` or `unknown` |
| `payload_disposition` | `available`, `withheld` or `unavailable` |
| `descriptor`, `output`, `evidence` | Currently permitted retained application values, or null |

`prepared` and `executing` steps retain pending host settlement and unknown external
execution. `completed` and `resolved` require the retained completion record to agree
with the output and evidence. A known completion whose payload is withheld keeps its
original external certainty. An authorised missing key returns `not-recorded`, unknown
settlement/execution, unavailable payload and null values. Repeating the effect still
requires its separate admission and recovery contract.

The reader checks query authority before opening a bounded read-only SQLite snapshot. It
validates the namespace and original effect identity, releases the snapshot, then checks
current rights for the complete view. Execution continues under its existing owner lock.
The reader returns stored task and effect states, leaving dispatch, settlement and
reconciliation to execution. A configured detector may run within its existing
processing and resource contract.

Retained application values undergo their declared schema and current inspection
checks. If those payload checks fail after an authorised lookup, a separately
authorised metadata view can retain settlement and certainty while returning
null application values and a withheld disposition. A failed query inspection,
revoked metadata right, unavailable policy, invalid namespace or corrupt record
returns the uniform `-32001` `effect_not_found` error. Authentication rejection
continues to use the service's ordinary HTTP authentication response.

The exact view is bounded to 65,536 bytes. Each read can consume one query
inspection budget and one application-payload inspection budget, within the
existing inspector-enabled HTTP connection bound. Lease and current scope are
rechecked before and after the read. The file-backed profile retains its
documented snapshot authority semantics.

Consumers combining case-intent and notification views must require matching
`namespace_ref`, `actor` and `parent_invocation`, then validate their exact logical
keys, event and destination binding. Each HTTP call observes its own snapshot, so
consumers must validate the relationship between the returned records. The [privacy
response example](../examples/privacy-response/README.md) keeps action state, recipient
delivery and payload availability separate.

## Governed retained-source reads

An A2A2 service configured with `mas-governed-local/2` additionally advertises
`urn:mas:host-governed-source-read:1`. Activate that URI and the required task-service
URI to call `GetRetainedSource` with exactly `{"id":"TASK_ID", "key":"acquire"}`. The
key selects an effect in the owned task's root workflow. The effect-reader extension is
independent. Acknowledgements contain only the supported activated URIs. The source
extension is available only to governed local/2 services.

The same authentication, owner, actor, namespace and key bounds apply as for
`GetRetainedEffect`. Missing activation returns `-32008`
`required_source_extension_missing`. Missing, denied or corrupt scope returns the
uniform `-32001` `source_not_found`. Current `resolve` authority is required for the
query, source metadata, full ancestor closure and returned projection. Every original
source policy and current applicable policy participates. Owned-task source and effect
reads also conjoin the complete current parent invocation exposure. An earlier source
cannot bypass a later restriction or a pending acquisition in that task. The response's
`lineage` describes the selected source's exact ancestry. Later unrelated sources also
constrain admission, while remaining outside that provenance view.

The closed `mas-governed-source-view/1` result contains:

| Fields | Meaning |
|---|---|
| `version`, `namespace_ref`, `actor`, `parent_invocation`, `key` | Version and exact owned workflow/source selector |
| `effect_id`, `state`, `host_settlement`, `external_execution` | Original retained effect identity, settlement and certainty |
| `source_id`, `source_metadata` | Host-bound source identity and its `mas-governed-source/1` metadata, or null |
| `lineage` | Complete bounded source metadata closure, including the selected source |
| `payload_disposition`, `projection` | Available retained effect-output projection, or an explicit unavailable/withheld disposition with null projection |
| `raw_source_disposition` | Always `not-retained`, preserving the distinction between the retained projection and discarded original bytes |

An absent or unfinished effect has no retained output source and returns an unavailable
projection. A known completion with withheld output keeps its settlement and certainty.
Returned metadata binds provenance and policy to the source. Freshness and detector
quality require separate evidence, and full per-component detection assessments require
their own retained records.

The reader resolves source bindings and complete ancestry through the existing read-only
journal snapshot. It observes retained task/effect states independently of the execution
owner. Fresh acquisition and recovery remain separate execution operations. The complete
closure must fit 64 sources and 65,536 bytes. An oversized, missing, cyclic or invalid
closure denies the read, preserving the requirement to consider every ancestor. The
complete response retains the 65,536-byte bound and at most the existing two inspection
budgets. After the final policy and inspection callbacks, a pure read rechecks the exact
namespace, root and full exposure before returning. Exposure growth or a pending
acquisition causes uniform denial. Every database snapshot is released before callbacks.
These checks retain the file-backed profile's snapshot semantics. Distributed
linearisation requires a separately coordinated authority path.

For local/2, existing task delivery, completed submission replay and effect reading also
enforce inherited source restrictions. Revoked or expired source rights cannot be
bypassed by choosing an older endpoint. Queued host status before a root invocation
exists remains governed by its baseline status policy. The first immediate submission
response is the authorised, payload-free `submitted` admission snapshot captured before
worker launch, with current authentication checked again before delivery. It reports
acceptance at that point, rather than a later execution state. Duplicate submissions and
later task reads use the full retained source closure. While acquisition remains
pending, a reader may receive the uniform unavailable error. Keep the source
restrictions and output uncertainty in force until an authorised retained result is
available. Polling retains its original task ID and deadline. See [governed
acquisition](governed-acquisition.md) for profile configuration, source exposure
inheritance and the trusted-local limitations.

Streaming, push notifications, `ListTasks`, task continuations, arbitrary file parts,
remote effect resolution and dynamic membership are outside this profile. A card
describes published metadata. Adding an actor or route requires a separate change to the
run's configuration and authority.

## Durability, cancellation and uncertainty

A private SQLite journal records the authenticated principal, message identity, complete
canonical submission, task identity and state before any worker starts. Repeating the
same principal and `messageId` with the same content returns that task. Changed content
is rejected. The response RPC ID is independent of task identity. Accepted identities
remain retained after completion and cancellation. `max_tasks` therefore limits lifetime
accepted submissions as well as bounding the stored identity set.

The service binds its state to the complete bundle, configuration and host
executable. An executable or configuration change, including a token hash or
grant change, rejects reuse of that state. There is currently **no migration
command**. Keep the serving executable and generated bundle immutable. Rebuilding
or replacing an in-use binary can prevent later execution identity checks.
Plan credential rotation and upgrades as explicit deployment migrations that
retain the original task records and outstanding-effect obligations.

On restart, unfinished `submitted` and `working` records become `unknown`. They are
never dispatched automatically, even if the process might have crashed before starting a
worker. A repeated original message observes that same record. Completed records retain
their outputs. Every execution error is conservatively reported as `unknown`. The
service journal retains a private bounded diagnostic for investigation. The remote
response does not expose internal error text.

Use task `metadata.mas.state` to recover these distinctions when deciding how to
proceed. A2A `UNSPECIFIED` leaves the underlying recovery state unresolved. Cancellation
persists intent and forwards it to the existing core journal. It does not establish that
remote provider work or external effects stopped. Cancellation observed before any
worker dispatch closes the service task as `failed`. After dispatch, an uncertain
outcome remains `unknown` until evidence establishes what happened.

The local core's `inspect`, `observe` and evidence-backed `resolve` commands remain
available for ordinary runs under `.service/runs`. Governed retained effects use the
authorised read extension above. Resolving a core effect does **not** automatically
settle or resume a gateway task. This first profile requires operator review of unknown
records. Remote and automatic reconciliation require a later execution contract.
Creating another state directory or message identity does not establish that repeating
its external action is safe.

## Bounds and verification

Hyper parses HTTP. The service allows 32 simultaneous connections, 32 headers, a 16 KiB
HTTP parsing buffer, a 65,536-byte request body and a 262,144-byte response. Header and
body reads each have a three-second deadline, and the complete connection has a
ten-second deadline. An A2A2 service with an explicitly configured [privacy
inspector](governed-privacy.md) adds two inspection budgets, up to 250 seconds, while
retaining the header/body bounds. Connection loss does not cancel accepted work.
`max_active` defaults to four (maximum 16). `max_tasks` defaults to 1,000 (maximum
10,000). The bundle's original core run budgets apply once its run is created. There is
one process owner per service state directory.

`cargo test -p mas-host --test a2a` exercises real HTTP, cards and grants, exact
retries, stale descriptors, a deterministic workflow with a fixture LLM provider,
completed restart retention, cancellation and unknown restart without duplicate provider
calls. These checks establish the exercised bounded adapter contract. Live model quality
and interoperability with other A2A implementations require separate validation. The
separate `mas-host-examples/examples/a2a-review` demonstration also checks an
independently maintained, pinned official A2A SDK.
