#!/usr/bin/env python3
"""Package frozen-source acquisition checks; reject incomplete gates or drift."""
import hashlib
import json
from pathlib import Path
import re
import runpy
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parent / 'mas-host'
OUT = REPO / 'evidence/nehemiah-governed-acquisition'


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def read(name):
    return json.loads((ROOT / name).read_text())


def verify_tree_cut(tree, cut):
    paths = subprocess.check_output(['git', 'ls-tree', '-r', '--name-only', tree], cwd=REPO).decode().splitlines()
    assert {p for p in paths if not p.startswith('evidence/')} == set(cut)
    for name, digest in cut.items():
        body = subprocess.check_output(['git', 'show', tree + ':' + name], cwd=REPO)
        assert hashlib.sha256(body).hexdigest() == digest, name


def verify_review_hashes(value, cut):
    matched = 0
    if isinstance(value, dict):
        for key, item in value.items():
            if key in cut and isinstance(item, str) and re.fullmatch(r'[0-9a-f]{64}', item):
                assert cut[key] == item, key
                matched += 1
            else:
                matched += verify_review_hashes(item, cut)
    elif isinstance(value, list):
        matched += sum(verify_review_hashes(item, cut) for item in value)
    return matched


def verify_failed_workspace(kind, identity, failed_test):
    """Bind one failed diagnostic to its own complete, immutable source cut."""
    receipt = read(f'acquisition-workspace-{kind}-failed.json')
    source = read(f'acquisition-source-{kind}-identity.json')
    remote = read(f'acquisition-source-{kind}-remote-commit.json')
    assert receipt['status'] == 'FAIL' and receipt['exit_code'] != 0
    assert receipt['label'] == 'workspace' and receipt['command'] == ['make', 'test']
    assert receipt['base'] == identity['base_commit'] and not receipt['source_changes_during_checks']
    log_path = ROOT / f'acquisition-workspace-{kind}-failed.log'
    assert sha(log_path) == receipt['log_sha256']
    assert re.findall(r'^test (.+?) \.\.\. FAILED$', log_path.read_text(), re.M) == [failed_test]
    assert remote['sha'] == source['source_commit']
    assert remote['tree']['sha'] == source['source_tree']
    assert source['base_commit'] == identity['base_commit']
    assert source['parent_commit'] == identity['parent_commit']
    assert source['parent_tree'] == identity['parent_tree']
    assert len(remote['parents']) == 1 and remote['parents'][0]['sha'] == identity['parent_commit']
    verify_tree_cut(source['source_tree'], receipt['source_sha256'])
    assert all(receipt['source_sha256'][name] == digest
               for name, digest in source['changed_source_sha256'].items())
    return receipt, source


def changed_paths(before, after):
    assert set(before) == set(after)
    return [name for name in sorted(after) if before[name] != after[name]]


def verify_harness_profile(identity, cut):
    """Separate the intermediate source object from the final harness profile."""
    previous = read('acquisition-source-pre-ci-profile-identity.json')
    remote = read('acquisition-source-pre-ci-profile-remote-commit.json')
    for key in ('base_commit', 'parent_commit', 'parent_tree', 'refactor_tree'):
        assert previous[key] == identity[key]
    assert remote['sha'] == previous['source_commit']
    assert remote['tree']['sha'] == previous['source_tree']
    assert len(remote['parents']) == 1 and remote['parents'][0]['sha'] == identity['parent_commit']
    ci_path = '.github/workflows/ci.yml'
    prior_cut = dict(cut, **{ci_path: previous['changed_source_sha256'][ci_path]})
    verify_tree_cut(previous['source_tree'], prior_cut)
    assert changed_paths(prior_cut, cut) == [ci_path]
    workflow = (REPO / ci_path).read_text()
    prior_workflow = subprocess.check_output(
        ['git', 'show', previous['source_tree'] + ':' + ci_path], cwd=REPO).decode()
    profile = "    env:\n      RUST_TEST_THREADS: '2'\n"
    assert workflow.count(profile) == 1 and workflow.replace(profile, '') == prior_workflow
    assert re.search(r'^  linux:\n    runs-on: ubuntu-24\.04\n    timeout-minutes: 30\n'
                     + re.escape(profile) + r'    steps:\n', workflow, re.M)
    review = read('exposure-ci-review.json')
    assert review['source_sha256'] == {ci_path: cut[ci_path]}
    assert review['yaml_profile_validation']['timeout_minutes'] == 30
    assert review['yaml_profile_validation']['environment'] == {'RUST_TEST_THREADS': '2'}
    return previous


