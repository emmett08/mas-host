# Detector qualification against a frozen corpus

`adapters/privacy/qualification.py` runs the existing `mas-host privacy-inspect`
command against labelled UTF-8 sources and compares its restricted assessment
metadata with independent annotations. It can replay the same source-bound
assessments without another model call. The command uses the Python standard
library and performs no dependency installation, model provisioning or download.

The result describes detector performance under the declared evaluation. Host
release authority continues to depend on the governed handling contract. The
benchmark never changes a host policy, unstructured-release setting or grant.

## Freeze the evaluation

Prepare an authorised JSONL corpus. Every line has this closed shape:

```json
{"version":"mas-privacy-labelled-source/1","id":"document-1","unit_id":"conversation-1","stratum":"en-tickets","text":"Contact Alice, count=7","prohibited":[{"start":8,"end":13,"category":"person","entity_id":"person-1"}],"required":[{"start":15,"end":22}]}
```

Offsets are half-open UTF-8 byte ranges in the original spelling. They must end
at Unicode scalar boundaries. `prohibited` identifies content the declared
handling policy requires the transformation to remove. `required` identifies
facts the downstream task must retain. Contradictory required/prohibited ranges
and duplicate source identities are rejected. An empty annotation list has an
explicit meaning and must be independently reviewed.

`unit_id` groups dependent documents, including turns in one conversation and
material generated from the same source template. Statistical bounds count each
unit once. A unit fails if any of its documents has the relevant failure. Use
one `entity_id` across repeated mentions of the same entity within a unit to
measure whether every occurrence is covered. Supported categories match
`mas-privacy-taxonomy/2`: `person`, `location`, `organisation`, `email-address`,
`phone-number`, `account-identifier`, `credential`, `sensitive-context`,
`postal-address`, `url` and `date`.

The corpus admits up to 100,000 sources, each at most 16,384 UTF-8 bytes, and at
most 256 MiB of encoded JSONL. A line admits at most 65,536 encoded bytes and
128 prohibited and required ranges each. Preserve complete conversations in
bounded sources where possible. Splitting a conversation into sources evaluates
the current per-source detector contract. Cross-source contextual inference
needs its own implementation and qualification.

Freeze this manifest before collecting results. Replace every digest and target
with reviewed values. The example thresholds illustrate a possible evaluation
and require explicit agreement for the intended domain.

```json
{
  "version": "mas-privacy-qualification-manifest/1",
  "corpus_sha256": "<SHA-256 of exact JSONL file bytes>",
  "detector": {
    "bundle_sha256": "<SHA-256 of provisioned bundle manifest bytes>",
    "code_revision": "<40 hexadecimal characters>",
    "model_revision": "<40 hexadecimal characters>",
    "backend": "opf-cpu-bfloat16/1",
    "adapter": "mas-privacy-opf/1"
  },
  "taxonomy": "mas-privacy-taxonomy/2",
  "offset_unit": "utf8-bytes",
  "confidence": 0.95,
  "evidence_scope": "deployment-domain",
  "annotation_evidence": "restricted-evaluation/independent-annotations-v1",
  "sampling_evidence": "restricted-evaluation/held-out-units-v1",
  "independent_units_attested": true,
  "strata": {
    "en-tickets": {
      "min_units": 299,
      "min_negative_units": 299,
      "max_residual_upper": 0.01,
      "max_false_block_upper": 0.01,
      "min_complete_fraction": 1.0,
      "min_required_preserved_fraction": 0.99
    }
  }
}
```

Source-template and conversation hold-outs belong in the sampling evidence.
The harness checks that evidence references and independence attestation are
present for a deployment-domain evaluation. An authorised reviewer must assess
their truth, annotation agreement and deployment representativeness. The tool
reports that these substantive properties have not been independently verified.
Use `evidence_scope: "synthetic"` for fixtures. Synthetic results always retain
`SYNTHETIC_CONFORMANCE_ONLY` when their numerical thresholds pass.

## Collect through the real host

Provision the reviewed local detector bundle using
[local detector instructions](privacy-detector.md), then run:

```sh
python3 adapters/privacy/qualification.py \
  --manifest /restricted/evaluation/manifest.json \
  --corpus /restricted/evaluation/corpus.jsonl \
  --host target/debug/mas-host \
  --python /opt/mas-privacy/venv/bin/python \
  --bundle-manifest /opt/mas-privacy/bundle.json \
  --assets /opt/mas-privacy/assets \
  --worker adapters/privacy/worker.py \
  --assessments-output /restricted/evaluation/assessments.jsonl \
  --output /restricted/evaluation/result.json
```

