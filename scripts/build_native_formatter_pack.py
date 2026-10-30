#!/usr/bin/env python3
"""Offline Linux x64 Go/Dart candidate reproduction; never downloads or executes.

Only resources/native-formatter-sources.json pins are accepted by the CLI.
1. Acquire that manifest's official source archive and supplemental license URLs
   under separately granted download authority (no arbitrary URLs in this tool).
2. `extract --tool gofmt|dartfmt --archive FILE --inputs EMPTY_DIRECTORY` checks
   the exact archive SHA/bytes and extracts only pinned regular selected members.
3. Put supplementary license files at their listed input_path under --inputs;
   their bytes/SHA are independently checked. Generated audit notices are internal.
4. `build --tool ... --inputs DIRECTORY --output FILE --candidate FILE
   --work-dir DIRECTORY` rebuilds the exact previously tested candidate digest.

Alternatively build from an existing exact pinned input tree. Other platforms
are metadata only. Dart third-party license auditing remains incomplete; neither
candidate is published or release-ready. No shell, compiler or SDK tool is run.
"""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import stat
import tarfile
import tempfile
import zipfile

from build_language_pack import ROOT, digest_file, safe_path, write_bundle

SOURCES = ROOT / 'resources/native-formatter-sources.json'
TARGET = 'x86_64-unknown-linux-gnu'


def regular(path):
    mode = path.lstat().st_mode
    if not stat.S_ISREG(mode) or path.is_symlink():
        raise ValueError('Expected regular non-symlink input')


def checked_input(root, name):
    safe_path(name)
    root = root.absolute()
    if root.is_symlink() or not root.is_dir():
        raise ValueError('Invalid input root')
    current = root
    for component in Path(name).parts:
        current = current / component
        if current.is_symlink():
            raise ValueError('Symlink input component')
    regular(current)
    return current


def verify_file(path, pin):
    regular(path)
    if path.stat().st_size != pin['bytes'] or digest_file(path) != pin['sha256']:
        raise ValueError('Pinned input hash/size mismatch')


def validate_plan(plan):
    if plan['target'] != TARGET or plan['candidate']['available'] is not False:
        raise ValueError('Only unpublished Linux x64 candidates are supported')
    paths = set()
    for pin in plan['files']:
        safe_path(pin['path']); safe_path(pin['input_path'])
        if pin['path'] in paths or pin['archive_path'] != pin['path']:
            raise ValueError('Duplicate or mismatched candidate destination')
        paths.add(pin['path'])
        if pin['origin'] not in ('archive', 'supplement', 'generated'):
            raise ValueError('Unsupported input origin')
    if 'PROVENANCE.json' in paths:
        raise ValueError('Reserved provenance destination')


