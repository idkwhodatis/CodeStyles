"""Orchestrator fixtures only; these do not establish native runtime acceptance."""
import json
import os
from pathlib import Path
import shutil
import tempfile
import unittest
from unittest.mock import patch

import verify_script_packs as v


class ScriptPackVerifierTests(unittest.TestCase):
    def test_exact_result_rejects_skipped_filtered_or_failed_tests(self):
        for result in ('', 'test result: ok. 0 passed; 0 failed; 0 ignored;',
                       'test result: ok. 0 passed; 0 failed; 1 ignored;',
                       'test result: FAILED. 0 passed; 1 failed; 0 ignored;'):
            with self.assertRaises(RuntimeError):
                v.require_pass(result)
        v.require_pass('test result: ok. 1 passed; 0 failed; 0 ignored;')

    def exercise(self, failure=None):
        with tempfile.TemporaryDirectory() as temp:
            base = Path(temp)
            root = base / 'repo'
            (root / 'resources').mkdir(parents=True)
            catalog = root / 'resources/toolchains.json'
            original = json.dumps({'toolchains': {t: {'version': v.VERSIONS[t], 'targets': {'native': {'available': False}}}
                                                  for t in v.PACKS}}).encode()
            catalog.write_bytes(original)
            commands = []
            def runner(argv, cwd, env, log):
                commands.append(argv)
                self.assertEqual(catalog.read_bytes(), original)
                self.assertEqual(env['CARGO_NET_OFFLINE'], 'true')
                self.assertEqual(env['RUSTUP_AUTO_INSTALL'], '0')
                if argv[:2] == ['git', 'rev-parse']:
                    return 'a' * 40
                if argv[0] == 'rustc':
                    return 'host: wrong' if failure == 'host' else 'host: native'
                if argv[:2] == ['git', 'clone']:
                    shutil.copytree(root, argv[-1])
                if argv[0] == 'cargo':
                    self.assertNotEqual(Path(cwd), root)
                    for flag in ('--offline', '--locked', '--ignored', '--exact'):
                        self.assertIn(flag, argv)
                    for tool in v.PACKS:
                        self.assertTrue(Path(env['CODESTYLES_' + tool.upper() + '_ARCHIVE']).is_file())
                    if failure == 'runtime':
                        raise RuntimeError('fixture runtime failure')
                    if failure == 'zero':
                        return 'test result: ok. 0 passed; 0 failed; 0 ignored;'
                    return 'test result: ok. 1 passed; 0 failed; 0 ignored;'
                return ''
            def build(*args):
                output = args[1] if len(args) == 2 else args[2]
                if failure == 'builder':
                    raise ValueError('fixture missing pinned asset')
                output.write_bytes(b'pinned fixture')
                return {'available': failure == 'enabled'}
            with patch.object(v.build_dprint_pack, 'build', side_effect=build), \
                 patch.object(v.build_caddy_formatter_pack, 'build', side_effect=build), \
                 patch.object(v.build_wasm_language_pack, 'build', side_effect=build):
                call = lambda: v.verify('native', base, base / 'caddy.wasm', base / 'work',
                                        base / 'report', root=root, runner=runner)
                if failure:
                    with self.assertRaises((RuntimeError, ValueError)):
                        call()
                else:
                    report = call()
                    self.assertEqual(len(report['tests']), 5 if os.name == 'posix' else 4)
                    self.assertFalse(report['published'])
                    self.assertFalse(report['release_clearance'])
            self.assertEqual(catalog.read_bytes(), original)
            report = json.loads((base / 'report/report.json').read_text())
            self.assertEqual(report['native_execution'], 'failed' if failure else 'passed')
            self.assertTrue(report['canonical_catalog_unchanged'])
            self.assertEqual(list((base / 'work').iterdir()), [])
            if failure in ('host', 'builder', 'enabled'):
                self.assertFalse(any(c[0] == 'cargo' for c in commands))

    def test_success_and_failure_cleanup(self):
        for failure in (None, 'host', 'builder', 'enabled', 'runtime', 'zero'):
            with self.subTest(failure=failure):
                self.exercise(failure)

    def test_refuses_outputs_inside_checkout(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            with self.assertRaisesRegex(ValueError, 'outside'):
                v.verify('native', root, root / 'a', root / 'work', root / 'report', root=root)

    def test_refuses_resource_symlinks(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp) / 'repo'
            (root / 'resources').mkdir(parents=True)
            try:
                (root / 'resources/link').symlink_to(root)
            except OSError:
                self.skipTest('symlinks unavailable')
            with self.assertRaisesRegex(ValueError, 'nonregular'):
                v.verify('native', root, root / 'a', root.parent / 'work',
                         root.parent / 'report', root=root)
