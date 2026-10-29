"""Offline verification-helper fixtures: no network, compiler, or foreign execution."""
import json
import itertools
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch
import time
import sys
import os

from build_language_pack import digest_file, write_bundle
from verify_native_language_pack import DEADLINE_ENV, MIN_FREE, run, terminate_tree, validate_candidate, verify

TARGET = 'aarch64-pc-windows-msvc'


def setup(root, published=False):
    resources = root / 'resources'
    resources.mkdir()
    member = {'archive_path': 'upstream/bin/rustfmt.exe', 'path': 'bin/rustfmt.exe',
              'bytes': 4, 'sha256': __import__('hashlib').sha256(b'test').hexdigest(), 'executable': True}
    source = {'available': True, 'executable': member['path'], 'environment': {}, 'max_installed_bytes': 4,
              'archives': [{'url': 'https://static.rust-lang.org/dist/pinned.tar.xz', 'sha256': 'a' * 64,
                            'members': [member]}]}
    sources = {'format_version': 1, 'toolchains': {'rustfmt': {'version': '1.99.1',
               'channel_manifest': {'sha256': 'b' * 64}, 'targets': {TARGET: source}}}}
    (resources / 'toolchain-sources.json').write_text(json.dumps(sources))
    catalog = {'format_version': 1, 'toolchains': {'rustfmt': {'version': '1.99.1',
               'targets': {TARGET: {'available': False}, 'other': {'available': False}}}}}
    if published:
        for spec in catalog['toolchains']['rustfmt']['targets'].values():
            spec.update(available=True, archives=[{
                'url': 'https://github.com/idkwhodatis/CodeStyles/releases/download/rustfmt-1.99.1/pinned.tar.gz',
                'sha256': 'c' * 64,
            }])
    (resources / 'toolchains.json').write_text(json.dumps(catalog))
    return sources


def build_fixture(bundle, sources, target=TARGET, empty_provenance=False):
    source = sources['toolchains']['rustfmt']['targets'][TARGET]
    members = [{**m, 'archive_path': m['path']} for a in source['archives'] for m in a['members']]
    stage = bundle.parent / 'stage'
    (stage / 'bin').mkdir(parents=True)
    (stage / 'bin/rustfmt.exe').write_bytes(b'test')
    receipt = {'format_version': 1, 'toolchain': 'rustfmt', 'version': '1.99.1', 'target': target,
               'channel_manifest': sources['toolchains']['rustfmt']['channel_manifest'],
               'source_archives': [{'url': a['url'], 'sha256': a['sha256']} for a in source['archives']],
               'files': members.copy()}
    provenance = stage / 'PROVENANCE.json'
    provenance.write_text('' if empty_provenance else json.dumps(receipt))
    members.append({'path': 'PROVENANCE.json', 'archive_path': 'PROVENANCE.json', 'bytes': provenance.stat().st_size,
                    'sha256': digest_file(provenance), 'executable': False})
    write_bundle(stage, bundle, members)
    installed = sum(m['bytes'] for m in members)
    return {'available': False, 'executable': source['executable'], 'environment': {},
            'max_installed_bytes': installed, 'archives': [{'format': 'tar.gz', 'asset_name': bundle.name,
            'sha256': digest_file(bundle), 'bytes': bundle.stat().st_size,
            'max_download_bytes': bundle.stat().st_size,
            'max_expanded_bytes': installed + len(members) * 1024 + 10240, 'members': members}]}


