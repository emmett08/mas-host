# Governed assessment review and notification lifecycle

This increment extends the existing trusted-local structured host. It retains
exact component assessments when explicitly configured, records separately
authorised reviews and runs a bounded notification lifecycle through the
existing effect journal. Domain-specific model quality and production endpoint
behaviour require deployment evidence.

## Contextual inspection and assessment custody

`mas-governed-privacy-inspector/1` retains its existing decoded string-component
contract. Version `2` additionally scans the exact canonical JSON representation
of each original and projected value. Numeric literals and relationships across
fields therefore reach the classifier in the same bounded source. The component
pass continues to inspect decoded strings, so JSON escapes cannot hide an email
or credential from that pass. Every source needs complete coverage within the
shared deadline. The original 16,384-byte boundary and 128-source bounds remain.

The classifier's ability to recognise cross-field meaning or repeated mentions
requires the evaluation described in [privacy qualification](privacy-qualification.md).
The context source contains the content assembled at this boundary. An earlier
conversation outside that content needs explicit, authorised assembly. Neither
version automatically authorises unstructured release.

Version `3` adds explicit assessment custody. Besides the version string, its
configuration fields match versions `1` and `2`. It requires a fresh namespace.
For every unique inspected source, the host retains a keyed source commitment,
its original/projection position, validated UTF-8 category/span metadata and a
keyed assessment binding inside the existing inspection receipt. Values stay
outside this metadata. Repeated identical source text shares its assessment.
Offsets in context assessments refer to the canonical JSON source. Offsets in
component assessments refer to the unchanged decoded component.

Every applicable policy must grant the following correlated tuple before this
metadata enters the existing journal:

```json
{
  "operation": "retain-assessment",
  "recipient": "host-assessment-journal",
  "destination": "host-journal"
}
```

Supply the remaining existing principal, actor, purpose, location,
classification, byte bound and retention-obligation fields in each policy rule.
This permission is separate from detector processing and ordinary retention.
Receipt size counts toward the existing retention admission. A large assessment
can therefore prevent admission even when the source fits its input bound.

`GetRetainedAssessment({id,key})` selects an owned task's exact root effect.
Negotiate both `urn:mas:host-task-service:1` and
`urn:mas:host-governed-assessment-review:1`. Current ordinary resolve permission,
inspection, complete inherited exposure and the separate `resolve-assessment`
permission to the authenticated principal at destination `client` are required.
The same separate permission protects detailed receipts returned by
`GetRetainedEffect`. `GetRetainedSource` continues to return the retained output
projection. A detailed assessment contains no reconstructed original source.

`AdjudicateAssessment` accepts:

```json
{
  "id": "<owned task ID>",
  "key": "<exact root effect key>",
  "review": {
    "review_key": "review-1",
    "assessment_binding": "<binding from this effect's component assessment>",
    "verdict": "inconclusive",
    "reason": "insufficient-evidence"
  }
}
```

Verdicts are `false-positive`, `confirmed-finding` and `inconclusive`. Reasons are
`authorised-source-review`, `domain-context` and `insufficient-evidence`.
An inconclusive verdict requires the last reason. No arbitrary narrative is
accepted. The `adjudicate-assessment` operation requires recipient
`host-assessment-journal` and destination `host-journal`, alongside current
assessment-read permissions and complete inherited exposure.

The host appends an immutable HMAC-bound review to `execution.sqlite`, binding
the namespace, invocation, effect, exact assessment/source and reviewer scope.
The same review key replays the original record. A changed verdict under that
key conflicts. At most 128 reviews fit one namespace. Reviews preserve the
original detector result, source commitments and completion. Every release
continues to need a fresh policy decision.

Custody currently covers assessments attached to successfully admitted
projections. A failed or prohibited projection is withheld through its existing
handling path. Its discarded original bytes and failed assessment details are
unavailable through this reader. Reviewing those bytes needs an explicitly
approved quarantine and custody extension. The implementation makes that
boundary observable instead of fabricating source access.

## Durable queued notification

`privacy_response.respond_queued` uses existing `Context.effect` operations for:

