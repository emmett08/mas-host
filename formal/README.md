# Host control checks

These models check whether bounded host control rules preserve admission, disclosure and
recovery across interruption and replay. Run `python3 formal/verify.py` from the
repository root. SPIN and a C compiler must be installed. The default output is
`formal/evidence`. Select another destination with `--out DIRECTORY`. The runner records
tool versions, model hashes, exact commands, stdout/stderr, search counts and replayed
counterexamples. Counterexample sources, generated verifiers and trails are retained
together. The committed [verification receipt](../evidence/revision/verification.json)
and [compressed raw evidence](../evidence/revision/spin-checks.tar.xz) retain the
checked assessment/revision models, generated verifiers, trails and execution logs from
their original increment. They do not cover the later delegation-policy model. Fresh
runs write to the ignored output directory and CI retains that directory as a workflow
artefact.

The [governed local model](governed-local.md) adds snapshot-authorised structured
admission, exact-source inspection, pre-persistence projection, completion with
withheld output and crash/replay checks. Its scope and source mapping are
separate from the historical revision/disclosure checks below.
The [privacy response adapter model](privacy-response.md) separately checks
safe-intent ordering, current notification admission, provider certainty and
duplicate/unknown handling for the initial N3 adapter.
The [read-only retained-effect model](retained-effect.md) checks coherent
snapshots, current whole-view disclosure and retained completion facts while a
live owner operates, with no reader recovery, journal mutation or dispatch.
The [structured exposure model](exposure-lineage.md) checks registered source
restrictions before inspector callbacks, pending acquisition before execute
permission, immutable parallel inputs, recipient exposure before reply and
retained source authority on later actions and replay. It adds 40 checks to the
original seven models' unchanged 184 checks.

Each model establishes named properties within a finite control abstraction. Diagnosis
quality, evaluator reliability and improvement through revision need empirical evidence.
The experiment in `mas-host-examples` measures those separate questions.

## Boundary and bounds

`revision.pml` represents one serialised host run with two candidate identities, one
possible revision, four possible invocations and a frozen invocation limit chosen from
one through four. The host observes at most one cancellation, deadline expiry or crash.
Reservation, external effect start, durable effect completion and invocation completion
are separate transitions. A crash can occur between these transitions. An incomplete
external effect blocks recovery. A restarted native handler reuses a completed effect,
and a completed invocation replays without dispatch. Pure handler computation may run
again between those checkpoints. The host assessment is retained separately from the
worker checkpoint. Terminal delivery failure may retain an error after a worker returned
a valid result. A crash between assessment retention and selection must reuse that exact
assessment, including its error, rather than recomputing it from the worker checkpoint.

Each represented invocation has at most one external inference effect. Multiple
inference calls or delegated invocations require more than this model's four slots.
Implementation tests cover their shared budget and inherited policy. The host's owner
lock serialises the run. Journal writes are modelled as atomic SQLite commits. Storage
loss, hash collisions and malicious workers are excluded. Deadlines are abstract
observations with fixed values, not quantitative timing proofs. Provider work must
return or reach its externally enforced deadline. No fairness switch is used: positive
executions contain finitely many transitions.

`disclosure.pml` compares two worlds whose public evidence and invocation policy are
equal and whose withheld peer findings differ. It asserts equality of the assembled
independent-reviewer input, then checks replay against its frozen snapshot after live
evidence changes. It also checks rejection of a policy change under the same invocation
identity, intersection of supplied-record reader sets, authorisation to disclose derived
input to a child, and inheritance of the parent's reader set by that child. It also
represents a collision between source and host-generated evidence identities: preflight
prevents draft dispatch, and candidate evidence retains the candidate's readers
independently of source readers. The model represents identity comparison with one
collision flag. Rust tests exercise the actual reserved strings and hashes.

The disclosure claim covers host-mediated inputs, peer history and feedback under the
existing trusted-worker boundary. Direct file access, external tools, model-provider
state and timing channels are not represented. Both worlds must have the same explicitly
permitted inputs. The non-disclosure comparison holds the permitted candidate fixed
while varying withheld findings.

## Implementation correspondence

