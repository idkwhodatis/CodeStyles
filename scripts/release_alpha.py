#!/usr/bin/env python3
"""Verify the fixed alpha binaries and finalize release notes only.

No asset uploads/deletions, tag changes, or source archive replacements are allowed.
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
OLD_SHA = '5135f7af5cf1da08b985e19470291f0d1cdcfe85'
RUN = 39200102985
SHA = '50d54d8a299eca891167dd251306fc67f1bffac1'
RELEASE_ID = 416881817
TAG = 'v0.1.0-alpha.1'
TARGETS = tuple(f'{arch}-{platform}'
                for platform in ('pc-windows-msvc', 'apple-darwin', 'unknown-linux-gnu')
                for arch in ('x86_64', 'aarch64'))


def require(condition, message):
    if not condition:
        raise ValueError(message)


def gh(*args, binary=False):
    return subprocess.check_output(['gh', *args], text=not binary)


def api(path, method='GET', payload=None):
    command = ['api', f'repos/{REPO}/{path}', '--method', method]
    if payload is None:
        response = gh(*command)
        return json.loads(response) if response.strip() else None
    response = subprocess.check_output(
        ['gh', *command, '--input', '-'], input=json.dumps(payload), text=True)
    return json.loads(response) if response.strip() else None


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


def validate_run(run, jobs, artifacts, run_id=RUN, commit=SHA):
    require(run['id'] == run_id and run['head_sha'] == commit, 'Wrong run/commit')
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
                and j['head_sha'] == commit for j in jobs), 'Failed or wrong-commit job')
    require(len(artifacts) == 6 and {a['name'] for a in artifacts} ==
            {f'codestyles-{t}' for t in TARGETS}, 'Expected six unique artifacts')
    for artifact in artifacts:
        require(not artifact['expired'], 'Artifact expired')
        require(re.fullmatch(r'sha256:[0-9a-f]{64}', artifact.get('digest') or ''),
                'Missing GitHub artifact digest')
        require(artifact['workflow_run']['id'] == run_id and
                artifact['workflow_run']['head_sha'] == commit, 'Wrong artifact provenance')


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


def prepare(output, run_id=RUN, commit=SHA):
    run = api(f'actions/runs/{run_id}')
    jobs = pages(f'actions/runs/{run_id}/jobs?filter=latest', 'jobs')
    artifacts = pages(f'actions/runs/{run_id}/artifacts', 'artifacts')
    validate_run(run, jobs, artifacts, run_id, commit)
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
        'repository': REPO, 'commit': commit, 'run_id': run_id, 'tag': TAG,
        'package_version': '0.1.0', 'run_attempt': run['run_attempt'],
        'artifacts': [{'id': a['id'], 'name': a['name'], 'digest': a['digest']}
                      for a in sorted(artifacts, key=lambda a: a['name'])],
    }, indent=2) + '\n', encoding='utf-8')
    manifest = ''.join(f'{digest(p)}  {p.name}\n' for p in sorted(output.iterdir()))
    (output / 'SHA256SUMS').write_text(manifest, encoding='utf-8')


def verify_files(directory, run_id=RUN, commit=SHA):
    expected = {n for t in TARGETS for n in names(t)} | {'PROVENANCE.json'}
    require({p.name for p in directory.iterdir()} == expected | {'SHA256SUMS'},
            'Unexpected release files')
    require(all(p.is_file() and not p.is_symlink() for p in directory.iterdir()),
            'Release inputs must be regular files')
    provenance = json.loads((directory / 'PROVENANCE.json').read_text(encoding='utf-8'))
    require(provenance['repository'] == REPO and provenance['commit'] == commit and
            provenance['run_id'] == run_id and provenance['tag'] == TAG, 'Wrong provenance')
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



def release_body():
    base = f'https://github.com/{REPO}/releases/download/{TAG}/'
    sections = []
    for platform, label in [('pc-windows-msvc', 'Windows'),
                            ('apple-darwin', 'macOS'),
                            ('unknown-linux-gnu', 'Linux')]:
        rows = [f'### {label}']
        for arch, arch_label in [('x86_64', 'x64'), ('aarch64', 'ARM64')]:
            packages = names(f'{arch}-{platform}')
            labels = ('Portable ZIP', 'Installer') if platform.startswith('pc-') else (
                '.app ZIP', 'DMG') if platform.startswith('apple') else ('tar.gz', 'Debian package')
            rows.append(f'- **{arch_label}**: ' + ' · '.join(
                f'[{label}]({base}{name})' for label, name in zip(labels, packages)))
        sections.append('\n'.join(rows))
    downloads = '\n\n'.join(sections)
    return f"""First alpha preview of CodeStyles, refreshed with the latest validated UI build.
