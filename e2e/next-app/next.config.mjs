import path from 'node:path'
import { fileURLToPath } from 'node:url'

const __dirname = path.dirname(fileURLToPath(import.meta.url))

// Toggle the @meonode/compiler SWC plugin via MEONODE_COMPILER=0/1 so the
// same fixture can be built twice (plugin-on / plugin-off) for HTML parity
// comparison (Task 12). Package-name form: Next resolves '@meonode/compiler'
// via node_modules and reads its package.json "main" (-> the .wasm binary).
const useMeonode = process.env.MEONODE_COMPILER !== '0'

/** @type {import('next').NextConfig} */
const nextConfig = {
  // bun's `file:` installs symlink individual entries (not one directory
  // symlink) whose targets point at ../../npm (@meonode/compiler) and
  // ../../../ui (@meonode/ui) — outside e2e/next-app entirely. Turbopack
  // refuses to read through a symlink that resolves outside its project
  // root, so root must cover the common ancestor of both targets (the
  // @meonode/ directory, one level above this repo).
  turbopack: {
    root: path.resolve(__dirname, '../../../'),
  },
  experimental: {
    swcPlugins: useMeonode ? [['@meonode/compiler', {}]] : [],
  },
}

export default nextConfig
