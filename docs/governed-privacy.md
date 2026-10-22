# Governed local detector mediation

The governed A2A service can require the existing pinned offline detector before
its structured admission, effect, retention and delivery boundaries. Enable this
through the operator's `mas-host-a2a/2` configuration. Existing configurations
retain their previous behaviour when the field is absent.

```json
{
  "privacy_inspector_file": "/absolute/operator/privacy-inspector.json"
}
```

The field belongs beside `governance_file` and `authentication_file` in the
complete [governed local configuration](governed-local.md). The referenced file
must be an ordinary absolute file and contain exactly these fields:

```json
{
  "version": "mas-governed-privacy-inspector/1",
  "python": "/absolute/provisioned-venv/bin/python",
  "worker": "/absolute/mas-host/adapters/privacy/worker.py",
  "manifest": "/absolute/operator/privacy-bundle.json",
  "assets": "/absolute/provisioned-assets",
  "detector": {
    "bundle_sha256": "<64 lowercase hexadecimal characters>",
    "code_revision": "<40 lowercase hexadecimal characters>",
    "model_revision": "<40 lowercase hexadecimal characters>",
    "backend": "opf-cpu-bfloat16/1",
    "adapter": "mas-privacy-opf/1"
  },
  "python_sha256": "<SHA-256 of the interpreter executable bytes>",
  "destination": "local-detector",
  "timeout_ms": 120000,
  "memory_mib": 12288
}
```

Replace each placeholder with the approved manifest/runtime identity from
[local detector provisioning](privacy-detector.md). The manifest hash covers
its exact file bytes. The code/model revisions and backend must match the
manifest. Preserve a virtual environment's interpreter path when calculating
its executable hash so the launcher selects the provisioned environment.

The operator approves the detector and executable identities through this
configuration. The launcher validates the manifest, worker and interpreter at
startup and before every scan. The existing worker verifies its declared code,
assets and installed runtime before inference. Runtime downloads are disabled.
The configuration file is reread before each scan and a changed configuration
stops use by the existing session. Configuration, detector, destination and
inspection contract join the retained session identity. Adopting a different
contract requires a fresh namespace or an explicit future migration.

Inspection requires its own current processing permission. Add a correlated
rule for each authorised workflow actor to every applicable policy, for example:

```json
{
  "operation": "inspect",
  "principal": "alice",
  "actor": "collector",
  "producer": "host",
  "recipient": "privacy-inspector",
  "purpose": "test",
  "destination": "local-detector",
  "location": "local",
  "classifications": ["restricted"],
  "obligations": ["inline-retention-approved"],
  "max_bytes": 16384
}
```

This rule authorises processing the exact original and projected values within
the selected local boundary. The host checks it before sending any component to
the inspector. Existing invocation, effect and delivery rules remain necessary.
Revocation or unavailable policy blocks later inspection and admission.

The host extracts JSON keys and decoded string values into exact immutable UTF-8
sources. It validates each neural response against that source, its random
source token and the approved detector. The existing deterministic recognisers
and overlap rules contribute findings. Identical text within one boundary reuses
the same validated result. Context across separate JSON components remains
outside this extraction contract.

Each boundary admits at most 16,384 encoded bytes for the original and projected
values together, at most 128 text components and one shared deadline of
`timeout_ms`, within 1–120,000 milliseconds. The launcher caps every worker's
deadline to that shared deadline and applies the configured guardian memory
limit, within 256–65,536 MiB. Source text uses the existing private framed channel.
Worker standard output and error are discarded and descendants are reaped before
a result is accepted.

One detector guardian may run at a time for the configured service. The shared
slot remains occupied until descendant cleanup completes. Concurrent excess
inspection fails closed without a queue, retaining the caller's hold/withhold
behaviour. This bounds cold model residency independently of the HTTP connection
count. Queueing, fairness and throughput qualification remain later capacity work.

The HTTP connection budget includes two inspection deadlines plus ten seconds,
up to 250 seconds. A repeated completed `SendMessage` may inspect both its input
and its retained output. Synchronous polling inspects a completed payload once.
Header and body reads retain their three-second bounds, and the connection set
remains limited to 32. Profiles without an inspector retain the ten-second whole
connection bound. Clients using the inspector must configure their response
timeout for the selected inspection budget. A disconnected response preserves
the original task identity and its uncertainty obligations.

Complete coverage is required for every admitted component. Invalid source or
detector identity, missing coverage, malformed output, timeout and worker failure
stop the boundary. Findings in a retained component stop its release. A finding
removed by the already approved structured projection can coexist with a
permitted projection. Inspection receipts retain keyed original/output bindings
and bounded counts without copying private values or span locations.

The structured schema and current policy continue to determine permissible
content. The detector supplies additional inspection evidence. Arbitrary text
release, population precision/recall and managed worker confinement retain their
separate qualification gates.

An effect that has already completed keeps its durable completion when later
inspection fails. The host withholds its payload, retains external certainty
separately and reuses the same effect identity after restart. A lost response
continues to require reconciliation. Changing detector availability cannot
authorise redispatch.

The actual HTTP regression tests use an explicitly synthetic detector worker
through the same guardian and exact-source response contract. They exercise
complete, partial, malformed, wrong-source, positive-finding, timeout and changed
implementation paths, including a completed effect whose output is withheld.
Those fixtures establish host handling behaviour. Actual-model execution and
deployment-domain quality require their own source-bound evidence.
