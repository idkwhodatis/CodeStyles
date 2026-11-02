#!/usr/bin/env python3
"""Sign a verified .app, notarize its ZIP, staple, then create final ZIP and DMG.

Requires an existing Developer ID identity and existing notarytool keychain
profile. This script does not create credentials or publish releases. Run on a
trusted build only. Runtime packs are separately distributed and signed; the app
contains no dprint executable or WASM payload.
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


def notarize(path, profile, report, keychain=None):
    command = ['xcrun', 'notarytool', 'submit', str(path),
               '--keychain-profile', profile, '--wait', '--output-format', 'json']
    if keychain is not None:
        command += ['--keychain', str(keychain)]
    result = subprocess.run(command,
                            check=True, capture_output=True, text=True)
    report.write_text(result.stdout)
    if json.loads(result.stdout).get('status') != 'Accepted':
        raise RuntimeError(f'Apple did not accept notarization; see {report}')


def sign(app, target, identity, profile, output, keychain=None):
    app = app.resolve()
    resources = app / 'Contents/Resources/resources'
    manifest_path = resources / 'manifest.json'
    manifest = json.loads(manifest_path.read_text())
    trusted = json.loads((ROOT / 'resources/manifest.json').read_text())
    trusted_files = {item['path']: item for item in trusted['files']}
    if (resources / 'bin').exists() or (resources / 'plugins').exists():
        raise ValueError('Runtime payload must be distributed as a separate managed pack')
    for item in manifest['files']:
        if not item['path'].startswith(('schemas/', 'licenses/')):
            continue
        expected = trusted_files.get(item['path'])
        file = resources / item['path']
        if (expected != item or not file.is_file()
                or hashlib.sha256(file.read_bytes()).hexdigest() != item['sha256']):
            raise ValueError(f'Invalid or missing resource: {item["path"]}')
    run('codesign', '--force', '--options', 'runtime', '--timestamp', '--sign', identity,
        app / 'Contents/MacOS/codestyles')
    run('codesign', '--force', '--options', 'runtime', '--timestamp', '--sign', identity, app)
    run('codesign', '--verify', '--deep', '--strict', '--verbose=2', app)
    output.mkdir(parents=True, exist_ok=True)
    # Verify signed application startup with no installed runtime. Runtime pack
    # signing/JIT compatibility requires its own release validation.
    run(sys.executable, ROOT / 'scripts/smoke_package.py', '--directory', app,
        '--report', output / f'{target}-signed-smoke.json')
    with tempfile.TemporaryDirectory(prefix='codestyles-notary-') as temporary:
        submission = Path(temporary) / 'submission.zip'
        run('ditto', '-c', '-k', '--keepParent', app, submission)
        notarize(submission, profile, output / f'{target}-app-notarization.json', keychain)
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
    notarize(dmg, profile, output / f'{target}-dmg-notarization.json', keychain)
    run('xcrun', 'stapler', 'staple', dmg)
    run('xcrun', 'stapler', 'validate', dmg)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--app', type=Path, required=True)
    parser.add_argument('--target', choices=['x86_64-apple-darwin', 'aarch64-apple-darwin'], required=True)
    parser.add_argument('--identity', required=True)
    parser.add_argument('--notary-profile', required=True)
    parser.add_argument('--notary-keychain', type=Path, help='File-based keychain used when storing the profile')
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    sign(args.app, args.target, args.identity, args.notary_profile, args.output, args.notary_keychain)


if __name__ == '__main__':
    main()
