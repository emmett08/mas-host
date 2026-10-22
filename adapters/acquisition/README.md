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
Timeouts or 5xx responses to mutations preserve execution uncertainty, including
after the original deadline. An accepted mutation still needs reconciliation.

Provider semantics follow the [GitHub rate-limit contract](https://docs.github.com/en/rest/using-the-rest-api/rate-limits-for-the-rest-api)
and [Kubernetes API list/watch contract](https://kubernetes.io/docs/reference/using-api/api-concepts/).

## Validation

```sh
python3 -m unittest discover -s adapters/acquisition/tests -v
```

Tests exercise immutable host identities, lost submission/publication replies,
private checkpoint restore, changed scope/epochs/input, independently denied
subscribers, cancellation during delivery, stale-source refusal and actual
loopback HTTP error/throttle/redirect/oversize/slow-response handling. Fake RPC
tests check adapter protocol control flow; they do not qualify a shared host or
deployment. Provider HTTP fixtures exercise actual known refusals and uncertain
mutations. Actual governed-host coalescing, multi-replica coordination, canonical
workload mixes, cost and hardware acceptance remain separate qualification.
