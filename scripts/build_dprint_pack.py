#!/usr/bin/env python3
"""Build a deterministic, unpublished dprint engine + pinned WASM runtime pack.

Offline only: explicitly fetch source assets with bundle_resources.py first.
No executable is run. The committed manifest is the input trust anchor. A pack
revision binds the engine and all five WASM versions; schemas stay in the app.
"""
import argparse
import hashlib
import json
from pathlib import Path
import tempfile

import bundle_resources
from build_language_pack import digest_file, write_bundle

VERSION = '0.63.0-p1'


def build(resources, target, output):
    resources, output = Path(resources), Path(output)
    manifest = json.loads((resources / 'manifest.json').read_text())
    if manifest['dprint_version'] != '0.63.0':
        raise ValueError('Update the reviewed pack revision when updating dprint')
    selected = bundle_resources.selected(manifest, target)
    engines = [a for a in selected if a['kind'] == 'executable-archive']
    if len(engines) != 1 or engines[0]['target'] != target:
        raise ValueError('Exactly one pinned engine is required')
    wasm = [a for a in selected if a['kind'] == 'wasm']
    expected = {'typescript': '0.96.1', 'json': '0.25.2', 'markdown': '0.26.0',
                'toml': '0.9.0', 'ruff': '0.11.3'}
    if manifest['plugins'] != expected or {a['path'] for a in wasm} != {
            f'plugins/{name}-{version}.wasm' for name, version in expected.items()}:
        raise ValueError('Update the reviewed pack revision when updating WASM plugins')
    executable = 'bin/dprint' + ('.exe' if 'windows' in target else '')
    inputs = [(engines[0]['executable'], executable, True)]
    inputs += [(a, a['path'], False) for a in wasm]
    # All shipped legal notices accompany the independently distributed runtime.
    pinned = {a['path']: a for a in manifest['files']}
    licenses = ['licenses/dprint.txt'] + [f'licenses/dprint-plugin-{name}.txt' for name in expected]
    for name in licenses:
        if name not in pinned:
            raise ValueError(f'License missing from manifest: {name}')
        inputs.append((pinned[name], name, False))
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='dprint-pack-', dir=output.parent) as temporary:
        stage = Path(temporary)
        members = []
        for entry, destination, native in inputs:
            source = resources / entry['path']
            if source.is_symlink():
                raise ValueError('Symlink input is not trusted')
            bundle_resources.verify(source, entry)
            data = source.read_bytes()
            if len(data) != entry['bytes'] or hashlib.sha256(data).hexdigest() != entry['sha256']:
                raise ValueError('Input changed during verification')
            path = stage / destination
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)
            members.append(dict(path=destination, archive_path=destination,
                                bytes=len(data), sha256=hashlib.sha256(data).hexdigest(), executable=native))
        provenance = dict(format_version=1, toolchain='dprint', version=VERSION, target=target,
                          engine_version=manifest['dprint_version'], plugins=manifest['plugins'],
                          sources=[engines[0]] + wasm, files=members.copy())
        data = (json.dumps(provenance, indent=2, sort_keys=True) + '\n').encode()
        (stage / 'PROVENANCE.json').write_bytes(data)
        members.append(dict(path='PROVENANCE.json', archive_path='PROVENANCE.json', bytes=len(data),
                            sha256=hashlib.sha256(data).hexdigest(), executable=False))
        write_bundle(stage, output, members)
    installed = sum(m['bytes'] for m in members)
    size = output.stat().st_size
    return dict(available=False, reason='Unpublished dprint runtime candidate; native validation and release approval required.',
                kind='dprint', executable=executable, wasm_plugins=[f'plugins/{n}-{v}.wasm' for n, v in expected.items()],
                environment={}, max_installed_bytes=installed,
                archives=[dict(format='tar.gz', asset_name=output.name, sha256=digest_file(output), bytes=size,
                               max_download_bytes=size, max_expanded_bytes=installed + len(members) * 1024 + 10240,
                               members=members)])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--resources', type=Path, default=bundle_resources.RESOURCES)
    parser.add_argument('--target', required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--candidate', type=Path, required=True)
    args = parser.parse_args()
    candidate = build(args.resources, args.target, args.output)
    args.candidate.write_text(json.dumps(candidate, indent=2) + '\n')
    print(json.dumps({'version': VERSION, 'sha256': candidate['archives'][0]['sha256']}))


if __name__ == '__main__':
    main()
