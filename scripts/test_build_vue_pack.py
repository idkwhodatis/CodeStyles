"""Offline deterministic Vue pack tests, with tiny local fixture pins only."""
import hashlib
import json
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest.mock import patch

import build_vue_pack as pack


class VuePackTests(unittest.TestCase):
    def fixture(self, root):
        source = root / 'inputs'; source.mkdir()
        records = []
        for name, destination, _, _, url in pack.ASSETS:
            data = ('fixture ' + name).encode()
            (source / name).write_bytes(data)
            records.append((name, destination, len(data), hashlib.sha256(data).hexdigest(), url))
        return source, records

    def test_deterministic_universal_data_only_bundle_and_receipt(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp); source, records = self.fixture(root)
            with patch.object(pack, 'ASSETS', records):
                first = pack.build(source, root / 'first.tar.gz')
                second = pack.build(source, root / 'second.tar.gz')
            self.assertEqual((root / 'first.tar.gz').read_bytes(), (root / 'second.tar.gz').read_bytes())
            self.assertEqual(first['archives'][0]['sha256'], second['archives'][0]['sha256'])
            self.assertIs(first['available'], False)
            self.assertEqual(first['kind'], 'wasm')
            self.assertEqual(first['wasm_plugins'], pack.WASM_PLUGINS)
            self.assertEqual(first['environment'], {})
            self.assertNotIn('executable', first)
            self.assertNotIn('url', first['archives'][0])
            with tarfile.open(root / 'first.tar.gz') as archive:
                entries = archive.getmembers()
                self.assertTrue(all(entry.isfile() and entry.mode == 0o644 for entry in entries))
                for entry in entries:
                    data = archive.extractfile(entry).read()
                    member = next(m for m in first['archives'][0]['members'] if m['path'] == entry.name)
                    self.assertFalse(member['executable'])
                    self.assertEqual(hashlib.sha256(data).hexdigest(), member['sha256'])
                receipt = json.load(archive.extractfile('PROVENANCE.json'))
                self.assertEqual(receipt['target'], 'universal-wasm')
                self.assertEqual(receipt['licenses'], {'markup_fmt': 'MIT', 'malva': 'MIT'})
            self.assertFalse(any(p.name.startswith('vue-pack-stage-') for p in root.iterdir()))

    def test_changed_input_rejected_and_stage_cleaned(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp); source, records = self.fixture(root)
            name = records[0][0]
            data = (source / name).read_bytes()
            (source / name).write_bytes(b'X' + data[1:])
            with patch.object(pack, 'ASSETS', records), self.assertRaisesRegex(ValueError, 'checksum'):
                pack.build(source, root / 'bundle.tar.gz')
            self.assertFalse((root / 'bundle.tar.gz').exists())
            self.assertFalse(any(p.name.startswith('vue-pack-stage-') for p in root.iterdir()))


if __name__ == '__main__':
    unittest.main()
