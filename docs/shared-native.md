# PostgreSQL native invocation slice

Build with `cargo build --locked -p mas-host --features shared-native` to enable
`mas-host shared-native`. This first P5c consumer uses the protocol's existing
PostgreSQL host repository for run identity, checkpoint, allocation, admission,
completion and uncertainty. The maintained worker guardian owns process cleanup.
The local state directory contains only lifetime locks. It holds no execution
journal and supplies no shared authority.

The supported operation is one credential-free source-function actor with empty
configuration, explicit object input/output schemas and exactly the client request
and reply routes. A single native invocation reserves its configured allocation.
Child requests and application effect frames stop the operation conservatively.
Managed privacy, revision, shared protected subscribers and automatic takeover
require their separately qualified implementations.

The operator provisions the existing native host/controller repositories and
allocation scopes before execution. The host never creates database schemas,
raises allowances or resets task identities. Its authority file has this shape:

```json
{
  "version": "mas-host-shared-native-authority/1",
  "schema": "qualified_namespace",
  "deployment": "deployment-id",
  "controller": "controller-id",
  "epoch": 1,
  "principal": "principal-id",
  "owner": "owner-id"
}
```

Set `MAS_POSTGRES_DSN` through the operator's credential mechanism and run:

```sh
mas-host shared-native --authority authority.json --bundle bundle.json \
  --input input.json --state .lifetimes --run original-run-id \
  --agent registered-actor --reservation-microusd 0
```

The initial CLI accepts a local PostgreSQL endpoint or an authenticated local
tunnel. The core API accepts an already authenticated `PostgresRepository` and
therefore keeps TLS and database-promotion policy with the existing connector.
The worker receives no database credentials. Controller leases and epochs remain
native protocol responsibilities.

Frozen run metadata binds the original input, validated bundle and executing
host binary. Reusing an identity with changed input fails. A consuming typed
`FreshDispatchPermit` authorises the only worker launch. A retained receipt,
definite refusal or uncertain database result grants no execution permission.
A lost response leaves the original invocation and allocation available for
reconciliation. Repeating the same CLI request reads that invocation and
performs no worker launch, including after a completed source action loses its
reply. A controller handover fences subsequent database mutations by old handles.
An already admitted external action keeps its original uncertainty.

The native authority supplies the original deadline. The host translates the
remaining database-clock allowance into the guardian's local clock frame, with
the authority round trip and a one-millisecond timestamp-rounding margin included
conservatively. Database/host clock offset
therefore cannot silently extend the remaining allowance. A queued task can impose
a stricter existing business/active-time deadline through `NativeRun.max_deadline_ms`. Successful completion needs
a schema-valid result and confirmed guardian cleanup, followed by the existing
native completion transaction. A failed observation commit withholds the result
and preserves uncertainty. This implementation supplies the ordinary trusted
slice's execution contract. Deployment privacy, availability and performance
claims require further evidence.

`cargo test --locked -p mas-host --features shared-native --test shared_native
-- --ignored --nocapture` runs the real PostgreSQL and native-compiler test.
It requires `MAS_TEST_POSTGRES_DSN`. The test independently records source actions,
then verifies completion/replay and committed source work with a lost reply,
preserved reservations and absence of redispatch. An unexecuted or ignored test
provides no PostgreSQL conformance result. The dedicated CI job supplies the
actual database service rather than a mock.

The [shared native model](../formal/shared-native.md) checks permit/replay, lost
admission replies and fencing in a bounded state space. It retains separate
implementation and PostgreSQL qualification obligations.

## One bounded queue turn

`shared-drain` consumes the existing native service queue with the complete
service-through-retention repository installation. Submission remains
with its authenticated native `task.submit` boundary. The operator registers an
immutable workflow descriptor and a `mas-native-implementation/1` source package
through the existing definition commands. Its descriptor and source package both
bind `bundle_digest`. Source files retain exact relative names, content bytes
and verified SHA-256 digests. Those retained source bytes establish package
identity. Build reproducibility and runtime attestation require separate evidence.

