# Host control checks

Run `python3 formal/verify.py` from the repository root. SPIN and a C compiler
must be installed. The default output is `formal/evidence`; `--out DIRECTORY`
selects another destination. The runner records tool versions, model hashes,
exact commands, stdout/stderr, search counts and replayed counterexamples.
Counterexample sources, generated verifiers and trails are retained together.
The committed [verification receipt](../evidence/revision/verification.json) and
[compressed raw evidence](../evidence/revision/spin-checks.tar.xz) retain the checked
assessment/revision models, generated verifiers, trails and execution logs from
their original increment. They do not cover the later delegation-policy model.
Fresh runs write to the ignored output directory and CI retains that directory
as a workflow artifact.

These models check a finite control abstraction. They do not prove that a
candidate diagnoses an incident correctly, that a model evaluator is reliable,
or that revision improves outcomes. The experiment in `mas-host-examples`
measures those separate questions.

## Boundary and bounds

`revision.pml` represents one serialised host run with two candidate identities,
one possible revision, four possible invocations and a frozen invocation limit
chosen from one through four. The host observes at most one cancellation,
deadline expiry or crash. Reservation, external effect start, durable effect
completion and invocation completion are separate transitions. A crash can
occur between these transitions. An incomplete external effect blocks recovery;
a completed effect can be reused by a restarted native handler, and a completed
invocation can be replayed without dispatch. The model does not assert that
native handler computation executes only once.
The host assessment is retained separately from the worker checkpoint. Terminal
delivery failure may retain an error after a worker returned a valid result. A
crash between assessment retention and selection must reuse that exact assessment,
including its error, rather than recomputing it from the worker checkpoint.

Each represented invocation has at most one external inference effect. Multiple
inference calls or delegated invocations are outside the revision model's four
slots; implementation tests cover their shared budget and inherited policy.
The host's owner lock serialises the run. Journal writes are modelled as atomic
SQLite commits. Storage loss, hash collisions and malicious workers are excluded.
Deadlines are abstract observations with fixed values, not quantitative timing
proofs. Provider work must return or reach its externally enforced deadline.
No fairness switch is used: positive executions contain finitely many transitions.

`disclosure.pml` compares two worlds whose public evidence and invocation policy
are equal and whose withheld peer findings differ. It asserts equality of the
assembled independent-reviewer input, then checks replay against its frozen
snapshot after live evidence changes. It also checks rejection of a policy
change under the same invocation identity, intersection of supplied-record
reader sets, authorisation to disclose derived input to a child, and inheritance
of the parent's reader set by that child.
It also represents a collision between source and host-generated evidence identities:
preflight prevents draft dispatch, and candidate evidence retains the candidate's
readers independently of source readers. It abstracts identity comparison to one
collision flag; Rust tests exercise actual reserved strings and hashes.

The disclosure claim covers host-mediated inputs, peer history and feedback
under the existing trusted-worker boundary. Direct file access, external tools,
model-provider state and timing channels are not represented. Both worlds must
have the same explicitly permitted inputs; changing a permitted candidate is
outside the non-disclosure claim.

## Implementation correspondence

| Model operation or property | Implementation boundary | Required implementation evidence |
| --- | --- | --- |
| Frozen plan, deadline and invocation budget | `Engine::revise`, `Inner::freeze_revision`, `Inner::bind_context`, original `Inner::deadline_ms` | Changed retained plan/input rejects; exact repeated reservation does not spend again; exhaustion prevents dispatch |
| Reservation before work | `revision_contexts` transaction and `Inner::request` | Unique invocation identity reserves budget before host execution; nested requests share that budget |
| Reuse of completed invocation/effect | `Inner::request`, `Inner::effect`, persisted `steps` | Restart returns retained output and effect evidence; no duplicate external effect |
| Unknown effect blocks recovery | `Engine::open` recovery of executing effects, `Inner::has_unknown`, `Inner::has_unsettled_effects` | Crash with unresolved effect cannot authorise a retry |
| Cancellation and expiry | `Inner::check_live`, transactional `Inner::bind_context`/`terminal_revision`, and worker deadline | A stop committed after assessment but before the next reservation or terminal commit prevents that new transition; exact retained receipts remain immutable; uncertainty remains recorded |
| Evaluator error never selects a passing result | Revision controller assessment handling and fixed selection rule | Invalid output/error remains distinct from a completed passing assessment |
| Input self-composition and replay | `revision::context::assemble`, `Inner::bind_context` | Changing withheld peer sources produces identical reviewer input; policy/context drift rejects |
| Derived readers and child permission | `revision::context::intersection`, `Inner::inherit_context` | A child must satisfy both allowed-agent and inherited-reader checks; descendants retain restrictions |
| Revision cannot authorise remediation | `Inner::check_effect_policy` | Only `mas-inference/2` is admitted by the revision context; command/remediation effects reject |
| Assessment identity, authority and immutable history | `mas` assessment contract plus host append-only `revision_records` | Exact duplicate is retained once; changed body/readers reject; candidate/evaluator binding is validated |
| Terminal assessment replay | `revision::evaluation::Inner::retained_assessment` | Restart revalidates every exact binding and reuses a known error despite a passing worker checkpoint |
| Host evidence namespace | `revision::context::check_sources`, candidate evidence retention in `Engine::revise` | Reserved source identities reject before draft; host candidate evidence inherits candidate readers |