| Model operation or property | Implementation boundary | Required implementation evidence |
| --- | --- | --- |
| Frozen plan, deadline and invocation budget | `Engine::revise`, `Inner::freeze_revision`, `Inner::bind_context`, original `Inner::deadline_ms` | Changed retained plan/input rejects. Exact repeated reservation reuses budget. Exhaustion prevents dispatch. |
| Reservation before work | `revision_contexts` transaction and `Inner::request` | Unique invocation identity reserves budget before host execution. Nested requests share that budget. |
| Reuse of completed invocation/effect | `Inner::request`, `Inner::effect`, persisted `steps` | Restart returns retained output and effect evidence with the external dispatch count unchanged. |
| Unknown effect blocks recovery | `Engine::open` recovery of executing effects, `Inner::has_unknown`, `Inner::has_unsettled_effects` | Crash with unresolved effect cannot authorise a retry |
| Cancellation and expiry | `Inner::check_live`, transactional `Inner::bind_context`/`terminal_revision`, and worker deadline | A stop committed after assessment but before the next reservation or terminal commit prevents that new transition. Exact retained receipts remain immutable, with uncertainty still recorded. |
| Evaluator error never selects a passing result | Revision controller assessment handling and fixed selection rule | Invalid output/error remains distinct from a completed passing assessment |
| Input self-composition and replay | `revision::context::assemble`, `Inner::bind_context` | Changing withheld peer sources preserves reviewer input. Policy/context drift rejects. |
| Derived readers and child permission | `revision::context::intersection`, `Inner::inherit_context` | A child must satisfy both allowed-agent and inherited-reader checks. Descendants retain restrictions. |
| Revision cannot authorise remediation | `Inner::check_effect_policy` | The revision context admits only `mas-inference/2`. Command/remediation effects reject. |
| Assessment identity, authority and immutable history | `mas` assessment contract plus host append-only `revision_records` | Exact duplicates reuse one record. Changed body/readers reject. Candidate/evaluator binding is validated. |
| Terminal assessment replay | `revision::evaluation::Inner::retained_assessment` | Restart revalidates every exact binding and reuses a known error despite a passing worker checkpoint |
| Host evidence namespace | `revision::context::check_sources`, candidate evidence retention in `Engine::revise` | Reserved source identities reject before draft. Host candidate evidence inherits candidate readers. |

The `mas` contract model supplies the separate identity and evaluator-authority checks.
The existing `mas-host-examples` remediation models and implementation tests check
actual execution preconditions. The host model's zero-remediation-effect assertion
checks that revision stays within its inference-only authority. The examples' models and
tests retain responsibility for Kubernetes admission rules.

The `stop_after_assessment_prevents_new_reservation_at_transaction_boundary`
regression places cancellation or expiry between the caller's live check and
the next journal transaction. It checks that no invocation reservation or terminal
completion is inserted and that the retained assessment remains unchanged.
`retained_terminal_replay_survives_later_stop_without_reclassification` checks
the opposite commit order: a previously committed terminal receipt survives.

The table maps model operations to implementation checks. A formal refinement proof
would require a separate relation between all relevant executions. Existing
per-candidate incident choreography remains inside each request, with the revision
controller adding a bounded outer sequence.

## Non-vacuity and result interpretation

The runner executes safety and each model's named LTL check both with
and without partial-order reduction. Safety builds disable never claims but
retain assertions and bounds checks. LTL builds explicitly select the claim and
enable acceptance-cycle detection. Search-depth exhaustion is an error. Each
subprocess has a 30-second timeout and a 512 MiB address-space limit.

Isolated deliberate defects must produce the intended assertion or LTL cycle:
unknown-effect retry, completed-effect replay, evaluator error promoted to pass,
revision-budget expansion, permission expansion, deadline extension, remediation
authorisation, hidden-peer leakage, replay reassembly, policy rebinding, derived reader
expansion, unauthorised child disclosure and child reader expansion. A
terminal-assessment mutation recomputes a retained error from the passing worker
checkpoint. An evidence-namespace mutation permits draft dispatch with a colliding
source identity. A separate mutation gives candidate evidence the source's readers. Each
must produce the intended assertion violation. A stalled-controller mutation tests the
liveness configuration.

Separate inverted assertions witness reachable successful completion, replay,
unknown effects, revision exhaustion, invocation-budget exhaustion, evaluator
error, restricted feedback and terminal assessment replay. A witness counterexample is expected evidence of
reachability, not a defect in the positive model. A mutation counterexample
establishes that the corresponding check detects the injected defect.

A successful finite search establishes only the named properties for these bounds and
assumptions. `formal/evidence/summary.json` reports the exact checked model hashes.
Regenerate it after any model change. Raw logs retain unreached-state reports and
compiler warnings for inspection.

