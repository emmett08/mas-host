# Governed structured acquisition

Acquiring restricted evidence must constrain the work that follows it. The opt-in
`mas-governed-local/2` contract binds a registered acquisition effect to its source
policies. The existing execution journal retains the approved structured projection and
carries those policies through later requests, effects and delivery. This trusted-local
path supplies a prerequisite for E3 evidence use and protected revision.

## Operator-owned policy association

Version 2 requires `baseline_policy_refs`, a nonempty set of policy-template IDs
from the current `mas-governed-authority/1` file. Every selected baseline policy
participates in a boundary decision. A registered effect can additionally name
a nonempty `source_policy_refs` set. Those policies govern acquisition and the
subsequent use of its result. The host derives the association from the effect
registration and its exact durable identity.

For example, an operator can register a collector as follows within its complete
version 2 configuration:

```json
{
  "version": "mas-governed-local/2",
  "baseline_policy_refs": ["baseline"],
  "agents": [{
    "actor": "collector",
    "effects": [{
      "format": "deployment-point/1",
      "source_policy_refs": ["restricted-observation"]
    }]
  }]
}
```

This fragment shows the association fields. The complete configuration also requires the
existing authority/key paths, location, destinations and closed input/output/evidence
schemas. Each policy rule retains its correlated operation, actor, principal, recipient,
purpose, destination and location tuple. Every applicable rule must permit the same
operation and disclosure tuple. Keeping the tuple intact prevents a purpose from one
rule being combined with a different rule's destination to widen access.

Version 1 retains its all-template policy semantics and closed configuration
shape. Version 2 binds its explicit selections into the retained namespace
fingerprint. It requires a fresh namespace or a separately verified migration.
The existing host-binary and bundle identity checks continue to apply.

## Acquisition and exposure

Acquisition admission checks the real effect descriptor against the baseline, inherited
restrictions and registered source policies before returning execution permission. The
host retains a pending acquisition obligation before granting that permission. Pending
acquisition prevents admission of new outbound work. A callback must therefore report
its acquired result before requesting further peers or actions through the host.
Interrupted acquisition preserves the pending obligation and its original expiry.
Version 2 applies this pending rule to every effect, including effects with
baseline-only source policies.

Completion retains the original effect identity, a source association and the
recipient's exposure in the existing journal transaction. The source points to
the approved retained projection. A private value removed during handling stays
outside that projection. The source's acquisition time and expiry originate at
admission, and its expiry is bounded by its grant, policies and ancestors.

Read `coverage: complete` as a statement about the retained closed projection and its
resolved binding. Detector coverage and quality require their separate receipts and
qualification.

The host records admitted exposure before a worker receives a value. A child inherits
its parent's complete source restrictions before input persistence and dispatch. A
parallel command freezes the parent's exposure for its children. Results admitted later
constrain subsequent commands. Worker-supplied lineage cannot reduce this
host-maintained history.

The same restrictions govern argument values, target selection, associated
metadata, returned results and retained delivery. Processing by a configured
detector requires its own current permission under the inherited policies
before a callback receives content.

## Complete bounded ancestry

Source associations and invocation exposure belong to `execution.sqlite`.
Payloads remain in existing step records. The operator's current authority file
continues to own grants and policy templates.

Every use resolves and validates the complete ancestor closure, original bindings,
policy references and expiry. Missing, conflicting or cyclic lineage prevents admission.
Admission requires the complete history. A policy template can apply to several sources,
with every source still contributing its own restrictions.

The implementation admits at most 64 sources in one complete closure and keeps the
existing 65,536-byte representation bounds. Either bound can prevent an operation.
Denial leaves the existing admission result in force, without creating a separate
persisted HOLD record. The 33-ancestor fixture checks the restriction on the last
ancestor explicitly. This is bounded qualification for the tested representation. Larger
closures and shared multi-principal readership require further work.

## Read the retained projection

`GetRetainedSource` selects one registered acquisition effect using an owned A2A
task ID and logical key. The service derives the run, actor and root invocation.
Current resolve authority and the complete original source closure govern the
returned metadata and projection. A configured inspection gate retains its
processing and payload checks. See [A2A usage](a2a.md) for the negotiated
extension and exact response fields.

The read keeps source availability, host settlement and external execution
certainty separate. It preserves a completed or uncertain effect's original
identity and performs observation without execution recovery or another fetch.
`raw_source_disposition` reports `not-retained`: discarded original bytes remain
unavailable. Exact per-component assessments and source-byte adjudication need
their own retention and authority contracts.

Existing task and effect readers apply the inherited restrictions for version 2
namespaces. These readers are scoped to the owned task: they conjoin its current
complete exposure with the selected source's ancestry. Reading an earlier effect
therefore cannot bypass restrictions acquired later by that task. The source view's
`lineage` describes the selected source's own ancestry. Current task exposure adds a
separate admission constraint. Pending acquisition blocks these reads.

Admission checks use the file-backed profile's authority and journal exposure snapshots.
The host rechecks recipient exposure after callbacks. A concurrent change can still
occur between the final check and transmission. Authority or expiry can change in that
interval too. Shared linearised admission requires the separately qualified shared
authority path.

Pending acquisition can make a status read temporarily unavailable. A new version 2
`SendMessage` returns the submitted admission snapshot captured before worker launch.
Subsequent status and replay responses use the current complete exposure. Repeating a
status query observes the same task. Further execution requires its separate admission
and recovery contract.

## Verification and next work

The [governed-acquisition example](../examples/governed-acquisition/README.md)
compares the same downstream action and peer before and after restricted
acquisition. A permitted peer consumes the acquired observation. A second
acquisition control uses baseline-only source policies. Restart and replay reuse
the original collection and source identities. The example uses a fixed
synthetic Deployment point and fixture neural responses. Its result establishes
host conformance for that scenario. External collector/backend behaviour and
detector quality retain their separate qualification gates.

Independent implementation checks cover source-policy conjunction, ancestry,
expiry, corruption, pending acquisition, current rights and existing-reader
delivery. The [exposure model](../formal/exposure-lineage.md) checks the
corresponding exposure and admission transitions in Promela. Evidence is tied
to the source revision that runs those checks under
`evidence/nehemiah-governed-acquisition/`.

The existing single-owner journal keeps execution ownership. This design extends its
source associations rather than introducing another evidence authority. Source-specific
restrictions require history across invocations, which the earlier per-value synthetic
source model leaves unrepresented. Extending the existing journal keeps that history
under the current execution owner. A separate evidence service would add credential,
consistency and migration obligations.

The supported profile remains trusted-local and structured. Exact assessment
custody, protected revision, dynamic credentials, shared subscribers, deletion
and restore, worker isolation and the full notification lifecycle remain planned
dependencies. Detector quality and deployment capacity retain their separate
qualification gates.
