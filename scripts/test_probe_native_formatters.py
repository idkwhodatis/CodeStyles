import io
import json
from pathlib import Path
import tempfile
import sys
import os
import unittest
import zipfile
from unittest.mock import patch

import probe_native_formatters as probe


class ProbeTests(unittest.TestCase):
    def test_all_twelve_official_pins_are_fixed(self):
        manifest = json.loads(probe.SOURCES.read_text())
        for target in probe.TARGETS:
            for tool in ('gofmt', 'dartfmt'):
                version, pin = probe.source_pin(manifest, tool, target)
                self.assertIn(version, pin['url'])
                self.assertEqual(len(pin['sha256']), 64)
                self.assertLessEqual(len(probe.members(tool, target)), 7)
        pin = next(p for p in manifest['metadata_only_other_targets']['go']['targets'] if p['os'] == 'darwin' and p['arch'] == 'amd64')
        pin['url'] = 'https://untrusted.invalid/sdk'
        with self.assertRaises(ValueError):
            probe.source_pin(manifest, 'gofmt', 'x86_64-apple-darwin')

    def test_native_host_gate(self):
        self.assertTrue(probe.host_matches('aarch64-apple-darwin', 'Darwin', 'arm64'))
        self.assertTrue(probe.host_matches('x86_64-pc-windows-msvc', 'Windows', 'AMD64'))
        self.assertFalse(probe.host_matches('aarch64-pc-windows-msvc', 'Windows', 'AMD64'))
        self.assertFalse(probe.host_matches('x86_64-unknown-linux-gnu', 'Darwin', 'x86_64'))

    def test_strict_members_and_receipt(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); archive = root / 'source.zip'
            with zipfile.ZipFile(archive, 'w') as bundle:
                bundle.writestr('sdk/bin/tool', b'fixed')
                bundle.writestr('sdk/unused', b'not extracted')
            receipts = probe.extract(archive, {'sdk/bin/tool': 'bin/tool'}, root / 'runtime')
            self.assertEqual(receipts[0]['bytes'], 5)
            self.assertEqual((root / 'runtime/bin/tool').read_bytes(), b'fixed')
            self.assertFalse((root / 'runtime/sdk/unused').exists())
            with self.assertRaises(ValueError):
                probe.extract(archive, {'sdk/missing.dll': 'bin/missing.dll'}, root / 'missing')

    def test_traversal_and_selected_symlink_fail_closed(self):
        for name, symlink in [('../outside', False), ('sdk/tool', True)]:
            with tempfile.TemporaryDirectory() as directory:
                root = Path(directory); archive = root / 'source.zip'
                with zipfile.ZipFile(archive, 'w') as bundle:
                    entry = zipfile.ZipInfo(name)
                    if symlink:
                        entry.create_system = 3; entry.external_attr = 0o120777 << 16
                    bundle.writestr(entry, b'target')
                with self.assertRaises(ValueError):
                    probe.extract(archive, {'sdk/tool': 'bin/tool'}, root / 'runtime')
                self.assertFalse((root / 'outside').exists())

    def test_download_never_accepts_wrong_hash(self):
        with tempfile.TemporaryDirectory() as directory:
            response = io.BytesIO(b'bytes')
            with patch.object(probe.urllib.request, 'build_opener') as opener:
                opener.return_value.open.return_value = response
                with self.assertRaises(ValueError):
                    probe.download({'url': 'https://go.dev/fixed', 'bytes': 5, 'sha256': '0'*64}, Path(directory)/'sdk')

    def test_no_cross_host_execution_or_download(self):
        with patch.object(probe, 'host_matches', return_value=False), patch.object(probe, 'download') as download:
            with self.assertRaises(ValueError):
                probe.probe('gofmt', 'aarch64-pc-windows-msvc', Path('unused'), Path('unused'))
            download.assert_not_called()

    def test_redirects_reject_untrusted_or_insecure_hosts(self):
        handler = probe.OfficialRedirects()
        for url in ['http://go.dev/sdk', 'https://evil.invalid/sdk', 'https://user:pass@go.dev/sdk']:
            with self.assertRaises(ValueError):
                handler.redirect_request(None, None, 302, 'redirect', {}, url)

    def test_download_failure_preserves_nonrelease_report(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with patch.object(probe, 'host_matches', return_value=True), patch.object(probe, 'download', side_effect=ValueError('fixture digest failure')):
                with self.assertRaisesRegex(ValueError, 'fixture digest failure'):
                    probe.probe('gofmt', 'x86_64-unknown-linux-gnu', root/'work', root/'reports')
            report = json.loads((root/'reports/gofmt-probe.json').read_text())
            self.assertEqual(report['smoke'], 'not-run')
            self.assertFalse(report['available'])
            self.assertFalse(report['release_approved'])
            self.assertEqual(report['manager_e2e'], 'not-run')
            self.assertIn('fixture digest failure', report['error'])
            self.assertEqual(list((root/'work').iterdir()), [])

    def test_smoke_output_is_bounded_and_failure_is_recorded(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            env = {k: v for k, v in os.environ.items() if k in ('SystemRoot', 'WINDIR')}
            with patch.object(probe, 'MAX_FORMAT_OUTPUT', 64):
                with self.assertRaisesRegex((RuntimeError, ValueError), 'overflow'):
                    probe.formatter_run([sys.executable, '-c', "print('x'*10000)"], root, env, '', root/'overflow.log')
            self.assertLessEqual((root/'overflow.log').stat().st_size, 128)
            self.assertEqual(probe.formatter_run([sys.executable, '-c', "print('ok')"], root, env, '', root/'ok.log').splitlines(), ['ok'])


if __name__ == '__main__': unittest.main()
