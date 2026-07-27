// Fixture 7: leading-spread
//
// `{ ...extra, backgroundColor: 'yellow' }`: the spread precedes every
// static prop, so it compiles (v0.2 Change 2) by leaving the spread
// top-level in the emitted object and bucketing only the static props
// (the schema marker, then the spread, then a `c` bucket holding
// `backgroundColor`). The runtime's own
// `{ ...passthroughCss, ...markerCss, ...css }` precedence
// (`processProps`'s compiled-marker fast path) reproduces plain-JS
// "later key wins" semantics exactly: `backgroundColor` from `c` wins over
// whatever `extra` contributes. Renamed from `bailout-spread`, which this
// shape used to be under v1 (any spread bailed unconditionally) — see
// `bailout-trailing-spread.ts` for the mirror-image case that still must
// bail.
//
// Note there's no call-site key (`k`) at all here — a spread's contents
// aren't known until runtime, so `k` (which only depends on source
// position) is never emitted when a spread is present. See
// `cache-path-spread.ts` for the regression test this closes: without it,
// two evaluations of the same call site with different spread contents
// would collide on an identical stable key.
import { Div } from '@meonode/ui'

const extra = { padding: '5px', id: 'spread-target' }

export default function LeadingSpread() {
  return Div({
    ...extra,
    backgroundColor: 'yellow',
  })
}
