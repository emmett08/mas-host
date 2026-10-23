# Registered acquisition clients

These Python clients use the existing governed A2A host and its retained effect
journal. An application registers an exact read, advances one original host
submission and gives each subscriber its own authorised retained-source read.
There is no adapter database, worker scheduler or additional effect engine.

`RegisteredRead` snapshots the exact input and commits the complete contract:
authority, tenant, acquisition grant, purpose, credential/policy epochs, source,
query, acquisition generation, projection, schema/transform versions, agent,
bundle descriptor digest, effect key, original submission cutoff and the
registered host transport quota scope. Contract/input changes produce a new
original message ID and publication alias. JSON object ordering alone does not.
The public query identity uses a domain-separated HMAC with a distinct private
application identity key of at least 32 random bytes. Predictable restricted
inputs therefore produce no published plain hash suitable for dictionary
testing. The key is neither a bearer nor a public salt. The application must
authenticate and authorise registration first; a commitment is not authorisation.
A generation is an immutable acquisition identity, not a mutable name for the
newest evidence.

`Acquisition.advance` sends `SendMessage` with `returnImmediately`, reads the
bounded `GetTaskStatus` view, then calls `PublishRetainedSource` for the exact
completed task/effect. Repeating progression after a lost reply keeps the same
original message and exact parameters. Host admission and effect identity remain
authoritative. No application result is inferred from host completion. A failed
or unknown task cannot produce a new fetch under that generation. A fresh
acquisition requires an explicitly admitted new generation. The immutable
publication alias includes the acquisition identity, so a later task cannot
retarget an earlier subscriber reference.

`Subscription.deliver` calls `GetSharedSource` with that subscriber's current
bearer. A producer's task response and its artifacts are never forwarded. Current
host grants and the complete source/task restrictions decide each delivery.
Subscribers retain separate request IDs, original deadlines, freshness bounds
and cancellation flags. Cancellation suppresses only that delivery and never
sends `CancelTask`. An old acquisition time or expired source returns `stale`;
the client neither refreshes the source nor substitutes the delivery time for
its acquisition time. Original lineage remains in the authorised source view.

The publication handle and checkpoint must come from trusted application state.
A reference alone is neither proof of a registered query nor a disclosure
capability. The host independently authorises every read. The application must
durably own exact registration, the private producer checkpoint, subscriber
admission/deadline/cancellation/accounting and their recovery. `checkpoint()`
contains a private producer task ID and belongs in that journal, never in a
subscriber response or log. This in-memory client does **not** complete P4's
durable logical subscriptions or cross-replica acquisition coordination.

The private identity key belongs to the retained registration lifecycle. Keep it
stable while any admission or external outcome remains unresolved. Loading an
old checkpoint under a different key fails the query binding and cannot submit
work. Key rotation cannot silently create a replacement registration or reset
an unknown outcome. The application must resolve the old acquisition and
explicitly admit any new registration/generation under its new key.

## Use

The governed local version 2 gateway must predeclare `source_publication: true`
and export the registered collector. It must advertise `GetTaskStatus`,
`PublishRetainedSource` and `GetSharedSource` with their extension profiles.
Endpoint-to-authority binding, current bearer lookup, registration and durable
checkpoint handling are application-owned trusted configuration.

