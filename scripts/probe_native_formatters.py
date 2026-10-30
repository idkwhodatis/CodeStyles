#!/usr/bin/env python3
"""Stage-one native closure probe, NOT packaging, manager acceptance or publication.

Only pinned official archives and a fixed member shortlist are accepted. Missing
members/runtime libraries fail closed; no SDK-wide fallback or dynamic inclusion.
Outputs are review evidence only, never an application catalog or release bundle.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import stat
import subprocess
import tarfile
import tempfile
import time
import threading
import urllib.request
import urllib.parse
import zipfile

from build_language_pack import ROOT, digest_file, safe_path
from verify_native_language_pack import run, terminate_tree, DEADLINE_ENV

SOURCES = ROOT / 'resources/native-formatter-sources.json'
TARGETS = {
    'x86_64-unknown-linux-gnu': ('linux', 'x64', 'amd64', 'x86_64'),
    'aarch64-unknown-linux-gnu': ('linux', 'arm64', 'arm64', 'arm64'),
    'x86_64-apple-darwin': ('macos', 'x64', 'amd64', 'x86_64'),
    'aarch64-apple-darwin': ('macos', 'arm64', 'arm64', 'arm64'),
    'x86_64-pc-windows-msvc': ('windows', 'x64', 'amd64', 'x86_64'),
    'aarch64-pc-windows-msvc': ('windows', 'arm64', 'arm64', 'arm64'),
}
MAX_EXPANDED = 2 * 1024**3
MAX_ENTRIES = 100_000
MAX_FORMAT_OUTPUT = 32 * 1024**2


def host_matches(target, system=None, machine=None):
    system = (system or platform.system()).lower()
    machine = (machine or platform.machine()).lower()
    system = {'darwin': 'macos'}.get(system, system)
    machine = {'amd64': 'x86_64', 'aarch64': 'arm64'}.get(machine, machine)
    return (system, machine) == (TARGETS[target][0], TARGETS[target][3])


def source_pin(manifest, tool, target):
    os_name, dart_arch, go_arch, _ = TARGETS[target]
    family = 'go' if tool == 'gofmt' else 'dart'
    source = manifest['metadata_only_other_targets'][family]
    platform_name = 'darwin' if family == 'go' and os_name == 'macos' else os_name
    arch = go_arch if family == 'go' else dart_arch
    matches = [p for p in source['targets'] if p['os'] == platform_name and p['arch'] == arch]
    if len(matches) != 1:
        raise ValueError('Expected exactly one pinned official source')
    pin = matches[0]
    version = source['version']
    if family == 'go':
        suffix = 'zip' if os_name == 'windows' else 'tar.gz'
        expected = f'https://go.dev/dl/go{version}.{platform_name}-{arch}.{suffix}'
    else:
        expected = f'https://storage.googleapis.com/dart-archive/channels/stable/release/{version}/sdk/dartsdk-{platform_name}-{arch}-release.zip'
    if pin['url'] != expected or not 0 < pin['bytes'] <= 512 * 1024**2:
        raise ValueError('Invalid fixed official archive URL/size')
    if len(pin['sha256']) != 64 or any(c not in '0123456789abcdef' for c in pin['sha256']):
        raise ValueError('Invalid source hash')
    return version, pin


def members(tool, target):
    exe = '.exe' if TARGETS[target][0] == 'windows' else ''
    if tool == 'gofmt':
        return {f'go/bin/gofmt{exe}': f'bin/gofmt{exe}', 'go/LICENSE': 'LICENSE', 'go/PATENTS': 'PATENTS'}
    return {f'dart-sdk/bin/dart{exe}': f'bin/dart{exe}',
            'dart-sdk/bin/snapshots/dartdev_aot.dart.snapshot': 'bin/snapshots/dartdev_aot.dart.snapshot',
            'dart-sdk/LICENSE': 'LICENSE', 'dart-sdk/version': 'version', 'dart-sdk/revision': 'revision',
            'dart-sdk/sdk_packages.yaml': 'sdk_packages.yaml'}


class OfficialRedirects(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        parsed = urllib.parse.urlparse(newurl)
        if parsed.scheme != 'https' or parsed.hostname not in ('go.dev', 'dl.google.com', 'storage.googleapis.com') or parsed.username or parsed.password:
            raise ValueError('Source redirect left fixed official hosts')
        return super().redirect_request(req, fp, code, msg, headers, newurl)


def download(pin, output):
    opener = urllib.request.build_opener(OfficialRedirects())
    digest = hashlib.sha256(); count = 0; deadline = time.monotonic() + 600
    with opener.open(pin['url'], timeout=30) as response, output.open('xb') as stream:
        while chunk := response.read(1024 * 1024):
            count += len(chunk)
            if count > pin['bytes'] or time.monotonic() > deadline:
                raise ValueError('Archive download limit exceeded')
            digest.update(chunk); stream.write(chunk)
    if count != pin['bytes'] or digest.hexdigest() != pin['sha256']:
        raise ValueError('Official archive hash/size mismatch')


def extract(archive, selected, destination):
    destination.mkdir()
    found = set(); names = set(); total = 0
    def member(name, size, regular, reader):
        nonlocal total
        safe_path(name.rstrip('/'))
        if name in names or len(names) >= MAX_ENTRIES:
            raise ValueError('Duplicate archive entry or excessive entry count')
        names.add(name); total += size
        if total > MAX_EXPANDED:
            raise ValueError('Archive expanded limit exceeded')
        if name not in selected:
            return
        if not regular:
            raise ValueError('Selected source member is not a regular file')
        path = destination / safe_path(selected[name]); path.parent.mkdir(parents=True, exist_ok=True)
        with reader() as source, path.open('xb') as output:
            remaining = size
            while remaining:
                chunk = source.read(min(1024 * 1024, remaining))
                if not chunk: raise ValueError('Truncated selected member')
                output.write(chunk); remaining -= len(chunk)
            if source.read(1): raise ValueError('Selected member exceeded size')
        path.chmod(0o755 if selected[name].startswith('bin/') else 0o644)
        found.add(name)
    if zipfile.is_zipfile(archive):
        with zipfile.ZipFile(archive) as bundle:
            for item in bundle.infolist():
                mode = item.external_attr >> 16
                regular = not item.is_dir() and stat.S_IFMT(mode) in (0, stat.S_IFREG)
                member(item.filename, item.file_size, regular, lambda i=item: bundle.open(i))
    else:
        with tarfile.open(archive, 'r:gz') as bundle:
            for item in bundle:
                member(item.name, item.size, item.isfile(), lambda i=item: bundle.extractfile(i))
    if found != set(selected):
        raise ValueError(f'Missing fixed closure members: {sorted(set(selected) - found)}')
    return [{'archive_path': name, 'path': selected[name], 'bytes': (destination / selected[name]).stat().st_size,
             'sha256': digest_file(destination / selected[name])} for name in sorted(selected)]


def formatter_run(args, home, env, source, log, expect_failure=False):
    options = {'creationflags': subprocess.CREATE_NEW_PROCESS_GROUP} if os.name == 'nt' else {'start_new_session': True}
    child_env = {k: v for k, v in env.items() if k != DEADLINE_ENV}
    cap = MAX_FORMAT_OUTPUT
    buffers = [bytearray(), bytearray()]
    overflow = threading.Event()
    pipe_errors = []
    def drain(pipe, buffer):
        try:
            while chunk := pipe.read(65536):
                remaining = cap - len(buffer)
                buffer.extend(chunk[:remaining])
                if len(chunk) > remaining: overflow.set()
        except OSError as error:
            pipe_errors.append(error)
        finally:
            pipe.close()
    with subprocess.Popen(args, cwd=home, env=child_env, stdin=subprocess.PIPE,
                          stdout=subprocess.PIPE, stderr=subprocess.PIPE, **options) as child:
        def write():
            try:
                child.stdin.write(source.encode()); child.stdin.close()
            except OSError as error:
                pipe_errors.append(error)
                child.stdin.close()
        threads = [threading.Thread(target=drain, args=(child.stdout, buffers[0]), daemon=True),
                   threading.Thread(target=drain, args=(child.stderr, buffers[1]), daemon=True),
                   threading.Thread(target=write, daemon=True)]
        for thread in threads: thread.start()
        deadline = time.monotonic() + 30
        failure = None
        while child.poll() is None or any(thread.is_alive() for thread in threads):
            if overflow.is_set():
                failure = 'Formatter smoke output overflow'; break
            if time.monotonic() >= deadline:
                failure = 'Formatter smoke timed out'; break
            time.sleep(0.01)
        if failure:
            terminate_tree(child)
        child.wait(timeout=10)
        for thread in threads: thread.join(timeout=10)
        if any(thread.is_alive() for thread in threads):
            raise RuntimeError('Formatter pipe cleanup failed')
    out, error = buffers
    log.write_bytes((out[-512*1024:] + error[-512*1024:]))
    if failure: raise RuntimeError(failure)
    if overflow.is_set(): raise ValueError('Formatter smoke output overflow')
    if expect_failure:
        if child.returncode == 0: raise ValueError('Invalid syntax was accepted')
        return None
    if child.returncode != 0 or error or pipe_errors:
        raise ValueError('Formatter smoke failed; inspect log (possibly incomplete native closure)')
    return out.decode('utf-8')


def probe(tool, target, work, reports):
    if not host_matches(target): raise ValueError('Refusing to execute a non-native target')
    manifest_bytes = SOURCES.read_bytes(); manifest = json.loads(manifest_bytes)
    version, pin = source_pin(manifest, tool, target)
    report = {'stage': 'native-closure-probe', 'tool': tool, 'target': target, 'version': version,
              'available': False, 'release_approved': False, 'manager_e2e': 'not-run',
              'license_audit': 'requires-separate-review', 'source': pin,
              'source_manifest_sha256': hashlib.sha256(manifest_bytes).hexdigest(), 'smoke': 'not-run'}
    reports.mkdir(parents=True, exist_ok=True); work.mkdir(parents=True, exist_ok=True)
    try:
        with tempfile.TemporaryDirectory(prefix=f'{tool}-probe-', dir=work) as temporary:
            stage = Path(temporary); archive = stage / 'source.archive'; runtime = stage / 'runtime'
            download(pin, archive)
            report['members'] = extract(archive, members(tool, target), runtime)
            executable = runtime / ('bin/' + ('gofmt' if tool == 'gofmt' else 'dart') + ('.exe' if os.name == 'nt' else ''))
            home = stage / 'home'; home.mkdir()
            env = {k: str(home) for k in ('HOME', 'USERPROFILE', 'APPDATA', 'LOCALAPPDATA', 'XDG_CONFIG_HOME', 'XDG_CACHE_HOME', 'TMPDIR', 'TMP', 'TEMP')}
            for key in ('SystemRoot', 'WINDIR'):
                if key in os.environ: env[key] = os.environ[key]
            env[DEADLINE_ENV] = str(time.monotonic() + 180)
            if tool == 'gofmt':
                config = home / 'Library/Application Support' if platform.system() == 'Darwin' else home
                telemetry = config / 'go/telemetry'; telemetry.mkdir(parents=True)
                (telemetry / 'mode').write_text('off 2026-10-30')
                source = stage / 'source.go'; source.write_text('package main\nfunc main( ){println(1)}\n')
                args = [str(executable)]
            else:
                source = stage / 'source.dart'; source.write_text("void main( ){print('hello');}\n")
                args = [str(executable), '--suppress-analytics', 'format', '--output=show', '--summary=none', '--show=none', '--language-version=3.13', '--page-width=80', '--trailing-commas=automate']
            scanner = 'otool' if platform.system() == 'Darwin' else ('dumpbin' if os.name == 'nt' else 'readelf')
            binary = shutil.which(scanner)
            if binary:
                flags = ['-L'] if scanner == 'otool' else (['/DEPENDENTS'] if scanner == 'dumpbin' else ['-d', '-V', '-h'])
                report['dependencies'] = run([binary, *flags, str(executable)], home, {**os.environ, DEADLINE_ENV: str(time.monotonic() + 60)}, reports / f'{tool}-dependencies.log')
            else:
                report['dependencies'] = f'{scanner} unavailable; dependency/ABI validation remains incomplete'
            original = source.read_text()
            output = formatter_run(args, home, env, original, reports / f'{tool}-format.log')
            if not output.strip() or output == original or source.read_text() != original:
                raise ValueError('Formatter did not produce changed output without modifying source')
            source.write_text(output)
            if formatter_run(args, home, env, output, reports / f'{tool}-idempotence.log') != output:
                raise ValueError('Formatter is not idempotent')
            formatter_run(args, home, env, 'package main\nfunc main( {' if tool == 'gofmt' else 'void main( {', reports / f'{tool}-invalid.log', expect_failure=True)
            report['smoke'] = 'native-change-idempotence-invalid-syntax-passed'

    except BaseException as error:
        report['error'] = str(error); raise
    finally:
        (reports / f'{tool}-probe.json').write_text(json.dumps(report, indent=2) + '\n')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--target', required=True, choices=TARGETS)
    parser.add_argument('--tool', required=True, choices=('gofmt', 'dartfmt'))
    parser.add_argument('--work-dir', required=True, type=Path)
    parser.add_argument('--report-dir', required=True, type=Path)
    args = parser.parse_args(); probe(args.tool, args.target, args.work_dir, args.report_dir)


if __name__ == '__main__': main()
