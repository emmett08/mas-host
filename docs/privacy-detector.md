# Local Privacy Filter inspection

`mas-host privacy-inspect` adds N1's local neural-detector path to the N0
assessment contracts. Its default packaged path runs the pinned OpenAI Privacy
Filter Python adapter under the existing Linux worker guardian, validates the
result against the original text, and combines it with the deterministic
recognisers. The command
returns inspection metadata with `release_permitted: false`. Protected release
remains held until N2 and E2/E3 mediate the complete execution path.

## Prepare a local bundle

The adapter lives in `adapters/privacy`. Provision its reviewed source, model,
tokeniser and Python runtime before inspection. Runtime inference uses those
local assets and an exact bundle manifest. Model acquisition and dependency
installation belong to a separate provisioning step with the operator's network
and storage permissions.

The reviewed package's backend is `opf-cpu-bfloat16/1`, using CPU inference,
bfloat16 weights, the `o200k_base` encoding and Viterbi decoding. The initial profile has
a 4,096-token context setting and a 16,384-byte source limit. Whole-input
inspection or an explicit incomplete result remains required within these
bounds. More than 4,096 tokens produces an unsupported `input-token-limit`
assessment, preserving the original input rather than silently truncating it.
Language/domain quality remains unqualified in this increment. Additional
backends and quantisation choices need their own evaluation.

| Component | Bound identity |
|---|---|
| Adapter | `mas-privacy-opf/1`, worker and adapter-file digests |
| Official implementation | `openai/privacy-filter@f7f00ca7fb869683eb732c010299d901457f19c3` |
| Model assets | `openai/privacy-filter@7ffa9a043d54d1be65afb281eddf0ffbe629385b`, with individual file sizes and SHA-256 digests |
| Tokeniser | Provisioned `o200k_base.tiktoken` bytes and digest |
| Runtime | Exact interpreter digest, Python patch version and locked dependency wheel identities/versions |
| Bundle | SHA-256 of the exact `mas-privacy-bundle/1` manifest bytes |

`upstream-code.lock.json` and `upstream-assets.lock.json` identify the reviewed
upstream files. The generated manifest additionally binds the local adapter,
settings and runtime dependency graph. Changed code, assets, settings or package
versions require a new identified bundle. Hashes identify the selected bytes
under the operator's immutable-runtime, asset and invocation assumptions.
Publisher trust requires separate operator approval. Evidence that the official
model ran depends additionally on the reviewed adapter and retained inference
execution evidence. Model quality and disclosure authority retain their
separate acceptance criteria.

The current `runtime.lock.json` selects CPython 3.12.14 on Linux x86_64 and
records the exact interpreter digest. Start with that identified interpreter
distribution. A matching version string alone cannot satisfy its byte identity.
Other interpreter builds and platforms require a separately reviewed runtime
lock and execution evidence. Python 3.11+ remains the source-code baseline for
the standard-library adapter and provisioning utilities.

Provision explicitly from the repository root. Replace the interpreter path
with the one identified by the runtime lock and choose writable deployment
paths for the virtual environment, assets and manifest:

```sh
/absolute/path/to/locked/python3 -m venv /opt/mas-privacy/venv
python3 adapters/privacy/provision.py requirements --output /tmp/mas-privacy-requirements.lock
/opt/mas-privacy/venv/bin/python -m pip install --require-hashes --no-deps -r /tmp/mas-privacy-requirements.lock
python3 adapters/privacy/provision.py fetch --assets /opt/mas-privacy/assets
/opt/mas-privacy/venv/bin/python -I adapters/privacy/provision.py seal \
  --assets /opt/mas-privacy/assets --output /opt/mas-privacy/bundle.json
```

`requirements` emits the locked wheel URLs and SHA-256 hashes. `fetch` acquires
and verifies reviewed model, tokeniser and source files during provisioning.
The model weights occupy approximately 2.8 GB before runtime and other storage
overheads. `seal` verifies the selected assets and runtime, then writes the
identified manifest. It performs no model inference.

Runtime checks preserve the locked dependency versions and interpreter digest.
The operator retains responsibility for immutability of the installed native
runtime and dependencies. Recorded wheel provenance and package version labels
support identity checks while full installed-runtime byte isolation remains a
separate deployment property. Shared-library byte identity and integrity remain
operator deployment obligations. Optional unused `pip` and `opf==0.1.0`
installations are tolerated. The adapter imports OPF from its verified local
source tree.

## Inspect bounded UTF-8 text

After provisioning, use absolute paths for the selected virtual environment,
manifest, asset directory and worker. The example paths below belong to an
operator-prepared local bundle.

```sh
cargo build --locked -p mas-host
printf '%s\n' 'Synthetic contact: person@example.invalid' > /tmp/synthetic-privacy-input.txt
target/debug/mas-host privacy-inspect \
  --input /tmp/synthetic-privacy-input.txt \
  --python /opt/mas-privacy/venv/bin/python \
  --manifest /opt/mas-privacy/bundle.json \
  --assets /opt/mas-privacy/assets \
  --worker /opt/mas-host/adapters/privacy/worker.py
```

Use `--input -` to read bounded text from stdin. Input files are read as their
literal UTF-8 content. JSON field extraction, unescaping, OCR, attachments and
their source mappings remain subsequent supported-format work. Invalid UTF-8
returns an unsupported/hold result. Over-limit input and invalid configuration
are rejected before launching the model.

| Option | Default and bound |
|---|---|
| `--timeout-ms` | 120,000 ms, configurable from 1 to 600,000 ms |
| `--memory-mib` | 12,288 MiB of virtual address space, configurable from 256 to 65,536 MiB |
| `--include-details` | `false`, with explicit `true` enabling restricted category/span metadata |
| `--worker` | Source-tree adapter path compiled into this host, with an explicit override for relocated installations |