The harness supplies each exact source on stdin and requests
`--include-details true`. It validates detector identity, source byte count,
normalisation/extraction, UTF-8 spans and complete neural coverage. A declared
complete result with missing final bytes fails the evaluation. Model failures
reported by the host remain incomplete results and contribute to the coverage
and false-block metrics.

The replay file binds each assessment to the frozen manifest and exact source
hash. Reordering, changed source text, changed detector identity and changed
manifest bytes are rejected:

```sh
python3 adapters/privacy/qualification.py \
  --manifest /restricted/evaluation/manifest.json \
  --corpus /restricted/evaluation/corpus.jsonl \
  --replay /restricted/evaluation/assessments.jsonl \
  --output /restricted/evaluation/replayed-result.json
```

Replay validates recorded identity and consistency. Its integrity depends on the
operator retaining the actual collection output and its provenance. It supplies
no signature, independent execution attestation or proof that an untrusted replay
file came from the selected model. The result identifies the canonical assessment
records by SHA-256 so a retained review can bind the exact evaluated evidence.

The corpus contains source content. Replay files contain span metadata and plain
source digests, which can reveal information about predictable inputs. Keep both
within the authorised benchmark storage boundary. New and existing outputs are
written with mode `0600`. Reports contain aggregate counts and identified
configuration, without source text, excerpts or per-document identifiers. The
command suppresses detector stderr and emits a bounded, content-free failure
message when evaluation evidence is invalid or unavailable. It leaves encryption,
retention, deletion and operator access controls to the selected deployment.

## Interpret the measurements

The benchmark transformation removes the union of findings after a complete
inspection and withholds an incomplete source. This fixed rule makes the
residual-disclosure calculation inspectable. It evaluates this transformation's
behaviour and supplies no permission to release its output.

For each declared stratum the result reports:

- Exact entity, Unicode codepoint and UTF-8 byte precision/recall by category.
- Documents with prohibited bytes left after the evaluated transformation.
- False blocks on fully permitted documents caused by incomplete inspection,
  and separate counts of complete negative documents changed by false findings.
- Redacted bytes outside the prohibited annotations and preserved required facts.
- Repeated-entity groups and groups whose every mention was detected.
- Complete, partial, unsupported and failed inspection counts.
- p50/p95/p99 wall latency, summed inspection time and processed source bytes per
  second under the sequential cold-process collection method.

The residual and false-block checks use exact one-sided Clopper–Pearson upper
bounds over the declared independent units. Zero failures in 299 independent
units gives a 95% upper bound below 1%. Repeated turns from one conversation
remain one unit, so adding dependent documents cannot manufacture that sample
size. The confidence applies separately to each reported bound; a joint
multi-stratum guarantee requires a separately planned multiplicity adjustment.

Entity detection and byte removal measure different consequences. A finding
with an incorrect category can remove the required bytes while lowering category
recall. Merged findings can reduce exact entity precision while still giving full
character coverage. Keep these metrics separate when selecting a candidate.

An unmeasured stratum, no negative units, no required-fact annotations or
insufficient sample size prevents its thresholds passing. Complete-coverage and
required-fact gates prevent universal withholding from appearing successful.
Memory and monetary cost remain explicitly `not-measured`. Capture those on the
identified deployment before making N5 capacity or cost claims. These results
also leave E4/E5 lifecycle/isolation and N2/N3 authority conformance with their
own acceptance evidence.

Exit status is `0` when all numerical thresholds pass, `2` when measurements
miss a threshold, and `1` when the evidence cannot be evaluated. A passing real
corpus reports `DECLARED_EVALUATION_THRESHOLDS_MET`. The report retains the
sampling/annotation-review qualification and `release_authority_granted: false`.

## Reproduce harness conformance

```sh
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover \
  -s adapters/privacy -p test_qualification.py -v
```

The actual-host test uses a synthetic full-coverage worker through the existing
guardian and the host's deterministic recognisers. It collects exact assessment
envelopes and verifies identical replayed metrics. Build `target/debug/mas-host`
or set `MAS_PRIVACY_QUALIFICATION_HOST` to the selected executable to include
this test. Its fixture runs no neural inference and establishes no deployment
quality claim.
