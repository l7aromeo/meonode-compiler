# @meonode/compiler

**Experimental.** An SWC WASM plugin (Rust, compiled to `wasm32-wasip1`) that
rewrites `@meonode/ui` component call sites at build time so the
`@meonode/ui` runtime can skip its own per-render work.

## What it does, and why

Every `@meonode/ui` factory call (`Div({...})`, `P('text', {...})`,
`Node('div', {...})`, ...) normally does two things at runtime, on every
render: classify each prop as a static CSS prop vs. a dynamic/DOM prop, and
hash the prop signature to key the generated class name. Both are pure
functions of the call site's *source text* — they don't depend on runtime
values, only on which prop names appear and whether the object literal was
written with static shape. This plugin does that classification once, at
build time, and writes the answer directly into the call site as
pre-partitioned marker props:

```js
// Source:
Div({ padding: '20px', width, onClick: handler, css: { color: 'red' }, children: [A, B] })

// Compiled:
Div({
  __meo$: 1,
  c: { padding: '20px', width },
  d: { onClick: handler },
  k: 'm1a2b3c',
  dyn: ['width', 'onClick'],
  css: { color: 'red' },
  children: [A, B],
})
```

`@meonode/ui`'s runtime fast path (Phase 1 of this project — **requires a
`@meonode/ui` build with compiler runtime support**; at the time of writing
this has landed on an unreleased branch, not yet in a tagged `@meonode/ui`
release) detects the `__meo$` marker and uses `c`/`d`/`k`/`dyn` directly
instead of re-deriving them, skipping the classification and hashing pass
entirely. Measured on `@meonode/ui`'s own benchmark suite, the compiled path
constructs nodes **1.66x faster** than the uncompiled runtime-classification
path.

