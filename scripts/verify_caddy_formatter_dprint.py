#!/usr/bin/env python3
"""Offline Caddy oracle replay through explicitly supplied trusted executables.

No downloads, config imports, source execution, catalog writes, or publication.
Only runs dprint formatting and optional official `caddy fmt -`.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import random
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ROOT / 'tests/fixtures/caddyfmt'
MAX_CASES = 1000
MAX_INPUT = 65536
MAX_OUTPUT = 8 * 1024 * 1024


def checked_file(path, digest):
    path = Path(path).resolve(strict=True)
    if not path.is_file() or hashlib.sha256(path.read_bytes()).hexdigest() != digest:
        raise ValueError(f'Trusted input checksum mismatch: {path}')
    return path


def generated_cases():
    rows = json.loads((FIXTURES / 'upstream-cases.json').read_text())
    for row in rows:
        row['origin'] = 'upstream'
    extra = ['', '\n', '\r\n', 'localhost { respond "unterminated',
             'localhost { respond `unterminated', 'import missing.caddy\n{ email {$PRIVATE_EMAIL} }',
             'localhost { respond <<EOF\n  unchanged body { } $RANDOM\nEOF\n}',
             'localhost { respond <<\n}', 'localhost { respond <<bad!marker\n}\n',
             '{ ' * 1000 + 'x\n' + '} ' * 1000, '你好.example { respond "こんにちは 🐈" }',
             'a\u00a0{\u2003b\u0085c\u2028d\n}\n', '\ufefflocalhost {}',
             'localhost { respond \\"literal\\" }']
    rows.extend(dict(description=f'extra-{n}', input=value, origin='edge')
                for n, value in enumerate(extra))
    rng = random.Random(2117)
    tokens = ['localhost', '{', '}', 'respond', '"hello world"', '`braces { literal }`',
              '{$ENV}', '{host}', '# comment { x }', 'import', 'missing/*.caddy', '\\<',
              '<<END', 'END', '😺', '\u2003', '"', '`', '\\', '\r\n', '\n', '  ', '\t',
              'abc', '0', '<<', '"{\\"nested\\": 1}"']
    for i in range(512):
        rows.append(dict(description=f'seed-2117-{i}',
                         input=''.join(rng.choice(tokens) for _ in range(rng.randrange(1, 90))),
                         origin='generated-malformed-and-valid'))
    return rows


def validate_cases(rows):
    if not 0 < len(rows) <= MAX_CASES:
        raise ValueError('Oracle case count is out of bounds')
    for row in rows:
        if len(row['input'].encode()) > MAX_INPUT or len(row.get('expect', '').encode()) > MAX_OUTPUT:
            raise ValueError(f'Oracle fixture exceeds bounded suite size: {row["description"]}')


def run_format(command, source, workspace, timeout):
    # Do not inherit proxy/plugin/config secrets or the caller's home/cache.
    env = {key: str(workspace) for key in ['HOME', 'USERPROFILE', 'XDG_CONFIG_HOME',
           'XDG_CACHE_HOME', 'XDG_DATA_HOME', 'APPDATA', 'LOCALAPPDATA', 'TMPDIR', 'TMP', 'TEMP']}
    env['DPRINT_CACHE_DIR'] = str(workspace / 'cache')
    for key in ['SystemRoot', 'WINDIR']:
        if key in os.environ:
            env[key] = os.environ[key]
    # Redirect to files, not unbounded capture_output buffers. Inputs and nesting
    # are bounded by this test corpus. This is not a process memory/disk quota.
    with tempfile.TemporaryFile() as stdout, tempfile.TemporaryFile() as stderr:
        result = subprocess.run(command, input=source.encode(), stdout=stdout, stderr=stderr,
                                timeout=timeout, cwd=workspace, env=env, check=False)
        if stdout.tell() > MAX_OUTPUT or stderr.tell() > 65536:
            raise AssertionError('Formatter output exceeds bounded suite size')
        stdout.seek(0)
        stderr.seek(0)
        output, error = stdout.read(), stderr.read()
    if result.returncode or error:
        raise AssertionError(f'Formatter failed ({result.returncode}): {error[:4096]!r}')
    return output.decode('utf-8')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--wasm', type=Path, required=True)
    parser.add_argument('--wasm-sha256', required=True)
    parser.add_argument('--dprint', type=Path, required=True)
    parser.add_argument('--dprint-sha256', required=True)
    parser.add_argument('--caddy', type=Path, help='Optional official Caddy binary for independent replay')
    parser.add_argument('--caddy-sha256')
    parser.add_argument('--oracle', type=Path, default=FIXTURES / 'oracle-cases.json')
    parser.add_argument('--write-oracle', type=Path, help='Regenerate deterministic oracle here; requires --caddy')
    parser.add_argument('--report', type=Path)
    parser.add_argument('--timeout', type=float, default=10, help='Per-process timeout, at most 60 seconds')
    args = parser.parse_args()
    if not 0 < args.timeout <= 60:
        parser.error('--timeout must be 0 < seconds <= 60')
    if bool(args.caddy) != bool(args.caddy_sha256) or (args.write_oracle and not args.caddy):
        parser.error('--caddy and --caddy-sha256 are required together and for --write-oracle')
    wasm = checked_file(args.wasm, args.wasm_sha256)
    dprint = checked_file(args.dprint, args.dprint_sha256)
    caddy = checked_file(args.caddy, args.caddy_sha256) if args.caddy else None
    rows = generated_cases() if args.write_oracle else json.loads(args.oracle.read_text())
    validate_cases(rows)
    with tempfile.TemporaryDirectory(prefix='verify-caddy-') as directory:
        workspace = Path(directory)
        config = workspace / 'dprint.json'
        config.write_text(json.dumps({'caddyfmt': {}, 'plugins': [str(wasm)], 'incremental': False}))
        command = [str(dprint), 'fmt', '--config-discovery=false', '--skip-stable-format',
                   '--log-level=silent', '--stdin=Caddyfile', '--config', str(config)]
        for row in rows:
            if caddy:
                expected = run_format([str(caddy), 'fmt', '-'], row['input'], workspace, args.timeout)
                if 'expect' in row and expected != row['expect']:
                    raise AssertionError(f'Official Caddy oracle mismatch: {row["description"]}')
                row['expect'] = expected
            actual = run_format(command, row['input'], workspace, args.timeout)
            if actual != row['expect']:
                raise AssertionError(f'dprint one-pass mismatch: {row["description"]}')
        nested = []
        for depth in [64, 256, 1000, 2000]:
            source = 'a {\n' * depth + 'x\n' + '}\n' * depth
            assert len(source.encode()) <= MAX_INPUT
            actual = run_format(command, source, workspace, args.timeout)
            if caddy and actual != run_format([str(caddy), 'fmt', '-'], source, workspace, args.timeout):
                raise AssertionError(f'Nested-brace differential mismatch at depth {depth}')
            nested.append(dict(depth=depth, input_bytes=len(source.encode()), output_bytes=len(actual.encode())))
    if args.write_oracle:
        args.write_oracle.write_text(json.dumps(rows, ensure_ascii=False, indent=2) + '\n')
    report = dict(status='passed', cases=len(rows), upstream_cases=sum(r['origin'] == 'upstream' for r in rows),
                  official_caddy_replayed=bool(caddy), nested_braces=nested,
                  sha256=dict(wasm=args.wasm_sha256, dprint=args.dprint_sha256, caddy=args.caddy_sha256),
                  resource_limit_note='Per-process timeout and bounded fixtures; no host-enforced memory ceiling or disk quota.')
    if args.report:
        args.report.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report, indent=2))


if __name__ == '__main__':
    main()
