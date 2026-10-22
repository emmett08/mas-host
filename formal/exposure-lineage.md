# Structured acquisition and exposure

`exposure-lineage.pml` checks finite control for opt-in governed configuration
version 2. It follows one parent, two parallel children, one registered
acquisition effect and one later external action. The acquisition callback
attempts nested outbound work before reporting completion. A sealed pending
source blocks that attempt, including when a started acquisition later becomes
unknown. The model does not reconstruct a general source graph or prove the
Rust implementation refines Promela.

The original seven models and their 184 case tuples remain unchanged. Run this
increment with `python3 formal/verify.py --model exposure-lineage --out DIRECTORY`.
The runner retains commands, tool versions, exact source hashes, generated
verifiers, trails and assertion/cycle replays. Parent validation runs the full
default suite after these sources have been frozen.

## Represented decisions and bounds

The baseline restriction is bit `B`; a registered additional restriction is
bit `R`. Baseline allows both represented admission tuples. `R` permits tuple
class 1 and denies class 2. Child/effect target classes represent different
destinations; result tuples can also differ by operation/payload constraints
for the same recipient. This is an abstraction of conjunction across all
original policy templates, rather than a new merged template. Worker-supplied
lineage is an empty or narrower claim and cannot replace the host's exposure.
Every opaque child output inherits its immutable input closure.

Closure classes are a single unrestricted ancestor, a valid 33-node ancestry
whose restrictive ancestor is visited at position 33, and missing, cyclic,
conflicting or oversized ancestry. The oversized class represents 65 sources
and is denied rather than truncated to 64. The 32-node unrestricted prefix is
compressed in metadata, but the model traverses all 33 positions. Arbitrary DAG
topologies, exact antichain/frontier computation, the exact 65,536-byte decision
budget, JSON schemas, MAC construction and cryptographic collision resistance
require implementation evidence. The Rust ancestor regressions construct actual
chains with at least 33 sources, including the independent 35-node case.

There is one parent batch snapshot and two independently scheduled children.
Both inputs use that same snapshot. A child's later acquisition can enlarge
the parent's received exposure without changing the already frozen sibling
input. Each child's frozen closure and current admission must permit its target
before context/step retention and dispatch. A later parent action consumes the
enlarged exposure. Receiving a later
baseline-only sibling reply unions its restrictions with existing exposure.
It cannot overwrite a restriction learned from the first child.
Sibling input admission and result delivery are distinct: the baseline sibling
can run under its frozen input and later have its return withheld under the
parent's enlarged exposure. Result checks conjoin source and recipient exposure
before inspection and again for the actual payload. A final pure comparison
after policy callbacks denies recipient-context growth observed at that cut.
Actual reply I/O is a separate transition, so later growth does not invalidate
the earlier admission snapshot. The model represents policy-relevant growth
from `B` to `B | R`; exact source-ID/frontier equality is implementation-tested.

There is at most one optional crash and one authority update. The update either
revokes current authority or advances the abstract time from 0 to 2. Parent
expiry 2 and registered policy expiry 3 exercise the minimum at acquisition;
the grant expiry is at least 3. Pending and completed sources retain that
original minimum. One failed execute transaction may roll back completely and
the same prepared effect can make one fresh admission attempt. There is one
explicit retained action retry, with matching or changed source-binding
classes. The retry consults current authority, inherited disclosure policy and
the retained source expiry. A known or unknown external action cannot issue a
second external call. Pure worker computation is not subject to an exactly-once
claim.

The model checks registered processing rights before descriptor and observed
result inspector callbacks. Inspector outcomes abstract complete admissible
inspection or any partial, failed or rejected inspection. Result inspection
can withhold the payload while keeping the known host receipt. Source-storage
failure also preserves the known receipt while preventing delivery; a missing
source is never treated as an unrestricted source.

## Atomicity and implementation correspondence

| Model operation | Implementation boundary | Scope |
| --- | --- | --- |
| Resolve complete closure or deny | `governance/runtime/exposure.rs::visit`, `load_manifest`, `Exposure::union`, `Source::parse` | The closure-validity classes abstract exact graph, source/step, manifest and payload bindings |
| Freeze both child inputs before either can influence the other | `execution.rs` parallel request branch and `request_with_exposure`; `save_context` | One immutable parent manifest is supplied to every child in the batch |
| Admit source processing before callbacks | `boundary.rs::governed_input`, `session.rs::prepare_effect_with_exposure`, `inspection.rs::inspect_boundary_with_exposure`, `exposure_authorization.rs::authorize_exposure_sized` | Registered additional templates constrain the real descriptor; result processing uses inherited source authority |
| Commit pending manifest and executing step together | `effects.rs::begin_effect`, `lineage.rs::begin_governed_effect`, `exposure.rs::begin_pending` | One journal transaction precedes execute permission; rollback leaves prepared without pending |
| Pending blocks native callback's nested work | `exposure.rs::load_invocation`, sealed pending membership in `load_manifest` | Pending is a separate sealed record, not a completed payload; unknown/restart preserves it |
| Keep completion truth while source handling fails | `effects.rs::complete_effect` completion transaction and lineage savepoint | A failed lineage savepoint may retain the host receipt but cannot authorise payload delivery |
| Create distinct complete source and remove pending atomically | `lineage.rs::bind_governed_effect_output`, `exposure.rs::create_source`, `finish_pending` within `complete_effect` | The completed source retains the pending acquisition time/expiry and full parent closure |
| Expose recipient before reply | `exposure.rs::expose` in fresh/cached request and effect delivery | Journal exposure commit precedes protocol/native payload; crash may occur between them |
| Recheck recipient history after callbacks | `lineage.rs::governed_recipient_exposure`, `final_governed_delivery`, used by native/cached reply helpers | Final actual-payload admission uses source plus current recipient union, followed by pure context/expiry comparison before later reply I/O |
| Apply full inherited policy to later effect and output | `authorize_exposure_sized`, effect preparation/delivery and root retained-delivery helpers | Source policies compose with current baseline authority; worker refs cannot narrow them |
| Reuse exact source and original expiry on replay | `create_source`, `load_step_source`, `source_resolver.rs` | Changed immutable source binding rejects; cached reads do not renew acquisition expiry |

