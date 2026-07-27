// @vitest-environment jsdom
//
// Regression test for the stable-key hazard fixed alongside v0.2's leading-
// spread support (Change 2). See `cache-path-spread.ts` and the doc comment
// on `partition::rewrite_object` (crate side) for the full mechanism.
//
// Unlike `equivalence.test.ts`/`equivalence.client.test.ts` (which each
// render a fixture's tree exactly ONCE and compare original-vs-compiled),
// this test renders the SAME compiled call site TWICE, with different
// spread contents, into the SAME React root — reproducing the exact
// "re-render with changed spread contents" shape needed to exercise
// `BaseNode`'s `elementCache`. A single-render suite structurally cannot
// catch this: the bug is about what a *second* evaluation returns, not
// about whether the first one is correct.
//
// `BaseNode`'s `elementCache`/stable-key machinery only engages on the
// client (`NodeUtil.isServer` gates both `_getStableKey` and
// `shouldCacheElement` — see `src/util/node.util.ts`), so this must run
// under `@vitest-environment jsdom` (where `window` is defined), not the
// `renderToString` SSR path the other equivalence suites use.
import { existsSync } from 'node:fs'
import { mkdir, readFile, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { pathToFileURL } from 'node:url'
import createEmotionCache from '@emotion/cache'
import { CacheProvider } from '@emotion/react'
import { transform } from '@swc/core'
import { act, createElement } from 'react'
import { createRoot } from 'react-dom/client'
import { beforeAll, describe, expect, it } from 'vitest'
// Imported directly from the real `@meonode/ui` package (not through a
// compiled fixture) purely to reset `BaseNode`'s process-global caches
// between test runs — irrelevant to the bug itself, just test hygiene.
import { Node } from '@meonode/ui'

const WASM_PATH = path.resolve(import.meta.dirname, '../npm/meonode_swc_plugin.wasm')
const FIXTURES_DIR = path.resolve(import.meta.dirname, 'equivalence-fixtures')
const TMP_DIR = path.resolve(import.meta.dirname, '.tmp')

if (!existsSync(WASM_PATH)) {
  throw new Error(`wasm artifact not found at ${WASM_PATH} — run \`bun run build:wasm\` first`)
}

async function compileWithPlugin(src: string, filename: string): Promise<string> {
  const result = await transform(src, {
    filename,
    jsc: {
      target: 'es2022',
      parser: { syntax: 'typescript', tsx: false },
      experimental: {
        plugins: [[WASM_PATH, {}]],
      },
    },
  })
  return result.code
}

interface RenderableNode {
  render(): Parameters<typeof createElement>[2] extends never ? never : ReturnType<typeof createElement>
}

type MakeRow = (extra: Record<string, unknown>) => RenderableNode

async function loadMakeRow(code: string, cacheBust: string): Promise<MakeRow> {
  const file = path.join(TMP_DIR, `cache-path-spread.compiled.${cacheBust}.mjs`)
  await writeFile(file, code, 'utf8')
  const mod = (await import(pathToFileURL(file).href)) as { makeRow: MakeRow }
  return mod.makeRow
}

beforeAll(async () => {
  await mkdir(TMP_DIR, { recursive: true })
})

describe('stable-key hazard: leading spread + deps array cache path', () => {
  it('reflects new spread contents on a second render, rather than reusing a stale cached element', async () => {
    // Fresh caches for this test run — `BaseNode`'s caches are process-global
    // (see `getGlobalState`), so a prior test's cached elements must not
    // leak in here.
    Node.clearCaches()

    const fixturePath = path.join(FIXTURES_DIR, 'cache-path-spread.ts')
    const src = await readFile(fixturePath, 'utf8')
    const compiledCode = await compileWithPlugin(src, 'cache-path-spread.ts')

    // Sanity: this call site must actually be compiled (marker present) —
    // otherwise this test would vacuously pass by exercising the
    // never-shortcut-taking legacy path regardless of the fix.
    expect(compiledCode).toContain('__meo$')
    // And, per the fix, `k` must be absent (spread present) — asserting
    // this pins down *why* the regression is closed, not just *that* it is.
    expect(compiledCode).not.toMatch(/\bk:\s*"m/)

    const makeRow = await loadMakeRow(compiledCode, 'run')

    const container = document.createElement('div')
    document.body.appendChild(container)
    const cache = createEmotionCache({ key: 'css' })
    const root = createRoot(container)

    // First render: spread contributes `id: 'row-red'`. Same (empty, shallow-
    // equal) `deps` array both times — see `cache-path-spread.ts`'s `makeRow`.
    await act(async () => {
      root.render(createElement(CacheProvider, { value: cache }, makeRow({ id: 'row-red' }).render()))
    })
    const firstHtml = container.innerHTML
    expect(firstHtml).toContain('id="row-red"')

    // Second render: SAME call site, DIFFERENT spread contents, into the
    // SAME root. Before the fix, `k` (identical across both calls — it only
    // depends on call-site source position) plus a shallow-equal `deps`
    // array meant `BaseNode.render()` would find and reuse the FIRST
    // render's cached element here, incorrectly keeping `id="row-red"`.
    await act(async () => {
      root.render(createElement(CacheProvider, { value: cache }, makeRow({ id: 'row-blue' }).render()))
    })
    const secondHtml = container.innerHTML

    expect(secondHtml).toContain('id="row-blue"')
    expect(secondHtml).not.toContain('id="row-red"')

    act(() => {
      root.unmount()
    })
    container.remove()
  })
})
