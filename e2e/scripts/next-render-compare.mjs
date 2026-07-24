#!/usr/bin/env node
// Compares the Next.js fixture's prerendered app-router HTML between a
// plugin-on build (.next-on/) and a plugin-off build (.next-off/), produced
// by running `next build` / `MEONODE_COMPILER=0 next build` and renaming the
// output directory each time (see e2e/scripts/run-e2e.mjs). Task 12 parity
// check, Next/Turbopack side.
//
// The whole prerendered document carries build-hashed asset URLs (chunk
// filenames, __next_f RSC payload ids) that legitimately differ between the
// two builds even when the compiler transform is a no-op at the DOM level.
// So instead of diffing the full document, this extracts just the
// #root-marker subtree (the app's own DOM, wrapped by the shared test page
// so the orchestrator can locate it) and compares that in isolation.
import { readFile } from 'node:fs/promises'
import path from 'node:path'

const APP_DIR = path.resolve(import.meta.dirname, '../next-app')

// Depth-counting <div> matcher rather than a regex: the marker's own
// children include further <div> elements (nested children, the `as`
// polymorphism case does NOT nest a div here, but Card and the inner onClick
// wrapper do), so the closing tag can't be found with a non-greedy regex.
function extractMarkerSubtree(html, sourceLabel) {
  const startIdx = html.indexOf('id="root-marker"')
  if (startIdx === -1) {
    throw new Error(`#root-marker not found in ${sourceLabel}`)
  }
  const divStart = html.lastIndexOf('<div', startIdx)
  const tagRe = /<div\b[^>]*>|<\/div>/g
  tagRe.lastIndex = divStart
  let depth = 0
  let end = -1
  let m
  while ((m = tagRe.exec(html))) {
    if (m[0].startsWith('</')) {
      depth--
      if (depth === 0) {
        end = tagRe.lastIndex
        break
      }
    } else {
      depth++
    }
  }
  if (end === -1) {
    throw new Error(`unbalanced <div> while extracting #root-marker from ${sourceLabel}`)
  }
  return html.slice(divStart, end)
}

async function loadMarkerHtml(buildDirName) {
  const file = path.join(APP_DIR, buildDirName, 'server/app/index.html')
  const html = await readFile(file, 'utf8')
  return extractMarkerSubtree(html, file)
}

async function findChunkFiles(buildDirName) {
  const { readdir } = await import('node:fs/promises')
  const root = path.join(APP_DIR, buildDirName, 'static/chunks')
  const out = []
  async function walk(dir) {
    for (const entry of await readdir(dir, { withFileTypes: true })) {
      const full = path.join(dir, entry.name)
      if (entry.isDirectory()) await walk(full)
      else if (entry.name.endsWith('.js')) out.push(full)
    }
  }
  await walk(root)
  return out
}

// `__meo$` alone is not a sufficient marker: @meonode/ui's own runtime bundles
// that same string as a constant it reads at render time, so it shows up in
// client chunks regardless of whether the compiler transform ran. What only
// the *transform* emits is the literal object property `__meo$:1` baked into
// a call site's args — check for that instead (matches the guard used by
// e2e/scripts/vite-render-compare.mjs).
const TRANSFORM_MARKER = /__meo\$:1/

async function countTransformMarker(buildDirName) {
  const files = await findChunkFiles(buildDirName)
  let count = 0
  for (const file of files) {
    const src = await readFile(file, 'utf8')
    const matches = src.match(TRANSFORM_MARKER)
    if (matches) count += matches.length
  }
  return count
}

const [onHtml, offHtml] = await Promise.all([loadMarkerHtml('.next-on'), loadMarkerHtml('.next-off')])

const equal = onHtml === offHtml

console.log('plugin-on  #root-marker length:', onHtml.length)
console.log('plugin-off #root-marker length:', offHtml.length)
console.log('PARITY:', equal ? 'MATCH' : 'MISMATCH')

if (!equal) {
  console.log('--- plugin-on ---')
  console.log(onHtml)
  console.log('--- plugin-off ---')
  console.log(offHtml)
}

const [onMarkerCount, offMarkerCount] = await Promise.all([
  countTransformMarker('.next-on'),
  countTransformMarker('.next-off'),
])

console.log('transform marker occurrences in .next-on/static/chunks:', onMarkerCount)
console.log('transform marker occurrences in .next-off/static/chunks:', offMarkerCount)

const guardOk = onMarkerCount > 0 && offMarkerCount === 0

if (!equal || !guardOk) {
  if (!guardOk) {
    console.error(
      'FAIL: transform-applied guard violated (plugin-on must have the marker, plugin-off must not — a 0/0 result would mean the plugin silently failed to load, making the parity match vacuous)',
    )
  }
  process.exit(1)
}

console.log('OK: next fixture parity + transform-applied guard both pass')
