#!/usr/bin/env python3
"""CI-only native candidate validation. Never publishes/enables a download.

Requires a clean catalog checkout and a native Rust host. Temporarily recompiles
with one locally pinned unpublished candidate, then restores exact catalog bytes.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import tarfile
import tempfile
import threading
import time

from build_language_pack import digest_file, safe_path

ROOT = Path(__file__).resolve().parents[1]
MIN_FREE = 2 * 1024 ** 3
MAX_LOG = 1024 ** 2
DEADLINE_ENV = 'CODESTYLES_NATIVE_VERIFY_DEADLINE'
TEST = 'real_managed_rust_pack_preview_and_reviewed_project_end_to_end'


def terminate_tree(process, windows=None):
    """Kill the complete child tree so inherited stdout cannot keep reads alive."""
    windows = os.name == 'nt' if windows is None else windows
    if windows:
        executable = Path(os.environ.get('SystemRoot', r'C:\Windows')) / 'System32' / 'taskkill.exe'
        try:
            subprocess.run([str(executable), '/PID', str(process.pid), '/T', '/F'],
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                           timeout=10, check=False, shell=False)
        finally:
            if process.poll() is None:
                process.kill()
    else:
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass


def run(argv, cwd, env, log):
    """Drain output with bounded retained log and a shared verification deadline."""
    remaining = float(env[DEADLINE_ENV]) - time.monotonic()
    if remaining <= 0:
        raise TimeoutError('Native verification deadline exceeded')
    child_env = {key: value for key, value in env.items() if key != DEADLINE_ENV}
    options = ({'creationflags': subprocess.CREATE_NEW_PROCESS_GROUP} if os.name == 'nt'
               else {'start_new_session': True})
    tail = bytearray()
    timed_out = threading.Event()
    with subprocess.Popen(argv, cwd=cwd, env=child_env, stdout=subprocess.PIPE,
                          stderr=subprocess.STDOUT, **options) as process:
        def timeout():
            timed_out.set()
            terminate_tree(process)
        timer = threading.Timer(remaining, timeout)
        timer.start()
        try:
            while chunk := process.stdout.read(65536):
                tail.extend(chunk)
                if len(tail) > MAX_LOG:
                    del tail[:-MAX_LOG]
            code = process.wait()
        finally:
            timer.cancel()
            log.write_bytes(tail)
    if timed_out.is_set() or code:
        raise RuntimeError(f'Child failed ({code}, timeout={timed_out.is_set()}): {argv[0]}; see {log.name}')
    return tail.decode('utf-8', errors='replace')


def validate_candidate(candidate, bundle, sources, target):
    tool = sources['toolchains']['rustfmt']
    source = tool['targets'][target]
    if candidate.get('available') is not False or 'url' in candidate:
        raise ValueError('Candidate must be unpublished')
    if candidate.get('executable') != source['executable'] or candidate.get('environment', {}) != source.get('environment', {}):
        raise ValueError('Candidate runtime settings differ from source')
    archives = candidate.get('archives', [])
    if len(archives) != 1:
        raise ValueError('Expected one candidate archive')
    archive = archives[0]
    if 'url' in archive or archive.get('format') != 'tar.gz' or archive.get('asset_name') != bundle.name:
        raise ValueError('Candidate is not a local gzip bundle')
    if archive['bytes'] != bundle.stat().st_size or archive['max_download_bytes'] != archive['bytes'] or digest_file(bundle) != archive['sha256']:
        raise ValueError('Candidate archive pin mismatch')
    expected = {m['path']: {**m, 'archive_path': m['path']} for a in source['archives'] for m in a['members']}
    records = archive['members']
    members = {m['path']: m for m in records}
    if len(members) != len(records) or set(members) != set(expected) | {'PROVENANCE.json'}:
        raise ValueError('Candidate member set differs from reviewed sources')
    for name, pin in expected.items():
        safe_path(name)
        if members[name] != pin:
            raise ValueError('Candidate member differs from reviewed source')
    provenance_pin = members['PROVENANCE.json']
    if (provenance_pin['archive_path'] != 'PROVENANCE.json'
            or provenance_pin['executable'] or not 0 < provenance_pin['bytes'] <= 1024 ** 2):
        raise ValueError('Invalid provenance member')
    installed = sum(m['bytes'] for m in records)
    if (installed != candidate['max_installed_bytes'] or installed > source['max_installed_bytes'] + 1024 ** 2
            or archive['max_expanded_bytes'] != installed + len(records) * 1024 + 10240):
        raise ValueError('Candidate expansion bounds differ')
    seen = set()
    expanded = 0
    with tarfile.open(bundle, 'r|gz') as tar:
        for entry in tar:
            expanded += 512 + ((entry.size + 511) // 512) * 512
            if expanded > archive['max_expanded_bytes']:
                raise ValueError('Candidate expansion exceeds bound')
            if entry.name not in members or entry.name in seen or not entry.isfile():
                raise ValueError('Unexpected candidate member/type')
            seen.add(entry.name)
            pin = members[entry.name]
            if entry.size != pin['bytes'] or entry.mode != (0o755 if pin['executable'] else 0o644):
                raise ValueError('Candidate member metadata differs')
            digest = hashlib.sha256()
            receipt = bytearray()
            with tar.extractfile(entry) as file:
                while chunk := file.read(1024 ** 2):
                    digest.update(chunk)
                    if entry.name == 'PROVENANCE.json':
                        receipt.extend(chunk)
            if digest.hexdigest() != pin['sha256']:
                raise ValueError('Candidate member digest mismatch')
            if entry.name == 'PROVENANCE.json':
                provenance = json.loads(receipt)
                if (provenance.get('format_version') != 1 or provenance.get('toolchain') != 'rustfmt'
                        or provenance.get('version') != tool['version'] or provenance.get('target') != target
                        or provenance.get('channel_manifest') != tool['channel_manifest']
                        or provenance.get('files') != list(expected.values())
                        or provenance.get('source_archives') != [{'url': a['url'], 'sha256': a['sha256']} for a in source['archives']]):
                    raise ValueError('Provenance target/version/sources mismatch')
    if seen != set(members):
        raise ValueError('Missing candidate members')
    return archive['sha256']


def verify(target, work_dir, report_dir, root=ROOT, runner=run, disk_usage=shutil.disk_usage):
    root = Path(root).resolve()
    work_dir, report_dir = Path(work_dir).resolve(), Path(report_dir).resolve()
    work_dir.mkdir(parents=True, exist_ok=True)
    report_dir.mkdir(parents=True, exist_ok=True)
    if disk_usage(work_dir).free < MIN_FREE:
        raise ValueError('Native candidate verification requires at least 2 GiB free')
    catalog_path = root / 'resources/toolchains.json'
    original = catalog_path.read_bytes()
    catalog = json.loads(original)
    source_path = root / 'resources/toolchain-sources.json'
    source_bytes = source_path.read_bytes()
    sources = json.loads(source_bytes)
    tool = sources['toolchains']['rustfmt']
    if (sources.get('format_version') != 1 or catalog.get('format_version') != 1
            or tool['version'] != catalog['toolchains']['rustfmt']['version']
            or target not in tool['targets'] or not tool['targets'][target]['available']):
        raise ValueError('Unreviewed source target/version')
    env = dict(os.environ)
    env['CARGO_BUILD_JOBS'] = '2'
    started = time.monotonic()
    env[DEADLINE_ENV] = str(started + 24 * 60)
    report = {'target': target, 'version': tool['version'], 'native_execution': 'failed',
              'published': False, 'catalog_restored': False,
              'source_manifest_sha256': hashlib.sha256(source_bytes).hexdigest(),
              'catalog_sha256': hashlib.sha256(original).hexdigest()}
    temporary = None
    try:
        runner(['git', 'diff', '--exit-code'], root, env, report_dir / 'catalog-before.log')
        host = runner(['rustc', '-vV'], root, env, report_dir / 'host.log')
        report['rustc_host_version'] = host
        if f'host: {target}' not in host.splitlines():
            raise ValueError('Target must match native rustc host')
        with tempfile.TemporaryDirectory(prefix='codestyles-native-check-', dir=work_dir) as temporary:
            own = Path(temporary)
            bundle, candidate_path = own / f'rustfmt-{target}.tar.gz', own / 'candidate.json'
            runner([sys.executable, str(root / 'scripts/build_language_pack.py'), '--target', target,
                    '--work-dir', str(own), '--output', str(bundle), '--candidate', str(candidate_path)],
                   root, env, report_dir / 'build.log')
            candidate = json.loads(candidate_path.read_text())
            digest = validate_candidate(candidate, bundle, sources, target)
            report.update(archive_sha256=digest, archive_bytes=bundle.stat().st_size,
                          installed_bytes=candidate['max_installed_bytes'])
            (report_dir / 'candidate.json').write_text(json.dumps(candidate, indent=2) + '\n')
            catalog['toolchains']['rustfmt']['targets'][target] = candidate
            catalog_path.write_text(json.dumps(catalog, indent=2) + '\n')
            env['CODESTYLES_TEST_RUSTFMT_ARCHIVE'] = str(bundle)
            runtime_temp = own / 'runtime-temp'
            runtime_temp.mkdir()
            for key in ('TMPDIR', 'TMP', 'TEMP'):
                env[key] = str(runtime_temp)
            # Test installs via the real manager, calls the adapter (env_clear +
            # absolute managed executable), tests preview/project, then uninstalls.
            output = runner(['cargo', 'test', '--locked', '--release', '--no-default-features', '--test', 'rustfmt',
                    TEST, '--', '--ignored', '--exact', '--nocapture', '--test-threads=1'],
                   root, env, report_dir / 'native-test.log')
            if 'test result: ok. 1 passed; 0 failed; 0 ignored;' not in output:
                raise RuntimeError('Exact native E2E did not report one executed passing test')
            report['native_execution'] = 'passed'
    finally:
        catalog_path.write_bytes(original)
        report['catalog_restored'] = catalog_path.read_bytes() == original
        report['own_temporary_removed'] = temporary is None or not Path(temporary).exists()
        (report_dir / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
        if source_path.read_bytes() != source_bytes or not report['catalog_restored']:
            raise RuntimeError('Tracked manifests changed during native verification')
        env[DEADLINE_ENV] = str(started + 25 * 60)  # reserve final minute for restoration checks
        runner(['git', 'diff', '--exit-code'],
               root, env, report_dir / 'catalog-after.log')
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--target', required=True)
    parser.add_argument('--work-dir', required=True, type=Path)
    parser.add_argument('--report-dir', required=True, type=Path)
    args = parser.parse_args()
    verify(args.target, args.work_dir, args.report_dir)


if __name__ == '__main__':
    main()
