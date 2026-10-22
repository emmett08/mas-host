# Auditable assessment and bounded revision

A revision run generates a candidate, assesses it against a frozen rubric and revises it
within explicit limits. Start from an existing bundle with `mas-host run --bundle
bundle.json --agent draft --input sources.json --state state --run incident-1
--revision-plan revision.json`. The original execution bindings and budgets still apply,
including topology, model settings, provider token caps and the absolute deadline.
Reusing the run ID requires the exact original plan and sources. A revision run cannot
be resumed as an ordinary unrestricted run, or converted from previously executed
ordinary work.

The public `Engine::revise(plan, input)` API implements the same operation. The
controller owns the loop. Native, LLM and workflow workers execute each stage through
their existing request/effect machinery. Application examples live in
`mas-host-examples`, including the incident workflow and three-arm experiment.

## Frozen plan and invocation input

A plan has version `mas-host-revision/1` and these fields:

| Field | Meaning |
|---|---|
| `draft_agent`, `revision_agent` | Registered worker names for generation and revision |
| `max_revisions` | Maximum additional candidates, 0–16 |
| `max_invocations` | Maximum admitted worker invocations, 1–1024, including nested calls and evaluators |
| `draft_inputs`, `revision_inputs` | Exact source IDs supplied to those stages |
| `allowed_agents` | Additional peer targets available to generation/revision and inherited by their children |
| `rubric` | Default `assessment.v1` rubric |
| `evaluators` | `{actor, identity, inputs, rubric?}` bindings, with an optional distinct rubric for model judgements alongside deterministic measurements |
| `feedback_evaluators` | Evaluator IDs whose assessments may be supplied during revision, subject to reader restrictions |

Each evaluator identity contains `id`, `version` and `kind`. Its rubric's criteria must
all use that measurement kind. Reusing a rubric ID requires identical content. The host
rejects an LLM implementation registered as a deterministic evaluator. The frozen
operator plan fixes evaluator identities and rubric authority before workers return
their findings.

Input has one `sources` array. Each source contains `id`, `kind` (`input`,
`evidence` or `peer_history`), object `content`, and `readers`. Readers are an
explicit array of actor names, or `["*"]` for public content. Labels are assertions
by the trusted operator. Empty readers disclose nothing to a worker.
Source IDs must not start with `candidate-`, `attempt-` or `budget-`: these
prefixes identify host-generated evidence. This check runs before draft dispatch,
including for sources that no worker requests. Candidate output evidence always
inherits the candidate's readers, independently of source lookup.

A worker receives `{sources, evidence, evidence_refs, candidate, feedback}`.
Sources contain only the permitted named content. Evidence records and precomputed
full-record digest references allow evaluators to cite exact evidence without
asking a model to compute hashes. Evaluators also receive their rubric and an
evidence record for the candidate's output. Candidate records bind their complete
output by SHA256 and link the exact parent and the feedback actually disclosed.

Evaluator workers return exactly `{status, findings}`. Findings contain `criterion_id`,
`verdict`, integer `score` (or null when inconclusive), `evidence` references, and
`explanation`. The host binds this result to the exact candidate, rubric and authorised
evaluator, then applies the protocol validator. Missing, malformed, out-of-range and
failed evaluator results become immutable error assessments with no passing findings.
Inconclusive results remain inconclusive. The host constructs these records in its
journal. Worker requests and results continue through the existing MAS envelope and task
payload.

## Disclosure and replay

Before an invocation starts, the host atomically retains its exact assembled input,
source references, peer/effect policy and effective readers. Reserving the same
identity twice does not spend another invocation. A changed context or policy
cannot reuse the identity. Budget rejection occurs before retaining an unscoped
request step. Source and evaluator bindings are checked before draft dispatch.

Independent assessment invocations receive no peer feedback and cannot delegate.
Changing a source they were not given cannot change their assembled input. Revision
feedback is filtered by both the evaluator allowlist and the recipient's reader
permission. The candidate links only assessments that were actually supplied.

A derived record inherits the intersection of all supplied records' readers. Delegated
requests inherit those readers and the original peer/effect policy. Descendants must
satisfy both restrictions. Only the maintained LLM implementation can admit
`mas-inference/2` effects during revision. Command effects and remediation effects are
rejected before reservation. Existing remediation admission and verification remain
outside this loop.

This guarantee covers host-mediated inputs, delegation, feedback and public observation.
Native workers retain trusted local filesystem/network access. Protecting data from
malicious workers or the operator requires additional isolation. Here, reviewer
independence means withholding other reviewers' findings. Statistical independence of
models, training data or errors requires separate evidence.

The journal retains candidates, assessments, rubrics and evidence append-only. Exact
duplicates reuse the retained record. Conflicting contents or reader labels under one
identity are rejected. Conflicting findings from different evaluators remain separate
records. On interruption, unfinished orchestration may restart under its original
invocation identity and reuse completed request/effect checkpoints. An admitted effect
with unknown outcome is never automatically repeated. An unknown evaluation attempt
remains retained as evidence and blocks execution. Finishing the pending assessment
requires explicit evidence-backed resolution through the existing effect mechanism. A
known error assessment cannot later be replaced with a pass. Replay first revalidates a
retained assessment against the exact candidate, rubric, authorised evaluator, available
evidence, invocation context and readers, then reuses that assessment without repeating
the evaluator request. This also applies when terminal delivery failed after a valid
worker output was checkpointed: recovery preserves the recorded error instead of
reclassifying that output. Frozen plan/input and terminal selection rows have the same
append-only database protection as assessment records and invocation contexts. Resuming
revision through `revise` installs these guards without rewriting existing rows.

## Selection, termination and observation

The fixed selection rule accepts the current candidate only when every evaluator returns
a completed assessment with every criterion passing. Otherwise it revises while the
original limits permit. At exhaustion it selects the last candidate and reports
`exhausted`. Refusal by the original call or communication quota produces the same
terminal status. If draft delivery never completed, the result has no selected
candidate. Message refusals use the protocol admission code. Call refusals use retained
usage counters because the existing call API returns unstructured errors. Worker error
wording cannot declare a quota exhausted. Treat an exhausted selection as the last
available candidate. Acceptance still requires every evaluator to pass every criterion,
and remediation requires its separate authority. Evaluator overhead, nested calls and
further assessment of revised candidates all consume existing limits. Cancellation and
expiry prevent further worker dispatch. Retained observation distinguishes cancelled,
expired and blocked runs. Completed selection is reused on restart under the same
original deadline.

`run` returns `output` and a `mas-host-revision-audit/1` projection. `observe` exposes
the same audit while the owner is active, without changing database state. Records carry
exact hashes, readers and references. Public payload opt-in never overrides reader
restrictions: restricted candidate outputs, context input/policy, assessment bodies,
protocol payloads and effect evidence are redacted. A restricted selected output is null
in the public result. `inspect` is the trusted local operator view and includes the
retained full audit. Keep that administrative view within its authorised audience.

The bounded Promela models, correspondence and mutation controls are in
[`formal/README.md`](../formal/README.md). Their results establish properties of the
stated finite abstractions. Rust unit tests and real CLI/native-worker tests cover
replay, conflicting identity, disclosure, cancellation, expiry, exhaustion and forbidden
remediation effects. Improved incident diagnosis requires the separately frozen held-out
experiment. Formal control checks establish execution properties, while internal scores
record the evaluators' judgements.
