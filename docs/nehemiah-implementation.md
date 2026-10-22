# Nehemiah implementation sequence

This increment begins the three implementation plans prepared on 22 October
2026. Its base is `mas-host@6508042ea86d26bb35106d919a8699d5f1a2e782`, with the
existing protocol pin and consumer execution contracts preserved.

| Plan | First implemented slice | Remaining acceptance work |
|---|---|---|
| Enterprise scaling, P0 | Canonical workload manifest, bounded deterministic demand generator, actual-host native and observer smoke runner | Freeze reference hardware and cost, establish assignment mapping, measure paced occupancy and PostgreSQL contention, run full workloads |
| Extensibility and governed data, E0 | Strict profile preflight, distinct scope identities, conjunctive policy/evidence records, separate protected result dimensions and conformance fixtures | Wire contracts into authenticated ingress and every persistence/disclosure boundary under E2/E3 |
| PII detection and response, N0 | Exact-source UTF-8 assessment contracts, independent findings and coverage, bounded synthetic recognisers, conservative decision fixtures | Freeze deployment-specific quality thresholds, package and qualify the neural detector under N1, then mediate the host path under N2 |

P0 remains partial. The synthetic detector supplies a deterministic contract
fixture. Model quality, multi-tenant enforcement, lifecycle guarantees and
enterprise capacity each need their own implementation and runtime evidence.
Managed and shared profiles fail preflight while those capabilities are missing.

## Ownership and design

The core owns governance value semantics and policy contract checks. The CLI
owns profile loading and compatibility reporting. The privacy crate owns source
binding, offset/coverage validation and synthetic detector fixtures. The
workload harness consumes the existing host executable and compiler fixtures.
Existing journal, protocol and effect authorities retain their responsibilities.

The selected design adds explicit versioned contracts beside existing execution
APIs. Directly extending current agent/bundle schemas would require a migration
and could suggest that protected execution was already available. A general
plugin runtime or persistence framework would add lifecycle obligations before
the first governed path exists. Both alternatives would increase this
increment's change surface.

The governing implementation profile is Rust 2021 with Rust 1.99.0 and the
repository's existing formatter and Clippy settings. Python workload utilities
use the standard library and the existing Python 3.11+ baseline. This increment
adds no external Python package, model asset or provider dependency.

The core policy evaluator accepts trusted current-authority facts for the exact
scope. It does not authenticate arbitrary input. Its conditional decision keeps
the current admission obligation visible. Runtime admission remains E2 work.
Legacy revision reader strings and public handler return shapes retain their
existing meaning.

## Next dependent increments

1. Complete P0's baseline using an identified reference deployment. P1's read
   transaction candidate needs that contention baseline before a performance
   claim. P9a's application connection profile can proceed on its specified
   consumer base independently.
2. Implement E1 capability packaging and E2/P2 authenticated scope, handling and
   grant rotation. Preserve the original request conflict relation when inputs
   are minimised. Inspect before the first journal write.
3. Package the pinned local neural detector under N1, then integrate N2 with
   E2/E3. Preserve known completion and accounting when output is withheld.
4. Add N3 safe cases and notification operations through the existing effect
   owner. Reconcile uncertain delivery under the original action identity.
5. Qualify governed payload lifecycle and deployment under E4/E5 before managed
   release. Shared authority and scheduling remain P5/P6 dependencies.

The CSO application PRs and their distinct host pins require a separately selected
integration base. This increment changes neither their branches nor their
release state. Provider calls, production notifications, deployments and manual
workflow dispatches are outside this foundation validation.

See [governance usage](nehemiah-governance.md) and
[workload usage](workload-baseline.md) for executable entry points. Validation
records identify performed checks separately from the remaining qualification.
