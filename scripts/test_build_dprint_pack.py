import hashlib
import json
from pathlib import Path
import tarfile
import tempfile
import unittest

import build_dprint_pack as builder


class DprintPackTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.resources = self.root / 'resources'
        self.target = 'x86_64-unknown-linux-gnu'
        self.manifest = dict(dprint_version='0.63.0', plugins={
            'typescript': '0.96.1', 'json': '0.25.2', 'markdown': '0.26.0',
            'toml': '0.9.0', 'ruff': '0.11.3'}, artifacts=[], files=[])
        engine = self.file('bin/' + self.target + '/dprint', b'engine')
        self.manifest['artifacts'].append(dict(kind='executable-archive', target=self.target, executable=engine))
        for name, version in self.manifest['plugins'].items():
            record = self.file(f'plugins/{name}-{version}.wasm', b'\0asm' + name.encode())
            self.manifest['artifacts'].append(dict(kind='wasm', **record))
        self.file('licenses/dprint.txt', b'MIT license')
        for name in self.manifest['plugins']:
            self.file(f'licenses/dprint-plugin-{name}.txt', b'MIT license')
        for name in builder.bundle_resources.dependency_notices('dprint', self.target):
            self.file(name, b'pinned dependency notice or source fixture')
        self.save()

    def file(self, path, data):
        file = self.resources / path
        file.parent.mkdir(parents=True, exist_ok=True)
        file.write_bytes(data)
        record = dict(path=path, bytes=len(data), sha256=hashlib.sha256(data).hexdigest())
        self.manifest['files'].append(record)
        return record

    def save(self):
        (self.resources / 'manifest.json').write_text(json.dumps(self.manifest))

    def build(self):
        return builder.build(self.resources, self.target, self.root / 'runtime.tar.gz')

    def test_atomic_engine_wasm_pack_is_reproducible_and_disabled(self):
        spec = self.build()
        first = (self.root / 'runtime.tar.gz').read_bytes()
        self.assertEqual(spec, self.build())
        self.assertEqual(first, (self.root / 'runtime.tar.gz').read_bytes())
        self.assertFalse(spec['available'])
        self.assertEqual(spec['kind'], 'dprint')
        self.assertEqual(spec['executable'], 'bin/dprint')
        self.assertEqual(len(spec['wasm_plugins']), 5)
        self.assertNotIn('url', spec['archives'][0])
        with tarfile.open(self.root / 'runtime.tar.gz') as archive:
            self.assertEqual(set(archive.getnames()), {m['path'] for m in spec['archives'][0]['members']})
            self.assertEqual(archive.getmember('bin/dprint').mode, 0o755)
            self.assertTrue(all(archive.getmember(p).mode == 0o644 for p in spec['wasm_plugins']))

    def test_corrupt_missing_and_version_drift_fail_closed(self):
        plugin = self.resources / 'plugins/json-0.25.2.wasm'
        plugin.write_bytes(b'corrupt')
        with self.assertRaisesRegex(ValueError, 'mismatch'):
            self.build()
        plugin.unlink()
        with self.assertRaisesRegex(ValueError, 'Missing'):
            self.build()
        self.manifest['plugins']['json'] = 'other'
        self.save()
        with self.assertRaisesRegex(ValueError, 'pack revision'):
            self.build()

    def test_unpinned_license_is_rejected(self):
        self.manifest['files'] = [a for a in self.manifest['files'] if a['path'] != 'licenses/dprint.txt']
        self.save()
        with self.assertRaisesRegex(ValueError, 'License missing'):
            self.build()

    def test_target_specific_transitive_notices_are_packaged(self):
        for arch in ('x86_64', 'aarch64'):
            for os in ('unknown-linux-gnu', 'apple-darwin', 'pc-windows-msvc'):
                self.target = f'{arch}-{os}'
                self.manifest['artifacts'][0]['target'] = self.target
                self.save()
                with self.subTest(target=self.target):
                    spec = self.build()
                    names = {m['path'] for m in spec['archives'][0]['members']}
                    required = builder.bundle_resources.dependency_notices('dprint', self.target)
                    self.assertTrue(set(required) <= names)
                    for name in builder.bundle_resources.OPTION_EXT_NOTICES:
                        self.assertEqual(name in names, 'linux' in os)
                    self.assertFalse(set(builder.bundle_resources.APP_DEPENDENCY_NOTICES) & names)

    def test_missing_unpinned_and_corrupt_dependency_notices_fail_closed(self):
        for name in builder.bundle_resources.dependency_notices('dprint', self.target):
            with self.subTest(name=name):
                original = list(self.manifest['files'])
                self.manifest['files'] = [a for a in original if a['path'] != name]
                self.save()
                with self.assertRaisesRegex(ValueError, 'License missing'):
                    self.build()
                self.manifest['files'] = original
                self.save()
                path = self.resources / name
                data = path.read_bytes()
                path.unlink()
                with self.assertRaisesRegex(ValueError, 'Missing resource'):
                    self.build()
                path.write_bytes(b'corrupt')
                with self.assertRaisesRegex(ValueError, 'mismatch'):
                    self.build()
                path.write_bytes(data)


if __name__ == '__main__':
    unittest.main()
