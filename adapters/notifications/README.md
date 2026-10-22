# Trusted-local privacy response

`privacy_response` adds two application operations through the existing Python
`Context.effect` API. The first retains a safe case event and notification
intent in the host journal. The second submits its fixed projection to one
operator-configured route. Each operation has its own host admission and durable
identity. This package introduces no case database or second effect engine.

`CaseEvent` accepts bounded enumerated reasons, dispositions, exposure states,
categories and counts. The application must obtain those facts from a trusted
handling decision. A withheld output alone supports `handling-withheld` with
zero reported findings. It supplies no evidence that PII was detected or that an
unauthorised recipient received it. Routes follow fixed rules: coverage failures
use `service-operations`, credentials or suspected exposure use
`security-response`, and other review events use `data-owner`.

Case identity derives from the original host invocation. Notification action
identity additionally binds the event version and symbolic route. The host's
random journal nonce separates invocation identities across execution namespaces.
The caller cannot substitute an email address or request identifier for these
identities. Both the case intent and notification descriptors bind the sink's
configuration digest. A changed destination/contract therefore
conflicts under the retained effect identity. Bearer-secret rotation preserves
the target digest and remains subject to the host's current grant checks.

The callback rechecks the sink binding before any send. A changed local target
produces a known `not-sent` receipt. Fresh admissions and retained deliveries
remain subject to the configured host policy. These checks operate within the
trusted native-worker profile. They supply no confinement against a worker that
uses its credentials or filesystem access outside the declared operation.

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

## Validation and remaining work

Run `make notification-test` for contract, destination substitution, malformed
metadata, deadline, capacity and replay tests. The actual-host scenarios in
`examples/privacy-response/verify.py` exercise the pinned native application,
host inspection gate, case intent, notification, restart and revocation.

This is the first trusted-local N3 response slice. Full N3 still requires a
governed case resolver/history, adjudication against original assessments,
durable cross-invocation queues and suppression, timed escalation, and qualified
provider reconciliation. The synthetic example demonstrates one event per
original request. A new request does not automatically deduplicate a repeated
real-world incident. Deployment privacy, model quality and endpoint delivery
qualification remain separate gates.
