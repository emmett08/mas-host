# Privacy assessment foundation

This crate makes privacy findings refer to exact source bytes and keeps coverage
separate from handling authority. Its first N0 fixtures implement bounded UTF-8 input,
validated spans and coverage, overlap merging and conservative decisions from the 22
October 2026 Nehemiah PII plan. The scanner recognises synthetic ASCII email/credential
patterns. A complete pattern pass still leaves overall inspection **partial**, recording
`neural-unqualified` as an omission.

Run the demonstration and conformance checks from the repository root:

```sh
cargo run --locked -p mas-host-privacy --example synthetic
cargo test --locked -p mas-host-privacy
```

The demonstration prints category names, coverage and dispositions. It performs
no network requests or notifications and requires no model assets or credentials.

## Contract and ownership

`Source` owns immutable text and a fresh opaque 256-bit source token obtained from Linux
`/dev/urandom`. A private in-memory fingerprint additionally checks content equality.
Serialised assessment metadata contains the opaque token, source ID and length, while
source values and raw fingerprints stay outside that metadata. Reconstructing an
assessment requires the exact retained `Source`. A new source instance, including one
reusing the same ID and length, receives a different token and rejects an old
assessment. The caller must retain its source during the adapter exchange and
authenticate that exchange. The token provides correlation, while the trusted source
owner and adapter establish authenticity. Durable commitments and restart identity
belong to the E2/N2 host integration. That integration must retain original-request
conflicts and govern key and identity lifecycles.

`mas-detection-assessment/1` is a closed fixture record with exact detector, taxonomy,
extraction, domain, offset and overlap versions. Unknown fields and versions fail. This
N0 record uses detector identity `mas-synthetic-patterns/1`, with `neural_qualified:
false`. The prototype taxonomy covers person, location, organisation, email address,
phone number, account identifier, credential and sensitive context. The implemented
recognisers exercise only synthetic email and credential cases. Each match records its
recogniser provenance. Probability estimates would require separately defined and
qualified scores. N1 adds a separate neural record and combined taxonomy with an
explicit label mapping and score semantics. The N0 fixture record retains its original
eight-category taxonomy.

Ranges use half-open UTF-8 byte offsets. Every boundary must be valid for the
original text. Complete coverage requires ordered ranges with no gaps, including
the final byte, and no omissions. Positive findings can coexist with partial,
failed or unsupported inspection, but must lie inside checked ranges. Canonical
findings are ordered and disjoint. `union-overlap/1` merges overlapping spans,
preserving all categories and recognisers. Adjacent spans remain separate.
The scanner rejects a finding-limit overflow rather than dropping later findings.

`decide` checks the source binding and preserves external execution certainty
independently of payload disposition. Prohibited credential findings route to
the symbolic security route, other prohibited findings to the data-owner route,
and incomplete inspection or unavailable policy to service operations. These are
suggested routes for fixture decisions. The caller's trusted configuration owns
recipient resolution and any eventual notification admission.

The protected profile holds even a complete synthetic assessment until neural quality
and the complete protected host path are qualified. `PermitSynthetic` is available only
for explicitly constructed complete synthetic fixtures with current policy and authority
inputs. It supplies no dispatch or disclosure permission.

## Evidence boundary

Conformance covers changed-source replay, malformed and Unicode byte offsets,
coverage gaps and reordered chunks, positive incomplete findings, overlapping
provenance, strict versions/fields, configured bounds, authority outages and
known-completed/unknown execution with withheld output. Source snippets and
fingerprints are excluded from public metadata and debug output. Span metadata,
source IDs, opaque tokens and categories may themselves be sensitive, so their
storage and delivery require the caller's governed access.

The N0 input contract accepts literal UTF-8 text. JSON-field extraction needs an
original-byte mapping through unescaping, while PDF, email and archive extraction need
separately supported format adapters. Bound input transport before parsing into
`serde_json::Value`.

N1's pinned local neural adapter and synthetic qualification are implemented. See [the
local detector guide](../../docs/privacy-detector.md) for provisioning, inspection,
bounds and retained evidence. Population quality remains unqualified. The
[implementation sequence](../../docs/nehemiah-implementation.md) records subsequent N2
structured mediation and retained-reader work, plus the first N3 case/notification
slice. Complete mediation, protected storage, lineage and case lifecycle remain wider
host obligations. Deletion and deployment qualification also require their own evidence.
This crate preserves the existing host configuration, journal and public SDK contracts.
Its fixture checks establish source and assessment conformance, with privacy, isolation
and exactly-once claims requiring separately qualified mechanisms.