Experimental; please report issues.

## Downloads

{downloads}

Native binaries built from commit {SHA}; all six native distribution jobs passed in
https://github.com/{REPO}/actions/runs/{RUN}.

## Source code notice

The existing `{TAG}` tag remains at commit {OLD_SHA}. GitHub's automatic
“Source code (zip)” and “Source code (tar.gz)” assets therefore contain that older
source, not the source used to build the native binaries above.
For source matching these binaries, use the exact build commit:
[ZIP](https://github.com/{REPO}/archive/{SHA}.zip) ·
[tar.gz](https://github.com/{REPO}/archive/{SHA}.tar.gz).

These are the exact CI-built 0.1.0 packages; filenames and embedded versions are
unchanged. macOS packages are unsigned and not notarized; Windows installers are
unsigned. Linux targets a glibc 2.28 baseline plus documented desktop dependencies.
Headless CI does not establish manual desktop acceptance on every supported OS.

[SHA256SUMS]({base}SHA256SUMS) covers all twelve native packages and
[PROVENANCE.json]({base}PROVENANCE.json). No Actions wrapper ZIPs are shipped.
The download links above are ordered Windows, macOS, Linux, with x64 before ARM64.
GitHub controls the order of its separate Assets list.
"""



def inspect_release(expected):
    refs = api(f'git/matching-refs/tags/{TAG}')
    refs = [ref for ref in refs if ref['ref'] == f'refs/tags/{TAG}']
    require(len(refs) == 1 and refs[0]['object']['type'] == 'commit' and
            refs[0]['object']['sha'] == OLD_SHA, 'Unexpected alpha tag; notes-only finalization refused')
    release = api(f'releases/{RELEASE_ID}')
    require(release['id'] == RELEASE_ID and release['tag_name'] == TAG and
            release['prerelease'] and not release['draft'] and
            not release.get('immutable', False) and
            release['html_url'] == f'https://github.com/{REPO}/releases/tag/{TAG}',
            'Unexpected existing alpha release')
    assets = pages(f'releases/{RELEASE_ID}/assets')
    require(len(assets) == len(expected) and
            all(a.get('state') == 'uploaded' for a in assets) and
            {a['name']: a.get('digest') for a in assets} ==
            {n: 'sha256:' + d for n, d in expected.items()},
            'All fourteen assets must match the validated new build; no asset repair is permitted')
    return release


def finalize_notes(directory):
    expected = verify_files(directory)
    release = inspect_release(expected)
    body = release_body()
    # Deliberately omit tag_name and target_commitish: this action edits notes only.
    if release.get('body') != body:
        api(f'releases/{RELEASE_ID}', 'PATCH', {'body': body})
    final = inspect_release(expected)
    require(final.get('body') == body, 'Release notes update was not confirmed')
    print(final['html_url'])


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=('prepare', 'finalize-notes'))
    parser.add_argument('directory', type=Path)
    args = parser.parse_args()
    {'prepare': prepare, 'finalize-notes': finalize_notes}[args.command](args.directory)
