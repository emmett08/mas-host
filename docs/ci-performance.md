# CI performance

The `Native host` workflow runs native tests and the complete SPIN suite in
independent jobs. Its existing `linux` status waits for both jobs and passes only
when both report success. Gate validation covers all 16 combinations of
success, failure, cancellation and skipped-job results.
Existing required-status settings can continue to require `linux`.

## Baseline and objective

The objective is at least a 20% reduction in successful feedback time, while
retaining every test, acceptance scenario and formal check. Runner execution
time and cold compilation are secondary constraints.

The following completed jobs establish the hosted baseline. The revisions differ,
so the rows describe observed runs rather than a controlled before/after comparison.
All durations come from GitHub's job and step timestamps.

| Run | Revision | Whole job | Tests and analysis | Acceptance | SPIN and installation |
| --- | --- | ---: | ---: | ---: | ---: |
| [39830500735](https://github.com/emmett08/mas-host/actions/runs/39830500735) | `db77a6704437` | 847 s | 514 s | 37 s | 269 s |
| [39840094948](https://github.com/emmett08/mas-host/actions/runs/39840094948) | `c84feb406eff` | 1,020 s | 637 s | 37 s | 313 s |
| [39859225460](https://github.com/emmett08/mas-host/actions/runs/39859225460) | `4fd4b536e658` (PR #20) | 1,816 s | 1,324 s | 43 s | 418 s |

Tests and SPIN accounted for 95.9% of the latest job. Starting SPIN independently
removes the dependency on the native tests. Using that run's observed durations,
perfect overlap would remove about 418 s from the critical path. Separate job
startup, queueing and the final status gate reduce this estimate. Parallel jobs
alone reduce waiting time. The executable-size change reduces work in the native
branch. Independent jobs retain both diagnoses after a failure, which can
increase runner time for unsuccessful runs.

## Test compilation

The host hashes retained executables when building bundles, admitting workers
and binding recovery state. CLI integration scenarios repeat these operations
across fresh processes. Their cost therefore depends on executable size and the
number of scenarios. Full inline debug information contributes bytes to every hash and
retained copy even though the worker executes only the program code.

The test job sets Cargo's development and test `split-debuginfo` setting to
`unpacked`. On Linux, full debug information is emitted into `.dwo` sidecars.
The executable retains references to those files. Keep the sidecars in the
build tree when using a debugger. Optimisation level, debug assertions and
overflow checks retain their existing development settings.

`RUST_TEST_THREADS=2` bounds concurrent test scenarios and their subprocesses.
Application concurrency tests still use their own explicit workers. Ordinary
developer builds retain their existing profile settings.

Rust documents [split debug information on Linux](https://doc.rust-lang.org/rustc/codegen-options/index.html#split-debuginfo).
Cargo documents the independent [profile settings](https://doc.rust-lang.org/cargo/reference/profiles.html)
and their [environment overrides](https://doc.rust-lang.org/cargo/reference/environment-variables.html).
GitHub documents the [`needs` and `always()` job dependency behaviour](https://docs.github.com/en/actions/reference/workflows-and-actions/workflow-syntax#jobsjob_idneeds).

## Local measurements

The profile comparison binds runtime source `8b535be6d3de`, before PR #20 merged.
Both profiles ran on the same shared x86_64 Linux host with an eight-core CPU
quota, 8 GiB memory limit and Rust 1.99.0. Both used two test threads. No CPU
isolation was applied. These results establish a local fixture improvement.
Full-suite validation uses merged main `293e394d5996`, including PR #20.

| Metric | Inline debug information | Split debug information | Reduction |
| --- | ---: | ---: | ---: |
| Host executable | 71,072,648 bytes | 31,797,624 bytes | 55.3% |
| All 14 configured-agent tests, one execution each | 259.89 s | 102.71 s | 60.5% |
| Workflow/replay scenario, median of three paired runs | 15.10 s | 7.35 s | 51.3% |
| Paired scenario range | 14.37–17.13 s | 6.68–9.69 s | — |

All measured executions passed. The paired order was baseline, candidate,
candidate, baseline, baseline, candidate. Each pair invoked the compiled test
executable directly, selecting
`configured_workflow_runs_llm_peer_requests_through_mas_and_replays_without_calls`.
The full-suite row uses the test harness duration and excludes Cargo startup
and a baseline package-cache wait. [Exact measurements](ci-performance-results.json)
retain the commands' success, sample values, source identity and limitations.

The selected candidate compiled from an empty target directory in 54.05 s using
two build jobs. The initial baseline build took 26.54 s with default parallelism.
Those observations provide compilation context with different concurrency.
A separate opt-level=1 candidate needed 273.77 s with two build jobs. Its suite
encountered a temporary-storage failure, so that experiment supplies no passing
full-suite comparison. Splitting debug information avoids that optimisation cost.

## Reproduce

Use Rust 1.99.0, Go 1.24.0, Python 3.11 or later, Node, a C++17 compiler and SPIN.
Run the following from a clean checkout. Give each profile its own initially
empty target directory, time cold compilation separately, then alternate the
same test command between the two profiles. Use at least three paired runs.

```sh
RUST_TEST_THREADS=2 CARGO_TARGET_DIR=/tmp/mas-host-baseline \
  cargo test --locked -p mas-host --test configured --no-run

RUST_TEST_THREADS=2 CARGO_TARGET_DIR=/tmp/mas-host-optimised \
CARGO_PROFILE_DEV_SPLIT_DEBUGINFO=unpacked \
CARGO_PROFILE_TEST_SPLIT_DEBUGINFO=unpacked \
  cargo test --locked -p mas-host --test configured --no-run
```

Repeat without `--no-run` to measure execution. The acceptance criterion includes
identical test counts and successful results. Validate the complete candidate
with the same environment by running `make test`, `make acceptance` and
`make provider-test`. Run `python3 formal/verify.py` with the existing command,
memory and search-depth limits. Validate the workflow with `actionlint`.

Compare hosted total elapsed time and the sum of job execution times after the
first candidate run. Treat the modelled overlap and local measurements separately
from a hosted improvement. Reverting the workflow restores the previous schedule
and compiler settings.
