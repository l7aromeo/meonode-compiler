#!/usr/bin/env node
// Orchestrates Task 12's two e2e fixtures end to end:
//
//   1. bun install each fixture: @meonode/compiler is file:-linked against
//      ../../npm, @meonode/ui is an exact npm registry pin (see
//      e2e/*/package.json and the README > Development note on bumping it).
//   2. Build each fixture twice: once with the @meonode/compiler swc plugin
//      enabled, once with MEONODE_COMPILER=0 (plugin disabled), renaming the
//      bundler's output directory after each build so both copies survive
//      (bundlers overwrite their output directory on every run).
//   3. Run the matching *-render-compare.mjs script, which extracts the
//      shared test tree's #root-marker subtree from each build and asserts
//      byte-identical HTML/DOM plus the transform-applied guard (marker
//      present when the plugin ran, absent when it didn't).
//
// Wired up as the root `test:e2e` script (bun run test:e2e) — intentionally
// NOT part of the default `bun run test`, since spawning two real bundler
// builds is much slower than the unit/vitest suite.
//
// Next.js is built via Turbopack only here (the fixture's default `build`
// script); `build:webpack` / `build:webpack:no-plugin` exist in
// e2e/next-app/package.json for manual/CI diagnostic use if Turbopack ever
// rejects the plugin, but aren't part of this automated pass.
import { execFileSync } from 'node:child_process'
import { cp, rm } from 'node:fs/promises'
import path from 'node:path'

const SCRIPTS_DIR = import.meta.dirname
const E2E_DIR = path.resolve(SCRIPTS_DIR, '..')
const NEXT_APP_DIR = path.join(E2E_DIR, 'next-app')
const VITE_APP_DIR = path.join(E2E_DIR, 'vite-app')

function run(cmd, args, cwd, env) {
  console.log(`\n$ (${path.relative(E2E_DIR, cwd) || '.'}) ${cmd} ${args.join(' ')}`)
  execFileSync(cmd, args, { cwd, stdio: 'inherit', env: { ...process.env, ...env } })
}

async function replaceDir(from, to) {
  await rm(to, { recursive: true, force: true })
  await cp(from, to, { recursive: true })
  await rm(from, { recursive: true, force: true })
}

async function buildTwice({ label, cwd, outDir, buildScript, noPluginScript, onDir, offDir }) {
  console.log(`\n=== ${label}: plugin-on build ===`)
  run('bun', ['run', buildScript], cwd)
  await replaceDir(path.join(cwd, outDir), path.join(cwd, onDir))

  console.log(`\n=== ${label}: plugin-off build ===`)
  run('bun', ['run', noPluginScript], cwd)
  await replaceDir(path.join(cwd, outDir), path.join(cwd, offDir))
}

console.log('=== bun install: e2e/next-app ===')
run('bun', ['install'], NEXT_APP_DIR)

// bun installs a `file:` dep's node_modules entry as a symlink per
// top-level file rather than one directory symlink (see
// link-local-packages.mjs's own header comment for the full story).
// Turbopack's package.json reader can't parse that shape, so normalize
// @meonode/compiler (still file:-linked against ../../npm) into a plain
// directory symlink after every install — bun re-creates the broken shape
// on each `bun install`. @meonode/ui no longer needs this: it's an ordinary
// npm registry dependency now, installed as a normal package directory.
run(
  'node',
  [path.join(SCRIPTS_DIR, 'link-local-packages.mjs'), 'node_modules/@meonode/compiler', '../../npm'],
  NEXT_APP_DIR,
)

console.log('\n=== bun install: e2e/vite-app ===')
run('bun', ['install'], VITE_APP_DIR)

await buildTwice({
  label: 'next-app (Turbopack)',
  cwd: NEXT_APP_DIR,
  outDir: '.next',
  buildScript: 'build',
  noPluginScript: 'build:no-plugin',
  onDir: '.next-on',
  offDir: '.next-off',
})

await buildTwice({
  label: 'vite-app',
  cwd: VITE_APP_DIR,
  outDir: 'dist',
  buildScript: 'build',
  noPluginScript: 'build:no-plugin',
  onDir: 'dist-on',
  offDir: 'dist-off',
})

console.log('\n=== next-app parity check ===')
run('node', [path.join(SCRIPTS_DIR, 'next-render-compare.mjs')], E2E_DIR)

console.log('\n=== vite-app parity check ===')
run('node', [path.join(SCRIPTS_DIR, 'vite-render-compare.mjs')], E2E_DIR)

console.log('\nOK: e2e Next (Turbopack) + Vite parity fixtures both pass')
