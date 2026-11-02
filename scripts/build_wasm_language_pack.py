#!/usr/bin/env python3
"""Build unpublished, deterministic YAML/SQL WASM packs from pinned local bytes.

Offline only; does not download or execute input. The managed dprint engine is a
separate prerequisite, never copied into these universal data-only packs.
Schemas/licenses remain available offline in the app and accompany each pack.
"""
import argparse
import hashlib
import json
from pathlib import Path
import tempfile

from build_language_pack import digest_file, write_bundle
from build_vue_pack import pinned_bytes

REQUIRES_DPRINT_PACK = '0.63.0-p1'
RESOURCES = Path(__file__).resolve().parent.parent / 'resources'
# (source root, relative source path, installed path, bytes, sha256, upstream URL)
PACKS = {
    'yaml': {
        'version': 'pretty_yaml-0.6.0',
        'upstream_versions': {'pretty_yaml': '0.6.0'},
        'licenses': {'pretty_yaml': 'MIT'},
        'wasm_plugins': ['plugins/pretty_yaml-0.6.0.wasm'],
        'assets': [
            ('source', 'pretty_yaml-0.6.0.wasm', 'plugins/pretty_yaml-0.6.0.wasm', 576981,
             '40a2fdda7040317eb1b23520f3a00769a5571eedb049c4ca9175c1b9eeba01ae',
             'https://plugins.dprint.dev/g-plane/pretty_yaml-v0.6.0.wasm'),
            ('resources', 'schemas/pretty_yaml-0.6.0.json', 'schemas/pretty_yaml-0.6.0.json', 4734,
             'efb75d9eda299ae13a6af508e5579d9bec9a14654d986de470847605d1df1e29',
             'https://plugins.dprint.dev/g-plane/pretty_yaml/v0.6.0/schema.json'),
            ('resources', 'licenses/pretty_yaml-0.6.0.txt', 'licenses/pretty_yaml-0.6.0.txt', 1073,
             '2efe0331e360cdaa2701ae0b42fa1ce6002ee52675d648a0c49029b5f27eb734',
             'https://raw.githubusercontent.com/g-plane/pretty_yaml/v0.6.0/LICENSE'),
        ],
    },
    'sql': {
        'version': 'sql-0.4.1',
        'upstream_versions': {'sql': '0.4.1'},
        'licenses': {'sql': 'MIT'},
        'wasm_plugins': ['plugins/sql-0.4.1.wasm'],
        'assets': [
            ('source', 'sql-0.4.1.wasm', 'plugins/sql-0.4.1.wasm', 660116,
             '148784d024138e1bca88cffc05a67cc3ce180b5a24170fe79df31e8b6dc3a410',
             'https://plugins.dprint.dev/sql-0.4.1.wasm'),
            ('resources', 'schemas/sql-0.4.1.json', 'schemas/sql-0.4.1.json', 3826,
             'ca9b08f7e3a0dd5cde169027bd0dcd8c81b9cb29cc3044ac2ab63885c6e191d6',
             'https://plugins.dprint.dev/dprint/dprint-plugin-sql/0.4.1/schema.json'),
            ('resources', 'licenses/sql-0.4.1.txt', 'licenses/sql-0.4.1.txt', 1080,
             '9039b024904a2a6f54c8594647d8f8e16029f9b6c0067ca345f3c59a2e033674',
             'https://raw.githubusercontent.com/dprint/dprint-plugin-sql/0.4.1/LICENSE'),
        ],
    },
}


def build(toolchain, source_dir, output, resources=RESOURCES):
    if toolchain not in PACKS:
        raise ValueError('Only reviewed YAML and SQL packs are supported')
    pack = PACKS[toolchain]
    roots = {'source': Path(source_dir), 'resources': Path(resources)}
    output = Path(output)
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='wasm-language-pack-', dir=output.parent) as temporary:
        stage = Path(temporary)
        members, origins = [], []
        for root, name, destination, size, digest, url in pack['assets']:
            data = pinned_bytes(roots[root] / name, size, digest)
            path = stage / destination
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)
            members.append(dict(path=destination, archive_path=destination, bytes=size,
                                sha256=digest, executable=False))
            origins.append(dict(path=destination, url=url, bytes=size, sha256=digest))
        provenance = dict(format_version=1, toolchain=toolchain, version=pack['version'],
                          target='universal-wasm', requires_dprint_pack=REQUIRES_DPRINT_PACK,
                          upstream_versions=pack['upstream_versions'], licenses=pack['licenses'],
                          wasm_plugins=pack['wasm_plugins'], files=members.copy(), sources=origins)
        data = (json.dumps(provenance, indent=2, sort_keys=True) + '\n').encode('utf-8')
        (stage / 'PROVENANCE.json').write_bytes(data)
        members.append(dict(path='PROVENANCE.json', archive_path='PROVENANCE.json', bytes=len(data),
                            sha256=hashlib.sha256(data).hexdigest(), executable=False))
        write_bundle(stage, output, members)
    installed = sum(member['bytes'] for member in members)
    size = output.stat().st_size
    return dict(available=False,
                reason='Unpublished universal WASM candidate; native validation, transitive license audit and release approval required.',
                kind='wasm', requires_dprint_pack=REQUIRES_DPRINT_PACK,
                validation={'license_audit': 'upstream-MIT-notices-included; transitive-target-feature-audit-pending'},
                wasm_plugins=pack['wasm_plugins'].copy(), environment={}, max_installed_bytes=installed,
                archives=[dict(format='tar.gz', asset_name=output.name, sha256=digest_file(output),
                               bytes=size, max_download_bytes=size,
                               max_expanded_bytes=installed + len(members) * 1024 + 10240, members=members)])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--toolchain', choices=PACKS, required=True)
    parser.add_argument('--source-dir', type=Path, required=True)
    parser.add_argument('--resources', type=Path, default=RESOURCES)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--candidate', type=Path, required=True)
    args = parser.parse_args()
    candidate = build(args.toolchain, args.source_dir, args.output, args.resources)
    args.candidate.write_bytes((json.dumps(candidate, indent=2) + '\n').encode('utf-8'))
    print(json.dumps({'version': PACKS[args.toolchain]['version'],
                      'sha256': candidate['archives'][0]['sha256'],
                      'bytes': candidate['archives'][0]['bytes']}))


if __name__ == '__main__':
    main()
