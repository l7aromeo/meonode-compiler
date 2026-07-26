import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react-swc'

// Toggle the @meonode/compiler SWC plugin via MEONODE_COMPILER=0/1 so the
// same fixture can be built twice (plugin-on / plugin-off) for render parity
// comparison (Task 12).
const useMeonode = process.env.MEONODE_COMPILER !== '0'

export default defineConfig({
  plugins: [
    react({
      plugins: useMeonode ? [['@meonode/compiler', {}]] : [],
    }),
  ],
  resolve: {
    // @meonode/ui (an npm registry dependency — see package.json) can end up
    // with its own nested react/react-dom under its own node_modules if
    // bun's resolver doesn't hoist it to this app's copy. Without forcing
    // dedupe, Rollup bundles two independent react-dom instances, and
    // components using hooks from ui's copy (e.g. useEffectEvent in its
    // unmount-cleanup wrapper) read a dispatcher that this app's
    // actively-rendering react-dom never sets — "Cannot read properties of
    // null (reading 'useEffectEvent')". Next.js avoids this by aliasing
    // react/react-dom to the project's own copy unconditionally; Vite needs
    // it spelled out via `dedupe`.
    dedupe: ['react', 'react-dom'],
  },
  build: {
    // Deterministic, unhashed output filenames so the orchestrator can find
    // the built entry module without globbing.
    rollupOptions: {
      output: {
        entryFileNames: 'entry.js',
        chunkFileNames: 'chunk-[name].js',
        assetFileNames: 'asset-[name][extname]',
      },
    },
  },
})
