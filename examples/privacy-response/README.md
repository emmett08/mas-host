# Synthetic governed privacy response

This example runs the following composition through the real Rust host:

1. Authenticated ingress projects a synthetic request before task persistence.
   The host inspection gate invokes the pinned local fixture worker and the
   existing deterministic recognisers on the exact admitted sources.
2. A child operation completes with prohibited synthetic output. The host keeps
   the known completion and returns an explicit withheld result.
3. A trusted responder records a safe handling-review case and pending
   notification intent through one durable effect.
4. A separately admitted notification effect submits the fixed safe projection
   to the fixture sink. Restart reuses the same result and action identity.
5. The authenticated client reads the two retained effects through the host's
   governed resolver and validates their shared case, event and target binding.
   Reading after restart preserves the original action and does not resubmit it.

The fixture worker supplies synthetic complete neural responses and performs
no model inference. It runs through the same bounded offline worker and
exact-source validation path as the local adapter. Deterministic recognisers
still identify the seeded email. The response uses `handling-withheld` with zero
reported category findings because withholding can also follow schema, policy
or coverage failures. The example makes no model-quality claim.

Its standard-library fixture explicitly selects `/usr/bin/python3`, checks
Python 3.11 or later, and pins that interpreter's executable hash. To select
another fixture interpreter, use `prepare.py --fixture-python /absolute/path`.
Every detector launch still revalidates the approved executable. Native worker
generation and real model environments keep their own interpreter selection.
This matters for bounded tests: hashing a large embedded analysis interpreter
for every text component can consume the declared inspection deadline.
The synthetic inspector allows 60 seconds per boundary: retained-view reads
inspect the selected application payload again, and four conformance processes
may run concurrently. This fixture budget preserves every launch identity check
and the production inspector's independent configured limits. The client allows
150 seconds for a retained-effect request, covering the service's two bounded
inspection phases and response margin.

From a built checkout, run each conformance scenario:

```bash
python3 examples/privacy-response/verify.py --host target/debug/mas-host --scenario accepted
python3 examples/privacy-response/verify.py --host target/debug/mas-host --scenario throttled
python3 examples/privacy-response/verify.py --host target/debug/mas-host --scenario unknown
python3 examples/privacy-response/verify.py --host target/debug/mas-host --scenario revoke-before-notify
```

Each run prepares a private temporary application, builds its frozen Python
workers, starts the loopback authenticated A2A2 service, and checks the retained
journals. The first three scenarios restart the service and resubmit the exact
original request. They check one collection and one notification attempt,
immutable safe payloads and changed-original rejection. The unknown scenario
retains uncertainty and performs no second submission. The revocation scenario
changes current authority after case intent commits and before notification
admission, then verifies that no notification runs and the revoked principal
cannot read the retained case. The other scenarios reject a second principal's
lookup despite that principal having its own current rights, and require the
effect-read extension. No failed lookup becomes a not-sent receipt.

The read-only JSON-RPC method is `GetRetainedEffect` with parameters
`{"id":"<original A2A task ID>","key":"privacy-case-intent"}` or the
`privacy-notification` key. Its `A2A-Extensions` header contains both
`urn:mas:host-task-service:1` and `urn:mas:host-governed-effect-read:1`.
The server derives the exact invocation and actor from the owned task. Each
read rechecks current `resolve` authority, inspection and disclosure policy.
The client passes both returned views to `privacy_response.case_status`.

The two records are separate snapshots. The client re-reads the immutable intent
after the notification record and requires the same safe case. This supplies a
final current-access check without claiming an atomic history. Notification
payload withholding or an explicitly authorised `not-recorded` view remains
unknown; neither proves non-transmission or permits a retry. The client derives
no case from an inaccessible intent. Recipient delivery remains unknown even
when endpoint acceptance is known.

To inspect a prepared application, choose a new destination:

```bash
python3 examples/privacy-response/prepare.py --destination /tmp/nehemiah-response
target/debug/mas-host build --project /tmp/nehemiah-response
target/debug/mas-host serve-a2a \
  --bundle /tmp/nehemiah-response/.mas/build/bundle.json \
  --state /tmp/nehemiah-response/state \
  --config /tmp/nehemiah-response/gateway.json
```

The generated gateway listens on `127.0.0.1:8787` and uses an explicitly synthetic
local token. It selects an approved structured schema and explicit inline
retention for the restricted metadata. The verification script shows the exact
A2A request and extension headers. It contacts no external notification service.

For production adapter code and its endpoint contract, see
[`adapters/notifications`](../../adapters/notifications/README.md). Replacing the
fixture requires an approved native implementation and target-binding schema,
current destination rights, scoped credentials and endpoint qualification.
Automatic retry, escalation, external case creation, adjudication and a complete
case-history/link resolver remain subsequent N3 work.
