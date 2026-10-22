# Local neural CLI qualification

The six-case corpus contains synthetic names and addresses, including Unicode,
combining characters, a final-token email, benign text and empty input. Its
annotations use UTF-8 byte ranges and were fixed before testing the custom
adapter. These examples check coverage of the annotated entities, valid UTF-8
boundaries and complete inspection. Positive cases permit surrounding whitespace
and additional findings, so their result supplies no exact span-precision estimate.
They provide no population quality estimate or permission to release data.

Routine negative controls need Python's standard library and no model assets:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s tests/privacy -p 'test_*.py' -v
```

After provisioning and sealing the pinned local detector, run explicit neural
qualification. Cases run sequentially to bound concurrent model memory:

```sh
python3 tests/privacy/qualify.py \
  --host target/debug/mas-host \
  --python /absolute/path/to/provisioned-venv/bin/python \
  --manifest /absolute/path/to/manifest.json \
  --assets /absolute/path/to/assets \
  --receipt /absolute/path/to/qualification.json
```

Each case has a host-worker deadline and an outer CLI deadline. The receipt
binds the harness, corpus, host binary, Python executable, worker and detector
manifest to the observed results. It contains categories and byte ranges,
timings, cumulative child peak RSS and fixed failure codes. Source text and
detected values remain in the synthetic corpus. A failed scan, missing entity,
coverage gap, invalid UTF-8 byte range, changed detector or source disclosure fails
the qualification. Checkpoints remain `RUNNING` until every expected case passes.
Failures retain `FAIL`; interrupted checkpoints cannot assert completed acceptance.
Every accepted response keeps `release_permitted` and
`quality_qualified` false.