def verify_focused_http(cut, pre_ci_source):
    """Retain focused observations without upgrading their source-capture limits."""
    expected_command = ['cargo', 'test', '--locked', '--offline', '-p', 'mas-host',
                        '--test', 'a2a_governed']
    diagnostic = read('exposure-governed-http-default-diagnostic.json')
    focused = read('exposure-governed-http-threads2-focused.json')
    assert diagnostic['status'] == 'FAIL-non-gate-diagnostic'
    assert diagnostic['command'] == focused['command'] == expected_command
    assert diagnostic['result'] == {'exit_code': 101, 'passed': 21, 'failed': 6,
                                    'test_seconds': 73.64}
    assert focused['status'] == 'PASS-focused-pending-root-full-gates'
    assert focused['environment']['RUST_TEST_THREADS'] == '2'
    assert focused['result'] == {'exit_code': 0, 'passed': 27, 'failed': 0,
                                'test_seconds': 141.34}
    for value in (diagnostic, focused):
        assert sha(ROOT / value['log']) == value['log_sha256']
        assert len(value['source_sha256']) == 16
        assert all(cut.get(name) == digest for name, digest in value['source_sha256'].items())
    assert re.search(r'test result: FAILED\. 21 passed; 6 failed;',
                     (ROOT / diagnostic['log']).read_text())
    assert re.search(r'test result: ok\. 27 passed; 0 failed;',
                     (ROOT / focused['log']).read_text())
    assert sha(ROOT / diagnostic['resource_snapshot']) == diagnostic['resource_snapshot_sha256']
    assert diagnostic['source_provenance']['parent_reported_staged_tree_after_run'] == pre_ci_source['source_tree']
    provenance = focused['source_provenance']
    assert sha(ROOT / provenance['during_observation']) == provenance['during_observation_sha256']
    assert read(provenance['during_observation'])['source_sha256'] == focused['source_sha256']
    assert not provenance['source_changes_between_during_and_after']
    return diagnostic, focused


