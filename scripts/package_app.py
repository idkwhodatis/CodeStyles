#!/usr/bin/env python3
"""Build offline native distributions from an already built CodeStyles executable.

No downloads occur here. Every selected formatter, plugin, and schema is verified
against the committed manifest before any output is published. ZIP, tar.gz and
Debian packages have deterministic metadata; native DMG and Inno Setup output do
not promise byte reproducibility. Requires Python 3.9+.
"""
import argparse
import gzip
import hashlib
import io
import json
import os
from pathlib import Path, PurePosixPath
import plistlib
import re
import shutil
import stat
import subprocess
import sys
import tarfile
import tempfile
import zipfile

import bundle_resources

ROOT = Path(__file__).resolve().parents[1]
EPOCH = 1577836800  # 2020-01-01, independent of checkout and build timestamps.
TARGETS = tuple(f'{arch}-{platform}' for arch in ('x86_64', 'aarch64')
                for platform in ('unknown-linux-gnu', 'apple-darwin', 'pc-windows-msvc'))
MACOS_MINIMUM = {'x86_64': '10.15.7', 'aarch64': '11.0'}
# The native CI Linux build targets glibc 2.28. Include dynamically loaded GUI
# libraries as well as ELF imports; audit_dependencies.py independently checks
# the actual binaries instead of treating this metadata as compatibility proof.
DEB_DEPENDS = ', '.join((
    'libc6 (>= 2.28)', 'libgcc-s1 | libgcc1', 'libstdc++6 (>= 8)',
    'libasound2t64 | libasound2', 'libfontconfig1', 'libfreetype6',
    'libwayland-client0', 'libwayland-cursor0', 'libwayland-egl1',
    'libx11-6', 'libx11-xcb1', 'libxcursor1', 'libxi6', 'libxrandr2',
    'libxcb1', 'libxcb-render0', 'libxcb-shape0',
    'libxcb-xfixes0', 'libxcb-xinput0', 'libxcb-xkb1',
    'libxkbcommon0', 'libxkbcommon-x11-0', 'libdbus-1-3',
    'libvulkan1', 'libegl1', 'libgl1', 'zlib1g', 'libzstd1',
))


def project_version(root):
    """Read the simple package version without requiring Python 3.11's tomllib."""
    cargo = (root / 'Cargo.toml').read_text(encoding='utf-8')
    package = re.search(r'^\[package\]\s*$(.*?)(?=^\[|\Z)', cargo, re.M | re.S)
    version = re.search(r'^version\s*=\s*"([^"]+)"', package[1], re.M) if package else None
    if not version:
        raise ValueError('Cargo.toml must declare [package].version, or provide --version')
    return version[1]


def validate_version(version):
    if not re.fullmatch(r'\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?', version):
        raise ValueError(f'Invalid application version: {version!r}')
    return version


def copy_file(source, destination, executable=False):
    if source.is_symlink() or not source.is_file():
        raise ValueError(f'Expected a regular file: {source}')
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(source, destination)
    destination.chmod(0o755 if executable else 0o644)


def write_file(path, data, executable=False):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(data if isinstance(data, bytes) else data.encode('utf-8'))
    path.chmod(0o755 if executable else 0o644)


def resource_path(root, name):
    """Only manifest-relative POSIX paths may enter a distribution."""
    relative = PurePosixPath(name)
    if (not name or '\\' in name or relative.is_absolute() or '..' in relative.parts
            or ':' in name or relative.as_posix() != name):
        raise ValueError(f'Unsafe resource path: {name!r}')
    path = root / relative
    if not path.resolve().is_relative_to(root.resolve()):
        raise ValueError(f'Resource escapes its root: {name!r}')
    return path


def stage_resources(source, destination, target):
    manifest = json.loads((source / 'manifest.json').read_text(encoding='utf-8'))
    artifacts = bundle_resources.selected(manifest, target)
    executables = [item for item in artifacts if item['kind'] == 'executable-archive']
    if len(executables) != 1 or executables[0].get('target') != target:
        raise ValueError(f'Manifest must contain exactly one formatter for {target}')
    expected = f'bin/{target}/dprint' + ('.exe' if 'windows' in target else '')
    if executables[0]['executable']['path'] != expected:
        raise ValueError(f'Formatter path must match the runtime layout: {expected}')
    records = list(bundle_resources.runtime_files(artifacts))
    names = set()
    for record in records:
        path = resource_path(source, record['path'])
        if path.is_symlink():
            raise ValueError(f'Symlink is not a pinned resource: {path}')
        bundle_resources.verify(path, record)
        names.add(record['path'])
    names.update(('manifest.json', 'README.md', 'THIRD_PARTY_NOTICES.md'))
    licenses = source / 'licenses'
    if not licenses.is_dir():
        raise ValueError('Resource licenses directory is missing')
    names.update(path.relative_to(source).as_posix() for path in licenses.rglob('*') if path.is_file())
    for name in sorted(names):
        copy_file(resource_path(source, name), destination / name,
                  executable=name == executables[0]['executable']['path'])
    # Check the staged bytes as well, so neither a bad copy nor source mutation
    # between verification and copying can silently produce a corrupt bundle.
    for record in records:
        bundle_resources.verify(destination / record['path'], record)


