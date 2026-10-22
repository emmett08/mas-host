# Synthetic queued privacy response

This example extends the existing privacy-response preparation code. It runs
queue intent, bounded timers, attempts, exact-action reconciliation,
acknowledgement and escalation through the actual governed host's existing
journal and effect permissions.

See [the contract and commands](../../docs/privacy-lifecycle.md). The six
scenarios exercise throttled retry, reconciled acceptance, authoritative absence,
unresolved uncertainty, recipient acknowledgement and timed escalation. Each
restarts the service and checks that replay causes no new submission.

All endpoint behaviour is synthetic. No notification service, real detector
model or production recipient is contacted by the conformance verifier.