```python
from adapters.acquisition.coalescing import (
    Acquisition, HTTPRPC, RegisteredRead, Subscription,
)

registered = RegisteredRead({
    "version": "mas-coalesced-read/1", "authority": "host/service-1",
    "tenant": "tenant-a", "acquisition_grant": "producer-grant",
    "purpose": "diagnosis", "credential_epoch": "credential-1",
    "policy_epoch": "policy-1", "source": "cluster/registered-1",
    "query": {"namespace_uid": "original-namespace-uid", "kind": "Deployment"},
    "generation": "acquisition-42", "projection": "deployment-point/1",
    "schema_version": "point/1", "transform_version": "collector/1",
    "agent": "collector", "descriptor_digest": registered_bundle_digest,
    "effect_key": "acquire", "submission_deadline_ms": original_cutoff_ms,
    "transport_quota_scope": "host/service-1",
}, {"mode": "acquire"}, identity_key=private_application_identity_key)

producer = HTTPRPC(endpoint, "host/service-1", "host/service-1", current_producer_bearer)
acquisition = Acquisition(registered, producer, trusted_checkpoint=trusted_checkpoint)
progress = acquisition.advance(now_ms=now_ms, resolution_deadline_ms=resolution_cutoff_ms)
# Persist acquisition.checkpoint() in the existing trusted owner journal.
# Schedule another bounded advance according to progress; do not spin on errors.
if progress["state"] == "published":
    subscriber = HTTPRPC(endpoint, "host/service-1", "host/service-1", current_subscriber_bearer)
    delivery = Subscription("request-42", registered, subscriber,
        deadline_ms=subscriber_cutoff_ms, max_age_ms=60000)
    result = delivery.deliver(progress["publication"], now_ms=now_ms)
```

Each HTTP client admits one concurrent request, uses verified TLS, resolves its
current bearer per request, follows no redirect/proxy and performs no automatic
retry. Requests/responses are capped at 65,536 bytes; JSON depth/collection and
header scheduling fields are bounded. Error bodies, arbitrary headers and RPC
error messages/data are discarded. Socket connection/I/O timeouts are at most
ten seconds; an absolute post-connect socket cutoff stops dripping responses.
DNS resolution can outlast a socket timeout. The application must supply its
own process/request lifetime bound where that is required. Loopback HTTP is an
explicit opt-in for local hosts and fixtures.

The `a2a` provider decision gates known read/status waits within one client
instance. Repeated calls during its original cooldown perform no HTTP request.
That memory is neither durable cooldown nor a distributed quota owner. HTTP
429 from a host is never counted as GitHub/Kubernetes quota consumption. A host
mutation with an uncertain response requires reconciliation through the same
original idempotent host protocol identity. This client never retries arbitrary
external mutations.

## Provider scheduling

`provider.observe` projects registered GitHub/Kubernetes transport facts into
`mas-provider-observation/1` without accepting a body. `provider.schedule`
returns `mas-provider-schedule/1`: a bounded deferral, refusal, reconciliation,
expiry or Kubernetes relist decision. No decision grants execution permission,
reserves allowance or refunds quota. Those remain authority-owned operations.

GitHub primary exhaustion waits for both reset and `Retry-After`; secondary or
unspecified throttles use at least the documented one-minute wait. Missing,
duplicate or malformed required hints hold. A hint beyond the permitted budget
is never clamped into an earlier retry. The original observation time anchors
cooldown, so evaluating it again cannot renew it. Kubernetes list/watch 410
records a lost interval and requires a new snapshot; that snapshot cannot repair
the earlier history. The application retains the gap across subsequent replies.
When supplying a watch cursor, the caller must supply a distinct private
`cursor_identity_key` of at least 32 random bytes. Its public cursor identity is
domain-separated HMAC-SHA256, never a plain hash of a predictable resource
version. The application retains that key with the original stream lifecycle.
Timeouts or 5xx responses to mutations preserve execution uncertainty, including
after the original deadline. An accepted mutation still needs reconciliation.

