#!/usr/bin/env python3
"""Publish exactly one alpha from an already validated, immutable CI run.

The preparation path reads artifacts only; it never executes their contents.
The publishing path is retry-safe and refuses replacement of existing assets.
"""
import argparse
import hashlib
import json
from pathlib import Path, PurePosixPath
import re
import stat
import subprocess
import tempfile
import zipfile

REPO = 'idkwhodatis/CodeStyles'
RUN = 39194115622
SHA = '5135f7af5cf1da08b985e19470291f0d1cdcfe85'
TAG = 'v0.1.0-alpha.1'
TARGETS = tuple(f'{arch}-{platform}' for arch in ('x86_64', 'aarch64')
                for platform in ('unknown-linux-gnu', 'apple-darwin', 'pc-windows-msvc'))


def require(condition, message):
    if not condition:
        raise ValueError(message)


def gh(*args, binary=False):
    return subprocess.check_output(['gh', *args], text=not binary)


def api(path, method='GET', payload=None):
    command = ['api', f'repos/{REPO}/{path}', '--method', method]
    if payload is None:
        return json.loads(gh(*command))
    return json.loads(subprocess.check_output(
        ['gh', *command, '--input', '-'], input=json.dumps(payload), text=True))


def pages(path, key=None):
    result = []
    for page in range(1, 101):
        body = api(f'{path}{"&" if "?" in path else "?"}per_page=100&page={page}')
        values = body[key] if key else body
        result.extend(values)
        if len(values) < 100:
            return result
    raise ValueError('Unexpected pagination size')


def names(target):
    suffixes = ('.app.zip', '.dmg') if 'apple' in target else (
        '.zip', '-setup.exe') if 'windows' in target else ('.tar.gz', '.deb')
    return [f'codestyles-0.1.0-{target}{suffix}' for suffix in suffixes]


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def validate_run(run, jobs, artifacts):
    require(run['id'] == RUN and run['head_sha'] == SHA, 'Wrong run/commit')
    require(run['repository']['full_name'] == REPO and
            run['head_repository']['full_name'] == REPO, 'Wrong repository')
    require(run['event'] == 'push' and run['head_branch'] == 'master' and
            run['path'] == '.github/workflows/ci.yml', 'Untrusted build origin')
    require(run['status'] == 'completed' and run['conclusion'] == 'success',
            'All build checks must finish successfully')
    expected_jobs = {('Linux old-ABI' if 'linux' in t else 'Native') + f' ({t})'
                     for t in TARGETS}
    require(len(jobs) == 6 and {j['name'] for j in jobs} == expected_jobs,
            'Expected exactly six native build jobs')
    require(all(j['status'] == 'completed' and j['conclusion'] == 'success'
                and j['head_sha'] == SHA for j in jobs), 'Failed or wrong-commit job')
    require(len(artifacts) == 6 and {a['name'] for a in artifacts} ==
            {f'codestyles-{t}' for t in TARGETS}, 'Expected six unique artifacts')
    for artifact in artifacts:
        require(not artifact['expired'], 'Artifact expired')
        require(re.fullmatch(r'sha256:[0-9a-f]{64}', artifact.get('digest') or ''),
                'Missing GitHub artifact digest')
        require(artifact['workflow_run']['id'] == RUN and
                artifact['workflow_run']['head_sha'] == SHA, 'Wrong artifact provenance')


def unpack_packages(archive, target, output):
    """Only copy exact package basenames; never extract paths or inner packages."""
    expected = set(names(target))
    found = set()
    seen = set()
    with zipfile.ZipFile(archive) as bundle:
        for member in bundle.infolist():
            name = member.orig_filename
            path = PurePosixPath(name)
            require(name and not path.is_absolute() and '\\' not in name and
                    ':' not in name and '..' not in path.parts and
                    str(path) == name.rstrip('/'), 'Unsafe artifact member')
            require(name not in seen, 'Duplicate artifact member')
            seen.add(name)
            mode = member.external_attr >> 16
            require(stat.S_IFMT(mode) in (0, stat.S_IFREG, stat.S_IFDIR),
                    'Non-regular artifact member')
            if path.name not in expected:
                continue
            require(not member.is_dir() and path.parts in ((path.name,), ('dist', path.name)),
                    'Unexpected package location')
            require(path.name not in found, 'Duplicate package basename')
            require(0 < member.file_size < 2_000_000_000, 'Invalid package size')
            with bundle.open(member) as source, (output / path.name).open('xb') as dest:
                while chunk := source.read(1024 * 1024):
                    dest.write(chunk)
            found.add(path.name)
    require(found == expected, 'Missing native package')