Call sites the plugin can't safely prove are order-independent are left
completely untouched (see [What gets compiled](#what-gets-compiled-vs-bailed)
below) — they keep working exactly as before, through the runtime's normal
classification path. Nothing about this plugin is required for correctness;
it's purely a build-time speedup for the call sites it can prove are safe.

## Install & configure

The plugin ships as a single `.wasm` file published under `@meonode/compiler`
(`main` points directly at the `.wasm`, no JS shim — see
[npm packaging](#npm-packaging)). It needs a host that can load an SWC WASM
plugin: Next.js (Turbopack or webpack) or Vite (via
`@vitejs/plugin-react-swc`).

### Next.js

```js
// next.config.js / next.config.mjs
export default {
  experimental: {
    // The package NAME, never an absolute path — Turbopack crashes on
    // absolute plugin paths.
    swcPlugins: [['@meonode/compiler', {}]],
  },
}
```

Verified against **Next 16.2.11**:

- **Turbopack** (`next build`, `next dev`) — the default bundler as of
  Next 16, and the primary target for this config shape.
- **webpack** (`next build --webpack`) — same `experimental.swcPlugins`
  config works unchanged; Next's webpack build path loads SWC plugins
  through the same `next-swc` binary as Turbopack does.

### Vite

Requires [`@vitejs/plugin-react-swc`](https://www.npmjs.com/package/@vitejs/plugin-react-swc)
(verified at `4.3.2`) — Vite's default `@vitejs/plugin-react` uses Babel, not
SWC, and can't load SWC plugins at all.

```js
// vite.config.js
import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react-swc'

export default defineConfig({
  plugins: [
    react({
      plugins: [['@meonode/compiler', {}]],
    }),
  ],
})
```

### If the plugin doesn't load

Bailing is always safe — see [Version compatibility](#version-compatibility)
below for the SWC WASM ABI caveat if the plugin fails to load under a given
host/`@swc/core` combination. Everything still runs correctly through
`@meonode/ui`'s normal runtime classification path; you just don't get the
build-time speedup.

## What gets compiled vs. bailed

The plugin only rewrites a call site when it can *prove* the rewrite is
evaluation-order-safe. Proof requires binding resolution (so a shadowed or
re-exported local named e.g. `Div` isn't mistaken for the real import) and a
structural check of the props object literal. Every prop value must be
provably free of side effects (a literal, identifier, arrow/function
expression, substitution-free template literal, or a nested object/array
literal built entirely from those) — because the rewrite reorders evaluation
(`c`-bucket props before `d`-bucket props, special keys moved to the tail).
Anything else bails the *entire call site*, leaving it byte-for-byte as
written; a bailed call behaves identically at runtime via `@meonode/ui`'s
normal classification path.

| Condition | Compiles? | Notes |
|---|---|---|
| Plain object literal, all values literals/idents/arrows/nested literals | Yes | The common case |
| `children` present and effectful, but last prop (or only prop) in source order | Yes | Special exception — see below |
| Callee is a shadowed/redeclared local, not the real `@meonode/ui` import | No (bail) | `ShadowedOrUnbound` |
| Callee via namespace import (`import * as M from '@meonode/ui'; M.Div(...)`) | No (bail) | `NamespaceImport` |
| No args, or nothing at the expected props argument position | No (bail) | `MissingPropsArg` |
| Props argument isn't a plain object literal (identifier, call, ternary, member expr, spread arg) | No (bail) | `NotObjectLiteral` |
| Object literal contains a spread property (`{ ...rest }`) | No (bail) | `SpreadProp` |
| Object literal contains a spread argument *before* the props position (e.g. `P(...stuff, {...})`) | No (bail) | `SpreadBeforeProps` — runtime arg count isn't statically known |
| Computed key (`{ [k]: v }`) | No (bail) | `ComputedKey` |
| Numeric/bigint literal key | No (bail) | `NumericKey` |
| Non-identifier-like string key (`{ 'foo-bar': 1 }`) | No (bail) | `NonIdentifierStringKey` |
| Getter/setter accessor property | No (bail) | `GetterSetterProp` |
| Shorthand method (`{ onClick() {} }`) | No (bail) | `MethodProp` |
| Any prop value not provably side-effect-free (calls, member access, `await`, `new`, assignments, conditionals, tagged templates, ...) | No (bail) | `EffectfulValue` |
| `children` effectful but **not** source-final | No (bail) | `EffectfulValue` — the tail-exception doesn't apply |
| Object literal already has a `__meo$` key | No (bail) | `ExistingMarker` — already compiled |

Special keys (`css`, `props`, `ref`, `key`, `children`, `as`, `theme`,
`disableEmotion`) are always left untouched at the top level of the emitted
object — they're moved to the tail in their original relative order, but
never bucketed into `c`/`d`.

## Marker contract

Compiled call sites get a `schema 1` marker object, recognized by the
`__meo$` key:

| Key | Meaning |
|---|---|
| `__meo$` | Marker schema version (currently always `1`). |
| `c` | Bucket of props recognized as CSS/static props (omitted if empty). |
| `d` | Bucket of props recognized as dynamic/DOM props (omitted if empty). |
| `k` | Deterministic call-site key (`m` + base36 FNV-1a64 hash of `filename:span`), used by the runtime to key the generated class name without re-hashing the prop signature. |
| `dyn` | Names of bucketed props (from `c` or `d`) whose value isn't a plain literal — i.e. props the runtime still needs to treat as reactive/dynamic, in first-occurrence source order (omitted if empty). |

Forward-compat policy: the runtime checks `__meo$` against the schema
version(s) it understands and falls back to its normal classification path
for anything it doesn't recognize — an unknown/future schema number is
ignored rather than crashing, so this plugin and the `@meonode/ui` runtime
can evolve the marker shape independently as long as both sides keep this
rule.

## Version compatibility

Plugin crate pinned to **`swc_core = "74"`** (currently resolving to
`74.0.2`), feature `ecma_plugin_transform`.

| Host | Version | Bundler | Status |
|---|---|---|---|
| Next.js | 16.2.11 | Turbopack | Verified |
| Next.js | 16.2.11 | webpack (`next build --webpack`) | Manually verified (not in automated e2e; Turbopack + Vite are) |
| Vite (`@vitejs/plugin-react-swc`) | 4.3.2 | esbuild/Rollup + SWC transform | Verified |
| `@swc/core` | 1.15.46 | (used directly by wasm-smoke tests) | Verified |

**SWC WASM ABI caveat**: the plugin/host boundary is a versioned wire
protocol (`ecma_plugin_transform`), not a linked ABI — the AST is serialized
across the WASM boundary, so a plugin built against a newer `swc_core` can
generally run under an older host as long as both sides speak a compatible
schema generation. This is *not* an unconditional guarantee: hosts do
occasionally bump their accepted plugin ABI generation in ways that reject
older or newer plugins outright. If the plugin fails to load under your
exact host/`@swc/core` version, check
[plugins.swc.rs](https://plugins.swc.rs) for the ABI/version compatibility
matrix before filing an issue — and remember bailing (plugin not loading at
all) is always safe; `@meonode/ui` runs correctly without it.

Release policy: a new `@meonode/compiler` release ships for every `swc_core`
minor version bump this crate adopts, so the published version compatibility
table above stays current with what's actually pinned in `Cargo.toml`.

## Development

```bash
bun install

bun run codegen        # regenerate css_props.rs + factories.rs from @meonode/ui
bun run check:drift     # regenerate + fail on any diff (run before releasing)
bun run build:wasm      # cargo build --release --target wasm32-wasip1, copies into npm/
bun run test            # cargo test --workspace + vitest (unit/fixture + wasm smoke + equivalence)
bun run test:e2e        # Next Turbopack + Vite real-build parity fixtures (slow — real bundler builds)
```

`@meonode/ui` is pinned to an exact prerelease version (currently
`1.7.0-beta.1`, the `beta` dist-tag) in the root `package.json` and in
`e2e/next-app`/`e2e/vite-app`'s `package.json`, rather than a semver range,
so `bun run test` / `bun run test:e2e` stay reproducible. This repo's compiler
runtime fast path depends on `@meonode/ui` runtime support that is still on
an unreleased branch (see the compatibility note above) — bump this pin
whenever `@meonode/ui` cuts a new `beta` (or, once the runtime support lands
on its default branch, switch to a real released version).

Test coverage as of this writing: 115 Rust tests (unit + SWC fixture tests),
24 Vitest tests (9 WASM artifact smoke tests via `@swc/core`'s real plugin
host, 8 server-side semantic-equivalence tests, 7 client-side), plus the
`test:e2e` Next/Vite real-bundler parity suite.

`crates/meonode-swc-plugin/src/css_props.rs` (689 recognized CSS properties)
and `src/factories.rs` (139 `@meonode/ui` HTML factories) are both generated
from `@meonode/ui`'s own exports — the same source of truth the runtime's
static/dynamic classification uses — and are committed to git. `check:drift`
regenerates both and fails the command if `@meonode/ui` has moved out from
under the committed snapshot; CI runs this in the `css-drift` job against a
source checkout of `@meonode/ui` (see `.github/workflows/ci.yml`), so drift
is caught on every push/PR, not just as a local/manual pre-release step.

### Publishing

Unlike `@meonode/ui`, this repo does not have `semantic-release` wired up
yet. The published version is a plain literal in `npm/package.json`
(currently `0.0.0-dev.0` — publishing is not live). `.github/workflows/ci.yml`
has a manual (`workflow_dispatch`-only) release job skeleton with the publish
steps stubbed out (npm OIDC trusted publishing, mirroring `@meonode/ui`'s own
release workflow — no `NPM_TOKEN` secret involved), gated behind its own
`if: false` until trusted publishing is configured on npmjs.com; nothing
publishes automatically. Cutting a real release for now means
manually bumping `npm/package.json`'s version and running
`npm publish --access public` from `npm/` by hand, after `bun run
check:drift` and `bun run test` both pass. Automating that (semantic-release
or otherwise) is future work, not v1.

## Layout

```
Cargo.toml                          workspace
rust-toolchain.toml
crates/meonode-swc-plugin/
  Cargo.toml                        crate-type = ["cdylib", "rlib"]
  src/lib.rs                        plugin entry point
  src/detect.rs                     call-site detection + bailout decisions
  src/effect.rs                     side-effect-freedom classifier
  src/partition.rs                  prop partitioning + marker emission
  src/css_props.rs                  @generated — see Development
  src/factories.rs                  @generated — see Development
  tests/                            SWC fixture tests
npm/
  package.json                      "@meonode/compiler", main → .wasm
  meonode_swc_plugin.wasm           build artifact, not committed to git
e2e/                                Next.js + Vite real-build parity fixtures
scripts/                            codegen for css_props.rs / factories.rs
.github/workflows/ci.yml            cargo test, wasm build, wasm smoke tests, e2e (gated), release skeleton (manual)
```

## License

MIT
