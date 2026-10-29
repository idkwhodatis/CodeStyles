#!/usr/bin/env python3
"""Offline packaging contracts using small, SHA-pinned fixture resources."""
import contextlib
import hashlib
import io
import json
import os
from pathlib import Path
import plistlib
import shutil
import stat
import subprocess
import tarfile
import tempfile
import unittest
from unittest import mock
import zipfile

import package_app as package


class PackageTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.binary = self.root / 'built-app'
        self.binary.write_bytes(b'app fixture')
        (self.root / 'Cargo.toml').write_text('[package]\nname = "codestyles"\nversion = "1.2.3"\n[dependencies]\n', encoding='utf-8')
        (self.root / 'LICENSE').write_text('App license\n', encoding='utf-8')
        shutil.copytree(package.ROOT / 'packaging', self.root / 'packaging')
        self.resources = self.root / 'resources'
        self.resources.mkdir()
        (self.resources / 'README.md').write_text('Resource usage\n', encoding='utf-8')
        (self.resources / 'THIRD_PARTY_NOTICES.md').write_text('Third party notices\n', encoding='utf-8')
        self.manifest = {'artifacts': [], 'files': []}
        for target in package.TARGETS:
            name = 'dprint.exe' if 'windows' in target else 'dprint'
            record = self.resource(f'bin/{target}/{name}', target.encode())
            self.manifest['artifacts'].append({'target': target, 'kind': 'executable-archive', 'executable': record})
        for plugin in ('typescript', 'json', 'markdown', 'toml', 'ruff'):
            for path, kind, data in ((f'plugins/{plugin}.wasm', 'wasm', b'\0asm fixture'),
                                     (f'schemas/{plugin}.json', 'schema', b'{"type":"object"}'),
                                     (f'licenses/{plugin}.txt', 'license', b'Plugin MIT license\n')):
                record = self.resource(path, data)
                self.manifest['artifacts'].append(dict(record, kind=kind))
        self.save_manifest()

    def resource(self, name, data):
        path = self.resources / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        record = {'path': name, 'bytes': len(data), 'sha256': hashlib.sha256(data).hexdigest()}
        self.manifest['files'].append(record)
        return record

    def save_manifest(self):
        (self.resources / 'manifest.json').write_text(json.dumps(self.manifest), encoding='utf-8')

    def build(self, target, suffix):
        output = self.root / f'dist/fixture{suffix}'
        with contextlib.redirect_stdout(io.StringIO()):
            package.package(self.root, self.binary, target, '1.2.3', [output])
        return output

    def assert_resources(self, names, prefix, target):
        executable = 'dprint.exe' if 'windows' in target else 'dprint'
        self.assertIn(f'{prefix}bin/{target}/{executable}', names)
        for other in package.TARGETS:
            if other != target:
                self.assertFalse(any(f'/bin/{other}/' in name for name in names))
        for plugin in ('typescript', 'json', 'markdown', 'toml', 'ruff'):
            for relative in (f'plugins/{plugin}.wasm', f'schemas/{plugin}.json', f'licenses/{plugin}.txt'):
                self.assertIn(prefix + relative, names)
        for relative in ('manifest.json', 'README.md', 'THIRD_PARTY_NOTICES.md'):
            self.assertIn(prefix + relative, names)

    def test_project_version_and_validated_artifact_names(self):
        self.assertEqual(package.project_version(self.root), '1.2.3')
        for value in ('1.2.3', '1.2.3-rc.1', '1.2.3+build.7'):
            self.assertEqual(package.validate_version(value), value)
        for value in ('../1.2.3', '1.2', 'v1.2.3', '1.2.3\nInjected: value'):
            with self.assertRaises(ValueError):
                package.validate_version(value)
        expected = {
            'x86_64-pc-windows-msvc': ['.zip', '-setup.exe'],
            'aarch64-apple-darwin': ['.app.zip', '.dmg'],
            'x86_64-unknown-linux-gnu': ['.tar.gz', '.deb'],
        }
        for target, suffixes in expected.items():
            self.assertEqual(package.default_outputs(Path('dist'), target, '1.2.3'),
                             [Path('dist') / f'codestyles-1.2.3-{target}{suffix}' for suffix in suffixes])

    def test_windows_portable_zip_both_architectures(self):
        for target in ('x86_64-pc-windows-msvc', 'aarch64-pc-windows-msvc'):
            with self.subTest(target=target), zipfile.ZipFile(self.build(target, '.zip')) as archive:
                names = archive.namelist()
                self.assertEqual(archive.read('codestyles.exe'), b'app fixture')
                self.assertIn('LICENSE', names)
                self.assert_resources(names, 'resources/', target)
                self.assertEqual(len(names), len(set(names)))
                self.assertTrue(all('\\' not in name and not name.startswith('/') for name in names))
                self.assertTrue(all(info.date_time == (2020, 1, 1, 0, 0, 0) for info in archive.infolist()))

    def test_macos_bundle_plist_paths_and_modes_both_architectures(self):
        for arch, minimum in package.MACOS_MINIMUM.items():
            target = f'{arch}-apple-darwin'
            with self.subTest(target=target), zipfile.ZipFile(self.build(target, '.app.zip')) as archive:
                prefix = 'CodeStyles.app/Contents/'
                info = plistlib.loads(archive.read(prefix + 'Info.plist'))
                self.assertEqual(info['CFBundlePackageType'], 'APPL')
                self.assertEqual(info['CFBundleExecutable'], 'codestyles')
                self.assertEqual(info['CFBundleVersion'], '1.2.3')
                self.assertEqual(info['CFBundleShortVersionString'], '1.2.3')
                self.assertEqual(info['LSMinimumSystemVersion'], minimum)
                self.assertIn('CFBundleIdentifier', info)
                self.assertEqual(archive.read(prefix + 'PkgInfo'), b'APPL????')
                self.assertEqual(archive.read(prefix + 'MacOS/codestyles'), b'app fixture')
                self.assertEqual(stat.S_IMODE(archive.getinfo(prefix + 'MacOS/codestyles').external_attr >> 16), 0o755)
                self.assertEqual(stat.S_IMODE(archive.getinfo(prefix + 'Info.plist').external_attr >> 16), 0o644)
                self.assertIn(prefix + 'Resources/LICENSE', archive.namelist())
                self.assert_resources(archive.namelist(), prefix + 'Resources/resources/', target)

    def test_linux_tar_root_resources_modes_and_reproducibility(self):
        target = 'x86_64-unknown-linux-gnu'
        output = self.build(target, '.tar.gz')
        first = output.read_bytes()
        for path in self.root.rglob('*'):
            if path.is_file():
                os.utime(path, (1700000000, 1700000000))
        self.assertEqual(self.build(target, '.tar.gz').read_bytes(), first)
        with tarfile.open(output) as archive:
            prefix = f'codestyles-1.2.3-{target}/'
            self.assert_resources(archive.getnames(), prefix + 'resources/', target)
            self.assertEqual(archive.getmember(prefix + 'codestyles').mode, 0o755)
            self.assertEqual(archive.getmember(prefix + 'resources/bin/' + target + '/dprint').mode, 0o755)
            self.assertEqual(archive.getmember(prefix + 'LICENSE').mode, 0o644)
            self.assertTrue(all(member.uid == 0 and member.gid == 0 and member.mtime == package.EPOCH
                                for member in archive.getmembers()))

    def read_deb(self, output):
        data = output.read_bytes()
        self.assertTrue(data.startswith(b'!<arch>\n'))
        members = {}
        position = 8
        while position < len(data):
            header = data[position:position + 60]
            self.assertEqual(header[58:60], b'`\n')
            name = header[:16].decode().strip().rstrip('/')
            length = int(header[48:58])
            position += 60
            members[name] = data[position:position + length]
            position += length + length % 2
        return members

    def test_debian_control_payload_desktop_and_license_both_architectures(self):
        for arch, expected in (('x86_64', 'amd64'), ('aarch64', 'arm64')):
            target = f'{arch}-unknown-linux-gnu'
            with self.subTest(target=target):
                output = self.build(target, '.deb')
                first = output.read_bytes()
                os.utime(self.binary, (1711111111, 1711111111))
                self.assertEqual(self.build(target, '.deb').read_bytes(), first)
                members = self.read_deb(output)
                self.assertEqual(list(members), ['debian-binary', 'control.tar.gz', 'data.tar.gz'])
                self.assertEqual(members['debian-binary'], b'2.0\n')
                with tarfile.open(fileobj=io.BytesIO(members['control.tar.gz'])) as control:
                    text = control.extractfile('control').read().decode()
                    self.assertIn(f'Architecture: {expected}\n', text)
                    self.assertIn('Version: 1.2.3\n', text)
                    self.assertIn('Depends: libc6 (>= 2.28)', text)
                    checksums = control.extractfile('md5sums').read().decode().splitlines()
                with tarfile.open(fileobj=io.BytesIO(members['data.tar.gz'])) as data:
                    names = data.getnames()
                    self.assertIn('usr/bin/codestyles', names)
                    self.assertIn('usr/share/applications/codestyles.desktop', names)
                    self.assertIn('usr/share/icons/hicolor/scalable/apps/codestyles.svg', names)
                    self.assertIn('usr/share/doc/codestyles/copyright', names)
                    self.assert_resources(names, 'usr/lib/codestyles/resources/', target)
                    self.assertEqual(data.getmember('usr/lib/codestyles/codestyles').mode, 0o755)
                    self.assertEqual(data.getmember('usr/bin/codestyles').mode, 0o755)
                    self.assertIn(b'exec /usr/lib/codestyles/codestyles "$@"', data.extractfile('usr/bin/codestyles').read())
                    for line in checksums:
                        checksum, name = line.split('  ', 1)
                        self.assertEqual(hashlib.md5(data.extractfile(name).read()).hexdigest(), checksum)
                if shutil.which('dpkg-deb'):
                    result = subprocess.check_output(['dpkg-deb', '--field', str(output), 'Architecture'], text=True)
                    self.assertEqual(result.strip(), expected)
                    subprocess.run(['dpkg-deb', '--contents', str(output)], check=True, stdout=subprocess.DEVNULL)

    def test_resource_corruption_missing_file_and_target_fail_closed(self):
        target = 'x86_64-unknown-linux-gnu'
        output = self.root / 'bad.zip'
        plugin = self.resources / 'plugins/typescript.wasm'
        original = plugin.read_bytes()
        plugin.write_bytes(b'X' * len(original))
        with self.assertRaisesRegex(ValueError, 'SHA-256 mismatch'):
            package.package(self.root, self.binary, target, '1.2.3', [output])
        self.assertFalse(output.exists())
        plugin.unlink()
        with self.assertRaisesRegex(ValueError, 'Missing resource'):
            package.package(self.root, self.binary, target, '1.2.3', [output])
        with self.assertRaisesRegex(ValueError, 'Unsupported target'):
            package.package(self.root, self.binary, 'made-up-platform', '1.2.3', [output])
        self.assertFalse(output.exists())

    def test_staged_resource_copy_is_checked_again_before_publication(self):
        original_copy = package.copy_file

        def corrupt_copy(source, destination, executable=False):
            original_copy(source, destination, executable)
            if destination.name == 'typescript.wasm':
                destination.write_bytes(b'X' * destination.stat().st_size)

        with mock.patch.object(package, 'copy_file', side_effect=corrupt_copy):
            with self.assertRaisesRegex(ValueError, 'SHA-256 mismatch'):
                self.build('x86_64-unknown-linux-gnu', '.zip')
        self.assertFalse((self.root / 'dist/fixture.zip').exists())

    def test_archive_execute_modes_do_not_depend_on_source_posix_bits(self):
        # Windows exposes no POSIX execute bits even after chmod(0755).
        stage = self.root / 'mode-stage'
        package.stage_application(self.root, self.binary, 'aarch64-apple-darwin', '1.2.3', stage)
        for path in stage.rglob('*'):
            if path.is_file():
                path.chmod(0o644)
        output = self.root / 'mode-test.zip'
        package.write_zip(stage, output)
        with zipfile.ZipFile(output) as archive:
            for path in ('CodeStyles.app/Contents/MacOS/codestyles',
                         'CodeStyles.app/Contents/Resources/resources/bin/aarch64-apple-darwin/dprint'):
                self.assertEqual(stat.S_IMODE(archive.getinfo(path).external_attr >> 16), 0o755)

    def test_missing_formatter_and_unsafe_manifest_paths_fail_closed(self):
        for name in ('../escape', '/absolute', 'C:/absolute', 'plugins\\windows', './relative'):
            with self.subTest(name=name), self.assertRaisesRegex(ValueError, 'Unsafe resource path'):
                package.resource_path(self.resources, name)
        self.manifest['artifacts'] = [item for item in self.manifest['artifacts'] if item.get('target') != 'aarch64-apple-darwin']
        self.save_manifest()
        with self.assertRaisesRegex(ValueError, 'exactly one formatter'):
            self.build('aarch64-apple-darwin', '.app.zip')

    def test_symlink_resources_are_rejected(self):
        plugin = self.resources / 'plugins/json.wasm'
        original = plugin.read_bytes()
        plugin.unlink()
        other = self.root / 'external.wasm'
        other.write_bytes(original)
        try:
            plugin.symlink_to(other)
        except OSError as error:
            self.skipTest(f'Host does not allow test symlinks: {error}')
        with self.assertRaisesRegex(ValueError, 'escapes|Symlink'):
            self.build('x86_64-unknown-linux-gnu', '.zip')

    def test_native_failure_does_not_replace_existing_output(self):
        output = self.root / 'app.dmg'
        output.write_bytes(b'existing good artifact')
        with mock.patch.object(package, 'write_dmg', side_effect=OSError('native failure')):
            with self.assertRaisesRegex(OSError, 'native failure'):
                package.package(self.root, self.binary, 'aarch64-apple-darwin', '1.2.3', [output])
        self.assertEqual(output.read_bytes(), b'existing good artifact')
        self.assertFalse(list(self.root.glob('.codestyles-*')))

    def test_windows_installer_compiler_arguments_and_complete_payload(self):
        for target, arch in (('x86_64-pc-windows-msvc', 'x64compatible'), ('aarch64-pc-windows-msvc', 'arm64')):
            stage = self.root / arch
            package.stage_application(self.root, self.binary, target, '1.2.3', stage)
            output = self.root / 'dist' / f'{arch}-setup.exe'
            output.parent.mkdir(exist_ok=True)

            def compiler(command, **kwargs):
                self.assertEqual(command[0], 'fixture-iscc')
                self.assertIn(f'/DAppArch={arch}', command)
                self.assertIn('/DAppVersion=1.2.3', command)
                self.assertIn(f'/DSourceDir={stage.resolve()}', command)
                self.assertEqual(command[-1], str(self.root / 'packaging/windows.iss'))
                self.assertTrue((stage / 'codestyles.exe').is_file())
                self.assertTrue((stage / 'resources/licenses/toml.txt').is_file())
                self.assertTrue(kwargs['check'])
                output.write_bytes(b'installer fixture')

            with self.subTest(target=target), mock.patch.object(package.subprocess, 'run', side_effect=compiler):
                package.write_installer(self.root, stage, output, target, '1.2.3', 'fixture-iscc')
            self.assertTrue(output.is_file())
        script = (self.root / 'packaging/windows.iss').read_text()
        self.assertIn('PrivilegesRequired=lowest', script)
        self.assertIn('recursesubdirs', script)
        self.assertIn('skipifsilent unchecked', script)
        self.assertIn('MinVersion=10.0.19045', script)
        self.assertIn('SetupArchitecture=x86', script)
        self.assertNotIn('MinVersion=10.0.22000', script)

    def test_macos_dmg_command_uses_app_bundle_and_applications_shortcut(self):
        stage = self.root / 'mac-stage'
        package.stage_application(self.root, self.binary, 'aarch64-apple-darwin', '1.2.3', stage)
        output = self.root / 'fixture.dmg'

        def hdiutil(command, **kwargs):
            source = Path(command[command.index('-srcfolder') + 1])
            self.assertTrue((source / 'CodeStyles.app/Contents/Info.plist').is_file())
            self.assertEqual(os.readlink(source / 'Applications'), '/Applications')
            self.assertIn('UDZO', command)
            self.assertTrue(kwargs['check'])
            output.write_bytes(b'dmg fixture')

        # Native Windows test runners often prohibit symlinks; real DMG creation
        # runs on macOS, while this command-contract test needs symlink support.
        if os.name == 'nt':
            self.skipTest('DMG requires macOS symlink semantics')
        with mock.patch.object(package.sys, 'platform', 'darwin'), mock.patch.object(package.shutil, 'which', return_value='/usr/bin/hdiutil'), mock.patch.object(package.subprocess, 'run', side_effect=hdiutil):
            package.write_dmg(stage, output)
        self.assertEqual(output.read_bytes(), b'dmg fixture')
        with mock.patch.object(package.sys, 'platform', 'linux'):
            with self.assertRaisesRegex(ValueError, 'requires macOS'):
                package.write_dmg(stage, output)

    def test_format_platform_mismatch_and_dependency_injection_rejected(self):
        for target, suffix in (('x86_64-pc-windows-msvc', '.deb'), ('aarch64-apple-darwin', '.exe'), ('x86_64-unknown-linux-gnu', '.dmg')):
            with self.subTest(target=target), self.assertRaisesRegex(ValueError, 'Unsupported package extension'):
                package.output_format(Path('bad' + suffix), target)
        with self.assertRaisesRegex(ValueError, 'single line'):
            package.write_deb(self.root, self.root / 'stage', self.root / 'bad.deb', 'x86_64-unknown-linux-gnu', '1.2.3', 'libc6\nInjected: bad')

    def test_zip_reproducibility_ignores_source_mtime(self):
        target = 'x86_64-pc-windows-msvc'
        first = self.build(target, '.zip').read_bytes()
        os.utime(self.binary, (1711111111, 1711111111))
        self.assertEqual(first, self.build(target, '.zip').read_bytes())


if __name__ == '__main__':
    unittest.main()
