#!/usr/bin/env python3
"""Build unpublished, deterministic optional WASM packs from pinned local bytes.

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


PACKS.update({'shfmt': {'version': 'shfmt-0.1.2',
           'upstream_versions': {'shfmt': '0.1.2', 'mvdan/sh': '3.14.1'},
           'licenses': {'shfmt': 'MIT', 'mvdan/sh': 'BSD-3-Clause'},
           'wasm_plugins': ['plugins/shfmt-0.1.2.wasm'],
           'assets': [('source',
                       'shfmt-0.1.2.wasm',
                       'plugins/shfmt-0.1.2.wasm',
                       543316,
                       'd9838fab9fe96ce455f73ffdcaf877cab3ec76350c771babe249da4a03487569',
                       'https://github.com/dprint/dprint-plugin-shfmt/releases/download/0.1.2/plugin.wasm'),
                      ('resources',
                       'schemas/shfmt-0.1.2.json',
                       'schemas/shfmt-0.1.2.json',
                       2513,
                       'fc19f09f2d2e9d57c76494c73d75afc1502f5707f198b2260c85b2ad05c98780',
                       'https://github.com/dprint/dprint-plugin-shfmt/releases/download/0.1.2/schema.json'),
                      ('resources',
                       'licenses/shfmt-0.1.2.txt',
                       'licenses/shfmt-0.1.2.txt',
                       3807,
                       'e0a8657e7764445e84c87e1758a735bbabcc74a4d8dba106f24fe999c73ee654',
                       'https://raw.githubusercontent.com/dprint/dprint-plugin-shfmt/ac5b9026bea861356e50311e1620500611c57c3b/LICENSE')]},
 'stylua': {'version': 'stylua-0.1.0',
            'upstream_versions': {'stylua': '0.1.0', 'StyLua': '2.6.0'},
            'licenses': {'stylua': 'MIT', 'StyLua': 'MPL-2.0'},
            'wasm_plugins': ['plugins/stylua-0.1.0.wasm'],
            'assets': [('source',
                        'stylua-0.1.0.wasm',
                        'plugins/stylua-0.1.0.wasm',
                        4054628,
                        '081604892f6ce13e7bb8cbc616f56cb93e9b0a76cf2c401d5ff494cccd3d25bf',
                        'https://github.com/dprint/dprint-plugin-stylua/releases/download/0.1.0/plugin.wasm'),
                       ('resources',
                        'schemas/stylua-0.1.0.json',
                        'schemas/stylua-0.1.0.json',
                        5304,
                        'bf7833c01f41db46c9a165b3527d4d41aa4d7cfc04d207520798e582aaec9246',
                        'https://github.com/dprint/dprint-plugin-stylua/releases/download/0.1.0/schema.json'),
                       ('resources',
                        'licenses/stylua-0.1.0-SOURCE.txt',
                        'licenses/stylua-0.1.0-SOURCE.txt',
                        567,
                        'ffca0921cce6dd15f3940c12f6faef4dbc94b92d09e3aa2aaffa8b98d3477efd',
                        'https://github.com/JohnnyMorganz/StyLua/tree/56f9630407f6bf6ee524fb1d7fe71f005a570478'),
                       ('resources',
                        'licenses/stylua-0.1.0.txt',
                        'licenses/stylua-0.1.0.txt',
                        1139,
                        'f220330d3736fb4c0c5f4a361b7e926b08576709a5648a59b14b692e8fa3b598',
                        'https://raw.githubusercontent.com/dprint/dprint-plugin-stylua/e1979d8137502523387aae472ae74dc599f8c49f/LICENSE'),
                       ('resources',
                        'licenses/stylua-upstream-2.6.0-MPL-2.0.txt',
                        'licenses/stylua-upstream-2.6.0-MPL-2.0.txt',
                        16724,
                        '4b89d4518bd135ab4ee154a7bce722246b57a98c3d7efc1a09409898160c2bd1',
                        'https://raw.githubusercontent.com/JohnnyMorganz/StyLua/56f9630407f6bf6ee524fb1d7fe71f005a570478/LICENSE.md')]},
 'dockerfile': {'version': 'dockerfile-0.7.0',
                'upstream_versions': {'dockerfile': '0.7.0'},
                'licenses': {'dockerfile': 'MIT'},
                'wasm_plugins': ['plugins/dockerfile-0.7.0.wasm'],
                'assets': [('source',
                            'dockerfile-0.7.0.wasm',
                            'plugins/dockerfile-0.7.0.wasm',
                            466818,
                            '04e0bef5fb60355247da7937af0df9641bf93922c60aaa19cc94ca141e293d1d',
                            'https://github.com/dprint/dprint-plugin-dockerfile/releases/download/0.7.0/plugin.wasm'),
                           ('resources',
                            'schemas/dockerfile-0.7.0.json',
                            'schemas/dockerfile-0.7.0.json',
                            1902,
                            'a67adb251b7135a612b06b30b5f9cac8de16845e74786f6fc99b36667d2573bc',
                            'https://github.com/dprint/dprint-plugin-dockerfile/releases/download/0.7.0/schema.json'),
                           ('resources',
                            'licenses/dockerfile-0.7.0.txt',
                            'licenses/dockerfile-0.7.0.txt',
                            1080,
                            '9039b024904a2a6f54c8594647d8f8e16029f9b6c0067ca345f3c59a2e033674',
                            'https://raw.githubusercontent.com/dprint/dprint-plugin-dockerfile/747f648275d02b19fcc20504ea5c9f8360e83ac7/LICENSE')]}})

def build(toolchain, source_dir, output, resources=RESOURCES):
    if toolchain not in PACKS:
        raise ValueError('Only reviewed pinned WASM packs are supported')
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
        if toolchain in ('shfmt', 'stylua', 'dockerfile'):
            receipts = json.loads((RESOURCES / 'script-formatter-sources.json').read_text())
            provenance['upstream_receipt'] = next(p for p in receipts['plugins'] if p['plugin'] == toolchain)
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
                validation={'license_audit': 'upstream-notices-included; transitive-target-feature-audit-pending'},
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
