// Semantic equivalence suite (Task 11) — THE correctness gate.
//
// For each fixture module under `equivalence-fixtures/`, this suite:
//   1. Transforms the fixture's source through @swc/core TWICE: once with the
//      real plugin (loaded from the built `npm/meonode_swc_plugin.wasm`
//      artifact, same as `wasm-smoke.test.ts`), once without it. Both passes
//      use the exact same jsc config (target es2022, TS syntax stripped) so
//      the *only* variable between "original" and "compiled" is whether the
//      plugin ran — isolating it as the single cause of any divergence.
//   2. Writes each transformed module to a temp `.mjs` file inside
//      `tests/.tmp/` (gitignored) so `@meonode/ui` resolves via this repo's
//      own `node_modules` (see package.json's `file:../ui` dependency), then
//      dynamically imports it.
//   3. Renders the fixture's default-exported NodeInstance tree via
//      `renderToString`, against the REAL `@meonode/ui` runtime (rebuilt
//      dist — see README/setup notes), and compares:
//        - the exact HTML string (byte-identical), and
//        - the set of Emotion-generated class names embedded in that HTML.
//
// Any divergence is a compiler bug by definition: the compiled call site is
// supposed to be behaviorally transparent to everything downstream. This
// suite must never be "fixed" by editing a fixture to dodge a real
// divergence — a genuine mismatch here means the compiler (or the runtime
// contract it targets) has a bug, and should be reported, not hidden.
//
// Each render uses its own freshly-created Emotion cache (via a fresh
// `<CacheProvider>`), because `@meonode/ui`'s plain-HTML-element rendering
// path (`StyledRenderer`) inserts styles through `@emotion/react`'s default
// cache-insertion machinery, which tracks "already inserted" state per
// `cache.inserted` map. Without isolating that per render, whichever variant
// happens to render *second* for a given fixture could look like it "lost" a
// `<style>` tag purely because the first render already registered that
// class — an artifact of shared process-global state, not a real compiler
// bug. A fresh cache per render removes that confound entirely.
import { existsSync } from 'node:fs'
import { mkdir, readFile, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { pathToFileURL } from 'node:url'
import createEmotionCache from '@emotion/cache'
import { CacheProvider } from '@emotion/react'
import { transform } from '@swc/core'
import { createElement } from 'react'
import { renderToString } from 'react-dom/server'
import { beforeAll, describe, expect, it } from 'vitest'

const WASM_PATH = path.resolve(import.meta.dirname, '../npm/meonode_swc_plugin.wasm')
const FIXTURES_DIR = path.resolve(import.meta.dirname, 'equivalence-fixtures')
const TMP_DIR = path.resolve(import.meta.dirname, '.tmp')

if (!existsSync(WASM_PATH)) {
  throw new Error(`wasm artifact not found at ${WASM_PATH} — run \`bun run build:wasm\` first`)
}

interface FixtureSpec {
  name: string
  /**
   * Whether the compiled variant is expected to contain the `__meo$` marker
   * anywhere in its output. Every fixture except `bailout-trailing-spread`
   * must compile at least one call site — this is the guard against the
   * whole suite silently going vacuously green if the plugin (or its wiring
   * here) started bailing on everything.
   */
  expectMarker: boolean
}

const FIXTURES: FixtureSpec[] = [
  { name: 'static-styles', expectMarker: true },
  { name: 'dynamic-values', expectMarker: true },
  { name: 'theme-tokens', expectMarker: true },
  { name: 'css-prop-merge', expectMarker: true },
  { name: 'custom-factory', expectMarker: true },
  { name: 'nested-children', expectMarker: true },
  { name: 'as-polymorphism', expectMarker: true },
  // v0.2: newly-compilable shapes (see the design doc's Change 1-3).
  { name: 'leading-spread', expectMarker: true },
  { name: 'bailout-trailing-spread', expectMarker: false },
  { name: 'keyed-list', expectMarker: true },
  { name: 'quoted-data-attribute', expectMarker: true },
]

/**
 * Transforms `src` through @swc/core, stripping TS types either way. Only
 * difference between variants: whether the real plugin artifact runs.
 */
async function compile(src: string, filename: string, withPlugin: boolean): Promise<string> {
  const result = await transform(src, {
    filename,
    jsc: {
      target: 'es2022',
      parser: { syntax: 'typescript', tsx: false },
      experimental: {
        plugins: withPlugin ? [[WASM_PATH, {}]] : [],
      },
    },
  })
  return result.code
}

/**
 * Writes `code` to a temp `.mjs` file inside `tests/.tmp/` (repo-rooted, so
 * `@meonode/ui` resolves through this repo's own `node_modules`) and
 * dynamically imports its default export.
 */
async function loadDefault(code: string, name: string, variant: 'original' | 'compiled'): Promise<() => unknown> {
  const file = path.join(TMP_DIR, `${name}.${variant}.mjs`)
  await writeFile(file, code, 'utf8')
  const mod = (await import(pathToFileURL(file).href)) as { default: () => unknown }
  return mod.default
}

// Matches both `@emotion/react`'s own default-cache class names (`css-xxx`)
// and meonode's server-computed ones (`meonode-css-xxx` — the match starts
// at the `css-` substring either way), which is all we need since we only
// compare the extracted sets between two renders of the very same fixture.
const CLASS_NAME_RE = /css-[a-z0-9]+/g

function extractClassNames(html: string): Set<string> {
  return new Set(html.match(CLASS_NAME_RE) ?? [])
}

interface RenderableNode {
  render(): Parameters<typeof createElement>[2] extends never ? never : ReturnType<typeof createElement>
}

/** Renders a NodeInstance to an HTML string under its own fresh, isolated Emotion cache. */
function renderNode(tree: RenderableNode): string {
  const cache = createEmotionCache({ key: 'css' })
  return renderToString(createElement(CacheProvider, { value: cache }, tree.render()))
}

beforeAll(async () => {
  await mkdir(TMP_DIR, { recursive: true })
})

describe.each(FIXTURES)('semantic equivalence: $name', ({ name, expectMarker }) => {
  it('original and compiled variants render byte-identical HTML with identical Emotion class names', async () => {
    const fixturePath = path.join(FIXTURES_DIR, `${name}.ts`)
    const src = await readFile(fixturePath, 'utf8')

    const originalCode = await compile(src, `${name}.ts`, false)
    const compiledCode = await compile(src, `${name}.ts`, true)

    if (expectMarker) {
      expect(compiledCode).toContain('__meo$')
    } else {
      expect(compiledCode).not.toContain('__meo$')
    }
    // The non-plugin pass must never emit the marker either — it's the fairness
    // control, proving the marker in `compiledCode` came from the plugin.
    expect(originalCode).not.toContain('__meo$')

    const originalDefault = await loadDefault(originalCode, name, 'original')
    const compiledDefault = await loadDefault(compiledCode, name, 'compiled')

    const originalTree = originalDefault() as RenderableNode
    const compiledTree = compiledDefault() as RenderableNode

    const originalHtml = renderNode(originalTree)
    const compiledHtml = renderNode(compiledTree)

    expect(compiledHtml).toBe(originalHtml)

    const originalClasses = extractClassNames(originalHtml)
    const compiledClasses = extractClassNames(compiledHtml)
    expect(compiledClasses).toEqual(originalClasses)
  })
})
