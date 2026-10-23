# Governed retained-effect reader evidence

This receipt records the first E3 retained-effect reader and its N3 case-status
consumer. Current resolve authority and configured inspection govern each read. The
privacy adapter reconstructs status through the actual HTTP service after restart, while
execution remains owned by the existing journals. Recipient delivery remains unknown.

Source commit: `b2db88e038db00cc33c1df88d993290b83fb24df`

Source tree: `25374cf7ae0d992287320f6385e9cf0115895e2e`

Merged main parent: `db77a670443776fc861875dd3a7328f5c8b3488f`

## Accepted checks

| Check | Result |
|---|---|
| `make test` | 295 Rust tests and 146 Python tests passed, plus Go race/vet, C++ conformance, TypeScript build/check, formatting and strict Clippy |
| `make acceptance` | One native multi-language/recovery acceptance test passed |
| `make provider-test` | Five Node tests passed |
| Formal coverage with explicit resource retries | 184 case configurations completed: 28 complete zero-error searches and 156 expected, replayed counterexamples |

Two full default formal invocations each hit one 30-second compiler timeout on different
unchanged witnesses. Neither default invocation passed. Their successful checks cover
all 184 configurations, and an explicit `--model delegation` retry passed the affected
model. `formal-summary.json` indexes coverage by contributing run for each case. The
original runner receipts retain their individual statuses. All original statuses,
commands, trails and limits are retained in the archive.

The 30 new formal checks cover the retained-effect reader. The default run also retains
the prior 154 checks. The model has finite identity/state classes and bounded callbacks.
Its assumptions and correspondence limits are explicit in
[`formal/retained-effect.md`](../../formal/retained-effect.md).

The workspace gate includes all four privacy-response scenarios, including actual
retained reads before and after service restart. Independent core, HTTP and consumer
adversarial tests are included in that gate. Their focused logs and reviews are retained
separately, and are not counted twice. Three normally ignored Rust entries are explained
by their test names in `validation.json`: the native acceptance test runs separately,
and the other two are private child entry points exercised by their parent tests.

All three command receipts record identical SHA256 values for all 276 non-evidence
files. They report the local base HEAD during execution. The separate source tree above
identifies the frozen files that were tested. `validation.json` binds those receipts,
independent reviewed source hashes and model hashes to that source commit. The following
evidence commit adds only this directory.

## Reproduction and artefacts

Run `make test`, `make acceptance`, `make provider-test`, then the default `python3
formal/verify.py --out /tmp/governed-reader-formal` with the required Rust, Go, Node,
C++, Python and SPIN toolchains. The local versions are retained in `toolchain.json`.
GitHub CI uses its declared toolchain, including Node 22 where the local run used Node
24. The command receipt helper accepts a repository, label, log path, result path and
command after `--`. It rejects source drift.

`formal-checks.tar.xz` contains the complete command ledger, model and runner hashes,
and replayed trails. `formal-summary.json` provides the coverage index. The raw run
summaries retain both failed command statuses. `python3 compose-formal-evidence.py
--repo /path/to/mas-host --attempt-root /path/to/extracted` reproduces the index from
the three extracted attempt directories and verifies identical sources, case
configurations and limits. The independent review receipts retain defects found and
fixed, source correspondence and the limits of their conclusions. Their parent-gate
pending labels describe their creation time. `validation.json` records the completed
gates over the frozen sources and the compiler-only resource retries.

This increment uses deterministic scanner and loopback notification fixtures. Structured
namespaces may omit an inspector under their existing contract. The inspected fixture
exercises the configured gate and verifies withholding when inspection is unavailable.
No real-model reader smoke, live provider call or external notification was performed.
Earlier runtime/quality receipts remain historical. The pure Python projection validates
supplied views but does not authenticate arbitrary JSON.

## Remaining plan work

This receipt completes the first authorised retained-effect reader and case-status
consumer slice for the source revision above. At this checkpoint, E3 still required
general source ancestry and exposure inheritance, followed by shared subscribers and
qualification with larger reader/ancestor sets. Exact assessment/source access before
adjudication, durable scheduling, retries, escalation and provider reconciliation remain
N3 work. Broader N2 paths and quality, E4/E5 and deployment gates remain open. See
[`docs/governed-evidence-reader.md`](../../docs/governed-evidence-reader.md) and
[`docs/nehemiah-implementation.md`](../../docs/nehemiah-implementation.md).
