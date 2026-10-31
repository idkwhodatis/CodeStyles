# Offline native candidate preparation

These helpers prepare and validate **unpublished Go and Dart candidates**. They
do not download inputs, publish assets, enable catalog downloads, or establish
license clearance. No new GitHub Actions workflow is installed by this change.
Vue uses the existing `build_vue_pack.py` WASM format, not this native executable
schema. Existing Rust package validation is unchanged.

## Review and build local inputs

Prepare a reviewed JSON plan with these fields:

- `tool`: `gofmt` or `dartfmt`; exact `version` and one of the six native targets
  allowlisted in `build_candidate.py`.
- `executable` and `environment`: the existing adapter's catalog values.
- `available: false`, `release_clearance: false`.
- `provenance.sources`: source receipts/pins; `provenance.caveats`: unresolved
  attribution, build provenance, compatibility and acceptance qualifications.
- `members`: explicit `input_path`, destination `path`, `bytes`, SHA-256 `sha256`,
  and boolean `executable` for every runtime and notice file.

Review the source receipts and notice applicability separately. A valid plan hash
proves the reviewed plan was used, not that its legal/provenance claims are true.
Derive its SHA-256 from independently reviewed bytes, not an untrusted downloaded
checksum. Never replace missing inputs with arbitrary URLs or PATH executables.

```sh
python scripts/candidate_pipeline/build_candidate.py \
  --plan /reviewed/plan.json --plan-sha256 REVIEWED_64_HEX_SHA256 \
  --input-dir /reviewed/local-inputs --output-dir /new/output-directory
```

The output directory must not exist. The builder checks member pins, regular
non-symlink paths, portable name collisions and bounded sizes. It adds generated
`PROVENANCE.json`, writes a deterministic archive and a
`candidate-manifest.json`, then checks exact hashes/modes/canonical metadata. All
output remains unavailable for download and uncleared for release. The helper
does not execute any candidate binary. Rebuild separately and compare archive
digests before selecting a candidate for native acceptance.

## Run exact native manager acceptance

This is an explicit native test, **not** a fixture test. It requires a clean local
application checkout at a reviewed commit, matching native Rust 1.99.1 already
installed through rustup, prepopulated dependency caches, verified backend
resources and at least 2 GiB free staging space. Missing inputs fail closed;
neither rustup auto-install nor Cargo network access is enabled.
The report directory must be new; existing directories are never adopted or
overwritten, preventing preexisting log/report symlinks from redirecting writes.

```sh
python scripts/candidate_pipeline/acceptance.py \
  --plan /reviewed/plan.json --plan-sha256 REVIEWED_64_HEX_SHA256 \
  --manifest /reviewed/candidate-manifest.json \
  --bundle /reviewed/EXACT_ASSET_NAME.tar.gz \
  --root /clean/CodeStyles --expected-commit REVIEWED_40_HEX_COMMIT \
  --work-dir /separate/staging --report-dir /separate/reports
```

The helper validates the complete archive against the reviewed plan, checks
native host and application identity, and creates an owned **local shared clone**.
Only that clone gets a temporary unavailable candidate catalog entry; the
canonical checkout is never patched. Existing local backend resources are
copied and verified in the clone. The archive is copied and revalidated before
the real manager test runs with `--offline --locked --ignored --exact`.

The selected `tests/new_languages.rs` test must report exactly one executed pass:
managed installation, preview, reviewed project writes and uninstall. A zero-test
success is rejected. The isolated catalog is restored on success/failure and
owned temporary files are removed. The original checkout/catalog is checked
again; concurrent external changes are reported, never overwritten. Only bounded
logs and `report.json` are retained. Resource capacity beyond the initial floor
depends on local caches and target; the floor is not a guarantee builds will fit.

## What remains before cross-platform execution or publication

- The prior twelve native probe artifacts contain reports, **not** executable
  packages. Their smoke success is not managed package acceptance.
- Materialize each reviewed target's exact runtime, notices and backend resources;
  complete plan/manifest/bundle locks and cache provisioning. No acquisition or
  cross-platform dispatch is supplied here.
- Run exact manager acceptance for each final candidate on the matching native
  target. Application CI does not replace this acceptance step.
- Resolve attribution/provenance qualifications. Symbol/header observations and
  contemporary runner smoke tests do not certify minimum operating-system support.
- Release publication and catalog activation require separate authorization and
  remain outside these helpers.

## Offline regression tests

```sh
python -m unittest discover -s scripts -p 'test_*.py'
```

The package's tests use tiny non-executable fixtures and mocked native
subprocesses. They cover deterministic builds, corrupt input/archive/metadata,
invalid plan hashes, unsafe paths, isolated restoration, missing toolchain,
wrong host/commit, and zero-test rejection. They are **not native acceptance
results** and do not supply missing third-party notice review.
