# Nehemiah inference result contract

One admitted `mas-inference/2` effect may produce at most one usable decision. Repeated
provider messages must agree on that decision before the host can validate it and admit
any peer request. This rule refines result selection within the existing inference
contract. Nehemiah wire fields, message kinds, retry permissions and profile negotiation
retain their existing definitions. `mas-provider/1`, `mas-agents/3` and the protocol
crate pin remain compatible. The OpenAI-specific binding lives in the provider adapter.

## Incident assessment

| Evidence | Finding | Mitigation and verification boundary |
| --- | --- | --- |
| Examples run [39140672322](https://github.com/emmett08/mas-host-examples/actions/runs/39140672322/job/117716230830) | After one MCP tool returned, the next HTTP 200 response contained two eligible messages and two text blocks. The previous selector rejected multiplicity before parsing. Neither text was retained. | [Host PR #13](https://github.com/emmett08/mas-host/pull/13), merged as `12073ac3a33f5a951390bcd628e4532b60604d25`, admits repeated equivalent results once. Conflicting results still fail. The original incident cannot be classified as equivalent or conflicting from those counts. |
| Examples run [39143210107](https://github.com/emmett08/mas-host-examples/actions/runs/39143210107/job/117721783915) | The HTTP/MCP parent test reached 600,000 ms after twelve scenarios passed. Individual scenarios took approximately 38–65 seconds with the debug host. | Examples conformance CI builds the pinned host in release mode, as the k3s image already does. Keep the same scenarios, integrity checks and time limit. Measure execution separately from model checking. |
| Required-check job [117725358832](https://github.com/emmett08/mas-host-examples/actions/runs/39143210107/job/117725358832) | Applications and incident checks passed. Conformance failed. | The aggregate gate correctly propagated that failure. It needs no weaker success rule. |
| Related delegation and replay cases | Premature completion, repeated required peers, malformed tool arguments, invalid decisions and lost observations can otherwise cause missing evidence or repeated work. | Existing runtime admission, frozen schemas, effect identities and durable recovery remain authoritative. HTTP/MCP tests and the delegation, revision and provider-result models cover their named boundaries. |

The failure belongs at the provider boundary, where result selection can be corrected
and regression-tested before any downstream request reaches the mailbox. The protocol
repository's [operation-outcome
contract](https://github.com/emmett08/mas/blob/7bbd1301ed651e0994390ac04b5c340a700fdfcd/formal/operation-outcomes-notes.md)
already assigns external execution classification to adapters. Its immutable
identity/content and unknown-outcome rules remain applicable. Updating to a different
protocol crate revision is unnecessary for these changes.

## Service, representation and exchange rules

The service turns one bounded provider observation into a retained usable result, a
known unusable outcome, a definitely unsent outcome, or unresolved execution.
Provider-return certainty, output validity and cleanup status are separate facts.
Establishing the truth of the answer requires application evidence beyond those
transport and validation checks.

For the Responses API binding:

1. Apply response status and refusal checks. Only `message` items with absent,
   null or `final_answer` phase are eligible. Explicit non-final phases cannot
   become results because their contents happen to parse.
2. Require at least one eligible message and exactly one `output_text` block in
   each eligible message. Do not join blocks, choose the first or last decision,
   or discard a malformed sibling. A refusal in message content remains unusable.
3. Parse **every** eligible text using the Nehemiah integer-JSON codec. Duplicate
   keys, decimal syntax, trailing text and invalid syntax reject the observation.
4. Compare full canonical encodings of the parsed values. All must be equal.
   Object-key order and insignificant whitespace may differ. Equality preserves
exact integers, array order and the distinction between Boolean, string and
number types.
   JSON string escapes decode normally. An `input_json` value is still a string:
   `"{}"` and `"{ }"` differ because the selector compares their string contents.
5. Return the sole distinct value once, pending the host's schema validation.
   Equivalent repetition is neither a quorum nor additional evidence. A schema
   mismatch, premature completion or unpermitted peer still forbids dispatch.

The surrounding vendor HTTP envelope uses the adapter's JSON parser. The exact
integer-JSON checks apply to the selected model texts. The surrounding envelope is
handled by the adapter's JSON parser, with complete vendor-schema validation requiring a
separate check.

The host constructs the current decision schema from the frozen configuration,
completed peer history, required peers and remaining turns. Peer input must also
parse as an object and satisfy the target operation's admission checks. A valid
model decision cannot expand topology, budgets, deadlines or tool authority.

## Retention and recovery

`inference-{turn}` is a stable effect key within its invocation. Its descriptor
binds configuration and input/history. Reusing that identity with a different
descriptor rejects. Provider execution, durable outcome retention, peer request
admission and downstream execution are separate boundaries.

Known refusal, incomplete output, ambiguous output and schema-invalid output are
retained as known outcomes. They stop the invocation without an automatic provider
retry. `NotSent` is retained distinctly. A missing or lost observation is `Unknown` and
requires reconciliation. Absence and timeout preserve that requirement under the
original effect identity. A crash after a response but before its durable commit can
therefore leave uncertainty even if the response was valid.

Re-execution may repeat pure host/handler computation. Completed inference effects and
peer invocations reuse their retained results, keeping provider and tool dispatch counts
unchanged on replay. The same effect can remain unresolved after restart. Distinct turns
have distinct identities: equivalent requests made on different turns are not globally
deduplicated. Required-peer coverage is a separate policy. The recovery guarantee is
scoped to these retained effect identities. Globally exactly-once execution, recovery
from permanent faults and successful model completion require additional contracts and
evidence.

Diagnostics retain bounded phase, candidate, distinct-value and malformed-text
counts, plus fixed failure codes. They do not retain provider text. Their absence
in older records cannot be repaired by guessing what the provider returned.

## Verification and correspondence

Run `python3 formal/verify.py`. The [formal specification and
bounds](../formal/README.md) distinguish the new provider-result model from the existing
delegation, revision and disclosure models. Safety and settlement are separate
full-state searches, each repeated without partial-order reduction. Deliberate defects
test whether the checks detect the intended failures. Reachability witnesses establish
that the represented paths can occur. Logs, hashes and replayable trails retain the
results of both. The [retained verification
receipt](../evidence/provider-results/README.md) records the complete 86-check host run
and independent review of the new model.

| Contract boundary | Implementation and executable evidence |
| --- | --- |
| Phase eligibility, whole-message selection and complete equivalence | `crates/mas-host-providers/src/http.rs`, with `responses_accept_only_one_distinct_json_value_across_complete_messages`, `responses_equivalence_preserves_exact_json_types_and_string_contents`, conflict/malformed/refusal/phase tests |
| Known invalid versus unresolved observations | `runtime/inference.rs::classify` and provider `Outcome`, exercised by the runtime invalid-completion test and HTTP transport tests |
| Schema before decision, required peer coverage and bounded turns | `runtime/inference.rs::execute` and `action_schema`, covered by the delegation model and examples HTTP/MCP cases for early completion, partial coverage, duplicate peers and malformed arguments |
| Stable effect identity and retained results | Core `Inner::prepare`, `begin_effect`/`complete_effect`, SDK `Context::effect` and invocation/effect journals, exercised by `effect_identity_and_evidence_fail_closed`, `scoped_inference_unknown_effect_remains_blocked_after_restart` and SDK cache tests |
| No additional external work on replay | Examples `investigation.integration.ts`: repeated valid output completes with five provider calls and four distinct tool calls. Rejected decisions retain their failure. Replay adds no calls |
| Conflict after a completed tool | The same integration suite retains the first tool result, rejects the next conflicting decision before another tool, and repeats no provider/tool calls on replay |

The model abstracts JSON values and schemas into finite classes. Exact parsing
and implementation correspondence are exercised by tests, not proved by SPIN.
It assumes trusted host execution, durable atomic journal commits and external
calls that return or reach their enforced deadline. It does not prove physical
timing, cryptographic security, arbitrary participant counts, model quality or
that every implementation execution refines the model. A fresh authorised live
run using the updated host is still needed to establish the original deployment
outcome.
