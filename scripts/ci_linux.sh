#!/usr/bin/env bash
set -euo pipefail
: "${TARGET:?native target required}"
python -c "import subprocess; host=subprocess.check_output(['rustc', '-vV'], text=True); print(host); assert 'host: ${TARGET}' in host"
# Record installed RPMs as provenance: base is digest-pinned; distro security
# updates can change development RPMs and the inventory makes this explicit.
mkdir -p dist
cp /opt/codestyles-build-packages.txt dist/build-packages-${TARGET}.txt
python -m unittest discover -s scripts -p 'test_*.py'
cargo fmt --check
cargo test --locked --no-default-features --lib
cargo check --locked
cargo clippy --locked --all-targets -- -D warnings
cargo build --locked --release
# Explicit parity-test inputs only, after a complete no-runtime app build.
python scripts/bundle_resources.py fetch --target "$TARGET"
# Keep real-CLI parity tests from oversubscribing cold WASM compilation.
cargo test --locked --no-default-features -- --test-threads=2
cargo test --locked --bin codestyles
python scripts/verify_dprint_pack.py --target "$TARGET" --work-dir /tmp/codestyles-dprint
python scripts/package_app.py --target "$TARGET" --output-dir dist
python scripts/check_distribution.py --target "$TARGET" --directory dist --audit-only
