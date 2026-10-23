# N1 local detector evidence

The implementation is commit `3902bd13b53bb0909975dcf418cf20a82afbb644`, tree
`9147baf6eaf23f8a6dcbf76e16dd28d4fa52e298`, based on the foundation commit
`dcdd73971aecd8e3962f582937e721b04d6a1379`. A separate evidence commit lets the receipt
identify the tested implementation without a self-reference. `validation.json` binds the
changed source files and supporting records by SHA-256.

## Performed checks

| Command or check | Observed result |
|---|---|
| `cargo test --locked --workspace` | 211 passed, two ignored |
| `make acceptance` | Passed. Recovery retained seven child steps, one protocol call and one unchanged receipt |
| `cargo fmt --all -- --check` and workspace Clippy with `-D warnings` | Passed |
| `make privacy-test` | 15 adapter tests and 13 qualification-harness tests passed |
| Existing SDK/platform commands in `sdk-regression.json` | Python 58, workload 14, Node provider five, Go SDK seven and Go fixture one passed. Go vet and C++ conformance passed |
| Explicit real-model qualification | All six synthetic CLI cases passed under the corrected harness |

The two normally ignored Rust entries are the separately executed native
acceptance test and a guardian subprocess entrypoint. `GOFLAGS=-buildvcs=false`
matches the original baseline's environment workaround. No protocol or consumer
host pin changed.

`qualification.json` binds the actual host binary, final harness, corpus, Python
executable, worker and bundle manifest. A case set becomes `PASS` only after every
expected case completes. Intermediate checkpoints remain `RUNNING`. The fixed corpus
tests English names/email, Unicode, combining characters, a final-token email, benign
text and empty input. Positive checks establish coverage of the annotated entities and
valid original UTF-8 boundaries. General precision, excess masking and deployment recall
require a separately designed domain evaluation.

The six independent worker launches took 105.0951 seconds in total. Maximum cumulative
child peak RSS was 4,269,696 KiB (about 4.07 GiB), under a 12 GiB virtual-address
allowance. These measurements describe this synthetic CPU run. Latency percentiles and
service capacity require repeated workload measurements on an identified deployment.

`bundle.json` is the exact manifest used for the accepted run. Its SHA-256 is
`832d463c76ab3417966a854cb8b67c8f7922ab9f3b4da05770c80374147cc4a5`. The upstream code
revision is `f7f00ca7fb869683eb732c010299d901457f19c3`. The model revision is
`7ffa9a043d54d1be65afb281eddf0ffbe629385b`. `upstream-smoke.json` records an earlier
official-API feasibility check. The packaged CLI qualification has its own receipt and
remains the evidence for that path.

## Failure controls and remaining scope

Fault controls cover source/detector substitution, wrong offset units, incomplete
coverage, missing decoder output, nonfinite scores, extra importable modules,
source-bearing exception text, output/frame limits, timeout and descendant
cleanup. The existing worker's 2 GiB default is preserved and tested alongside
an explicit larger memory allowance. Provisioning cleans up only its own
temporary files. A partial or interrupted qualification cannot claim `PASS`.

Unexpected temporary files in the initial asset staging directory caused a safe
pre-inference refusal. The same manifest passed after staging the exact verified
files in a clean directory. No asset or coverage check was relaxed.

All real-model results retain `release_permitted: false` and `quality_qualified: false`.
This is a trusted local N1 detector implementation. Immutable approved
code/assets/runtime and cooperative supervised workers remain assumptions. At this N1
checkpoint, complete N2/E2/E3 mediation and N3 durable cases and notifications remained
outstanding. N4 consumer integration and E4/E5/N5 lifecycle, isolation and deployment
qualification also required later work. The [implementation
sequence](../../docs/nehemiah-implementation.md) records subsequent slices without
extending this receipt's evidence scope. No provider notification, deployment, merge or
manual workflow dispatch forms part of this evidence.

Reproduction and provisioning commands are in
[the detector guide](../../docs/privacy-detector.md) and
[the qualification guide](../../tests/privacy/README.md).
