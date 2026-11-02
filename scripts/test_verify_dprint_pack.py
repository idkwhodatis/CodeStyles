import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock

import verify_dprint_pack as verifier


class VerifyDprintPackTests(unittest.TestCase):
    def test_native_candidate_is_disabled_and_catalog_restored_on_success_and_failure(self):
        for fails in (False, True):
            with self.subTest(fails=fails), tempfile.TemporaryDirectory() as temp:
                root = Path(temp)
                (root / 'resources').mkdir()
                catalog = root / 'resources/toolchains.json'
                original = b'{"toolchains":{"dprint":{"version":"0.63.0-p1","targets":{}}}}\n'
                catalog.write_bytes(original)
                candidate = {'available': False, 'kind': 'dprint'}
                def run(command, **kwargs):
                    current = json.loads(catalog.read_bytes())
                    self.assertEqual(current['toolchains']['dprint']['targets']['test'], candidate)
                    self.assertIn('--include-ignored', command)
                    self.assertTrue(kwargs['env']['CODESTYLES_DPRINT_ARCHIVE'].endswith('.tar.gz'))
                    if fails:
                        raise subprocess.CalledProcessError(1, command)
                with mock.patch.object(verifier.subprocess, 'check_output', return_value='host: test\n'), \
                        mock.patch.object(verifier.build_dprint_pack, 'build', return_value=candidate):
                    if fails:
                        with self.assertRaises(subprocess.CalledProcessError):
                            verifier.verify('test', root / 'work', root, run)
                    else:
                        verifier.verify('test', root / 'work', root, run)
                self.assertEqual(catalog.read_bytes(), original)

    def test_cross_compile_is_not_native_acceptance(self):
        with mock.patch.object(verifier.subprocess, 'check_output', return_value='host: other\n'), \
                mock.patch.object(verifier.build_dprint_pack, 'build') as build:
            with self.assertRaisesRegex(ValueError, 'native Rust host'):
                verifier.verify('test', Path('not-created'))
            build.assert_not_called()
