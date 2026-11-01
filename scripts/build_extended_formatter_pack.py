#!/usr/bin/env python3
"""Offline, deterministic Java/C#/C++ candidate packaging from fixed member pins.

Never downloads, executes, publishes, or edits the application catalog. An absent
reviewed plan fails closed. Upstream release metadata alone is not a pack plan.
"""
import argparse
import json
import os
from pathlib import Path
import re
import shutil
import tempfile
import unicodedata

from build_language_pack import ROOT, digest_file, safe_path, write_bundle
from build_native_formatter_pack import checked_input, verify_file

SOURCES = ROOT / 'resources/extended-formatter-sources.json'
TARGETS = ('x86_64-unknown-linux-gnu', 'aarch64-unknown-linux-gnu',
           'x86_64-apple-darwin', 'aarch64-apple-darwin',
           'x86_64-pc-windows-msvc', 'aarch64-pc-windows-msvc')
TOOLS = ('google-java-format', 'csharpier', 'clang-format')
MAX_INSTALLED = 512 * 1024 * 1024


def validate_plan(tool, target, plan):
    if tool not in TOOLS or target not in TARGETS:
        raise ValueError('Unsupported formatter or target')
    expected_executable = ('dotnet/dotnet' if tool == 'csharpier' else f'bin/{tool}') + ('.exe' if 'windows' in target else '')
    if plan.get('executable') != expected_executable:
        raise ValueError('Expected direct managed native executable; no SDK shim')
    if plan.get('available') is not False or plan.get('environment', {}) != {}:
        raise ValueError('Only isolated unpublished candidates are supported')
    files = plan['files']
    if not files or len(files) > 4096:
        raise ValueError('Missing or excessive fixed members')
    seen = set()
    portable_paths = {'provenance.json'}
    total = 0
    for pin in files:
        path = pin['path']; safe_path(path); safe_path(pin['input_path'])
        if path in seen:
            raise ValueError('Duplicate or reserved candidate path')
        seen.add(path)
        folded = unicodedata.normalize('NFC', path).casefold()
        if (folded in portable_paths
                or any(folded.startswith(p + '/') or p.startswith(folded + '/') for p in portable_paths)):
            raise ValueError('Case-insensitive collision or file/directory prefix conflict')
        portable_paths.add(folded)
        for part in path.split('/'):
            stem = part.split('.')[0].upper()
            if (len(part.encode('utf-8')) > 255 or part.endswith(('.', ' '))
                    or any(ord(c) < 32 or ord(c) == 127 or c in '<>"|?*' for c in part)
                    or stem in ('CON', 'PRN', 'AUX', 'NUL')
                    or re.fullmatch(r'(COM|LPT)[0-9]', stem)):
                raise ValueError('Nonportable or reserved candidate path component')
        if (type(pin['bytes']) is not int or pin['bytes'] < 0
                or not re.fullmatch('[0-9a-f]{64}', pin['sha256'])
                or type(pin['executable']) is not bool):
            raise ValueError('Invalid member pin')
        total += pin['bytes']
    if total > MAX_INSTALLED:
        raise ValueError('Candidate exceeds size bound')
    if not any(p['path'] == expected_executable and p['executable'] for p in files):
        raise ValueError('Pinned executable is missing')
    if tool == 'csharpier':
        hostfxr = 'hostfxr.dll' if 'windows' in target else ('libhostfxr.dylib' if 'apple' in target else 'libhostfxr.so')
        required = {'lib/csharpier/CSharpier.dll', 'lib/csharpier/CSharpier.deps.json',
                    'lib/csharpier/CSharpier.runtimeconfig.json',
                    'dotnet/shared/Microsoft.NETCore.App/10.0.14/System.Private.CoreLib.dll',
                    'dotnet/host/fxr/10.0.14/' + hostfxr}
        if not required.issubset(seen):
            raise ValueError('Fixed managed .NET 10.0.14/CSharpier payload is missing')
    if not any(p['path'].startswith('licenses/') and not p['executable'] for p in files):
        raise ValueError('Pinned license/audit notices are required')
    if not isinstance(plan.get('license_audit'), str) or not plan['license_audit']:
        raise ValueError('Explicit license audit status required')


