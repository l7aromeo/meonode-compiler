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
  // bun's `file:` install of @meonode/compiler symlinks individual entries
  // (not one directory symlink) whose target is ../../npm — this repo's own
  // npm/ directory, outside e2e/next-app entirely. Turbopack refuses to read
  // through a symlink that resolves outside its project root, so root must
  // cover that target: this repo's root, two levels up from e2e/next-app.
  // @meonode/ui is an ordinary npm registry dependency (see package.json),
  // not file:-linked, so it doesn't factor into this.
  turbopack: {
    root: path.resolve(__dirname, '../../'),
  },
  experimental: {
    swcPlugins: useMeonode ? [['@meonode/compiler', {}]] : [],
  },
}

export default nextConfig
