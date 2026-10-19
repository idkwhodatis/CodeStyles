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


    def test_api_delete_accepts_empty_204_body(self):
        with patch.object(release, 'gh', return_value='') as gh:
            self.assertIsNone(release.api('releases/assets/123', 'DELETE'))
        gh.assert_called_once_with('api', f'repos/{release.REPO}/releases/assets/123',
                                   '--method', 'DELETE')

    def test_tag_push_requires_exact_old_sha_lease(self):
        with patch.object(release.subprocess, 'run') as run:
            release.move_tag_with_lease()
        fetch, push = [c.args[0] for c in run.call_args_list]
        self.assertEqual(fetch[-4:], ['fetch', '--no-tags',
                                     f'https://github.com/{release.REPO}.git', release.SHA])
        self.assertIn(f'--force-with-lease=refs/tags/{release.TAG}:{release.OLD_SHA}', push)
        self.assertEqual(push[-1], f'{release.SHA}:refs/tags/{release.TAG}')
        self.assertNotIn('--force', push)
        self.assertTrue(all(c.kwargs == {'check': True} for c in run.call_args_list))

    def test_ordered_downloads(self):
        body = release.release_body()
        self.assertLess(body.index('### Windows'), body.index('### macOS'))
        self.assertLess(body.index('### macOS'), body.index('### Linux'))
        for section in body.split('### ')[1:]:
            self.assertLess(section.index('**x64**'), section.index('**ARM64**'))

    def refresh_fixture(self):
        root = self.prepared()
        new = Path(self.temp.name) / 'new'
        new.mkdir()
        for path in list(root.iterdir()):
            if path.is_file():
                path.rename(new / path.name)
        previous = root / 'previous'
        previous.mkdir()
        for path in new.iterdir():
            (previous / path.name).write_bytes(path.read_bytes())
        provenance = json.loads((previous / 'PROVENANCE.json').read_text())
        provenance.update(commit=release.OLD_SHA, run_id=release.OLD_RUN)
        (previous / 'PROVENANCE.json').write_text(json.dumps(provenance))
        # All twelve packages differ, as a real UI rebuild does.
        for target in release.TARGETS:
            for name in release.names(target):
                (previous / name).write_bytes(b'old package')
        (previous / 'SHA256SUMS').write_text(''.join(
            f'{release.digest(p)}  {p.name}\n' for p in sorted(previous.iterdir())
            if p.name != 'SHA256SUMS'))
        expected = release.verify_files(new)
        old = release.verify_files(previous, release.OLD_RUN, release.OLD_SHA)
        self.state = {
            'tag': release.OLD_SHA,
            'release': {'id': release.RELEASE_ID, 'tag_name': release.TAG,
                        'prerelease': True, 'draft': False, 'body': 'Old alpha',
                        'target_commitish': release.OLD_SHA,
                        'html_url': f'https://github.com/{release.REPO}/releases/tag/{release.TAG}'},
            'assets': [{'id': i, 'name': n, 'digest': 'sha256:' + d, 'state': 'uploaded'}
                       for i, (n, d) in enumerate(old.items(), 1)], 'writes': []}
        def api(path, method='GET', payload=None):
            if method != 'GET':
                self.state['writes'].append((path, method, payload))
            if path.startswith('git/matching-refs'):
                return [{'ref': f'refs/tags/{release.TAG}',
                         'object': {'type': 'commit', 'sha': self.state['tag']}}]
            if method == 'DELETE':
                asset_id = int(path.split('/')[-1])
                self.state['assets'] = [a for a in self.state['assets'] if a['id'] != asset_id]
                return None
            if path.startswith('git/refs'):
                self.state['tag'] = payload['sha']
                return {}
            if path == f'releases/{release.RELEASE_ID}':
                if payload:
                    self.state['release'].update(payload)
                return dict(self.state['release'])
            raise AssertionError(path)
        def gh(*args):
            self.state['writes'].append(args)
            name = Path(args[3]).name
            self.state['assets'].append({'id': 1000 + len(self.state['writes']), 'name': name,
                                        'digest': 'sha256:' + expected[name], 'state': 'uploaded'})
        self.enterContext(patch.object(release, 'api', side_effect=api))
        self.enterContext(patch.object(release, 'pages', side_effect=lambda _: list(self.state['assets'])))
        self.upload = self.enterContext(patch.object(release, 'gh', side_effect=gh))
        def move_tag():
            self.assertEqual(self.state['tag'], release.OLD_SHA)
            self.state['writes'].append(('git/refs/lease', 'PUSH'))
            self.state['tag'] = release.SHA
        self.enterContext(patch.object(release, 'move_tag_with_lease', side_effect=move_tag))
        backup = root / 'backup'
        release.snapshot(new, previous, backup)
        return new, previous, backup

    def test_first_refresh_then_noop(self):
        args = self.refresh_fixture()
        release.publish(*args)
        self.assertEqual(self.state['tag'], release.SHA)
        self.assertEqual(self.upload.call_count, 14)
        self.assertEqual(self.state['release']['make_latest'], 'false')
        self.assertEqual(len(self.state['assets']), 14)
        self.state['writes'].clear()
        release.publish(*args)
        self.assertEqual(self.state['writes'], [])

    def test_retry_after_delete_before_upload(self):
        args = self.refresh_fixture()
        self.state['assets'].pop(0)
        release.publish(*args)
        self.assertEqual(len(self.state['assets']), 14)

    def test_retry_mixed_old_new_assets(self):
        args = self.refresh_fixture()
        expected = release.verify_files(args[0])
        for asset in self.state['assets'][:5]:
            asset['digest'] = 'sha256:' + expected[asset['name']]
        release.publish(*args)
        self.assertEqual(self.upload.call_count, 9)

    def test_retry_after_tag_move(self):
        args = self.refresh_fixture()
        self.state['tag'] = release.SHA
        release.publish(*args)
        self.assertFalse(any(w[0].startswith('git/refs') for w in self.state['writes']))

    def test_unexpected_release_states(self):
        args = self.refresh_fixture()
        original = dict(self.state['release'])
        for key, value in [('id', 7), ('draft', True), ('prerelease', False),
                           ('immutable', True), ('tag_name', 'other'), ('html_url', 'other')]:
            with self.subTest(key=key):
                self.state['release'] = dict(original, **{key: value})
                with self.assertRaises(ValueError):
                    release.publish(*args)
                self.assertEqual(self.state['writes'], [])

    def test_unknown_duplicate_and_incomplete_assets(self):
        args = self.refresh_fixture()
        original = [dict(a) for a in self.state['assets']]
        bad_sets = [original + [dict(original[0])],
                    original + [dict(original[0], name='unexpected')],
                    [dict(original[0], state='starter')] + original[1:]]
        for assets in bad_sets:
            self.state['assets'] = assets
            with self.assertRaises(ValueError):
                release.publish(*args)
            self.assertEqual(self.state['writes'], [])

    def test_wrong_digest_never_mutates(self):
        args = self.refresh_fixture()
        self.state['assets'][0]['digest'] = 'sha256:' + '0' * 64
        with self.assertRaises(ValueError):
            release.publish(*args)
        self.assertEqual(self.state['writes'], [])

    def test_wrong_tag_never_mutates(self):
        args = self.refresh_fixture()
        self.state['tag'] = 'bad'
        with self.assertRaises(ValueError):
            release.publish(*args)
        self.assertEqual(self.state['writes'], [])

    def test_missing_backup_never_mutates(self):
        args = self.refresh_fixture()
        (args[2] / 'before-refresh.json').unlink()
        with self.assertRaises(FileNotFoundError):
            release.publish(*args)
        self.assertEqual(self.state['writes'], [])

if __name__ == '__main__':
    unittest.main()
