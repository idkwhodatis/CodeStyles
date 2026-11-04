#!/usr/bin/env python3
"""Offline reproducible repacking of exact official Caddy archives; never executes.

Only fmt - is exposed by CodeStyles. Full upstream binaries still contain server
functionality. All candidates remain disabled pending native/license release gates.
"""
import argparse
import hashlib
import json
from pathlib import Path
import stat
import tarfile
import tempfile
import zipfile
from build_language_pack import digest_file, write_bundle
from build_native_formatter_pack import verify_file
RESOURCES = Path(__file__).resolve().parent.parent / 'resources'
SOURCES = RESOURCES / 'caddy-formatter-sources.json'
MAX_MEMBER_BYTES = 128 * 1024 * 1024


def build(target, archive, output, sources=SOURCES):
    plan = json.loads(Path(sources).read_text())
    pin = plan['targets'][target]
    archive, output = Path(archive), Path(output)
    verify_file(archive, pin)
    wanted = {pin['binary']: 'bin/' + pin['binary'], 'LICENSE': 'licenses/caddy-2.11.7.txt'}
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='caddy-pack-', dir=output.parent) as temporary:
        stage = Path(temporary)
        members, seen = [], set()
        def copy(name, size, data):
            if name in seen or size <= 0 or size > MAX_MEMBER_BYTES:
                raise ValueError('Duplicate, empty or oversized selected archive member')
            seen.add(name)
            content = data.read(MAX_MEMBER_BYTES + 1)
            if len(content) != size:
                raise ValueError('Incorrect archive member size')
            destination = wanted[name]
            file = stage / destination
            file.parent.mkdir(parents=True, exist_ok=True)
            file.write_bytes(content)
            members.append(dict(path=destination, archive_path=destination, bytes=size,
                sha256=hashlib.sha256(content).hexdigest(), executable=name == pin['binary']))
        if pin['archive_format'] == 'tar.gz':
            with tarfile.open(archive, 'r:gz') as source:
                for item in source:
                    if item.name in wanted:
                        if not item.isfile():
                            raise ValueError('Selected archive member must be regular')
                        with source.extractfile(item) as stream:
                            copy(item.name, item.size, stream)
        else:
            with zipfile.ZipFile(archive) as source:
                for item in source.infolist():
                    if item.filename in wanted:
                        mode = item.external_attr >> 16
                        if item.is_dir() or (stat.S_IFMT(mode) not in (0, stat.S_IFREG)):
                            raise ValueError('Selected archive member must be regular')
                        with source.open(item) as stream:
                            copy(item.filename, item.file_size, stream)
        if seen != set(wanted):
            raise ValueError('Missing selected binary or license')
        members.sort(key=lambda m: m['path'])
        provenance = dict(format_version=1, toolchain='caddyfmt', version=plan['version'],
            target=target, upstream_version=plan['upstream_version'], source_url=plan['source_url'],
            license=plan['license'], archive=pin, files=members.copy(), invocation=['fmt', '-'])
        data = (json.dumps(provenance, sort_keys=True, indent=2) + '\n').encode()
        (stage / 'PROVENANCE.json').write_bytes(data)
        members.append(dict(path='PROVENANCE.json', archive_path='PROVENANCE.json', bytes=len(data),
            sha256=hashlib.sha256(data).hexdigest(), executable=False))
        write_bundle(stage, output, members)
    size = output.stat().st_size
    installed = sum(m['bytes'] for m in members)
    return dict(available=False, reason='Unpublished Caddy formatter candidate; native platform validation, transitive license audit and release approval required.',
        kind='native', executable='bin/' + pin['binary'], environment={},
        validation={'license_audit': 'upstream-Apache-2.0-included; transitive-audit-pending'},
        max_installed_bytes=installed,
        archives=[dict(format='tar.gz', asset_name=output.name, sha256=digest_file(output), bytes=size,
            max_download_bytes=size, max_expanded_bytes=installed + len(members)*1024+10240, members=members)])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--target', required=True)
    parser.add_argument('--archive', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--candidate', type=Path, required=True)
    args = parser.parse_args()
    args.candidate.write_text(json.dumps(build(args.target,args.archive,args.output),indent=2)+'\n')


if __name__ == '__main__':
    main()
