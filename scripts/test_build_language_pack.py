"""Offline deterministic packaging and strict extraction contract tests."""
import hashlib
import io
from pathlib import Path
import tarfile
import tempfile
import unittest

import build_language_pack as pack


class LanguagePackBuildTests(unittest.TestCase):
    def test_provenance_newlines_depend_on_target_not_host(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / 'PROVENANCE.json'
            for target in ('x86_64-pc-windows-msvc', 'aarch64-pc-windows-msvc',
                           'x86_64-apple-darwin', 'aarch64-unknown-linux-gnu'):
                pack.write_provenance(output, {'version': '1.99.1'}, target)
                expected = b'{\n  "version": "1.99.1"\n}\n'
                if 'windows' in target:
                    expected = expected.replace(b'\n', b'\r\n')
                self.assertEqual(output.read_bytes(), expected)


    def test_reproducible_single_gzip_and_metadata(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / 'bin').mkdir()
            (root / 'bin/rustfmt').write_bytes(b'fixture')
            members = [{'path': 'bin/rustfmt', 'executable': True}]
            first, second = root / 'one.tar.gz', root / 'two.tar.gz'
            pack.write_bundle(root, first, members)
            pack.write_bundle(root, second, members)
            self.assertEqual(first.read_bytes(), second.read_bytes())
            with tarfile.open(first) as tar:
                info = tar.getmember('bin/rustfmt')
                self.assertEqual(info.mode, 0o755)
                self.assertEqual(info.mtime, pack.EPOCH)
                self.assertEqual(tar.extractfile(info).read(), b'fixture')

    def test_paths_fail_closed(self):
        for name in ('/bin/rustfmt', '../rustfmt', 'bin/../rustfmt', 'C:/rustfmt', 'bin\\rustfmt', './rustfmt'):
            with self.subTest(name=name), self.assertRaises(ValueError):
                pack.safe_path(name)

    def test_extract_only_pinned_regular_member_and_hash(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            archive = root / 'test.tar.xz'
            with tarfile.open(archive, 'w:xz') as tar:
                info = tarfile.TarInfo('component/bin/rustfmt')
                info.size = 7
                tar.addfile(info, io.BytesIO(b'fixture'))
                ignored = tarfile.TarInfo('component/install.sh')
                ignored.size = 4
                tar.addfile(ignored, io.BytesIO(b'nope'))
            entry = {'max_expanded_bytes': 16384, 'members': [{'archive_path': 'component/bin/rustfmt',
                     'path': 'bin/rustfmt', 'bytes': 7, 'sha256': hashlib.sha256(b'fixture').hexdigest(), 'executable': True}]}
            stage = root / 'stage'
            stage.mkdir()
            pack.extract_component(archive, entry, stage)
            self.assertEqual((stage / 'bin/rustfmt').read_bytes(), b'fixture')
            self.assertFalse((stage / 'install.sh').exists())
            with self.assertRaises(ValueError):
                pack.extract_component(archive, entry, stage)
            entry['max_expanded_bytes'] = 1
            with self.assertRaisesRegex(ValueError, 'expansion'):
                pack.extract_component(archive, entry, root / 'unused')


if __name__ == '__main__':
    unittest.main()
