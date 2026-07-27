import { defineConfig } from 'vitest/config'

// Guards against `@meonode/ui` resolving a *different* React instance than the
// test file itself uses, which breaks hooks (`Invalid hook call`) the moment a
// component (e.g. `StyledRenderer`, `ThemeProvider`) calls `useContext`.
//
// Currently redundant: `@meonode/ui` is a normal registry dependency, so it and
// its peers hoist into this repo's own `node_modules` and there is only one
// React copy. The suite passes with this whole block removed.
//
// Retained because the hazard returns the moment the dep is pointed at the
// sibling `../ui` checkout as a `file:` dependency — that checkout has its own
// fully-installed `node_modules` with its own `react`/`react-dom`/`@emotion/*`,
// and plain Node resolution then hands `@meonode/ui` the wrong React. The
// resulting failure is an opaque `Invalid hook call` with no pointer to module
// duplication, so the insurance is worth its two lines.
//
// `resolve.dedupe` forces every resolution of these packages (however deeply
// nested) back to this repo's copies; `ssr.noExternal` is required alongside it
// so Vitest routes `@meonode/ui`'s module graph through Vite's resolver instead
// of handing it to plain Node `import` (which would ignore `dedupe` entirely).
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
