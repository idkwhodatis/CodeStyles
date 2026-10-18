#!/usr/bin/env python3
"""Check the actual distribution outputs, including native installation containers.

No release is published. Native installation tests run only in ephemeral CI or an
explicit local invocation; Windows uses a private per-user install directory.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile

import audit_compatibility

SCRIPTS = Path(__file__).resolve().parent


def run_script(name, *args):
    subprocess.run([sys.executable, str(SCRIPTS / name), *map(str, args)], check=True)


def extract_deb(archive, destination):
    """Read the standard ar wrapper and extract only the data tar safely."""
    with archive.open('rb') as stream:
        if stream.read(8) != b'!<arch>\n':
            raise ValueError('Invalid Debian archive magic')
        found = False
        while header := stream.read(60):
            if len(header) != 60 or header[58:] != b'`\n':
                raise ValueError('Invalid Debian ar header')
            name = header[:16].decode('ascii').strip().rstrip('/')
            size = int(header[48:58])
            data = stream.read(size)
            if len(data) != size:
                raise ValueError('Truncated Debian member')
            if size % 2:
                stream.read(1)
            if name.startswith('data.tar'):
                import io
                with tarfile.open(fileobj=io.BytesIO(data), mode='r:*') as tar:
                    tar.extractall(destination, filter='data')
                found = True
        if not found:
            raise ValueError('Debian archive has no data.tar member')



def audit_bootstrap(path, report):
    """The explicitly x86 Inno bootstrap is a helper, not the native app."""
    data = path.read_bytes()
    record = audit_compatibility.parse_pe(data)
    policy = json.loads((SCRIPTS.parent / 'resources/compatibility.json').read_text())['windows']
    errors = []
    if record['architecture'] != 'x86' or record['is_dll'] or record['subsystem'] != 2:
        errors.append('Expected the explicitly configured x86 GUI Inno bootstrap')
    for field in ('os_header_version', 'subsystem_version'):
        if audit_compatibility.version_tuple(record[field]) > (10, 0):
            errors.append(f'{field} exceeds Windows 10 target')
    for name in record['imports'] + record['delay_imports']:
        lower = name.lower()
        if lower not in policy['external_libraries'] and not any(
                lower.startswith(prefix) for prefix in policy['system_import_prefixes']):
            errors.append(f'Undeclared bootstrap import: {name}')
    result = {'status': 'failed' if errors else 'passed', 'role': 'installer-bootstrap',
              'sha256': hashlib.sha256(data).hexdigest(), 'binary': record, 'errors': errors,
              'note': 'x86 helper uses Windows emulation on ARM64; application and dprint remain native. Header/import audit does not establish old-OS API availability.'}
    report.write_text(json.dumps(result, indent=2) + '\n')
    if errors:
        raise ValueError('; '.join(errors))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--target', required=True)
    parser.add_argument('--directory', type=Path, required=True)
    parser.add_argument('--reports', type=Path)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument('--smoke-only', action='store_true')
    mode.add_argument('--audit-only', action='store_true')
    parser.add_argument('--require-readonly', action='store_true')
    parser.add_argument('--network-isolation', choices=['external'])
    args = parser.parse_args()
    reports = (args.reports or args.directory).resolve()
    reports.mkdir(parents=True, exist_ok=True)
    files = sorted(args.directory.glob('*'))
    archives = [p for p in files if p.name.endswith(('.zip', '.tar.gz'))]
    if len(archives) != 1:
        parser.error(f'Expected one portable archive for {args.target}, got {archives}')

    def check(path, label, directory=False):
        if not args.smoke_only:
            run_script('audit_compatibility.py', '--target', args.target, '--bundle', path,
                       '--report', reports / f'{label}-compatibility.json')
        if args.audit_only:
            return
        flags = ['--directory' if directory else '--archive', path,
                 '--report', reports / f'{label}-smoke.json']
        if args.require_readonly:
            flags += ['--require-readonly']
        if args.network_isolation:
            flags += ['--network-isolation', args.network_isolation]
        run_script('smoke_package.py', *flags)

    check(archives[0], 'portable')
    with tempfile.TemporaryDirectory(prefix='codestyles-installer-') as temporary:
        root = Path(temporary)
        if 'linux' in args.target:
            debs = [p for p in files if p.suffix == '.deb']
            if len(debs) != 1:
                parser.error('Expected one Debian package')
            extract_deb(debs[0], root)
            check(root / 'usr/lib/codestyles', 'deb', directory=True)
        elif 'apple' in args.target:
            dmgs = [p for p in files if p.suffix == '.dmg']
            if len(dmgs) != 1:
                parser.error('Expected one disk image')
            mount = root / 'mount'
            subprocess.run(['hdiutil', 'attach', '-readonly', '-nobrowse', '-mountpoint',
                            str(mount), str(dmgs[0])], check=True)
            try:
                check(mount / 'CodeStyles.app', 'dmg', directory=True)
            finally:
                subprocess.run(['hdiutil', 'detach', str(mount)], check=True)
        elif 'windows' in args.target:
            installers = [p for p in files if p.name.endswith('-setup.exe')]
            if len(installers) != 1:
                parser.error('Expected one Windows installer')
            if not args.smoke_only:
                audit_bootstrap(installers[0], reports / 'installer-bootstrap.json')
            install = root / '安装 CodeStyles'
            subprocess.run([str(installers[0].resolve()), '/VERYSILENT', '/SUPPRESSMSGBOXES',
                            '/NORESTART', '/SP-', f'/DIR={install}'], check=True)
            try:
                check(install, 'installer', directory=True)
            finally:
                uninstaller = install / 'unins000.exe'
                if uninstaller.exists():
                    subprocess.run([str(uninstaller), '/VERYSILENT', '/SUPPRESSMSGBOXES',
                                    '/NORESTART'], check=True)
    print('Distribution containers and extracted runtime checks passed.')


if __name__ == '__main__':
    main()
