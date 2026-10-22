# Structured Kubernetes evidence packaging fixture

Two ordinary command implementations produce the same bounded Deployment
observation: a checked TypeScript capability and a Python collector. They use the
host's existing `mas-agents/3` command registration, immutable file bindings,
operation schemas and admitted invocation identity.

The fixture reads one synthetic Deployment snapshot. Its target contract binds
the cluster and namespace incarnations, resource kind/name, UID and exact
resource version. It reports exact generation and replica quantities. Both
implementations retain `assessment: NOT_RUN` and `freshness: NOT_RUN` because
pods, images, events, logs and live freshness are outside this fixture's scope.

The input selects an operation identity and the expected target. The packaged
fixture supplies the source. Endpoints, command arguments, file paths and
credentials are configured outside the input. The collectors perform no
network calls or Kubernetes mutations.

## Build and check

From the repository root:

```sh
npm --prefix examples/structured-evidence/typescript ci --ignore-scripts
npm --prefix examples/structured-evidence/typescript run check
npm --prefix examples/structured-evidence/typescript run build
python3 -m unittest discover -s examples/structured-evidence/tests -v
```

TypeScript 5.8.3 is the sole build dependency and is locked with its package
integrity. The compiled module has no runtime package dependencies. The command
wrapper uses Node built-ins. The Python collector uses only its interpreter's
standard library. Keep build environments free of deployment credentials.

Create separate, new project directories for the two implementations:

```sh
e1_root=$(mktemp -d)
python3 examples/structured-evidence/prepare.py --language python --destination "$e1_root/python"
python3 examples/structured-evidence/prepare.py --language typescript --destination "$e1_root/typescript"
```

`prepare.py` resolves and hashes the selected interpreter, copies the declared
file closure and writes `agents.toml`, shared operation schemas, a topology,
input, capability declaration and demonstration approval. Select another local
interpreter explicitly with `--runtime ABSOLUTE_PATH`.

For each generated project, inspect `capability.json` and `approvals.json`.
The supplied approval deliberately approves that exact synthetic package for
this trusted-local demonstration. Production approval belongs to the operator.
Installing a declaration supplies neither credentials nor a live target grant.

Build and run the Python package with the host:

```sh
target/debug/mas-host check-capability --declaration "$e1_root/python/capability.json"
target/debug/mas-host run --project "$e1_root/python" --agent evidence_python --input "$e1_root/python/input.json" --state "$e1_root/python-state" --run e1-python --capabilities "$e1_root/python/capabilities.json" --capability-policy "$e1_root/python/approvals.json"
```

Substitute the `typescript` project, `evidence_typescript` agent and distinct
state/run identities to exercise the TypeScript implementation. Compare their
outputs after accounting for the different admitted `invocation_id` values.
Both outputs retain the original application `operationId` and exact target.
Replaying a completed run reuses its retained observation. A new source read
requires a new operation identity and current admission.

The generated runtime record pins the interpreter binary. The trusted-local
deployment still owns its standard library, shared libraries, platform and
working directory. Stronger runtime closure and isolation need separately
qualified deployment controls. Credential epochs and current disclosure
admission remain E2 obligations.

## Source characterisation

The fixture preserves the following existing ownership decisions:

| Inspected source | Existing contract | Fixture consequence |
|---|---|---|
| [Self-healing Capability](https://github.com/emmett08/mas-host-examples/blob/main/examples/self-healing/tools/capabilities.ts), blob `d3c22d6d0fecf922911986b911a51563b1b61f21` | Schemas, implementation identity, effect class, resources, credential owner, bounds and reconciliation belong to the capability | `declareCapability` retains these fields for a credential-free bounded read |
| [runTrustedOperation](https://github.com/emmett08/mas-host-examples/blob/main/examples/self-healing/host/tool.ts), blob `180792bd19a9417659ae6edf6ee14e8c9ffb3037` | Trusted composition fixes executable/arguments and correlates application `operationId` with admitted host `invocation_id` | Fixed file arguments and stdin preserve this relation without model-selected executables |
| [CSO Kubernetes collector](https://github.com/emmett08/catalog-sync-operator-monograph/blob/f57969820458312e81aff9e8d782ce89db80fe3e/apps/mas-remediation/live_kubernetes.py) | Exact cluster/namespace/resource identity and integer quantities precede application readiness assessment | The fixture projects that bounded point contract and leaves the broader assessment unrun |
| [CSO observation contract](https://github.com/emmett08/catalog-sync-operator-monograph/blob/f57969820458312e81aff9e8d782ce89db80fe3e/apps/mas-remediation/live_common.py) | Native result, target, freshness and provenance are separate, with exact integer JSON | Synthetic provenance and missing live freshness remain explicit |

These are source-grounded characterisations. The example supplies packaging and
substitution evidence over synthetic data. Live acquisition, provider permissions,
source freshness, retained-run migration and protected deployment require their
own implementation and validation.
