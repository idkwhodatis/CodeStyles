"""Offline production-wrapper tests; never download or execute a runtime."""
import copy
import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest

from test_verify_native_language_pack import TARGET, setup, build_fixture
from verify_published_language_pack import MIN_FREE, TEST, published_pin, verify


def fixture(root):
    sources = setup(root)
    candidate = build_fixture(root / 'candidate.tar.gz', sources)
    candidate['available'] = True
    candidate['archives'][0]['url'] = 'https://github.com/idkwhodatis/CodeStyles/releases/download/rustfmt-1.99.1/candidate.tar.gz'
    path = root / 'resources/toolchains.json'
    catalog = json.loads(path.read_bytes())
    catalog['toolchains']['rustfmt']['targets'][TARGET] = candidate
    path.write_text(json.dumps(catalog))
    return catalog, sources


class PublishedVerificationTests(unittest.TestCase):
    def test_success_failure_zero_tests_and_owned_cleanup(self):
        for outcome in ('pass', 'failure', 'zero', 'wrong-host'):
            with self.subTest(outcome=outcome), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp); fixture(root)
                catalog = root / 'resources/toolchains.json'
                original = catalog.read_bytes()
                work = root / 'work'; work.mkdir()
                sentinel = work / 'unrelated'; sentinel.write_text('keep')
                calls = []
                def runner(argv, cwd, env, log):
                    calls.append(argv); log.write_text('fixture')
                    if argv[0] == 'rustc':
                        return 'host: wrong\n' if outcome == 'wrong-host' else f'host: {TARGET}\n'
                    if argv[0] == 'cargo':
                        self.assertIn(TEST, argv)
                        self.assertIn('--exact', argv)
                        self.assertIn('--ignored', argv)
                        self.assertNotIn('CODESTYLES_TEST_RUSTFMT_ARCHIVE', env)
                        self.assertEqual(env['TMP'], env['TEMP'])
                        self.assertEqual(env['TMP'], env['TMPDIR'])
                        self.assertTrue(Path(env['TMP']).is_relative_to(work))
                        self.assertEqual(catalog.read_bytes(), original)
                        Path(env['TMP']).joinpath('installed-fixture').write_text('runtime')
                        if outcome == 'failure': raise RuntimeError('child failed')
                        count = 0 if outcome == 'zero' else 1
                        return f'test result: ok. {count} passed; 0 failed; 0 ignored;\n'
                    return ''
                options = dict(root=root, runner=runner, disk_usage=lambda _: SimpleNamespace(free=MIN_FREE))
                if outcome == 'pass':
                    self.assertEqual(verify(TARGET, work, root / 'reports', **options)['production_download'], 'passed')
                else:
                    with self.assertRaises((RuntimeError, ValueError)):
                        verify(TARGET, work, root / 'reports', **options)
                self.assertEqual(catalog.read_bytes(), original)
                self.assertEqual(list(work.iterdir()), [sentinel])
                report = json.loads((root / 'reports/report.json').read_bytes())
                self.assertTrue(report['catalog_unchanged'])
                self.assertTrue(report['own_temporary_removed'])
                self.assertEqual([c for c in calls if c[0] == 'git'], [['git', 'diff', 'HEAD', '--exit-code']] * 2)

    def test_published_pins_reject_unpublished_urls_digests_and_source_drift(self):
        with tempfile.TemporaryDirectory() as tmp:
            catalog, sources = fixture(Path(tmp))
            published_pin(catalog, sources, TARGET)
            for mutation in ('unpublished', 'url', 'hash', 'source', 'version', 'bound', 'duplicate'):
                with self.subTest(mutation=mutation):
                    changed = copy.deepcopy(catalog)
                    tool = changed['toolchains']['rustfmt']; spec = tool['targets'][TARGET]
                    archive = spec['archives'][0]
                    if mutation == 'unpublished': spec['available'] = False
                    if mutation == 'url': archive['url'] = 'https://github.com/other/CodeStyles/releases/download/v/candidate.tar.gz'
                    if mutation == 'hash': archive['sha256'] = 'not-a-pin'
                    if mutation == 'source': archive['members'][0]['sha256'] = 'f' * 64
                    if mutation == 'version': tool['version'] = 'next'
                    if mutation == 'bound': archive['max_download_bytes'] += 1
                    if mutation == 'duplicate': archive['members'].append(archive['members'][0])
                    with self.assertRaises(ValueError): published_pin(changed, sources, TARGET)

    def test_low_disk_and_unpublished_preflight_run_no_child(self):
        for unpublished in (False, True):
            with self.subTest(unpublished=unpublished), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp); catalog, _ = fixture(root)
                if unpublished:
                    catalog['toolchains']['rustfmt']['targets'][TARGET]['available'] = False
                    (root / 'resources/toolchains.json').write_text(json.dumps(catalog))
                with self.assertRaises(ValueError):
                    verify(TARGET, root / 'work', root / 'reports', root=root,
                           runner=lambda *args: self.fail('preflight must not run a child'),
                           disk_usage=lambda _: SimpleNamespace(free=MIN_FREE if unpublished else MIN_FREE - 1))


if __name__ == '__main__':
    unittest.main()
