"""Offline compiler source trust-boundary tests; no compiler execution."""
import hashlib
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import compile_caddy_formatter_wasm as c


class SourceTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        override = patch.object(c, 'ROOT', self.root)
        override.start()
        self.addCleanup(override.stop)
        self.plan = {'source_files': {}}

    def add(self, name, data=b'package main\n'):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        self.plan['source_files'][name] = {
            'bytes': len(data), 'sha256': hashlib.sha256(data).hexdigest()}
        return path

    def test_only_pinned_formatter_files_are_staged(self):
        self.add('formatters/caddyfmt/go.mod', b'module test\n')
        self.add('formatters/caddyfmt/internal/caddyfile/formatter.go')
        self.add('scripts/compile_caddy_formatter_wasm.py', b'not Go source')
        source = self.root / 'formatters/caddyfmt'
        (source / 'unlisted_test.go').write_text('package main // TestMain or init payload')
        (source / 'unlisted.go').write_text('package main // init payload')
        output = self.root / 'staged'
        c.stage_formatter_sources(c.verified_sources(self.plan), output)
        self.assertEqual(sorted(p.relative_to(output).as_posix()
                                for p in output.rglob('*') if p.is_file()),
                         ['go.mod', 'internal/caddyfile/formatter.go'])

    def test_staging_uses_verified_snapshot_not_reread_checkout(self):
        original = b'package main\n'
        path = self.add('formatters/caddyfmt/main.go', original)
        inputs = c.verified_sources(self.plan)
        path.write_bytes(b'changed after verification')
        output = self.root / 'staged'
        c.stage_formatter_sources(inputs, output)
        self.assertEqual((output / 'main.go').read_bytes(), original)

    def test_unsafe_manifest_paths_fail(self):
        pin = {'bytes': 0, 'sha256': hashlib.sha256(b'').hexdigest()}
        for name in ['', '../outside', '/absolute', 'a/../b', 'a//b',
                     './relative', 'a\\b', 'C:/outside']:
            with self.subTest(name=name), self.assertRaisesRegex(ValueError, 'Unsafe source'):
                c.verified_sources({'source_files': {name: pin}})

    def test_symlink_file_fails_even_with_matching_bytes(self):
        path = self.add('formatters/caddyfmt/main.go')
        target = self.root / 'target.go'
        path.rename(target)
        path.symlink_to(target)
        with self.assertRaisesRegex(ValueError, 'Symlink source'):
            c.verified_sources(self.plan)

    def test_symlink_parent_fails_even_with_matching_bytes(self):
        self.add('formatters/caddyfmt/main.go')
        source = self.root / 'formatters/caddyfmt'
        target = self.root / 'real-source'
        source.rename(target)
        source.symlink_to(target, target_is_directory=True)
        with self.assertRaisesRegex(ValueError, 'Symlink source'):
            c.verified_sources(self.plan)

    def test_modified_or_missing_pinned_input_fails(self):
        path = self.add('formatters/caddyfmt/main.go')
        path.write_bytes(b'Package main\n')
        with self.assertRaisesRegex(ValueError, 'checksum'):
            c.verified_sources(self.plan)
        path.unlink()
        with self.assertRaisesRegex(ValueError, 'regular pinned input'):
            c.verified_sources(self.plan)


if __name__ == '__main__':
    unittest.main()
