# Workflow composition

`mas-host-workflow` composes actor requests into recursive plans. Its interpreter finds
ready requests and passes them through the SDK adapter to the same host admission path
used by application code. Plans determine execution order, while topology determines who
may communicate. Star, tree and mesh applications therefore use the same nodes with
different permitted edges.

`validate(plan, agents)` checks the complete plan, including unused definitions and
unselected branches. `execute(context, plan, input)` executes it. `execute_with`
accepts a small Runtime implementation for independent composition tests.

## Nodes

Every node is an object with a required `type`. Unknown fields are rejected. Any
root node may additionally contain a `definitions` object mapping names to nodes.

| Type | Additional fields | Result |
| --- | --- | --- |
| `request` | `key`, `target`, `input` | Requested actor's output object |
| `sequence` | Non-empty `steps` array | Last step's output |
| `parallel` | Non-empty `steps` array | `{"results":[...]}` in plan order |
| `branch` | Boolean `condition`, `then`, `else` | Selected node's output |
| `repeat` | `max_iterations`, `body`, Boolean `until` | First body output satisfying `until` |
| `foreach` | Array `items`, `body`, `mode` (`sequence` or `parallel`) | `{"results":[...]}` in item order |
| `call` | `name` | Named definition's output |
| `value` | `output` | Resolved output object |

Request input and value output templates must be objects. Resolved inputs,
outputs and actor results must be objects. Conditions and item arrays may be
bindings. Repeat executes the body at least once. Reaching its bound without a
true `until` returns an error. Definitions must be acyclic: deliberate repetition
uses `repeat` or `foreach`.

## Bindings and propagation

Templates recursively preserve literal JSON values. An object containing exactly
one of these fields substitutes a JSON Pointer (RFC 6901):

| Binding | Source |
| --- | --- |
| `{"$input":"/change"}` | Original workflow input |
| `{"$previous":"/review"}` | Previous output in the surrounding sequence or iteration |
| `{"$item":"/name"}` | Current foreach item |
| `{"$iteration":"/index"}` | Zero-based repeat/foreach iteration |

The empty pointer selects the complete source. Pointer escapes are `~0` for `~` and `~1`
for `/`. Missing values return an error, preserving the distinction between an absent
binding and an explicit null. `$`-prefixed template keys are reserved and cannot encode
literal keys.

A root sequence initially sees the workflow input as `$previous`. Nested groups inherit
their parent's context. A sequence updates `$previous` after each child. Parallel
branches inherit the same context and cannot observe sibling results until the group
finishes. A branch or call preserves context. Repeat updates `$previous` to each
completed body result before evaluating `until`. Sequential foreach also passes the
previous item result into the next item. `$input` remains the original input throughout.
Outside a foreach, `$item` is null. The initial iteration index is zero.

```json
{
  "type": "sequence",
  "definitions": {
    "review": {
      "type": "parallel",
      "steps": [
        {"type":"request","key":"security","target":"security","input":{"$previous":""}},
        {"type":"request","key":"correctness","target":"correctness","input":{"$previous":""}}
      ]
    }
  },
  "steps": [
    {"type":"call","name":"review"},
    {"type":"request","key":"synthesise","target":"editor","input":{"reviews":{"$previous":"/results"}}}
  ]
}
```

## Scheduling, recovery and bounds

The interpreter advances each branch until it needs an actor response. It collects the
ready requests in deterministic depth-first plan order and submits batches of at most
eight to the host. Nested parallel groups share these batches. Subsequent steps become
ready after their predecessor results are available. A group with more than eight ready
leaves executes in consecutive batches. The batch size bounds concurrency, and
scheduling determines when each leaf actually runs.

Each request key hashes its structural path, definition call sites, iteration indices
and declared key. This keeps keys within host limits and reproducible across replay.
Invocation identities, retained outputs, admission, deadlines, failures and effect
recovery remain the host's responsibility. Plan changes require a new run. Reusing a
retained checkpoint requires the same request input.

Plans and materialised values obey MAS's JSON rules: exact integers, at most 65536
serialised bytes and the codec's depth bound of 32. There are at most 4096 declared
nodes, 1024 iterations per repeat/foreach, 64 expanded control levels and 100000 node
evaluations. The host's own worker, request, deadline and call limits still apply. These
limits bound the resources available to composed plans. A failed request stops the
workflow. Declare retry, fallback and compensation as explicit application operations so
they retain their own admission and recovery semantics.

## Agreement on one candidate

An ordinary `parallel` preserves its all-results contract. An optional `join`
adds the versioned `mas-candidate-quorum/1` agreement contract:

```json
{
  "type":"parallel",
  "steps":[
    {"type":"request","key":"model-review","target":"reviewer","input":{"$input":""}},
    {"type":"request","key":"exact-check","target":"checker","input":{"$input":""}}
  ],
  "join":{
    "version":"mas-candidate-quorum/1",
    "minimum":2,
    "candidate":{"$input":"/candidate"},
    "context":{"$input":"/review_context"}
  }
}
```

The expected `candidate` resolves to an exact `{"id":"…","sha256":"…"}` reference with a
valid protocol identity and lowercase SHA-256 digest. The expected `context` resolves to
a non-empty object. The application supplies the frozen rubric, criterion, evidence and
input identities needed to distinguish the judgement being requested. Both bindings are
resolved and validated before dispatch, fixing the expected candidate and context
independently of worker output. Shape validation checks the reference representation.
The application must separately verify the candidate's integrity and supply the complete
decision context.

Every joined branch must directly request a distinct registered actor. Each review
result has `candidate`, `context` and a Boolean `accepted` field. Other output fields
remain available. A vote counts only when its candidate and entire context equal the
expected bindings and `accepted` is `true`. Malformed records, different candidates and
stale contexts cannot contribute to one quorum. Actor identity comes from the declared
request target rather than a field returned by the worker. Distinct actor identities
establish whose votes count. Independence of their models, evidence or judgements
requires separate application evidence.

All branches finish through the existing request path. The result retains every output
in `results`, including rejected and conflicting reviews. Its `join` receipt contains
`version`, Boolean `accepted`, `minimum`, the expected `candidate` and `context`,
`binding_sha256`, `selected` and `observations`. `binding_sha256` hashes the canonical
JSON object containing `version`, `minimum`, `candidate` and `context`. `selected` names
the first `minimum` qualifying branches in plan order as
`{"branch":0,"actor":"reviewer"}` records. It is empty when the threshold is unmet. Each
observation records that branch and actor with `status` equal to `accepted`, `rejected`,
`mismatched` or `invalid`. A rejected quorum still returns the receipt. A following
branch can inspect `{"$previous":"/join/accepted"}` to select the next action. Request
failures retain the existing workflow failure behaviour. Every branch completes under
the existing request contract. A factual correctness claim requires evidence beyond vote
agreement.

Replaying a retained run reuses the same request identities and results, then
recomputes the same selection and receipt. The plan, expected bindings and host
implementation remain part of the existing immutable run identity.
