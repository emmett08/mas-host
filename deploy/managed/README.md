# Single-owner Kubernetes deployment

This package wires the maintained `mas-host serve-a2a` entry point into a
bounded Kubernetes deployment. It accepts an approved application image and
an identified external acquisition image through fixed command arguments.
Select the existing CSO acquisition package after qualifying its image,
command and export contract for this placement. The existing host owns
execution, replay, task authentication and governed delivery. The acquisition
application owns source semantics and its bounded export contract.

The supported execution boundary is **trusted-local**, including trusted native
callbacks. Kubernetes placement adds explicit credentials, storage, resource
and network boundaries. It supplies no hostile-worker isolation theorem.
`managed-cluster` remains an explicit selection whose required capabilities
must pass the maintained host's `check-profile`. A failed check stops rendering
and also stops the generated Pod's init container. No fallback changes it into
a trusted-local invocation.

The renderer supplies concrete host flags, a loopback-only A2A listener and a
TLS reverse proxy. The proxy is part of the trusted host Pod. Untrusted workers
require a separately qualified worker transport and separate Pod/sandbox.
The current native guardian executes children inside the host's trust boundary.

## Prepare the existing producer and consumer

1. Build an application image containing the identified host executable,
   immutable retained bundle, executables and complete approved runtime closure.
   Preserve each executable's absolute paths from the bundle. Build dependencies
   without deployment credentials. Record the source, protocol pin, bundle and
   image digest. Image building is an independent qualification step.
2. Select the existing CSO acquisition package and its exact image digest.
   Its fixed command must consume `NEHEMIAH_OBSERVATION_NAMESPACE`,
   `NEHEMIAH_OPERATION_ID` and `NEHEMIAH_EXPORT_DIRECTORY`, bound output size,
   report missing coverage/overflow and publish atomically. A replacement Job
   uses the same original operation identity. Job/Pod names confer no permission
   to fetch or mutate. A Job can execute twice despite these bounded settings.
3. Provision three **separate** volume claims. The execution claim must use
   `ReadWriteOncePod` and the selected CSI driver must support it. The export
   claim is writable by acquisition and read-only for the host. The lifecycle
   claim holds the current custody ledger independently of execution backups.
   Configure governance v2 `lifecycle_file` as `/control/lifecycle.sqlite`.
   Preserve that ledger across restores. A stale restored run must meet its
   current ledger before becoming available. Qualify actual storage locks,
   permissions and restore behaviour on the selected driver.
4. Provision the external configuration Secret with `authentication.json`,
   `governance.json` and `authority.json`.
   Their cross-references must use `/etc/nehemiah/...` absolute paths and the
   selected public export/scopes. Set the governed location to the actual
   deployment. Admission requires the grant's current authority and policies.
   The host supplies its own validation at startup; rendering never inspects
   external Secret values. Configuration mounts use mode `0440`, group 65532.
   Provision the separate commitment Secret with a 32-byte `commitment.key`.
   Set governance's key path to `/private-keys/commitment.key`. A fixed init
   command copies that key into bounded memory storage as UID 65532, mode
   `0600`, as required by the host's private-key validator. The host image must
   include `/bin/sh`, `/bin/cp` and `/bin/chmod` for this fixed command. No
   user-selected argument is interpolated into it. The key retains its original
   identity across Pod replacements. Key changes require the host's explicit
   namespace/migration rules. Configure dynamic credential-file custody as
   `/control/credentials.json`, mode `0600`, and use the current grant epoch.
   A trusted operator updates that file atomically. Kubernetes environment
   references are the existing static local credential option and update only
   after Pod restart.
5. Provision the TLS Secret for the HTTPS public endpoint. Supply a pinned
   nginx-compatible proxy image containing `/usr/sbin/nginx` that runs as
   UID/GID 65532. Its certificate and private key are readable by that group.
   Deploy the ingress in the explicitly selected namespace/Pod label scope and
   forward TLS to the generated ClusterIP Service. Set up namespaces and the
   current cluster CA ConfigMap before collector creation.

Copy `config.example.json` and replace all three digest placeholders. The tool
rejects tag-only images, example zero digests, arbitrary environment values,
duplicate credential names, runtime environment overrides, broad egress CIDRs,
unknown fields and a shared-enterprise profile. Keep the configuration and its
generated resources under the same restricted handling as their source policy.
The configuration digest binds the exact image, source scope, export, credential
reference names and network destinations. Current grant epochs and secret
rotation remain host/adapter authority responsibilities.

