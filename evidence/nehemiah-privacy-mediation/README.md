# Nehemiah detector mediation and response evidence

This increment mediates governed execution with local inspection over exact source text.
A trusted native adapter then records safe notification intent before submitting a
separately admitted notification. Two finite Promela models check the corresponding host
and response control rules. The source commit is
`48f72886e59a5971020f668813381393fce1d9e4`, tree
`d2dbe7826c041ef011a645aed7f354abaf37b469`, stacked on PR #17 at
`2f334a8226317adaed6e19076149765a4794757f`. Evidence is a separate commit.

## Plan requirement and completed checks

Section 6 of the enterprise scaling plan reviewed for this increment requires small
finite models for changed ownership/admission transitions, controlled implementation
schedules, independent history checking and deliberate guard-removal controls. The three
reviewed plan snapshot hashes are retained in `validation.json`.

- `formal-summary.json`: all **154 checks passed**, with no formal source drift.
  There are 24 exhaustive zero-error safety/LTL searches and 130 expected
  mutation/reachability counterexamples, each replayed. The two new models add
  68 checks to the existing 86. Searches use SPIN 6.5.1, depth 100,000 and 512 MiB
  with/without partial-order reduction, without fairness or approximate hashing.
- `formal-checks.tar.xz`: complete summary, commands, generated counterexample
  configurations, verifier sources and trails. SHA256:
  `949c5c9c71945c7c516ff6244abfc67cca9fb01c8773119148b5b4b5a85bdb5c`.
- `workspace-run.*`, `acceptance-run.*`, `provider-run.*`: accepted final command
  receipts and logs. All contain the same complete non-evidence source hashes
  and report no source drift. The recorded local HEAD is the source commit's
  parent. The source tree and exact file hashes bind the tested worktree.
  The workspace run passed 268 Rust tests and 130 Python tests, plus Go race,
  C++, TypeScript and static checks. Its three ignored entries are the native
  acceptance test (passed separately) and two private subprocess entrypoints
  exercised by their parent tests.
- `independent-review.json`: reviewed implementation/model hashes, concrete
  repaired defects, nine independent Rust tests and six Python boundary tests.
  Those tests overlap the full suite and must not be added to its totals.
- `response-review.json` and `response-focused.log`: four real CLI/A2A native
  scenarios passed: accepted submission, throttling, lost acknowledgement and
  revocation after intent. They verify safe persistence and restart without a
  second effect. The scanner and endpoint are local synthetic fixtures, so the scenarios send no
external notification. All 23 adapter tests also pass in the full suite.
- `actual-neural-smoke.json`: the real pinned local Privacy Filter completed a
  minimal governed `SendMessage`, `GetTask` and identical-message replay, with
  one handler execution in 138.779 seconds. Host SHA256 is
  `7853e96d90dbf56990329b125eb64cec5b5463153661b3da0fe0be56204429e7`.
  `actual-neural-smoke.py` reproduces that synthetic integer observation with
  separately provisioned approved model assets. This checks runtime integration. Population PII quality requires a separate
domain evaluation.
- `p9a-review.json`: the reviewed remote CSO PR #381 matched all twelve reviewed
  implementation/evidence files. Its historical receipt records 72 passes and
  one Vue dependency skip, plus 10/10 profile cases. Those tests were not rerun
  here. Its independent producer pin and consumer base remain unchanged.

## Diagnostic runs

`workspace-fixture-failure.*` records the first failed workspace run. Repeated Rust
debug-build hashing of the 30.9 MB analysis interpreter exhausted synthetic inspection
budgets. `fixture-profile-diagnostic.json` measured 108.658 seconds of executable
hashing and 6.948 seconds of guardian work. The corrected standard-library fixture
explicitly selects and validates the 8 MB system Python. Every launch still checks its
exact executable identity. Production limits and real-model interpreter selection remain
unchanged. Final focused and full suite runs pass.

`workspace-disk-full.*` records a subsequent environmental failure when an
unused task-created checkpoint transfer staging directory filled the disk. That
staging area and unused incremental cache were retired before the final rerun.
It made no source changes. Earlier neural attempts also correctly refused an
asset closure containing transfer staging files. The accepted neural run used
only the existing manifest-approved files in a separate clean asset tree, with
all exact asset checks retained. These diagnostics are not accepted passes.

## Reproduction and scope

Use the repository's pinned Rust/Go toolchains and Python 3.11 or later. CI selects Node
22.14.0. This local run used Node 24.19.0. The synthetic response fixture selects
`/usr/bin/python3`. Its standalone preparation/verifier accepts an explicit absolute
`--fixture-python` override and approves that exact binary.

```sh
CARGO_INCREMENTAL=0 GOFLAGS=-buildvcs=false make test
make acceptance
make provider-test
PATH=/path/to/spin/bin:$PATH python3 formal/verify.py --out /tmp/formal-evidence
```

`validate-command.py` records a complete non-evidence source hash set before and
after a command, command status, elapsed time and log digest. The formal runner
records its own bounds, hashes, exact commands and replay checks. See the two
model correspondence documents under `formal/` for properties and assumptions.

These results establish finite control properties and implementation conformance for the
exercised paths. A Rust/Python refinement proof remains a separate obligation. The
trusted native callback remains outside malicious-worker confinement. File-snapshot
authority checks are not linearised with later dispatch. Endpoint acceptance is distinct
from recipient delivery. An unresolved submission blocks redispatch under its original
action identity. Automated retry and reconciliation require a later scheduler and
admission contract.

At this checkpoint, completing N2 required acquisition, inference, revision and
contextual coverage. E3 still needed resolver, lineage, subscriber and
credential-lifecycle work. N3 required case adjudication and durable scheduling. E4
deletion/restore and E5 isolation then precede consumer and measured deployment, quality
and capacity qualification. The [implementation
sequence](../../docs/nehemiah-implementation.md) records later increments and the
remaining dependencies. Managed/shared profiles stay closed. No merge, deployment or
manual workflow dispatch is part of this increment.
