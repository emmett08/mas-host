# Governed local control model

`governed-local.pml` asks whether the trusted-local structured path preserves authority,
source identity and known completion across inspection, interruption and replay. It
extends the existing host checks with the optional exact-source inspection gate. Run it
alone with `python3 formal/verify.py --model governed-local`, or run the existing
default command to include every host model. CI uses the default command.

## Service and finite environment

The model represents one tenant-bound invocation, one mediated external effect,
two tenant identities and two distinct original inputs with the same permitted
projection. Request and result channels each hold one message. One independently
scheduled authority update and one crash are optional. The authority update can
revoke a grant, make its file unavailable, rotate a token, change the grant epoch,
or change the policy/key identity. One explicit core replay can re-authenticate
under the new token or same-policy grant epoch. A changed key or policy cannot
reopen that namespace.
The explicit retry and the independent crash recovery can traverse cached
completion twice, while the external dispatch count remains at most one.

An external participant returns a safe result, a prohibited result or an unknown
transport outcome. Returned external certainty has four distinct values:
unknown, completed, not-sent and refused. A completed host receipt never supplies
an external fact absent from that observation. A crash after durable executing
permission, including before actual external dispatch, conservatively leaves an
unknown effect. A crash after the execution-journal completion transaction
retains its receipt and can repair the separate protocol completion.

Each input, effect-descriptor, completion and retained-output inspection uses the same
five callback classes:

1. Complete, exact-source inspection with clean content.
2. A clean projection with a positive finding in removed original content.
3. Incomplete or failed coverage.
4. Wrong source or detector identity.
5. A finding in the retained projection.

The final three classes refuse input admission or withhold a known completion's payload.
The original-positive case can use the explicitly approved narrower structured
projection. Current authority is checked before the callback and again after it.
Available retained output is inspected again for delivery. An exact host-owned withheld
marker retains safe completion facts without depending on detector availability.

`Safe`, `Raw` and `Withheld` name representation classes. Parsing and detector quality
require separate implementation and empirical evidence. `inspection_exact` abstracts
exact source and detector/config bindings. `inspection_complete` includes full admitted
coverage and successful termination within the implementation's registered bounds. The
model has no real-time clock. Rust checks separately cover the 16 KiB/128-text-unit
limits, UTF-8 offsets, decoded JSON keys/values and the shared callback deadline.

## Properties and actual atomicity

Each host atomic block represents one local step or one SQLite transaction.
Request preparation, protocol request retention, durable effect executing state,
external dispatch, observation, inspection, execution completion, protocol
completion and delivery are separate transitions. Receipt and execution-step
completion share their actual journal transaction. The authority process can
interleave between every pair of these transitions.

The successful `CURRENT` predicate abstracts the callback/key/authority read
sequence for one check. Those files have independent read boundaries in Rust.
The finite environment changes one authority component once. It does not model
arbitrary replacement sequences or establish an atomic multi-file snapshot.
The `snapshot-race-witness` deliberately reaches an authority change between a
successful read and the following executing-state commit. The supported claim
is a fresh snapshot check at each represented boundary. Linearised revocation
requires stronger authority/storage coordination.

| Property | Model check | Implementation correspondence |
| --- | --- | --- |
| Project before retention | Neither journal nor protocol input/output nor receipt contains `Raw` | `Inner::governed_input`, `Inner::governed_effect_completion`, `Inner::prepare`, `Inner::complete_effect`, `protocol::finish_effect` |
| Exact original identity | The retained commitment remains the first original even when another input has the same projection | `storage::bind_input`, `GovernedSession::commitment`, A2A original-parameter commitment |
| Current admission and delivery | Each represented boundary reads current authority before proceeding | `GovernedSession::current`, `Inner::admit_request`, `Inner::begin_effect`, `GovernedSession::check_delivery` |
| Complete exact-source inspection | Acceptance implies complete coverage, exact bindings and a clean retained projection | `InspectionGate::check_implementation`, `GovernedSession::inspect_boundary`, `NeuralAssessment::validate`, `CombinedAssessment::new` |
| Recheck after inspection | A callback cannot reuse its earlier positive authority check after revocation | `inspect_boundary` and subsequent `authorize_inspected` |
| Immutable known completion | Receipt and completed step agree, payload disposition and external certainty remain sealed | `storage::save_completion`, `Inner::complete_effect` transaction |
| Independent result dimensions | Withholding never erases host settlement or promotes external uncertainty to completion | `boundary::certainty`, `completion_values`, `governed_effect_completion` |
| Repair and replay | Completed/unknown effects retain their dispatch count. Protocol repair copies only retained safe values. | `Inner::reconcile_completed_effects`, `Inner::begin_effect`, `protocol::finish_effect` |
| Tenant separation | A foreign tenant cannot receive the represented result | A2A owner derivation and retained task ownership checks |
| Exact host marker | An unrelated parent or actor cannot borrow another effect's completed certainty | `Inner::governed_output` exact parent/actor/output receipt lookup |
| Finite settlement | `governed_local_settles`: every represented execution finishes with delivery, denial or blockage | Bounded returning-work abstraction, rather than a wall-clock or eventual-reconciliation theorem |

The model composes the listed admission, completion and recovery transitions. The source
mapping supplies implementation correspondence. Establishing refinement requires a
separate proof over the implementation and model executions. The Rust implementation
applies the shared inspection gate to additional task assembly and peer delivery. Their
exact serialisation and call-site coverage require the separate implementation tests.

Core replay is an explicitly requested retry using retained identity. A2A's
service recovery keeps submitted/working tasks unknown after restart and returns
their recovery identifiers. This model does not turn a duplicate A2A submission
into permission to restart application execution.

## Sensitivity and reachability

The runner checks safety and `governed_local_settles` with and without
partial-order reduction. Full-state checks retain assertions and invalid-end
checks. Search depth is 100,000, each command has a 30-second limit and a 512 MiB
address-space limit, and no fairness or approximate hashing options are used.

Isolated mechanism mutations must expose raw input/completion/protocol retention,
stale effect/delivery checks, external-certainty promotion, erased completion,
borrowed completion facts, foreign-tenant delivery, changed-original rebinding,
unknown/completed redispatch, missing inspection coverage/binding/finding checks,
or stale authority after inspection. A permanent external-stall mutation must
produce an acceptance cycle. Every expected assertion or cycle is replayed and
retained with its source, generated verifier and trail.

Inverted assertions witness successful available delivery, known completed withheld
output, cached replay, unknown effects, restart with protocol repair, completion after
revocation, changed-input and foreign-tenant refusal, the snapshot race, same-policy
epoch refresh, positive-original minimisation, inspection refusal and inspection-induced
completion withholding. Each inverted assertion deliberately fails when the named path
is reachable. Its counterexample establishes that reachability in the positive model.
The restart-repair witness requires the crash to occur after execution-journal
completion and before protocol completion. An additional witness attempts an explicit
retry of an unknown effect and reaches blockage without dispatch.

Storage loss, malicious callbacks/workers, hash collisions, arbitrary participant
counts, shared ownership, notifications, deletion/restore, general free-text release and
actual detector population performance remain outside this model. The concrete callback
implementation must return or reach its enforced deadline. The progress claim depends on
this finite-return premise. Extending it to unbounded provider progress or deployment
availability requires stronger assumptions and evidence.

The runner hashes every model and itself before and after execution, records
selected models, versions, commands, resource limits and complete-search results,
and rejects source drift. Fresh results belong to their exact model bytes and
implementation revision. Historical state counts do not qualify changed code.