Provider semantics follow the [GitHub rate-limit contract](https://docs.github.com/en/rest/using-the-rest-api/rate-limits-for-the-rest-api)
and [Kubernetes API list/watch contract](https://kubernetes.io/docs/reference/using-api/api-concepts/).

## Credential-free registered GET workers

`read_worker.PublicGET` makes one bounded GET after the host controller has
consumed both original native dispatch and registered provider request credit.
The workflow definition's `provider_quota` is the exact immutable version 2
provider circuit policy. Admission reserves native and provider credit in the
same authority transaction. Replay, quota exhaustion, circuit deferral and an
uncertain commit cannot launch a worker. The worker receives neither database
credentials nor a controller capability.

The fixed `mas-provider-public-get/1` request contract belongs in the pinned
source package. Only verified `https://api.github.com` reads and explicitly
opted-in anonymous loopback fixtures are supported. Kubernetes here qualifies
the list/watch response contract on loopback. Authenticated provider workers and
protected shared profiles remain closed. Application inputs may select an
operator-registered enum case; they cannot supply an endpoint, header, credential
or new query. The input and frozen source package bind the original native call.

The request is GET only. Headers are fixed `Accept`, `User-Agent` and an optional
registered `If-None-Match`. There is no redirect, proxy, mutation or HTTP retry.
Response bytes are capped at 32 KiB, socket time at ten seconds and output
projection at 16 KiB. A deterministic JSON pointer selection returns only
registered string/integer/boolean fields. Raw bodies and arbitrary response
headers are discarded. Projection is ordinary data; it does not establish a
managed privacy inspection or override any source restrictions.

The versioned result carries exact `provider_observation`, scheduling decision,
payload disposition, approved projection and the known HTTP status, if headers
arrived. A body timeout can therefore remain uncertain without erasing a known
status. A 304 returns `not-modified` with no new projection or acquisition time.
A Kubernetes 410 retains its lost interval. No later snapshot repairs that
history. The host retains parsed throttle deferral and provider observation
before completing the original native invocation; no quota credit is refunded.
GitHub secondary deferral includes its one-minute floor even if the raw
`Retry-After` is shorter. The controller also enforces its database-clock
cooldown. UTC retry hints assume a qualified worker clock; protected deployment
and clock agreement remain separate acceptance work. DNS can outlast the local
socket timeout; the original native guardian deadline is the hard worker bound.

The opt-in PostgreSQL tests in
`crates/mas-host-cli/tests/shared_native/provider_read.rs` execute real anonymous
HTTP under the native controller. They check approved projection, conditional
304, lost-history 410, one-time native/provider credit, quota exhaustion and
retained cooldown before another GET. Run them against an actual prepared
PostgreSQL service; compiling or skipping them is not backend qualification.

## Validation

```sh
python3 -m unittest discover -s adapters/acquisition/tests -v
python3 adapters/acquisition/verify_host.py --host target/debug/mas-host --sdk-root .
```

Tests exercise immutable host identities, lost submission/publication replies,
private checkpoint restore, changed scope/epochs/input, independently denied
subscribers, cancellation during delivery, stale-source refusal and actual
loopback HTTP error/throttle/redirect/oversize/slow-response handling. Fake RPC
tests check adapter protocol control flow; they do not qualify a shared host or
deployment. Provider HTTP fixtures exercise actual known refusals and uncertain
mutations.

`verify_host.py` exercises the real governed local host: it loses a real accepted
submission's reply, replays the original message, verifies exactly one collector
execution, publishes its retained source and reads it under two independent
reader credentials. A third reader lacks source permission. Revoking one
reader's current grant denies only that reader. Stale/cancelled subscriptions
start no acquisition. Restart preserves the exact source, lineage and original
times. The fixture uses synthetic structured evidence without the optional
neural inspector; it performs no external request or model inference.

The acceptance passed against integrated host revision `582961d`, executable
SHA-256 `591105dc08cc510f73f8b1b490ba589f4e0e628b0ca26aa40577567ef4f8883a`.
The Python suite passed 38 tests. Durable logical subscriptions, multi-replica
coordination, external provider reconciliation, canonical workload mixes, cost
and hardware acceptance remain separate qualification.

The host transfers retry durations onto the PostgreSQL clock and also preserves
provider epoch reset/date lower bounds. Invalid retry metadata places the
registered resource in a retained indefinite hold (`i64::MAX`), requiring
operator reconciliation and an explicitly reviewed replacement contract; it
does not fall back to an automatic short retry. The queue wake-up hint is
capped by the original business deadline without shortening that resource hold.
