#!/usr/bin/env python3
"""Offline tests for resource integrity and zip reproducibility."""
import hashlib
import json
import os
import shutil
import zipfile
from unittest import mock
import importlib.util
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location('bundle_resources', Path(__file__).with_name('bundle_resources.py'))
bundle = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(bundle)


class IntegrityTests(unittest.TestCase):
    def test_manifest_has_all_six_native_targets(self):
        manifest = json.loads((bundle.RESOURCES / "manifest.json").read_text())
        expected = {f"{arch}-{platform}" for arch in ("x86_64", "aarch64")
                    for platform in ("apple-darwin", "pc-windows-msvc", "unknown-linux-gnu")}
        actual = {a["target"] for a in manifest["artifacts"] if "target" in a}
        self.assertEqual(actual, expected)
        for target in expected:
            selected = bundle.selected(manifest, target)
            executables = [a for a in selected if a["kind"] == "executable-archive"]
            self.assertEqual(len(executables), 1)
            record = executables[0]["executable"]
            self.assertTrue(any(f["path"] == record["path"] and f["sha256"] == record["sha256"]
                                for f in manifest["files"]))

    def test_valid_and_corrupt_resources(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'resource'
            path.write_bytes(b'correct')
            entry = {'bytes': 7, 'sha256': hashlib.sha256(b'correct').hexdigest()}
            bundle.verify(path, entry)
            path.write_bytes(b'corrupt')
            with self.assertRaisesRegex(ValueError, 'SHA-256 mismatch'):
                bundle.verify(path, entry)
            path.write_bytes(b'wrong length')
            with self.assertRaisesRegex(ValueError, 'Size mismatch'):
                bundle.verify(path, entry)

    def test_missing_resource_fails_closed(self):
        with self.assertRaisesRegex(ValueError, 'Missing resource'):
            bundle.verify(Path('/nonexistent/codestyles-resource'), {})

    def test_target_selection_includes_shared_plugins(self):
        manifest = {'artifacts': [{'target': 'one'}, {'target': 'two'}, {'kind': 'wasm'}]}
        self.assertEqual(bundle.selected(manifest, 'one'), [{'target': 'one'}, {'kind': 'wasm'}])


    def test_failed_download_removes_partial_file(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            entry = {'path': 'plugins/test.wasm', 'url': 'https://example.invalid/test'}
            response = mock.MagicMock()
            response.__enter__.return_value = response
            response.read.side_effect = [b'partial bytes', OSError('interrupted')]
            with mock.patch.object(bundle, 'RESOURCES', root), mock.patch.object(
                    bundle.urllib.request, 'urlopen', return_value=response):
                with self.assertRaisesRegex(OSError, 'interrupted'):
                    bundle.fetch([entry])
            self.assertEqual(list((root / 'plugins').iterdir()), [])

    def test_application_package_has_fixed_metadata_and_portable_names(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            scripts = root / 'scripts'
            scripts.mkdir()
            for name in ('bundle_resources.py', 'package_app.py'):
                shutil.copy(Path(__file__).with_name(name), scripts / name)
            resources = root / 'resources'
            (resources / 'licenses').mkdir(parents=True)
            (resources / 'licenses' / 'plugin.txt').write_text('Plugin license')
            for name in ('README.md', 'THIRD_PARTY_NOTICES.md'):
                (resources / name).write_text('Notice')
            target = 'x86_64-unknown-linux-gnu'
            relative = f'bin/{target}/dprint'
            executable = resources / relative
            executable.parent.mkdir(parents=True)
            executable.write_bytes(b'formatter fixture')
            record = {'path': relative, 'bytes': executable.stat().st_size,
                      'sha256': hashlib.sha256(executable.read_bytes()).hexdigest()}
            (resources / 'manifest.json').write_text(json.dumps({'artifacts': [
                {'kind': 'executable-archive', 'target': target, 'executable': record}]}))
            binary = root / 'app'
            binary.write_bytes(b'application fixture')
            license_file = root / 'LICENSE'
            license_file.write_text('Application license')
            packages = []
            for index, timestamp in enumerate((1600000000, 1700000000)):
                os.utime(license_file, (timestamp, timestamp))
                output = root / f'package-{index}.zip'
                subprocess.run([sys.executable, str(scripts / 'package_app.py'),
                                '--target', target, '--binary', str(binary),
                                '--output', str(output)], check=True, capture_output=True)
                packages.append(output.read_bytes())
                with zipfile.ZipFile(output) as archive:
                    self.assertIn('resources/licenses/plugin.txt', archive.namelist())
                    self.assertTrue(all('\\' not in name for name in archive.namelist()))
                    for info in archive.infolist():
                        self.assertEqual(info.date_time, (2020, 1, 1, 0, 0, 0))
            self.assertEqual(*packages)


if __name__ == '__main__':
    unittest.main()
