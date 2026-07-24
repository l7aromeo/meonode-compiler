#!/usr/bin/env node
// Renders the Vite fixture's built entry.js (plugin-on and plugin-off
// builds, produced separately as dist-on/ and dist-off/) inside jsdom via
// react-dom/client's createRoot, then compares the #root-marker subtree's
// innerHTML between the two builds (Task 12 parity check).
//
// This is the "least machinery" approach called for in the task: no
// puppeteer/playwright, no vitest — a plain Node script that boots jsdom
// globals, dynamic-imports the built ESM bundle (which runs
// createRoot(...).render(<App/>) itself, same as it would in a real
// browser), and reads back the DOM it produced.
import { readFile } from 'node:fs/promises'
import path from 'node:path'
import { JSDOM } from 'jsdom'

const APP_DIR = path.resolve(import.meta.dirname, '../vite-app')

async function renderBuild(distDirName) {
  const dom = new JSDOM('<!doctype html><html><body><div id="root"></div></body></html>', {
    url: 'http://localhost/',
    pretendToBeVisual: true,
  })

  const { window } = dom
  const define = (name, value) => Object.defineProperty(global, name, { value, configurable: true, writable: true })
  define('window', window)
  define('document', window.document)
  define('navigator', window.navigator) // Node >=21 has a built-in read-only `navigator` getter; must override via defineProperty
  define('HTMLElement', window.HTMLElement)
  define('Node', window.Node)
  define('customElements', window.customElements)
  define('requestAnimationFrame', (cb) => setTimeout(() => cb(Date.now()), 0))
  define('cancelAnimationFrame', (id) => clearTimeout(id))
  define('MutationObserver', window.MutationObserver)
  // Deliberately NOT overriding `performance`: jsdom's Performance.now()
  // recurses infinitely when detached from `window` and invoked as a bare
  // global function (a jsdom branding-check quirk). Node's own native
  // `performance.now()` (already global) works fine for React's scheduler.
  // MessageChannel/fetch: not provided by jsdom. Node has native fetch; the
  // scheduler falls back to setTimeout-based scheduling when MessageChannel
  // is absent from `global`, which is fine here (nothing depends on frame
  // timing), so it's intentionally left unset rather than polyfilled.

  const entryPath = path.join(APP_DIR, distDirName, 'entry.js')
  await import(`${'file://' + entryPath}?t=${Date.now()}`) // cache-bust: re-import both builds in one process

  // Flush any microtask-scheduled React work.
  await new Promise((resolve) => setTimeout(resolve, 50))

  const marker = window.document.getElementById('root-marker')
  if (!marker) {
    throw new Error(`#root-marker not found after rendering ${distDirName}/entry.js`)
  }
  const html = marker.outerHTML

  delete global.window
  delete global.document
  delete global.navigator
  delete global.HTMLElement
  delete global.Node
  delete global.customElements
  delete global.requestAnimationFrame
  delete global.cancelAnimationFrame

  return html
}

const [onHtml, offHtml] = await Promise.all([]).then(async () => {
  // Sequential, not parallel: each render mutates process-global `window`/
  // `document`, so two renders can't safely overlap in one process.
  const on = await renderBuild('dist-on')
  const off = await renderBuild('dist-off')
  return [on, off]
})

const equal = onHtml === offHtml

console.log('plugin-on  #root-marker length:', onHtml.length)
console.log('plugin-off #root-marker length:', offHtml.length)
console.log('PARITY:', equal ? 'MATCH' : 'MISMATCH')

if (!equal) {
  console.log('--- plugin-on ---')
  console.log(onHtml)
  console.log('--- plugin-off ---')
  console.log(offHtml)
  process.exit(1)
}

const entryOn = await readFile(path.join(APP_DIR, 'dist-on', 'entry.js'), 'utf8')
const entryOff = await readFile(path.join(APP_DIR, 'dist-off', 'entry.js'), 'utf8')
const markerRe = /__meo\$:1/
const transformAppliedOn = markerRe.test(entryOn)
const transformAppliedOff = markerRe.test(entryOff)

console.log('transform marker present in dist-on/entry.js:', transformAppliedOn)
console.log('transform marker present in dist-off/entry.js:', transformAppliedOff)

if (!transformAppliedOn || transformAppliedOff) {
  console.error('FAIL: transform-applied guard violated (plugin-on must have the marker, plugin-off must not)')
  process.exit(1)
}

console.log('OK: vite fixture parity + transform-applied guard both pass')
