// End-to-end benchmark for v0.4's build-time theme-token rewrite.
//
// Unlike the density experiment that motivated the feature (which hand-wrote
// marker props to model compiled output), this drives the *real* wasm plugin:
// the same source is transformed twice, once with the plugin and once without,
// and both are rendered through `renderToPipeableStream` under production
// React/Emotion. So the number here is what the transform actually buys, not
// what a model of it predicted.
//
// The tree's token density (~1 token per call site) is matched to the real
// @meonode/ui docs site, which averages ~0.8 quoted token strings across its
// 872 compiled call sites. Density matters a lot: an earlier run at 7 tokens
// per node overstated the win by roughly 2x.
//
// Pass `--no-tokens` to run the identical tree with every theme token replaced
// by its literal equivalent. The plugin then only partitions props, so the gap
// between the two runs isolates the theme rewrite's own contribution from the
// prop-partitioning gain that shipped earlier.
//
// Usage: node e2e/bench-theme-tokens.mjs [iterations] [--no-tokens]
import { transform } from '@swc/core'
import { mkdirSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { pathToFileURL } from 'node:url'
import { Writable } from 'node:stream'

if (process.env.NODE_ENV !== 'production') {
  throw new Error('run with NODE_ENV=production — dev builds of React/Emotion add validation overhead that no deployed site pays')
}

const ITERATIONS = Number(process.argv[2] ?? 400)
const NO_TOKENS = process.argv.includes('--no-tokens')
const ROOT = path.resolve(import.meta.dirname, '..')
const WASM = path.join(ROOT, 'npm/meonode_swc_plugin.wasm')
const TMP = path.join(ROOT, 'e2e/.bench-tmp')

// Depth 3 / breadth 5 => 156 nodes. One theme token per node, the rest plain
// literals, so prop count is identical across variants and only the token
// handling differs.
const SOURCE_TEMPLATE = `
import { Div, ThemeProvider } from '@meonode/ui'

const theme = {
  mode: 'light',
  system: {
    base: { default: '#ffffff', deep: '#0b1724', content: '#111827' },
    primary: { default: '#4f46e5', content: '#ffffff' },
    spacing: { sm: '8px', md: '16px' },
    text: { md: '15px' },
    radius: { md: '8px' },
  },
}

function buildTree(depth, id) {
  const children = depth > 0
    ? Array.from({ length: 5 }, (_, i) => buildTree(depth - 1, id + '.' + i))
    : ['leaf ' + id]
  return Div({
    display: 'flex',
    flexDirection: 'column',
    padding: 'theme.spacing.md',
    gap: '8px',
    color: '#111827',
    backgroundColor: '#ffffff',
    fontSize: '15px',
    borderRadius: '8px',
    border: '1px solid #0b1724',
    id: 'n-' + id,
    'data-depth': depth,
    children,
  })
}

export default function Page() {
  return ThemeProvider({ theme, children: buildTree(3, 'r') })
}
`

// '16px' is exactly what theme.spacing.md resolves to, so the two modes render
// visually identical trees and differ only in how the value is obtained.
const SOURCE = NO_TOKENS ? SOURCE_TEMPLATE.replace(/'theme\.spacing\.md'/g, "'16px'") : SOURCE_TEMPLATE

if (NO_TOKENS && SOURCE.includes('theme.spacing.md')) {
  throw new Error('--no-tokens failed to strip the token: the two modes would be identical and the comparison meaningless')
}

async function load(withPlugin) {
  const { code } = await transform(SOURCE, {
    filename: 'bench-tree.ts',
    jsc: {
      target: 'es2022',
      parser: { syntax: 'typescript', tsx: false },
      experimental: { plugins: withPlugin ? [[WASM, {}]] : [] },
    },
  })
  mkdirSync(TMP, { recursive: true })
  const file = path.join(TMP, `${withPlugin ? 'compiled' : 'original'}.mjs`)
  writeFileSync(file, code, 'utf8')
  const markers = (code.match(/__meo\$:/g) ?? []).length
  if (withPlugin && markers === 0) {
    throw new Error('plugin produced no markers — benchmark would compare identical code and report a meaningless ~1.0x')
  }
  const varRefs = (code.match(/var\(--meonode-theme-/g) ?? []).length
  if (withPlugin && !NO_TOKENS && varRefs === 0) {
    throw new Error('plugin produced no var() rewrites — the very thing being measured did not happen')
  }
  if (withPlugin && NO_TOKENS && varRefs > 0) {
    throw new Error('--no-tokens mode still produced var() rewrites, so it is not isolating anything')
  }
  const mod = await import(pathToFileURL(file).href)
  return { render: mod.default, markers, varRefs }
}

const { renderToPipeableStream } = await import('react-dom/server')

const renderOnce = element =>
  new Promise((resolve, reject) => {
    const { pipe } = renderToPipeableStream(element, { onError: reject })
    const out = new Writable({ write: (_c, _e, cb) => cb() })
    out.on('finish', resolve)
    pipe(out)
  })

async function bench(entry) {
  for (let i = 0; i < 20; i++) await renderOnce(entry.render().render())
  const batches = []
  for (let b = 0; b < 5; b++) {
    const t0 = performance.now()
    for (let i = 0; i < ITERATIONS; i++) await renderOnce(entry.render().render())
    batches.push((performance.now() - t0) / ITERATIONS)
  }
  batches.sort((a, b) => a - b)
  return batches[2]
}

const original = await load(false)
const compiled = await load(true)

// Interleave a warmup of both before measuring either, so neither variant pays
// for the other's JIT tiering.
await bench({ render: original.render })
await bench({ render: compiled.render })

const originalMs = await bench(original)
const compiledMs = await bench(compiled)

console.log(
  JSON.stringify(
    {
      mode: NO_TOKENS ? 'partitioning only (tokens stripped)' : 'partitioning + theme rewrite',
      iterations: ITERATIONS,
      markers: compiled.markers,
      varRewrites: compiled.varRefs,
      originalMsPerRender: +originalMs.toFixed(4),
      compiledMsPerRender: +compiledMs.toFixed(4),
      speedup: +(originalMs / compiledMs).toFixed(3),
      percentFaster: +(((originalMs - compiledMs) / originalMs) * 100).toFixed(1),
    },
    null,
    2,
  ),
)
