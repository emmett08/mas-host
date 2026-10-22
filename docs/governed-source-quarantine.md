# Exact source quarantine and authorised review

The opt-in `mas-governed-source-quarantine/1` owner retains encrypted original boundary data and its exact assessment, including rejected ingress that has no task yet. It owns payload custody only. It does not queue work, change effect certainty, grant worker access, or turn a human verdict into release permission. Existing profiles without this configuration retain no raw quarantine copy.

Add to a fresh `mas-governed-local/2` configuration:

```json
{
  "quarantine": {
    "version": "mas-governed-source-quarantine/1",
    "payload_file": "/custody-payload/quarantine.sqlite",
    "control_file": "/custody-control/keys.sqlite",
    "max_cases": 128,
    "retention_ms": 86400000
  }
}
```

Both existing parent directories must have private permissions (0700). The stores must have different canonical parent directories and separate backup ownership. Bound execution additionally requires the independent `lifecycle_file`; neither quarantine store may be inside the run directory. Use the v3 privacy inspector (`mas-governed-privacy-inspector/3`), which opts into retaining assessments. Its current scanner contract and configuration remain part of the session identity.

Initialisation is explicit and refuses existing files. Inspection, retrieval and retirement never recreate missing stores or keys:

```sh
mas-host govern-source-quarantine --governance /etc/governance.json \
  --tenant tenant-a --principal principal-a --purpose diagnosis --grant grant-a \
  --actor worker --action initialize --privacy-inspector /etc/inspector.json
```

The same command supports `list`, `read`, `adjudicate`, `hold`, `release-hold`, `expire`, and `retire`. Exact case actions add `--reference quarantine-…`. Adjudication adds `--review FILE` containing `review_key`, `verdict` (`false-positive`, `confirmed-finding`, or `inconclusive`) and a closed reason (`authorised-source-review`, `domain-context`, or `insufficient-evidence`). An inconclusive verdict requires insufficient evidence. Reusing a review key with a changed verdict fails. Reads and returned review records are sensitive; the authorised local caller owns their output handling.

List replies contain at most 64 records. Follow `next_after` using `--after` until `complete` is true. Empty pages can continue. Expired namespace cases are omitted while unrelated rejected-ingress cases remain discoverable. Missing or malformed custody/control state fails closed. Public Rust APIs supply the same bounded operations; A2A does not expose a raw-source method.

Every retention requires current `retain-source` and `retain-assessment` permission for recipient `host-source-quarantine`, destination `quarantine-payload`, conjoined with the full inherited source policies. Policy byte limits cover the actual encrypted envelope. Discovery requires `list-source-quarantine`; exact source reads require `resolve-source` and `resolve-assessment`; adjudication separately requires `adjudicate-source` and `adjudicate-assessment`. These disclosure/review tuples use the current principal as recipient and `quarantine-review` as destination. Their full source ancestry is rechecked. The initial administrative lookup uses recipient `host-source-quarantine`, destination `quarantine-control`, with the corresponding operation. Initialisation requires `quarantine-admin`, hold changes require `hold-source`, and destructive/expiry changes require `retire-source`. No permission follows from a detector result.

Encrypted plaintext contains the exact canonical original and projected JSON, the assessment or bounded failure record, and complete inherited exposure metadata. Decoded string contents are preserved exactly; transport whitespace and HTTP headers are not copied. Completed assessments bind component scope/index, exact string commitment and category/span metadata. Partial assessments retain completed component metadata; a scanner failure retains bounded failure status and exact original. Scanner exception text is not retained. New inspection attempts may produce distinct assessment identities and cases; old references remain immutable. Retrieval rechecks inspection without acquiring another raw copy. A case captured during effect completion resolves its exact pending effect source from the existing journal before review; unresolved ancestry is withheld and never treated as empty or fetched again.

The plaintext case limit is 30,000 bytes, with a complete ciphertext envelope of at most 65,536 bytes. There are 1–4096 immutable case reservations and at most 128 reviews per case. The retention interval is 1 millisecond–7 days and cannot outlive admitted policy/source expiry. Capacity includes retired reservations; exhaustion withholds admission. Oversized custody or incomplete ancestry fails closed without truncating an assessment.

Encryption uses pinned [RustCrypto AES-GCM 0.10.3](https://docs.rs/aes-gcm/0.10.3/aes_gcm/) AES-256-GCM. Every source case and every immutable review receives an independent random 256-bit key and 96-bit nonce from the operating system. Associated data binds the exact case/review, owner, profile, context, assessment, store epoch and admitted expiry. Keys are held only in the private independent control ledger. No source plaintext, user-supplied review keys or review verdicts are stored in plaintext columns. Source and review ciphertext commits before its key transaction; a crash can leave inaccessible ciphertext. Review retries reuse an authenticated immutable record; an orphan ciphertext without a committed key is replaced under a fresh key. Key buffers and encoded plaintext byte buffers use `zeroize`; this does not erase all parsed JSON, scanner memory, crash dumps or process copies.

Reads and reviews hold namespace lifecycle admission before acquiring the quarantine control transaction, through the final authority check. Retirement serialises with these operations. Namespace context includes canonical directory, run, random retained nonce and fingerprint; reusing a run name in another state root cannot alias custody. Expiring/retiring a namespace blocks its raw-source reads. Expiring or retiring a bound source case conservatively shortens access to the whole namespace before key retirement, withholding derived views while preserving unresolved effects and duplicate-rejection identity. The namespace is this profile's invalidation unit. Rejected ingress has no namespace and uses its own case lifecycle.

A hold prevents destruction but does not extend access. Retirement commits source and review key tombstones before removing their ciphertext. Old payload-only backups remain unreadable against the current independent control ledger. Losing a ledger fails closed. Restoring an old control/key backup as well as old ciphertext is outside that guarantee; operators must preserve current deletion/revocation truth and keep key backups separate. Logical retirement, local purge and cryptographic inaccessibility under the current ledger are distinct from physical-media erasure. External review output, replicas, historical plaintext journals and already issued worker credentials are not erased by this owner. Current-authorised cleanup is implemented; an independently authenticated platform recovery identity after all user grants are revoked remains a separate prerequisite.

This trusted-local implementation does not remove the managed profile's hostile-worker isolation gate or establish detector accuracy, language coverage, incident-route qualification, a deletion-service objective, or live deployment/backup restoration qualification.