def extract(plan, archive, inputs):
    validate_plan(plan)
    verify_file(archive, plan['source_receipt'])
    # Refuse adoption of existing trees; a failed extraction removes only our
    # newly created staging tree, never unrelated caller data.
    if inputs.exists():
        raise ValueError('Extraction destination must not already exist')
    inputs.parent.mkdir(parents=True, exist_ok=True)
    wanted = {pin['source_archive_path']: pin for pin in plan['files'] if pin['origin'] == 'archive'}
    if len(wanted) != sum(p['origin'] == 'archive' for p in plan['files']):
        raise ValueError('Duplicate source archive entry')
    seen = set()
    with tempfile.TemporaryDirectory(prefix='native-inputs-', dir=inputs.parent) as temporary:
        stage = Path(temporary) / 'inputs'; stage.mkdir()
        def copy(name, size, source):
            pin = wanted[name]
            if name in seen or size != pin['bytes']:
                raise ValueError('Duplicate member or wrong member size')
            seen.add(name)
            destination = stage / safe_path(pin['input_path'])
            destination.parent.mkdir(parents=True, exist_ok=True)
            with source, destination.open('xb') as output:
                shutil.copyfileobj(source, output, length=1024 * 1024)
            verify_file(destination, pin)
        total = 0
        if plan['archive_format'] == 'tar.gz':
            with tarfile.open(archive, 'r|gz') as tar:
                for member in tar:
                    total += 512 + ((member.size + 511) // 512) * 512
                    if total > plan['max_source_expanded_bytes']:
                        raise ValueError('Source expansion limit exceeded')
                    if member.name in wanted:
                        if not member.isfile():
                            raise ValueError('Selected tar member is not regular')
                        copy(member.name, member.size, tar.extractfile(member))
        elif plan['archive_format'] == 'zip':
            with zipfile.ZipFile(archive) as zip:
                for member in zip.infolist():
                    total += member.file_size
                    if total > plan['max_source_expanded_bytes']:
                        raise ValueError('Source expansion limit exceeded')
                    if member.filename in wanted:
                        mode = stat.S_IFMT(member.external_attr >> 16)
                        if member.is_dir() or mode not in (0, stat.S_IFREG):
                            raise ValueError('Selected ZIP member is not regular')
                        copy(member.filename, member.file_size, zip.open(member))
        else:
            raise ValueError('Unsupported fixed archive format')
        if seen != set(wanted):
            raise ValueError('Missing pinned source members')
        stage.rename(inputs)


def build(plan, tool, inputs, output, candidate, work_dir):
    validate_plan(plan)
    if output.resolve() == candidate.resolve():
        raise ValueError('Bundle and candidate paths must differ')
    if output.resolve().is_relative_to(inputs.resolve()) or candidate.resolve().is_relative_to(inputs.resolve()):
        raise ValueError('Outputs must be outside the pinned input tree')
    if output.exists() or candidate.exists() or output.with_name(output.name + '.part').exists():
        raise ValueError('Refusing to replace existing candidate artifacts')
    work_dir.mkdir(parents=True, exist_ok=True)
    members = []
    with tempfile.TemporaryDirectory(prefix='native-build-', dir=work_dir) as temporary:
        stage = Path(temporary)
        for pin in sorted(plan['files'], key=lambda value: value['path']):
            destination = stage / safe_path(pin['path'])
            destination.parent.mkdir(parents=True, exist_ok=True)
            if pin['origin'] == 'generated':
                destination.write_bytes(pin['text'].encode('utf-8'))
            else:
                source = checked_input(inputs, pin['input_path'])
                verify_file(source, pin)
                shutil.copyfile(source, destination)
            verify_file(destination, pin)
            members.append({k: pin[k] for k in ('archive_path', 'path', 'sha256', 'bytes', 'executable')})
        receipt = {'format_version': 1, 'toolchain': tool, 'version': plan['version'],
                   'target': TARGET, 'source': plan['source_receipt'], 'files': members.copy(),
                   'license_audit': plan['candidate']['validation']['license_audit']}
        path = stage / 'PROVENANCE.json'
        path.write_bytes((json.dumps(receipt, indent=2, sort_keys=True) + '\n').encode())
        members.append({'archive_path': path.name, 'path': path.name, 'sha256': digest_file(path),
                        'bytes': path.stat().st_size, 'executable': False})
        installed = sum(m['bytes'] for m in members)
        if installed != plan['candidate']['max_installed_bytes']:
            raise ValueError('Installed candidate byte count changed')
        try:
            write_bundle(stage, output, members)
            if (digest_file(output) != plan['expected_bundle_sha256']
                    or output.stat().st_size != plan['expected_bundle_bytes']):
                raise ValueError('Rebuilt candidate differs from the previously verified bundle')
            spec = dict(plan['candidate'])
            spec['archives'] = [{'format': 'tar.gz', 'asset_name': output.name,
                                 'sha256': digest_file(output), 'bytes': output.stat().st_size,
                                 'max_download_bytes': output.stat().st_size,
                                 'max_expanded_bytes': installed + len(members) * 1024 + 10240,
                                 'members': members}]
            candidate.parent.mkdir(parents=True, exist_ok=True)
            candidate.write_bytes((json.dumps({'version': plan['version'], 'targets': {TARGET: spec}}, indent=2) + '\n').encode())
        except BaseException:
            output.unlink(missing_ok=True)
            candidate.unlink(missing_ok=True)
            raise


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('action', choices=('extract', 'build'))
    parser.add_argument('--tool', required=True, choices=('gofmt', 'dartfmt'))
    parser.add_argument('--inputs', required=True, type=Path)
    parser.add_argument('--archive', type=Path)
    parser.add_argument('--output', type=Path)
    parser.add_argument('--candidate', type=Path)
    parser.add_argument('--work-dir', type=Path)
    args = parser.parse_args()
    sources = json.loads(SOURCES.read_text())
    if sources['format_version'] != 1:
        raise ValueError('Unsupported source manifest')
    plan = sources['tools'][args.tool]
    if args.action == 'extract':
        if args.archive is None:
            parser.error('extract requires --archive')
        extract(plan, args.archive, args.inputs)
    else:
        if any(value is None for value in (args.output, args.candidate, args.work_dir)):
            parser.error('build requires --output, --candidate and --work-dir')
        build(plan, args.tool, args.inputs, args.output, args.candidate, args.work_dir)


if __name__ == '__main__':
    main()
