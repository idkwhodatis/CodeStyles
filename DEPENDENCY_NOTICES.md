# Targeted dependency notice audit

This change addresses the named app/dprint gaps from the source-level audit.
It is **not a complete SBOM, legal certification or runtime-release acceptance**.
All existing runtime availability flags, versions, binary/WASM hashes and
production candidate records remain unchanged. Packaging now fails if a required
notice/source pin is absent or its bytes do not match `resources/manifest.json`.

## Wasmtime and Cranelift: independently distributed dprint

The six official dprint 0.63.0 executables were downloaded through the committed
resource manifest, with both archive and extracted-executable SHA-256 checks.
Every Linux/macOS/Windows x86_64/aarch64 executable contains the strings
`wasmtime-49.0.2` and `cranelift-codegen-0.136.2`. This corroborates the pinned
source feature configuration and executable content, not just lockfile presence.
No executable was run for this inspection.

The pinned dprint source is
[11d00400e54c5c5d2b743f8237ea305de580514c](https://github.com/dprint/dprint/tree/11d00400e54c5c5d2b743f8237ea305de580514c).
Its `crates/dprint/Cargo.toml` enables Wasmtime's `cranelift` and `runtime` features
for all six supported native targets. `load_instance.rs` constructs and invokes
the engine. Exact Wasmtime 49.0.2 and cranelift-codegen 0.136.2 registry archives
were verified against that source's Cargo.lock; both ship the same full
Apache-2.0 WITH LLVM-exception text.

The unchanged text and source pins are now mandatory inputs to every dprint
candidate built by `scripts/build_dprint_pack.py`, recorded in its provenance
and included in the archive. Top-level MIT notices remain. This fixes the old
builder's selection of only six top-level license files. It does not inventory
all native dependencies, the Rust standard library, or the five WASM plugins.

## option-ext: source availability without overstating linkage

Both app and dprint lockfiles contain option-ext 0.2.0 (MPL-2.0). The exact
crates.io source archive matches checksum
`04744f49eae99ab78e0d5c0b603ab218f515ea8cfe5a456d7629ad883a3b6e7d`.
Its complete unchanged source and license are now shipped locally, with clear
instructions in `resources/licenses/option-ext-0.2.0-SOURCE.txt`; source access
does not depend on a future network request. Any future modified covered source
must be supplied too; this unmodified archive would not cover those changes.

Source reachability is narrower than the original lockfile finding:

- dirs-sys 0.4.1/0.5.0 use OptionExt only in `xdg_user_dirs.rs`, gated to Unix
  excluding macOS/iOS. Windows/macOS lock entries are not evidence of use.
- The app's dirs 6.0.0 path through rust-embed-impl/shellexpand is build-host code.
  shellexpand uses `home_dir`; zed-font-kit/dirs 5.0.1 uses `home_dir`/`data_dir`.
- dprint's inspected direct dirs 6.0.0 calls use `home_dir`, `config_dir` and
  `cache_dir`. These paths do not call the XDG user-directory parser.
- No option-ext marker was found in the six dprint executables. The absence of
  a string does **not** prove optimized/inlined code is absent.

Consequently, no actual option-ext retention conclusion is claimed, even for
Linux. The builder conservatively requires the MPL/source bundle for Linux;
app packaging also retains the source/legal directory on other targets. This
is not an assertion that unrelated CodeStyles files become MPL-licensed.

Exact inspected source archives:
[dirs-sys 0.4.1](https://static.crates.io/crates/dirs-sys/dirs-sys-0.4.1.crate),
[dirs-sys 0.5.0](https://static.crates.io/crates/dirs-sys/dirs-sys-0.5.0.crate),
[option-ext 0.2.0](https://static.crates.io/crates/option-ext/option-ext-0.2.0.crate).

## GPUI code and Lucide/Feather assets

App packages now require GPUI 0.7.3's exact Apache text and the exact composite
`LICENSE-LUCIDE` from gpui-kit-assets 0.7.3. That file includes Lucide's 2026 ISC
notice, the Feather-derived icon list, and Cole Bemis's MIT notice. Replacing it
with a generic current Lucide license would lose the pinned asset attribution.
Source archive checksums and the published GPUI revision are in
`resources/licenses/gpui-kit-assets-0.7.3-SOURCE.txt`.

The default desktop dependency enables asset support, and upstream provides
RustEmbed/include_bytes machinery. However, the app does not explicitly register
`Assets`/`with_assets`; those mechanisms alone do not establish which SVG bytes
survive final linking. Notices are conservatively included for all six desktop
targets without claiming the full icon catalog ships. Final app binaries and
link maps still need inspection. Other GPUI transitive licenses/fonts remain
part of the complete inventory gate.

## Validation and remaining gates

Offline regression coverage verifies exact upstream notice hashes, the packaged
MPL source archive and its license, target-specific selection, missing/corrupt
notice rejection, duplicate pin rejection, deterministic archives, and required
app notices on all six package targets. Run:

```sh
python -m unittest discover -s scripts -p 'test_*.py'
```

For actual dprint inputs, explicitly fetch the pinned resources, then build a
candidate with `scripts/build_dprint_pack.py` and inspect its members/provenance.
No fetch or build publishes, enables or installs a runtime. New notice bytes
change candidate hashes/sizes; prior candidate receipts/catalog pins do not
validate the rebuilt archive. The production catalog is deliberately untouched.

The full transitive notice inventory, source/link review for final app binaries,
native managed-runtime acceptance and GUI checks on the oldest supported OS for
all six targets remain open. Fixture executables validate packaging only, not
native startup. Nothing here resumes the paused Windows startup investigation.
