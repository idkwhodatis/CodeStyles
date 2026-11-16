#!/usr/bin/env python3
"""Offline deterministic formatter-only Caddy WASM pack; never executes inputs.

The separately verified managed dprint engine is required. All candidates remain
unpublished/disabled pending native, license and release gates.
"""
import argparse
import hashlib
import json
from pathlib import Path
import tempfile

from build_language_pack import digest_file, write_bundle, safe_path
from compile_caddy_formatter_wasm import verify

ROOT = Path(__file__).resolve().parents[1]
RESOURCES = ROOT / 'resources'
SOURCES = RESOURCES / 'caddy-formatter-sources.json'


def build(wasm, output, sources=SOURCES, root=ROOT):
    root, output = Path(root), Path(output)
    plan = json.loads(Path(sources).read_text())
    verify(wasm, plan['wasm'])
    data = Path(wasm).read_bytes()
    if data[:8] != b'\0asm\x01\0\0\0':
        raise ValueError('Pinned Caddy input must be a WebAssembly module')
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='caddy-wasm-pack-', dir=output.parent) as temporary:
        stage = Path(temporary)
        members = []
        def add(destination, content):
            path = stage / safe_path(destination)
            if path.exists():
                raise ValueError('Duplicate Caddy pack destination')
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(content)
            members.append(dict(path=destination, archive_path=destination, bytes=len(content),
                                sha256=hashlib.sha256(content).hexdigest(), executable=False))
        add(plan['wasm']['path'], data)
        for path, pin in sorted(plan['source_files'].items()):
            verify(root / safe_path(path), pin)
            add('sources/' + path, (root / path).read_bytes())
        for asset in plan['pack_assets']:
            path = root / safe_path(asset['source'])
            verify(path, asset)
            add(asset['path'], path.read_bytes())
        add('sources/resources/caddy-formatter-sources.json', Path(sources).read_bytes())
        provenance = dict(format_version=2, toolchain='caddyfmt', version=plan['version'],
                          target='universal-wasm', requires_dprint_pack=plan['requires_dprint_pack'],
                          upstream_version=plan['upstream_version'], source_commit=plan['source_commit'],
                          source_url=plan['source_url'], build=plan['build'],
                          wasm_plugins=[plan['wasm']['path']], files=members.copy(),
                          invocation=['fmt', '--config-discovery=false', '--skip-stable-format', '--stdin=Caddyfile'])
        add('PROVENANCE.json', (json.dumps(provenance, indent=2, sort_keys=True) + '\n').encode())
        write_bundle(stage, output, members)
    size = output.stat().st_size
    installed = sum(m['bytes'] for m in members)
    return dict(available=False,
                reason='Unpublished formatter-only Caddy WASM candidate; requires managed dprint, native/oldest-OS validation, final license audit and release approval.',
                kind='wasm', requires_dprint_pack=plan['requires_dprint_pack'],
                wasm_plugins=[plan['wasm']['path']], environment={},
                validation={'license_audit': 'upstream-and-runtime-notices-included; final-target-feature-review-pending'},
                max_installed_bytes=installed,
                archives=[dict(format='tar.gz', asset_name=output.name, sha256=digest_file(output), bytes=size,
                               max_download_bytes=size, max_expanded_bytes=installed + len(members)*1024+10240,
                               members=members)])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--wasm', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--candidate', type=Path, required=True)
    args = parser.parse_args()
    candidate = build(args.wasm, args.output)
    args.candidate.write_text(json.dumps(candidate, indent=2)+'\n')


if __name__ == '__main__':
    main()