```sh
mas-host shared-drain --authority authority.json --bundle bundle.json \
  --state .lifetimes --agent registered-actor --workflow registered-workflow \
  --workflow-identity exact-descriptor-sha256 --turn original-turn-id \
  --after 0 --page-size 1000 --max-pages 4 --max-claims 1
```

A turn examines at most 64 pages of at most 1,000 candidates and claims at most
sixteen tasks. The default claims one task. It traverses `task.pending`
continuations and checks/claims a specific candidate through the current native
scheduler. A blocked first page therefore leaves later eligible work reachable.
Principal rotation applies within each observed page. The returned continuation
revisits a partially examined page and advances only after a complete page.
`candidate_snapshot_tail_seen` describes that scan's observed tail.
`global_queue_exhaustion_established` stays false.

Only a fresh task-claim receipt reaches the native callback. The callback still
needs its separate consuming invocation permit. A replayed claim, ambiguous
claim result, stale definition or changed bundle supplies no worker permission.
A task's original business expiry and remaining active budget cap its worker's
absolute deadline. Completion and uncertainty settle that scheduling attempt
under the original task/run identity. Releasing its worker slot preserves native
call charges and any unresolved execution obligation. The turn performs no
automatic recovery or external-operation retry.

This entry point establishes a bounded native queued slice. Automatic recovery
capacity, shared protected-data execution and deployment fairness/capacity
qualification remain subsequent work. Local A2A limits retain their existing
values. Provider request allowances have a separate authority contract; they do
not grant permission to retry a native effect.


## Authenticated shared A2A ingress

With `shared-native`, the existing `serve-a2a` boundary accepts this optional
administrator-owned field in a `mas-host-a2a/1` configuration:

```json
"shared_queue": {
  "version": "mas-host-a2a-shared-native/1",
  "authority_file": "/operator/authority.json",
  "service_epoch": "original-service-epoch",
  "workflows": {"registered-actor": {
    "name": "registered-workflow", "identity": "exact-descriptor-sha256"
  }}
}
```

The existing bearer authentication, export grants, descriptor and schema checks
run before PostgreSQL submission. Each export has one exact immutable native
workflow/source-package binding. Its principal limits, global queue policy,
allocation and scheduler must already be provisioned by the operator. Ingress
neither provisions allowances nor dispatches workers. `shared-drain` remains the
native queue consumer. No shared task is inserted in the local SQLite queue.

The logical deployment/schema, original service epoch, authenticated principal
and original `messageId` determine one request, task and command identity. The
canonical digest includes business input, actor and immutable workflow/bundle
identity; RPC ids, JSON whitespace/key order and `returnImmediately` do not
change it. A lost response is reconciled by repeating that original message at
another ingress with the same logical authority/service epoch. Conflicting
content cannot replace the original task. Credential rotation and controller
handover do not create a fresh request namespace. Changing the authority file
while a service is running fails closed and requires an operator restart.

`GetTask` and `CancelTask` require the exact authenticated principal, current
export grant and original service/epoch/workflow binding. `GetTaskStatus` uses
the native `task.status` projection: one owned lookup without input, output,
history, deadline reconciliation or the authority writer lock. Cancellation
records intent in the existing task owner; it makes no claim that an external
operation stopped. Full result delivery separately validates its retained
schema and availability. An unavailable database, ambiguous admission or
replayed receipt grants no worker permission.

This initial shared ingress supports only the credential-free single-function
native slice. It rejects governed-local/managed privacy execution and custom
`contextId`; those profiles require shared governance and context contracts.
The real PostgreSQL HTTP test submits through one ingress, repeats through
another, rejects a different principal and changed content, then checks that
one native drain produced one independent source-ledger action and neither
local service acquired a task queue record. Sustained workload, provider-specific
integration and full shared protected-data execution remain qualification and
implementation obligations, respectively.