def main():
    identity = read('acquisition-source-identity.json')
    commands = {label: read('acquisition-' + label + '-final.json')
                for label in ('workspace', 'acceptance', 'provider', 'formal')}
    expected = {'workspace': ['make', 'test'], 'acceptance': ['make', 'acceptance'],
                'provider': ['make', 'provider-test'],
                'formal': ['python3', 'formal/verify.py', '--out', str(ROOT / 'acquisition-formal-final')]}
    assert all(c['status'] == 'PASS' and c['exit_code'] == 0 and not c['source_changes_during_checks']
               for c in commands.values())
    assert all(c['command'] == expected[label] and c['label'] == label
               and c['base'] == identity['base_commit'] for label, c in commands.items())
    assert all(commands[label]['environment']['RUST_TEST_THREADS'] == '2'
               for label in ('workspace', 'acceptance'))
    cut = commands['workspace']['source_sha256']
    assert all(c['source_sha256'] == cut for c in commands.values())
    sys.dont_write_bytecode = True
    current_sources = runpy.run_path(str(ROOT / 'validate_acquisition_command.py'))['sources']
    assert current_sources(REPO) == cut
    assert all(sha(REPO / name) == digest for name, digest in cut.items())
    assert all(cut[name] == digest for name, digest in identity['changed_source_sha256'].items())
    assert subprocess.check_output(['git', 'write-tree'], cwd=REPO).decode().strip() == identity['source_tree']
    verify_tree_cut(identity['source_tree'], cut)
    source_remote = read('acquisition-source-remote-commit.json')
    assert source_remote['sha'] == identity['source_commit']
    assert source_remote['tree']['sha'] == identity['source_tree']
    assert len(source_remote['parents']) == 1
    assert source_remote['parents'][0]['sha'] == identity['parent_commit']
    initial, initial_source = verify_failed_workspace(
        'initial', identity, 'lost_effect_response_keeps_uncertainty_and_authorised_recovery_ids')
    expiry, expiry_source = verify_failed_workspace(
        'expiry', identity,
        'retained_source::source_expiry_survives_same_epoch_grant_renewal_and_old_delivery_methods')
    polling_test = 'crates/mas-host-cli/tests/a2a_governed.rs'
    expiry_test = 'crates/mas-host-cli/tests/a2a_governed/retained_source.rs'
    ci_path = '.github/workflows/ci.yml'
    initial_to_expiry = changed_paths(initial['source_sha256'], expiry['source_sha256'])
    expiry_to_final = changed_paths(expiry['source_sha256'], cut)
    initial_to_final = changed_paths(initial['source_sha256'], cut)
    assert initial_to_expiry == [polling_test]
    assert expiry_to_final == sorted([expiry_test, ci_path])
    assert initial_to_final == sorted([polling_test, expiry_test, ci_path])
    pre_ci_source = verify_harness_profile(identity, cut)
    http_diagnostic, http_focused = verify_focused_http(cut, pre_ci_source)
    refactor = read('acquisition-refactor-identity.json')
    refactor_run = read('acquisition-refactor-final.json')
    refactor_remote = read('acquisition-refactor-remote-commit.json')
    assert refactor_run['status'] == 'PASS' and refactor_run['exit_code'] == 0
    assert refactor_run['label'] == 'refactor-characterization'
    assert refactor_run['command'] == ['cargo', 'test', '--locked', '--offline', '-p', 'mas-host', '--test', 'a2a_governed', 'retained_effect', '--', '--test-threads=1', '--nocapture']
    assert not refactor_run['source_changes_during_checks']
    assert sha(ROOT / 'acquisition-refactor-final.log') == refactor_run['log_sha256']
    assert refactor_run['base'] == refactor['local_test_commit']
    verify_tree_cut(refactor['tree'], refactor_run['source_sha256'])
    assert subprocess.check_output(['git', 'rev-parse', refactor['local_test_commit'] + '^{tree}'], cwd=REPO).decode().strip() == refactor['tree']
    assert refactor['remote_commit'] == identity['parent_commit'] == refactor_remote['sha']
    assert refactor['tree'] == identity['refactor_tree'] == identity['parent_tree'] == refactor_remote['tree']['sha']
    assert len(refactor_remote['parents']) == 1
    assert refactor_remote['parents'][0]['sha'] == identity['base_commit'] == refactor['base_commit']
    assert all(refactor_run['source_sha256'][name] == digest for name, digest in refactor['changed_source_sha256'].items())
    assert re.search(r'test result: ok\. 8 passed; 0 failed;', (ROOT / 'acquisition-refactor-final.log').read_text())
    for review in ('exposure-independent-review.json', 'exposure-cli-review.json',
                   'exposure-core-review.json', 'exposure-example-review.json',
                   'exposure-lineage/review.json', 'exposure-fixture-review.json',
                   'exposure-ci-review.json'):
        value = read(review)
        final_hashes = value.get('source_sha256') or value.get('source_identity', {}).get('reviewed_file_sha256')
        assert isinstance(final_hashes, dict) and final_hashes, review
        assert verify_review_hashes(final_hashes, cut) == len(final_hashes), review
    formal = read('acquisition-formal-final/summary.json')
    assert formal['status'] == 'PASS' and len(formal['checks']) == 224
    sys.dont_write_bytecode = True
    cases = runpy.run_path(str(REPO / 'formal/verify.py'))['CASES']
    signatures = [(f'{model}-{name}', f'{model}.pml', macro, claim,
                   not no_reduce, 'counterexample' if assertion else 'zero_errors')
                  for model, name, macro, claim, no_reduce, assertion in cases]
    actual = [(c['name'], c['model'], c['macro'], c['claim'],
               c['partial_order_reduction'], c['expected']) for c in formal['checks']]
    assert len(set(signatures)) == len(signatures) == 224
    assert len(set(actual)) == len(actual) == 224 and set(actual) == set(signatures)
    assert not formal['source_changed_during_verification']
    assert sum(c['result'] == 'COMPLETE_ZERO_ERRORS' for c in formal['checks']) == 32
    assert sum(c['result'] == 'EXPECTED_COUNTEREXAMPLE' for c in formal['checks']) == 192
    assert sha(REPO / 'formal/verify.py') == formal['runner_sha256']
    assert all(sha(REPO / 'formal' / name) == digest
               for name, digest in formal['model_sha256'].items())
    archive = ROOT / 'governed-acquisition-formal.tar.xz'
    subprocess.run(['tar', '-C', str(ROOT), '-I', 'xz -9 -T1', '-cf', str(archive),
                    'acquisition-formal-final', 'exposure-lineage'], check=True)
    OUT.mkdir(parents=True, exist_ok=False)
    copies = {
        'formal-summary.json': 'acquisition-formal-final/summary.json',
        'formal-checks.tar.xz': archive.name,
        'validate-command.py': 'validate_acquisition_command.py',
        'package-evidence.py': 'package_acquisition_evidence.py',
        'toolchain.json': 'acquisition-toolchain.json',
        'plan-snapshots.json': 'followon-plan-snapshots.json',
        'acceptance-contract.json': 'exposure-contract.json',
        'independent-review.json': 'exposure-independent-review.json',
        'cli-review.json': 'exposure-cli-review.json',
        'core-architecture-review.json': 'exposure-core-review.json',
        'example-review.json': 'exposure-example-review.json',
        'fixture-review.json': 'exposure-fixture-review.json',
        'ci-profile-review.json': 'exposure-ci-review.json',
        'formal-review.json': 'exposure-lineage/review.json',
        'formal-tool-reference-review.json': 'exposure-lineage/tool-reference-review.json',
        'refactor-characterization.json': 'acquisition-refactor-final.json',
        'refactor-characterization.log': 'acquisition-refactor-final.log',
        'refactor-identity.json': 'acquisition-refactor-identity.json',
        'refactor-remote-commit.json': 'acquisition-refactor-remote-commit.json',
        'source-remote-commit.json': 'acquisition-source-remote-commit.json',
        'ci-budget-basis.json': 'acquisition-ci-budget-basis.json',
        'exposure-cli-final-clippy.log': 'exposure-cli-final-clippy.log',
        'exposure-source-final-tests.log': 'exposure-source-final-tests.log',
        'exposure-adversarial-final-tests.log': 'exposure-adversarial-final-tests.log',
        'exposure-source-final-http.log': 'exposure-source-final-http.log',
        'exposure-source-http-diagnostic.log': 'exposure-source-http-diagnostic.log',
        'exposure-source-32-call-diagnostic.log': 'exposure-source-32-call-diagnostic.log',
        'exposure-example-focused.log': 'exposure-example-focused.log',
        'exposure-example-stdout.json': 'exposure-example-stdout.json',
        'exposure-example-run.json': 'exposure-example-run.json',
        'exposure-example-static.log': 'exposure-example-static.log',
        'exposure-runtime-diagnostic.log': 'exposure-runtime-diagnostic.log',
        'packaging-review.json': 'exposure-packaging-review.json',
        'workspace-initial-failed.json': 'acquisition-workspace-initial-failed.json',
        'workspace-initial-failed.log': 'acquisition-workspace-initial-failed.log',
        'source-initial-identity.json': 'acquisition-source-initial-identity.json',
        'source-initial-remote-commit.json': 'acquisition-source-initial-remote-commit.json',
        'workspace-expiry-failed.json': 'acquisition-workspace-expiry-failed.json',
        'workspace-expiry-failed.log': 'acquisition-workspace-expiry-failed.log',
        'source-expiry-identity.json': 'acquisition-source-expiry-identity.json',
        'source-expiry-remote-commit.json': 'acquisition-source-expiry-remote-commit.json',
        'source-pre-ci-profile-identity.json': 'acquisition-source-pre-ci-profile-identity.json',
        'source-pre-ci-profile-remote-commit.json': 'acquisition-source-pre-ci-profile-remote-commit.json',
        'exposure-legacy-uncertainty-target.log': 'exposure-legacy-uncertainty-target.log',
        'exposure-source-expiry-target.log': 'exposure-source-expiry-target.log',
        'exposure-governed-http-parallel-final.log': 'exposure-governed-http-parallel-final.log',
        'exposure-governed-http-threads2-focused.json': 'exposure-governed-http-threads2-focused.json',
        'exposure-http-threads2-source-during.json': 'exposure-http-threads2-source-during.json',
        'exposure-governed-http-default-diagnostic.json': 'exposure-governed-http-default-diagnostic.json',
        'exposure-governed-http-expiry-followup-diagnostic.log': 'exposure-governed-http-expiry-followup-diagnostic.log',
        'exposure-http-resource-diagnostic.json': 'exposure-http-resource-diagnostic.json',
    }
    for label, value in commands.items():
        assert sha(ROOT / ('acquisition-' + label + '-final.log')) == value['log_sha256']
        for ext in ('json', 'log'):
            copies[label + '-run.' + ext] = 'acquisition-' + label + '-final.' + ext
    for target, source in copies.items():
        shutil.copyfile(ROOT / source, OUT / target)
    (OUT / '.gitattributes').write_text('# Preserve verbatim command output.\n*.log -whitespace\n')
    contract = json.loads((OUT / 'acceptance-contract.json').read_text())
    contract['validation_status'] = 'PASS_FOR_IDENTIFIED_SOURCE'
    contract['source_commit'] = identity['source_commit']
    (OUT / 'acceptance-contract.json').write_text(json.dumps(contract, indent=2) + '\n')
    log = (OUT / 'workspace-run.log').read_text()
    rust = [tuple(map(int, groups)) for groups in re.findall(
        r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored', log)]
    python = [int(n) for n in re.findall(r'^Ran (\d+) tests? in ', log, re.M)]
    metadata = {
        'schema': 'nehemiah-governed-acquisition-evidence/1', 'status': 'PASS',
        'repository': 'emmett08/mas-host', **identity,
        'validation_source_identity': 'All four final command receipts contain identical hashes for every non-evidence source. Their base is the local parent HEAD during execution. Source and evidence are separate commits.',
        'plans': read('followon-plan-snapshots.json'),
        'commands': {label: {key: c[key] for key in ('command', 'environment', 'status', 'seconds', 'exit_code', 'log_sha256', 'source_changes_during_checks')} for label, c in commands.items()},
        'test_counts': {'workspace_rust_passed': sum(r[0] for r in rust),
                        'workspace_rust_ignored': sum(r[2] for r in rust),
                        'workspace_rust_ignored_names': re.findall(r'^test (.+?) \.\.\. ignored.*$', log, re.M),
                        'workspace_python_passed': sum(python), 'python_suite_counts': python,
                        'native_acceptance_additional': 1, 'provider_node_tests_additional': 5,
                        'counting_rule': 'Workspace summaries only; focused independent and example checks overlap and are not added again.'},
        'formal': {'status': formal['status'], 'checks': 224, 'new_checks': 40,
                   'unchanged_previous_checks': 184, 'complete_zero_error_searches': 32,
                   'expected_replayed_counterexamples': 192, 'limits': formal['limits'],
                   'seconds': formal['elapsed_seconds'], 'archive_sha256': sha(OUT / 'formal-checks.tar.xz'),
                   'scope': 'Finite control verification with implementation correspondence. No Rust/SQLite refinement, arbitrary graph theorem, detector quality or distributed revocation linearisation claim.'},
        'contract': [
            'Opt-in local/2 selects operator-owned baseline templates and additional registered source templates; local/1 remains closed and retains historical all-template decisions.',
            'Original source policies govern acquisition before fetch and detector processing before callbacks. Pending exposure and executing state commit atomically before execute permission.',
            'Host-sealed complete ancestry and exposure manifests bind source, step, payload, original expiry and policy selection in the existing execution journal.',
            'Children inherit exposure before persistence/dispatch; parallel children use one frozen parent snapshot and received results monotonically constrain subsequent commands.',
            'Complete closure is bounded to 64 sources and 65536 representation bytes. A restrictive 33rd ancestor participates. Excessive, missing, cyclic or inconsistent lineage denies without truncation.',
            'Owned-task GetRetainedSource reads an exact retained projection under current resolve and complete original policy closure. Discarded raw bytes are not retained.',
            'Existing task, replay and effect delivery paths enforce local/2 closure and original expiry. New SendMessage returns a pre-worker admission snapshot.',
            'Read-only snapshots release the database before callbacks and perform no recovery or execution. Known host settlement remains independent of payload access and external certainty.',
        ],
        'fixture_profile': 'Synthetic structured Deployment point, deterministic recognisers, fixture neural responses and loopback HTTP; exact offline-worker assets/executable pins. No actual neural inference or live Kubernetes/provider/notification call.',
        'workspace_diagnostics': [
            {'status': initial['status'], 'kind': 'initial-lost-response',
             'source_commit': initial_source['source_commit'], 'source_tree': initial_source['source_tree'],
             'seconds': initial['seconds'], 'next_source_commit': expiry_source['source_commit'],
             'changed_files_before_next_cut': initial_to_expiry,
             'reason': 'An existing lost-response test observed working immediately after the blocking deadline. Its only repair polls the SAME task for terminal unknown with a bounded deadline while preserving no-redispatch assertions.',
             'promoted_to_final_gate': False},
            {'status': expiry['status'], 'kind': 'expiry-fixture-startup',
             'source_commit': expiry_source['source_commit'], 'source_tree': expiry_source['source_tree'],
             'seconds': expiry['seconds'], 'next_source_commit': identity['source_commit'],
             'changed_files_before_next_cut': expiry_to_final,
             'reason': 'The new expiry test armed an eight-second fixture lease before server startup and observed unauthorised. Startup consuming the fixture lease is an inference from the failure and schedule. The fixture repair starts the server before arming a thirty-second lease; original-expiry, read-denial and no-redispatch assertions remain. The separate CI harness change fixes test threads at two.',
             'promoted_to_final_gate': False},
        ],
        'failed_cut_transitions': {'initial_to_expiry': initial_to_expiry,
                                   'expiry_to_final': expiry_to_final,
                                   'initial_to_final': initial_to_final,
                                   'production_or_formal_changes': []},
        'rust_test_harness_profile': {
            'RUST_TEST_THREADS': '2', 'ci_job_timeout_minutes': 30,
            'required_gate_environments': ['workspace', 'acceptance'],
            'pre_ci_source_commit': pre_ci_source['source_commit'],
            'pre_ci_source_tree': pre_ci_source['source_tree'],
            'pre_ci_to_final_changed_files': [ci_path],
            'pre_ci_source_object_is_gate': False,
            'qualification': 'Explicit two-thread local Rust test harness profile; no default/high-parallel capacity or hosted CI PASS claim.'},
        'focused_http_observations': {
            'default_launch': {'status': http_diagnostic['status'],
                               'result': http_diagnostic['result'],
                               'parallelism': http_diagnostic['parallelism'],
                               'source_provenance': http_diagnostic['source_provenance'],
                               'interpretation': http_diagnostic['interpretation']},
            'explicit_two_threads': {'status': http_focused['status'],
                                     'result': http_focused['result'],
                                     'environment': http_focused['environment'],
                                     'source_provenance': http_focused['source_provenance']},
            'promoted_to_final_gates': False,
            'source_binding_limit': 'Default diagnostic has a post-exit16-file map only; focused explicit-two-thread observation has during/after16-file maps, not a before-start complete map. The four final gates independently bind the complete source cut.'},
        'remaining_plan_work': contract['remaining'],
        'publication': {'merged': False, 'deployed': False, 'manual_workflow_dispatches': 0,
                        'external_notifications_sent': 0, 'live_external_provider_calls': 0},
        'artifact_sha256': {name: sha(OUT / name) for name in [*copies, '.gitattributes']},
    }
    (OUT / 'validation.json').write_text(json.dumps(metadata, indent=2) + '\n')
    counts = metadata['test_counts']
    (OUT / 'README.md').write_text(f'''# Governed structured acquisition evidence

This increment implements opt-in local/2 acquisition with operator-selected source
policies, durable host-owned exposure inheritance and an authorised retained
source projection reader. The existing execution journal remains the owner.

Source commit: `{identity['source_commit']}`

Source tree: `{identity['source_tree']}`

Base: `{identity['base_commit']}` (PR #19; this increment is stacked on its branch).

The preceding behaviour-preserving reader extraction is
`{identity['refactor_commit']}` (tree `{identity['refactor_tree']}`), the source
commit's direct parent. Its eight existing HTTP reader tests pass separately.

| Gate | Result |
|---|---|
| `make test` | {counts['workspace_rust_passed']} Rust and {counts['workspace_python_passed']} Python tests; Go race/vet, C++ conformance, TypeScript check/build, formatting and strict Clippy passed |
| `make acceptance` | Native multi-language and recovery acceptance passed |
| `make provider-test` | Five Node tests passed |
| Default SPIN suite | All 224 checks passed: 32 complete zero-error searches and 192 expected replayed counterexamples |

The 40 new formal checks cover finite acquisition, exposure, pending/callback,
closure, inherited child-target, expiry and replay control. All 184 previous case
tuples and seven previous model bytes remain unchanged. No search limits were
relaxed. See [the model contract](../../formal/exposure-lineage.md).

All four final command receipts record identical SHA256 values for every
non-evidence source and reject source drift. The identified source commit,
preceding refactor and following evidence-only commit preserve that separation.
`validation.json`
binds source, plans, command logs, counts and artefact checksums. The independent
reviews record findings, repairs, exact source identities and focused-check
limitations. Focused exploratory passes overlap the authoritative final suite.

`formal-checks.tar.xz` retains the full default run plus focused final and
prototype model runs, each with its own hashes, commands, bounds and replayed
trails. Prototype results are historical diagnostics, not final-source gates.
Reproduce with the three Make targets above and
`python3 formal/verify.py --out /tmp/acquisition-formal` using the recorded
toolchain. `validate-command.py` wraps a command and rejects source changes.

Two workspace runs remain separate diagnostics. The initial cut failed an
existing recovery test's immediate
`unknown` assertion while the task was still `working` after a blocking response
deadline. Only `tests/a2a_governed.rs` changed for the next cut: the test polls
the SAME task for terminal uncertainty with a bounded deadline and preserves
all no-redispatch assertions. That second cut failed the new expiry fixture's
eight-second lease, which had included server startup. Startup consuming the
lease is an inference from the observed failure and fixture schedule. The
fixture repair in `tests/a2a_governed/retained_source.rs` starts the server first,
then arms a thirty-second lease while preserving original-expiry and
no-redispatch assertions. The final cut also adds the CI job environment
`RUST_TEST_THREADS='2'`; its existing thirty-minute aggregate budget remains.
The pre-profile source object is retained as an intermediate identity with no
gate claim. No runtime or formal code changed between these cuts. Each failed
source cut, remote tree, full source map, log and receipt is retained; neither
partial run is promoted to a final gate. The final source cut passes the
complete gates above under the recorded command environments.

The workspace and acceptance gates use the explicit two-thread Rust test
harness profile, also configured in CI. An earlier default-launch focused run
failed six of twenty-seven fixtures: five existing ten-second client socket
reads and one elapsed-time assertion. Its process environment and exact test
thread count were not captured; the preserved resource observations are
consistent with contention, not an isolated causal measurement. Its source map
was captured only after exit. The explicit two-thread focused rerun passed all
twenty-seven tests in 141.34 seconds and has during/after sixteen-file source
observations, rather than a complete before-start cut. Both observations remain
separate from the four authoritative final gates. No default/high-parallel
capacity, production throughput or hosted CI PASS is claimed. Existing runtime,
socket, detector and SPIN bounds remain unchanged.

The acquisition example runs through the actual loopback A2A service. It checks
baseline success, restricted acquisition, a permitted consuming peer, denial of
the same constant downstream actions, read/restart/replay identity and a
baseline-only acquisition control. Independent tests check actual 33+ ancestry,
the 64-source bound, corruption, pending crash recovery and existing read APIs.
The source reader returns retained projections; discarded original bytes and
component assessment custody remain unavailable.

This is a trusted-local structured increment. Synthetic collector and neural
responses establish host conformance, not live collector or detector quality.
Authority and exposure checks are snapshots with a final-read-to-transmission
interval. Larger/shared readership,
dynamic credentials, governed revision, exact assessment/source-byte custody,
full N3 scheduling/reconciliation, E4 deletion/restore, E5 isolation and measured
deployment/quality/capacity remain open in the plans. No merge or deployment is
performed by this increment. See [usage and limits](../../docs/governed-acquisition.md).
''')
    print(json.dumps({'source_commit': identity['source_commit'], 'source_tree': identity['source_tree'],
                      'test_counts': counts, 'formal_checks': 224, 'files': len(copies) + 3}))


if __name__ == '__main__':
    main()
