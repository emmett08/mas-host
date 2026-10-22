# Final integration validation

Runtime cut: host `0f2ab6f518ea0f00f85b76374bbb538a6985720e`, local
`d9ac1014a1f2428e3c20aa7d7ee17506a012f7ad`, identical tree
`ae877bf0ce764227756faa44c6eb58148e6cec60`. The host pins protocol
`e1257fb7125cdc971d94e38221c5185818bb577d`. Later CI/evidence-only changes do
not replace these identified runtime results.

| Check | Result |
| --- | --- |
| `make test` | PASS, 990.019 s; 375 Rust tests passed, zero failed; SDK, privacy, acquisition, capture, workload and deployment checks passed |
| Native acceptance, provider and paced targets | PASS, 68.843 s; actual native recovery, independent acquisition subscribers and two bounded local fault fixtures |
| Shared provider observation units | PASS, two tests; authority/worker clock offset, invalid-hint hold and strict fields |
| Full finite SPIN suite | 251 declared expectations passed; 961.747 s; model and runner hashes match this source |
| Protocol PostgreSQL CI | 35 backend tests passed at protocol e1257fb |
| Host PostgreSQL CI | Five native/A2A/HTTP cases, 11 queue decisions and a separate 1,002-candidate traversal passed at host 0f2ab6f |
| Consumer PR #382 | Published separate receipts: 288 distinct passes, 22 skips; two native cases use this host binary |

The three Rust ignores in the default suite are the native acceptance entry
(run separately above) and two subprocess entrypoints exercised by their parent
tests. Optional PostgreSQL tests run in their hosted job, not the local default
feature suite.

The initial native command failed because Go could not obtain VCS status in
this workspace. Its failed receipt is retained. The successful retry records
`GOFLAGS=-buildvcs=false`; native package/source verification remains active.

Command receipts retain source hashes before and after, commands, environment,
exit status and log hashes. Runtime source did not change during these checks.
`runtime-cut.json` records the observed binary identity and its limits. These
are trusted local build observations, not runtime attestation. Formal raw-output
hashes are retained; the CI formal job uploads full commands and traces.

Paced results remain LOCAL_EXPERIMENT_ONLY. They establish neither queued
service capacity nor V/L/S/A/A+/E/E+ acceptance. Detector quality, live provider
operation, managed confinement, autoscaling/failover, protected shared execution,
platform recovery after user revocation and external-copy/physical deletion
retain the boundaries in `docs/nehemiah-integration-status.md`.

Hosted checks:
- [Protocol PostgreSQL](https://github.com/emmett08/mas/actions/runs/39870744024/job/120144246242)
- [Host PostgreSQL](https://github.com/emmett08/mas-host/actions/runs/39871138916/job/120145575649)
- [Consumer integration](https://github.com/emmett08/catalog-sync-operator-monograph/pull/382)

The protocol security gate remains blocked by audit failures in its pinned
external reference-host workflow; its runtime dependency profile passed. No
security check is skipped by this integration.
