// Fixture: theme-tokens-in-keys
//
// Guards the boundary of v0.4's build-time `theme.*` -> `var(--meonode-theme-*)`
// rewrite (see `theme.rs` / `partition::rewrite_theme_tokens_in_buckets`).
//
// The rewrite may only touch **bucketed prop values**. A theme token sitting in
// an object *key* must survive untouched, because CSS variables are invalid
// inside media-feature and selector text — `@media (max-width: var(--x))` simply
// does not match. `@meonode/ui` resolves such keys to concrete values at runtime
// via `ThemeUtil.resolveObjWithTheme`, which holds the live theme, and documents
// that same values-only invariant on `replaceThemeTokensWithCssVars`.
//
// Today the invariant holds structurally: media queries and pseudo-selectors
// live inside a `css:` block, `css` is a special key, and special keys are never
// bucketed — so the rewrite never sees them. On the real docs site every one of
// the 19 media-query theme tokens is inside a `css:` block. This fixture exists
// so that if anyone later widens the rewrite to recurse into nested objects, the
// breakage shows up here as diverging HTML rather than as silently dead
// responsive styles in production.
//
// Also covers two value cases worth pinning:
//   - a token embedded in a shorthand value ('1px solid theme.base.deep'),
//     which must be rewritten in place rather than wholesale-replaced
//   - `theme.mode`, which names something outside `theme.system`. Since
//     `buildThemeVariablesCss` only walks `theme.system`, no `:root` rule ever
//     defines `--meonode-theme-mode`. Both variants must be equally undefined:
//     the point is that compiling changes nothing, not that the reference works.
import { Div, ThemeProvider } from '@meonode/ui'
import type { Theme } from '@meonode/ui'

const theme: Theme = {
  mode: 'light',
  system: {
    base: { default: '#ffffff', deep: '#0b1724', content: '#111827' },
    primary: { default: '#4f46e5', content: '#ffffff' },
    spacing: { sm: '8px', md: '16px' },
    breakpoint: { md: '768px' },
  },
}

export default function ThemeTokensInKeys() {
  return ThemeProvider({
    theme,
    children: Div({
      // Bucketed values: all of these must come out as var() references.
      padding: 'theme.spacing.md',
      gap: 'theme.spacing.sm',
      color: 'theme.base.content',
      border: '1px solid theme.base.deep',
      // Names nothing under theme.system — must still compile to the same
      // (undefined) var reference the runtime would have produced.
      content: 'theme.mode',
      // A DOM attribute, not a CSS property: lands in the `d` bucket. The
      // runtime converts tokens in elementProps too, so compiled and
      // uncompiled must agree here as well.
      'data-token': 'theme.primary',
      // Special key: never bucketed, so the rewrite must not reach inside it.
      // The media-query *key* keeps its raw token and is resolved concretely at
      // runtime; only then does the breakpoint actually match.
      css: {
        backgroundColor: 'theme.base',
        '@media (max-width: theme.breakpoint.md)': {
          padding: 'theme.spacing.sm',
          color: 'theme.primary.content',
        },
        '&:hover': {
          backgroundColor: 'theme.primary',
        },
      },
      children: 'Themed box with responsive rules',
    }),
  })
}
