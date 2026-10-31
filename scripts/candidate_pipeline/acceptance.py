"""Local-input, native candidate acceptance. Never downloads or publishes.

Only an isolated local clone receives a temporary unpublished catalog entry.
Fixture tests of this orchestrator are NOT native manager acceptance evidence.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import sys
import tempfile
import time

HERE = Path(__file__).resolve().parent
HELPERS = HERE.parent
sys.path.insert(0, str(HELPERS))
from verify_native_language_pack import run, DEADLINE_ENV
try:
    from .build_candidate import validate_candidate
except ImportError:  # Direct CLI invocation.
    from build_candidate import validate_candidate

TESTS = {
    'gofmt': ('CODESTYLES_TEST_GO_PACK', 'real_go_pack_preview_review_write_and_uninstall'),
    'dartfmt': ('CODESTYLES_TEST_DART_PACK', 'real_dart_pack_preview_review_write_and_uninstall'),
}
MIN_FREE = 2 * 1024 ** 3


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def regular_tree(path):
    if path.is_symlink() or not path.is_dir():
        raise ValueError('Resources must be an existing regular directory')
    for child in path.rglob('*'):
        if child.is_symlink() or not (child.is_file() or child.is_dir()):
            raise ValueError('Resources contain nonregular path')


def verify(plan_path, plan_sha256, manifest_path, bundle, root, expected_commit,
           work_dir, report_dir, runner=run, disk_usage=shutil.disk_usage):
    for value, size in [(plan_sha256, 64), (expected_commit, 40)]:
        if not re.fullmatch('[0-9a-f]{%d}' % size, value):
            raise ValueError('Expected immutable SHA digest')
    plan_path, manifest_path, bundle = map(lambda p: Path(p).absolute(), (plan_path, manifest_path, bundle))
    for p in (plan_path, manifest_path, bundle):
        if p.is_symlink() or not p.is_file():
            raise ValueError('Candidate inputs must be regular files')
    if sha(plan_path) != plan_sha256:
        raise ValueError('Review plan digest mismatch')
    plan = json.loads(plan_path.read_text())
    manifest = json.loads(manifest_path.read_text())
    validate_candidate(plan, manifest, bundle)
    tool, target = plan['tool'], plan['target']
    if tool not in TESTS:
        raise ValueError('Unsupported exact acceptance test')
    root = Path(root).resolve()
    work_dir, report_dir = Path(work_dir).resolve(), Path(report_dir).resolve()
    if work_dir.is_relative_to(root) or report_dir.is_relative_to(root):
        raise ValueError('Work and reports must be outside canonical checkout')
    work_dir.mkdir(parents=True, exist_ok=True)
    # Own every output path before bounded runner logs are opened. Reusing a
    # caller directory could follow preexisting log/report symlinks elsewhere.
    report_dir.mkdir(parents=True, exist_ok=False)
    if disk_usage(work_dir).free < MIN_FREE:
        raise ValueError('Native acceptance needs at least 2 GiB free')
    source_catalog = root / 'resources/toolchains.json'
    original = source_catalog.read_bytes()
    env = dict(os.environ)
    toolchain = '1.99.1-' + target
    env.update(CARGO_BUILD_JOBS='1', CARGO_INCREMENTAL='0', CARGO_NET_OFFLINE='true',
               RUSTUP_AUTO_INSTALL='0', RUSTUP_TOOLCHAIN=toolchain)
    for key in ('RUSTC_WRAPPER', 'RUSTC_WORKSPACE_WRAPPER', 'CARGO_BUILD_RUSTC_WRAPPER'):
        env.pop(key, None)
    env[DEADLINE_ENV] = str(time.monotonic() + 30 * 60)
    report = {'tool': tool, 'target': target, 'application_commit': expected_commit,
              'plan_sha256': plan_sha256, 'archive_sha256': sha(bundle),
              'available': False, 'release_clearance': False, 'published': False,
              'native_execution': 'failed', 'canonical_catalog_unchanged': False,
              'canonical_checkout_unchanged': False,
              'isolated_catalog_restored': False, 'temporary_removed': False}
    owned = None
    try:
        runner(['git', 'diff', 'HEAD', '--exit-code'], root, env, report_dir / 'before.log')
        head = runner(['git', 'rev-parse', 'HEAD'], root, env, report_dir / 'head.log').strip()
        if head != expected_commit:
            raise ValueError('Application commit differs from reviewed input')
        binaries = {}
        for name in ('rustc', 'cargo'):
            resolved = runner(['rustup', 'which', '--toolchain', toolchain, name], root, env,
                              report_dir / (name + '-path.log')).strip()
            path = Path(resolved)
            if not path.is_absolute() or path.is_symlink() or not path.is_file() or path.stem != name:
                raise ValueError('Pinned native toolchain binary is not already installed')
            binaries[name] = str(path)
        if Path(binaries['rustc']).parent != Path(binaries['cargo']).parent:
            raise ValueError('Native compiler and Cargo must belong to the same installed toolchain')
        env['RUSTC'] = binaries['rustc']
        host = runner([binaries['rustc'], '-vV'], root, env, report_dir / 'host.log')
        if f'host: {target}' not in host.splitlines():
            raise ValueError('Native Rust host differs from target')
        regular_tree(root / 'resources')
        with tempfile.TemporaryDirectory(prefix='candidate-acceptance-', dir=work_dir) as temporary:
            owned = Path(temporary)
            checkout = owned / 'checkout'
            # Local object sharing only; no remote fetch and no worktree metadata
            # or branch updates in the canonical repository.
            runner(['git', 'clone', '--shared', '--no-checkout', str(root), str(checkout)],
                   work_dir, env, report_dir / 'clone.log')
            runner(['git', 'checkout', '--detach', expected_commit], checkout, env,
                   report_dir / 'checkout.log')
            catalog_path = checkout / 'resources/toolchains.json'
            if catalog_path.read_bytes() != original:
                raise ValueError('Isolated catalog does not match clean canonical source')
            shutil.copytree(root / 'resources', checkout / 'resources', dirs_exist_ok=True)
            runner([sys.executable, str(checkout / 'scripts/bundle_resources.py'), 'verify', '--target', target],
                   checkout, env, report_dir / 'resources.log')
            catalog = json.loads(original)
            entry = catalog['toolchains'][tool]
            if entry['version'] != plan['version'] or target not in entry['targets']:
                raise ValueError('Candidate target/version differs from application catalog')
            baseline = entry['targets'][target]
            if any(baseline.get(k) != manifest.get(k) for k in ('executable', 'environment')):
                raise ValueError('Candidate changes the existing adapter executable/environment')
            # Snapshot the validated archive so concurrent edits cannot change
            # which bytes the real manager consumes after review validation.
            own_bundle = owned / bundle.name
            shutil.copyfile(bundle, own_bundle)
            validate_candidate(plan, manifest, own_bundle)
            try:
                entry['targets'][target] = manifest
                catalog_path.write_text(json.dumps(catalog, indent=2) + '\n')
                env_var, test_name = TESTS[tool]
                for key, _ in TESTS.values():
                    env.pop(key, None)
                env[env_var] = str(own_bundle)
                env['CARGO_TARGET_DIR'] = str(owned / 'target')
                runtime_tmp = owned / 'runtime-temp'
                runtime_tmp.mkdir()
                for key in ('TMPDIR', 'TMP', 'TEMP'):
                    env[key] = str(runtime_tmp)
                result = runner([binaries['cargo'], 'test', '--offline', '--locked', '--release',
                                 '--no-default-features', '--test', 'new_languages', test_name,
                                 '--', '--ignored', '--exact', '--nocapture', '--test-threads=1'],
                                checkout, env, report_dir / 'native-test.log')
                if not re.search(r'test result: ok\. 1 passed; 0 failed; 0 ignored;', result):
                    raise RuntimeError('Exact manager acceptance must execute one passing test')
                report['native_execution'] = 'passed'
            finally:
                catalog_path.write_bytes(original)
                report['isolated_catalog_restored'] = catalog_path.read_bytes() == original
                if not report['isolated_catalog_restored']:
                    raise RuntimeError('Failed to restore isolated catalog')
    except BaseException:
        report['native_execution'] = 'failed'
        raise
    finally:
        report['canonical_catalog_unchanged'] = source_catalog.read_bytes() == original
        report['temporary_removed'] = owned is None or not owned.exists()
        final_error = None
        try:
            env[DEADLINE_ENV] = str(time.monotonic() + 60)
            runner(['git', 'diff', 'HEAD', '--exit-code'], root, env, report_dir / 'after.log')
            final_head = runner(['git', 'rev-parse', 'HEAD'], root, env, report_dir / 'final-head.log').strip()
            if final_head != expected_commit:
                raise RuntimeError('Canonical application commit changed externally')
            report['canonical_checkout_unchanged'] = True
        except BaseException as error:
            final_error = error
            report['native_execution'] = 'failed'
        if not report['canonical_catalog_unchanged']:
            report['native_execution'] = 'failed'
        (report_dir / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
        if not report['canonical_catalog_unchanged']:
            raise RuntimeError('Canonical catalog changed externally; refusing to overwrite it')
        if final_error is not None:
            raise final_error
    return report


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    for key in ('plan', 'plan-sha256', 'manifest', 'bundle', 'root', 'expected-commit', 'work-dir', 'report-dir'):
        p.add_argument('--' + key, required=True)
    a = p.parse_args()
    verify(a.plan, a.plan_sha256, a.manifest, a.bundle, a.root, a.expected_commit, a.work_dir, a.report_dir)
