# Local neural CLI qualification

The six-case corpus checks whether the packaged detector covers pre-annotated synthetic
entities and returns valid UTF-8 byte ranges with complete inspection. It includes
Unicode and combining characters, an email at the final token, benign text and empty
input. Annotations were fixed before testing the custom adapter.

Positive cases permit surrounding whitespace and additional findings. Passing therefore
establishes coverage of these annotations, while exact span precision and population
quality require separate evaluation. Release permission remains subject to host policy.

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

Each case has a host-worker deadline and an outer CLI deadline. The receipt binds the
harness, corpus, host binary, Python executable, worker and detector manifest to the
observed results. It contains categories and byte ranges, timings, cumulative child peak
RSS and fixed failure codes. Source text and detected values remain in the synthetic
corpus. A failed scan, missing entity, coverage gap, invalid UTF-8 byte range, changed
detector or source disclosure fails the qualification. Checkpoints remain `RUNNING`
until every expected case passes. Failed runs retain `FAIL`, and interrupted checkpoints
retain their incomplete status. Every accepted response keeps `release_permitted` and
`quality_qualified` false.
