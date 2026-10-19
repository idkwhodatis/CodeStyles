"""Host-independent checks of native-container extraction and signing sequencing."""
from contextlib import redirect_stderr
import hashlib
import io
import json
from pathlib import Path
import shutil
import subprocess
import struct
import tarfile
import tempfile
from types import SimpleNamespace
import unittest
from unittest import mock

import check_distribution
import package_app
import sign_macos
import test_audit_compatibility as binary_fixtures


class DistributionToolsTests(unittest.TestCase):
    def test_installer_bootstrap_is_reported_and_architecture_checked(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            setup = root / 'setup.exe'
            report = root / 'bootstrap.json'
            setup.write_bytes(binary_fixtures.pe(machine=0x14c, is64=False))
            check_distribution.audit_bootstrap(setup, report)
            self.assertEqual(json.loads(report.read_text())['binary']['architecture'], 'x86')
            windows10 = bytearray(binary_fixtures.pe(machine=0x14c, is64=False))
            for offset in (40, 48):
                struct.pack_into('<HH', windows10, 0x98 + offset, 10, 0)
            setup.write_bytes(windows10)
            check_distribution.audit_bootstrap(setup, report)
            for offset in (40, 48):
                newer = bytearray(windows10)
                struct.pack_into('<HH', newer, 0x98 + offset, 10, 1)
                setup.write_bytes(newer)
                with self.assertRaises(ValueError):
                    check_distribution.audit_bootstrap(setup, report)
            for payload in (binary_fixtures.pe(), binary_fixtures.pe(machine=0x14c, is64=False, imports=('VCRUNTIME140.dll',))):
                setup.write_bytes(payload)
                with self.assertRaises(ValueError):
                    check_distribution.audit_bootstrap(setup, report)
                self.assertEqual(json.loads(report.read_text())['status'], 'failed')

    def test_extracts_real_debian_payload(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            stage = root / 'stage'
            (stage / 'usr/lib/codestyles').mkdir(parents=True)
            (stage / 'usr/lib/codestyles/codestyles').write_bytes(b'app')
            deb = root / 'test.deb'
            package_app.write_ar(deb, [('debian-binary', b'2.0\n'),
                                     ('data.tar.gz', package_app.tar_bytes(stage))])
            check_distribution.extract_deb(deb, root / 'out')
            self.assertEqual((root / 'out/usr/lib/codestyles/codestyles').read_bytes(), b'app')

    def test_debian_extraction_rejects_traversal(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            content = io.BytesIO()
            with tarfile.open(fileobj=content, mode='w:gz') as archive:
                member = tarfile.TarInfo('../escape')
                member.size = 1
                archive.addfile(member, io.BytesIO(b'x'))
            deb = root / 'bad.deb'
            package_app.write_ar(deb, [('data.tar.gz', content.getvalue())])
            with self.assertRaises(tarfile.FilterError):
                check_distribution.extract_deb(deb, root / 'out')
            self.assertFalse((root / 'escape').exists())

    def test_uninstaller_waits_for_delayed_self_cleanup(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            install = root / '安装 CodeStyles'
            install.mkdir()
            (install / 'unins000.exe').write_bytes(b'uninstaller fixture')
            report = root / 'uninstall.json'
            clock = [10.0]
            sleeps = []

            def sleep(seconds):
                clock[0] += seconds
                sleeps.append(seconds)
                if len(sleeps) == 2:
                    shutil.rmtree(install)

            with mock.patch.object(check_distribution.subprocess, 'run',
                                   return_value=subprocess.CompletedProcess([], 0)) as run, \
                    mock.patch.object(check_distribution.time, 'monotonic', side_effect=lambda: clock[0]), \
                    mock.patch.object(check_distribution.time, 'sleep', side_effect=sleep):
                check_distribution.uninstall_windows(install, report, timeout=2, poll_interval=0.2)
            self.assertEqual(len(sleeps), 2)
            command = run.call_args.args[0]
            self.assertIn('/NORESTART', command)
            self.assertIn(f'/LOG={report.with_suffix(".log").resolve()}', command)
            self.assertEqual(run.call_args.kwargs['timeout'], 2)
            evidence = json.loads(report.read_text())
            self.assertEqual(evidence['status'], 'passed')
            self.assertTrue(evidence['launcher_exited'])
            self.assertTrue(evidence['installation_removed'])
            self.assertEqual(evidence['elapsed_seconds'], 0.4)

    def test_uninstaller_timeout_records_remaining_payload_without_deleting_it(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            install = root / 'installed'
            install.mkdir()
            (install / 'unins000.exe').write_bytes(b'uninstaller fixture')
            report = root / 'uninstall.json'
            clock = [0.0]
            sleeps = []

            def sleep(seconds):
                clock[0] += seconds
                sleeps.append(seconds)

            with mock.patch.object(check_distribution.subprocess, 'run',
                                   return_value=subprocess.CompletedProcess([], 0)), \
                    mock.patch.object(check_distribution.time, 'monotonic', side_effect=lambda: clock[0]), \
                    mock.patch.object(check_distribution.time, 'sleep', side_effect=sleep):
                with self.assertRaisesRegex(TimeoutError, 'within 1 seconds'):
                    check_distribution.uninstall_windows(install, report, timeout=1, poll_interval=0.4)
            self.assertAlmostEqual(sum(sleeps), 1)
            self.assertTrue((install / 'unins000.exe').exists())
            evidence = json.loads(report.read_text())
            self.assertEqual(evidence['status'], 'failed')
            self.assertFalse(evidence['installation_removed'])
            self.assertEqual(evidence['remaining_entries'], ['unins000.exe'])
            self.assertTrue(evidence['launcher_exited'])

    def test_uninstaller_native_failures_are_recorded_and_propagated(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            install = root / 'installed'
            install.mkdir()
            (install / 'unins000.exe').write_bytes(b'uninstaller fixture')
            report = root / 'uninstall.json'
            failures = (subprocess.CalledProcessError(7, ['uninstall']),
                        subprocess.TimeoutExpired(['uninstall'], 120))
            for failure in failures:
                with self.subTest(failure=type(failure).__name__), \
                        mock.patch.object(check_distribution.subprocess, 'run', side_effect=failure), \
                        mock.patch.object(check_distribution.time, 'sleep') as sleep:
                    with self.assertRaises(type(failure)) as caught:
                        check_distribution.uninstall_windows(install, report)
                self.assertIs(caught.exception, failure)
                sleep.assert_not_called()
                evidence = json.loads(report.read_text())
                self.assertEqual(evidence['status'], 'failed')
                self.assertIn(type(failure).__name__, evidence['error'])
                self.assertEqual(evidence['remaining_entries'], ['unins000.exe'])
                if isinstance(failure, subprocess.CalledProcessError):
                    self.assertEqual(evidence['exit_code'], 7)

    def test_missing_uninstaller_does_not_pass_cleanup(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            report = root / 'uninstall.json'
            with mock.patch.object(check_distribution.subprocess, 'run') as run:
                with self.assertRaisesRegex(FileNotFoundError, 'Missing installed uninstaller'):
                    check_distribution.uninstall_windows(root / 'absent', report)
            run.assert_not_called()
            self.assertEqual(json.loads(report.read_text())['status'], 'failed')

    def test_secondary_cleanup_failure_preserves_original_audit_error(self):
        original = subprocess.CalledProcessError(1, ['audit_compatibility.py'])
        cleanup = mock.Mock(side_effect=PermissionError('WinError 5: unins000.exe'))
        stderr = io.StringIO()
        with redirect_stderr(stderr), self.assertRaises(subprocess.CalledProcessError) as caught:
            try:
                raise original
            finally:
                check_distribution.preserve_primary_error(cleanup, original)
        self.assertIs(caught.exception, original)
        self.assertIn('WinError 5: unins000.exe', stderr.getvalue())
        with self.assertRaisesRegex(PermissionError, 'WinError 5'):
            check_distribution.preserve_primary_error(cleanup, None)

    def test_workspace_cleanup_records_failure_without_masking_original(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            report = root / 'cleanup.json'
            workspace = SimpleNamespace(name=str(root / 'workspace'),
                                        cleanup=mock.Mock(side_effect=PermissionError('WinError 5')))
            original = ValueError('original installed audit failure')
            with mock.patch.object(check_distribution.tempfile, 'TemporaryDirectory', return_value=workspace), \
                    redirect_stderr(io.StringIO()), self.assertRaises(ValueError) as caught:
                with check_distribution.distribution_workspace(report):
                    raise original
            self.assertIs(caught.exception, original)
            evidence = json.loads(report.read_text())
            self.assertEqual(evidence['status'], 'failed')
            self.assertIn('WinError 5', evidence['error'])
            with mock.patch.object(check_distribution.tempfile, 'TemporaryDirectory', return_value=workspace):
                with self.assertRaisesRegex(PermissionError, 'WinError 5'):
                    with check_distribution.distribution_workspace(report):
                        pass

    def test_workspace_cleanup_success_is_reported(self):
        with tempfile.TemporaryDirectory() as temporary:
            report = Path(temporary) / 'cleanup.json'
            with check_distribution.distribution_workspace(report) as root:
                (root / 'file').write_bytes(b'fixture')
            self.assertFalse(root.exists())
            self.assertEqual(json.loads(report.read_text())['status'], 'passed')

    def test_failed_report_write_does_not_mask_original_cleanup_failure(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            original = FileNotFoundError('missing uninstaller fixture')
            with redirect_stderr(io.StringIO()), self.assertRaises(FileNotFoundError) as caught:
                try:
                    raise original
                finally:
                    check_distribution.write_cleanup_report(root / 'missing/report.json', {'status': 'failed'})
            self.assertIs(caught.exception, original)

    def test_windows_main_preserves_audit_error_and_uninstall_evidence(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / 'app.zip').write_bytes(b'archive fixture')
            (root / 'app-setup.exe').write_bytes(b'installer fixture')
            original = subprocess.CalledProcessError(1, ['installed-audit'])

            def check_script(name, *args):
                if name == 'audit_compatibility.py' and 'installer-compatibility.json' in str(args):
                    raise original

            def native_run(command, **kwargs):
                if command[0].endswith('app-setup.exe'):
                    install = Path(next(arg[5:] for arg in command if arg.startswith('/DIR=')))
                    install.mkdir()
                    (install / 'unins000.exe').write_bytes(b'uninstaller fixture')
                    return subprocess.CompletedProcess(command, 0)
                raise subprocess.CalledProcessError(8, command)

            argv = ['check_distribution.py', '--target', 'aarch64-pc-windows-msvc',
                    '--directory', str(root), '--audit-only']
            with mock.patch.object(check_distribution.sys, 'argv', argv), \
                    mock.patch.object(check_distribution, 'run_script', side_effect=check_script), \
                    mock.patch.object(check_distribution, 'audit_bootstrap'), \
                    mock.patch.object(check_distribution.subprocess, 'run', side_effect=native_run), \
                    redirect_stderr(io.StringIO()), self.assertRaises(subprocess.CalledProcessError) as caught:
                check_distribution.main()
            self.assertIs(caught.exception, original)
            evidence = json.loads((root / 'installer-uninstall.json').read_text())
            self.assertEqual(evidence['status'], 'failed')
            self.assertEqual(evidence['exit_code'], 8)
            self.assertIn('unins000.exe', evidence['remaining_entries'])
            self.assertEqual(json.loads((root / 'container-cleanup.json').read_text())['status'], 'passed')

    def test_notarization_requires_explicit_acceptance(self):
        with tempfile.TemporaryDirectory() as temporary:
            report = Path(temporary) / 'notary.json'
            for status in ('Invalid', 'Rejected', 'In Progress'):
                response = subprocess.CompletedProcess([], 0, json.dumps({'status': status}), '')
                with mock.patch.object(sign_macos.subprocess, 'run', return_value=response):
                    with self.assertRaises(RuntimeError):
                        sign_macos.notarize('app.zip', 'existing-profile', report)
            response = subprocess.CompletedProcess([], 0, '{"status":"Accepted"}', '')
            with mock.patch.object(sign_macos.subprocess, 'run', return_value=response):
                sign_macos.notarize('app.zip', 'existing-profile', report)
            self.assertEqual(json.loads(report.read_text())['status'], 'Accepted')
            with mock.patch.object(sign_macos.subprocess, 'run', return_value=response) as command:
                sign_macos.notarize('app.zip', 'existing-profile', report, '/tmp/signing.keychain-db')
                self.assertEqual(command.call_args.args[0][-2:], ['--keychain', '/tmp/signing.keychain-db'])

    def test_signing_verifies_upstream_and_updates_only_signed_executable(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            app = root / 'CodeStyles.app'
            resources = app / 'Contents/Resources/resources'
            target = 'aarch64-apple-darwin'
            dprint_path = f'bin/{target}/dprint'
            original = b'upstream-dprint'
            plugin = b'pinned-wasm'
            records = []
            for path, data in ((dprint_path, original), ('plugins/a.wasm', plugin)):
                file = resources / path
                file.parent.mkdir(parents=True, exist_ok=True)
                file.write_bytes(data)
                records.append({'path': path, 'sha256': hashlib.sha256(data).hexdigest(), 'bytes': len(data)})
            manifest = {'files': records}
            (resources / 'manifest.json').write_text(json.dumps(manifest))
            (root / 'resources').mkdir()
            (root / 'resources/manifest.json').write_text(json.dumps(manifest))
            calls = []

            def command(*args):
                calls.append(args)
                if args[0] == 'codesign' and args[-1] == resources / dprint_path:
                    (resources / dprint_path).write_bytes(original + b'-signature')

            with mock.patch.object(sign_macos, 'ROOT', root), \
                    mock.patch.object(sign_macos, 'run', side_effect=command), \
                    mock.patch.object(sign_macos, 'notarize'):
                sign_macos.sign(app, target, 'existing-identity', 'existing-profile', root / 'signed')
            final = json.loads((resources / 'manifest.json').read_text())['files']
            self.assertEqual(final[0]['upstream_sha256'], records[0]['sha256'])
            self.assertEqual(final[0]['sha256'], hashlib.sha256(original + b'-signature').hexdigest())
            self.assertEqual(final[1], records[1])
            smoke_index = next(i for i, args in enumerate(calls) if 'smoke_package.py' in str(args))
            seal_index = next(i for i, args in enumerate(calls) if args[0] == 'codesign' and args[-1] == app)
            self.assertGreater(smoke_index, seal_index)
            # Re-signing transformed bytes must not bypass the upstream trust pin.
            with mock.patch.object(sign_macos, 'ROOT', root), mock.patch.object(sign_macos, 'run') as run:
                with self.assertRaises(ValueError):
                    sign_macos.sign(app, target, 'identity', 'profile', root / 'signed')
                run.assert_not_called()


if __name__ == '__main__':
    unittest.main()