def prepare(output):
    run = api(f'actions/runs/{RUN}')
    jobs = pages(f'actions/runs/{RUN}/jobs?filter=latest', 'jobs')
    artifacts = pages(f'actions/runs/{RUN}/artifacts', 'artifacts')
    validate_run(run, jobs, artifacts)
    output.mkdir(parents=True, exist_ok=False)
    with tempfile.TemporaryDirectory() as temp:
        for artifact in artifacts:
            archive = Path(temp) / 'artifact.zip'
            with archive.open('wb') as stream:
                subprocess.run(['gh', 'api', f'repos/{REPO}/actions/artifacts/{artifact["id"]}/zip'],
                               stdout=stream, check=True)
            require('sha256:' + digest(archive) == artifact['digest'], 'Artifact digest mismatch')
            unpack_packages(archive, artifact['name'].removeprefix('codestyles-'), output)
    (output / 'PROVENANCE.json').write_text(json.dumps({
        'repository': REPO, 'commit': SHA, 'run_id': RUN, 'tag': TAG,
        'package_version': '0.1.0', 'run_attempt': run['run_attempt'],
        'artifacts': [{'id': a['id'], 'name': a['name'], 'digest': a['digest']}
                      for a in sorted(artifacts, key=lambda a: a['name'])],
    }, indent=2) + '\n', encoding='utf-8')
    manifest = ''.join(f'{digest(p)}  {p.name}\n' for p in sorted(output.iterdir()))
    (output / 'SHA256SUMS').write_text(manifest, encoding='utf-8')


def verify_files(directory):
    expected = {n for t in TARGETS for n in names(t)} | {'PROVENANCE.json'}
    require({p.name for p in directory.iterdir()} == expected | {'SHA256SUMS'},
            'Unexpected release files')
    require(all(p.is_file() and not p.is_symlink() for p in directory.iterdir()),
            'Release inputs must be regular files')
    provenance = json.loads((directory / 'PROVENANCE.json').read_text(encoding='utf-8'))
    require(provenance['repository'] == REPO and provenance['commit'] == SHA and
            provenance['run_id'] == RUN and provenance['tag'] == TAG, 'Wrong provenance')
    lines = (directory / 'SHA256SUMS').read_text(encoding='utf-8').splitlines()
    require(len(lines) == 13, 'Checksum manifest must cover twelve packages and provenance')
    recorded = {}
    for line in lines:
        match = re.fullmatch(r'([0-9a-f]{64})  ([A-Za-z0-9_.-]+)', line)
        require(match is not None, 'Malformed checksum manifest')
        checksum, name = match.groups()
        require(name in expected and name not in recorded, 'Unexpected checksum entry')
        require(digest(directory / name) == checksum, 'Package checksum mismatch')
        recorded[name] = checksum
    recorded['SHA256SUMS'] = digest(directory / 'SHA256SUMS')
    return recorded


def publish(directory):
    expected = verify_files(directory)
    # Refuse an existing tag at any other commit; never force-update tags.
    refs = api(f'git/matching-refs/tags/{TAG}')
    refs = [ref for ref in refs if ref['ref'] == f'refs/tags/{TAG}']
    if refs:
        require(len(refs) == 1 and refs[0]['object']['type'] == 'commit' and
                refs[0]['object']['sha'] == SHA, 'Existing tag differs from validated commit')
    else:
        api('git/refs', 'POST', {'ref': f'refs/tags/{TAG}', 'sha': SHA})
    releases = [r for r in pages('releases') if r['tag_name'] == TAG]
    require(len(releases) <= 1, 'Duplicate release')
    if releases:
        release = releases[0]
        require(release['prerelease'], 'Existing release is not a prerelease')
    else:
        release = api('releases', 'POST', {
            'tag_name': TAG, 'target_commitish': SHA, 'name': f'CodeStyles {TAG}',
            'draft': True, 'prerelease': True, 'make_latest': 'false',
            'body': f'''First alpha preview of CodeStyles. Experimental; please report issues.

Built from commit {SHA}; all six native distribution jobs passed in
https://github.com/{REPO}/actions/runs/{RUN}.

Windows x64/ARM64: portable ZIP and installer. macOS Intel/Apple Silicon:
.app ZIP and DMG. Linux x64/ARM64: tar.gz and Debian package (glibc 2.28 baseline).

These are the exact CI-built 0.1.0 packages, published under the alpha release
label without rebuilding or changing their filenames/embedded version.
macOS packages are unsigned and not notarized; Windows installers are unsigned.
SHA256SUMS covers the twelve native packages and PROVENANCE.json. No Actions wrapper ZIPs are shipped.
''',
        })
    assets = pages(f'releases/{release["id"]}/assets')
    require(len({a['name'] for a in assets}) == len(assets), 'Duplicate release asset')
    for asset in assets:
        require(asset['name'] in expected and asset.get('digest') ==
                'sha256:' + expected[asset['name']], 'Existing release asset differs')
    missing = set(expected) - {a['name'] for a in assets}
    require(release['draft'] or not missing, 'Published release is incomplete; refusing mutation')
    for name in sorted(missing):
        gh('release', 'upload', TAG, str(directory / name), '--repo', REPO)
    assets = pages(f'releases/{release["id"]}/assets')
    require(len(assets) == len(expected) and {a['name']: a.get('digest') for a in assets} ==
            {n: 'sha256:' + d for n, d in expected.items()}, 'Uploaded asset verification failed')
    if release['draft']:
        api(f'releases/{release["id"]}', 'PATCH',
            {'draft': False, 'prerelease': True, 'make_latest': 'false'})
    final = api(f'releases/{release["id"]}')
    require(final['tag_name'] == TAG and final['prerelease'] and not final['draft'],
            'Release publication was not confirmed')
    print(f'https://github.com/{REPO}/releases/tag/{TAG}')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=('prepare', 'publish'))
    parser.add_argument('directory', type=Path)
    args = parser.parse_args()
    {'prepare': prepare, 'publish': publish}[args.command](args.directory)
