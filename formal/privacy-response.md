# Privacy response adapter model

`privacy-response.pml` checks whether the first bounded N3 native adapter records safe
intent before notification and preserves known or uncertain outcomes during replay. The
adapter lives in `adapters/notifications/privacy_response/`. Run `python3
formal/verify.py --model privacy-response` for this model or the default command for the
complete host suite.

The service retains a safe case-event intent through `ctx.effect`, requires its
exact host-approved acknowledgement, and separately admits one notification
effect. The trusted route is selected from privacy-policy, suspected-secret and
coverage-failure reason classes. Model event identity denotes the host task,
event version, safe facts and route. A changed event under an existing logical
effect identity conflicts. Another tenant cannot reuse the action.
Both effect descriptors also bind the exact configured destination and delivery
contract. Changing endpoint, route, timeout or acceptance contract conflicts with
the retained intent. Rotating only its bearer credential preserves that binding.
The two destination classes compare retained intent with a later explicit
attempt. Implementation tests separately check a configuration change between
notification preparation and network dispatch.

The finite environment contains one event, one intent effect, one notification effect,
two event-body classes, two destination classes, two tenants, three configured route
classes, one optional authority revocation, one optional crash/restart and one explicit
retry. Endpoint request and response channels each have capacity one. The endpoint
interaction has five outcome classes: accepted, refused, throttled, not-sent and unknown
transport outcome. Work is assumed to return or reach its enforced deadline. The model
uses no real-time clock or fairness switch. An explicit retry interrupted by the
independent crash can traverse cached completion twice. Restart preserves that retry's
event, tenant and destination checks. Each external effect is still dispatched at most
once.

| Property | Model assertion or temporal claim | Implementation correspondence |
| --- | --- | --- |
| Durable intent precedes notification | A notification call requires a retained intent and its exact approved output | `handler.respond`: `privacy-case-intent` effect and acknowledgement check before `privacy-notification` |
| Trusted route | Route equals the route selected by the reason policy | `contracts.CaseEvent` validation and notification construction from configured route rules |
| Current authority | Notification admission performs its own rights check after the intent | Each `Context.effect` delegates to the host's current `begin_effect` checks |
| Separate certainty dimensions | Endpoint acceptance keeps recipient delivery unknown | `notifier.HttpsIncidentSink.submit` receipt mapping |
| Throttling stays refusal | A retained throttled response cannot become acceptance or a fresh automatic dispatch | Explicit 429 handling and absence of automatic retry in the adapter |
| Completed replay | At most one call for each effect, with completed and withheld notification receipts retained | Existing host effect identity/checkpoint reuse and SDK completion acknowledgement |
| Unknown execution | Interrupted executing effects remain unknown and block another dispatch | Existing host `executing` recovery and `EffectUncertain` handling |
| Exact action identity | Event and route bindings remain immutable under a duplicate logical identity | Safe event in both effect descriptors, opaque action identity and host original-input conflict check |
| Exact destination | The retained intent's destination binding remains immutable | Both effect descriptors retain `sink.configuration_binding()` independently of rotating bearer credentials |
| Finite settlement | `privacy_response_settles`: eventually delivered-to-endpoint settlement, denial, held intent or unknown blockage | Finite returning-work control abstraction |

Each atomic block is a local computation or a host-journal transition. Intent
executing, callback execution, intent completion, notification admission,
notification executing, actual submission, response observation and completion
are separate transitions. A crash after an executing permission and before
actual dispatch remains conservatively unknown. A completed intent can be reused
after a crash before notification admission. Revocation denies a later admission.
Authority can change after the read and before dispatch under the documented
file-snapshot contract.

The model abstracts each host receipt as one durable commit. The separate
`governed-local.pml` checks receipt/step atomicity, withheld payload handling and repair
of the protocol database. This correspondence separates the adapter's ordering
obligation from the shared host durability mechanism. A formal cross-model refinement
proof and a complete notification-delivery theorem require separate work. The interval
between receipt persistence and a worker's ACK observation remains a host recovery
concern covered by the governed model and native integration tests.

Safety and `privacy_response_settles` are checked with and without partial-order
reduction. Deliberate mechanism defects bypass intent ordering or withheld-intent
checks, choose an untrusted route, reuse revoked authority, redispatch unknown,
completed, throttled or withheld work, promote throttling to acceptance, infer
recipient delivery from endpoint acceptance, or rebind a changed event or
destination. A replay-identity mutation bypasses the original identity checks
after a crash and must fail the settlement-time identity assertion. A
permanent endpoint-stall mutation must produce a replayed acceptance cycle.

Inverted assertions establish reachability of these paths:

- Accepted, refused, throttled and not-sent receipts, together with unknown execution.
- Restart between the two effects and cached replay.
- Completed notification with withheld payload, and withheld intent preventing notification.
- Revocation, changed-event or changed-destination refusal, and foreign-tenant refusal.
- An explicit duplicate of unknown work remaining blocked.

All expected counterexamples are replayed and retained by the existing runner. Positive
searches must finish with zero errors, and depth, time or memory exhaustion remains
inconclusive.

The model assumes the bounded safe-event parser, current scoped credentials, approved
endpoint contract and host/SDK effect mechanisms. Exact Python types, HTTP encoding,
redirect rejection, response-body limits, runtime bounds and actual configuration are
checked by implementation tests. Arbitrary user content and detector accuracy require
separate models or empirical evaluation.

Unknown submissions retain their original identities for a later, separately qualified
reconciliation service. A reconciliation endpoint, retry or escalation scheduler,
shared-owner case database and end-to-end recipient-delivery guarantee each require
their own implementation and evidence. These finite checks run entirely within the
model.
