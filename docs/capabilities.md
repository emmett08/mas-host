# Approved local capability packages

Approve an exact command package before allowing the builder or a retained bundle to use
it. E1 adds this approval boundary to the existing `agents.toml` command path. A package
binds the operation to its implementation bytes, declared files and runtime
requirements, with an explicit recovery rule. The first supported package reads
structured Kubernetes evidence in TypeScript or Python.

The [structured evidence example](../examples/structured-evidence/README.md)
supplies both implementations, strict schemas and a deterministic source.
Their declared target, exact integer quantities and application `operationId`
survive packaging. The host supplies the admitted `invocation_id` through the
existing command adapter. Broader native-worker packaging and managed capability
activation remain separate increments.

## Declare and approve

Keep ordinary command registration in `agents.toml`. Its explicit sidecar has
exactly `version: mas-capabilities/1` and a `packages` array containing 1–31
declaration objects. The example generator supplies complete usable files.
Each declaration has these closed, versioned fields:

| Field | Required binding |
|---|---|
| `version` | `mas-capability/1` |
| `package_id`, `agent`, `operation` | Bounded package identity and exact existing agent/operation |
| `working_directory` | Existing project-relative working directory |
| `implementation_sha256` | Canonical resolved command specification, including fixed argv, executable path, stdin/stdout, success codes and limits |
| `executable_sha256` | Exact command executable bytes |
| `files` | Project-relative paths and SHA-256 digests for every file argument and declared source/schema/lock closure |
| `runtime` | Approved runtime ID and explicit absolute file paths with digests |
| `input_schema_sha256`, `output_schema_sha256` | Canonical supported operation schemas |
| `requires` | Named grants, credential references and destinations |
| `limits` | Exact command `timeout_seconds` and `max_output_bytes` |
| `recovery` | `effect: read`, `replay: retained-observation`, `unknown: operator-reconciliation` |

Both operation schemas are required. Unsupported schema keywords fail through the
existing schema validator. Packaged commands use fixed arguments with JSON stdin and
JSON-object stdout. Input-derived argv is rejected. The declared credential-reference
set must equal the agent's existing environment-name set. The existing environment
declaration and target grants remain authoritative when a package is installed.

An absolute executable must appear in the runtime file list. Project files stay
within the project, use ordinary relative path components and reject symlinks.
Each project file is at most 16 MiB, with at most 64 MiB in the package's declared
project closure. Runtime files are at most 1 GiB each and 2 GiB in total. Each
file list contains at most 64 records. Sidecar parsing and the complete retained
bundle retain the existing 65,536-byte representation bound.

The generated example approval has this shape:

```json
{
  "version": "mas-capability-approvals/1",
  "profile": "trusted-local",
  "approved_packages": ["CANONICAL_DECLARATION_SHA256"],
  "requirements": {
    "grants": ["synthetic-deployment-read"],
    "credential_references": [],
    "destinations": ["host-local"],
    "runtime_ids": ["IDENTIFIED_LOCAL_RUNTIME"]
  },
  "require_attestation": false
}
```

Use an explicit operator-controlled approval file. `check-capability` validates
the declaration's shape and returns its canonical digest:

```sh
mas-host check-capability --declaration capability.json
mas-host build --project PROJECT --capabilities capabilities.json --capability-policy approvals.json
```

Project-relative sidecar and approval paths resolve under `PROJECT` during
build or project-run. A retained bundle uses the explicitly supplied policy path
from the caller's current directory:

```sh
mas-host run --bundle BUNDLE --agent ACTOR --input INPUT --state STATE --run ID --capability-policy APPROVALS
```

The operator approves the exact declaration digest and supplies each named static
requirement. Keeping a package name while changing its bytes, command arguments,
schemas, required runtime or declared closure requires new approval. `check-capability`
reports `approval_granted: false`. Approval and file/binding checks occur separately
during build and execution. Policies requiring signatures and stronger deployment
profiles remain unavailable until their verifiers and enforcement capabilities are
implemented.

## Retained identity and dispatch

The builder embeds `mas-capability-binding/1` metadata inside the existing worker
configuration. Existing `mas-agents/3`, `mas-bundle/3`, native frames, public handler
APIs and core execution policy retain their schemas and responsibilities. Projects that
omit a sidecar follow the existing build path.

Declared project files are copied through the existing immutable resource
retention mechanism. Runtime files remain digest-bound in their approved
locations. The complete command, schemas, declaration, initial approval digest
and retained paths participate in the bundle identity. Aggregate-size validation
precedes publication, preserving the previous valid bundle after rejection.

Retained execution rechecks current explicit package approval, static
requirements and file identities. Every declared retained project file and
runtime file also enters the existing command `assets` list. The maintained
built-in runtime rechecks those digests before admitting the external command
effect. This dispatch check also applies to alternate core callers that use the
same retained command configuration. The CLI's policy preflight supplements
that execution-time identity check.

Retained package validation also requires the current host's maintained command
runner digest, exact `__builtin` launch arguments and the closed command
configuration. A workflow/LLM kind, additional configuration fields or substituted
worker runner prevents packaged execution. Updating the host runner requires
rebuilding the package bundle under the new host and preserving any old retained
run with its original identified host.

Completed command effects reuse their original retained observations. Replacing
an authoring file leaves an old bundle's retained resource unchanged. A fresh
build must match the newly declared bytes and obtain approval for them. Changed
in-place runtime files prevent fresh dispatch. Lost or uncertain external
outcomes continue through the existing evidence-backed reconciliation path.

The selected profile trusts the local operator and registered implementation. Digests
establish byte identity under the declared immutable deployment assumptions. Publisher
approval, signatures and model or source quality retain their own evidence obligations.
The runtime ID and declared file list express an approved dependency contract. The
operator must keep the declared runtime closure complete and immutable, including
standard libraries, shared libraries and other installed state required by the command.
Managed isolation, current grant epochs and credential rotation remain E2/E5
requirements.

## Design and verification

| Alternative | Consequence | Selection |
|---|---|---|
| Sidecar plus existing builder and command asset gate | Adds approval and identity checks while preserving current execution ownership | Selected |
| New agent/bundle schema and generic plugin registry | Adds migrations and a second package lifecycle before a governed path needs them | Deferred |
| Existing command registration alone | Retains local execution while leaving package approval and prerequisite checks to external convention | Preserved for unpackaged local applications |

The capability module separates declaration validation, operator approval,
bounded file verification and retained command binding. The existing resource
retainer remains responsible for publishing the exact hashed bytes. The builder
selects and binds packages while command dispatch continues to own execution.
These extractions follow independently changing contracts and failure modes.

Rust follows the existing 2021 edition, selected toolchain, formatter and Clippy
profile. Example preparation uses Python's standard library. The example's
TypeScript compiler is locked separately and its generated command uses no
runtime package dependency. Dependency builds use an environment without
deployment credentials.

The focused CLI tests exercise real collector execution, retained replay, same-name
substitution, missing prerequisites, incompatible runtimes, strict versions and schemas,
file escapes, changed argv, runtime drift, dispatch asset checks and aggregate bundle
rejection. The TypeScript/Python example tests compare the shared structured operation.
Record executed commands and results in the increment's validation evidence. Static
package acceptance establishes the declared approval and identity checks. Managed
disclosure and production execution require their own authority and qualified deployment
controls.
