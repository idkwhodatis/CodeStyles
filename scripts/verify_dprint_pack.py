#!/usr/bin/env python3
"""Native, offline dprint candidate acceptance. Never publishes or enables targets.

Use bundle_resources.py fetch explicitly before this command. The catalog is
restored byte-for-byte even if compilation/execution fails. This validates the
candidate through the real manager rather than invoking an extracted binary.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess

import build_dprint_pack

ROOT = Path(__file__).resolve().parents[1]


def verify(target, work_dir, root=ROOT, run=subprocess.run):
    root, work_dir = Path(root), Path(work_dir)
    host = subprocess.check_output(['rustc', '-vV'], text=True)
    if f'host: {target}' not in host.splitlines():
        raise ValueError('Candidate acceptance requires its native Rust host')
    work_dir.mkdir(parents=True, exist_ok=True)
    archive = work_dir / f'codestyles-dprint-{build_dprint_pack.VERSION}-{target}.tar.gz'
    candidate = build_dprint_pack.build(root / 'resources', target, archive)
    (work_dir / 'candidate.json').write_text(json.dumps(candidate, indent=2) + '\n')
    path = root / 'resources/toolchains.json'
    original = path.read_bytes()
    catalog = json.loads(original)
    tool = catalog['toolchains']['dprint']
    if tool['version'] != build_dprint_pack.VERSION or candidate['available'] is not False:
        raise ValueError('Candidate version/availability is not the reviewed contract')
    tool['targets'][target] = candidate
    env = dict(os.environ, CODESTYLES_DPRINT_ARCHIVE=str(archive.resolve()))
    try:
        path.write_text(json.dumps(catalog, indent=2) + '\n')
        run(['cargo', 'test', '--locked', '--no-default-features', '--test', 'managed_dprint',
             '--', '--include-ignored'], cwd=root, env=env, check=True)
    finally:
        path.write_bytes(original)
    return candidate


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--target', required=True)
    parser.add_argument('--work-dir', type=Path, required=True)
    args = parser.parse_args()
    verify(args.target, args.work_dir)


if __name__ == '__main__':
    main()
