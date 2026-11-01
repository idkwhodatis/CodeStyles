"""Offline packaging contract tests; fixture binaries are never executed."""
import copy
import hashlib
import json
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest.mock import patch

import build_extended_formatter_pack as pack
from build_language_pack import digest_file


def fixture(root, tool='google-java-format', target=pack.TARGETS[0]):
    inputs = root / 'inputs'; inputs.mkdir()
    executable = ('dotnet/dotnet' if tool == 'csharpier' else f'bin/{tool}') + ('.exe' if 'windows' in target else '')
    files = []
    for name, path, data, mode in [('native', executable, b'native-fixture', True),
                                    ('notice', 'licenses/AUDIT.txt', b'Pending transitive audit\n', False)]:
        (inputs / name).write_bytes(data)
        files.append({'input_path': name, 'path': path, 'sha256': hashlib.sha256(data).hexdigest(),
                      'bytes': len(data), 'executable': mode})
    if tool == 'csharpier':
        for name in ('CSharpier.dll', 'CSharpier.deps.json', 'CSharpier.runtimeconfig.json'):
            (inputs/name).write_bytes(b'fixture')
            files.append({'input_path':name,'path':'lib/csharpier/'+name,'bytes':7,
                          'sha256':hashlib.sha256(b'fixture').hexdigest(),'executable':False})
    if tool == 'csharpier':
        hostfxr = 'hostfxr.dll' if 'windows' in target else ('libhostfxr.dylib' if 'apple' in target else 'libhostfxr.so')
        for path in ('dotnet/shared/Microsoft.NETCore.App/10.0.14/System.Private.CoreLib.dll', 'dotnet/host/fxr/10.0.14/'+hostfxr):
            name = Path(path).name; (inputs/name).write_bytes(b'fixture')
            files.append({'input_path':name,'path':path,'bytes':7,
                          'sha256':hashlib.sha256(b'fixture').hexdigest(),'executable':False})
    return inputs, {'available': False, 'executable': executable, 'environment': {},
                    'license_audit': 'pending', 'files': files}