class NativeVerificationTests(unittest.TestCase):
    def test_restore_failure_success_and_cleanup_ownership(self):
        for published, fail in itertools.product((False, True), (False, True, "zero-tests", "last-git")):
            with self.subTest(published=published, fail=fail), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                sources = setup(root, published=published)
                catalog = root / 'resources/toolchains.json'
                before = catalog.read_bytes()
                work = root / 'work'; work.mkdir()
                keep = work / 'unrelated'; keep.write_text('keep')
                calls = []
                def runner(argv, cwd, env, log):
                    calls.append(argv)
                    log.write_text('fixture')
                    if fail == 'last-git' and log.name == 'catalog-after.log':
                        raise RuntimeError('fixture final tracked failure')
                    if argv[0] == 'rustc': return f'host: {TARGET}\n'
                    if '--candidate' in argv:
                        bundle = Path(argv[argv.index('--output') + 1])
                        candidate = build_fixture(bundle, sources)
                        Path(argv[argv.index('--candidate') + 1]).write_text(json.dumps(candidate))
                    if argv[0] == 'cargo':
                        self.assertIn('--exact', argv)
                        self.assertIn('--ignored', argv)
                        self.assertTrue(Path(env['CODESTYLES_TEST_RUSTFMT_ARCHIVE']).is_file())
                        owned = Path(env['CODESTYLES_TEST_RUSTFMT_ARCHIVE']).parent
                        for key in ('TMPDIR', 'TMP', 'TEMP'):
                            self.assertEqual(Path(env[key]), owned / 'runtime-temp')
                            self.assertTrue(Path(env[key]).is_dir())
                        targets = json.loads(catalog.read_text())['toolchains']['rustfmt']['targets']
                        self.assertIs(targets[TARGET]['available'], False)
                        self.assertNotIn('url', targets[TARGET])
                        self.assertTrue(all('url' not in archive for archive in targets[TARGET]['archives']))
                        self.assertEqual(targets['other'], json.loads(before)['toolchains']['rustfmt']['targets']['other'])
                        if fail is True: raise RuntimeError('fixture native failure')
                        if fail == 'zero-tests': return 'test result: ok. 0 passed; 0 failed; 0 ignored;'
                        return 'test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out;'
                    return ''
                args = (TARGET, work, root / 'reports')
                options = dict(root=root, runner=runner, disk_usage=lambda _: SimpleNamespace(free=MIN_FREE))
                if fail:
                    expected = ('native failure' if fail is True else
                                'final tracked failure' if fail == 'last-git' else 'one executed')
                    with self.assertRaisesRegex(RuntimeError, expected): verify(*args, **options)
                else:
                    self.assertEqual(verify(*args, **options)['native_execution'], 'passed')
                self.assertEqual(catalog.read_bytes(), before)
                self.assertEqual([argv for argv in calls if argv[0] == 'git'],
                                 [['git', 'diff', 'HEAD', '--exit-code']] * 2)
                self.assertEqual(list(work.iterdir()), [keep])
                report = json.loads((root / 'reports/report.json').read_text())
                self.assertTrue(report['catalog_restored'])
                self.assertTrue(report['own_temporary_removed'])
                self.assertEqual(report['native_execution'], 'failed' if fail else 'passed')

    def test_reject_candidate_trust_and_provenance(self):
        for mutation in ('available', 'url', 'hash', 'target', 'member', 'empty-provenance'):
            with self.subTest(mutation=mutation), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp); sources = setup(root); bundle = root / 'candidate.tar.gz'
                candidate = build_fixture(bundle, sources, 'wrong' if mutation == 'target' else TARGET,
                                          empty_provenance=mutation == 'empty-provenance')
                if mutation == 'available': candidate['available'] = True
                if mutation == 'url': candidate['archives'][0]['url'] = 'https://untrusted.invalid'
                if mutation == 'hash': candidate['archives'][0]['sha256'] = 'f' * 64
                if mutation == 'member': candidate['archives'][0]['members'][0]['bytes'] = 5
                with self.assertRaises(ValueError): validate_candidate(candidate, bundle, sources, TARGET)

    def test_termination_tree_dispatch_no_shell(self):
        process = Mock(pid=12345)
        process.poll.return_value = None
        with patch('verify_native_language_pack.os.killpg', create=True) as kill, \
                patch('verify_native_language_pack.signal.SIGKILL', 9, create=True):
            terminate_tree(process, windows=False)
            self.assertEqual(kill.call_args.args[0], 12345)
        with patch('verify_native_language_pack.subprocess.run') as child:
            terminate_tree(process, windows=True)
            argv = child.call_args.args[0]
            self.assertTrue(argv[0].endswith('System32/taskkill.exe') or argv[0].endswith(r'System32\taskkill.exe'))
            self.assertEqual(argv[1:], ['/PID', '12345', '/T', '/F'])
            self.assertFalse(child.call_args.kwargs['shell'])
            process.kill.assert_called_once()

    @unittest.skipIf(os.name == 'nt', 'POSIX process-group integration; Windows dispatch tested separately')
    def test_real_timeout_kills_group_and_returns(self):
        with tempfile.TemporaryDirectory() as tmp:
            env = dict(os.environ)
            env[DEADLINE_ENV] = str(time.monotonic() + 0.3)
            started = time.monotonic()
            with self.assertRaisesRegex(RuntimeError, 'timeout=True'):
                run([sys.executable, '-c',
                     'import subprocess,sys,time; subprocess.Popen([sys.executable,"-c","import time; time.sleep(20)"]); time.sleep(20)'],
                    Path(tmp), env, Path(tmp) / 'log')
            self.assertLess(time.monotonic() - started, 5)

    def test_disk_bound_before_child_and_catalog_mutation(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp); setup(root)
            before = (root / 'resources/toolchains.json').read_bytes()
            with self.assertRaisesRegex(ValueError, '2 GiB'):
                verify(TARGET, root / 'work', root / 'reports', root=root,
                       runner=lambda *args: self.fail('child must not run'),
                       disk_usage=lambda _: SimpleNamespace(free=MIN_FREE - 1))
            self.assertEqual((root / 'resources/toolchains.json').read_bytes(), before)


if __name__ == '__main__':
    unittest.main()
