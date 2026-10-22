# Go application integration

Keep application handlers as exported functions in an importable Go package. The host
generates the worker around a typed function such as:

```go
func Assess(ctx *mas.Context, proposal Proposal) (Assessment, error)
```

The two configuration files select functions and permitted communication edges.
`mas-host build` generates the executable entry point, transport binding and
registration bundle from those declarations. The [Go
application](https://github.com/emmett08/mas-host-examples/tree/main/examples/go-release)
includes parallel assessments, bounded capacity revisions, independent acceptance and
optional audit publication.

## Application API

- `ctx.Request(key, target, input)` returns a durable child result as
  `json.RawMessage`. Decode it with `mas.Decode[Result](output)`.
- `ctx.Parallel([]mas.RequestSpec{...})` schedules independent children and
  returns results in request order. Each request has `Key`, `Target` and `Input`.
- `ctx.Effect(key, descriptor, callback)` asks the host for permission before
  calling the external operation. The callback receives a stable invocation ID
  and returns `mas.EffectResult{Output: result, Evidence: evidence}`. Evidence is
  a nonempty object. A previously completed effect returns its recorded output
  without calling the callback.
- The context embeds `context.Context` and exposes `Actor` and `InvocationID`.
  The host also enforces the deadline and supervises the worker process.

Keys identify operations within their parent invocation. Reuse a key only for the same
operation and input. An interrupted handler may execute again while completed requests
and effects reuse their host-journal records. Keep code outside effect callbacks
deterministic and free of external effects so replay preserves the same orchestration
decisions. Failure before acknowledged effect completion requires reconciliation.
Catching the error does not clear uncertainty or authorise another external attempt.

A governed completion returns the host's safe output projection as
`json.RawMessage`, with identical output on cached replay. Withheld payloads use
the reserved `mas-governed-unavailable/1` object and retain completed host settlement.
Malformed projection metadata returns `invalid governed effect completion`,
without exposing local raw output or retrying. An acknowledgement without
`completion` preserves the ordinary callback output. The
[native contract](../../docs/native-contract.md#external-effects-and-recovery)
defines the closed versioned acknowledgement, effect identity, external certainty
and payload disposition.

## Data profile

Inputs, outputs, descriptors and evidence are JSON objects. Structs and string keyed
maps work naturally. Integer fields remain exact. `interface{}` fields decode numbers as
`json.Number`. Floating-point fields are rejected, including integral floating-point
values. Use scaled integers or explicit string encodings for decimals. Explicit
`json.Marshaler` codecs are supported when their emitted JSON satisfies the same
contract. Binary data follows Go's explicit base64 JSON encoding for byte slices.

The SDK rejects duplicate object keys, fractional/exponent numbers, integers longer than
1024 decimal digits, invalid UTF-8, unpaired Unicode surrogates, frames above 65536
bytes and container nesting beyond 32 levels. The host remains the authority and
validates independently. The first implementation validates runtime values through Go's
typed JSON codec. Generating JSON Schema or statically validating custom codecs requires
further tooling. Unsupported runtime values produce a handler error.

## Generator

For direct development use:

```sh
go run ./cmd/mas-go-gen --project /path/to/application \
  --output /path/to/application/.mas/build --sdk-dir /path/to/mas-host/sdk/go
```

The generator obtains package information with `go list`, validates exported function
names and lets Go's compiler verify the typed signatures through `mas.Adapt`. It
generates one executable per source package, sharing that executable between registered
handlers from the package. It preserves module requirements, replacements and
exclusions, resolves relative replacements from the application's module root and copies
its checksums. Builds use `-mod=readonly`, so the application must already declare and
checksum its dependencies. Active Go workspaces require a standalone module build or
explicit `GOWORK=off`. Go 1.24 or newer and Linux are the initial supported profile.

All generated sources and `go-workers.json` live under the selected output directory,
preserving application source files. Use `--go` to select the compiler and `--agents` to
select an alternative agent configuration path.

Run `go test -race ./...` in this directory. Tests exercise the actual Unix
socketpair protocol, exact integer roundtrips, rejected effect admission, cached
effect suppression and generated workers compiled from an independent module
with a relative dependency replacement.