The example declares `governance_version: "mas-governed-local/2"` and leaves
`source_publication` false. To enable a source coordinator's runtime publication,
set that explicit boolean true and grant the producer the existing source-read
and `publish-source` permissions under every applicable source policy. The
renderer passes the boolean into the generated A2A2 service configuration and
binds it into the deployment digest. It rejects an absent/local/1 governance
declaration or a non-boolean value when this field is supplied. The declaration
expresses the expected external Secret version. The maintained A2A service
independently parses the mounted governance JSON and rejects an actual version that
does not support source reads. The local rendering receipt therefore keeps
external-Secret validation `NOT_RUN` and source publication at
`startup-validation-required`. Enabling publication on a retained service still
requires the host's explicit service-identity migration contract.

```sh
python3 deploy/managed/prepare.py configuration.json \
  --host target/release/mas-host --output prepared-deployment
```

The local check contacts no cluster or provider. For a review-only render while
required capabilities are unavailable, use `--candidate` in place of `--host`.
Its qualification receipt states `host_preflight: NOT_RUN`. The generated
runtime init check remains present, so it cannot activate an unsupported profile.
The output directory must be new. A failed check leaves no `resources.json`.

Review `resources.json` and apply it through the application's normal release
process. This tool performs no deployment and its output contains no credential
values. The generated service configuration is a Secret holding the exact
selected exports and loopback service paths. Bearer authentication and the A2A
version/extension headers pass through the TLS proxy to the maintained host.

## Declared boundaries

| Component | Granted authority and storage |
|---|---|
| Host Pod | No Kubernetes token/RBAC grants. Execution and lifecycle claims writable, export/configuration read-only. Explicit provider Secret references and single-IP/port egress only |
| Read-only collector Job | Projected short-lived Kubernetes token. Namespaced `get/list/watch deployments.apps` only. Export claim writable. API-server IP/port is its sole egress rule |
| Action identity | Separate ServiceAccount with no binding or mounted token. Mutation remains disabled until the application supplies a separately qualified exact-target adapter and authority |
| TLS proxy | Same trusted Pod, no Kubernetes token or host state mount. Request/access logging and request/response disk buffering disabled |

All ordinary containers run non-root with a read-only root filesystem,
`RuntimeDefault` seccomp, dropped Linux capabilities, no privilege escalation,
bounded CPU/memory and bounded memory-backed temporary storage. No host PID,
network, runtime socket, host mount, node sensor or DaemonSet is installed.
No credential uses `envFrom`, and no package receives authority by its name.

The host has one replica and `Recreate` rollout. Its existing ownership lock
and selected volume semantics remain the authority for overlapping execution.
Replica count and rollout strategy supply no distributed fencing guarantee.
The collector has one completion, zero controller retries and a 300-second
deadline. These settings bound requested work and do not establish exactly-once
external execution. The collector command must retain/reconcile its original
operation identity as the existing application requires.

The collector role deliberately excludes Secrets, ConfigMaps, logs, exec,
nodes and mutations. Extend an identified acquisition capability through its
own reviewed scope. Label filters never replace RBAC. Kubernetes patch
permission also cannot restrict permitted fields. Any future action adapter
must enforce target UID/resourceVersion, allowed fields and current business
preconditions before invoking its effect.

## Qualification

Run the local contracts with:

```sh
python3 -m unittest discover -s deploy/managed/tests -v
```

After an authorised deployment, this read-only command tests effective RBAC and
the execution claim's advertised access mode against the connected cluster:

```sh
python3 deploy/managed/qualify.py configuration.json \
  --output cluster-preconditions.json
```

The caller needs permission to impersonate the selected service accounts for
these checks. Missing access, API errors and malformed responses count as
failed checks. They never count as successful negative RBAC assertions.
The receipt records the server version and exact deployment configuration digest.
It states separate `NOT_RUN`/`UNSUPPORTED` results for the remaining runtime
claims. It preserves a successful precondition result without asserting that
the entire deployment is qualified.

Complete these measured checks on the selected runtime before claiming a
managed release: enforcing CNI egress and ingress probes, image/runtime closure,
collector overflow and missing-coverage outcomes, duplicate Job/restart replay,
overlapping owners and volume locks, deleted/retired state restore, output and
container-log custody, and a second invocation after restricted first-invocation
data. Include source, policy, credentials-by-reference, storage/CSI, CNI and
runtime identities in the retained result. External collectors and providers
retain their separate correctness and lifecycle contracts.

The generated template uses a new single-owner execution namespace. It never
relabels historical journals or migrates existing CSO application runs. CSO
consumer pin changes and retained-run migration remain separate release actions.
