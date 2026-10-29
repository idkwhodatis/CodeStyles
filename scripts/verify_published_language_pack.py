#!/usr/bin/env python3
"""Manual production acceptance: consume only the unchanged embedded GitHub pins.

Runs two full downloads plus one cancelled partial through the real manager.
Never builds, republishes, rewrites pins, or accepts a URL/runtime override.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import tempfile
import time
from urllib.parse import urlsplit

from build_language_pack import safe_path
from verify_native_language_pack import DEADLINE_ENV, MIN_FREE, ROOT, run

TEST = 'published_github_rust_pack_download_cancel_reinstall_and_uninstall'


def published_pin(catalog, sources, target):
    tool = catalog['toolchains']['rustfmt']
    source_tool = sources['toolchains']['rustfmt']
    spec = tool['targets'][target]
    source = source_tool['targets'][target]
    if (catalog.get('format_version') != 1 or sources.get('format_version') != 1
            or tool['version'] != source_tool['version'] or spec.get('available') is not True
            or source.get('available') is not True):
        raise ValueError('Expected a published, reviewed native target/version')
    if spec['executable'] != source['executable'] or spec.get('environment', {}) != source.get('environment', {}):
        raise ValueError('Published runtime differs from reviewed source')
    if len(spec['archives']) != 1:
        raise ValueError('Expected one pinned published bundle')
    archive = spec['archives'][0]
    url = urlsplit(archive['url'])
    parts = url.path.split('/')
    if (url.scheme != 'https' or url.netloc != 'github.com' or url.query or url.fragment
            or len(parts) != 7 or parts[1:5] != ['idkwhodatis', 'CodeStyles', 'releases', 'download']
            or not re.fullmatch(r'[A-Za-z0-9._-]+', parts[5]) or parts[6] != archive['asset_name']):
        raise ValueError('Expected a fixed CodeStyles GitHub release asset')
    safe_path(archive['asset_name'])
    if (archive['format'] != 'tar.gz' or not re.fullmatch(r'[0-9a-f]{64}', archive['sha256'])
            or type(archive['bytes']) is not int or archive['bytes'] <= 0
            or archive['max_download_bytes'] != archive['bytes']):
        raise ValueError('Invalid published archive pin')
    expected = {m['path']: {**m, 'archive_path': m['path']} for a in source['archives'] for m in a['members']}
    members = {m['path']: m for m in archive['members']}
    if len(members) != len(archive['members']) or set(members) != set(expected) | {'PROVENANCE.json'}:
        raise ValueError('Published member set differs from reviewed sources')
    for name, member in members.items():
        safe_path(name)
        if (member['archive_path'] != name or type(member['bytes']) is not int or member['bytes'] <= 0
                or not re.fullmatch(r'[0-9a-f]{64}', member['sha256']) or type(member['executable']) is not bool):
            raise ValueError('Invalid published member pin')
        if name in expected and member != expected[name]:
            raise ValueError('Published member differs from reviewed source')
    receipt = members['PROVENANCE.json']
    installed = sum(m['bytes'] for m in members.values())
    if (receipt['executable'] or receipt['bytes'] > 1024 ** 2
            or spec['max_installed_bytes'] != installed
            or installed > source['max_installed_bytes'] + 1024 ** 2
            or archive['max_expanded_bytes'] != installed + len(members) * 1024 + 10240):
        raise ValueError('Invalid published expansion bounds')
    return tool['version'], archive


def verify(target, work_dir, report_dir, root=ROOT, runner=run, disk_usage=shutil.disk_usage):
    root = Path(root).resolve()
    work_dir, report_dir = Path(work_dir).resolve(), Path(report_dir).resolve()
    work_dir.mkdir(parents=True, exist_ok=True)
    report_dir.mkdir(parents=True, exist_ok=True)
    catalog_path, source_path = root / 'resources/toolchains.json', root / 'resources/toolchain-sources.json'
    original, source_bytes = catalog_path.read_bytes(), source_path.read_bytes()
    env = dict(os.environ)
    # No local-candidate override survives into this production test.
    env.pop('CODESTYLES_TEST_RUSTFMT_ARCHIVE', None)
    env['CARGO_BUILD_JOBS'] = '2'
    env['CARGO_INCREMENTAL'] = '0'
    started = time.monotonic()
    env[DEADLINE_ENV] = str(started + 24 * 60)
    report = {'target': target, 'production_download': 'failed', 'catalog_unchanged': False, 'test_exit_code': None,
              'catalog_sha256': hashlib.sha256(original).hexdigest(),
              'source_manifest_sha256': hashlib.sha256(source_bytes).hexdigest()}
    temporary = None
    checked_tree = False
    try:
        if disk_usage(work_dir).free < MIN_FREE:
            raise ValueError('Production acceptance requires at least 2 GiB free')
        version, archive = published_pin(json.loads(original), json.loads(source_bytes), target)
        report.update(version=version, archive_sha256=archive['sha256'], archive_bytes=archive['bytes'])
        checked_tree = True
        runner(['git', 'diff', 'HEAD', '--exit-code'], root, env, report_dir / 'tracked-before.log')
        host = runner(['rustc', '-vV'], root, env, report_dir / 'host.log')
        if f'host: {target}' not in host.splitlines():
            raise ValueError('Target must match native rustc host')
        with tempfile.TemporaryDirectory(prefix='codestyles-published-check-', dir=work_dir) as temporary:
            for key in ('TMPDIR', 'TMP', 'TEMP'):
                env[key] = temporary
            output = runner(['cargo', 'test', '--locked', '--release', '--no-default-features', '--test', 'rustfmt',
                             TEST, '--', '--ignored', '--exact', '--nocapture', '--test-threads=1'],
                            root, env, report_dir / 'production-test.log')
            report['test_exit_code'] = 0  # run() raises on every nonzero exit or timeout.
            if not re.search(r'^test result: ok\. 1 passed; 0 failed; 0 ignored;', output, re.MULTILINE):
                raise RuntimeError('Exact production E2E did not report one executed passing test')
        report['production_download'] = 'passed'
    finally:
        report['catalog_unchanged'] = catalog_path.read_bytes() == original
        report['sources_unchanged'] = source_path.read_bytes() == source_bytes
        report['own_temporary_removed'] = temporary is None or not Path(temporary).exists()
        try:
            if not report['catalog_unchanged'] or not report['sources_unchanged']:
                raise RuntimeError('Tracked manifests changed during production acceptance')
            env[DEADLINE_ENV] = str(started + 25 * 60)
            if checked_tree:
                runner(['git', 'diff', 'HEAD', '--exit-code'], root, env, report_dir / 'tracked-after.log')
        except Exception:
            report['production_download'] = 'failed'
            raise
        finally:
            (report_dir / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--target', required=True)
    parser.add_argument('--work-dir', required=True, type=Path)
    parser.add_argument('--report-dir', required=True, type=Path)
    args = parser.parse_args()
    verify(args.target, args.work_dir, args.report_dir)


if __name__ == '__main__':
    main()