1. One safe case/queue intent, with a retained creation time and expiry.
2. A retained timer before each of at most eight attempts.
3. Separately admitted submission attempts under one fixed notification action.
4. A separately admitted read-only provider lookup after an ambiguous send.

Register `mas-privacy-queue-intent/1`, `mas-privacy-queue-timer/1`,
`mas-privacy-queue-submit/1` and `mas-privacy-queue-reconcile/1` with the closed
schemas shown in [the executable example](../examples/privacy-lifecycle/prepare.py).
The fixed policy and destination identity form part of every descriptor.
Current host authority applies to every fresh effect, including retries.

Known throttling and proved-unsent attempts can retry after the larger of the
configured exponential backoff and provider retry delay. An ambiguous send is
retained as externally unknown. Another attempt requires a qualified lookup's
explicit `not-accepted` observation. A failed, withheld or unknown lookup stops
resubmission. Every attempt uses the original endpoint idempotency key. A
completed accepted attempt remains accepted when replay occurs after queue
expiry. Expiry can prevent a fresh send and preserves prior attempt evidence.

`HttpsReconciledIncidentSink` requires the explicit
`idempotency-key-authoritative-lookup/1` contract. Its same-origin GET lookup
maps `200`/`204` to accepted and `404` to authoritative absence for the exact
original action. Other statuses preserve uncertainty. The endpoint must qualify
those meanings and POST deduplication. Lookup never claims recipient delivery.

## Acknowledgement and escalation

`respond_until_acknowledged` composes initial notification, a retained
acknowledgement deadline, at most 33 separately admitted observations and one
separately admitted escalation. `AcknowledgementPolicy` bounds elapsed waiting
to 900 seconds and the polling schedule to 32 intervals.

`mas-privacy-acknowledgement-timer/1` and
`mas-privacy-acknowledgement-read/1` require their own registered schemas and
current effect permissions. An acknowledgement lookup returns `acknowledged`,
`pending` or `unknown`. Endpoint acceptance alone supplies none of these facts.
The optional `idempotency-key-authoritative-lookup-and-ack/1` HTTPS contract
additionally requires a configured same-origin acknowledgement path whose
`200`/`204` confirms recipient acknowledgement and `404` means pending.

An unacknowledged case at its retained deadline can create the fixed
`privacy-handling-escalation/1` template. It keeps the original case and uses a
distinct action identity. Its destination remains the configured approved route.
Current authority can prevent escalation after the initial send. An escalation
never changes whether the original notification was accepted or acknowledged.

## Execution and limits

Run the actual-host examples:

```sh
python3 examples/privacy-lifecycle/verify.py --host target/debug/mas-host --scenario retry-once
python3 examples/privacy-lifecycle/verify.py --host target/debug/mas-host --scenario reconcile-accepted
python3 examples/privacy-lifecycle/verify.py --host target/debug/mas-host --scenario reconcile-absent
python3 examples/privacy-lifecycle/verify.py --host target/debug/mas-host --scenario reconcile-unknown
python3 examples/privacy-lifecycle/verify.py --host target/debug/mas-host --scenario acknowledged
python3 examples/privacy-lifecycle/verify.py --host target/debug/mas-host --scenario escalate
```

These conformance tests use synthetic notification/lookup contracts and the
actual governed Rust host. They disable the example's optional inspector to
isolate N3 timer/replay behaviour from per-component detector launch latency.
The additional `--with-inspector` regression uses inspector version `3` and
composes actual supervised inspection, withheld known completion, queue/retry,
assessment custody, authorised read, adjudication/replay/conflict and service
restart. It checks that the seeded private value stays outside persisted state.
This fixture expands the queue lifetime to 1,800 seconds and its invocation
budget to account for the current per-source supervised launches. Those fixture
budgets establish no production latency objective. Existing N2 tests also
exercise inspector faults independently.

Durable timers currently occupy their trusted native worker. They resume through
the existing host invocation recovery contract. No independent scheduling
service, external queue or execution authority is introduced. An interrupted
admitted callback keeps the existing uncertainty requirement. Shared scheduling,
tenant fairness, protected consumer integration, source-byte quarantine,
production endpoint qualification and measured 60-second/15-minute objectives
retain their separate acceptance gates.
