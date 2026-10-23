# Resolve source ancestry through A2A

Governed local/2 can select bounded referenced ancestry while preserving the
65,536-byte native message and source-view limits. The host resolves and admits
the **complete** source closure before publishing its reference or any metadata
page. Pagination changes the delivered representation. Every ancestor continues
to constrain admission.

Select the [referenced-lineage governance contract](governed-referenced-lineage.md)
when creating the namespace. That contract specifies the complete graph and
metadata budgets, policy-conjunction argument and compatibility obligations.
The following interface delivers its bounded source views and metadata pages.

## Resolve a source reference

For a completed source, `GetRetainedSource` and `GetSharedSource` return the
existing source facts and projection in `mas-governed-source-view/2`:

```json
{
  "version": "mas-governed-source-view/2",
  "lineage_disposition": "referenced",
  "lineage": [],
  "lineage_reference": {
    "version": "mas-governed-lineage-reference/1",
    "closure_ref": "REPLACE_WITH_THE_EXACT_RETURNED_REFERENCE",
    "source_count": 83,
    "page_size": 16
  }
}
```

This fragment omits the source view's other unchanged fields. The explicit
`referenced` disposition means that the ancestor metadata is available through
the authorised page interface. A client must follow that reference to obtain the
full ancestry needed by its own declared use. Incomplete or unavailable source
states may still use source-view/1. The opted-in Agent Card advertises both
possible result versions and the lineage-resolution extension.

Activate the base task-service and lineage extension:

```text
A2A-Extensions: urn:mas:host-task-service:1, urn:mas:host-governed-lineage-read:1
```

An owner selects its existing task and durable source-effect key:

```json
{
  "jsonrpc": "2.0",
  "id": "lineage-1",
  "method": "GetRetainedSourceLineage",
  "params": {
    "id": "a2a-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    "key": "collect",
    "closure_ref": "REPLACE_WITH_THE_EXACT_RETURNED_REFERENCE",
    "offset": 0
  }
}
```

An independently authenticated subscriber selects its existing operator-approved
source reference instead:

```json
{
  "jsonrpc": "2.0",
  "id": "lineage-1",
  "method": "GetSharedSourceLineage",
  "params": {
    "reference": "deployment-point-001",
    "closure_ref": "REPLACE_WITH_THE_EXACT_RETURNED_REFERENCE",
    "offset": 0
  }
}
```

The service derives the producer, task, actor, filesystem directory and effect
key from the same approved source-export selection used by `GetSharedSource`.
The subscriber supplies its own bearer. The source producer's current authority
and the subscriber's current authority both govern every page. Ordinary
owned-task APIs preserve their ownership requirement.

## Complete the page sequence

Each result is a `mas-governed-lineage-page/1` record with these fields:

| Field | Meaning |
|---|---|
| `closure_ref` | Exact keyed reference to the selected complete closure |
| `source_count` | Number of source metadata records in that closure |
| `offset` | Current page's zero-based position |
| `records` | Up to 16 source metadata records in deterministic order, bounded by the complete page's 65,536-byte limit |
| `next_offset` | Next position, or `null` after the last page |
| `coverage` | `complete-closure-resolved-page` |

The keyed closure reference has exactly 64 lowercase hexadecimal bytes.
Start at offset zero and use each returned `next_offset`. Stop when it is null.
Check that every page preserves the exact closure reference and source count,
then check the assembled record count and parent references before treating the
metadata as complete. `page_size: 16` specifies the maximum record count. Dense
metadata can produce a smaller page. Page offsets use unsigned integers and
the returned deterministic boundary. The transport accepts at most 256; the
core additionally checks the selected closure's exact size and page boundary.

The core resolves complete original source and task exposure, validates the
exact namespace/source binding and applies current `resolve` policy before
returning each page. A missing ancestor, changed reference, invalid offset,
revocation or unavailable policy prevents delivery. The bounded public error is
`source_not_found`, preserving the existing source-reader disclosure policy.
Malformed request fields receive `invalid_parameters`. A missing extension
receives `required_lineage_extension_missing`.

Each response observes its current authority/exposure snapshot. Permission may
change after an admitted response, and later pages may consequently become
unavailable. Retain the original reference and incomplete-coverage status for
that interrupted read. Fetching a page creates no collection, worker dispatch,
effect settlement or recovery permission.

## Validation

The focused HTTP integration case exercises an opted-in source view, exact owned
and shared page reads, extension advertisement/activation, changed references,
invalid page boundaries and foreign owned-task queries. Journal snapshots and
collector counts check that the reads preserve the original execution history.
The core's referenced-lineage cases exercise larger complete closures and
representation bounds separately.

```sh
cargo test --locked -p mas-host --test a2a_governed referenced_lineage_http
```

Current file-backed authority retains the trusted-local snapshot boundary.
Distributed linearised revocation, shared execution fencing and managed worker
confinement remain separate dependencies. Payload pages and metadata pages
retain their respective disclosure and lifecycle restrictions.
