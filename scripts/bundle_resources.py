#!/usr/bin/env python3
"""Fetch pinned official assets, verify offline resources, or package an offline target.

No dependencies beyond Python 3.9+. The committed manifest is the trust anchor;
fetch never replaces its hashes with values downloaded at build time.
"""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import sys
import tempfile
import urllib.request
import zipfile

ROOT = Path(__file__).resolve().parents[1]
RESOURCES = ROOT / 'resources'

# Minimum notices for the audited dependencies. This is not a complete SBOM or
# an assertion that every dependency/target has passed its redistribution gate.
APP_DEPENDENCY_NOTICES = (
    'licenses/gpui-kit-0.7.3-APACHE-2.0.txt',
    'licenses/gpui-kit-assets-0.7.3-LICENSE-LUCIDE.txt',
    'licenses/gpui-kit-assets-0.7.3-SOURCE.txt',
)
DPRINT_DEPENDENCY_NOTICES = (
    'licenses/wasmtime-49.0.2-cranelift-0.136.2.txt',
    'licenses/dprint-0.63.0-dependencies-SOURCE.txt',
)
OPTION_EXT_NOTICES = (
    'licenses/option-ext-0.2.0-MPL-2.0.txt',
    'licenses/option-ext-0.2.0-SOURCE.txt',
    'licenses/option-ext-0.2.0.crate',
)


def dependency_notices(component, target):
    if component == 'app':
        names = APP_DEPENDENCY_NOTICES
    elif component == 'dprint':
        names = DPRINT_DEPENDENCY_NOTICES
    else:
        raise ValueError(f'Unknown notice component: {component}')
    # Conservative coverage, not proof of retained code: dirs-sys gates its
    # option-ext-using xdg_user_dirs module to non-Apple Unix, but the reviewed
    # app/dprint call sites do not reach it. Lock presence is not link evidence.
    if target.endswith('-unknown-linux-gnu'):
        names += OPTION_EXT_NOTICES
    return names


def pinned_notices(manifest, names):
    records = {}
    for entry in manifest['files']:
        name = entry['path']
        if name in records:
            raise ValueError(f'Duplicate manifest file: {name}')
        records[name] = entry
    for name in names:
        if name not in records:
            raise ValueError(f'License missing from manifest: {name}')
    return [records[name] for name in names]


def verify(path, entry):
    if not path.is_file():
        raise ValueError(f'Missing resource: {path}; run fetch first')
    if path.stat().st_size != entry['bytes']:
        raise ValueError(f'Size mismatch: {path}')
    digest = hashlib.sha256(path.read_bytes()).hexdigest()
    if digest != entry['sha256']:
        raise ValueError(f'SHA-256 mismatch: {path}')


def selected(manifest, target):
    return [a for a in manifest['artifacts'] if 'target' not in a or target is None or a['target'] == target]


def fetch(artifacts):
    for entry in artifacts:
        path = RESOURCES / entry['path']
        path.parent.mkdir(parents=True, exist_ok=True)
        if not path.exists():
            print(f'Downloading {entry["url"]}', flush=True)
            request = urllib.request.Request(entry['url'], headers={'User-Agent': 'CodeStyles-resource-bundler'})
            temporary = None
            try:
                with urllib.request.urlopen(request, timeout=180) as response:
                    with tempfile.NamedTemporaryFile(dir=path.parent, delete=False) as temp:
                        temporary = Path(temp.name)
                        shutil.copyfileobj(response, temp)
                verify(temporary, entry)
                temporary.replace(path)
            finally:
                if temporary is not None:
                    temporary.unlink(missing_ok=True)
        verify(path, entry)
        if entry['kind'] == 'executable-archive':
            executable = entry['executable']
            dest = RESOURCES / executable['path']
            dest.parent.mkdir(parents=True, exist_ok=True)
            # Extract exactly one known member; never extract arbitrary paths.
            with zipfile.ZipFile(path) as archive:
                data = archive.read(dest.name)
            if len(data) != executable['bytes'] or hashlib.sha256(data).hexdigest() != executable['sha256']:
                raise ValueError(f'Executable checksum mismatch in {path}')
            dest.write_bytes(data)
            dest.chmod(0o755)
        print(f'Verified {entry["path"]}')


def runtime_files(artifacts):
    for entry in artifacts:
        yield entry['executable'] if entry['kind'] == 'executable-archive' else entry


def main():
    manifest = json.loads((RESOURCES / 'manifest.json').read_text())
    targets = [a['target'] for a in manifest['artifacts'] if 'target' in a]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=['fetch', 'verify', 'package'])
    parser.add_argument('--target', choices=targets, help='Omit for all platforms; required for package')
    parser.add_argument('--output', type=Path, help='Output ZIP for package')
    args = parser.parse_args()
    artifacts = selected(manifest, args.target)
    if args.command == 'fetch':
        fetch(artifacts)
    else:
        files = list(runtime_files(artifacts))
        for entry in files:
            verify(RESOURCES / entry['path'], entry)
        if args.command == 'package':
            if not args.target or not args.output:
                parser.error('package requires --target and --output')
            args.output.parent.mkdir(parents=True, exist_ok=True)
            # Fixed timestamps and permissions make byte-identical packages.
            with zipfile.ZipFile(args.output, 'w', compression=zipfile.ZIP_DEFLATED, compresslevel=9) as archive:
                names = [f['path'] for f in files] + ['manifest.json', 'README.md', 'THIRD_PARTY_NOTICES.md']
                names += [p.relative_to(RESOURCES).as_posix() for p in (RESOURCES / 'licenses').glob('*')]
                for name in sorted(set(names)):
                    info = zipfile.ZipInfo('resources/' + name, date_time=(2020, 1, 1, 0, 0, 0))
                    mode = 0o755 if name.startswith('bin/') else 0o644
                    info.external_attr = (0o100000 | mode) << 16
                    info.compress_type = zipfile.ZIP_DEFLATED
                    archive.writestr(info, (RESOURCES / name).read_bytes())
            print(f'Created offline resource bundle: {args.output}')
        print(f'Verified {len(files)} runtime resources without network access.')


if __name__ == '__main__':
    try:
        main()
    except (OSError, ValueError, KeyError, zipfile.BadZipFile) as error:
        print(f'Error: {error}', file=sys.stderr)
        sys.exit(1)
