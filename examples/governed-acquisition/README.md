# Governed structured acquisition

This trusted-local example shows how acquiring restricted evidence changes the
operations an invocation may perform. A synthetic Deployment collector returns a
structured point, and the host carries its source restrictions into a permitted consumer
and authenticated retained reads. Native invocation and effect state stay in the
existing execution journal.

The bounded point follows the target and quantity separation demonstrated in
[`structured-evidence`](../structured-evidence/README.md): cluster and namespace
incarnations, exact resource UID/version, generation, desired replicas and ready
replicas. `freshness` and `assessment` remain `NOT_RUN`. The fixture reads local
synthetic data. Live freshness and model quality require observations from separately
qualified collectors and detectors.

## Exercise the host

From a built checkout:

```sh
python3 examples/governed-acquisition/verify.py --host target/debug/mas-host
```

The verification prepares one private temporary application and runs two
sequential tasks through the real loopback A2A2 service:

1. A baseline task executes a registered action and calls a peer successfully.
2. An acquisition task executes the collector once and receives its approved
   structured point. The host retains the registered source restrictions with
   that effect. The same constant action and peer request are then denied. Their
   execution counters remain at the baseline counts. A permitted peer consumes
   the acquired `ready_replicas` value and succeeds.

The acquired task reads its projected source and original effect, restarts the service,
and reads both again. Their exact retained identities and projections remain equal.
Replaying the original request performs no collection, action or peer call again.
Finally, withdrawing the current grant by removing its active record from the authority
file denies both read APIs, ordinary task retrieval and message replay. The authority
file remains structurally valid, with the selected grant removed.

A final positive control reuses the same bundle, acquire-mode workflow, schemas and
detector assets in a fresh state namespace. Only the registered source policy selection
changes to `['baseline']`. Acquisition and consumption still occur, and the action and
peer now execute. Their counters increase. This control attributes the denial to the
selected source policies: acquisition succeeds in both cases, and downstream permission
changes with that selection. The original restricted namespace is not reopened with
changed configuration. The original active grant is restored after the restricted
service stops. This control performs no additional source reads or restart.

The collector supplies an extra synthetic private field outside its output schema. The
deterministic recognisers inspect that original field, and the host retains the
permitted projection. The test checks that the original private bytes appear in neither
returned views nor execution state. The original commitment binds the discarded input to
its identity. Recovering those bytes would require a separate authorised source store.

The synthetic neural worker returns complete fixture responses without model inference.
It uses the real bounded offline worker channel with an explicitly selected Python
3.11-or-later interpreter and exact executable/worker hashes. The default interpreter is
`/usr/bin/python3`. Select another with `--fixture-python /absolute/path`. Each boundary
has a configured 60-second inspection budget. All endpoint traffic stays on loopback and
no provider credentials are configured.

## Inspect a prepared application

Choose a new destination:

```sh
python3 examples/governed-acquisition/prepare.py --destination /tmp/nehemiah-acquisition
target/debug/mas-host build --project /tmp/nehemiah-acquisition
target/debug/mas-host serve-a2a \
  --bundle /tmp/nehemiah-acquisition/.mas/build/bundle.json \
  --state /tmp/nehemiah-acquisition/state \
  --config /tmp/nehemiah-acquisition/gateway.json
```

`mas-governed-local/2` selects the `baseline` authority template through
`baseline_policy_refs`. The collector's registered effect selects
`source-restrictions` through `source_policy_refs`. These are operator-owned
configuration fields. The worker supplies no policy selection or dependency
list. Once source material reaches an invocation, its inherited restrictions
govern later commands and deliveries, including commands with constant inputs.

`GetRetainedSource` accepts `{"id":"<owned A2A task ID>","key":"acquire"}`.
Activate `urn:mas:host-task-service:1` and
`urn:mas:host-governed-source-read:1` in the `A2A-Extensions` header. The service
derives the root actor, invocation and journal from the owned task. Current
`resolve` authority and the source's complete inherited restrictions govern the
projection and its metadata. The earlier `GetRetainedEffect` API remains subject
to those restrictions as well.

These reads observe retained state while leaving recovery and fresh acquisition to the
execution path. Missing or denied data preserves the original admission and recovery
requirements. Discarded originals and exact component-level assessment custody remain
unavailable through this fixture. Separate core tests exercise larger closures,
including a restrictive 33rd ancestor. This example keeps its source graph small. Shared
subscribers, dynamic credential lifecycle, protected deployment, deletion/restore,
adjudication, durable scheduling and provider reconciliation remain separate work.