Each journal transaction is one atomic model transition. A current-authority
read is a separate snapshot, followed by callback, commit or dispatch; it is
not synchronised with changes to the authority file. The explicit race witness
permits revocation after a successful current check and before execute callback.
The model claims fresh checks at the represented boundary, not distributed
revocation linearisation.
Recipient exposure has a similar final-read/reply window: the pure journal read
defines an admission snapshot and does not linearise subsequent I/O against
parallel exposure growth. Reentrant descriptor callbacks that grow the same
acquiring invocation before pending commit are outside the single-acquisition
callback class; the runtime's locked admitted-context comparison was reviewed
in implementation and is outside this finite model. No policy callback is
assumed to execute inside a journal transaction.
For owned retained-effect/source views, public lineage can describe only the
selected source ancestry, while admission additionally conjoins current parent
invocation exposure. The cached-payload gate here uses that represented full
parent restriction. `retained-effect.pml` separately supplies the coherent
read-snapshot and independently valid whole-view policy abstraction; these two
models do not prove a joint refinement for arbitrary reader/source graphs.

External execute permission, adapter invocation, observed result, durable
completion and recipient reply are separate transitions. Crashing after execute
permission but before any network call is conservatively unknown. Crashing
after observing a result but before the durable completion likewise retains
uncertainty. A durable completion remains known under restart, revocation,
inspection failure and failed lineage storage. The model represents explicit
core retry, rather than A2A automatically resuming an unknown task.

## Properties and counterexample controls

Safety and `exposure_lineage_finishes` are checked with and without partial-order
reduction. Positive paths are finite under the existing returning-work/deadline
premise; no fairness flag is used. A deliberate stalled acquisition supplies an
acceptance cycle to check the LTL configuration. Full-state searches retain
assertions and bounds checks, with the unchanged 30-second command limit,
512 MiB address-space limit and depth 100,000.

Mechanism mutations truncate traversal at 32, accept invalid closure, substitute
live parent exposure for a frozen sibling context, omit a child's inherited
target permission, dispatch before persisting
child exposure, inspect under baseline-only authority, inspect a result without
its processing permission, defer pending attachment until completion, split the
pending/executing transaction, reply before exposure, overwrite accumulated
exposure, trust narrower worker lineage, admit a baseline sibling result using
only its source despite the recipient's newly restrictive exposure, accept a
changed cached binding, retry unknown or known external work, refresh expiry on
cached delivery and erase a known completion after revocation. Each mutation
must violate its intended assertion and replay that violation. In particular,
the expiry mutation changes the guard used by cached delivery, and the identity
mutation admits a changed binding to the original cached result.

Separate reachability controls witness an approved action, an inherited-policy
denial, the 33rd ancestor's effective restriction, an actually delivered allowed
frozen sibling,
blocked nested pending work, retained known-but-withheld completion, an unknown
retry denial, crash between exposure and reply, expiry denial, invalid closure
denial, inherited child-target denial before retention, processing denial, the
permitted check/dispatch race, cached completion,
live-authority withholding of a baseline sibling under restrictive recipient
history, prepared retry after transaction rollback and
failed result inspection with retained completion. Witness counterexamples are
expected evidence of reachability, rather than positive-model defects.

This increment does not establish semantic information-flow completeness,
declassification, free-text privacy quality, arbitrary graph closure, malicious
worker confinement, unmediated files/provider memory, deletion, remote shared
case ownership or a complete E3 implementation. Exact source seals and bounded
policy decisions are validated by Rust tests and independently reviewed source
mapping. The formal result is limited to the finite control distinctions above.

The tool semantics were rechecked against the official SPIN
[verifier options](https://spinroot.com/spin/Man/Pan.html),
[inline LTL](https://spinroot.com/spin/Man/ltl.html) and
[atomic sequence](https://spinroot.com/spin/Man/atomic.html) references.
