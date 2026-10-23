# Governed structured acquisition evidence

This receipt binds opt-in local/2 acquisition checks to the implementation revision
below. The host retains operator-selected source policies and inherited exposure in its
existing execution journal. An authorised reader returns the retained source projection
under those restrictions.

Source commit: `44b1fab0c50f96d36731faeaaa734fce51b5860a`

Source tree: `7a7732474f4517946b7773adcade7b667797335c`

Base: `c84feb406eff46e9c2857b5ad92a817ab4bc12ec` (PR #19, whose branch supplied this
increment's base).

The preceding behaviour-preserving reader extraction is
`5f9ff2eaeb130fa15ba9cecb1daf40652c1b112c` (tree `6f5f7acd4d7bd18a891e414fe09e234845a041d3`), the source
commit's direct parent. Its eight existing HTTP reader tests pass separately.

| Gate | Result |
|---|---|
| `make test` | 328 Rust and 146 Python tests passed, together with Go race/vet, C++ conformance, TypeScript check/build, formatting and strict Clippy |
| `make acceptance` | Native multi-language and recovery acceptance passed |
| `make provider-test` | Five Node tests passed |
| Default SPIN suite | All 224 checks passed: 32 complete zero-error searches and 192 expected replayed counterexamples |

The 40 new formal checks cover finite acquisition, exposure, pending/callback,
closure, inherited child-target, expiry and replay control. All 184 previous case
tuples and seven previous model bytes remain unchanged. No search limits were
relaxed. See [the model contract](../../formal/exposure-lineage.md).

All four final command receipts record identical SHA256 values for every
non-evidence source and reject source drift. The identified source commit,
preceding refactor and following evidence-only commit preserve that separation.
`validation.json`
binds source, plans, command logs, counts and artefact checksums. The independent
reviews record findings, repairs, exact source identities and focused-check
limitations. Focused exploratory passes overlap the authoritative final suite.

`formal-checks.tar.xz` retains the full default run plus focused final and prototype
model runs, each with its own hashes, commands, bounds and replayed trails. Prototype
results retain their historical diagnostic role. The final-source gates have their own
identified runs. Reproduce with the three Make targets above and `python3
formal/verify.py --out /tmp/acquisition-formal` using the recorded toolchain.
`validate-command.py` wraps a command and rejects source changes.

Two workspace runs remain separate diagnostics. The initial cut failed an existing
recovery test's immediate `unknown` assertion while the task was still `working` after a
blocking response deadline. Only `tests/a2a_governed.rs` changed for the next cut: the
test polls the SAME task for terminal uncertainty with a bounded deadline and preserves
all no-redispatch assertions. That second cut failed the new expiry fixture's
eight-second lease, which had included server startup. Startup consuming the lease is an
inference from the observed failure and fixture schedule. The fixture repair in
`tests/a2a_governed/retained_source.rs` starts the server first, then arms a
thirty-second lease while preserving original-expiry and no-redispatch assertions. The
final cut also adds the CI job environment `RUST_TEST_THREADS='2'`. Its existing
thirty-minute aggregate budget remains. The pre-profile source object is retained as an
intermediate identity with no gate claim. No runtime or formal code changed between
these cuts. Each failed source cut retains its remote tree, full source map, log and
receipt. Final acceptance comes from the complete gates over the repaired source cut.
The final source cut passes the complete gates above under the recorded command
environments.

The workspace and acceptance gates use the explicit two-thread Rust test harness
profile, also configured in CI. An earlier default-launch focused run failed six of
twenty-seven fixtures: five existing ten-second client socket reads and one elapsed-time
assertion. The receipt leaves the process environment and exact test thread count
unknown. The resource observations are consistent with contention, while isolating its
cause would require a controlled comparison. Its source map was captured only after
exit. The explicit two-thread focused rerun passed all twenty-seven tests in 141.34
seconds and has during/after sixteen-file source observations, rather than a complete
before-start cut. Both observations remain separate from the four authoritative final
gates. No default/high-parallel capacity, production throughput or hosted CI PASS is
claimed. Existing runtime, socket, detector and SPIN bounds remain unchanged.

The acquisition example runs through the actual loopback A2A service. It checks baseline
success, restricted acquisition, a permitted consuming peer, denial of the same constant
downstream actions, read/restart/replay identity and a baseline-only acquisition
control. Independent tests check actual 33+ ancestry, the 64-source bound, corruption,
pending crash recovery and existing read APIs. The source reader returns retained
projections. Discarded original bytes and component assessment custody require
separately authorised retention and access.

This is a trusted-local structured increment. Synthetic collector and neural responses
establish conformance for the exercised host paths. Live collector and detector quality
require separate evidence. Authority and exposure checks are snapshots with a
final-read-to-transmission interval. Larger/shared readership, dynamic credentials,
governed revision, exact assessment/source-byte custody, full N3
scheduling/reconciliation, E4 deletion/restore, E5 isolation and measured
deployment/quality/capacity remain open in the plans. No merge or deployment is
performed by this increment. See [usage and limits](../../docs/governed-acquisition.md).
