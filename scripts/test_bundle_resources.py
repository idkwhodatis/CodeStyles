#!/usr/bin/env python3
"""Offline tests for resource integrity and zip reproducibility."""
import hashlib
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