def build(tool, target, version, plan, inputs, output, candidate, work_dir):
    validate_plan(tool, target, plan)
    if output.resolve() == candidate.resolve():
        raise ValueError('Bundle and receipt paths must differ')
    if any(p.resolve().is_relative_to(inputs.resolve()) for p in (output, candidate)):
        raise ValueError('Outputs must be outside the input tree')
    if any(p.exists() or p.is_symlink() for p in (output, candidate, output.with_name(output.name + '.part'))):
        raise ValueError('Refusing to replace existing artifacts')
    work_dir.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='extended-pack-', dir=work_dir) as tmp:
        stage = Path(tmp); members = []
        for pin in sorted(plan['files'], key=lambda p: p['path']):
            source = checked_input(inputs, pin['input_path']); verify_file(source, pin)
            destination = stage / safe_path(pin['path']); destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(source, destination); verify_file(destination, pin)
            members.append({**{k: pin[k] for k in ('path', 'sha256', 'bytes', 'executable')},
                            'archive_path': pin['path']})
        provenance = {'format_version': 1, 'toolchain': tool, 'version': version,
                      'target': target, 'files': members.copy(),
                      'license_audit': plan['license_audit'],
                      'source_receipts': plan.get('source_receipts', []),
                      'validation': 'member-hashes-only; no native execution or redistribution acceptance'}
        path = stage / 'PROVENANCE.json'
        path.write_text(json.dumps(provenance, indent=2, sort_keys=True) + '\n')
        members.append({'path': path.name, 'archive_path': path.name,
                        'sha256': digest_file(path), 'bytes': path.stat().st_size, 'executable': False})
        installed = sum(m['bytes'] for m in members)
        # Never let write_bundle's historical predictable .part/replace behavior
        # touch caller paths. Both private staging directories are siblings of
        # their final path, so hard-link publication is same-filesystem and
        # atomic no-replace (including when another build wins the race).
        output.parent.mkdir(parents=True, exist_ok=True)
        candidate.parent.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(prefix='.extended-bundle-', dir=output.parent) as bundle_tmp, \
                tempfile.TemporaryDirectory(prefix='.extended-receipt-', dir=candidate.parent) as receipt_tmp:
            private_bundle = Path(bundle_tmp) / 'bundle.tar.gz'
            private_receipt = Path(receipt_tmp) / 'candidate.json'
            write_bundle(stage, private_bundle, members)
            bundle_digest = digest_file(private_bundle)
            bundle_bytes = private_bundle.stat().st_size
            if ('expected_bundle_sha256' in plan
                    and (bundle_digest != plan['expected_bundle_sha256']
                         or bundle_bytes != plan['expected_bundle_bytes'])):
                raise ValueError('Candidate differs from the recorded deterministic bundle')
            spec = {'available': False, 'reason': 'Unpublished candidate; license audit and managed native acceptance required.',
                    'executable': plan['executable'], 'environment': {}, 'max_installed_bytes': installed,
                    'validation': {'member_hashes': 'verified', 'native_execution': 'not-run',
                                   'license_audit': plan['license_audit']},
                    'archives': [{'format': 'tar.gz', 'asset_name': output.name,
                                  'sha256': bundle_digest, 'bytes': bundle_bytes,
                                  'max_download_bytes': bundle_bytes,
                                  'max_expanded_bytes': installed + len(members) * 1024 + 10240,
                                  'members': members}]}
            with private_receipt.open('x') as receipt:
                json.dump({'version': version, 'targets': {target: spec}}, receipt, indent=2)
                receipt.write('\n')
            # Unsupported hard links fail closed: do not use a replace/copy
            # fallback that can clobber another invocation's output. The source
            # in each private directory is a regular file owned by this call.
            os.link(private_bundle, output, follow_symlinks=False)
            try:
                os.link(private_receipt, candidate, follow_symlinks=False)
            except OSError as error:
                # A two-file publication is not atomic. Preserve the valid
                # published bundle, and never unlink a path another process may
                # have replaced in the meantime. Only private staging is cleaned.
                raise RuntimeError(
                    f'Bundle created at {output}; receipt not published at {candidate}: {error}'
                ) from error



def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--tool', required=True, choices=TOOLS)
    parser.add_argument('--target', required=True, choices=TARGETS)
    for name in ('inputs', 'output', 'candidate', 'work-dir'):
        parser.add_argument('--' + name, required=True, type=Path)
    args = parser.parse_args()
    sources = json.loads(SOURCES.read_text())
    if sources['format_version'] != 1:
        raise ValueError('Unsupported fixed source manifest')
    tool = sources['tools'][args.tool]
    plan = tool['plans'].get(args.target)
    if plan is None:
        raise ValueError('No reviewed member-level plan for this formatter/target; metadata is not installable')
    build(args.tool, args.target, tool['version'], plan, args.inputs, args.output, args.candidate, args.work_dir)


if __name__ == '__main__':
    main()
