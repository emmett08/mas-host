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
admission, then verifies that no notification runs.

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
Automatic retry, escalation, external case creation and a governed case-link
resolver remain subsequent N3 work.
