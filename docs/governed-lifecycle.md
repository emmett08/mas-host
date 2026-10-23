# Governed namespace lifecycle

The opt-in `lifecycle_file` field in `mas-governed-local/2` selects a private,
independently retained SQLite control ledger. Configure it before starting a new
namespace. Existing namespaces require an explicit migration. Keep this ledger
outside run directories and restore it from the current control history,
separately from execution backups.

```json
{"lifecycle_file":"/control/lifecycle.sqlite"}
```

A fresh namespace registers its original run, nonce and profile fingerprint.
Every subsequent engine open, live execution check and retained source/effect
read requires the current registration. A missing ledger or registration stops
access. A retired run cannot register another nonce under the same profile.
Restoring an old run backup therefore cannot restore its delivery or execution
permission while the current control ledger is retained.

The local operator administers a stopped namespace through the existing host:

```sh
mas-host govern-lifecycle --state /state/runs --run RUN \
  --governance /etc/nehemiah/governance.json \
  --tenant TENANT --principal PRINCIPAL --purpose PURPOSE --grant GRANT \
  --action hold
```

Actions are `hold`, `release-hold`, `expire` and `retire`. `expire` also requires
`--expires-at-ms N`; expiry can be shortened and cannot be extended. A hold
prevents destructive retirement while preserving the original access expiry.
Include `--privacy-inspector FILE` when that inspector belongs to the retained
profile. A2A runs additionally use `--service-state /state` so the command takes
the service lock and purges that task's retained service output and error.
The service and execution owner must both be stopped. The selected principal
still needs a current grant; a separate administrative recovery identity is a
remaining integration requirement for retirement after user-grant expiry.

Unresolved effects block destructive retirement so their exact recovery descriptors and reconciliation obligations remain available. The operator must resolve them with external evidence or provide a separately qualified recovery-custody contract. Access expiry can still withhold their payloads. Retirement then commits its tombstone before purging payloads. It retains effect IDs,
step states and conservative external-execution certainty in
`governed_retired_effects`. It removes input/output/evidence payloads from the
execution journal, source/exposure/context records and revision payload tables.
It retires the whole local protocol file through file ownership, without editing
the protocol's private tables. SQLite secure deletion, checkpointing and vacuum
cover the selected local journal. Failure leaves the control tombstone committed;
retrying retirement completes the remaining purge and grants no execution.

`runtime::lifecycle::with_admission` holds a control-ledger read transaction over
an additional payload writer's journal transaction. A retirement tombstone
serialises after an admitted writer. New writers must use this boundary so a
late commit cannot repopulate a retired namespace.

This is conservative namespace-level retirement. Derived data in the namespace
becomes unavailable together. Individual-payload retention, automatic physical
purging at expiry, external provider copies, application stores, exported files,
replicas and storage-device erasure require their own lifecycle integration.
The operation reports logical retirement and local-store purge, never
cryptographic or physical-media erasure. Restoring a stale control ledger along
with stale payload backups defeats the independent-current-control assumption.

An opt-in [source quarantine](governed-source-quarantine.md) separately owns
encrypted exact-source and failed-assessment custody. Namespace retirement also
retires its bound source/review keys and purges ciphertext. Retiring or expiring
one bound case conservatively withholds dependent namespace views while leaving
execution certainty and unresolved obligations intact.

# Current credential references

The optional `credentials_file` field resolves declared worker environment
references immediately before each new worker launch. It replaces environment
lookup for those declared names. The private regular file must have mode `0600`
and contain a bounded `mas-governed-credentials/1` record:

```json
{"version":"mas-governed-credentials/1","credentials":[{
  "tenant":"tenant-a","principal":"principal-a","grant_ref":"grant-a",
  "grant_epoch":1,"actor":"collector","destination":"local-collector",
  "reference":"PROVIDER_TOKEN","credential_epoch":1,
  "expires_at_ms":1799999999999,"value":"SYNTHETIC_SECRET"
}]}
```

The host requires exact tenant, principal, grant epoch, actor, destination and
reference matches. Missing, expired or ambiguous credentials stop the launch.
Records cannot add environment names to the retained bundle. The host rereads
current authority after resolution and retains only credential epochs and keyed
commitments. An older credential epoch, or a changed value under the same epoch,
fails the journal admission check. Incrementing `credential_epoch` rotates the
secret without changing the bundle or operation identity. Completed work reuses
its retained result and performs no credential lookup or new provider call.

Secrets stay outside bundles, effect descriptors and start frames. Already
running trusted native workers retain the credentials they received. This
mechanism supplies current launch-time resolution; enforcing revocation against
hostile workers still depends on qualified process/credential confinement.
