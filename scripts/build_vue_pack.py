#!/usr/bin/env python3
"""Build a deterministic, unpublished universal Vue WASM pack from fixed local pins.

No network, installer, compiler or downloaded executable is invoked. Every input
byte must match its approved upstream release/LICENSE pin. All members are data,
never OS executables; only the application's sandboxed dprint host loads WASM.
"""
import argparse
import hashlib
import json
from pathlib import Path
import tempfile

from build_language_pack import digest_file, write_bundle

VERSION = 'markup-0.27.6-malva-0.16.0'
WASM_PLUGINS = ['plugins/markup-0.27.6.wasm', 'plugins/malva-0.16.0.wasm']
ASSETS = [
    ('markup_fmt-0.27.6-plugin.wasm', WASM_PLUGINS[0], 2009404,
     '93dd0c870b60431ceea59f055792fee4a353868223c813d3a44414b31d1df602',
     'https://github.com/g-plane/markup_fmt/releases/download/v0.27.6/plugin.wasm'),
    ('malva-0.16.0-plugin.wasm', WASM_PLUGINS[1], 1465125,
     '1c1b31f972ef5aae052f97b6723aaf962e0175c5bbe4ba877e092512af354113',
     'https://github.com/g-plane/malva/releases/download/v0.16.0/plugin.wasm'),
    ('markup_fmt-0.27.6-LICENSE', 'licenses/markup_fmt/LICENSE', 1073,
     'fe85f268adfe7f3f30994c01093db77b96b917b730cba4275765b1ae97b1f471',
     'https://raw.githubusercontent.com/g-plane/markup_fmt/v0.27.6/LICENSE'),
    ('malva-0.16.0-LICENSE', 'licenses/malva/LICENSE', 1073,
     'fe85f268adfe7f3f30994c01093db77b96b917b730cba4275765b1ae97b1f471',
     'https://raw.githubusercontent.com/g-plane/malva/v0.16.0/LICENSE'),
    ('markup_fmt-0.27.6-schema.json', 'schemas/markup-0.27.6.json', 13822,
     '754d4869aff8a1b3bf1489a90181c08e39662bb84029ccf29d9eb9a595fd9d50',
     'https://github.com/g-plane/markup_fmt/releases/download/v0.27.6/schema.json'),
    ('malva-0.16.0-schema.json', 'schemas/malva-0.16.0.json', 11233,
     'ead39a1236c079f5096e96854fa9ed3c682405a32157a638fdb191d071e11d9c',
     'https://github.com/g-plane/malva/releases/download/v0.16.0/schema.json'),
]


def pinned_bytes(path, size, expected):
    if path.is_symlink() or not path.is_file() or path.stat().st_size != size:
        raise ValueError(f'Not a regular pinned input of the expected size: {path.name}')
    with path.open('rb') as source:
        data = source.read(size + 1)
    if len(data) != size or hashlib.sha256(data).hexdigest() != expected:
        raise ValueError(f'Input checksum mismatch: {path.name}')
    return data


def build(source_dir, output):
    source_dir, output = Path(source_dir), Path(output)
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='vue-pack-stage-', dir=output.parent) as temporary:
        stage = Path(temporary)
        members, origins = [], []
        for name, destination, size, digest, url in ASSETS:
            data = pinned_bytes(source_dir / name, size, digest)
            path = stage / destination
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)
            members.append(dict(path=destination, archive_path=destination, bytes=size,
                                sha256=digest, executable=False))
            origins.append(dict(path=destination, url=url, bytes=size, sha256=digest))
        provenance = dict(format_version=1, toolchain='vue', version=VERSION,
                          target='universal-wasm', upstream_versions={'markup_fmt': '0.27.6', 'malva': '0.16.0'},
                          licenses={'markup_fmt': 'MIT', 'malva': 'MIT'},
                          wasm_plugins=WASM_PLUGINS, files=members.copy(), sources=origins)
        data = (json.dumps(provenance, indent=2, sort_keys=True) + '\n').encode('utf-8')
        (stage / 'PROVENANCE.json').write_bytes(data)
        members.append(dict(path='PROVENANCE.json', archive_path='PROVENANCE.json', bytes=len(data),
                            sha256=hashlib.sha256(data).hexdigest(), executable=False))
        write_bundle(stage, output, members)
    installed = sum(member['bytes'] for member in members)
    size = output.stat().st_size
    return dict(available=False, reason='Unpublished universal Vue WASM candidate; transitive dependency license audit pending.', kind='wasm',
                validation={'license_audit': 'upstream-MIT-notices-included; transitive-target-feature-audit-pending'},
                wasm_plugins=WASM_PLUGINS.copy(), environment={}, max_installed_bytes=installed,
                archives=[dict(format='tar.gz', asset_name=output.name, sha256=digest_file(output),
                               bytes=size, max_download_bytes=size,
                               max_expanded_bytes=installed + len(members) * 1024 + 10240, members=members)])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source-dir', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--candidate', type=Path, required=True)
    args = parser.parse_args()
    candidate = build(args.source_dir, args.output)
    args.candidate.write_bytes((json.dumps(candidate, indent=2) + '\n').encode('utf-8'))
    print(json.dumps({'version': VERSION, 'sha256': candidate['archives'][0]['sha256'],
                      'bytes': candidate['archives'][0]['bytes']}))


if __name__ == '__main__':
    main()