def stage_application(root, binary, target, version, destination):
    """Return the executable and resource directory in a portable staging root."""
    if 'apple-darwin' in target:
        contents = destination / 'CodeStyles.app' / 'Contents'
        executable = contents / 'MacOS' / 'codestyles'
        resources = contents / 'Resources' / 'resources'
        arch = target.split('-')[0]
        plist = {
            'CFBundleDevelopmentRegion': 'en',
            'CFBundleDisplayName': 'CodeStyles',
            'CFBundleExecutable': 'codestyles',
            'CFBundleIdentifier': 'io.github.idkwhodatis.codestyles',
            'CFBundleInfoDictionaryVersion': '6.0',
            'CFBundleName': 'CodeStyles',
            'CFBundlePackageType': 'APPL',
            'CFBundleShortVersionString': version.split('-')[0].split('+')[0],
            'CFBundleVersion': version.split('-')[0].split('+')[0],
            'LSApplicationCategoryType': 'public.app-category.developer-tools',
            'LSMinimumSystemVersion': MACOS_MINIMUM[arch],
            'NSHighResolutionCapable': True,
            'NSPrincipalClass': 'NSApplication',
        }
        write_file(contents / 'Info.plist', plistlib.dumps(plist, sort_keys=True))
        write_file(contents / 'PkgInfo', b'APPL????')
        copy_file(root / 'LICENSE', contents / 'Resources' / 'LICENSE')
    else:
        executable = destination / ('codestyles.exe' if 'windows' in target else 'codestyles')
        resources = destination / 'resources'
        copy_file(root / 'LICENSE', destination / 'LICENSE')
    copy_file(binary, executable, executable=True)
    stage_resources(root / 'resources', resources, target)
    return executable, resources


def tree_entries(root):
    for path in sorted(root.rglob('*')):
        if path.is_symlink():
            raise ValueError(f'Unexpected symlink in staged application: {path}')
        yield path


def file_mode(path):
    # Windows chmod/stat do not preserve POSIX execute bits. Archive modes are
    # assigned from these fixed package roles so cross-platform fixture tests
    # and archives created on Windows keep the same executable permissions.
    executable = path.name in ('codestyles', 'codestyles.exe', 'dprint', 'dprint.exe')
    return 0o755 if path.is_dir() or executable else 0o644


def write_zip(stage, output):
    with zipfile.ZipFile(output, 'w', compression=zipfile.ZIP_DEFLATED, compresslevel=9) as archive:
        for path in tree_entries(stage):
            # Explicit directory entries preserve empty directories and Unix modes.
            name = path.relative_to(stage).as_posix() + ('/' if path.is_dir() else '')
            info = zipfile.ZipInfo(name, date_time=(2020, 1, 1, 0, 0, 0))
            info.create_system = 3
            info.external_attr = ((stat.S_IFDIR if path.is_dir() else stat.S_IFREG) | file_mode(path)) << 16
            if path.is_dir():
                info.external_attr |= 0x10
            info.compress_type = zipfile.ZIP_DEFLATED
            archive.writestr(info, b'' if path.is_dir() else path.read_bytes(), compresslevel=9)


def tar_bytes(stage, prefix=''):
    output = io.BytesIO()
    with gzip.GzipFile(filename='', mode='wb', fileobj=output, mtime=EPOCH, compresslevel=9) as compressed:
        with tarfile.open(fileobj=compressed, mode='w', format=tarfile.GNU_FORMAT) as archive:
            paths = ([stage] if prefix else []) + list(tree_entries(stage))
            for path in paths:
                relative = path.relative_to(stage).as_posix()
                name = prefix if relative == '.' else '/'.join(filter(None, (prefix, relative)))
                info = tarfile.TarInfo(name)
                info.mode = file_mode(path)
                info.mtime = EPOCH
                info.uid = info.gid = 0
                info.uname = info.gname = 'root'
                if path.is_dir():
                    info.type = tarfile.DIRTYPE
                    archive.addfile(info)
                else:
                    data = path.read_bytes()
                    info.size = len(data)
                    archive.addfile(info, io.BytesIO(data))
    return output.getvalue()


