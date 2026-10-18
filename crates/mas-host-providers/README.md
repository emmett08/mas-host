# Provider strategies

The crate adapts one admitted host inference effect to a provider. It depends on
neither the MAS store nor the workflow interpreter. `Provider` is a strategy port;
`create` selects maintained adapters; the versioned command port supports other
vendors without changing host execution. The process transport has one deadline
and byte limits, runs no shell, and performs no retries. Run it inside the host's
supervised worker so its guardian also owns descendant cleanup.

The [Nehemiah inference result contract](../../docs/inference-results.md) defines
single-result selection, local validation, retention and replay, with the incident
assessment and formal/implementation verification boundaries.

| Connection kind | Configuration |
| --- | --- |
| `mock` | Optional `output` constant; absent output echoes the JSON input. Explicit `null` returns null. |
| `openai` | Optional complete `endpoint` (defaults to `https://api.openai.com/v1/responses`), `api_key_env` (defaults to `OPENAI_API_KEY`) and `env` array. Uses the Responses API with JSON Schema, no tools and `store:false`. |
| `openai-compatible` | Required complete `endpoint`; optional `api_key_env`, `env`, `structured_outputs` (defaults to true). Uses Chat Completions. Set `structured_outputs:false` for JSON-object mode on providers without JSON Schema support. |
| `cursor` | Required `command` argv array pointing to the maintained Node helper; optional `api_key_env` (defaults to `CURSOR_API_KEY`) and `env`. |
| `command` | Required `command` argv array; optional `api_key_env` and `env`. Implements the extension contract below. |

Only connection-specific fields are accepted. `env` contains explicit environment
variable names rather than values. HTTP credentials travel in curl's stdin
configuration, so keys and prompts do not appear in process arguments or temporary
files. curl reads no user configuration and follows no redirects. HTTPS is required
except for a loopback development endpoint. The host scopes the worker environment;
the provider further scopes the child process to PATH and its explicit allowlist.

`Completed` means the provider returned parseable JSON, pending the host's local
schema validation. `Unusable` preserves a known refusal, incomplete/invalid output
or HTTP rejection. `NotSent` proves a local pre-dispatch failure; `Unknown` requires
reconciliation. HTTP rejection is recorded as `evidence.execution:"rejected"`.
Rejected HTTP responses also retain `http_status` and recognised `error_code`,
`error_type` and `error_param` identifiers. These three fields use fixed allowlists:
for example, `invalid_json_schema` with `text.format.schema`, `invalid_api_key`,
`model_not_found` and `insufficient_quota`. Unknown identifiers, arbitrary schema
property paths, error messages, response headers and stderr are excluded. A 429
status alone does not distinguish quota exhaustion from a rate limit; the retained
code can make that distinction. These diagnostics introduce no retries and do not
change rejection or uncertain-outcome classification. Evidence retains
the selected adapter, requested model, request identity/hash, bounded provider
identifiers and integer token counts. It does not infer monetary cost from tokens.
Cursor exposes no enforceable output-token limit; its evidence states that limit.

For Responses API results, only `message` items whose `phase` is `final_answer`,
missing or `null` can supply the operation result. Explicit `commentary` and other
phases are excluded even when their text is valid JSON. Non-message output items
are not operation results. Each eligible message must contain one `output_text`
block. Every eligible text must parse as the same JSON value. Repeated equivalent
messages therefore produce one result and one downstream decision. Whitespace
and object-key order may differ. Distinct values, a malformed sibling or an empty
eligible message remain unusable. The adapter never joins separate text blocks
into a new JSON value. This is stricter than the SDK helper's first-parsed-output
selection because one host inference effect admits one distinct result.
Refusals in message content and incomplete responses remain unusable. A malformed
final answer never falls back to commentary. The selected text still passes
through the MAS integer-JSON parser, including duplicate-key and decimal-syntax
checks, before the host validates the operation schema.

HTTP output failures retain bounded `output_diagnostics` with fixed `stage` and
`code` identifiers, distinguishing response-envelope, output-selection, output-json
and host output-schema failures. Responses selection records counts named
`eligible_messages`, `eligible_text_blocks`, `excluded_messages` and
`other_output_items`, including when selection succeeds but schema validation
subsequently fails. `final_messages` and `unphased_messages` distinguish eligible
phase categories. Once message structure is valid, `distinct_json_outputs` and
`invalid_json_blocks` distinguish repetition, conflict and malformed text without
retaining their contents. These counts describe parsing, not schema acceptance.
Invalid envelope JSON can retain `response_bytes` and
`response_sha256` for the HTTP response bytes. Invalid selected JSON can retain
`output_bytes` and `output_sha256` for the selected text; host schema failures use
those two fields for canonical serialised JSON instead. No raw output, refusal
text, parser errors or arbitrary phase/status strings enter these diagnostics.
Transport byte limits bound their counts, and these fields introduce no retry or
alternative decision path.

## Command extension contract

Read one JSON document from stdin and write one JSON document to stdout. Exit zero
when an outcome envelope is available; a nonzero exit leaves the outcome unknown.
The request is:

```json
{
  "version": "mas-provider/1",
  "model": "explicit-model-id",
  "instructions": "Return the operation result as JSON",
  "input": {},
  "output_schema": {"type":"object"},
  "max_output_tokens": 4096,
  "request_id": "stable-effect-id",
  "deadline_ms": 1792100000000,
  "api_key_env": "OPTIONAL_CREDENTIAL_ENVIRONMENT_NAME"
}
```

The response is `{"version":"mas-provider/1","status":"completed","output":{},"evidence":{}}`.
An adapter implemented in JavaScript should return `output_json` containing the
raw JSON text instead of parsing it into `output`. The request also contains
`input_json` and `output_schema_json` strings, preserving domain integers through
JavaScript. The Rust adapter validates raw model text with the MAS codec, detecting
duplicate fields, decimal syntax and arbitrary integer values before conversion.
Other status values are `not_sent`, `unusable` and `unknown`; omit `output` when it
is unavailable. `evidence.output_status` can be `invalid`, `refused` or `incomplete`
for an unusable result. `evidence.cleanup` can be `confirmed` or `incomplete`.
Only bounded `id`, `request_id`, `model`, `agent_id`, `run_id` and integer `usage`
fields pass through to the journal. Extensions are trusted integration code and
must report dispatch certainty honestly, enforce their declared token policy and
make no hidden retries. The host independently owns schema checks and recovery.
The generic command port records `token_limit_enforced:false`, since an arbitrary
extension's generation policy cannot be inferred from its command line.

Tests use mock providers, injected Cursor SDK objects, a local HTTP server and
short-lived local commands. They make no live inference calls.

Primary API references: [OpenAI Responses](https://developers.openai.com/api/reference/resources/responses/methods/create),
[OpenAI structured outputs](https://developers.openai.com/api/docs/guides/structured-outputs?api-mode=responses),
and [Cursor TypeScript SDK](https://cursor.com/docs/sdk/typescript), checked 15 October 2026.
Phase selection follows the [OpenAI Python SDK structured-output helper documentation](https://github.com/openai/openai-python/blob/main/helpers.md),
checked 18 October 2026, with the stricter single-result rule described above.
