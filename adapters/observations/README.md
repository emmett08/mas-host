# Bounded ordinary task-status capture

`capture.py` consumes the maintained authenticated `GetTaskStatus` operation.
It calls no worker, source collector, retained payload reader or mutation. Only
an ordinary local service advertises the host-generated
`captureProfile: ordinary-trusted-local-metadata/1` marker. The consumer requires
that marker and the exact local status schema. Governed local and shared
services cannot activate this profile. Their status delivery grants supply no
permission to persist or export protected traces. Protected triggered capture
requires a separate current full-exposure retention/export gate and is unsupported.

The approved ordinary projection contains committed service state, revision,
timestamps, cancel intent and settlement. Task, agent and bundle identities
become HMAC tokens scoped to the registered host authority and private identity
key epoch. Tokens preserve correlation within that scope; they are not an
anonymity claim. Source content, application input/output, bearer values,
diagnostics and arbitrary response fields never enter the capture spool.

Register at most sixteen exact task/agent/bundle triples in a private JSON file:

```json
[{"id":"a2a-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
  "agent":"registered-agent", "bundleDigest":"dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"}]
```

Use a dedicated identity key of 32–64 bytes and a current bearer file, each mode
`0600`. The registration file is also private. Provision a private `0700` spool
directory. These paths belong to the trusted local operator; no worker receives
the key, bearer or spool. The bearer file is reread for every bounded request.

```sh
python3 adapters/observations/capture.py \
  --endpoint https://host.example/a2a --authority registered-host-epoch-1 \
  --quota-scope registered-host/status \
  --bearer-file /private/current-bearer --identity-key-file /private/capture.key \
  --registrations /private/status-tasks.json --spool /private/capture/status.sqlite \
  --duration-ms 30000
```

The executable polls for at most one hour. HTTP connections use verified TLS,
current bearer authentication, A2A version/extension negotiation, no redirects
or proxies, a bounded response and deadline, and existing provider deferral
rules. Loopback HTTP requires the explicit local-fixture flag. The source
service remains the owner of status permission and performs no provider reads
when viewers refresh status. Polling status is not collection of external facts.

The `Monitor` API accepts explicit `Limits`: pre/post windows each at most sixty
seconds, a 64 KiB in-memory prebuffer, at most 128 buffered samples, at most
sixteen retained captures with at most 64 KiB and 256 samples each, and expiry
within one day. Poll intervals are at least 100 ms. `unknown` status and skipped
materialised revisions trigger capture automatically; an operator can trigger
an already classified registered task. The trigger selects only that task's
metadata. Repeated same-task/reason triggers increment occurrence count without
extending the original post-window. Capacity refusals, buffer eviction, lost
samples, revision gaps and byte overflow increment bounded persistent counters.
The capture never grows to a full event stream during a trigger storm.

SQLite rows contain only this classified ordinary projection. One advisory
owner lock protects the bounded spool. Changed host authority, registration,
key epoch or limits requires a new spool rather than relabelling retained rows.
Row MACs reject changed bodies. Restart closes unfinished post-windows with an
explicit gap. Expiry gates every public read and deletes expired rows before
each operation. `secure_delete` clears deleted SQLite cells; the API claims no
physical erasure, backup expiry or autonomous cleanup while the process is
stopped. The operator must expire copies/backups independently and maintain a
correct local clock. The next start purges expired local rows before serving
them. Protected payload retention and restore qualification remain separate.

`check_capture` detects conflicting/regressed revisions and preserves capture
gaps. A healthy snapshot or completed host settlement cannot establish absence
of lost work, duplicate external execution or competing ownership. Capture-only
checks are therefore `INCONCLUSIVE` unless a revision fault establishes `FAIL`.
The existing independent checker accepts `check_history(...,
status_capture=capture)`: capture faults/gaps affect the result, while independent
source histories remain necessary for source-execution claims. It never invents
source execution events from a status observation.
Structural checking of a supplied JSON object provides no authentication; use
the `Monitor.read` path for the private, MAC-validated local spool and retain the
experiment's independent source provenance separately.

Run focused checks with:

```sh
python3 -m unittest discover -s adapters/observations/tests -v
python3 -m unittest discover -s tests/performance -p test_history.py -v
```

The HTTP fixture tests actual client authentication and fixed-schema handling.
They do not qualify a Kubernetes runtime, external source history, clock service
or storage backup process. Host integration tests separately verify that the
ordinary marker is present and protected status omits it.
