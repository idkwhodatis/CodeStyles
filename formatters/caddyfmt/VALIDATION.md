# Offline Caddy formatter validation

These checks do not download, install, publish, enable packs, load Caddy configs,
run shell snippets from fixtures, or modify the runtime catalog. Supply trusted
local executable paths and their independently verified SHA-256 hashes. The
scripts execute only dprint formatting and optional official `caddy fmt -`.

## Fixtures and provenance

`tests/fixtures/caddyfmt/upstream-cases.json` contains all 34 cases extracted from
Caddy 2.11.7 `caddyconfig/caddyfile/formatter_test.go`, at commit
`72dd0fb067f6d7826c7f79907670ba4a713bfe37` (Git blob
`7eef04766b4229fb434d689eb93186c842193080`). These upstream cases are Apache-2.0;
see `licenses/Caddy-Apache-2.0.txt`. The corresponding original source remains in
`internal/caddyfile/formatter_test.go` with its copyright/license notice.

`oracle-cases.json` records 560 official-Caddy one-pass outputs: those 34 cases,
14 explicit edge cases, and 512 deterministic seed-2117 generated cases. The
Python script's `generated_cases()` reconstructs all inputs in the same order.
The oracle covers malformed syntax too; this validates formatter equivalence,
not acceptance by CodeStyles' stricter source guards or Caddy semantic validity.
Both JSON fixtures together are under 500 KB.

## Replay

From the repository root, set `WASM`, `DPRINT`, and optionally `CADDY` to local
absolute paths. Get their hashes from the reviewed source/catalog pins or from
an independently verified official archive, rather than trusting an arbitrary
binary merely because a locally calculated hash matches itself.

For the original reviewed Linux x86_64 candidate/reference:

- WASM: `0ed1cea807384b5b79fb172e8619679f9563aa594c3fba4b53635c3fa88be9a5`
- dprint 0.63.0 executable: `43611a9705843e43c2798552592d78d0a2a67de2785226f1bfed74aa8c4cd657`
- Official Caddy 2.11.7 executable: `678ade3bfc088749c81a681adc603333ee0bb023b6a6cfe3c0f58bef8ff854e9`

These executable hashes apply only to those Linux x86_64 downloads, not other
platforms. The current authoritative WASM pin is in
`resources/caddy-formatter-sources.json`; a reviewed update can supersede the
historical candidate hash above.

```sh
node scripts/verify_caddy_formatter_wasm.mjs \
  --wasm "$WASM" --sha256 "$WASM_SHA256" \
  --oracle tests/fixtures/caddyfmt/oracle-cases.json \
  --report /tmp/caddy-wasm-report.json

python3 scripts/verify_caddy_formatter_dprint.py \
  --wasm "$WASM" --wasm-sha256 "$WASM_SHA256" \
  --dprint "$DPRINT" --dprint-sha256 "$DPRINT_SHA256" \
  --report /tmp/caddy-dprint-report.json
```

The Node check verifies no host imports, schema-v4 ABI behavior, exact fixture
outputs, configuration/override/range errors and recovery, 10,000 repeated
calls, registration/release, input/output boundaries, and bounded nested-brace
probes. It runs in a disposable Node subprocess with a 120-second timeout
(configurable with `--timeout`, maximum 300 seconds). Requires Node with native
WebAssembly support.

The Python check uses an isolated temporary configuration/home/cache and the
same one-pass dprint flags as the application, including
`--skip-stable-format`. Add `--caddy "$CADDY" --caddy-sha256 "$CADDY_SHA256"`
to independently replay every expected result through the official binary.
Each subprocess has a 10-second timeout (configurable up to 60 seconds).

## Deterministic oracle regeneration

To reconstruct all 560 inputs and obtain their expected output from trusted
official Caddy, pass both Caddy flags and an explicit output path:

```sh
python3 scripts/verify_caddy_formatter_dprint.py \
  --wasm "$WASM" --wasm-sha256 "$WASM_SHA256" \
  --dprint "$DPRINT" --dprint-sha256 "$DPRINT_SHA256" \
  --caddy "$CADDY" --caddy-sha256 "$CADDY_SHA256" \
  --write-oracle /tmp/caddy-oracle-replayed.json
cmp tests/fixtures/caddyfmt/oracle-cases.json /tmp/caddy-oracle-replayed.json
```

Original upstream expected outputs must match the official binary, and every
new oracle output must also match the WASM plugin through dprint before the
output file is written. Regeneration never overwrites the tracked oracle unless
you explicitly choose its path. A plugin change should not be accepted by merely
regenerating expected results; review source identities and actual differences.

## Resource limits and remaining release gates

Nested-brace tests use depths 64, 256, 1000, and 2000, with at most 12,002 input
bytes and an 8 MiB expected-output ceiling. The upstream formatter caps indentation nesting at ten; these probes test
saturation and recovery, not unbounded quadratic indentation. Ordinary oracle inputs are capped at
64 KiB, outputs at 8 MiB, and case count at 1000. The Node boundary test separately
uses up to 32 MiB + 1 input bytes and checks oversized-transfer rejection.

These are deliberately bounded regression tests, not a hostile-input OOM test.
Earlier boundary runs reached 274,726,912 bytes of guest linear memory. Reported
linear memory is neither host RSS nor a worst-case bound. Neither test harness
nor the application imposes a host-enforced memory ceiling; Python temporary
output files also have no filesystem quota. Timeouts do not prevent short-lived
memory/output spikes. A formal resource policy, final license audit, native and
oldest-supported-OS tests, and release approval remain separate gates. All packs
must remain disabled until those gates are satisfied.
