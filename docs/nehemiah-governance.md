# Governance contract preflight

The host can now check a deployment's required capabilities before an operator
attempts to configure it. The check is local, bounded and read-only. It opens no
execution journal and contacts no provider.

```sh
mas-host check-profile --profile profile.json
```

For the existing trusted local execution profile, use:

```json
{
  "version": "mas-host-profile/1",
  "profile": "trusted-local",
  "required_capabilities": [
    "trusted-local-execution",
    "local-effect-replay"
  ],
  "worker_isolation": false
}
```

Success reports the required and implemented capabilities. `activation: "none"`
means the command has only checked compatibility. Existing `run`, `build`, A2A,
SDK and bundle contracts retain their trusted local execution behaviour.

`managed-cluster` adds mandatory scope, ingress handling, disclosure, payload
lifecycle and effect-output requirements. `shared-enterprise` also requires
shared authority, fenced scheduling and shared quotas. These profiles currently
fail preflight with the missing capabilities. Requirements supplied in the file
cannot declare implementation support into existence. An isolation requirement
also fails until its runtime boundary has been implemented and qualified.

Unknown versions, fields, capability names, duplicate requirements and malformed
values fail validation. The profile is a separate `mas-host-profile/1` contract.
It grants no credentials or authority and is not an activation field in existing
agent or bundle schemas.

## Data-use contract

`mas_host_core::governance` owns the first versioned scope, policy and protected
result contracts. Tenant, principal, actor, producer, destination and evidence
identities remain separate types. A matching actor name in two tenants supplies
no cross-tenant permission. Wildcard actor readership cannot widen these types.

Disclosure checks preserve each allowed operation, purpose, destination and
recipient relation as a whole. Combining separately permitted purposes and
destinations would create additional permissions. Applicable source policies
instead accumulate through conjunction. Unavailable current authority, revoked
grants, missing obligations and unsupported records prevent a permit.

With `binding_complete: true`, held and permitted decisions retain the exact
proposed operation, source/output binding references, recipient, scope and
supplied governance references. Held
decisions carry `authority: "none"` and zero admission expiry. The supplied scope
and requested exposure sets remain separate when they disagree. These records
contain governed metadata and require an authorised storage and disclosure path.

The assembled decision has a 65,536-byte limit. If the reference closure exceeds
that limit, the result is `hold` with `reason: "decision-binding-limit"` and
`binding_complete: false`. It retains the bounded request identity and omits the
exposure/policy arrays. This outcome cannot be interpreted as a complete empty
exposure or used as a disclosure permit. `binding_complete` describes retained
reference coverage; it authenticates none of the supplied facts.

These deterministic functions take authenticated scope and current authority
from trusted integration code. Runtime mediation, grant rotation and protected
commitment storage belong to the subsequent governed execution increment.
Existing workers and callbacks retain the trust boundary documented in
[the architecture](architecture.md).

The protected result view keeps task state, host settlement, external execution
certainty, payload availability, evidence freshness and coverage, and application
assessment separate. A known external completion can have a withheld or deleted
payload. A completed host settlement can accompany an uncertain external result.
Neither representation grants another execution attempt.

## Privacy fixtures

The `mas-host-privacy` crate validates assessments against the exact in-memory
UTF-8 source. Ranges use byte offsets and must fall on character boundaries.
Coverage and positive findings are independent: a partial scan can retain a valid
finding, while a complete pattern scan establishes only its declared coverage.

```sh
cargo test --locked -p mas-host-privacy
cargo run --locked -p mas-host-privacy --example synthetic
```

The example uses synthetic text and a deterministic fixture detector. Its output
contains safe classifications and handling dispositions. The neural detector
remains unqualified, so the fixture cannot authorise protected unstructured
disclosure. Model packaging, complete host persistence and retrieval mediation,
durable privacy cases and notification delivery follow N1–N3 in the PII plan.

See [implementation status](nehemiah-implementation.md) for dependencies and the
evidence boundary of this increment.
