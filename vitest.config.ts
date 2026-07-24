import { defineConfig } from 'vitest/config'

// `@meonode/ui` is consumed here as a `file:` dependency pointing at the
// sibling `../ui` checkout (see package.json). Because that checkout has its
// own fully-installed `node_modules` (including its own copies of
// `react`/`react-dom`/`@emotion/*`), plain Node module resolution would give
// `@meonode/ui`'s imports a *different* React instance than the one this
// repo's test file uses directly — breaking hooks (`Invalid hook call`) the
// moment a component (e.g. `StyledRenderer`, `ThemeProvider`) calls
// `useContext`. `resolve.dedupe` forces every resolution of these packages
// (however deeply nested) back to this repo's own copies; `ssr.noExternal`
// is required alongside it so Vitest actually routes `@meonode/ui`'s module
// graph through Vite's resolver instead of handing it to plain Node `import`
// (which would ignore `dedupe` entirely).
export default defineConfig({
  resolve: {
    dedupe: ['react', 'react-dom', '@emotion/react', '@emotion/cache'],
  },
  ssr: {
    noExternal: ['@meonode/ui'],
  },
  test: {
    include: ['tests/**/*.test.ts'],
  },
})
