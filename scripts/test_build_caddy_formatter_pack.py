"""Offline Caddy formatter-only pack integrity and source contract tests."""
import copy
import hashlib
import json
from pathlib import Path
import tarfile
import tempfile
import unittest

import build_caddy_formatter_pack as b


class CaddyTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.plan = copy.deepcopy(json.loads(b.SOURCES.read_text()))
        self.wasm = self.root / 'plugin.wasm'
        self.wasm.write_bytes(b'\0asm\x01\0\0\0')
        self.plan['wasm'].update(self.pin(self.wasm))
        for path, pin in self.plan['source_files'].items():
            file = self.root / path
            file.parent.mkdir(parents=True, exist_ok=True)
            file.write_text('fixture: ' + path)
            pin.update(self.pin(file))
        for pin in self.plan['pack_assets']:
            file = self.root / pin['source']
            file.parent.mkdir(parents=True, exist_ok=True)
            file.write_text('fixture: ' + pin['source'])
            pin.update(self.pin(file))
        self.sources = self.root / 'plan.json'
        self.save()

    @staticmethod
    def pin(path):
        data = path.read_bytes()
        return dict(bytes=len(data), sha256=hashlib.sha256(data).hexdigest())

    def save(self):
        self.sources.write_text(json.dumps(self.plan))

    def build(self, name='one.tar.gz'):
        return b.build(self.wasm, self.root / name, self.sources, self.root)

    def test_deterministic_data_only_candidate_and_explicit_host(self):
        result = self.build()
        self.build('two.tar.gz')
        self.assertEqual((self.root/'one.tar.gz').read_bytes(), (self.root/'two.tar.gz').read_bytes())
        self.assertIs(result['available'], False)
        self.assertEqual(result['kind'], 'wasm')
        self.assertEqual(result['requires_dprint_pack'], '0.63.0-p1')
        self.assertNotIn('executable', result)
        self.assertNotIn('url', result['archives'][0])
        with tarfile.open(self.root/'one.tar.gz') as archive:
            receipt = json.load(archive.extractfile('PROVENANCE.json'))
            self.assertIn('--skip-stable-format', receipt['invocation'])
            self.assertEqual(receipt['target'], 'universal-wasm')
            self.assertIn('sources/resources/caddy-formatter-sources.json', archive.getnames())
            for m in result['archives'][0]['members']:
                item = archive.getmember(m['path'])
                self.assertTrue(item.isfile())
                self.assertEqual(item.mode, 0o644)
                self.assertFalse(m['executable'])
                self.assertEqual(hashlib.sha256(archive.extractfile(item).read()).hexdigest(), m['sha256'])

    def test_corrupt_wasm_source_schema_and_license_fail(self):
        paths = [self.wasm, *[self.root/p for p in self.plan['source_files']],
                 *[self.root/p['source'] for p in self.plan['pack_assets']]]
        for path in paths:
            original = path.read_bytes()
            path.write_bytes(b'X'+original[1:])
            with self.assertRaisesRegex(ValueError, 'checksum'):
                self.build()
            self.assertFalse((self.root/'one.tar.gz').exists())
            path.write_bytes(original)

    def test_missing_wrong_size_and_symlink_fail(self):
        self.wasm.write_bytes(b'wrong')
        with self.assertRaisesRegex(ValueError, 'regular pinned input'):
            self.build()
        self.wasm.unlink()
        with self.assertRaisesRegex(ValueError, 'regular pinned input'):
            self.build()
        target = self.root/'real.wasm'
        target.write_bytes(b'\0asm\x01\0\0\0')
        try:
            self.wasm.symlink_to(target)
        except OSError:
            self.skipTest('symlinks unavailable')
        with self.assertRaisesRegex(ValueError, 'regular pinned input'):
            self.build()

    def test_non_wasm_and_unsafe_destination_rejected(self):
        self.wasm.write_bytes(b'not wasm')
        self.plan['wasm'].update(self.pin(self.wasm)); self.save()
        with self.assertRaisesRegex(ValueError, 'WebAssembly'):
            self.build()
        self.wasm.write_bytes(b'\0asm\x01\0\0\0')
        self.plan['wasm'].update(self.pin(self.wasm))
        self.plan['wasm']['path'] = '../outside'
        self.save()
        with self.assertRaisesRegex(ValueError, 'Unsafe member'):
            self.build()

    def test_catalog_is_disabled_universal_and_pinned(self):
        plan = json.loads(b.SOURCES.read_text())
        tool = json.loads((b.RESOURCES/'toolchains.json').read_text())['toolchains']['caddyfmt']
        self.assertEqual(tool['version'], plan['version'])
        self.assertEqual(len(tool['targets']), 6)
        for spec in tool['targets'].values():
            self.assertIs(spec['available'], False)
            self.assertEqual(spec['kind'], 'wasm')
            self.assertEqual(spec['requires_dprint_pack'], plan['requires_dprint_pack'])
            self.assertEqual(spec['wasm_plugins'], [plan['wasm']['path']])
            self.assertNotIn('executable', spec)
            self.assertNotIn('url', spec['archives'][0])
            self.assertTrue(all(not m['executable'] for m in spec['archives'][0]['members']))
            wasm = next(m for m in spec['archives'][0]['members'] if m['path'] == plan['wasm']['path'])
            self.assertEqual(wasm['sha256'], plan['wasm']['sha256'])

    def test_upstream_formatter_and_tests_are_unmodified(self):
        for name, blob in [('formatter.go','d25aad2d3d4f1d31c851e3043e9e5cebefc05194'),
                           ('formatter_test.go','7eef04766b4229fb434d689eb93186c842193080')]:
            data = (b.ROOT/'formatters/caddyfmt/internal/caddyfile'/name).read_bytes()
            actual = hashlib.sha1(b'blob '+str(len(data)).encode()+b'\0'+data).hexdigest()
            self.assertEqual(actual, blob)
        self.assertNotIn('require', (b.ROOT/'formatters/caddyfmt/go.mod').read_text())
        plan = json.loads(b.SOURCES.read_text())
        for path, pin in plan['source_files'].items():
            b.verify(b.ROOT/path, pin)


if __name__ == '__main__':
    unittest.main()
