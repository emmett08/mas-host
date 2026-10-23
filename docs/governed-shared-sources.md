# Authorised shared source reads

`GetSharedSource` delivers an approved retained acquisition to an
independently authenticated recipient. It reuses the retained source reader and
the existing execution journal. Every read checks both the producer's current
session and the recipient's current grant, full original ancestry and complete
task exposure. The operation performs no acquisition, dispatch, recovery or
settlement. An unavailable source stays unavailable under its original identity.

The current implementation supports a trusted-local service-reader profile.
It admits multiple authenticated subscribers to one retained source. Shared
distributed execution, atomic distributed revocation and automatic watch/fetch
coalescing retain their separate authority/scheduling requirements.
Live watch registration, long-lived subscription delivery and acquisition
cancellation also remain separate increments. Each request here observes one
retained projection under its original collection identity.

## Publish a completed acquisition

A producer can publish its own retained source after collection when the
operator selects `"source_publication": true` in the complete A2A2 configuration.
The service requires governed local/2. This option belongs to its retained
configuration identity. Enable it before starting the service or through an
explicit retained-service migration.

The producer submits its original `SendMessage`, waits for its retained source
to complete, then negotiates:

```text
A2A-Extensions: urn:mas:host-task-service:1, urn:mas:host-governed-source-publication:1
```

Publish that owned task's exact durable source-effect key:

```json
{
  "jsonrpc": "2.0",
  "id": "publish-1",
  "method": "PublishRetainedSource",
  "params": {
    "id": "a2a-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    "key": "collect",
    "reference": "stable-acquisition-specific-alias"
  }
}
```

The authenticated producer must own the selected task and retain current
`resolve` and `publish-source` authority under the complete original source and
task restrictions. The publication operation uses the producer principal as
recipient and `client` as destination. Current authentication, grant, policy,
expiry, lifecycle and configured inspection obligations all apply. A registered
publisher acquires no recipient permissions by publishing a reference.

Publication uses local snapshot admission. The core checks the complete source
policy and binding before the service transaction, then the service rechecks
the publisher's authentication lease and grant immediately before commit.
The published status records the immutable selection. Availability and recipient
permission are checked afresh on every read. This boundary supplies no atomic
distributed revocation or recall of an already admitted response.

A successful result has this shape:

```json
{
  "version": "mas-governed-source-publication/1",
  "reference": "shared-REPLACE_WITH_THE_RETURNED_64_HEXADECIMAL_DIGITS",
  "status": "published"
}
```

The opaque reference commits the logical alias, exact source task, effect key
and actor under the publisher's identity-commitment key. The raw alias has at
most 128 UTF-8 bytes. It is excluded from retained source-publication rows and
responses. A second keyed alias reservation and keyed body commitment preserve
the original exact binding. Lookup authenticates that body under the current
producer session before deriving its owned task selection.
Repeating the same publication returns the same reference. Reusing that alias
for another source fails with `source_publication_conflict`.

Use an acquisition-specific alias for each fresh observation. A coordinator can
derive it from the canonical query digest and original acquisition message ID.
That preserves retries of the original fetch while allowing a later observation
to have its own immutable publication identity. A historical recipient reference
continues to identify its original retained observation.

The existing service SQLite journal stores active publication selections and
immutable alias reservations. The number of reservations is bounded by the
configured `max_tasks`; retirement preserves those reservations. Offline
`govern-lifecycle ... --service-state ...` removes the retired task's active
publication bodies along with its A2A output. It preserves alias reservations
so retirement cannot free an old alias for rebinding. A complete independently
retained service-identity epoch is still required to preserve reservation history
when restoring a backup that predates a reservation. Even after such history
loss, an existing returned reference cannot select a different task because
its keyed identity binds that exact task/key/actor.

The application passes the returned opaque reference through its approved
publication channel. Each recipient then calls `GetSharedSource` with that
reference under its **own** current bearer and policies. Owned-task access stays
with the producer. Publication creates no collection, worker dispatch or
execution recovery. A source coordinator may coalesce acquisition through this
interface while retaining its own source scope, scheduling, freshness and
coverage contract.

## Predeclare a fixed publication

An A2A2 service using `mas-governed-local/2` may declare up to 64 exact source
exports in its operator-owned service configuration:

```json
{
  "source_exports": [{
    "name": "deployment-point-001",
    "task_id": "a2a-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    "key": "collect",
    "tenant": "tenant-a",
    "principal": "collector-service",
    "purpose": "diagnosis",
    "grant_ref": "collector-grant",
    "actor": "collector"
  }]
}
```

This is an additional field in a complete A2A2 service configuration. Replace
the example task ID with the exact approved task. Ordinary A2A services and
governed local/1 reject the field. Unknown fields, repeated reference names,
unknown exported actors and invalid task/key selections reject configuration.

The selection binds the exact producer authentication tuple, source task, root
actor and durable effect key. At read time the service reconstructs the producer
session from the **current** authentication table and verifies its ownership of
that retained task. It never reconstructs or exposes a bearer. Removing the
producer's authentication record or current grant prevents sharing. Rotation
of the producer's bearer under the same authority tuple preserves source identity.

The source export belongs to the immutable service configuration fingerprint.
Predeclare the collection's exact task reference before starting that service,
or use an explicitly verified retained-service migration. A changed source
export cannot silently relabel existing service state. Trusted operator tooling
can compute the prospective task ID through the existing `GovernedSession`
identity commitments: commit `["a2a-owner", tenant, principal]`, commit
`["a2a-message", original_message_id]`, then SHA-256 the compact JSON array of
those two commitments and prefix the first 56 hexadecimal digits with `a2a-`.
Use the maintained session API to obtain those commitments. The HTTP integration
test demonstrates this calculation before the first collection.

The public/extended Agent Card advertises the method contract, optional extension
and byte bounds. It publishes no source-reference names, producer authentication
tuple, task IDs or grant references. Clients obtain their approved reference
through the application's authorised publication path.

## Request and admission

Negotiate both extensions:

```text
A2A-Extensions: urn:mas:host-task-service:1, urn:mas:host-governed-shared-source-read:1
```

Then submit the closed request under the recipient's own bearer:

```json
{
  "jsonrpc": "2.0",
  "id": "read-1",
  "method": "GetSharedSource",
  "params": {"reference": "deployment-point-001"}
}
```

The reference has at most 128 UTF-8 bytes and uses the existing bounded identity
alphabet. The client selects no task ID, actor, scope, policy, filesystem path
or effect key. Extra fields are invalid parameters. The recipient must have a
current authentication grant for the selected exported actor. Source policies
must independently permit that recipient's `resolve` operation, purpose,
destination, location and permitted payload scope. Producer and recipient must
belong to the same current tenant/policy domain and baseline-policy set.

The result uses `mas-governed-source-view/1`, preserving its exact source identity,
metadata, complete ancestry, host settlement, external execution certainty and
payload disposition. The recipient receives an available projection only when
its current policies permit it. Original source bytes discarded during handling
remain unavailable. The source metadata's complete coverage refers to its
retained closed structured projection and binding. Detector quality and broader
text coverage retain their separate qualifications.

An unknown reference, foreign tenant, missing producer, unavailable source or
denied source produces the same bounded `source_not_found` error. Missing or
revoked authentication returns the existing HTTP `401` response. A missing
negotiated extension produces `required_shared_source_extension_missing`.
Ordinary owned-task APIs preserve their original ownership rules.

## Validation and limits

```sh
cargo test --locked -p mas-host shared_source
```

The real-HTTP cases cover extension negotiation, unpublished and cross-tenant
reads, rejection of caller-selected scope/path fields, producer bearer rotation,
removed producer authentication, restart without reacquisition and **33 current
subscribers** reading the same retained source. Revoking the final subscriber
withholds its read while another authorised subscriber continues. Journal
snapshots and collector counts verify that reading creates no execution work.
Dynamic-publication cases cover owned selection, idempotent restart, foreign
tasks, baseline/source publication denials, changed-alias binding conflicts,
producer bearer rotation, recipient revocation and retirement without rebinding.
Core tests additionally exercise the subscriber policy and lifecycle gates.

Every delivery uses current file-backed authority and exposure snapshots. An
authority change may follow the final admission check before network delivery.
This profile supplies no distributed linearisation or recall of an already
admitted transmission. The default source closure retains its bound of 64
records and the existing 65,536-byte representation constraints. The opt-in
[referenced-lineage contract](governed-source-lineage.md) admits a separately
bounded larger complete closure and publishes authorised metadata pages.
Excess, missing or conflicting ancestry prevents admission. Deployment
confinement retains its separate implementation and measured evidence.