def write_ar(output, members):
    """Write Debian 2.0's small, portable ar subset (all names <16 bytes)."""
    with output.open('wb') as archive:
        archive.write(b'!<arch>\n')
        for name, data in members:
            header = f'{name + "/":<16}{EPOCH:<12}{0:<6}{0:<6}{"100644":<8}{len(data):<10}`\n'
            archive.write(header.encode('ascii'))
            archive.write(data)
            if len(data) % 2:
                archive.write(b'\n')


def write_deb(root, stage, output, target, version, depends):
    if '\n' in depends or '\r' in depends or not depends.strip():
        raise ValueError('Debian dependencies must be a non-empty single line')
    architecture = 'amd64' if target.startswith('x86_64-') else 'arm64'
    with tempfile.TemporaryDirectory(prefix='codestyles-deb-') as temp:
        temp = Path(temp)
        data = temp / 'data'
        control = temp / 'control'
        app = data / 'usr' / 'lib' / 'codestyles'
        shutil.copytree(stage, app)
        write_file(data / 'usr/bin/codestyles', '#!/bin/sh\nexec /usr/lib/codestyles/codestyles "$@"\n', executable=True)
        copy_file(root / 'packaging/codestyles.desktop', data / 'usr/share/applications/codestyles.desktop')
        copy_file(root / 'packaging/codestyles.svg', data / 'usr/share/icons/hicolor/scalable/apps/codestyles.svg')
        copyright_dir = data / 'usr/share/doc/codestyles'
        copy_file(root / 'LICENSE', copyright_dir / 'copyright')
        copy_file(root / 'resources/THIRD_PARTY_NOTICES.md', copyright_dir / 'THIRD_PARTY_NOTICES.md')
        for source in (stage / 'resources/licenses').rglob('*'):
            if source.is_file():
                copy_file(source, copyright_dir / 'licenses' / source.relative_to(stage / 'resources/licenses'))
        size = sum(path.stat().st_size for path in tree_entries(data) if path.is_file())
        metadata = (
            f'Package: codestyles\nVersion: {version.replace("-", "~", 1)}\n'
            f'Architecture: {architecture}\nSection: devel\nPriority: optional\n'
            'Maintainer: CodeStyles contributors <noreply@github.com>\n'
            'Homepage: https://github.com/idkwhodatis/CodeStyles\n'
            f'Installed-Size: {(size + 1023) // 1024}\nDepends: {depends}\n'
            'Description: Native offline-first dprint style preview\n'
            ' Preview and configure formatting with bundled, SHA-256-pinned\n'
            ' dprint and TypeScript, JSON, Markdown and TOML plugins.\n'
        )
        write_file(control / 'control', metadata)
        checksums = ''.join(f'{hashlib.md5(path.read_bytes()).hexdigest()}  {path.relative_to(data).as_posix()}\n'
                            for path in tree_entries(data) if path.is_file())
        write_file(control / 'md5sums', checksums)
        write_ar(output, [('debian-binary', b'2.0\n'), ('control.tar.gz', tar_bytes(control)),
                          ('data.tar.gz', tar_bytes(data))])


def find_iscc(explicit=None):
    if explicit:
        return explicit
    configured = os.environ.get('ISCC')
    if configured:
        return configured
    found = shutil.which('ISCC.exe') or shutil.which('iscc')
    if found:
        return found
    for variable in ('ProgramFiles(x86)', 'ProgramFiles'):
        if os.environ.get(variable):
            candidate = Path(os.environ[variable]) / 'Inno Setup 6' / 'ISCC.exe'
            if candidate.is_file():
                return str(candidate)
    raise ValueError('Inno Setup 6.3+ is required for Windows installers; provide --iscc or set ISCC')


def write_installer(root, stage, output, target, version, compiler=None):
    arch = 'x64compatible' if target.startswith('x86_64-') else 'arm64'
    compiler = find_iscc(compiler)
    subprocess.run([compiler, f'/DSourceDir={stage.resolve()}', f'/DAppVersion={version}',
                    f'/DAppArch={arch}', f'/DOutputDir={output.parent.resolve()}',
                    f'/DOutputBase={output.stem}', str(root / 'packaging/windows.iss')], check=True)
    if not output.is_file() or not output.stat().st_size:
        raise ValueError(f'Inno Setup did not create {output}')


