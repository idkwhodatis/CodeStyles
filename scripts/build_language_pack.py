#!/usr/bin/env python3
"""Build deterministic rustfmt bundles from reviewed official component pins.

Build time only: static.rust-lang.org is NOT an application download endpoint.
No installer, rustup, cargo, executable or downloaded script is run. The final
bundle is a candidate for explicitly approved publication on GitHub.
"""
import argparse
import gzip
import hashlib
import io
import json
from pathlib import Path, PurePosixPath
import shutil
import tarfile
import tempfile
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
EPOCH = 1577836800


def digest_file(path):
    digest = hashlib.sha256()
    with path.open('rb') as file:
        while chunk := file.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def safe_path(name):
    path = PurePosixPath(name)
    if not name or path.is_absolute() or '..' in path.parts or '\\' in name or ':' in name or str(path) != name:
        raise ValueError(f'Unsafe member path: {name}')
    return path


def extract_component(archive_path, archive, stage):
    wanted = {m['archive_path']: m for m in archive['members']}
    if len(wanted) != len(archive['members']):
        raise ValueError('Duplicate source paths')
    found = set()
    expanded = 0
    with tarfile.open(archive_path, 'r:xz') as tar:
        for entry in tar:
            expanded += ((entry.size + 511) // 512 + 1) * 512
            if expanded > archive['max_expanded_bytes']:
                raise ValueError('Archive expansion bound exceeded')
            if entry.name not in wanted:
                continue
            if entry.name in found or not entry.isfile():
                raise ValueError('Duplicate or nonregular allowlisted member')
            found.add(entry.name)
            member = wanted[entry.name]
            destination = stage / safe_path(member['path'])
            if destination.exists() or entry.size != member['bytes']:
                raise ValueError('Conflicting output or unexpected member size')
            destination.parent.mkdir(parents=True, exist_ok=True)
            with tar.extractfile(entry) as source, destination.open('xb') as output:
                shutil.copyfileobj(source, output)
            if digest_file(destination) != member['sha256']:
                raise ValueError(f'Member checksum mismatch: {entry.name}')
            destination.chmod(0o755 if member['executable'] else 0o644)
    if found != set(wanted):
        raise ValueError('Missing allowlisted archive member')


def write_bundle(stage, output, members):
    output.parent.mkdir(parents=True, exist_ok=True)
    temporary = output.with_name(output.name + '.part')
    try:
        with temporary.open('wb') as raw, gzip.GzipFile(filename='', fileobj=raw, mode='wb', mtime=EPOCH, compresslevel=9) as compressed:
            with tarfile.open(fileobj=compressed, mode='w|', format=tarfile.USTAR_FORMAT) as tar:
                for member in sorted(members, key=lambda m: m['path']):
                    name = member['path']
                    path = stage / safe_path(name)
                    info = tarfile.TarInfo(name)
                    info.size = path.stat().st_size
                    info.mode = 0o755 if member['executable'] else 0o644
                    info.mtime = EPOCH
                    info.uid = info.gid = 0
                    info.uname = info.gname = ''
                    with path.open('rb') as source:
                        tar.addfile(info, source)
        temporary.replace(output)
    finally:
        temporary.unlink(missing_ok=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--sources', type=Path, default=ROOT / 'resources/toolchain-sources.json')
    parser.add_argument('--target', required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--candidate', type=Path, required=True)
    parser.add_argument('--work-dir', type=Path, help='Temporary staging parent with enough disk space')
    args = parser.parse_args()
    manifest = json.loads(args.sources.read_text())
    tool = manifest['toolchains']['rustfmt']
    target = tool['targets'][args.target]
    if not target['available']:
        raise ValueError('Source target is unverified; refusing to build')
    if args.work_dir is not None:
        args.work_dir.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='codestyles-toolchain-build-', dir=args.work_dir) as temporary:
        root = Path(temporary)
        stage = root / 'stage'
        stage.mkdir()
        members = []
        for archive in target['archives']:
            if not archive['url'].startswith('https://static.rust-lang.org/dist/'):
                raise ValueError('Unexpected upstream source host')
            path = root / 'component.tar.xz'
            print('Fetching verified build input:', archive['url'], flush=True)
            with urllib.request.urlopen(archive['url'], timeout=180) as response, path.open('wb') as dest:
                size = 0
                while chunk := response.read(1024 * 1024):
                    size += len(chunk)
                    if size > archive['max_download_bytes']:
                        raise ValueError('Compressed component size limit exceeded')
                    dest.write(chunk)
            if size != archive['bytes'] or digest_file(path) != archive['sha256']:
                raise ValueError('Official source archive checksum/size mismatch')
            extract_component(path, archive, stage)
            path.unlink()
            members.extend({**m, 'archive_path': m['path']} for m in archive['members'])
        receipt = {'format_version': 1, 'toolchain': 'rustfmt', 'version': tool['version'],
                   'target': args.target, 'channel_manifest': tool['channel_manifest'],
                   'source_archives': [{'url': a['url'], 'sha256': a['sha256']} for a in target['archives']],
                   'files': members}
        receipt_path = stage / 'PROVENANCE.json'
        receipt_path.write_text(json.dumps(receipt, indent=2, sort_keys=True) + '\n')
        members.append({'path': 'PROVENANCE.json', 'archive_path': 'PROVENANCE.json', 'bytes': receipt_path.stat().st_size,
                        'sha256': digest_file(receipt_path), 'executable': False})
        installed = sum(m['bytes'] for m in members)
        if installed > target['max_installed_bytes'] + 1024 * 1024:
            raise ValueError('Installed payload size bound exceeded')
        write_bundle(stage, args.output, members)
        candidate = {'available': False, 'reason': 'Unpublished local build candidate; GitHub release approval required.',
                     'executable': target['executable'], 'environment': target.get('environment', {}),
                     'max_installed_bytes': installed, 'validation': target['validation'],
                     'archives': [{'format': 'tar.gz', 'asset_name': args.output.name,
                                   'sha256': digest_file(args.output), 'bytes': args.output.stat().st_size,
                                   'max_download_bytes': args.output.stat().st_size,
                                   'max_expanded_bytes': installed + len(members) * 1024 + 10240,
                                   'members': members}]}
        args.candidate.write_text(json.dumps(candidate, indent=2) + '\n')
        print('Built unpublished candidate:', args.output, candidate['archives'][0]['sha256'], flush=True)


if __name__ == '__main__':
    main()
