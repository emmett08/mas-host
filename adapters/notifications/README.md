# Trusted-local privacy response

`privacy_response` records intent before attempting a notification. Two Python
`Context.effect` operations make that ordering durable: the first retains the safe case
event and notification intent, and the second submits its fixed projection to an
operator-configured route. Each has its own admission and identity in the existing host
journal.

`CaseEvent` accepts bounded enumerated reasons, dispositions, exposure states,
categories and counts. The application must obtain those facts from a trusted handling
decision. A withheld output alone supports `handling-withheld` with zero reported
findings. Claiming PII detection or unauthorised recipient exposure requires additional
evidence about the source or delivery. Routes follow fixed rules: coverage failures use
`service-operations`, credentials or suspected exposure use `security-response`, and
other review events use `data-owner`.

Case identity derives from the original host invocation. Notification action
identity additionally binds the event version and symbolic route. The host's
random journal nonce separates invocation identities across execution namespaces.
The caller cannot substitute an email address or request identifier for these
identities. Both the case intent and notification descriptors bind the sink's
configuration digest. A changed destination/contract therefore
conflicts under the retained effect identity. Bearer-secret rotation preserves
the target digest and remains subject to the host's current grant checks.

The callback rechecks the sink binding before any send. A changed local target produces
a known `not-sent` receipt. Fresh admissions and retained deliveries remain subject to
the configured host policy. These checks apply to host-mediated operations in the
trusted native-worker profile. Confining a worker's other credential or filesystem
access requires additional deployment controls.

## Adapters

`MemorySink` is a bounded deterministic sink with duplicate-body checking. Its
memory is ephemeral. Durable replay comes from the host's effect journal.

`HttpsIncidentSink` implements one HTTPS incident-endpoint submission. Its
explicit `idempotency-key-acceptance/1` contract requires the deployed endpoint
to deduplicate `Idempotency-Key`, reject changed content under the same key, and
guarantee that a 4xx refusal has accepted no incident. The endpoint owner must
qualify those promises. The adapter sends a fixed JSON template containing safe
case metadata. A case link awaits a separately authorised resolver.

The adapter verifies TLS and binds the exact host, authority, path, route,
timeout and delivery contract. It performs no redirect, proxy, fallback or
automatic retry. A single instance admits one concurrent request. Notification
JSON is capped at 2,048 bytes, response headers at 8,192 bytes and 32 fields, and
response bodies are discarded without being read. The configured deadline is
checked before HTTP transmission and while reading headers. DNS resolution can
outlast a socket timeout. The native guardian's original wall deadline remains
the hard process-lifetime bound.

| Observation | Retained submission result | Recipient delivery |
|---|---|---|
| Valid 2xx status | Accepted, even if later metadata is malformed | Unknown |
| Valid 429 status | Throttled/refused, bounded retry eligibility if available | Unknown |
| Other contract-covered 4xx status | Refused | Unknown |
| Connection/TLS/configuration failure before HTTP send | Not sent | Unknown |
| Lost response, server error, redirect, 408 or 425 | Effect remains uncertain | Unknown |

The host retains known acceptance or refusal independently of response metadata.
An uncertain effect requires evidence-backed operator reconciliation under its
original identity. The adapter exposes no automatic retry operation. A throttle
receipt records eligibility and remains a refusal on replay.

## Read a retained case

`case_status(intent_view, notification_view)` projects a case from two
`mas-governed-effect-view/1` records obtained through the host's authenticated
`GetRetainedEffect` operation. Query the original task's `privacy-case-intent`
and `privacy-notification` keys. The resolver derives the journal, root actor
and invocation from task ownership and checks current `resolve` authority.
The caller supplies neither a filesystem path nor another actor's identity.

The consumer requires an available completed case intent and exact agreement on
namespace, actor and parent invocation. Available notification descriptors must also
match the safe event and destination binding. The case and action IDs are recomputed
from the original invocation. The result preserves acceptance, refusal, throttling,
known non-transmission and uncertainty. Recipient delivery remains unknown in every
case. An inaccessible notification body preserves the host's safe execution metadata but
reports an unknown submission and unavailable binding. An authorised `not-recorded` view
also means unknown. A failed lookup leaves access or availability unresolved and must
remain a separate outcome.

This pure function validates supplied views. Authentication and authorised reads belong
to the host boundary that obtains those views. The example obtains both records from the
host and re-reads the immutable intent after the notification lookup to check current
access again. Those reads are separate snapshots. An atomic case history or continuing
access after the final response requires additional guarantees. It starts no
notification, retry, acknowledgement or adjudication operation.

## Validation and remaining work

Run `make notification-test` for contract, destination substitution, malformed
metadata, deadline, capacity and replay tests. The actual-host scenarios in
`examples/privacy-response/verify.py` exercise the pinned native application,
host inspection gate, case intent, notification, restart and revocation.

The trusted-local N3 response slice now includes a read-only case projection over the
first governed effect resolver. Completing N3 requires authorised original assessments
for adjudication and a complete case history. Durable queues and suppression must extend
across invocations, with timed escalation and provider reconciliation qualified
separately. The synthetic example demonstrates one event per original request. A new
request does not automatically deduplicate a repeated real-world incident. Deployment
privacy, model quality and endpoint delivery qualification remain separate gates.
