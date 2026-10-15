# Workflow composition

`mas-host-workflow` supplies a recursive Composite interpreter and a Runtime
Strategy. The SDK adapter delegates ready requests through the same host request
and MAS admission path used by application code. Workflow structure is independent
of agent topology: star, tree and mesh applications use ordinary actors and edges.
The host enforces which actor may contact each target.

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

The empty pointer selects the complete source. Pointer escapes are `~0` for `~`
and `~1` for `/`. Missing values return an error; they are never silently replaced
with null. `$`-prefixed template keys are reserved and cannot encode literal keys.

A root sequence initially sees the workflow input as `$previous`. Nested groups
inherit their parent's context. A sequence updates `$previous` after each child.
Parallel branches inherit the same context and cannot observe sibling results
until the group finishes. A branch or call preserves context. Repeat updates
`$previous` to each completed body result before evaluating `until`. Sequential
foreach also passes the previous item result into the next item. `$input` remains
the original input throughout. Outside a foreach, `$item` is null; the initial
iteration index is zero.

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

The interpreter advances each branch until it needs an actor response. It collects
the ready requests in deterministic depth-first plan order and submits batches of
at most eight to the host. Nested parallel groups share these batches. Subsequent
steps become ready after their predecessor results are available. A group with
more than eight ready leaves executes in consecutive batches; this is bounded
concurrency rather than a promise of simultaneous execution.

Each request key hashes its structural path, definition call sites, iteration
indices and declared key. This keeps keys within host limits and reproducible
across replay. Invocation identities, retained outputs, admission, deadlines,
failures and effect recovery remain the host's responsibility. Plan changes
require a new run; changed request inputs cannot reuse a retained checkpoint.

Plans and materialised values obey MAS's JSON rules: exact integers, at most
65536 serialised bytes and the codec's depth bound of 32. There are at most 4096
declared nodes, 1024 iterations per repeat/foreach, 64 expanded control levels
and 100000 node evaluations. The host's own worker, request, deadline and call
limits still apply. These limits bound execution; finite composition does not
promise unlimited resources. Any failed request stops the workflow; retry,
fallback and compensation policies must be explicit application operations.
