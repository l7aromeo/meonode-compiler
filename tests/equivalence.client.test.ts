// @vitest-environment jsdom
//
// Client-side companion to `equivalence.test.ts`'s mandatory server pass.
// Renders the same fixtures via `react-dom/client`'s `createRoot` into a
// jsdom container and compares `container.innerHTML` for the original vs
// compiled variant. This is NOT the correctness gate (the server pass is) —
// it's a secondary signal that hydration-shaped rendering (client
// `createRoot`, not `renderToString`) agrees too. See module docs in
// `equivalence.test.ts` for the full rationale behind the transform/import
// pipeline and the fresh-Emotion-cache-per-render isolation, both reused here
// unchanged.
import { existsSync } from 'node:fs'
import { mkdir, readFile, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { pathToFileURL } from 'node:url'
import createEmotionCache from '@emotion/cache'
import { CacheProvider } from '@emotion/react'
import { transform } from '@swc/core'
import { act, createElement } from 'react'
import { createRoot } from 'react-dom/client'
import { afterEach, beforeAll, describe, expect, it } from 'vitest'

const WASM_PATH = path.resolve(import.meta.dirname, '../npm/meonode_swc_plugin.wasm')
const FIXTURES_DIR = path.resolve(import.meta.dirname, 'equivalence-fixtures')
const TMP_DIR = path.resolve(import.meta.dirname, '.tmp')

if (!existsSync(WASM_PATH)) {
  throw new Error(`wasm artifact not found at ${WASM_PATH} — run \`bun run build:wasm\` first`)
}

const FIXTURES = [
  'static-styles',
  'dynamic-values',
  'theme-tokens',
  'css-prop-merge',
  'custom-factory',
  'nested-children',
  'as-polymorphism',
]

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

async function loadDefault(code: string, name: string, variant: 'original' | 'compiled'): Promise<() => unknown> {
  const file = path.join(TMP_DIR, `${name}.client.${variant}.mjs`)
  await writeFile(file, code, 'utf8')
  const mod = (await import(pathToFileURL(file).href)) as { default: () => unknown }
  return mod.default
}

interface RenderableNode {
  render(): Parameters<typeof createElement>[2] extends never ? never : ReturnType<typeof createElement>
}

beforeAll(async () => {
  await mkdir(TMP_DIR, { recursive: true })
})

afterEach(() => {
  document.body.innerHTML = ''
})

describe.each(FIXTURES)('semantic equivalence (client createRoot): %s', name => {
  it('original and compiled variants produce identical container.innerHTML', async () => {
    const fixturePath = path.join(FIXTURES_DIR, `${name}.ts`)
    const src = await readFile(fixturePath, 'utf8')

    const originalCode = await compile(src, `${name}.ts`, false)
    const compiledCode = await compile(src, `${name}.ts`, true)

    const originalDefault = await loadDefault(originalCode, name, 'original')
    const compiledDefault = await loadDefault(compiledCode, name, 'compiled')

    const originalTree = originalDefault() as RenderableNode
    const compiledTree = compiledDefault() as RenderableNode

    const originalContainer = document.createElement('div')
    document.body.appendChild(originalContainer)
    const originalCache = createEmotionCache({ key: 'css' })
    const originalRoot = createRoot(originalContainer)
    await act(async () => {
      originalRoot.render(createElement(CacheProvider, { value: originalCache }, originalTree.render()))
    })
    const originalHtml = originalContainer.innerHTML
    act(() => {
      originalRoot.unmount()
    })
    originalContainer.remove()

    const compiledContainer = document.createElement('div')
    document.body.appendChild(compiledContainer)
    const compiledCache = createEmotionCache({ key: 'css' })
    const compiledRoot = createRoot(compiledContainer)
    await act(async () => {
      compiledRoot.render(createElement(CacheProvider, { value: compiledCache }, compiledTree.render()))
    })
    const compiledHtml = compiledContainer.innerHTML
    act(() => {
      compiledRoot.unmount()
    })
    compiledContainer.remove()

    expect(compiledHtml).toBe(originalHtml)
  })
})
