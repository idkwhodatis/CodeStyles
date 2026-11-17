#!/usr/bin/env python3
"""Offline five-pack native acceptance, using already provisioned pinned inputs.

Example: python scripts/verify_script_packs.py --target x86_64-unknown-linux-gnu \
  --source-dir /trusted/script-assets --caddy-wasm /trusted/caddyfmt.wasm \
  --work-dir /tmp/script-work --report-dir /tmp/new-script-report

Provision resources separately using the repository's reviewed resource workflow.
This command never downloads, publishes, promotes catalog entries, or runs user
source. The optional Unix test executes only six fixed Bash fixtures. A passing
run is host-only runtime evidence, not GUI, oldest-OS or release acceptance.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import tempfile
import time

import build_dprint_pack
import build_wasm_language_pack
import build_caddy_formatter_pack
from candidate_pipeline.acceptance import regular_tree
from verify_native_language_pack import run, DEADLINE_ENV

ROOT = Path(__file__).resolve().parents[1]
PACKS = ('dprint', 'shfmt', 'stylua', 'dockerfile', 'caddyfmt')
VERSIONS = dict(dprint='0.63.0-p1', shfmt='shfmt-0.1.2', stylua='stylua-0.1.0',
                dockerfile='dockerfile-0.7.0', caddyfmt='caddy-2.11.7-wasm-p1')
TESTS = (
    'real_scripts_preview_settings_review_apply_and_lifecycle',
    'real_parser_errors_and_unsafe_project_inputs_preserve_originals',
    'real_literals_aliases_and_heredocs_are_preserved',
    'real_caddy_wasm_requires_engine_and_preserves_isolation_and_lifecycle',
)
SHELL_TEST = 'real_shell_fixed_fixtures_preserve_observable_behavior'


def require_pass(output):
    if 'test result: ok. 1 passed; 0 failed; 0 ignored;' not in output:
        raise RuntimeError('Exact runtime test did not execute one passing test')


def verify(target, source_dir, caddy_wasm, work_dir, report_dir, root=ROOT, runner=run):
    root = Path(root).resolve()
    source_dir, caddy_wasm = Path(source_dir).resolve(), Path(caddy_wasm).resolve()
    work_dir, report_dir = Path(work_dir).resolve(), Path(report_dir).resolve()
    if work_dir.is_relative_to(root) or report_dir.is_relative_to(root):
        raise ValueError('Work and report directories must be outside the checkout')
    regular_tree(root / 'resources')
    work_dir.mkdir(parents=True, exist_ok=True)
    report_dir.mkdir(parents=True, exist_ok=False)
    catalog_path = root / 'resources/toolchains.json'
    original = catalog_path.read_bytes()
    catalog = json.loads(original)
    # Pin the same native compiler as the reviewed application, without rustup downloads.
    env = dict(os.environ, CARGO_NET_OFFLINE='true', RUSTUP_AUTO_INSTALL='0',
               RUSTUP_TOOLCHAIN='1.99.1-' + target, CARGO_BUILD_JOBS='2')
    for key in ('RUSTC', 'RUSTDOC', 'RUSTC_WRAPPER', 'RUSTC_WORKSPACE_WRAPPER', 'CARGO_BUILD_RUSTC_WRAPPER'):
        env.pop(key, None)
    env[DEADLINE_ENV] = str(time.monotonic() + 30 * 60)
    report = dict(native_execution='failed', published=False, release_clearance=False,
                  target=target, tests=[], catalog_sha256=hashlib.sha256(original).hexdigest(),
                  canonical_catalog_unchanged=False)
    try:
        runner(['git', 'diff', 'HEAD', '--exit-code'], root, env, report_dir / 'clean.log')
        head = runner(['git', 'rev-parse', 'HEAD'], root, env, report_dir / 'head.log').strip()
        report['application_commit'] = head
        host = runner(['rustc', '-vV'], root, env, report_dir / 'host.log')
        if f'host: {target}' not in host.splitlines():
            raise ValueError('Acceptance requires the native Rust host')
        for tool in PACKS:
            if catalog['toolchains'][tool]['version'] != VERSIONS[tool]:
                raise ValueError('Catalog version differs from reviewed builder')
            if catalog['toolchains'][tool]['targets'][target]['available'] is not False:
                raise ValueError('Only disabled candidate targets are supported')
        with tempfile.TemporaryDirectory(prefix='script-packs-', dir=work_dir) as temporary:
            own = Path(temporary)
            checkout = own / 'checkout'
            runner(['git', 'clone', '--shared', '--no-checkout', str(root), str(checkout)],
                   work_dir, env, report_dir / 'clone.log')
            runner(['git', 'checkout', '--detach', head], checkout, env, report_dir / 'checkout.log')
            if (checkout / 'resources/toolchains.json').read_bytes() != original:
                raise ValueError('Isolated catalog differs from reviewed checkout')
            shutil.copytree(root / 'resources', checkout / 'resources', dirs_exist_ok=True)
            report['archives'] = {}
            for tool in PACKS:
                bundle = own / (tool + '.tar.gz')
                if tool == 'dprint':
                    candidate = build_dprint_pack.build(checkout / 'resources', target, bundle)
                elif tool == 'caddyfmt':
                    candidate = build_caddy_formatter_pack.build(caddy_wasm, bundle)
                else:
                    candidate = build_wasm_language_pack.build(tool, source_dir, bundle,
                                                               checkout / 'resources')
                if candidate.get('available') is not False:
                    raise ValueError('Builder must produce a disabled candidate')
                catalog['toolchains'][tool]['targets'][target] = candidate
                env['CODESTYLES_' + tool.upper() + '_ARCHIVE'] = str(bundle)
                report['archives'][tool] = hashlib.sha256(bundle.read_bytes()).hexdigest()
            (checkout / 'resources/toolchains.json').write_text(json.dumps(catalog, indent=2) + '\n')
            env['CARGO_TARGET_DIR'] = str(own / 'target')
            runtime_temp = own / 'runtime-temp'
            runtime_temp.mkdir()
            for key in ('TMPDIR', 'TMP', 'TEMP'):
                env[key] = str(runtime_temp)
            tests = list(TESTS)
            if os.name == 'posix':
                tests.append(SHELL_TEST)
            for name in tests:
                output = runner(['cargo', 'test', '--offline', '--locked', '--no-default-features',
                                 '--test', 'script_config', name, '--', '--ignored', '--exact',
                                 '--nocapture', '--test-threads=1'], checkout, env,
                                report_dir / (name + '.log'))
                require_pass(output)
                report['tests'].append(name)
            report['shell_behavior'] = 'passed' if SHELL_TEST in tests else 'not_applicable'
        report['native_execution'] = 'passed'
    finally:
        report['canonical_catalog_unchanged'] = catalog_path.read_bytes() == original
        if not report['canonical_catalog_unchanged']:
            report['native_execution'] = 'failed'
        (report_dir / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
        if not report['canonical_catalog_unchanged']:
            raise RuntimeError('Canonical catalog changed externally; refusing to overwrite it')
    return report


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('target', 'source-dir', 'caddy-wasm', 'work-dir', 'report-dir'):
        parser.add_argument('--' + name, required=True)
    args = parser.parse_args()
    verify(args.target, args.source_dir, args.caddy_wasm, args.work_dir, args.report_dir)
