# Shared native dispatch model

`shared-native.pml` models one original invocation, one committed admission and
one authority handover. Run `python3 formal/verify.py --model shared-native`.
The default verifier includes these cases with the existing host models.

A committed admission can return a fresh consuming permit or lose its reply.
Only the fresh permit starts work. A retained command receipt supplies observation.
A completed native worker can retain completion under its original current owner,
or remain uncertain after lost output or fencing. Identical request replay keeps
completion and allocation rather than issuing another permit.

The abstraction preserves allocation count, worker dispatch count, admitted epoch,
current epoch, completion, uncertainty and fresh-permit possession. Exact SQL
transactions, database schema compatibility, isolation, source/binary identity,
clock arithmetic, configured resource bounds and guardian cleanup remain separate
implementation obligations. The model establishes no PostgreSQL implementation
refinement, distributed confidentiality, scheduler fairness or capacity result.

| Model transition | Implementation boundary |
|---|---|
| Committed admission | Existing native `admit_invocation` transaction, wrapped by typed `admit_native_invocation` |
| Fresh permit consumed | `shared_native::execute` consumes `FreshDispatchPermit` before `Worker::spawn_scoped` |
| Lost admission reply | Typed uncertain outcome returns without a worker launch |
| Current completion | Existing `observe_invocation` checks run owner and authority epoch in its transaction |
| Completion reply lost | Original invocation remains observable and a repeated CLI request cannot dispatch |
| Replayed command | Typed retained outcome reads the exact invocation |

Four complete searches cover safety and finite settlement with and without
partial-order reduction. Four guard-removal mutations demonstrate sensitivity
to replay dispatch, uncertain-admission reallocation, stale-owner completion and
completion erasure. Four replayed witnesses establish reachability of completion,
uncertainty, lost admission acknowledgement and fenced completion. Every search
uses the existing verifier's explicit time, memory and depth limits.

Actual PostgreSQL/native-worker checks live in
`crates/mas-host-cli/tests/shared_native.rs`. They independently record synthetic
source execution. The formal result and real-backend acceptance retain separate
status, and a skipped backend check supplies no conformance evidence.