The `mas` contract model supplies the separate identity and evaluator-authority
checks. The existing `mas-host-examples` remediation models and implementation
tests check actual execution preconditions. The host model's zero-remediation
effect assertion establishes that the revision wrapper grants no remediation
authority; it does not reconstruct the Kubernetes admission rules.

The `stop_after_assessment_prevents_new_reservation_at_transaction_boundary`
regression places cancellation or expiry between the caller's live check and
the next journal transaction. It checks that no invocation reservation or terminal
completion is inserted and that the retained assessment remains unchanged.
`retained_terminal_replay_survives_later_stop_without_reclassification` checks
the opposite commit order: a previously committed terminal receipt survives.

This table is an implementation correspondence review, not a formal refinement
proof. Existing per-candidate incident choreography remains inside each request;
the revision controller adds a bounded outer sequence.

## Non-vacuity and result interpretation

The runner executes safety and each model's named LTL check both with
and without partial-order reduction. Safety builds disable never claims but
retain assertions and bounds checks. LTL builds explicitly select the claim and
enable acceptance-cycle detection. Search-depth exhaustion is an error. Each
subprocess has a 30-second timeout and a 512 MiB address-space limit.

Isolated deliberate defects must produce the intended assertion or LTL cycle:
unknown-effect retry, completed-effect replay, evaluator error promoted to pass,
revision-budget expansion, permission expansion, deadline extension, remediation
authorisation, hidden-peer leakage, replay reassembly, policy rebinding, derived
reader expansion, unauthorised child disclosure and child reader expansion.
A terminal-assessment mutation recomputes a retained error from the passing worker
checkpoint. An evidence-namespace mutation permits draft dispatch with a colliding
source identity; a separate mutation gives candidate evidence the source's readers.
Each must produce the intended assertion violation.
A stalled-controller mutation tests the liveness configuration.

Separate inverted assertions witness reachable successful completion, replay,
unknown effects, revision exhaustion, invocation-budget exhaustion, evaluator
error, restricted feedback and terminal assessment replay. A witness counterexample is expected evidence of
reachability, not a defect in the positive model. A mutation counterexample
establishes that the corresponding check detects the injected defect.

A successful finite search establishes only the named properties for these
bounds and assumptions. `formal/evidence/summary.json` reports the exact checked
model hashes; it must be regenerated if a model changes. Raw logs retain
unreached-state reports and compiler warnings for inspection.

