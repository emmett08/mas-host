# Read retained governed effects

The first E3 resolver increment provides an authorised projection of one retained
effect from the existing execution journal. The privacy-response adapter uses
two such projections to reconstruct case status after restart. The existing
task, effect and protocol journals retain their execution responsibilities.

## Authority and identity

Trusted host callers supply an authenticated `GovernedSession`, a governed run
directory, an actor, the parent invocation and the logical effect key. The
reader checks current authentication, grant, key, detector configuration and a
separate `resolve` policy permission before looking up a record. The selected
namespace must match the session's retained fingerprint.

Declare the `resolve` rule when creating the governed namespace. Adding it to
an existing retained policy changes that namespace's identity and requires an
explicit migration. The A2A service's existing host-binary identity check also
continues to apply when upgrading the executable.

The HTTP service derives the run directory, actor and root invocation from an
already authorised A2A task. Its `GetRetainedEffect` request accepts the task ID
and logical key. A task ID or effect key selects a record within that scope.
Current policy determines access to its existence, metadata and payload.
The `resolve` policy operation governs this retained-data read. Effect
reconciliation retains its separate execution contract.

The HTTP extension is `urn:mas:host-governed-effect-read:1`. Clients activate it
alongside the existing `urn:mas:host-task-service:1` extension. The Agent Card
advertises the optional read extension for governed A2A2 services. See
[A2A usage](a2a.md) for the exact request and negotiation contract.

## Read and delivery

The reader opens the execution journal with SQLite's read-only flags and uses
one transaction for the selected namespace, parent, effect and completion
receipt. Queries select exact identities and use bounded retained values. The
read transaction ends before detector callbacks run. An active execution owner
can continue using its existing journal connection.

The reader validates the retained descriptor, output and supporting evidence
against the selected effect's current schemas. It inspects the application
descriptor, output and reported evidence. Separately validated host receipts
retain their exact binding and configuration checks. Policy authorises the whole
returned envelope, including identity, state, certainty and receipt metadata.
A final current-authority check covers changes observed
during inspection. These remain file-snapshot checks under the trusted-local
profile's existing authority contract.

Inspection retains the configured component, byte and deadline limits. The
current extraction contract admits at most 128 text units and 16,384 combined
original/projection bytes per boundary. A larger retained application payload
can therefore produce a metadata-only view even when its original effect was
valid. The whole response must also fit the policy's byte allowance and the
65,536-byte representation limit.

`mas-governed-effect-view/1` keeps the following dimensions separate:

| Dimension | Meaning |
|---|---|
| Namespace, actor, parent invocation and key | Identity of the selected retained record |
| Effect state | The committed execution checkpoint observed by this read |
| Host settlement | Whether a consistent retained host completion exists |
| External execution | The certainty retained in that completion receipt |
| Payload disposition | Whether the selected descriptor/output/evidence can be delivered |

A known completion survives payload withholding. A query-inspection failure
stops resolution before lookup, including when a completion is retained.
An unresolved effect keeps its uncertainty. An explicitly authorised
`not-recorded` view reports the absence of
the selected effect under a verified parent. Denied or unavailable resolution
has a uniform failure response. Callers must preserve that distinction when
building status views.

Reads preserve journal contents and execution state. They perform observation
without recovery, acknowledgement, expiry reconciliation or external dispatch.
An unavailable retained result therefore supplies no permission to fetch or
execute again. Ordinary journal inspection continues to require the ordinary
namespace contract.

## Privacy case consumer

`privacy_response.case_status` validates the retained safe-case intent and
notification views, including their common namespace, actor, parent, event and
destination binding. It derives the case identity from the original invocation
and preserves accepted, refused, throttled, not-sent, uncertain and withheld
outcomes. Recipient delivery remains unknown.

The two effect reads are independent snapshots. The resulting case projection
reports the observations it received. It supplies no atomic multi-record history
or assertion that a later state has already occurred. The pure projection creates
no execution authority and performs no network or journal operations.

The example exercises this path through the actual CLI service after execution
and restart. Its detector and notification endpoint remain deterministic
fixtures. Current-authority and cross-scope failures exercise the same read
boundary.

## Ownership, verification and next variation

The core owns exact journal resolution, namespace identity and current delivery
checks. The CLI owns authenticated task selection and extension negotiation.
The notification adapter owns the case interpretation of its own registered
effect formats. The existing authority file and journal remain their respective
sources of authority and execution facts.

Implementation checks and the finite control model are retained with exact
source hashes under `evidence/nehemiah-governed-reader/`. The model checks the
declared lookup, inspection, delivery and read-only control rules. Concrete
tests exercise the actual journal and service boundaries.

This increment supplies the read boundary needed by later resolver consumers.
Complete source ancestry, per-invocation exposure inheritance, shared
subscribers and more-than-32-reader/ancestor qualification remain subsequent E3
work. Case adjudication requires authorised exact assessment/source access.
Durable retry, escalation and reconciliation remain subsequent N3 work. The
managed/shared deployment gates and the existing unstructured-release
qualification requirements continue to apply.
