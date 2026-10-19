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
        target = release.TARGETS[0]
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

    def test_wrong_tag_never_written(self):
        root = self.prepared()
        with patch.object(release, 'api', return_value=[{
                'ref': 'refs/tags/' + release.TAG, 'object': {'type': 'commit', 'sha': 'bad'}}]) as api:
            with self.assertRaises(ValueError):
                release.publish(root)
            self.assertEqual(api.call_count, 1)

    def test_existing_asset_never_replaced(self):
        root = self.prepared()
        with patch.object(release, 'api', return_value=[{
                'ref': 'refs/tags/' + release.TAG,
                'object': {'type': 'commit', 'sha': release.SHA}}]), patch.object(
                release, 'pages', side_effect=[
                    [{'tag_name': release.TAG, 'prerelease': True, 'draft': True, 'id': 1}],
                    [{'name': 'SHA256SUMS', 'digest': 'sha256:wrong'}]]), patch.object(release, 'gh') as gh:
            with self.assertRaises(ValueError):
                release.publish(root)
            gh.assert_not_called()


    def test_published_retry_is_read_only(self):
        root = self.prepared()
        checksums = release.verify_files(root)
        published = {'tag_name': release.TAG, 'prerelease': True, 'draft': False, 'id': 1}
        assets = [{'name': n, 'digest': 'sha256:' + d} for n, d in checksums.items()]
        refs = [{'ref': 'refs/tags/' + release.TAG,
                 'object': {'type': 'commit', 'sha': release.SHA}}]
        with patch.object(release, 'api', side_effect=[refs, published]) as api, patch.object(
                release, 'pages', side_effect=[[published], assets, assets]), patch.object(
                release, 'gh') as gh:
            release.publish(root)
            gh.assert_not_called()
            self.assertTrue(all(len(call.args) == 1 for call in api.call_args_list))

    def test_draft_publishes_only_after_asset_validation(self):
        root = self.prepared()
        checksums = release.verify_files(root)
        draft = {'tag_name': release.TAG, 'prerelease': True, 'draft': True, 'id': 1}
        final = dict(draft, draft=False)
        assets = [{'name': n, 'digest': 'sha256:' + d} for n, d in checksums.items()]
        refs = [{'ref': 'refs/tags/' + release.TAG,
                 'object': {'type': 'commit', 'sha': release.SHA}}]
        with patch.object(release, 'api', side_effect=[refs, final, final]) as api, patch.object(
                release, 'pages', side_effect=[[draft], [], assets]), patch.object(
                release, 'gh') as gh:
            release.publish(root)
            self.assertEqual(gh.call_count, 14)
            self.assertEqual(api.call_args_list[1].args,
                             ('releases/1', 'PATCH', {'draft': False, 'prerelease': True,
                                                      'make_latest': 'false'}))

if __name__ == '__main__':
    unittest.main()
