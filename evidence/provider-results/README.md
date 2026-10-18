# Inference result verification receipt

The [receipt](verification.json) records all **86 host formal checks**: 16
complete positive safety/settlement searches with zero errors, and 70 expected,
replayed mutation or reachability counterexamples. It covers the revision,
disclosure, delegation and provider-result models. Of these, the new provider
model contributes 32 checks. The
[compressed raw evidence](spin-checks.tar.xz) contains commands, logs, model and
runner copies, generated counterexample verifiers, trails and configurations.
The expanded ZIP contents are under `counterexamples/`.

SPIN 6.5.1 reports `20 December 2019`; the C compiler is GCC 13.3.0. SPIN was the
maintainer's `Bin/spin651_linux64.gz` from repository revision
`090f74209025a53297990ec17deca1bd51cb92a5`; the decompressed executable SHA-256 is
`21bedb934fa0a70badb9b9f713b2bae8a72d3d5ec41c623cf23e9bbee51b3419`.
Search limits, assumptions, exact commands and all model hashes are in the
receipt and archive. No approximate hashing or fairness switch was used.

The provider model's no-reduction safety search explored 27,853 stored states
and 36,379 transitions, with zero errors. Independent agent review was performed
separately from model construction. It found a draft crash-interleaving flaw:
waiting inside an already-selected non-crash branch could observe a response
after the crash. Moving that wait into the branch guard preserved the actual
crash boundary. All final searches use the corrected source. The reviewer also
checked the request-admission bypass and known-invalid-to-unknown mutations,
and the retained-refusal and inadmissible-request witnesses. No review finding
remained open on the final model hash:
`789b4d966b9ad90d739bd6d104fc6ab19a4a0e0c70830520c9867191dd836f43`.

The retained [provider/runtime tests](provider-contract-tests.log) report 26
provider and 3 runtime tests passing. The
[core/SDK recovery tests](provider-recovery-tests.log) report 39 core and 6 SDK
tests passing. [Clippy](provider-clippy.log) passed with warnings denied, and
Rust 1.99.0 formatting checks passed. Tests use local fixtures, not live models.

Read the [contract](../../docs/inference-results.md) and
[model bounds](../../formal/README.md#inference-result-admission-and-durable-replay)
before interpreting these results. They establish finite control properties and
selected implementation correspondence, not an implementation refinement proof,
elapsed-time guarantee, globally exactly-once effects or live-provider success.
