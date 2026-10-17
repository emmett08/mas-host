# Assessment and revision control checks

Run `python3 formal/verify.py` from the repository root. SPIN and a C compiler
must be installed. The default output is `formal/evidence`; `--out DIRECTORY`
selects another destination. The runner records tool versions, model hashes,
exact commands, stdout/stderr, search counts and replayed counterexamples.
Counterexample sources, generated verifiers and trails are retained together.
The committed [verification receipt](../evidence/revision/verification.json) and
[compressed raw evidence](../evidence/revision/spin-checks.tar.xz) retain the checked
models, generated verifier sources, trails and execution logs from this increment.
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
| Cancellation and expiry | `Inner::check_live` and worker deadline | Persisted cancellation and original expiry stop further admission; uncertainty remains recorded |
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

This table is an implementation correspondence review, not a formal refinement
proof. Existing per-candidate incident choreography remains inside each request;
the revision controller adds a bounded outer sequence.

## Non-vacuity and result interpretation

The runner executes safety and `settles`/`context_finishes` LTL checks both with
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
model hashes; it must be regenerated if either model changes. Raw logs retain
unreached-state reports and compiler warnings for inspection.

Execution follows the official SPIN documentation:
[verifier options](https://spinroot.com/spin/Man/Pan.html),
[inline LTL](https://spinroot.com/spin/Man/ltl.html), and
[atomic sequences](https://spinroot.com/spin/Man/atomic.html).
