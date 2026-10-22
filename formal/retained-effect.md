# Read-only retained-effect resolver model

`retained-effect.pml` checks the first governed retained-effect resolver on top of
the existing local journal. Run `python3 formal/verify.py --model retained-effect`
for this model or the default command for every host model.

## Supported question and abstraction

The model asks whether an authorised reader can obtain a consistent,
identity-bound observation without changing an effect or executing work. It
preserves the distinctions needed for that question: current permission,
namespace, parent request, actor, logical effect key, whole returned view,
retained host settlement, external execution certainty and payload availability.

The source space comprises the existing SQLite namespace, request/effect rows,
sealed completion row and current governed session. The abstract space uses exact
identity classes, five effect-state classes, four external-certainty classes and
available/withheld/unavailable payload classes. Equality is required for retained
identity and completion facts. Failed payload inspection conservatively reduces
availability. It grants no execution permission. Exact JSON encoding, HMAC
collision resistance, schema/byte validation and policy implementation remain
separate implementation obligations.

The finite model pairs query and metadata admission in one fixed permission
class. Independent byte-limit rejection of a whole metadata view after a smaller
query was admitted is covered by the Rust implementation tests. The model checks
the final decision's identity and current-authority binding, without proving the
policy evaluator or its byte arithmetic.

There is one reader, one live owner, at most one owner journal commit and at most
one independently scheduled authority revocation or outage. Each namespace,
parent, actor and logical-key domain has two identities. The represented effect
states are absent, prepared, executing, completed and unknown. An authorised
absence under a verified parent returns `not-recorded`. A missing or foreign
parent and denied access produce the same unavailable failure class.

Completed host settlement retains an independently represented external fact:
unknown, completed, not-sent or refused. In particular, a completed host receipt
with external certainty unknown is admitted. Withholding preserves both facts.
The model does not infer that an executing effect was unsent and does not turn
absence, uncertainty or payload unavailability into permission to dispatch.

The model covers a single resolver observation. A consumer combining several
resolver calls has separate snapshots and requires its own disclosure and
consistency contract. General evidence-lineage closure, shared subscribers,
deletion/restore and notification reconciliation remain subsequent work.

## Transitions and source correspondence

Query authorisation and exact-query inspection precede database access. A fresh
current check follows query inspection. The read transaction captures the
namespace, parent, effect and completion as one SQLite snapshot. It commits and
releases the connection before payload inspection or subsequent authority
callbacks. The live owner may commit before or after this snapshot. The model
uses the ordinary SQLite read-lock boundary and makes no database throughput or
wall-clock claim.
An incomplete or failed query inspection returns unavailable before database
access. Metadata-only fallback covers a payload failure after an admitted query.

Within the read snapshot, the reader validates exact namespace, parent and actor
bindings before querying the effect-specific row or completion. A different
logical key can legitimately yield an authorised `not-recorded` observation. It
cannot return another key's effect. For completed content, current payload
schemas and a complete, exact-source clean inspection determine availability.
Partial, failed or prohibited inspection withholds the payload. Current resolve
policy must independently admit the exact whole metadata-only view.

The final policy decision binds the actual returned envelope. A decision for a
placeholder cannot authorise a larger or differently bound view. Current rights
are checked after inspection. As with other file-backed local paths, an authority
change after that check and before return remains a permitted snapshot race. The
explicit authority-race witness preserves that limitation.

| Property | Model assertion | Implementation correspondence |
| --- | --- | --- |
| Authorisation before lookup | `lookup` implies a successful fresh precheck | `read_governed_effect` query resolve/inspection gate before the read-only connection |
| One coherent snapshot | Step and completion revisions agree | Read-only SQLite transaction containing namespace, parent, effect and completion reads |
| Release before callbacks | Inspection requires a closed database connection | Transaction commit and connection drop before payload inspection and final resolve |
| Exact identity | Effect lookup requires verified namespace, parent and actor; returned identities equal the request | Namespace/session fingerprint checks and parent request target before the keyed effect/receipt lookup |
| Actual envelope permission | Authorised view identity equals returned view identity | Whole `mas-governed-effect-view/1` resolve decision |
| Current post-inspection rights | A successful final check observed current authority | Current session and final whole-view authorisation |
| Conservative handling | Available output requires valid payload permission and complete exact-source clean inspection | Current effect schemas and `PrivacyInspector` assessment validation |
| Retained facts survive withholding | Returned host state and external certainty equal the snapshot | Sealed governed completion consistency and metadata-only fallback |
| Read-only operation | Reader write, recovery and dispatch counters remain zero | `SQLITE_OPEN_READ_ONLY` and direct resolver calls without `Engine::open` |
| Finite settlement | `retained_effect_finishes`: eventually returned or denied | Finite bounded-inspector abstraction, rather than a latency or availability theorem |

The model abstracts each returned payload to a class. The implementation's closed
13-field view also carries `version`, opaque `namespace_ref`, `actor`,
`parent_invocation`, `key`, `effect_id`, `state`, `host_settlement`,
`external_execution`, `payload_disposition`, `descriptor`, `output` and
`evidence`. Implementation tests check exact whole-envelope policy/byte binding
and the separate application-payload inspection.

`read_governed_root_effect` additionally selects the authorised root request.
Its exact root-selection and stored-identity checks require implementation coverage.
The finite model begins with the selected parent and checks the resolver's
subsequent binding and disclosure obligations.

## Checks and interpretation

The existing runner adds four positive searches: safety and
`retained_effect_finishes`, each with and without partial-order reduction. It
retains assertions, bounds and invalid-end-state checks. Commands have a
30-second timeout, 512 MiB address-space limit and search depth 100,000.
Full-state searches use no approximate hashing or fairness switches.

Fifteen isolated mechanism mutations bypass namespace, parent, actor, key or
whole-view binding; reuse stale pre-lookup or post-inspection authority; split
the SQLite snapshot; invoke callbacks before releasing the database; introduce
reader writes, recovery or dispatch; bypass inspection; erase completed
settlement after withholding; or promote external certainty. Each must produce
and replay its intended assertion failure. A stalled-inspector mutation must
produce a replayed acceptance cycle.

Ten inverted assertions witness available output, known completion with
inspection-induced withholding, unknown effects, an authorised not-recorded
effect, a live owner commit, an older coherent snapshot, revocation after result
inspection, binding refusal, query-inspection refusal and the permitted final
authority-check race. A witness counterexample establishes reachability in the
positive model. It does not identify a defect in the implemented resolver.

The runner records exact model/runner hashes, versions, bounds, commands,
search completion and replayed trails. It rejects source changes during checking.
Results establish only the named properties of this finite control abstraction.
The source mapping and controlled Rust/HTTP tests provide correspondence evidence
without claiming a formal refinement proof of Rust, SQLite, the policy engine,
cryptography or detector quality.

Execution follows the official [SPIN verifier options](https://spinroot.com/spin/Man/Pan.html),
[inline LTL semantics](https://spinroot.com/spin/Man/ltl.html) and
[atomic sequence semantics](https://spinroot.com/spin/Man/atomic.html). The tool
versions and the actual command output in the receipt determine the executed
configuration.
