# Nehemiah integration and qualification

This change continues the three implementation plans from 22 October 2026
after governed acquisition in PR #20. The protocol producer is
[mas PR #55](https://github.com/emmett08/mas/pull/55); the host integration is
[mas-host PR #24](https://github.com/emmett08/mas-host/pull/24). The selected
CSO consumer builds on its existing acquisition branch. Existing validation
receipts remain evidence for their identified revisions, rather than evidence
for every later integration.

The supported profiles have different capabilities. Trusted-local structured
governance uses the existing SQLite execution owner. Optional ordinary shared
native execution uses the existing PostgreSQL task, invocation and accounting
owners. A protected shared service must not be enabled by combining those two
profiles: shared governance and recovery parity require their own acceptance.

| Plan area | Implementation in this integration | Acceptance boundary |
| --- | --- | --- |
| P0 / P7 workload evidence | Paced demand, independent source identities, occupancy observations, adversarial history checks and bounded ordinary-local metadata trigger captures | Reference hardware, cost ceilings, investigation-to-assignment mapping and full V/L/S/A/A+/E/E+ runs remain unqualified |
| P1 observational queries | Explicit read-only repeatable-read query allowlist, including exact owned shared task status | Real PostgreSQL checks establish writer independence for the tested queries; no throughput claim follows |
| P2 / E2 authenticated handling | Existing current scope mediation, protected structured revision, current credential reference resolution and monotone credential epochs | File-backed authorisation remains a current local snapshot; it provides no distributed revocation or worker-confinement claim |
| P3 bounded status | Host-materialised status contains no application values and does not trigger acquisition or worker dispatch | A status snapshot does not establish independent external completion; protected/governed/shared trigger capture remains unsupported |
| P4 / E3 shared acquisition | Exact registered read identities use private keyed commitments; original A2A request identity survives lost replies. Immutable publication and independent current-authorised subscriber reads share the acquired source | The application retains its immutable registration/checkpoint and each subscription's lifecycle. Broader incident grouping and continuous watches require their own contracts |
| P5a / P5b shared authority | Existing local recovery fixtures, typed native fresh/retained/rejected/uncertain outcomes, non-cloneable consuming dispatch permission, authoritative deadlines | A retained admission receipt never supplies fresh permission. Composition profiles outside the qualified slice remain refused |
| P5c / P6b shared host | Credential-free native source functions, authenticated shared A2A submit/read/cancel and bounded paged queue turns use the existing PostgreSQL authority | Shared protected effects, full handler/revision parity, automatic takeover and deployment availability remain separate gates |
| P6a queue bounds | Versioned scheduler limits, immutable workflow input/work/cost estimates, bounded record scans and expected-revision amendments preserve unresolved native debt when limits are lowered | Queue input-copy budgets do not cover every physical receipt/artifact byte. Configured maxima are not measured capacity or a population-independent fairness guarantee |
| P6c provider allowance | Existing provider circuit/permit rows retain fixed-window credit and monotone cooldown. A registered workflow's provider policy participates in its native admission transaction | The first worker profile is an immutable registered credential-free GET with bounded approved scalar projections, no automatic HTTP retry and retained invalid-hint holds. Mutation retries, authenticated provider profiles, verified webhook ingestion and complete list/watch reconciliation are not implied |
| E0 / E1 capability contracts | Closed versioned profiles and approved packages retain implementation, schema and runtime bindings through the existing builder | Byte identity does not establish publisher trust, runtime attestation or isolation |
| E3 larger source histories | Opt-in referenced decisions resolve sealed 65/128-source fixtures and preserve original policy conjunctions. Authorised lineage pages obey both count and encoded-byte bounds | Follow returned continuations. Compact and expanded representations can have different actual byte counts; equivalence assumes a fixed disclosure request |
| E4 retention and lifecycle | AEAD-encrypted exact-source/failed-assessment quarantine, separately encrypted reviews, independent key custody, bounded discovery, expiry, holds and retirement preserve unresolved external recovery obligations | Logical access retirement is distinct from physical erasure and deletion of every external copy |
| E5 / P8 deployment | Hardened managed placement renderer and executable qualification commands retain explicit ownership and supported-profile requirements | Images, CNI, volumes, hostile-worker isolation, shared autoscaling, RPO/RTO and external fencing need deployment evidence |
| N0 / N1 detector qualification | Source-bound quality harness, incomplete-state handling and pinned synthetic/local detector contracts | Population error rates require an accepted labelled deployment-domain corpus and thresholds |
| N2 contextual inspection | Opt-in canonical JSON inspection includes numeric and cross-field context; component assessments retain exact source bindings under distinct permission | Complete model output does not authorise arbitrary text release or establish cross-source conversational coverage |
| N3 response lifecycle | Existing effects retain notification intents, retries, age/attempt bounds, reconciliation, acknowledgement deadlines and fixed-template escalation. Assessment review requires separate current rights and cannot change release permission | Endpoint acceptance, recipient delivery, external execution and application outcome remain separate facts |
| P9 / N4 consumer integration | The selected CSO acquisition path consumes host-approved source projections before evidence sealing and preserves native/EAL uncertainty | Ordinary connection-profile work and the consumer's target/action restrictions retain their existing ownership |
| E6 / N5 / P10 release evidence | Actual-host fixtures, strict analysis, real PostgreSQL tests and finite SPIN checks have separate revision-bound results | Finite models are not implementation refinement proofs. Live detector, confidentiality, capacity, deletion and failover qualification must precede those claims |

Unsupported capabilities fail preflight. Domain quality or deployment evidence
must not be replaced by a synthetic fixture, an ignored test, a healthy process
heartbeat or a declaration in configuration. The staged implementation does not
make the three programmes fully accepted.
