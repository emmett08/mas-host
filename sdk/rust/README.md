# Rust application SDK

In the initial Linux profile, bind an application function to an agent and let the host
generate its worker:

```rust
use mas_host_sdk::{json, Context, Json, Result};

pub fn assess(_context: &mut Context, input: Json) -> Result<Json> {
    Ok(json!({"approved": input["amount_minor"] == 12500}))
}
```

An agent's `source` names this Rust file and `handler` names `assess`. `mas-host build`
generates a Cargo package and its `main` function, builds the worker and records its
digest. The generated worker supplies registration and transport. The source can use the
standard library, `mas_host_sdk` and Rust modules relative to the source file. Arbitrary
application Cargo dependencies and automatic struct-schema generation require a later
build profile.

The first build seeds dependency versions from the SDK distribution's `Cargo.lock`.
Cargo adds the generated package and retains the resulting lockfile in `.mas/build/`.
Subsequent builds require that lockfile to remain valid with `--locked`. An SDK update
requiring new dependencies needs a fresh generated build directory. Retain the old
immutable bundle when resuming an older run.

`Context::request(key, target, input)` runs a durable child operation.
`Context::parallel(&[Request::new(...)])` requests bounded concurrency and returns
outputs in the original request order. Mutable access serialises commands on one
context. Use named requests for work that needs replayable checkpoints. Ordinary threads
remain part of the handler computation.

`Context::effect(key, descriptor, |invocation_id| ...)` performs an external operation
only after host admission. The closure returns `(output, evidence)`. Both values are
objects, and evidence must be nonempty. The host may instead return a cached result
without invoking the closure. Failure before acknowledged completion requires
reconciliation. The SDK never retries automatically. Stable keys, descriptors and
evidence semantics belong to the application.

A governed acknowledgement may replace the local output with the host's safe
projection. Fresh and cached calls return that same object. Withheld output is
the reserved `mas-governed-unavailable/1` object and keeps the host effect completed.
Malformed replacement metadata returns `Err("invalid governed effect completion")`
without returning local raw output or retrying. A legacy acknowledgement without
`completion` returns the local output. See the exact version, identity, certainty
and disposition fields in the [native contract](../../docs/native-contract.md#external-effects-and-recovery).

Input/output values must be JSON objects. Integers remain exact. Use `Json` number
values for integers larger than machine integer types. Floating-point JSON, duplicate
keys, invalid Unicode, excessive nesting and frames larger than 65,536 bytes are
rejected through MAS's pinned parser. `deadline_ms` is the original absolute run
deadline, enforced by the host.

The generated worker communicates through a private inherited socket managed by the SDK
and builder. The guardian discards stdout and stderr. `serve` and the socket environment
variable remain SDK/build internals. Application code runs with trusted local
privileges, so hostile-code isolation requires additional deployment controls.

From this repository, run `cargo test -p mas-host-sdk` for protocol and effect
tests. The [Rust application](https://github.com/emmett08/mas-host-examples/tree/main/examples/rust-order) demonstrates a complete generated-entrypoint application.
