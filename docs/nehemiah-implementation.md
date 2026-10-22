# Nehemiah implementation sequence

Implementation follows the three plans updated on 22 October 2026. The original
foundation starts from `mas-host@6508042ea86d26bb35106d919a8699d5f1a2e782`.
Review repairs are published in PR #15 at
`7558168f6b05f478773207fa94d3b3988f197cda` and PR #16 at
`f651ddac8a5afabfbb955de805b3ff59b1ecde99`. Their original validation receipts
remain historical evidence for the source revisions they identify. Review
receipts under `evidence/nehemiah-review/` bind the corrected files and the
current plan snapshots.

| Plan increment | Implemented scope | Remaining acceptance work |
|---|---|---|
| Enterprise P0 | Provisional canonical workload manifest, deterministic demand generator and actual-host native/observer smoke | Reference hardware/cost, assignment mapping, paced occupancy, PostgreSQL contention and full workloads |
| Extensibility E0 | Profile preflight, distinct scope identities, conjunctive policy contracts, bounded exact decision bindings and separate protected result dimensions | Stronger profiles remain disabled until their runtime dependencies are qualified |
| Extensibility E1 | Explicitly approved command capability packages, retained implementation/schema/runtime closure, TypeScript and Python structured evidence examples | Managed activation, attestation verification when required, broader package forms and deployment closure qualification |
| PII N0/N1 | Exact-source assessments, deterministic fixtures, pinned local neural adapter and supervised inspection | Labelled deployment-domain quality thresholds and qualification; inspection never grants unstructured release |
| E2/E3 and N2 first structured slice | Authenticated A2A2/core sessions, pre-persistence projection, keyed original-request conflicts, current admission/delivery checks, safe effect completion and replay | Complete N2 coverage, shared acquisition/subscribers, general evidence lineage/resolution, revision integration, dynamic runtime credentials and unstructured handling |
| N2 detector mediation | Optional operator-pinned local inspection at structured ingress, invocation, effect arguments, result/evidence retention and retained payload delivery; exact-source coverage, current processing authority and version-bound receipts | Cross-field/conversational inspection, numeric identifiers, arbitrary text qualification and remaining acquisition/inference/revision paths |
| N2/E3 retained-effect reader | Current-authority, read-only resolution of an exact retained effect, with separate metadata/payload checks and no execution recovery | Complete source ancestry, exposure inheritance, shared subscribers and reader/ancestor qualification |
| N3 first response slice | Trusted native safe-case intent and separately admitted notification through existing effects; bounded memory fixture, HTTPS incident-submission adapter and authorised retained case-status projection | Exact assessment/source resolution, adjudication, durable queue, timed retry/escalation, provider reconciliation and endpoint/deployment qualification |
| Independent P9a | Connection-profile consumption is published in [CSO PR #381](https://github.com/emmett08/catalog-sync-operator-monograph/pull/381), stacked on PR #379, at `65de861aa7c71fae3eaf4dcea825b66e7ebe4d1b` | Consumer merge and the separately selected PR #380 integration; broader dynamic credential/privacy qualification |
| E4/E5 and N4/N5 | Required capabilities stay explicit and unsupported paths reject | Deletion/expiry/restore, worker isolation, consumer and measured deployment qualification |

The first governed path is a single-owner trusted-local implementation. Its
schemas allow closed objects, enumerated strings, bounded integers/arrays,
booleans and null. Arbitrary text remains unavailable. Minimized output can
still be sensitive: all admitted content is classified restricted, and every
applicable policy must explicitly permit inline immutable retention. The
profile does not claim declassification, deletion or isolation from native
callbacks. Managed and shared deployment profiles continue to fail preflight.

## Ownership and design

The core owns scope checks, bounded projection, keyed bindings and mediation in
the existing execution, effect and protocol owners. The CLI owns current
credential lookup, A2A task handling and package approval. The command builder
and retained-resource mechanism own approved package identity; existing command
asset checks revalidate it at dispatch. SDKs return the host-approved effect
projection on both fresh completion and replay.

The privacy crate and pinned local Python adapter retain source/coverage and
inspection responsibilities. The new structured gate does not interpret neural
success as permission to release arbitrary text. The workload harness remains
separate from capacity claims.

The detector integration reuses that adapter through the existing guardian. A
separate processing permission precedes each inspection boundary. Exact JSON
keys and decoded string values are checked as individual UTF-8 sources. A
complete positive finding in removed input can coexist with an approved clean
structured projection. Incomplete inspection and findings in retained components
stop admission or withhold the completed result. This component extraction does
not establish coverage of cross-field narratives or numeric identifiers.

The response example derives a generic handling-withheld event from a retained
host marker. Its safe case intent precedes the notification effect. Opaque case
and action identities derive from the host invocation, event and fixed route.
The notification descriptor also binds the configured destination contract.
Credential rotation retains that destination identity. Endpoint acceptance,
known refusal, uncertain submission and recipient delivery remain distinct.
The example supplies one event and one bounded notification attempt per
invocation. Its case-status consumer now reads the original intent and
notification records through the host's authorised retained-effect reader.
Full N3 lifecycle and scheduling remain subsequent work.

The retained-effect reader extends the existing read-only observation pattern.
It selects an exact governed namespace, parent invocation, actor and logical key,
then checks current rights before delivering a bounded record. It preserves
completion and uncertainty without opening an execution engine. The A2A service
derives the run and root invocation from an authenticated, owned task. This first
resolver increment supplies no source-lineage or shared-reader qualification.

The selected design adds explicit A2A2 and `Engine::open_governed` entry points.
Existing local APIs and configuration versions retain their behavior. A
namespace cannot switch between ordinary and governed modes, and ordinary
inspect/observe/resolve/revision paths reject governed state. New receipt and
original-binding tables live in the existing journal; no second effect engine,
scheduler, notification owner or general plugin runtime is introduced.

Separate authenticated transport identity from purpose/grant binding so a grant
change cannot reset a request's replay identity. A fixed private commitment key
binds original content without retaining predictable plain hashes. Current
credential and policy snapshots are reread at admission and delivery. Changing
an epoch stops an existing session; a newly authenticated session with the same
policy can retrieve the same retained task. Policy, key and namespace migration
remain explicit later work. The file-backed profile makes no distributed
linearisation claim and cannot recall an already admitted action.

Implementation follows the repository's Rust 2021 edition, selected toolchain,
formatter and strict Clippy settings. Python helpers use the existing Python
3.11+ baseline. The example's TypeScript compiler has its own lockfile. Dependency
builds use an environment without deployment credentials.

## Validation and limits

The review repairs are tested independently on their published PR bases. New
integration tests exercise the actual HTTP service, current-authority failures,
credential rotation, cross-tenant requests, original-input conflicts, removal of
private fixture fields before persistence, known-completed withheld output and
restart without redispatch. Core tests cover the same contracts at journal and
protocol boundaries. SDK/runtime conformance preserves completion truth and
validates complete native response framing.

The E1 tests exercise real collector execution and reject changed declarations,
command kinds, runners, argv, files, runtimes, schemas and aggregate bundles.
TypeScript and Python implement the same identified synthetic Kubernetes
observation. Full workspace and native-acceptance checks remain the regression
gate; each retained validation receipt states exactly which commands ran.

No population PII error rate, production confidentiality, reference hardware
capacity, deletion guarantee or shared takeover result follows from these tests.
N1 actual-model receipts remain bound to their original implementation; model
qualification is not silently transferred to later source revisions.

The new [governed control model](../formal/governed-local.md) composes snapshot
authority checks, exact-source inspection, completion withholding and recovery.
The separate [privacy response model](../formal/privacy-response.md) checks safe
intent ordering and notification effect outcomes. Both use exhaustive finite
SPIN searches, mechanism mutations and replayed reachability witnesses. The
existing models remain regression checks. Model results and implementation
conformance have separate source-bound receipts under
`evidence/nehemiah-privacy-mediation/`. These finite models abstract parsing,
cryptography and detector behaviour, and establish no implementation refinement,
population detector quality or distributed revocation linearisation theorem.

The [retained-effect model](../formal/retained-effect.md) checks the new reader's
current lookup/delivery admission, coherent read-only snapshot, binding and
completion-preserving withholding rules. Its implementation and formal receipts
are kept separately under `evidence/nehemiah-governed-reader/`. Case-status
projection consumes independent effect snapshots and preserves that limitation.

## Next dependent increments

1. Complete N2/E2 across the remaining supported acquisition, inference,
   revision, evidence and diagnostic paths, with version-bound unstructured
   inspection and declared handling outcomes. Keep unsupported paths closed.
2. Extend E3's retained-effect resolution with complete source ancestry,
   source-exposure inheritance and shared subscriber delivery; qualify
   revocation and resolver-failure cases, including
   more than 32 consumers/ancestors. Complete dynamic credential lifecycle before
   protected sharing.
3. Extend N3's case-status reader with authorised exact assessment/source
   resolution, adjudication, a durable queue, permitted retry/escalation and
   provider reconciliation. A lost
   acknowledgement currently remains uncertain under its original action
   identity and prevents redispatch.
4. Qualify E4 deletion/expiry/restore and E5 worker isolation before managed
   release. Shared authority/scheduling remain P5/P6 dependencies.
5. Complete P0 on an identified reference deployment before P1 performance
   claims. Preserve P9a's independently selected consumer base and producer pin
   until its qualified consumer migration.

The CSO application's host pin and integration base remain separate decisions.
The current P9a review matched all twelve implementation/evidence files to
PR #381's remote blobs. Its source-bound historical receipt records 72 passes,
one Vue dependency skip and ten passing connection-profile cases. That review
did not rerun those tests or transfer their evidence to the new governed path.
This increment does not change consumer branches or release state. No production
notifications, provider calls, deployments or manual workflow dispatches form
part of local validation.

See [capability packaging](capabilities.md), [governed local usage](governed-local.md),
[detector mediation](governed-privacy.md),
[contract preflight](nehemiah-governance.md), [local detector usage](privacy-detector.md)
[retained effect reading](governed-evidence-reader.md) and
[workload usage](workload-baseline.md) for executable entry points.
