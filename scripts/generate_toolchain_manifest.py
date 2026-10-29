#!/usr/bin/env python3
"""Maintainer-only Rust toolchain pin refresh (Python 3.11+).

Downloads official versioned manifests/archives, verifies upstream SHA256,
then hashes an exact minimal runtime/notice allowlist. Never runs installers.
One archive at a time bounds disk use; --cache is build staging, not app data.
Review generated changes and native dependency closure before enabling targets.
"""
import argparse
import hashlib
import json
from pathlib import Path, PurePosixPath
import tarfile
import urllib.request

VERSION = '1.99.1'
BASE = 'https://static.rust-lang.org/dist/'
TARGETS = [f'{arch}-{os}' for os in ('unknown-linux-gnu', 'apple-darwin', 'pc-windows-msvc')
           for arch in ('x86_64', 'aarch64')]


def digest_file(path):
    digest = hashlib.sha256()
    with path.open('rb') as source:
        while data := source.read(1024 * 1024):
            digest.update(data)
    return digest.hexdigest()


def selected_path(name, component):
    parts = PurePosixPath(name).parts
    if len(parts) < 2 or any(p in ('..', '.') for p in parts):
        return None
    if len(parts) == 2 and parts[1] in ('LICENSE-APACHE', 'LICENSE-MIT', 'COPYRIGHT'):
        return f'licenses/{component}/{parts[1]}'
    if len(parts) < 3:
        return None
    relative = '/'.join(parts[2:])
    if component == 'rustfmt-preview' and relative in ('bin/rustfmt', 'bin/rustfmt.exe'):
        return relative
    if component == 'rustc':
        # Reviewed Rust 1.99.1 loader closure (see TOOLCHAIN_PACKAGING.md).
        # Re-audit imports on version bumps; macro/sanitizer libs are not runtime dependencies.
        # Do not ship rustc/cargo/linkers/std.
        if (len(parts) == 4 and parts[2] == 'lib'
                and parts[3].startswith(('librustc_driver', 'libLLVM'))
                and ('.so' in parts[3] or parts[3].endswith('.dylib'))):
            return relative
        if len(parts) == 4 and parts[2] == 'bin' and parts[3].startswith('rustc_driver') and parts[3].lower().endswith('.dll'):
            return relative
        if relative.startswith('share/doc/rust/') and PurePosixPath(relative).name.startswith('COPYRIGHT'):
            return 'licenses/rustc/' + PurePosixPath(relative).name
    return None


def main():
    import tomllib  # Maintainer pin refresh requires Python 3.11+, not runtime packaging.
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--cache', type=Path, required=True)
    parser.add_argument('--output', type=Path, default=Path('resources/toolchain-sources.json'))
    parser.add_argument('--target', action='append', choices=TARGETS, required=True,
                        help='Explicitly authorize each target to download and inspect')
    args = parser.parse_args()
    args.cache.mkdir(parents=True, exist_ok=True)
    url = f'{BASE}channel-rust-{VERSION}.toml'
    data = urllib.request.urlopen(url, timeout=180).read()
    expected = urllib.request.urlopen(url + '.sha256', timeout=180).read().decode().split()[0]
    assert hashlib.sha256(data).hexdigest() == expected, 'channel manifest checksum mismatch'
    channel = tomllib.loads(data.decode())
    output = {'format_version': 1, 'toolchains': {'rustfmt': {
        'version': VERSION, 'channel_manifest': {'url': url, 'sha256': expected, 'date': channel['date']},
        'targets': {}}}}
    tool = output['toolchains']['rustfmt']
    if args.output.exists():
        old = json.loads(args.output.read_text())
        if old['toolchains']['rustfmt']['version'] == VERSION:
            tool['targets'].update(old['toolchains']['rustfmt']['targets'])
    for target in args.target:
        result = {'available': False, 'executable': 'bin/rustfmt' + ('.exe' if 'windows' in target else ''),
                  'environment': {}, 'archives': [], 'max_installed_bytes': 0,
                  'validation': {'archive_and_members': 'verified', 'native_execution': 'not-tested'}}
        try:
            for component in ('rustfmt-preview', 'rustc'):
                source = channel['pkg'][component]['target'][target]
                if not source['available']:
                    raise ValueError(f'Official component unavailable: {component}')
                url = source['xz_url']
                if not url.startswith(BASE + channel['date'] + '/'):
                    raise ValueError(f'Unexpected upstream URL: {url}')
                path = args.cache / url.rsplit('/', 1)[1]
                if not path.exists():
                    print('Downloading', url, flush=True)
                    with urllib.request.urlopen(url, timeout=180) as response, path.open('wb') as dest:
                        total = 0
                        while chunk := response.read(1024 * 1024):
                            total += len(chunk)
                            if total > 256 * 1024 * 1024:
                                raise ValueError('Official compressed component exceeds review limit')
                            dest.write(chunk)
                if digest_file(path) != source['xz_hash']:
                    raise ValueError(f'Archive checksum mismatch: {path}')
                archive = {'url': url, 'format': 'tar.xz', 'sha256': source['xz_hash'],
                           'bytes': path.stat().st_size, 'max_download_bytes': path.stat().st_size,
                           'max_expanded_bytes': 0, 'members': []}
                expanded = 0
                seen = set()
                with tarfile.open(path, 'r:xz') as tar:
                    for entry in tar:
                        expanded += ((entry.size + 511) // 512 + 1) * 512
                        if expanded > 2 * 1024 ** 3:
                            raise ValueError('Expanded component exceeds review limit')
                        dest = selected_path(entry.name, component)
                        if dest is None:
                            continue
                        if not entry.isfile() or dest in seen:
                            raise ValueError(f'Nonregular/duplicate selected member: {entry.name}')
                        seen.add(dest)
                        digest = hashlib.sha256()
                        with tar.extractfile(entry) as file:
                            while chunk := file.read(1024 * 1024):
                                digest.update(chunk)
                        record = {'archive_path': entry.name, 'path': dest, 'sha256': digest.hexdigest(),
                                  'bytes': entry.size, 'executable': dest.startswith(('bin/', 'lib/'))}
                        archive['members'].append(record)
                        result['max_installed_bytes'] += entry.size
                # Permit tar padding/extended headers, not unbounded expansion.
                archive['max_expanded_bytes'] = expanded + 4 * 1024 * 1024
                result['archives'].append(archive)
                print(target, component, 'verified', len(archive['members']), 'members', flush=True)
                path.unlink()
            members = [m for a in result['archives'] for m in a['members']]
            names = [m['path'] for m in members]
            if result['executable'] not in names or not any('rustc_driver' in name for name in names):
                raise ValueError('Missing rustfmt/compiler runtime closure')
            if len(names) != len(set(names)):
                raise ValueError('Conflicting target output paths')
            # Hashing an archive is not a dependency-closure review. A maintainer
            # must inspect imports and explicitly promote these build inputs.
            result['reason'] = 'Archive/member hashes verified; static dependency closure review required.'
            result['validation']['runtime_dependency_closure'] = 'not-verified'
        except Exception as error:
            result['reason'] = str(error)
            result['validation']['archive_and_members'] = 'incomplete'
            print(target, 'UNAVAILABLE:', error, flush=True)
        tool['targets'][target] = result
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(output, indent=2) + '\n')


if __name__ == '__main__':
    main()