class ExtendedPackTests(unittest.TestCase):
    def test_all_tools_and_six_targets_build_unpublished_deterministically(self):
        for tool in pack.TOOLS:
            for target in pack.TARGETS:
                with self.subTest(tool=tool, target=target), tempfile.TemporaryDirectory() as tmp:
                    root = Path(tmp); inputs, plan = fixture(root, tool, target)
                    for suffix in ('one', 'two'):
                        pack.build(tool, target, 'test', plan, inputs, root/f'{suffix}.tgz', root/f'{suffix}.json', root/'work')
                    self.assertEqual(digest_file(root/'one.tgz'), digest_file(root/'two.tgz'))
                    spec = json.loads((root/'one.json').read_text())['targets'][target]
                    self.assertFalse(spec['available']); self.assertEqual(spec['validation']['native_execution'], 'not-run')
                    self.assertNotIn('url', spec['archives'][0])
                    with tarfile.open(root/'one.tgz') as tar:
                        self.assertEqual(set(tar.getnames()), {p['path'] for p in plan['files']} | {'PROVENANCE.json'})
                    self.assertEqual(list((root/'work').iterdir()), [])

    def test_invalid_plan_and_member_mutations_fail_closed(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp); inputs, original = fixture(root)
            for mutation in ('published', 'environment', 'shim', 'no-notices', 'traversal', 'duplicate', 'casefold', 'prefix', 'dot', 'device', 'reserved', 'bundle-hash', 'hash', 'bytes'):
                with self.subTest(mutation=mutation):
                    plan = copy.deepcopy(original)
                    if mutation == 'published': plan['available'] = True
                    if mutation == 'environment': plan['environment'] = {'PATH': 'bad'}
                    if mutation == 'shim': plan['executable'] = 'bin/dotnet'
                    if mutation == 'no-notices': plan['files'].pop()
                    if mutation == 'traversal': plan['files'][0]['input_path'] = '../bad'
                    if mutation == 'duplicate': plan['files'].append(plan['files'][0].copy())
                    if mutation == 'casefold':
                        other = plan['files'][0].copy(); other['path'] = other['path'].upper(); plan['files'].append(other)
                    if mutation == 'prefix':
                        other = plan['files'][0].copy(); other['path'] = 'bin'; plan['files'].append(other)
                    if mutation == 'dot': plan['files'][1]['path'] = 'licenses/notice.'
                    if mutation == 'device': plan['files'][1]['path'] = 'licenses/NUL.txt'
                    if mutation == 'reserved': plan['files'][1]['path'] = 'provenance.JSON'
                    if mutation == 'bundle-hash':
                        plan['expected_bundle_sha256'] = 'f'*64; plan['expected_bundle_bytes'] = 1
                    if mutation == 'hash': plan['files'][0]['sha256'] = 'f'*64
                    if mutation == 'bytes': plan['files'][0]['bytes'] = 999
                    with self.assertRaises(ValueError):
                        pack.build('google-java-format', pack.TARGETS[0], 'test', plan, inputs, root/'out', root/'receipt', root/'work')
                    self.assertFalse((root/'out').exists())

    def test_does_not_replace_outputs_or_follow_input_symlinks(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp); inputs, plan=fixture(root); out=root/'out'; out.write_text('keep')
            with self.assertRaises(ValueError):
                pack.build('google-java-format', pack.TARGETS[0], 'test', plan, inputs, out, root/'receipt', root/'work')
            self.assertEqual(out.read_text(),'keep')
            source=inputs/'native'; source.unlink()
            try: source.symlink_to(inputs/'notice')
            except OSError: return
            with self.assertRaises(ValueError):
                pack.build('google-java-format', pack.TARGETS[0], 'test', plan, inputs, root/'other', root/'receipt', root/'work')

    def test_late_output_creation_is_not_replaced_or_deleted(self):
        for fail_writer in (False, True):
            with self.subTest(fail_writer=fail_writer), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp); inputs, plan = fixture(root); out = root/'out'
                original = pack.write_bundle
                def racing_writer(stage, private_output, members):
                    self.assertNotEqual(private_output, out)
                    out.write_text('concurrent caller data')
                    if fail_writer:
                        raise OSError('failure before this invocation produced a bundle')
                    return original(stage, private_output, members)
                with patch.object(pack, 'write_bundle', racing_writer):
                    with self.assertRaises(OSError):
                        pack.build('google-java-format', pack.TARGETS[0], 'test', plan, inputs,
                                   out, root/'receipt', root/'work')
                self.assertEqual(out.read_text(), 'concurrent caller data')
                self.assertFalse((root/'receipt').exists())
                self.assertEqual(list(root.glob('.extended-*')), [])
                self.assertEqual(list((root/'work').iterdir()), [])

    def test_late_predictable_part_file_is_not_opened_or_removed(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp); inputs, plan = fixture(root); out = root/'out'
            part = root/'out.part'; foreign = root/'foreign'; foreign.write_text('keep')
            original = pack.write_bundle
            def racing_writer(stage, private_output, members):
                try:
                    part.symlink_to(foreign)
                except OSError:
                    part.write_text('foreign part file')
                return original(stage, private_output, members)
            with patch.object(pack, 'write_bundle', racing_writer):
                pack.build('google-java-format', pack.TARGETS[0], 'test', plan, inputs,
                           out, root/'receipt', root/'work')
            self.assertEqual(foreign.read_text(), 'keep')
            self.assertTrue(part.exists())
            if not part.is_symlink():
                self.assertEqual(part.read_text(), 'foreign part file')
            self.assertEqual(list(root.glob('.extended-*')), [])

    def test_receipt_race_preserves_valid_bundle_and_foreign_receipt(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp); inputs, plan = fixture(root)
            out = root/'out'; receipt = root/'receipt'; original = pack.os.link
            def racing_link(source, destination, **kwargs):
                if destination == receipt:
                    receipt.write_text('concurrent receipt')
                return original(source, destination, **kwargs)
            with patch.object(pack.os, 'link', racing_link):
                with self.assertRaisesRegex(RuntimeError, 'Bundle created.*receipt not published'):
                    pack.build('google-java-format', pack.TARGETS[0], 'test', plan, inputs,
                               out, receipt, root/'work')
            self.assertEqual(receipt.read_text(), 'concurrent receipt')
            with tarfile.open(out) as tar:
                self.assertIn('PROVENANCE.json', tar.getnames())
            self.assertEqual(list(root.glob('.extended-*')), [])

    def test_unsupported_no_replace_publication_fails_without_fallback(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp); inputs, plan = fixture(root)
            with patch.object(pack.os, 'link', side_effect=OSError('hard links unsupported')):
                with self.assertRaisesRegex(OSError, 'hard links unsupported'):
                    pack.build('google-java-format', pack.TARGETS[0], 'test', plan, inputs,
                               root/'out', root/'receipt', root/'work')
            self.assertFalse((root/'out').exists())
            self.assertFalse((root/'receipt').exists())
            self.assertEqual(list(root.glob('.extended-*')), [])

    def test_shipped_catalog_is_unavailable_with_no_download_urls(self):
        catalog=json.loads((pack.ROOT/'resources/toolchains.json').read_text())['toolchains']
        sources=json.loads(pack.SOURCES.read_text())['tools']
        for tool in pack.TOOLS:
            self.assertEqual(catalog[tool]['version'],sources[tool]['version'])
            self.assertEqual(set(catalog[tool]['targets']),set(pack.TARGETS))
            for target, plan in sources[tool]['plans'].items():
                pack.validate_plan(tool, target, plan)
            for spec in catalog[tool]['targets'].values():
                self.assertIs(spec['available'],False)
                self.assertEqual(spec['archives'],[])


if __name__ == '__main__': unittest.main()
