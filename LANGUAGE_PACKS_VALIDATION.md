# Language-pack implementation validation

Local successor branch: `chatgpt-dot/language-packs`, based on Python commit
`8e839b040aac64c14e261948cfafaa1ec752312f`. Push and a draft PR were subsequently
authorized, but remote publication must be confirmed separately; this document
is local validation evidence, not proof that a branch or PR exists. GitHub release
assets still require separate publishing approval.

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
- Only Linux x64 native execution is verified. The initial bulk download was
  stopped after review rejection. After explicit user approval on October 29,
  the remaining five targets were inspected serially: 331,130,168 source bytes
  and selected members were checksum-verified, with no foreign binaries executed.
  See `TOOLCHAIN_PACKAGING.md` for static runtime closure findings and gaps.
  Native CI and minimum-OS execution remain distinct requirements.
- No six-platform/minimum-OS/native-window visual acceptance claim. Current tests
  do not establish ZIP detection in every launcher, Windows ACL hardening, macOS
  signing/quarantine behavior or old-version cleanup across future manifest pins.
- Rust project mode is explicitly separate from dprint mode and rejects unsupported
  discovery/inheritance rather than pretending mixed-language parity.
- Existing Python and other WASM languages remain bundled/offline; migrating them
  to optional packs is future work, not an unnoticed removal in this change.
