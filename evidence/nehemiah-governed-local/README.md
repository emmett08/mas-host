# Governed local implementation evidence

Source commit: `06f150b5e6a32b42b9134f27f67c93256cfb544e`.
Source tree: `b9e215030d4bd450283616f5a7349a39d8703157`.
Stack base: `f651ddac8a5afabfbb955de805b3ff59b1ecde99`.

`validation.json` binds the changed source files and accepted checks. The full
workspace run used a clean production-core rebuild with incremental compilation
disabled. Source bytes stayed fixed throughout the run. Final changes after that
run affect only test scheduling waits and documentation revision references;
the four A2A tests and formatting were then checked again. Earlier cached
intermediate artifacts are superseded and supply no accepted runtime evidence.

The checks include 245 Rust tests, separate native recovery acceptance, all SDK
conformance, TypeScript/Python fixtures and actual package execution/replay,
privacy adapter/harness regressions and five Node provider tests. The three
ignored workspace entries are the separately executed native acceptance and two
subprocess harness entry points exercised by their parent tests.

`formal-checks.tar.xz` retains the complete commands, replay witnesses, tool
provenance and summary for 86 checks of the unchanged finite models. Its original
index hashes the original ZIP; the archive expands that ZIP's contents under
`counterexamples/`. These checks do not provide a new model or refinement proof
for the governed runtime.

The accepted runtime slice uses closed structured schemas, current file-backed
authority and explicit inline-retention permission under a trusted local
operator/worker boundary. It does not establish general unstructured privacy,
worker isolation, distributed revocation, deletion/restore, notification
recovery, production capacity or population model accuracy. Complete N2 and the
later plan gates remain pending. The prior real-model receipt remains bound to
its original N1 source; the model was not rerun for this increment.
