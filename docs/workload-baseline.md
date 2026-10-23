# Workload baseline

Use `tests/performance/workloads.json` to select one of seven provisional workload
families from the Nehemiah scaling plan dated 22 October 2026. The manifest records
qualification conditions and missing evidence. Before comparing capacity, fix the
reference hardware, cost ceilings and investigation-to-assignment mapping, which remain
unknown.

The first executable slice measures existing trusted local execution and
payload-free CLI observation. It reuses `tests/fixtures/rust-invoice` and the
maintained host builder. No provider, Kubernetes or GitHub calls are made.
Existing local concurrency limits and public schemas remain unchanged.

```sh
python3 -m unittest discover -s tests/performance -v
python3 tests/performance/baseline.py generate --count 100 --distribution skewed-80-percent --output /tmp/p0-fixtures.json
cargo build --locked -p mas-host
python3 tests/performance/baseline.py smoke --host target/debug/mas-host --count 3 --polls 3 --output /tmp/p0-baseline.json
```

The generator supplies stable original request IDs, synthetic tenant/cluster and
permission labels, hot objects and exact encoded input sizes. Eight of each ten skewed
requests address the hot tenant. Labels describe synthetic demand. Authority and
multi-tenant isolation require separate enforcement. Fixture generation records demand
only, with executed work measured by the host smoke. Request count, input bytes and
smoke duration are bounded, including a 64 MiB aggregate fixture-input ceiling. The
smoke is sequential and unpaced. Exercising the L scenario requires a later run with its
declared thirty-second occupancy and offered rate.

Each smoke request runs through the actual native host. The runner retains separate
admission and execution knowledge, original identity, input size and CLI duration. The
returned run ID and invoice result are checked against the original request. A
contradictory or empty post-completion root projection fails the smoke. Application
assessment remains separate from task completion. Classify each request using the
retained evidence. A successful task establishes admission, start and completion. A
retained root rejection establishes non-admission, while a root Request in protocol
observation establishes admission. A failed CLI or a prepared checkpoint without that
evidence leaves admission unknown. A payload-free snapshot cannot distinguish a
retryable refusal from every other prepared history, so this slice does not report a
definite deferral. Future shared-service adapters must supply typed deferral evidence
before that count can increase.

Completion, failure and unknown outcome remain separate. `started` counts known host
starts, while `start_unknown` preserves missing transition evidence. Polls can miss
intermediate states. Keep per-request records beside the totals so duplicate identities
and lost work remain detectable even when aggregate counts agree. The generated IDs
remain in every native invocation, observation sample and report row.

Observation samples follow completion and compare payload-free committed state across
repeated polls, ignoring only the observation timestamp. This checks whether the
exercised observer path changes its projected state. Establishing that every database
byte remains unchanged would require a broader check. Protocol and journal snapshots
retain their separate consistency boundaries. Latencies include CLI process startup,
JSON parsing and local storage access. Nearest-rank p95/p99 values from these small
samples describe this smoke. Qualifying V's API latency or capacity targets requires the
declared sustained workload and reference deployment.

Reports bind the workload, executable, Cargo lock and workspace manifest by SHA-256,
bind the harness source separately, and record the checkout revision, tracked diff,
compiler availability, Python version and basic hardware. Storage and database versions,
reference cost ceiling, provider token counts and billing remain unknown where
unmeasured. The runner uses a temporary state directory and synthetic integer invoice
inputs. No external effect is part of this fixture. Reports retain safe metadata rather
than command output or application payloads. Reports flag an incomplete source-tree
binding whenever untracked files remain. Stage intended changes before retaining an
implementation baseline. The executable hash identifies the actual binary. Its source
revision remains unknown until separate build provenance binds it to source. The
checkout revision describes the harness and fixture checkout. The command runner
captures compiler/CLI output in memory and assumes trusted processes produce bounded
diagnostics. Hostile-output confinement and a full process-memory ceiling require
separate controls and measurement.

All V/L/S/A/A+/E/E+ qualification rows remain `NOT_RUN`, including after a
successful smoke. P0 remains partial until actual PostgreSQL query/admission
contention, deterministic endpoint faults, paced occupancy and the reference
environment are measured. Later comparisons require warm-up, at least three
interleaved baseline/candidate runs and thirty-minute sustained qualification.
Recovery and restart qualification remain separately identified work.
