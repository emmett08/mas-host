# Referenced governed lineage

A fresh `mas-governed-local/2` namespace can select an explicit larger internal
source budget:

```json
{"lineage":{"version":"mas-governed-referenced-lineage/1",
            "max_sources":128,"max_metadata_bytes":262144}}
```

The profile permits 65–256 sources and 65,536–1,048,576 bytes of internal source
metadata. Its exact bounds belong to the retained namespace fingerprint. Source
records, native frames, payloads and all public responses retain their existing
65,536-byte limits. Each individual source has at most 64 direct parents. A
longer ancestry is represented by separate host-sealed records in the existing
SQLite journal. Configurations without this field retain the 64-source and
65,536-byte closure budget.

Every load resolves the complete source graph, checks every source's exact
retained payload binding, verifies its original policy association and rejects
missing ancestors, cycles, expired sources or a budget violation. The reference
contains a keyed commitment to the complete sorted source-ID set. Each source ID
already commits its payload binding, parents, policy references and validity.
A missing or incomplete closure cannot be represented as an empty history.

For a closure above 64 sources, the host evaluates an explicit policy conjunction
representation and returns `mas-data-use-referenced-decision/1`. The compact
receipt binds the exact operation, input/output, principal, recipient, policy
snapshot, complete closure reference and source count. It keeps the full DAG in
its existing owner instead of repeating it in a native frame.

For a fixed disclosure request, including a fixed byte count, this representation preserves the current rule language's permit and obligation answers over a complete conjunction. It extends the source budget beyond the older representation's 64-source limit.
Every retained source is classified `restricted`. Rules inspect the same
operation, principal, actor, producer, recipient, purpose, destination, location,
classification and byte count for every source. Therefore requiring every
original source policy is equivalent to requiring their union against one
closure identity. The host checks all source validity intervals separately;
the representative interval starts at the latest acquisition and ends at the
earliest expiry. Matched obligations accumulate by set union. Parent closure is
verified before this projection. The runtime accounts for the actual encoded retained/delivered representation. Compact receipts can fit a byte allowance that an expanded receipt exceeds. The byte allowance covers that encoding, rather than the total underlying DAG storage. This argument depends on the current rule
language and classification contract. A future rule that examines an individual
source's identity, payload or relationship needs a new representation and
acceptance analysis.

Completed source views use `mas-governed-source-view/2`,
`lineage_disposition: referenced` and a `lineage_reference`. Their empty inline
`lineage` array means the complete graph is available through that reference.
`GetRetainedSourceLineage` and `GetSharedSourceLineage` require the separately
negotiated `urn:mas:host-governed-lineage-read:1` extension. Pages carry at most
16 exact metadata records, their original closure reference and count, and the
next offset. The caller supplies the exact reference and the returned `next_offset`. Pages also obey the complete 65,536-byte envelope limit, so a dense page can contain fewer than 16 records. Only the deterministic page boundaries are accepted. Changed references, missing ancestors and invalid offsets fail as a whole.

Owned and shared page reads evaluate the complete task and source exposure for
every page. Shared reads retain independent producer and subscriber current
sessions. An earlier successful page grants no right to retrieve a later page
after revocation. These are retained-evidence reads and never fetch another
observation or perform execution recovery.

The 65- and 128-source fixtures populate identified source records through the
source owner, then exercise one real protected final acquisition. They test the
last source's restriction, complete page reconstruction, missing-ancestor
refusal and current revocation. Fixture qualification and the preservation
argument remain separate from deployment capacity and a Rust/SQLite refinement
proof.
