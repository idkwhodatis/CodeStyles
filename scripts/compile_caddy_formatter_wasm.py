#!/usr/bin/env python3
"""Offline build from pinned source and verified official compiler archives.

Explicitly runs the reviewed Go/TinyGo/Binaryen toolchain; never downloads,
publishes, updates the catalog or enables a runtime. Linux x86_64 build host.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import subprocess
import tarfile
import tempfile

from caddy_wasm_start import patch

ROOT = Path(__file__).resolve().parents[1]
SOURCES = ROOT / 'resources/caddy-formatter-sources.json'


def verify(path, pin):
    path = Path(path)
    if path.is_symlink() or not path.is_file() or path.stat().st_size != pin['bytes']:
        raise ValueError(f'Not a regular pinned input: {path}')
    data = path.read_bytes()
    if len(data) != pin['bytes'] or hashlib.sha256(data).hexdigest() != pin['sha256']:
        raise ValueError(f'Pinned input checksum mismatch: {path}')
    return data


def verified_sources(plan):
    """Snapshot only pinned files; never execute incidental checkout content."""
    result = {}
    for name, pin in plan['source_files'].items():
        path = PurePosixPath(name)
        if (not name or path.is_absolute() or '..' in path.parts
                or '\\' in name or ':' in name or str(path) != name):
            raise ValueError(f'Unsafe source path: {name}')
        current = ROOT
        for part in path.parts:
            current = current / part
            if current.is_symlink():
                raise ValueError(f'Symlink source path: {name}')
        result[path] = verify(current, pin)
    return result


def stage_formatter_sources(inputs, destination):
    prefix = PurePosixPath('formatters/caddyfmt')
    destination.mkdir(parents=True)
    for path, data in inputs.items():
        if path.is_relative_to(prefix):
            target = destination / path.relative_to(prefix)
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(data)


def build(archive_dir, output, work_dir=None):
    plan = json.loads(SOURCES.read_text())
    output = Path(output).resolve()
    output.parent.mkdir(parents=True, exist_ok=True)
    inputs = verified_sources(plan)
    for pin in plan['build']['archives']:
        verify(Path(archive_dir) / pin['name'], pin)
    with tempfile.TemporaryDirectory(prefix='caddy-wasm-build-', dir=work_dir) as temporary:
        root = Path(temporary)
        for pin in plan['build']['archives']:
            with tarfile.open(Path(archive_dir) / pin['name']) as source:
                source.extractall(root / pin['component'], filter='data')
        go = root / 'go/go'
        tinygo = root / 'tinygo/tinygo'
        binaryen = root / 'binaryen/binaryen-version_133'
        source = root / 'source'
        stage_formatter_sources(inputs, source)
        env = dict(os.environ, PATH=os.pathsep.join([str(go / 'bin'), str(tinygo / 'bin'),
                   str(binaryen / 'bin'), os.environ.get('PATH', '')]),
                   GOROOT=str(go), TINYGOROOT=str(tinygo), GOCACHE=str(root / 'cache/go'),
                   XDG_CACHE_HOME=str(root / 'cache'), GOPATH=str(root / 'gopath'),
                   GOTOOLCHAIN='local', GOPROXY='off', GOSUMDB='off', GOFLAGS='-buildvcs=false',
                   GOMAXPROCS='2', CGO_ENABLED='0', TZ='UTC', LC_ALL='C', SOURCE_DATE_EPOCH='0')
        subprocess.run([str(go / 'bin/go'), 'test', '-count=1', './...'], cwd=source, env=env, check=True)
        raw = root / 'plugin.raw.wasm'
        subprocess.run([str(tinygo / 'bin/tinygo'), 'build', *plan['build']['flags'],
                        '-o', str(raw), '.'], cwd=source, env=env, check=True)
        data = patch(raw.read_bytes())
        pin = plan['wasm']
        if len(data) != pin['bytes'] or hashlib.sha256(data).hexdigest() != pin['sha256']:
            raise ValueError('Compiled WASM differs from the reviewed candidate pin')
        output.write_bytes(data)
    return dict(bytes=len(data), sha256=hashlib.sha256(data).hexdigest())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--archive-dir', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--work-dir', type=Path)
    args = parser.parse_args()
    print(json.dumps(build(args.archive_dir, args.output, args.work_dir)))


if __name__ == '__main__':
    main()
