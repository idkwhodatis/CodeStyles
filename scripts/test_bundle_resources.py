#!/usr/bin/env python3
"""Offline tests for resource integrity and zip reproducibility."""
import hashlib
import json
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


if __name__ == '__main__':
    unittest.main()