def write_dmg(stage, output):
    if sys.platform != 'darwin':
        raise ValueError('DMG creation requires macOS and hdiutil; use .app.zip on other hosts')
    tool = shutil.which('hdiutil')
    if not tool:
        raise ValueError('hdiutil is required to create a DMG')
    with tempfile.TemporaryDirectory(prefix='codestyles-dmg-') as temp:
        image_root = Path(temp) / 'image'
        shutil.copytree(stage, image_root)
        (image_root / 'Applications').symlink_to('/Applications')
        subprocess.run([tool, 'create', '-volname', 'CodeStyles', '-srcfolder', str(image_root),
                        '-format', 'UDZO', '-fs', 'HFS+', '-ov', str(output)], check=True)
    if not output.is_file() or not output.stat().st_size:
        raise ValueError(f'hdiutil did not create {output}')


def output_format(output, target):
    name = output.name.lower()
    if name.endswith('.zip'):
        return 'zip'
    if 'apple-darwin' in target and name.endswith('.dmg'):
        return 'dmg'
    if 'windows' in target and name.endswith('.exe'):
        return 'installer'
    if 'linux' in target and name.endswith('.tar.gz'):
        return 'tar'
    if 'linux' in target and name.endswith('.deb'):
        return 'deb'
    raise ValueError(f'Unsupported package extension for {target}: {output.name}')


def package(root, binary, target, version, outputs, iscc=None, deb_depends=DEB_DEPENDS):
    if target not in TARGETS:
        raise ValueError(f'Unsupported target: {target}')
    validate_version(version)
    # Reject invalid output combinations before staging or invoking native tools.
    formats = [(Path(output).resolve(), output_format(Path(output), target)) for output in outputs]
    with tempfile.TemporaryDirectory(prefix='codestyles-package-') as temporary:
        stage = Path(temporary) / 'stage'
        stage_application(root, binary, target, version, stage)
        for output, kind in formats:
            output.parent.mkdir(parents=True, exist_ok=True)
            # Native tools require a valid suffix. Build beside the destination
            # and atomically replace it only after successful creation.
            with tempfile.TemporaryDirectory(prefix='.codestyles-', dir=output.parent) as partial:
                pending = Path(partial) / output.name
                if kind == 'zip':
                    write_zip(stage, pending)
                elif kind == 'tar':
                    pending.write_bytes(tar_bytes(stage, f'codestyles-{version}-{target}'))
                elif kind == 'deb':
                    write_deb(root, stage, pending, target, version, deb_depends)
                elif kind == 'installer':
                    write_installer(root, stage, pending, target, version, iscc)
                elif kind == 'dmg':
                    write_dmg(stage, pending)
                pending.replace(output)
            print(f'Packaged {output}; formatter resources are included and verified.')
    return [output for output, _ in formats]


def default_outputs(directory, target, version):
    base = f'codestyles-{version}-{target}'
    suffixes = ('.app.zip', '.dmg') if 'apple-darwin' in target else (
        '.zip', '-setup.exe') if 'windows' in target else ('.tar.gz', '.deb')
    return [directory / (base + suffix) for suffix in suffixes]


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--target', required=True, choices=TARGETS)
    destination = parser.add_mutually_exclusive_group(required=True)
    destination.add_argument('--output', type=Path, help='Single package, format inferred from extension')
    destination.add_argument('--output-dir', type=Path, help='Build both native distribution formats')
    parser.add_argument('--profile', choices=['debug', 'release'], default='release')
    parser.add_argument('--binary', type=Path, help='Explicit native executable, e.g. a stripped copy')
    parser.add_argument('--version', help='Override Cargo package version (e.g. for packaging fixtures)')
    parser.add_argument('--iscc', help='Path to Inno Setup 6.3+ compiler on Windows')
    parser.add_argument('--deb-depends', default=DEB_DEPENDS, help='Audited Debian runtime dependency list')
    args = parser.parse_args(argv)
    name = 'codestyles.exe' if 'windows' in args.target else 'codestyles'
    binary = args.binary or ROOT / 'target' / args.profile / name
    if not binary.is_file():
        parser.error(f'Build the native {args.profile} executable first: missing {binary}')
    version = validate_version(args.version or project_version(ROOT))
    outputs = [args.output] if args.output else default_outputs(args.output_dir, args.target, version)
    package(ROOT, binary, args.target, version, outputs, args.iscc, args.deb_depends)


if __name__ == '__main__':
    try:
        main()
    except (OSError, ValueError, KeyError, subprocess.CalledProcessError, zipfile.BadZipFile) as error:
        print(f'Error: {error}', file=sys.stderr)
        sys.exit(1)
