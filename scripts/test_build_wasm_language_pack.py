"""Offline fixed-input packing tests; no network or WASM execution."""
import copy
import hashlib
import json
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest.mock import patch

import build_wasm_language_pack as builder


class WasmLanguagePackTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.source = self.root / 'source'
        self.resources = self.root / 'resources'
        self.packs = copy.deepcopy(builder.PACKS)
        for pack in self.packs.values():
            assets = []
            for root, name, destination, _, _, url in pack['assets']:
                data = ('fixture ' + name).encode()
                path = (self.source if root == 'source' else self.resources) / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(data)
                assets.append((root, name, destination, len(data), hashlib.sha256(data).hexdigest(), url))
            pack['assets'] = assets
        patches = patch.object(builder, 'PACKS', self.packs)
        patches.start()
        self.addCleanup(patches.stop)

    def build(self, name, filename='first.tar.gz'):
        return builder.build(name, self.source, self.root / filename, self.resources)

    def test_deterministic_data_only_archives_and_explicit_host_contract(self):
        for name in self.packs:
            with self.subTest(name=name):
                first = self.build(name)
                second = self.build(name, 'second.tar.gz')
                self.assertEqual((self.root / 'first.tar.gz').read_bytes(),
                                 (self.root / 'second.tar.gz').read_bytes())
                self.assertEqual(first['archives'][0]['sha256'], second['archives'][0]['sha256'])
                self.assertIs(first['available'], False)
                self.assertEqual(first['requires_dprint_pack'], '0.63.0-p1')
                self.assertEqual(first['wasm_plugins'], self.packs[name]['wasm_plugins'])
                self.assertEqual(first['kind'], 'wasm')
                self.assertNotIn('executable', first)
                self.assertNotIn('url', first['archives'][0])
                with tarfile.open(self.root / 'first.tar.gz') as archive:
                    self.assertEqual(set(archive.getnames()),
                                     {m['path'] for m in first['archives'][0]['members']})
                    for entry in archive.getmembers():
                        self.assertTrue(entry.isfile())
                        self.assertEqual(entry.mode, 0o644)
                        data = archive.extractfile(entry).read()
                        member = next(m for m in first['archives'][0]['members'] if m['path'] == entry.name)
                        self.assertEqual(hashlib.sha256(data).hexdigest(), member['sha256'])
                        self.assertFalse(member['executable'])
                    provenance = json.load(archive.extractfile('PROVENANCE.json'))
                    self.assertEqual(provenance['requires_dprint_pack'], '0.63.0-p1')
                    self.assertEqual(provenance['target'], 'universal-wasm')
                    self.assertEqual(provenance['toolchain'], name)
                    self.assertEqual(len(provenance['sources']), 3)

    def test_corrupt_wasm_license_and_schema_fail_without_partial_output(self):
        for name, pack in self.packs.items():
            for root, path, *_ in pack['assets']:
                with self.subTest(name=name, path=path):
                    file = (self.source if root == 'source' else self.resources) / path
                    original = file.read_bytes()
                    file.write_bytes(b'X' + original[1:])
                    with self.assertRaisesRegex(ValueError, 'checksum'):
                        self.build(name)
                    file.write_bytes(original)
                    self.assertFalse((self.root / 'first.tar.gz').exists())
                    self.assertFalse(any(p.name.startswith('wasm-language-pack-') for p in self.root.iterdir()))

    def test_missing_and_wrong_size_inputs_rejected(self):
        file = self.source / 'sql-0.4.1.wasm'
        file.write_bytes(b'wrong-size')
        with self.assertRaisesRegex(ValueError, 'regular pinned input'):
            self.build('sql')
        file.unlink()
        with self.assertRaisesRegex(ValueError, 'regular pinned input'):
            self.build('sql')

    def test_symlink_input_rejected(self):
        file = self.source / 'sql-0.4.1.wasm'
        saved = self.source / 'saved.wasm'
        file.rename(saved)
        try:
            file.symlink_to(saved)
        except OSError:
            self.skipTest('symlinks unavailable')
        with self.assertRaisesRegex(ValueError, 'regular pinned input'):
            self.build('sql')

    def test_committed_catalog_keeps_all_new_targets_disabled_and_pinned(self):
        catalog = json.loads((builder.RESOURCES / 'toolchains.json').read_text())['toolchains']
        for name in self.packs:
            entry = catalog[name]
            self.assertEqual(entry['version'], self.packs[name]['version'])
            self.assertEqual(set(entry['targets']), set(catalog['vue']['targets']))
            self.assertEqual(len(entry['targets']), 6)
            for spec in entry['targets'].values():
                self.assertIs(spec['available'], False)
                self.assertEqual(spec['requires_dprint_pack'], catalog['dprint']['version'])
                self.assertEqual(spec['wasm_plugins'], self.packs[name]['wasm_plugins'])
                self.assertEqual(spec['kind'], 'wasm')
                self.assertNotIn('url', spec['archives'][0])
                self.assertTrue(all(not m['executable'] for m in spec['archives'][0]['members']))

    def test_unknown_pack_is_rejected(self):
        with self.assertRaisesRegex(ValueError, 'Only reviewed'):
            self.build('vue')


if __name__ == '__main__':
    unittest.main()
