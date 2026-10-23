# Governed local implementation evidence

Source commit: `06f150b5e6a32b42b9134f27f67c93256cfb544e`.
Source tree: `b9e215030d4bd450283616f5a7349a39d8703157`.
Stack base: `f651ddac8a5afabfbb955de805b3ff59b1ecde99`.

`validation.json` binds the accepted checks to the changed source files identified
above. Read these results as evidence for that implementation revision. The full
workspace run used a clean production-core rebuild with incremental compilation
disabled. Source bytes stayed fixed throughout the run. The final changes after that run
affected test scheduling waits and documentation revision references only. The four A2A
tests and formatting were then checked again. Earlier cached intermediate artefacts are
superseded and supply no accepted runtime evidence.

The checks include 245 Rust tests, separate native recovery acceptance, all SDK
conformance, TypeScript/Python fixtures and actual package execution/replay,
privacy adapter/harness regressions and five Node provider tests. The three
ignored workspace entries are the separately executed native acceptance and two
subprocess harness entry points exercised by their parent tests.

`formal-checks.tar.xz` retains the complete commands, replay witnesses, tool provenance
and summary for 86 checks of the unchanged finite models. The original index hashes the
original ZIP. The archive expands that ZIP's contents under `counterexamples/`. These
checks cover the unchanged models, with a governed-runtime model or refinement proof
requiring separate evidence.

The accepted runtime slice uses closed structured schemas, current file-backed authority
and explicit inline-retention permission under a trusted local operator/worker boundary.
General unstructured privacy and population model accuracy require qualified detector
evidence. Worker isolation, distributed revocation, deletion/restore, notification
recovery and production capacity require their own mechanisms and validation. At this
checkpoint, complete N2 and the later plan gates remained pending. The prior real-model
receipt applies to its original N1 source, with no model rerun included in this
increment. See the [implementation sequence](../../docs/nehemiah-implementation.md) for
later slices and remaining work.
