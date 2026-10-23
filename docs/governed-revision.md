# Governed structured revision

`mas-governed-local/2` can opt into the existing bounded revision controller with
an explicit `revision` contract. The contract names a declared controller actor
and closed schemas for `plan`, `context`, `candidate`, `rubric`, `evidence`,
`assessment`, `terminal` and `delivery`. Each kind accepts one schema or 1–16
closed alternatives. These alternatives select complete records and never merge
permissions. The `delivery` schema covers the selected application output.
Unsupported free text and schema forms continue to reject.

Every frozen plan, source input, invocation context, audit record and terminal
selection passes its exact structured contract, current `revision-retain`
permission and configured inspection before existing journal persistence. The
recipient for this control operation is `controller:ACTOR`, with that declared
actor's destination. Record projection must preserve the entire record.

Before each root draft, evaluator or reviser invocation, the host unions the
complete source exposure of earlier revision requests. The next invocation
inherits those restrictions through the same governed request/effect path.
Its approved input projection preserves complete evidence, evidence references,
feedback and rubric. Candidate identity, output and output hash also remain
unchanged. A closed projection may omit candidate ancestry metadata when the
worker's declared interface permits that omission. Native callbacks retain the
trusted-local boundary and revision effects remain inference-only.

The result contains the separately authorised selected output and a metadata
projection of the audit. Full candidate, assessment and context bodies remain
withheld from that result. Ordinary local revision retains its existing API and
reader behaviour. Retained governed state still rejects ordinary inspect,
observe and resolution entry points. Lifecycle retirement removes the revision
payload tables and permanently blocks execution under the original namespace.

The local operator can run a governed revision directly:

```sh
mas-host run --bundle bundle.json --agent drafter --input input.json \
  --state /state/runs --run RUN --revision-plan plan.json \
  --governance governance.json --tenant TENANT --principal PRINCIPAL \
  --purpose PURPOSE --grant GRANT
```

These CLI identity arguments belong to the trusted local operator. Network
clients obtain identity from the authenticated service. This command grants no
remote authentication capability. Add `--privacy-inspector FILE` when configured.

`tests/governance/test_revision.py` qualifies an exact deterministic closed
assessment loop through the real CLI and guardian, then checks restart without
another worker launch. Its schemas derive from an explicitly retained synthetic
reference trace. The test supports that fixture's contract. Deployment-specific
revision schemas, unstructured detector quality, deeper revision rounds and
managed-worker isolation require additional qualification.
