#!/usr/bin/env python3
"""Sign a verified .app, notarize its ZIP, staple, then create final ZIP and DMG.

Requires an existing Developer ID identity and existing notarytool keychain
profile. This script does not create credentials or publish releases. Run on a
trusted build only. Signing changes the dprint bytes; preserve the upstream hash
as provenance and update only its runtime checksum before sealing the app.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]


def run(*args):
    subprocess.run(list(map(str, args)), check=True)


def notarize(path, profile, report):
    result = subprocess.run(['xcrun', 'notarytool', 'submit', str(path),
                             '--keychain-profile', profile, '--wait', '--output-format', 'json'],
                            check=True, capture_output=True, text=True)
    report.write_text(result.stdout)
    if json.loads(result.stdout).get('status') != 'Accepted':
        raise RuntimeError(f'Apple did not accept notarization; see {report}')


def sign(app, target, identity, profile, output):
    app = app.resolve()
    resources = app / 'Contents/Resources/resources'
    manifest_path = resources / 'manifest.json'
    manifest = json.loads(manifest_path.read_text())
    path = f'bin/{target}/dprint'
    entries = [entry for entry in manifest['files'] if entry['path'] == path]
    if len(entries) != 1:
        raise ValueError('Expected exactly one target dprint checksum')
    entry = entries[0]
    executable = resources / path
    upstream = hashlib.sha256(executable.read_bytes()).hexdigest()
    # Validate against the committed upstream trust anchor, not only bundle data.
    trusted = json.loads((ROOT / 'resources/manifest.json').read_text())
    expected = next(item for item in trusted['files'] if item['path'] == path)
    if upstream != expected['sha256'] or upstream != entry['sha256']:
        raise ValueError('Unsigned dprint does not match pinned upstream bytes')
    for item in manifest['files']:
        file = resources / item['path']
        if item['path'].startswith('bin/') and item['path'] != path:
            continue
        if not file.is_file() or hashlib.sha256(file.read_bytes()).hexdigest() != item['sha256']:
            raise ValueError(f'Invalid or missing resource: {item["path"]}')
    run('codesign', '--force', '--options', 'runtime', '--timestamp', '--sign', identity,
        '--entitlements', ROOT / 'packaging/macos/dprint-entitlements.plist', executable)
    entry['upstream_sha256'] = upstream
    entry['sha256'] = hashlib.sha256(executable.read_bytes()).hexdigest()
    entry['bytes'] = executable.stat().st_size
    entry['distribution_transform'] = 'Developer ID codesign; upstream pin retained'
    manifest_path.write_text(json.dumps(manifest, indent=2) + '\n')
    run('codesign', '--force', '--options', 'runtime', '--timestamp', '--sign', identity,
        app / 'Contents/MacOS/codestyles')
    run('codesign', '--force', '--options', 'runtime', '--timestamp', '--sign', identity, app)
    run('codesign', '--verify', '--deep', '--strict', '--verbose=2', app)
    output.mkdir(parents=True, exist_ok=True)
    # The signed binary must still format through all four WASM plugins. This
    # detects JIT entitlement/runtime trouble before requesting notarization.
    run(sys.executable, ROOT / 'scripts/smoke_package.py', '--directory', app,
        '--report', output / f'{target}-signed-smoke.json')
    with tempfile.TemporaryDirectory(prefix='codestyles-notary-') as temporary:
        submission = Path(temporary) / 'submission.zip'
        run('ditto', '-c', '-k', '--keepParent', app, submission)
        notarize(submission, profile, output / f'{target}-app-notarization.json')
    run('xcrun', 'stapler', 'staple', app)
    run('xcrun', 'stapler', 'validate', app)
    run('spctl', '--assess', '--type', 'execute', '--verbose=2', app)
    zip_path = output / f'codestyles-{target}-signed.app.zip'
    run('ditto', '-c', '-k', '--keepParent', app, zip_path)
    dmg = output / f'codestyles-{target}-signed.dmg'
    with tempfile.TemporaryDirectory(prefix='codestyles-dmg-') as temporary:
        stage = Path(temporary)
        run('ditto', app, stage / 'CodeStyles.app')
        (stage / 'Applications').symlink_to('/Applications')
        run('hdiutil', 'create', '-volname', 'CodeStyles', '-srcfolder', stage,
            '-ov', '-format', 'UDZO', dmg)
    run('codesign', '--sign', identity, '--timestamp', dmg)
    notarize(dmg, profile, output / f'{target}-dmg-notarization.json')
    run('xcrun', 'stapler', 'staple', dmg)
    run('xcrun', 'stapler', 'validate', dmg)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--app', type=Path, required=True)
    parser.add_argument('--target', choices=['x86_64-apple-darwin', 'aarch64-apple-darwin'], required=True)
    parser.add_argument('--identity', required=True)
    parser.add_argument('--notary-profile', required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    sign(args.app, args.target, args.identity, args.notary_profile, args.output)


if __name__ == '__main__':
    main()
