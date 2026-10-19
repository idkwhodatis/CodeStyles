#!/usr/bin/env python3
"""Check the actual distribution outputs, including native installation containers.

No release is published. Native installation tests run only in ephemeral CI or an
explicit local invocation; Windows uses a private per-user install directory.
"""
import argparse
from contextlib import contextmanager
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import time

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
        if audit_compatibility.version_tuple(record[field]) > audit_compatibility.version_tuple('10.0'):
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


def preserve_primary_error(cleanup, primary_error):
    """A cleanup failure is additional evidence, never a replacement failure."""
    try:
        cleanup()
    except Exception as error:
        if primary_error is None:
            raise
        note = f'Additional distribution cleanup failure: {type(error).__name__}: {error}'
        print(note, file=sys.stderr)
        if hasattr(primary_error, 'add_note'):
            primary_error.add_note(note)


def write_cleanup_report(report, result):
    # Even a report-write failure must not replace the exception being diagnosed.
    preserve_primary_error(
        lambda: report.write_text(json.dumps(result, indent=2) + '\n', encoding='utf-8'),
        sys.exc_info()[1])


@contextmanager
def distribution_workspace(report):
    temporary = tempfile.TemporaryDirectory(prefix='codestyles-installer-')
    root = Path(temporary.name)
    try:
        yield root
    finally:
        def cleanup():
            result = {'status': 'failed', 'directory': str(root)}
            try:
                temporary.cleanup()
                result['status'] = 'passed'
            except BaseException as error:
                result['error'] = f'{type(error).__name__}: {error}'
                raise
            finally:
                write_cleanup_report(report, result)
        preserve_primary_error(cleanup, sys.exc_info()[1])


def uninstall_windows(install, report, timeout=120.0, poll_interval=0.2):
    """Wait for Inno's complete private installation to disappear.

    Inno uses a temporary uninstaller process, so the original launcher's exit
    is not a filesystem-cleanup barrier. Its documented command line has no
    wait switch. Keep /LOG outside the install and check the actual postcondition
    before TemporaryDirectory can race removal of a still-running unins000.exe.
    https://jrsoftware.org/isfaq.php (temporary _iu*.tmp uninstaller)
    https://jrsoftware.org/ishelp/topic_uninstcmdline.htm
    """
    if timeout <= 0 or poll_interval <= 0:
        raise ValueError('Uninstall timeout and polling interval must be positive')
    started = time.monotonic()
    deadline = started + timeout
    uninstaller = install / 'unins000.exe'
    log = report.with_suffix('.log').resolve()
    result = {'status': 'failed', 'install_directory': str(install),
              'log': str(log), 'timeout_seconds': timeout,
              'launcher_exited': False, 'installation_removed': False}
    try:
        if not uninstaller.is_file():
            raise FileNotFoundError(f'Missing installed uninstaller: {uninstaller}')
        completed = subprocess.run(
            [str(uninstaller), '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', f'/LOG={log}'],
            check=True, timeout=timeout)
        result['launcher_exited'] = True
        result['exit_code'] = completed.returncode
        while os.path.lexists(install):
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError(f'Uninstall did not remove {install} within {timeout:g} seconds')
            time.sleep(min(poll_interval, remaining))
        result['installation_removed'] = True
        result['status'] = 'passed'
    except BaseException as error:
        result['error'] = f'{type(error).__name__}: {error}'
        if isinstance(error, subprocess.CalledProcessError):
            result['launcher_exited'] = True
            result['exit_code'] = error.returncode
        if os.path.lexists(install):
            try:
                # Evidence only: never delete remaining files to make uninstall
                # appear successful, or traverse an untrusted directory tree.
                from itertools import islice
                result['remaining_entries'] = [path.name for path in islice(install.iterdir(), 50)]
            except OSError as listing_error:
                result['remaining_entries_error'] = str(listing_error)
        raise
    finally:
        result['elapsed_seconds'] = round(time.monotonic() - started, 3)
        write_cleanup_report(report, result)


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
    with distribution_workspace(reports / 'container-cleanup.json') as root:
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
                preserve_primary_error(
                    lambda: uninstall_windows(install, reports / 'installer-uninstall.json'),
                    sys.exc_info()[1])
    print('Distribution containers and extracted runtime checks passed.')


if __name__ == '__main__':
    main()
