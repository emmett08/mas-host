# Governance contract preflight

Use profile preflight to check whether the host implements the capabilities a deployment
requires before attempting to configure it. The command performs a bounded local
compatibility check using read-only inputs. Execution journals and providers remain
outside this check.

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

`managed-cluster` adds mandatory scope, ingress handling, disclosure, payload lifecycle
and effect-output requirements. `shared-enterprise` also requires shared authority,
fenced scheduling and shared quotas. These profiles currently fail preflight with the
missing capabilities. The requirement file declares what the deployment needs. The
host's implemented capability set determines whether that requirement is met. An
isolation requirement also fails until its runtime boundary has been implemented and
qualified.

Unknown versions, fields, capability names, duplicate requirements and malformed values
fail validation. The profile is a separate `mas-host-profile/1` contract. Execution
still uses the existing agent and bundle schemas, with credentials and authority
supplied through their own configured boundaries.

## Data-use contract

`mas_host_core::governance` owns the first versioned scope, policy and protected
result contracts. Tenant, principal, actor, producer, destination and evidence
identities remain separate types. A matching actor name in two tenants supplies
no cross-tenant permission. Wildcard actor readership cannot widen these types.

Disclosure checks preserve the relation between an allowed operation, purpose,
destination and recipient. A purpose allowed for one destination therefore cannot be
combined with a different permitted destination to invent a wider grant. Applicable
source policies instead accumulate through conjunction. Unavailable current authority,
revoked grants, missing obligations and unsupported records prevent a permit.

With `binding_complete: true`, held and permitted decisions retain the exact
proposed operation, source/output binding references, recipient, scope and
supplied governance references. Held
decisions carry `authority: "none"` and zero admission expiry. The supplied scope
and requested exposure sets remain separate when they disagree. These records
contain governed metadata and require an authorised storage and disclosure path.

The assembled decision has a 65,536-byte limit. If the reference closure exceeds that
limit, the result is `hold` with `reason: "decision-binding-limit"` and
`binding_complete: false`. It retains the bounded request identity and omits the
exposure/policy arrays. This outcome cannot be interpreted as a complete empty exposure
or used as a disclosure permit. `binding_complete` describes retained reference
coverage. Authenticating the supplied facts remains a separate integration obligation.

These deterministic functions take authenticated scope and current authority
from trusted integration code. The [governed local path](governed-local.md) now
wires those checks into a bounded structured execution slice, with current grant
checks and keyed original-content bindings. Complete N2 mediation remains pending.
Existing workers and callbacks retain the trust boundary documented in
[the architecture](architecture.md).

Read the protected result view by the question each field answers. Task state describes
orchestration, host settlement records the durable checkpoint, and external certainty
records what is known about the action. Payload availability governs delivery. Evidence
freshness and coverage bound the observations available to an application assessment. A
known external completion can have a withheld or deleted payload. A completed host
settlement can accompany an uncertain external result. Another execution attempt still
requires its separate admission and recovery contract.

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
contains safe classifications and handling dispositions. The neural detector remains
unqualified, so the fixture cannot authorise protected unstructured disclosure. N1 local
model packaging and supervised inspection are available in the [local detector
guide](privacy-detector.md). The [implementation sequence](nehemiah-implementation.md)
records the implemented N2–N3 slices and the remaining persistence, retrieval,
case-lifecycle and notification requirements. A closed structured subset is available
through [governed local execution](governed-local.md).
