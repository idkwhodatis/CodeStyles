"""Offline orchestrator fixtures only: no native execution or compiler invoked."""
import hashlib
import errno
import json
from pathlib import Path
import shutil
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch
from candidate_pipeline import acceptance

TARGET = 'x86_64-unknown-linux-gnu'
COMMIT = 'a' * 40


class AcceptanceTests(unittest.TestCase):
    def exercise(self, failure=None, tool='gofmt'):
        with tempfile.TemporaryDirectory() as tmp:
            base = Path(tmp)
            root = base / 'canonical'
            (root / 'resources').mkdir(parents=True)
            catalog = {'format_version': 1, 'toolchains': {tool: {'version': '1', 'targets': {
                TARGET: {'available': False, 'executable': 'bin/tool', 'environment': {}}, 'other': {'available': False}}}}}
            source_catalog = root / 'resources/toolchains.json'
            original = json.dumps(catalog).encode()
            source_catalog.write_bytes(original)
            plan = base / 'plan.json'
            plan.write_text(json.dumps({'tool': tool, 'target': TARGET, 'version': '1'}))
            manifest = base / 'manifest.json'
            manifest.write_text(json.dumps({'available': False, 'release_clearance': False,
                                            'executable': 'bin/tool', 'environment': {}}))
            bundle = base / 'candidate.tar.gz'
            bundle.write_bytes(b'fixture only')
            toolchain = base / 'installed-toolchain'
            toolchain.mkdir()
            for binary in ('rustc', 'cargo'):
                (toolchain / binary).write_text('fixture only')
            work, reports = base / 'work', base / 'reports'
            work.mkdir()
            (work / 'unrelated').write_text('keep')
            calls = []
            cloned = []

            def runner(argv, cwd, env, log):
                calls.append(argv)
                log.write_text('fixture only')
                if argv[:2] == ['git', 'rev-parse']:
                    return ('b' * 40 if failure == 'commit' else COMMIT) + '\n'
                if argv[:2] == ['rustup', 'which']:
                    self.assertEqual(env['RUSTUP_AUTO_INSTALL'], '0')
                    if failure == 'toolchain':
                        raise RuntimeError('Pinned toolchain missing; no install attempted')
                    return str(toolchain / argv[-1])
                if Path(argv[0]).stem == 'rustc':
                    return 'host: ' + ('other-target' if failure == 'host' else TARGET) + '\n'
                if argv[:2] == ['git', 'clone']:
                    clone = Path(argv[-1])
                    cloned.append(clone)
                    shutil.copytree(root, clone)
                if Path(argv[0]).stem == 'cargo':
                    cwd = Path(cwd)
                    self.assertNotEqual(cwd, root)
                    self.assertEqual(source_catalog.read_bytes(), original)
                    self.assertIn('--offline', argv)
                    self.assertIn('--exact', argv)
                    self.assertIn('--ignored', argv)
                    self.assertIn(acceptance.TESTS[tool][1], argv)
                    self.assertTrue(Path(env[acceptance.TESTS[tool][0]]).is_file())
                    self.assertEqual(env['CARGO_NET_OFFLINE'], 'true')
                    candidate = json.loads((cwd / 'resources/toolchains.json').read_text())
                    self.assertFalse(candidate['toolchains'][tool]['targets'][TARGET]['available'])
                    self.assertEqual(candidate['toolchains'][tool]['targets']['other'], {'available': False})
                    if failure == 'canonical-change':
                        source_catalog.write_text('externally changed')
                    if failure == 'test':
                        raise RuntimeError('simulated failure')
                    if failure == 'zero':
                        return 'test result: ok. 0 passed; 0 failed; 0 ignored;'
                    return 'test result: ok. 1 passed; 0 failed; 0 ignored;'
                return ''

            with patch.object(acceptance, 'validate_candidate') as validator:
                kwargs = dict(runner=runner, disk_usage=lambda _: SimpleNamespace(free=acceptance.MIN_FREE))
                args = (plan, hashlib.sha256(plan.read_bytes()).hexdigest(), manifest, bundle,
                        root, COMMIT, work, reports)
                if failure:
                    with self.assertRaises((ValueError, RuntimeError)):
                        acceptance.verify(*args, **kwargs)
                else:
                    result = acceptance.verify(*args, **kwargs)
                    self.assertEqual(result['native_execution'], 'passed')
                    self.assertTrue(result['isolated_catalog_restored'])
                self.assertTrue(validator.called)
            report = json.loads((reports / 'report.json').read_text())
            self.assertTrue(report['temporary_removed'])
            self.assertTrue(all(not path.exists() for path in cloned))
            self.assertEqual((work / 'unrelated').read_text(), 'keep')
            if failure == 'canonical-change':
                self.assertEqual(source_catalog.read_text(), 'externally changed')
                self.assertFalse(report['canonical_catalog_unchanged'])
            else:
                self.assertEqual(source_catalog.read_bytes(), original)
            if failure:
                self.assertEqual(report['native_execution'], 'failed')
            if failure in ('test', 'zero', 'canonical-change'):
                self.assertTrue(report['isolated_catalog_restored'])
            if failure in ('commit', 'host', 'toolchain'):
                self.assertFalse(any(Path(x[0]).stem == 'cargo' for x in calls))

    def test_success_for_each_fixed_test(self):
        for tool in acceptance.TESTS:
            with self.subTest(tool=tool):
                self.exercise(tool=tool)

    def test_failure_restoration_and_zero_test_rejection(self):
        for failure in ('test', 'zero', 'canonical-change', 'commit', 'host', 'toolchain'):
            with self.subTest(failure=failure):
                self.exercise(failure)

    def test_resource_symlink_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / 'file').write_text('safe')
            try:
                (root / 'link').symlink_to(root / 'file')
            except OSError as error:
                if getattr(error, 'winerror', None) == 1314 or error.errno in (errno.EPERM, errno.ENOSYS, errno.EOPNOTSUPP):
                    self.skipTest('Symlink creation unavailable on this runner')
                raise
            with self.assertRaisesRegex(ValueError, 'nonregular'):
                acceptance.regular_tree(root)

    def test_existing_report_directory_is_never_adopted(self):
        with tempfile.TemporaryDirectory() as tmp:
            base = Path(tmp)
            root = base / 'canonical'
            root.mkdir()
            reports = base / 'reports'
            reports.mkdir()
            sentinel = reports / 'before.log'
            sentinel.write_text('must not overwrite')
            plan, manifest, bundle = (base / name for name in ('plan.json', 'manifest.json', 'bundle'))
            plan.write_text(json.dumps({'tool': 'gofmt', 'target': TARGET, 'version': '1'}))
            manifest.write_text('{}')
            bundle.write_bytes(b'fixture')
            with patch.object(acceptance, 'validate_candidate'), patch.object(acceptance, 'run') as runner:
                with self.assertRaises(FileExistsError):
                    acceptance.verify(plan, hashlib.sha256(plan.read_bytes()).hexdigest(), manifest,
                                      bundle, root, COMMIT, base / 'work', reports, runner=runner)
                runner.assert_not_called()
            self.assertEqual(sentinel.read_text(), 'must not overwrite')


if __name__ == '__main__':
    unittest.main()
