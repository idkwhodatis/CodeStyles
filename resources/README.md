# Pinned offline formatter resources

Engine: **dprint 0.63.0**. Plugins: **TypeScript 0.96.1**, **JSON 0.25.2**.

`manifest.json` pins official GitHub release URLs, published SHA-256 digests,
archive sizes, and verified extracted executable digests. WASM and schemas are
vendored; platform binaries and source ZIPs are ignored build staging. The
packaged application must include the target `bin/` folder and both `plugins/`
and `schemas/` folders. A fresh application cache needs no network access to
format with these local WASM files. Do not use remote plugin URLs at runtime.

From repository root:

```
python3 scripts/bundle_resources.py fetch --target x86_64-unknown-linux-gnu
python3 scripts/bundle_resources.py verify --target x86_64-unknown-linux-gnu
python3 scripts/bundle_resources.py package --target x86_64-unknown-linux-gnu --output dist/offline-resources-linux-x64.zip
```

Omit `--target` on fetch or verify for all six platforms. `verify` and `package`
never access the network; missing or modified resources cause a hard failure.
`fetch` never updates the pinned manifest. Updates require explicitly reviewing
a new official release and regenerating reviewed hashes. ZIPs have fixed
metadata for reproducible packaging. This script packages formatter resources,
not the application UI or executable; application packaging must include them.

Supported resource targets: Linux x64/ARM64 GNU, macOS arm64/x64, Windows arm64/x64.
Only Linux x64 is executed during this build; other binaries are checksum-
verified official release artifacts, not claimed as runtime-tested.
