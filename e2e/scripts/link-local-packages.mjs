#!/usr/bin/env node
// Bun's `file:` installer links each entry of a local dependency individually
// (node_modules/@scope/pkg/package.json -> <target>/package.json, one symlink
// per top-level file), rather than making node_modules/@scope/pkg itself a
// single symlink to the target directory (what npm/yarn produce). Turbopack's
// package.json reader chokes on that per-entry shape whenever the symlink
// target resolves outside its configured `turbopack.root`:
//
//   Error parsing package.json file
//   package.json is not parseable: invalid JSON: a redirect can't be parsed as json
//
// This repo's e2e fixtures need `turbopack.root` widened to this repo's root
// (npm/ lives at ../../npm relative to e2e/next-app, outside its own
// directory) — see e2e/next-app/next.config.mjs. This script normalizes that
// one `file:` dep (currently just @meonode/compiler — @meonode/ui is an
// ordinary npm registry dependency now, installed as a normal package
// directory with no symlink involved) into a plain single directory symlink
// so Turbopack's parser sees an ordinary package the same way npm/yarn would
// have installed it — no functional change versus bun's own resolution
// (Node's require/import follow the same real files either way).
import { existsSync, lstatSync, realpathSync, rmSync, symlinkSync } from 'node:fs'
import path from 'node:path'

const args = process.argv.slice(2)
if (args.length === 0 || args.length % 2 !== 0) {
  console.error('usage: link-local-packages.mjs <link-path> <real-target-dir> [...pairs]')
  process.exit(1)
}

for (let i = 0; i < args.length; i += 2) {
  const linkPath = path.resolve(args[i])
  const realTarget = path.resolve(args[i + 1])

  if (!existsSync(realTarget)) {
    console.warn(`skip (target missing): ${realTarget}`)
    continue
  }
  if (existsSync(linkPath)) {
    const stat = lstatSync(linkPath)
    if (stat.isSymbolicLink() && realpathSync(linkPath) === realpathSync(realTarget)) {
      console.log(`already linked: ${linkPath} -> ${realTarget}`)
      continue
    }
  }
  rmSync(linkPath, { recursive: true, force: true })
  symlinkSync(realTarget, linkPath, 'dir')
  console.log(`linked ${linkPath} -> ${realTarget}`)
}
