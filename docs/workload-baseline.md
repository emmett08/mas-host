# Workload baseline

`tests/performance/workloads.json` is the canonical provisional P0 workload
manifest for the Nehemiah scaling plan dated 22 October 2026. It records the
seven proposed workload families, qualification conditions and missing evidence.
Reference hardware, cost ceilings and the investigation-to-assignment mapping
must be fixed before capacity comparison. Their values remain explicit unknowns.

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
permission labels, hot objects and exact encoded input sizes. Eight of each ten
skewed requests address the hot tenant. Labels describe demand and confer no
authority. They do not establish multi-tenant isolation. Fixture generation
records no executed work. Request count, input bytes and smoke duration are
bounded, including a 64 MiB aggregate fixture-input ceiling. The smoke is sequential and unpaced. It exercises neither the L
scenario's thirty-second occupancy nor its offered rate.

Each smoke request runs through the actual native host. The runner retains
separate admission and execution knowledge, original identity, input size and
CLI duration. The returned run ID and invoice result are checked against the
original request. A contradictory or empty post-completion root projection
fails the smoke. Application assessment remains separate from task completion.
A successful task supplies known admission, start and completion.
A retained root rejection establishes non-admission. A root Request in the
protocol observation supplies admission evidence. A failed CLI or a prepared
checkpoint without that evidence retains unknown admission. A payload-free
snapshot cannot distinguish a retryable refusal from every other prepared
history, so this slice does not report a definite deferral. Future shared-service
adapters must supply typed deferral evidence before that count can increase.

Completion, failure and unknown outcome remain separate. `started` counts known
host starts, while `start_unknown` preserves missing transition evidence. Polls
can miss intermediate states. Totals supplement per-request records because equal
totals can conceal duplicates and lost work. The generated IDs remain in every
native invocation, observation sample and report row.

Observation samples follow completion and compare payload-free committed state
across repeated polls, ignoring only the observation timestamp. This checks the
exercised observer path for changes to its projected state. It does not establish
absence of every possible database mutation. Protocol and journal snapshots
retain their separate consistency boundaries. Latencies include CLI process
startup, JSON parsing and local storage access. Tiny nearest-rank p95/p99 samples
describe the smoke only. They cannot qualify V's API latency or capacity targets.

Reports bind the workload, executable, Cargo lock and workspace manifest by
SHA-256, bind the harness source separately, and record the checkout revision,
tracked diff, compiler availability,
Python version and basic hardware. Storage and database versions, reference
cost ceiling, provider token counts and billing remain unknown where unmeasured.
The runner uses a temporary state directory and synthetic integer invoice inputs.
No external effect is part of this fixture. Reports retain safe metadata rather
than command output or application payloads. Reports flag an incomplete source-tree
binding whenever untracked files remain. Stage intended changes before retaining
an implementation baseline. The executable hash identifies the actual binary.
Its source revision remains unknown until separate build provenance binds it
to source. The checkout revision describes the harness and fixture checkout.
The command runner captures compiler/CLI output in
memory and assumes these trusted processes produce bounded diagnostics. It
does not qualify hostile-output confinement or a full process-memory ceiling.

All V/L/S/A/A+/E/E+ qualification rows remain `NOT_RUN`, including after a
successful smoke. P0 remains partial until actual PostgreSQL query/admission
contention, deterministic endpoint faults, paced occupancy and the reference
environment are measured. Later comparisons require warm-up, at least three
interleaved baseline/candidate runs and thirty-minute sustained qualification.
Recovery and restart qualification remain separately identified work.

## Paced execution and independent history checks

`tests/performance/paced.py` adds bounded offered demand, concurrent native host
execution and a synthetic source that remains occupied for a declared interval.
For example, this short fault experiment offers four original requests at 20/s,
uses at most two CLI processes, and loses every second effect's completion:

```sh
python3 tests/performance/paced.py --host target/debug/mas-host \
  --count 4 --rate 20 --occupancy-ms 100 --concurrency 2 \
  --fault-every 2 --fault-mode lost-completion --output /tmp/p0-paced-loss.json
```

`make workload-acceptance` builds the host, runs the workload unit tests and
executes both short fault scenarios. Its JSON reports remain under
`target/workload-acceptance/`. These checks exercise fixture conformance and
replay. Sustained deployment qualification uses a separately selected workload.

The native fixture records its actual admitted effect identity in a separate,
synced source ledger before returning through `ctx.effect`. A lost-completion
fault exits the worker after the source has completed and before the host sees
its result. The runner resumes the same original run and verifies that the
source ledger remains unchanged. A second deterministic fault,
`refuse-before-effect`, returns an application error before effect admission.
Both faults exercise local host boundaries without a provider call.

`history.py` compares offered identities, retained host results and source
histories. It detects duplicate source execution, missing result identities and
effect-identity substitution. Missing source coverage produces `INCONCLUSIVE`.
A completed source operation with an unknown host result remains a distinct
fact. Counterexample tests deliberately duplicate one completion and remove
another while preserving totals, remove source events, swap effect identities
and simulate a replay that repeats the source operation.

The runner snapshots and hashes its host executable before the experiment, so
a concurrent build cannot change the binary used by later requests. The report
records the offered schedule, harness start and queue delays, source
occupancy, native CLI latency, retained state bytes, replay outcomes and exact
experiment/fixture hashes. Harness queue time describes waiting for a local
test thread. Host admission latency and eligible service queue time remain
unknown. The source's millisecond wall-clock intervals provide a limited
occupancy observation. They measure neither CPU time nor shared scheduler
assignments. Concurrent invocations use distinct run identities and local
owners. This experiment does not exercise a shared A2A scheduler or establish
multi-tenant isolation.

Bounds are explicit: 10,000 offered requests, sixteen active CLI processes,
30,000 ms maximum source occupancy, sixty seconds per CLI attempt and 64 KiB
per source ledger. Parameters reject a scheduled or minimum occupancy interval
longer than one hour. Reports retain synthetic metadata and source events. The
trusted fixture's output and process tree retain the existing host supervision
contract. External database, provider and reference deployment qualification
remains separate.

To exercise the L demand and occupancy parameters, use `--rate 0.25
--occupancy-ms 30000 --concurrency 16`; a thirty-minute offered-demand interval
requires at least 451 requests. A qualification programme still needs warm-up,
three interleaved baseline/candidate runs, the identified reference environment,
the full L compatibility checks and the declared hardware/cost envelope.
The runner therefore keeps every canonical scenario `NOT_RUN`; its own
`LOCAL_EXPERIMENT_ONLY` status records the narrower observed result.