The Python executable must be an absolute existing file. Its virtual-environment
path is preserved so that interpreter symlink resolution cannot silently select
the base runtime. Provisioned worker and asset paths must remain under the
operator's control throughout inspection.

The CLI follows the trusted local registered-worker contract. `--worker` can
select an operator-trusted alternative whose response follows the same wire
contract. The reviewed packaged adapter enforces the official source, model
and runtime locks described above. Trust in an alternative worker requires its
own approval and execution evidence. Every CLI path continues to report
`release_permitted: false`.

The guardian supplies the existing private framed channel, deadline, CPU and
file-size limits, discarded worker stdout/stderr and confirmed descendant
cleanup. The memory allowance bounds virtual address space. Resident memory and
CPU capacity require measurement on the selected machine. Ordinary workers
retain their existing 2 GiB default. Trusted cooperative-worker assumptions and
the guardian's inherited-process-group requirements continue to apply.

The adapter selects offline model/tokeniser operation, suppresses bytecode and
content previews, and applies a Python network fault guard. OS network, filesystem
and credential confinement remain deployment obligations under E5. Inspection
uses an in-memory source and an anonymous ownership descriptor. Durable run
handling and response operations remain N2/N3 responsibilities.

## Interpret the result

The default `mas-privacy-inspection/1` result reports source byte count,
detector identity, completion, finding count, handling reason and disposition.
Raw source text and detected values stay out of this response. Explicit details
add category/span metadata whose locations and classifications still require
appropriate handling.

| Field or state | Interpretation |
|---|---|
| `completion: complete` | The declared inspection covers the admitted input within this adapter profile |
| `partial` | Findings may be valid while inspection coverage or another required condition is incomplete |
| `unsupported` | A declared format or capability falls outside the supported profile |
| `failed` | The worker or response could not supply an accepted assessment |
| `quality_qualified: false` | Deployment-specific detection quality remains unevaluated |
| `release_permitted: false` | Protected disclosure and execution remain subject to their host-owned admission gates |
| `hold` / `withhold` | Conservative local handling remains in force |

Deterministic positive findings survive a neural timeout, unavailable model or
invalid neural response. A completed deterministic pattern pass supplies its own
limited evidence while neural completion remains failed. Empty findings and
complete coverage still require labelled-domain evaluation before any release
policy can rely on them. Assessment metadata records `score_semantics:
not-exposed`. Future score exposure requires a versioned definition and any
calibration needed by its intended use.

The worker returns half-open ranges in original Unicode codepoints. Rust maps
each endpoint through the unchanged source's Unicode-scalar boundaries into
half-open UTF-8 byte ranges. Combining characters remain separate codepoints.
Conversion preserves the original spelling, normalisation and whitespace.
The host validates source identity, a fresh opaque source token, byte length,
detector manifest identity, labels, ranges and declared coverage. Unknown labels,
changed bytes, mismatched identities and malformed ranges prevent acceptance.

The raw source fingerprint remains private process state. Durable protected
commitments, key rotation and replay identity belong to E2/N2. Overlapping spans
merge by byte-range union and accumulate category and recogniser provenance.
Adjacent findings retain separate ranges. Both individual and combined finding
counts remain bounded, and encoded requests/responses must fit their 65,536-byte
frame limit.

## Ownership and design choice

| Design | Consequence | Decision |
|---|---|---|
| Small adapter plus existing `Worker` guardian | Adds one pinned classifier while preserving process lifetime and cleanup ownership | Selected |
| Second process manager or general plugin runtime | Adds another supervision and compatibility contract before protected execution is integrated | Deferred |
| Existing deterministic fixture alone | Preserves N0's contract examples and leaves contextual neural classification unimplemented | Retained for fixtures alongside the N1 adapter |

The CLI separates launch configuration from supervised execution. The privacy
crate separates detector identity, source-correlated neural records and combined
findings. The Python package separates bundle/runtime verification, wire records
and model execution. These responsibilities accommodate a later evaluated
backend without moving host admission into the classifier.

Configuration, result parsing and model integration are extracted responsibilities
because they change for different reasons and have independent failure tests.
The worker's existing lifetime machinery remains the cleanup owner. Its new
resource-limit validation stays beside worker construction because it checks one
bounded launch invariant. The span merger remains the single owner of overlap
and provenance rules.

Rust follows the existing 2021 edition, selected toolchain, `rustfmt` and Clippy
profile. The adapter uses Python 3.11+ standard-library transport/provisioning
code and an exactly recorded upstream runtime dependency graph. Existing
repository tooling continues to apply, and runtime installation belongs to
explicit provisioning.

## Acceptance still required

Contract tests, synthetic failures and an actual model smoke each supply evidence
for their exercised paths. Their commands and outcomes belong in the retained
validation record. Model quality, throughput, isolation and deployment claims
need separately identified evidence.

| Plan increment | Status and remaining gate |
|---|---|
| N1 | Local adapter implementation, with execution evidence recorded separately |
| N2 and E2/E3 | `NOT_DONE`: authenticated scope and pre-persistence handling across ingress, dispatch, results and retrieval |
| N3 | `NOT_DONE`: durable safe cases, registered notification operations, revocation and uncertain-delivery reconciliation |
| N4 | `NOT_DONE`: integrated examples and the selected CSO consumer base |
| E4/E5 and N5 | `NOT_DONE`: payload lifecycle, restore, worker isolation and measured deployment qualification |

Protected profiles remain held while their required capabilities are unavailable.
See [implementation sequence](nehemiah-implementation.md) for shared dependencies.
