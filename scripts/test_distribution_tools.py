"""Host-independent checks of native-container extraction and signing sequencing."""
import hashlib
import io
import json
from pathlib import Path
import subprocess
import tarfile
import tempfile
import unittest
from unittest import mock

import check_distribution
import package_app
import sign_macos


class DistributionToolsTests(unittest.TestCase):
    def test_extracts_real_debian_payload(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            stage = root / 'stage'
            (stage / 'usr/lib/codestyles').mkdir(parents=True)
            (stage / 'usr/lib/codestyles/codestyles').write_bytes(b'app')
            deb = root / 'test.deb'
            package_app.write_ar(deb, [('debian-binary', b'2.0\n'),
                                     ('data.tar.gz', package_app.tar_bytes(stage))])
            check_distribution.extract_deb(deb, root / 'out')
            self.assertEqual((root / 'out/usr/lib/codestyles/codestyles').read_bytes(), b'app')

    def test_debian_extraction_rejects_traversal(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            content = io.BytesIO()
            with tarfile.open(fileobj=content, mode='w:gz') as archive:
                member = tarfile.TarInfo('../escape')
                member.size = 1
                archive.addfile(member, io.BytesIO(b'x'))
            deb = root / 'bad.deb'
            package_app.write_ar(deb, [('data.tar.gz', content.getvalue())])
            with self.assertRaises(tarfile.FilterError):
                check_distribution.extract_deb(deb, root / 'out')
            self.assertFalse((root / 'escape').exists())

    def test_notarization_requires_explicit_acceptance(self):
        with tempfile.TemporaryDirectory() as temporary:
            report = Path(temporary) / 'notary.json'
            for status in ('Invalid', 'Rejected', 'In Progress'):
                response = subprocess.CompletedProcess([], 0, json.dumps({'status': status}), '')
                with mock.patch.object(sign_macos.subprocess, 'run', return_value=response):
                    with self.assertRaises(RuntimeError):
                        sign_macos.notarize('app.zip', 'existing-profile', report)
            response = subprocess.CompletedProcess([], 0, '{"status":"Accepted"}', '')
            with mock.patch.object(sign_macos.subprocess, 'run', return_value=response):
                sign_macos.notarize('app.zip', 'existing-profile', report)
            self.assertEqual(json.loads(report.read_text())['status'], 'Accepted')

    def test_signing_verifies_upstream_and_updates_only_signed_executable(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            app = root / 'CodeStyles.app'
            resources = app / 'Contents/Resources/resources'
            target = 'aarch64-apple-darwin'
            dprint_path = f'bin/{target}/dprint'
            original = b'upstream-dprint'
            plugin = b'pinned-wasm'
            records = []
            for path, data in ((dprint_path, original), ('plugins/a.wasm', plugin)):
                file = resources / path
                file.parent.mkdir(parents=True, exist_ok=True)
                file.write_bytes(data)
                records.append({'path': path, 'sha256': hashlib.sha256(data).hexdigest(), 'bytes': len(data)})
            manifest = {'files': records}
            (resources / 'manifest.json').write_text(json.dumps(manifest))
            (root / 'resources').mkdir()
            (root / 'resources/manifest.json').write_text(json.dumps(manifest))
            calls = []

            def command(*args):
                calls.append(args)
                if args[0] == 'codesign' and args[-1] == resources / dprint_path:
                    (resources / dprint_path).write_bytes(original + b'-signature')

            with mock.patch.object(sign_macos, 'ROOT', root), \
                    mock.patch.object(sign_macos, 'run', side_effect=command), \
                    mock.patch.object(sign_macos, 'notarize'):
                sign_macos.sign(app, target, 'existing-identity', 'existing-profile', root / 'signed')
            final = json.loads((resources / 'manifest.json').read_text())['files']
            self.assertEqual(final[0]['upstream_sha256'], records[0]['sha256'])
            self.assertEqual(final[0]['sha256'], hashlib.sha256(original + b'-signature').hexdigest())
            self.assertEqual(final[1], records[1])
            smoke_index = next(i for i, args in enumerate(calls) if 'smoke_package.py' in str(args))
            seal_index = next(i for i, args in enumerate(calls) if args[0] == 'codesign' and args[-1] == app)
            self.assertGreater(smoke_index, seal_index)
            # Re-signing transformed bytes must not bypass the upstream trust pin.
            with mock.patch.object(sign_macos, 'ROOT', root), mock.patch.object(sign_macos, 'run') as run:
                with self.assertRaises(ValueError):
                    sign_macos.sign(app, target, 'identity', 'profile', root / 'signed')
                run.assert_not_called()


if __name__ == '__main__':
    unittest.main()
