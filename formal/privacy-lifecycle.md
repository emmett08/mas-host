# Bounded privacy lifecycle model

`privacy-lifecycle.pml` models one durable intent, up to two notification
attempts under one logical action, retained ordered timers, authoritative
same-action reconciliation, current admission, acknowledgement, separately
identified escalation and immutable source-bound review without release power.

The verification harness runs safety and progress searches with and without
partial-order reduction. Eight mechanism mutations challenge retry uncertainty,
replay, stale authority, premature retry, acknowledgement promotion, escalation
identity and review binding/release separation. Three replayed reachability
witnesses establish accepted, escalated and unknown paths.

The model abstracts SQLite transactions, HMACs, source parsing, Python SDK
framing, the real clock and provider contracts. Implementation conformance tests
remain necessary. The model supplies no detector-quality, production delivery,
shared scheduling or cryptographic refinement theorem. The existing first-slice
`privacy-response` model remains an independent regression contract.