Execution follows the official SPIN documentation:
[verifier options](https://spinroot.com/spin/Man/Pan.html),
[inline LTL](https://spinroot.com/spin/Man/ltl.html), and
[atomic sequences](https://spinroot.com/spin/Man/atomic.html).

## Required delegation policy

`delegation.pml` checks the optional `min_delegate_calls` and `required_delegates` host
policies independently of the assessment models. The bounded service has one configured
LLM invocation, two permitted peers, every subset of those peers as the required set,
three inference turns, a minimum of zero, one or two successful peer replies, and at
most one replay. A provider can complete, request either peer, return
invalid/unavailable output, or leave execution unknown. A peer either returns
successfully or fails. A successful response may report unavailable observations.
Coverage counts completed acquisition attempts, while the application must assess
whether their contents are useful.

Cancellation/expiry is abstracted as a terminal stop. External work is assumed to return
or reach its existing deadline. Elapsed-time bounds require measurements or a separate
timing model under the same deadline. The empty required set preserves the earlier
schema and minimum-call behaviour. For a nonempty required set, the last inference turn
excludes requests. Before coverage is complete, the advertised request targets are
exactly the outstanding required peers. Once covered, all configured peers are available
again on earlier turns. Completion additionally requires the configured minimum
successful reply count.

| Property | Required behaviour | Check |
| --- | --- | --- |
| Completion threshold | A successful final result has at least the configured number of successful peer replies | `terminal == Completed` implies `successful >= minimum` |
| Distinct required coverage | Successful completion requires a returned observation from every required peer. Repeating a peer leaves other required peers outstanding. | Completion implies `(covered & required) == required` |
| Advertised schema | Both policies determine completion availability. Outstanding required peers restrict targets. A configured required set excludes final-turn requests. | Assertions for `complete_allowed`, `permitted_requests` and `request_allowed` |
| Replay preservation | Reconstructing completed history preserves the reply count and identity union and dispatches no new provider or peer work | Counter and bitset equality assertions in `replay_history` |
| Bounded terminal outcome | Every execution reaches a terminal outcome under the returning-work premise | `delegation_finishes`, checked separately from safety |
| Success is possible | Two distinct successful delegations can be followed by completion within three turns, in either order | `WITNESS_COMPLETE` and `WITNESS_REVERSE_ORDER` |
| Default compatibility | An empty required set and zero minimum permit completion without a peer call | `WITNESS_ZERO` |
| Failure remains terminal | Early completion, partial coverage or a repeated target while another is required fails without another dispatch or retry | `WITNESS_EARLY_INVALID`, `WITNESS_PARTIAL_INVALID` and `WITNESS_DUPLICATE_INVALID` |

The runner checks safety and progress with and without partial-order reduction.
Nine deliberate defects exercise early completion admission, partial-coverage
admission, an over-permissive completion schema, an over-permissive target set,
repeated-target schema admission, final-turn request admission, duplicate replay counting, changed replay coverage
and dispatch during replay. A stalled inference mutation exercises the LTL
failure path. Inverted assertions witness successful coverage, reverse ordering,
the default completion path, completed-history replay and invalid early,
partial or repeated-target decisions. Every expected counterexample is replayed
and retained by the runner.

Implementation responsibilities follow the same sequence. `config::bindings` validates
the minimum, distinct required subset and sufficient turn limit.
`runtime::inference::action_schema` advertises the permitted decision variants and
targets. `runtime::inference::execute` reconstructs successful history, checks coverage
and admits completion within the turn limit. `Context::effect` and `Context::request`
retain inference and peer outcomes. The replay model reconstructs the same count and
identity union before the next active turn. Invalid or unknown effects remain subject to
their existing terminal and recovery rules. Rust tests provide the separate evidence for
schema generation, runtime admission, retained outcomes and actual redispatch counts.

This model checks host policy, with no changes to the Nehemiah wire protocol. It does
not prove that a returned peer result contains useful observations, that an LLM follows
a prompt, or that a live investigation succeeds. The model assumes the existing
identity, journal and delegation mechanisms. Implementation tests and the other relevant
models cover their storage and interleaving behaviour. These are finite control checks
and implementation correspondence, rather than a refinement proof of the Rust runtime.

## Inference-result admission and durable replay

`provider-results.pml` checks the [host inference-result profile](../docs/inference-results.md)
without changing Nehemiah wire version 1. Its service is one current inference
decision which may authorise one downstream external effect. An optional retained
earlier peer has a different identity: a conflicting current result must not
authorise the next effect. This does not forbid independent later turns from
requesting the same peer again, and it does not assert that a native handler
executes only once.

The finite environment has eighteen response classes, with one to three messages and
either original or reversed order. Classes include one result, two or three equal
values, conflicting values, different opaque `input_json` strings, a malformed sibling,
commentary alone or beside a final result, refusal in excluded commentary, incomplete
and nonterminal responses, unknown transport, single or repeated schema-invalid results,
inadmissible request input, and split, absent or non-string text. These are
representative classes, not every combination of three arbitrary provider messages.
Exact parsed values become abstract identities `VALUE_A` and `VALUE_B`. JSON parsing and
the treatment of JSON encoded inside strings require separate implementation checks.
Parser and equality tests supply that separate implementation evidence.

The host counts eligible absent/null/final phases, checks the shape of every
eligible message, parses every represented text and compares all parsed values.
Its selection result is checked against an independent response-class oracle.
This includes a positive service requirement: repeated equal eligible messages
must be accepted, subject to the remaining checks. `schema_valid` represents
local output validation. `request_valid` separately represents exact parsing of
`input_json`, its object requirement, and target/operation admission. Both are
required before a downstream effect, so schema conformance alone does not
authorise execution.

The bound permits one independently scheduled crash and one terminal replay.
Preparation, durable executing state, actual external dispatch, observation, selection,
schema classification and durable result retention are separate transitions. The
delegate effect has separate start, dispatch, observation and retention transitions too.
A crash after observing a result but before retaining it durably leaves an uncertain
effect and blocks recovery. Certainty requires the retained observation, which the crash
has lost. Retained invalid results stay known and invalid. A retained valid inference
may be reused after a crash while the downstream handler continues. An uncertain
downstream effect still blocks execution. Atomic sections cover one local step or one
journal transition, not the whole inference/dispatch/retention sequence.

| Model property or transition | Implementation correspondence | Independent implementation evidence |
| --- | --- | --- |
| Eligible-message selection and acceptance of equal values once | `http::parse_response` and `parse_output` | Provider tests for phase filtering, repeated equal values, malformed siblings, conflicting values, text shape and exact JSON types/strings |
| Schema validity distinct from request admission | `inference::classify` followed by `inference::execute` and `Context::request` | Runtime schema tests and application integration fixtures reject invalid decisions/requests without another tool call |
| Executing state before external work and known result retained before action | `Inner::begin_effect`, `Context::effect`, `Inner::complete_effect` | Core/SDK effect tests and native recovery acceptance tests exercise actual journal and worker boundaries |
| Known-invalid replay stays invalid with unchanged dispatch counts | Retained effect output plus terminal invocation state | Scripted real-host integration fixtures replay failed investigations without new provider or MCP calls |
| Uncertain effect blocks and cached valid result skips provider work | `Inner::begin_effect`, recovery and cached `Context::effect` | Core recovery tests distinguish executing/unknown from completed/resolved effects |
| Descriptor changes cannot rebind an identity | `Inner::prepare` | Core identity tests reject changed kind, actor, target or descriptor under a retained key |

Assertions require at most one current inference dispatch and one current downstream
effect, valid admission for that effect, immutable retained status, immutable identity
binding and unchanged effect counters during replay. Without a crash, a known unusable
observation must remain a known retained failure, rather than becoming uncertain. The
conditional service assertion requires successful completion when the result is
admissible, the delegate returns successfully and no crash occurs. The separate
`provider_results_settle` LTL claim requires eventual terminal settlement, including
failure or blockage. It assumes represented external work returns or reaches its
existing deadline. Elapsed-time bounds and eventual reconciliation remain separate
obligations. No fairness switch is used.

The runner adds four full-state safety/progress searches, with and without partial-order
reduction. Fourteen deliberate defects check first-output selection, ignored malformed
siblings, commentary promotion, ignored refusal, rejection of equal repetitions, skipped
schema or request-admission checks, known-invalid results misclassified as uncertain,
inference/peer replay dispatch, retry after known-invalid or uncertain outcomes,
reclassification of retained invalid results and descriptor rebinding. Thirteen
reachability witnesses cover the following paths:

- Single, double and triple valid results, plus conflict after an earlier peer.
- Malformed and schema-invalid retained replay, schema-valid but inadmissible requests, and retained refusal.
- Unknown replay, loss before retention, cached inference recovery and delegate uncertainty after a crash.
- Rejected descriptor rebinding.

A permanent-stall mutation must produce a replayed acceptance cycle.

The model assumes one owner, durable atomic journal writes and admitted effect
identities, and excludes storage loss, hash collisions, hostile workers, external
reconciliation, cancellation/deadline interleavings and provider process cleanup.
Not-sent outcomes and explicit HTTP rejection have their own provider/runtime
implementation tests and remain outside this model's response classes. The existing
revision model and implementation tests cover relevant excluded host behaviour. These
results establish finite control properties and an implementation correspondence. A Rust
refinement proof or a claim about live provider success requires separate evidence.