Execution follows the official SPIN documentation:
[verifier options](https://spinroot.com/spin/Man/Pan.html),
[inline LTL](https://spinroot.com/spin/Man/ltl.html), and
[atomic sequences](https://spinroot.com/spin/Man/atomic.html).

## Required delegation policy

`delegation.pml` checks the optional `min_delegate_calls` and
`required_delegates` host policies independently of the assessment models. The
bounded service has one configured LLM invocation, two permitted peers, every
subset of those peers as the required set, three inference turns, a minimum of
zero, one or two successful peer replies, and at most one replay. A provider can
complete, request either peer, return invalid/unavailable output, or leave
execution unknown. A peer either returns successfully or fails. A successful
response may report unavailable observations: coverage concerns completed
acquisition attempts, not the usefulness of their contents.

Cancellation/expiry is abstracted as a terminal stop. External work must return
or reach its existing deadline; the model does not establish elapsed-time bounds
or change that deadline. The empty required set preserves the earlier schema
and minimum-call behaviour. For a nonempty required set, the last inference
turn excludes requests. Before coverage is complete, the advertised request
targets are exactly the outstanding required peers. Once covered, all configured
peers are available again on earlier turns. Completion additionally requires the
configured minimum successful reply count.

| Property | Required behaviour | Check |
| --- | --- | --- |
| Completion threshold | A successful final result has at least the configured number of successful peer replies | `terminal == Completed` implies `successful >= minimum` |
| Distinct required coverage | Every required peer has returned before successful completion; repeated calls cannot replace another peer | Completion implies `(covered & required) == required` |
| Advertised schema | Completion is available exactly when both policies hold; outstanding required peers restrict targets; a configured required set excludes final-turn requests | Assertions for `complete_allowed`, `permitted_requests` and `request_allowed` |
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

The implementation mapping is `config::bindings` for admission of a valid
minimum, distinct required subset and sufficient turn limit;
`runtime::inference::action_schema` for the advertised decision variants and
targets; and `runtime::inference::execute` for successful history, coverage,
completion admission and the existing turn limit. `Context::effect` and
`Context::request` retain inference and peer outcomes. The replay model abstracts
their traversal to rebuilding the same count and identity union before the next
active turn; it does not model a new retry after an invalid or unknown effect.
Rust tests provide the separate evidence for schema generation, runtime
admission, retained outcomes and actual redispatch counts.

This model checks host policy, with no changes to the Nehemiah wire protocol.
It does not prove that a returned peer result contains useful observations,
that an LLM follows a prompt, or that a live investigation succeeds. It assumes
the existing identity, journal and delegation mechanisms; their storage and
interleaving behaviour remains covered by implementation tests and the other
relevant models. These are finite control checks and implementation
correspondence, rather than a refinement proof of the Rust runtime.

## Inference-result admission and durable replay

`provider-results.pml` checks the [host inference-result profile](../docs/inference-results.md)
without changing Nehemiah wire version 1. Its service is one current inference
decision which may authorise one downstream external effect. An optional retained
earlier peer has a different identity: a conflicting current result must not
authorise the next effect. This does not forbid independent later turns from
requesting the same peer again, and it does not assert that a native handler
executes only once.

The finite environment has eighteen response classes, with one to three messages
and either original or reversed order. Classes include one result, two or three
equal values, conflicting values, different opaque `input_json` strings, a
malformed sibling, commentary alone or beside a final result, refusal in excluded
commentary, incomplete and nonterminal responses, unknown transport, single or
repeated schema-invalid results, inadmissible request input, and split, absent or
non-string text. These are representative classes, not every combination of
three arbitrary provider messages. Exact parsed values are abstract identities
`VALUE_A` and `VALUE_B`; the model neither parses JSON nor recursively normalises
JSON encoded inside strings. Parser and equality tests supply that separate
implementation evidence.

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
Preparation, durable executing state, actual external dispatch, observation,
selection, schema classification and durable result retention are separate
transitions. The delegate effect has separate start, dispatch, observation and
retention transitions too. A crash after an observed known result but before its
durable retention leaves an uncertain effect and blocks recovery; it must not
reconstruct certainty from the discarded observation. Retained invalid results
stay known and invalid. A retained valid inference may be reused after a crash
while the downstream handler continues; a downstream effect whose execution is
uncertain blocks. Atomic sections cover one local step or one journal transition,
not the whole inference/dispatch/retention sequence.

| Model property or transition | Implementation correspondence | Independent implementation evidence |
| --- | --- | --- |
| Eligible-message selection; all equal values accepted once | `http::parse_response` and `parse_output` | Provider tests for phase filtering, repeated equal values, malformed siblings, conflicting values, text shape and exact JSON types/strings |
| Schema validity distinct from request admission | `inference::classify` followed by `inference::execute` and `Context::request` | Runtime schema tests and application integration fixtures reject invalid decisions/requests without another tool call |
| Executing state before external work; known result retained before action | `Inner::begin_effect`, `Context::effect`, `Inner::complete_effect` | Core/SDK effect tests and native recovery acceptance tests exercise actual journal and worker boundaries |
| Known-invalid replay stays invalid with unchanged dispatch counts | Retained effect output plus terminal invocation state | Scripted real-host integration fixtures replay failed investigations without new provider or MCP calls |
| Uncertain effect blocks; cached valid result skips provider work | `Inner::begin_effect`, recovery and cached `Context::effect` | Core recovery tests distinguish executing/unknown from completed/resolved effects |
| Descriptor changes cannot rebind an identity | `Inner::prepare` | Core identity tests reject changed kind, actor, target or descriptor under a retained key |

Assertions require at most one current inference dispatch and one current
downstream effect, valid admission for that effect, immutable retained status,
immutable identity binding and unchanged effect counters during replay. Without
a crash, a known unusable observation must remain a known retained failure,
rather than becoming uncertain. The
conditional service assertion requires successful completion when the result is
admissible, the delegate returns successfully and no crash occurs. The separate
`provider_results_settle` LTL claim requires eventual terminal settlement,
including failure or blockage. It assumes represented external work returns or
reaches its existing deadline; it proves no elapsed-time bound or eventual
reconciliation. No fairness switch is used.

The runner adds four full-state safety/progress searches, with and without
partial-order reduction. Fourteen deliberate defects check first-output selection,
ignored malformed siblings, commentary promotion, ignored refusal, rejection of
equal repetitions, skipped schema or request-admission checks, known-invalid
results misclassified as uncertain, inference/peer replay dispatch, retry
after known-invalid or uncertain outcomes, reclassification of retained invalid
results and descriptor rebinding. Thirteen reachability witnesses cover single,
double and triple valid results; conflict after an earlier peer; malformed and
schema-invalid retained replay; schema-valid but inadmissible requests; retained
refusal; unknown replay; loss before retention; cached
inference recovery; delegate uncertainty after a crash; and rejected rebinding.
A permanent-stall mutation must produce a replayed acceptance cycle.

The model assumes one owner, durable atomic journal writes and admitted effect
identities, and excludes storage loss, hash collisions, hostile workers, external
reconciliation, cancellation/deadline interleavings and provider process cleanup.
Not-sent outcomes and explicit HTTP rejection are not separate response classes
in this model; provider/runtime implementation tests cover those boundaries.
The existing revision model and implementation tests cover relevant excluded
host behaviour. These results establish finite control properties and an
implementation correspondence; they are not a proof that the Rust implementation
refines the model or that any live provider run will succeed.
