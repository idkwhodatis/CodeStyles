"""Offline fixture tests. No network, SDK execution or full SDK archive required."""
import copy
import hashlib
import io
import json
from pathlib import Path
import stat
import tarfile
import tempfile
import unittest
import zipfile

import build_native_formatter_pack as pack
from build_language_pack import write_bundle, digest_file


def fixture(root):
    data = b'fixed executable fixture\n'
    inputs = root / 'inputs'; (inputs / 'sdk/bin').mkdir(parents=True)
    source = inputs / 'sdk/bin/tool'; source.write_bytes(data)
    pin = {'origin': 'archive', 'input_path': 'sdk/bin/tool', 'source_archive_path': 'sdk/bin/tool',
           'path': 'bin/tool', 'archive_path': 'bin/tool', 'bytes': len(data),
           'sha256': hashlib.sha256(data).hexdigest(), 'executable': True}
    source_receipt = {'url': 'https://official.example.invalid/fixed', 'bytes': 1, 'sha256': 'a' * 64}
    plan = {'target': pack.TARGET, 'version': 'test', 'files': [pin], 'source_receipt': source_receipt,
            'archive_format': 'tar.gz', 'max_source_expanded_bytes': 1024 * 1024,
            'candidate': {'available': False, 'executable': 'bin/tool', 'environment': {},
                          'validation': {'license_audit': 'pending'}},
            'expected_bundle_sha256': '', 'expected_bundle_bytes': 0}
    stage = root / 'expected-stage'; (stage / 'bin').mkdir(parents=True)
    (stage / 'bin/tool').write_bytes(data)
    member = {k: pin[k] for k in ('path', 'archive_path', 'bytes', 'sha256', 'executable')}
    receipt = {'format_version': 1, 'toolchain': 'gofmt', 'version': 'test', 'target': pack.TARGET,
               'source': source_receipt, 'files': [member], 'license_audit': 'pending'}
    p = stage / 'PROVENANCE.json'; p.write_bytes((json.dumps(receipt, indent=2, sort_keys=True)+'\n').encode())
    members = [member, {'path': p.name, 'archive_path': p.name, 'bytes': p.stat().st_size,
                       'sha256': digest_file(p), 'executable': False}]
    bundle = root / 'expected.tar.gz'; write_bundle(stage, bundle, members)
    plan['candidate']['max_installed_bytes'] = sum(m['bytes'] for m in members)
    plan['expected_bundle_sha256'] = digest_file(bundle)
    plan['expected_bundle_bytes'] = bundle.stat().st_size
    return plan, inputs


class NativePackBuildTests(unittest.TestCase):
    def test_reproduces_exact_candidate_without_network_and_preserves_pending(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp); plan, inputs = fixture(root)
            output, candidate = root / 'actual.tar.gz', root / 'candidate.json'
            pack.build(plan, 'gofmt', inputs, output, candidate, root / 'work')
            self.assertEqual(digest_file(output), plan['expected_bundle_sha256'])
            spec = json.loads(candidate.read_text())['targets'][pack.TARGET]
            self.assertFalse(spec['available'])
            self.assertNotIn('url', spec['archives'][0])
            self.assertEqual(spec['validation']['license_audit'], 'pending')
            self.assertEqual(list((root / 'work').iterdir()), [])

    def test_mismatched_inputs_expected_bundle_and_existing_outputs_fail_closed(self):
        for mutation in ('input', 'hash', 'existing', 'same-path'):
            with self.subTest(mutation=mutation), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp); plan, inputs = fixture(root)
                output, candidate = root / 'actual.tar.gz', root / 'candidate.json'
                if mutation == 'input': (inputs / 'sdk/bin/tool').write_bytes(b'changed')
                if mutation == 'hash': plan['expected_bundle_sha256'] = 'f' * 64
                if mutation == 'existing': output.write_bytes(b'keep')
                if mutation == 'same-path': candidate = output
                with self.assertRaises(ValueError): pack.build(plan, 'gofmt', inputs, output, candidate, root / 'work')
                if mutation == 'existing': self.assertEqual(output.read_bytes(), b'keep')
                else: self.assertFalse(output.exists())

    def test_plan_paths_targets_and_symlink_inputs_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp); plan, inputs = fixture(root)
            for field, value in [('target', 'other'), ('path', '../outside'), ('published', True)]:
                changed = copy.deepcopy(plan)
                if field == 'path': changed['files'][0]['path'] = value
                elif field == 'published': changed['candidate']['available'] = value
                else: changed[field] = value
                with self.assertRaises(ValueError): pack.validate_plan(changed)
            link = inputs / 'sdk/bin/link'
            try: link.symlink_to(inputs / 'sdk/bin/tool')
            except OSError: return  # Windows runners may not grant symlink creation.
            with self.assertRaises(ValueError): pack.checked_input(inputs, 'sdk/bin/link')

    def test_tar_zip_exact_members_and_reject_links_duplicates_expansion(self):
        for kind in ('tar', 'zip'):
            for bad in ('none', 'link', 'duplicate', 'bound'):
                with self.subTest(kind=kind, bad=bad), tempfile.TemporaryDirectory() as tmp:
                    root = Path(tmp); plan, inputs = fixture(root)
                    data = (inputs / 'sdk/bin/tool').read_bytes(); archive = root / 'source.archive'
                    if kind == 'tar':
                        with tarfile.open(archive, 'w:gz') as tar:
                            for _ in range(2 if bad == 'duplicate' else 1):
                                entry = tarfile.TarInfo('sdk/bin/tool'); entry.size = len(data)
                                if bad == 'link': entry.type = tarfile.SYMTYPE; entry.linkname = '/outside'
                                tar.addfile(entry, io.BytesIO(data))
                    else:
                        plan['archive_format'] = 'zip'
                        with zipfile.ZipFile(archive, 'w') as z:
                            for _ in range(2 if bad == 'duplicate' else 1):
                                entry = zipfile.ZipInfo('sdk/bin/tool')
                                entry.external_attr = ((stat.S_IFLNK if bad == 'link' else stat.S_IFREG) | 0o755) << 16
                                z.writestr(entry, data)
                    plan['source_receipt'].update(bytes=archive.stat().st_size, sha256=digest_file(archive))
                    if bad == 'bound': plan['max_source_expanded_bytes'] = 1
                    destination = root / 'extracted'
                    if bad == 'none':
                        pack.extract(plan, archive, destination)
                        self.assertEqual((destination / 'sdk/bin/tool').read_bytes(), data)
                    else:
                        with self.assertRaises(ValueError): pack.extract(plan, archive, destination)
                        self.assertFalse(destination.exists())


if __name__ == '__main__':
    unittest.main()
