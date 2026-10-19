import json
from pathlib import Path
import tempfile
import unittest
import zipfile
from unittest.mock import patch

import release_alpha as release


class ReleaseTests(unittest.TestCase):
    def fixture(self):
        run = {'id': release.RUN, 'head_sha': release.SHA,
               'repository': {'full_name': release.REPO},
               'head_repository': {'full_name': release.REPO},
               'event': 'push', 'head_branch': 'master', 'path': '.github/workflows/ci.yml',
               'status': 'completed', 'conclusion': 'success'}
        jobs = [{'name': ('Linux old-ABI' if 'linux' in t else 'Native') + f' ({t})',
                 'status': 'completed', 'conclusion': 'success', 'head_sha': release.SHA}
                for t in release.TARGETS]
        artifacts = [{'name': f'codestyles-{t}', 'expired': False, 'digest': 'sha256:' + '0' * 64,
                      'workflow_run': {'id': release.RUN, 'head_sha': release.SHA}}
                     for t in release.TARGETS]
        return run, jobs, artifacts

    def test_trusted_run(self):
        release.validate_run(*self.fixture())

    def test_untrusted_runs(self):
        for key, value in [('id', 1), ('head_sha', '0' * 40), ('event', 'pull_request'),
                           ('head_branch', 'feature'), ('path', 'evil.yml'),
                           ('status', 'in_progress'), ('conclusion', 'failure'),
                           ('head_repository', {'full_name': 'fork/CodeStyles'})]:
            with self.subTest(key=key):
                run, jobs, artifacts = self.fixture()
                run[key] = value
                with self.assertRaises(ValueError):
                    release.validate_run(run, jobs, artifacts)

    def test_missing_or_failed_job(self):
        for mutation in (lambda j: j.pop(), lambda j: j[0].update(conclusion='skipped'),
                         lambda j: j[0].update(head_sha='bad'), lambda j: j.append(j[0])):
            run, jobs, artifacts = self.fixture()
            mutation(jobs)
            with self.assertRaises(ValueError):
                release.validate_run(run, jobs, artifacts)

    def test_invalid_artifacts(self):
        for update in ({'expired': True}, {'digest': None}, {'name': 'unexpected'},
                       {'workflow_run': {'id': 1, 'head_sha': release.SHA}}):
            run, jobs, artifacts = self.fixture()
            artifacts[0].update(update)
            with self.assertRaises(ValueError):
                release.validate_run(run, jobs, artifacts)

    def archive(self, target, prefix='', extra=None):
        root = Path(self.temp.name)
        archive = root / 'input.zip'
        with zipfile.ZipFile(archive, 'w') as bundle:
            for name in release.names(target):
                bundle.writestr(prefix + name, b'package bytes; never execute')
            if extra:
                bundle.writestr(*extra)
        output = root / 'out'
        output.mkdir()
        return archive, output

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)

    def test_flat_native_packages(self):
        target = release.TARGETS[1]
        archive, output = self.archive(target)
        release.unpack_packages(archive, target, output)
        self.assertEqual({p.name for p in output.iterdir()}, set(release.names(target)))

    def test_linux_dist_packages(self):
        target = 'x86_64-unknown-linux-gnu'
        archive, output = self.archive(target, 'dist/', ('isolated-reports/result.json', '{}'))
        release.unpack_packages(archive, target, output)
        self.assertEqual(len(list(output.iterdir())), 2)

    def test_traversal_rejected(self):
        archive, output = self.archive(release.TARGETS[0], extra=('../escape', 'bad'))
        with self.assertRaises(ValueError):
            release.unpack_packages(archive, release.TARGETS[0], output)

    def test_symlink_rejected(self):
        member = zipfile.ZipInfo('link')
        member.external_attr = 0o120777 << 16
        archive, output = self.archive(release.TARGETS[0], extra=(member, 'bad'))
        with self.assertRaises(ValueError):
            release.unpack_packages(archive, release.TARGETS[0], output)

    def test_duplicate_package_rejected(self):
        target = release.TARGETS[0]
        archive, output = self.archive(target, extra=('dist/' + release.names(target)[0], 'bad'))
        with self.assertRaises(ValueError):
            release.unpack_packages(archive, target, output)

    def test_missing_package_rejected(self):
        archive, output = self.archive(release.TARGETS[0])
        with self.assertRaises(ValueError):
            release.unpack_packages(archive, release.TARGETS[1], output)

    def prepared(self):
        root = Path(self.temp.name)
        for target in release.TARGETS:
            for name in release.names(target):
                (root / name).write_bytes(b'package')
        (root / 'PROVENANCE.json').write_text(json.dumps({
            'repository': release.REPO, 'commit': release.SHA,
            'run_id': release.RUN, 'tag': release.TAG}))
        manifest = ''.join(f'{release.digest(p)}  {p.name}\n' for p in sorted(root.iterdir()))
        (root / 'SHA256SUMS').write_text(manifest)
        return root

    def test_checksums(self):
        self.assertEqual(len(release.verify_files(self.prepared())), 14)

    def test_tampered_package(self):
        root = self.prepared()
        (root / release.names(release.TARGETS[0])[0]).write_bytes(b'tampered')
        with self.assertRaises(ValueError):
            release.verify_files(root)



    def test_ordered_downloads_and_source_warning(self):
        body = release.release_body()
        self.assertLess(body.index('### Windows'), body.index('### macOS'))
        self.assertLess(body.index('### macOS'), body.index('### Linux'))
        for section in body.split('### ')[1:]:
            self.assertLess(section.index('**x64**'), section.index('**ARM64**'))
        self.assertIn(f'tag remains at commit {release.OLD_SHA}', body)
        self.assertIn(f'/archive/{release.SHA}.zip', body)
        self.assertIn(f'/archive/{release.SHA}.tar.gz', body)

    def notes_fixture(self):
        root = self.prepared()
        checksums = release.verify_files(root)
        self.tag = release.OLD_SHA
        self.remote = {'id': release.RELEASE_ID, 'tag_name': release.TAG,
                       'prerelease': True, 'draft': False, 'body': 'old notes',
                       'target_commitish': release.OLD_SHA,
                       'html_url': f'https://github.com/{release.REPO}/releases/tag/{release.TAG}'}
        self.assets = [{'id': i, 'name': n, 'digest': 'sha256:' + d, 'state': 'uploaded'}
                       for i, (n, d) in enumerate(checksums.items(), 1)]
        self.writes = []
        def api(path, method='GET', payload=None):
            if method != 'GET':
                self.writes.append((path, method, payload))
                self.assertEqual(path, f'releases/{release.RELEASE_ID}')
                self.assertEqual(method, 'PATCH')
                self.assertEqual(set(payload), {'body'})
                self.remote.update(payload)
            if path.startswith('git/matching-refs'):
                return [{'ref': f'refs/tags/{release.TAG}',
                         'object': {'type': 'commit', 'sha': self.tag}}]
            self.assertEqual(path, f'releases/{release.RELEASE_ID}')
            return dict(self.remote)
        self.enterContext(patch.object(release, 'api', side_effect=api))
        self.enterContext(patch.object(release, 'pages', side_effect=lambda _: list(self.assets)))
        self.gh = self.enterContext(patch.object(release, 'gh', side_effect=AssertionError('No uploads')))
        self.process = self.enterContext(patch.object(release.subprocess, 'run',
                                                     side_effect=AssertionError('No git writes')))
        return root

    def test_finalize_changes_only_body_and_retry_is_noop(self):
        root = self.notes_fixture()
        release.finalize_notes(root)
        self.assertEqual(self.writes, [(f'releases/{release.RELEASE_ID}', 'PATCH',
                                        {'body': release.release_body()})])
        self.assertEqual(self.tag, release.OLD_SHA)
        self.assertEqual(self.remote['target_commitish'], release.OLD_SHA)
        self.writes.clear()
        release.finalize_notes(root)
        self.assertEqual(self.writes, [])
        self.gh.assert_not_called()
        self.process.assert_not_called()

    def test_unexpected_tag_never_mutates(self):
        root = self.notes_fixture()
        for sha in [release.SHA, 'unknown']:
            self.tag = sha
            with self.assertRaises(ValueError):
                release.finalize_notes(root)
            self.assertEqual(self.writes, [])

    def test_wrong_release_never_mutates(self):
        root = self.notes_fixture()
        original = dict(self.remote)
        for key, value in [('id', 7), ('draft', True), ('prerelease', False),
                           ('immutable', True), ('tag_name', 'other'), ('html_url', 'other')]:
            with self.subTest(key=key):
                self.remote = dict(original, **{key: value})
                with self.assertRaises(ValueError):
                    release.finalize_notes(root)
                self.assertEqual(self.writes, [])

    def test_incomplete_or_unexpected_assets_never_mutate(self):
        root = self.notes_fixture()
        original = [dict(a) for a in self.assets]
        variants = [original[:-1], original + [dict(original[0])],
                    [dict(original[0], digest='sha256:' + '0' * 64)] + original[1:],
                    [dict(original[0], name='unexpected')] + original[1:],
                    [dict(original[0], state='starter')] + original[1:]]
        for assets in variants:
            self.assets = assets
            with self.assertRaises(ValueError):
                release.finalize_notes(root)
            self.assertEqual(self.writes, [])

if __name__ == '__main__':
    unittest.main()
