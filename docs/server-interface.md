# Public server interface foundation

The `server_interface` Rust module is an additive, transport-neutral core API.
It compiles without `desktop`, stays in the open repository, and can be reused
by future self-hosted adapters and separately maintained hosted implementations.
The native application and all its functionality remain unchanged. A private
repository must not become a dependency of self-hosting.

**This is not a running server, Web UI or login implementation.** No HTTP routes,
credentials, persistence, team policies or commercial feature boundaries are
introduced here. Capabilities advertise implemented operations, not installed
formatter languages or availability of a hosted product.

## Version 1 message contract

Requests are JSON with required integer `protocol_version` and tagged `operation`.
Unknown request fields/operations and duplicate fields are rejected. Clients
must tolerate additional response fields and check capabilities' protocol version
before other requests. Breaking wire changes require a new protocol version.

```json
{"protocol_version":1,"operation":{"type":"capabilities"}}
```

Other operations:
- `{"type":"standalone_config"}` returns the core's pinned standalone JSONC,
  exactly as the native application generates it.
- `{"type":"validate_config_syntax","config_jsonc":"{/* comment */}"}`
  returns `{"type":"config_syntax","valid":true}`. It uses the native editor's
  JSONC-object parser. It does **not** validate formatter options, reject every
  duplicate property, resolve references, authorize plugins or establish that a
  configuration is safe to execute. Unknown settings and comments are not rewritten.

`decode_request` caps encoded bodies at 1 MiB; `handle` caps decoded config at
256 KiB of UTF-8 and 64 combined object/array levels (including the root).
An iterative JSONC-aware guard checks depth before recursive CST parsing and
ignores delimiters in quoted strings and comments. Too-deep input returns
`config_too_deep`; malformed structural input returns `valid: false`.
Typed `ErrorCode` values are stable and contain no user content. A future transport
must map them to appropriate status/error envelopes. Direct callers of `handle`
bypass encoded-body parsing, but still receive protocol and decoded-config checks.

## Adapter responsibilities and next slices

A real self-hosted/hosted adapter still needs bounded streaming HTTP reads,
content-type/status mapping, authentication/session and authorization, origin/CSRF
rules appropriate to the chosen auth mechanism, rate/concurrency limits, safe
logging, deployment configuration and integration tests. No login method is
selected by this module; Web login must not be confused with core functionality.

Remote formatting is deliberately not enabled. Existing native FormatRequest
contains machine paths and a config snapshot with relative references. Never
serialize it directly or accept arbitrary executable/plugin paths, `extends`,
process plugins or project paths from HTTP clients. A formatting slice needs an
explicit managed-only configuration policy, isolated execution, resource limits
and tests before it can be advertised. A browser cannot access a native project
folder merely by sending its path. Preserve native project check/review/apply;
a future Web workspace/upload or local-agent design is separate work.

Any future formatting adapter must validate source and configuration before
invoking a formatter. Unsafe or unsupported inputs must return an error without
changing the original source or configuration. Preserve this validate-before-format
boundary; do not use successful formatting as evidence of validity. In particular,
Caddyfile checks must describe their actual syntax/safety coverage and must not be
presented as full Caddy semantic validation.

Verification: `cargo test --locked --no-default-features server_interface` and
`cargo test --locked --no-default-features --test server_interface_limits`.
The latter runs hostile nesting inputs in a subprocess, including both JSONC
containers and nested wire JSON. These are resource-bound regressions, not a
claim of network/server readiness.
