# Privacy assessment foundation

This crate implements the first N0 contract fixtures in the 22 October 2026
Nehemiah PII plan. It admits bounded UTF-8 text, validates exact byte spans and
coverage, merges overlapping findings and computes conservative fixture handling.
The scanner recognises synthetic ASCII email/credential patterns. Its full
pattern pass leaves overall inspection **partial**, with `neural-unqualified`
recorded as an omission.

Run the demonstration and conformance checks from the repository root:

```sh
cargo run --locked -p mas-host-privacy --example synthetic
cargo test --locked -p mas-host-privacy
```

The demonstration prints category names, coverage and dispositions. It performs
no network requests or notifications and requires no model assets or credentials.

## Contract and ownership

`Source` owns immutable text and a fresh opaque 256-bit source token obtained
from Linux `/dev/urandom`. A private in-memory fingerprint additionally checks
content equality. Serialised assessment metadata contains the opaque token,
source ID and length, while source values and raw fingerprints stay outside that
metadata. Reconstructing an assessment requires the exact retained `Source`.
A new source instance, including one reusing the same ID and length, receives a
different token and rejects an old assessment. The caller must retain its
source during the adapter exchange and authenticate that exchange. The token
provides correlation, while the trusted source owner and adapter establish
authenticity. E2/N2 must supply durable protected commitments, original-request
conflict retention, key/identity lifecycles and restart recovery.

`mas-detection-assessment/1` is a closed fixture record with exact detector,
taxonomy, extraction, domain, offset and overlap versions. Unknown fields and
versions fail. Its only implemented detector identity is
`mas-synthetic-patterns/1`, with `neural_qualified: false`. The prototype taxonomy
covers person, location, organisation, email address, phone number, account
identifier, credential and sensitive context. The implemented recognisers
exercise only synthetic email and credential cases. They provide matching
provenance without invented probabilities. N1 adds a separate neural record and
combined taxonomy with an explicit label mapping and score semantics. The N0
fixture record retains its original eight-category taxonomy.

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

The protected profile holds even a complete synthetic assessment because neural
quality and the protected host path remain unqualified. `PermitSynthetic` is available
only for explicitly constructed complete synthetic fixtures with current policy
and authority inputs. It supplies no dispatch or disclosure permission.

## Evidence boundary

Conformance covers changed-source replay, malformed and Unicode byte offsets,
coverage gaps and reordered chunks, positive incomplete findings, overlapping
provenance, strict versions/fields, configured bounds, authority outages and
known-completed/unknown execution with withheld output. Source snippets and
fingerprints are excluded from public metadata and debug output. Span metadata,
source IDs, opaque tokens and categories may themselves be sensitive, so their
storage and delivery require the caller's governed access.

JSON field extraction is explicitly unsupported until original-byte mappings
through unescaping are implemented. PDF, email and archive extraction are also
unsupported. Input transport must be bounded before parsing into `serde_json::Value`.

N1's pinned local neural adapter and synthetic qualification are implemented.
See [the local detector guide](../../docs/privacy-detector.md) for provisioning,
inspection, bounds and retained evidence. Population quality remains unqualified.
N2 ingress/result persistence enforcement, protected storage, lineage, current
policy integration, deletion and restart qualification remain unimplemented.
N3 durable cases and notification operations remain unimplemented. This crate
changes no existing host configuration, journal or public SDK shape and supplies
no privacy, isolation, exactly-once or deployment-quality guarantee.
