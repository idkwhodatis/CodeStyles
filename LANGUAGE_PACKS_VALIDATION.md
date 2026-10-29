# Language-pack implementation validation

Implementation was developed locally on `chatgpt-dot/language-packs`, based on
Python commit `8e839b040aac64c14e261948cfafaa1ec752312f`. The user subsequently
authorized direct master publication (superseding the draft-PR plan). The verified
product head is `0c582e518feea8ea151810e6eb51a99f2a826bd8`; connector publication
preserved the exact local tree, not local commit IDs. GitHub release assets still
require separate publishing approval.

## Verified locally (Linux x64)

- Deterministic Rust 1.99.1 language-pack candidate, SHA256
  `8487489db6b6021ee9bc531d932acf9ff5d9856d701d8b528d99ccd14500631b`,
  131,313,370 archive bytes. Exact installed members and source provenance are
  pinned separately in application/source manifests.
- Real isolated rustfmt execution without preinstalled Rust, including required
  compiler-driver and LLVM libraries and complete upstream license notices.
- Exact-digest local install; settings A/B/A and idempotence; edition/syntax
  diagnostics; Rust-only project Check, review, selected writes, automatic
  recheck; unchanged sibling modules/other languages/ignored files; stale source,
  added-file and ignore-rule rejection; uninstall followed by missing-tool error.
- Release desktop UI tests: **14 passed**, including highlighting for all eleven
  selectable languages. Release executable build passed.
- Actual release executable offline self-test: **10/10 bundled language variants**
  passed. Optional Rust is intentionally not required by an offline base package.
- Release desktop clippy (`--bin codestyles -- -D warnings`) passed.
- Final release core all-targets clippy (`--no-default-features -- -D warnings`)
  passed. The final strict-extractor real candidate E2E passed separately (1/1).
- Original packaging/script suite: **114 passed, 1 skipped** (115 discovered).
  After the five-platform scan and bounded Windows ARM64 PE parser were added,
  the final script suite passed **117 tests, 1 skipped** (118 discovered).
- `cargo fmt --all` and `git diff --check` used before commit.

The final release core Rust aggregate passed **164 tests**, with two explicit
ignored tests (a benchmark and the separately exercised native-bundle E2E).
Initial debug aggregate attempts exhausted the shared workspace disk during
linking. Only this task's regenerable compiler caches were removed; release cache
reuse and serial builds avoid a second large debug dependency graph. This was an
environment failure, not a passing test result.

## Security review corrections

An independent read-only pass found and coordinated fixes for: deleting an
already-activated runtime after a directory fsync error; ignored `.ignore` parse
errors; unbounded source hashing before the 32 MiB limit; and uncancellable final
status/uninstall waits. A final review also closed dangling-lock symlink creation
and bounded async downloader shutdown. Regression fixtures cover activation failure/commit
boundary, exact hashes, corruption, traversal, link and unknown-member rejection,
compressed/expanded/installed limits, locking, cancellation and protected cleanup.
The UI pins the chosen storage root and guards late file-dialog/confirmation
responses and shutdown. Only fixed embedded catalog data can authorize downloads.

## Explicit gaps / approval gates

- Final GitHub assets are **unpublished**; all target network availability flags
  remain false. No end-to-end production GitHub download claim is made.
- The initial bulk download was stopped after review rejection. After explicit
  user approval on October 29, the remaining five targets were inspected serially
  (331,130,168 source bytes). Later native CI execution is documented below;
  minimum-OS acceptance remains distinct from current hosted-runner acceptance.
- No minimum-OS/native-window visual acceptance claim. Current tests
  do not establish ZIP detection in every launcher, Windows ACL hardening, macOS
  signing/quarantine behavior or old-version cleanup across future manifest pins.
- Rust project mode is explicitly separate from dprint mode and rejects unsupported
  discovery/inheritance rather than pretending mixed-language parity.
- Existing Python and other WASM languages remain bundled/offline; migrating them
  to optional packs is future work, not an unnoticed removal in this change.

## Native candidate acceptance — October 29, 2026

At product head `0c582e518feea8ea151810e6eb51a99f2a826bd8`,
[manual native run 40841604286](https://github.com/idkwhodatis/CodeStyles/actions/runs/40841604286)
passed **5/5**: Linux ARM64, macOS x64/ARM64, and Windows x64/ARM64. Linux x64
remains separately backed by the local real-candidate execution described above,
including the post-fix seven-test Rust integration run. It was not a sixth job
in this manual workflow.

Each native report confirms `native_execution: passed`, `catalog_restored: true`,
`own_temporary_removed: true`, and `published: false`. The test installs the exact
candidate, executes its managed stable rustfmt with an isolated environment,
checks preview/edition/diagnostics, applies reviewed project writes, verifies
unselected files stay unchanged, and uninstalls the runtime. Candidate archive
hashes were reproducible across both native runs:

| Target | Candidate SHA256 |
| --- | --- |
| Linux ARM64 | `e4310214e01c6b4f8a4561d4f488340cdea785ecc599fba540043532c22f8102` |
| macOS x64 | `a9e745cc0fafb84f58fcab92b625f5ca76a8e93d1c35a3cf6662357c2da3db99` |
| macOS ARM64 | `338dde567e9a955c1d894e1a9885d82f4d6eee636281f1ac3ecca8d125bfc78b` |
| Windows x64 | `d4d75cfbb9b842c12f0944b154acd5b8eb3d6fa06551fad5304cc224c5fa3ad0` |
| Windows ARM64 | `3f2c912ec525cca77fa08688a246eb4c079252f3ba5f698061323d07be252eff` |

The first native run exposed Windows error 5 during project-source replacement.
The fix closes staging/identity handles before atomic replacement, rejects
read-only sources, and preserves stale/config/cancellation guards; it never
unlinks the original as a fallback. Both Windows native end-to-end tests now
pass, with additional ordinary replacement/readonly/stale regression tests.
The exact underlying Windows API failure was not separately instrumented.

The native-workflow helper suite increased the local Python script total to
**122 passed, 1 skipped** (123 discovered). The pure POSIX process-group test is
also intentionally skipped on Windows.

The same product head also passed **6/6**
[Native distribution checks, run 40841488662](https://github.com/idkwhodatis/CodeStyles/actions/runs/40841488662),
including native application builds, tests, packaging/audits and offline smoke
checks for Linux, macOS and Windows x64/ARM64. This is distinct from the optional
Rust candidate acceptance above. A later documentation-only commit may record
these results without rerunning unchanged product code.

### Remaining release gate

No candidate has been published as a GitHub release asset, and all production
catalog targets remain `available: false`. Publishing requires separate approval,
then reproducible six-target package assembly, exact hash/member/license review,
GitHub attachment upload, and a catalog update to verified fixed asset URLs.
A production download/install/cancel/upgrade acceptance pass must follow; the
current local/native candidate acceptance does not claim this network path works.
