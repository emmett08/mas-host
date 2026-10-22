#!/usr/bin/env python3
"""Index completed identical-source formal checks without hiding command failures."""
import argparse
import hashlib
import json
from pathlib import Path
import runpy

ROOT = Path(__file__).resolve().parent
ATTEMPTS = ('governed-reader-formal-resource-timeout', 'governed-reader-formal-final',
            'governed-reader-formal-delegation-retry')


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    global ROOT
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--repo', type=Path, required=True)
    parser.add_argument('--attempt-root', type=Path, default=ROOT)
    args = parser.parse_args()
    ROOT = args.attempt_root.resolve()
    repo = args.repo.resolve()
    runs = [json.loads((ROOT / name / 'summary.json').read_text()) for name in ATTEMPTS]
    assert runs[-1]['status'] == 'PASS', 'The explicit delegation model retry must pass'
    expected = {}
    for model, name, macro, claim, no_reduce, counterexample in runpy.run_path(str(repo / 'formal/verify.py'))['CASES']:
        expected[f'{model}-{name}'] = (f'{model}.pml', macro, claim, not no_reduce,
                                      'counterexample' if counterexample else 'zero_errors')
    assert len(expected) == 184
    first = runs[0]
    selected = {}
    attempts = []
    for name, run in zip(ATTEMPTS, runs):
        for field in ('model_sha256', 'runner_sha256', 'limits', 'bounds', 'assumptions'):
            assert run[field] == first[field], (name, field)
        assert not run['source_changed_during_verification']
        failures = []
        for check in run['checks']:
            signature = tuple(check[k] for k in ('model', 'macro', 'claim', 'partial_order_reduction', 'expected'))
            assert signature == expected[check['name']]
            if check['result'] == 'FAILED_OR_INCOMPLETE':
                assert "'/usr/bin/cc'" in check['reason'] and 'timed out after 30 seconds' in check['reason']
                failures.append(check)
            else:
                assert check['result'] in ('COMPLETE_ZERO_ERRORS', 'EXPECTED_COUNTEREXAMPLE')
                selected[check['name']] = dict(check, evidence_attempt=name)
        attempts.append({'directory': name, 'status': run['status'],
                         'checks': len(run['checks']), 'seconds': run['elapsed_seconds'],
                         'failures': failures, 'artifact_sha256': {
                             f: sha(ROOT / name / f) for f in ('summary.json', 'commands.json', 'counterexamples.zip')}})
    assert set(selected) == set(expected)
    assert sha(repo / 'formal/verify.py') == first['runner_sha256']
    assert all(sha(repo / 'formal' / n) == v for n, v in first['model_sha256'].items())
    counts = {result: sum(c['result'] == result for c in selected.values())
              for result in ('COMPLETE_ZERO_ERRORS', 'EXPECTED_COUNTEREXAMPLE')}
    assert counts == {'COMPLETE_ZERO_ERRORS': 28, 'EXPECTED_COUNTEREXAMPLE': 156}
    result = {'schema': 'mas-host-formal-retry-coverage/1', 'status': 'PASS_WITH_RESOURCE_RETRIES',
              'interpretation': 'All 184 default case configurations completed successfully at the original bounds on identical model/runner sources. Each of two full default invocations failed one C compilation deadline; neither invocation is claimed as PASS. The affected delegation model was then explicitly retried with --model delegation and passed. Per-case provenance selects the latest successful observation. This coverage index does not alter or replace the raw runner receipts.',
              'model_sha256': first['model_sha256'], 'runner_sha256': first['runner_sha256'],
              'limits': first['limits'], 'bounds': first['bounds'], 'assumptions': first['assumptions'],
              'source_changed_during_verification': [], 'attempts': attempts,
              'elapsed_seconds': sum(run['elapsed_seconds'] for run in runs),
              'result_counts': counts, 'checks': [selected[n] for n in expected]}
    (ROOT / 'reader-formal-coverage.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps({'status': result['status'], 'checks': len(selected), 'counts': counts}))


if __name__ == '__main__':
    main()
