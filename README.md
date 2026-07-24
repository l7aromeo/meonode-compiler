# @meonode/compiler

An SWC WASM plugin (Rust, compiled to `wasm32-wasip1`) that rewrites
`@meonode/ui` component call sites into pre-partitioned marker props at
build time — `{ __meo$: 1, c: {...}, d: {...}, k: '...', dyn: [...] }` —
so the `@meonode/ui` runtime can skip its own static/dynamic prop analysis
on every render. It is loaded by:

- **Next.js**, via `experimental.swcPlugins: [['@meonode/compiler', {}]]`
  (the npm package *name*, never an absolute path — Turbopack crashes on
  absolute plugin paths).
- **Vite**, via `@vitejs/plugin-react-swc`'s `plugins` option.

This package currently ships only a passthrough (no-op) transform. The
real call-site rewrite lands in later tasks.

## swc_core version

Pinned: **`swc_core = "74"`** (feature `ecma_plugin_transform`), currently
resolving to `74.0.2`.

### Evidence

| Source | swc_core pin | Notes |
|---|---|---|
| `swc-project/plugins` (official community plugins monorepo — styled-components, emotion, etc.) `Cargo.toml` `[workspace.dependencies]` | `74.0.0` | This is the reference convention for third-party plugin authors today. |
| `@swc/core` npm, `latest` dist-tag (`1.15.46`, published 2026-07-19) | `74.0.1` | `@swc/core` and `swc_core` are released together from the `swc-project/swc` monorepo; this is the host used by `@vitejs/plugin-react-swc` for Vite. |
| `vercel/next.js`, `canary` branch `Cargo.toml` (heading to Next 16.3) | `73.0.0` | One release train behind the above, same plugin ABI generation. |
| `vercel/next.js`, tag `v16.2.11` (current `latest` stable on npm) `Cargo.toml` | `57.0.0` | The `next-swc` binary actually bundled with the Next.js release most people run today. |

### The version gap, and why 74 was still chosen

Next.js 16 stable's bundled `next-swc` (57.0.0) is materially behind
`@swc/core`/Vite (74.x) and Next's own canary (73.0.0). The SWC plugin
system is designed around a versioned wire schema between host and guest
(`ecma_plugin_transform`, schema v1) specifically so a plugin built with a
newer `swc_core` can run under an older host — the AST is serialized
across the WASM boundary rather than linked, so exact crate-version
parity between host and plugin is not required, only schema compatibility.

Given that, `74` was chosen because it is the version the SWC team's own
current reference plugins (and current `@swc/core`/Vite) build against —
i.e. the version most likely to receive continued documentation, examples,
and bugfixes going forward. **This has not yet been empirically verified
against a real Next.js 16.2.11 install** (only against `cargo build`).
Fixture tests against both Next.js 16 stable and Next.js canary/Vite
(Task 8+) should confirm this pin actually loads and runs on both hosts;
if Next 16.2.11 stable rejects the plugin, the fallback is to pin to
`57` (or whatever `next-swc` in the target Next release uses) instead.

### Toolchain

`rust-toolchain.toml` pins `channel = "stable"` with the `wasm32-wasip1`
target. `swc_core`'s declared `rust-version` (MSRV) is `1.70`; the
community plugins repo pins a nightly toolchain, but that appears to be
for their own internal lint/format CI matrix, not a hard build requirement
of `swc_core` itself — stable Rust (verified locally at 1.97.1) builds the
plugin without issue.

## npm packaging convention

Followed the convention used by `@swc/plugin-styled-components` (and the
rest of `swc-project/plugins`): `package.json` `"main"` points **directly**
at the compiled `.wasm` file (`meonode_swc_plugin.wasm`) — no JS shim, no
`exports` map. `preferUnplugged: true` is set for Yarn PnP. The `.wasm`
binary is a build artifact (produced by `npm run build` in `npm/`, which
runs `cargo build --release --target wasm32-wasip1` and copies the output
in) and is never committed to git.

## Layout

```
Cargo.toml                          workspace
rust-toolchain.toml
crates/meonode-swc-plugin/
  Cargo.toml                        crate-type = ["cdylib", "rlib"]
  src/lib.rs                        passthrough #[plugin_transform]
  src/css_props.rs                  @generated — see below
  tests/fixture/                    fixture tests land in Task 8
npm/
  package.json                      "@meonode/compiler", main → .wasm
scripts/codegen-css-set.ts          codegen for css_props.rs (see below)
.github/workflows/ci.yml            cargo test + wasm32-wasip1 release build
```

`crates/meonode-swc-plugin/src/css_props.rs` is generated (`bun run codegen`,
i.e. `scripts/codegen-css-set.ts`) from `@meonode/ui`'s exported CSS property
set — the single source of truth the runtime's static/dynamic prop
classification also uses — and is committed to git; `bun run check:drift`
regenerates it and fails on any diff, but CI does not yet run that check
since the workflow doesn't check out a `@meonode/ui` ref (see the `if: false`
TODO in `.github/workflows/ci.yml`), so drift detection is currently
local/manual only.
