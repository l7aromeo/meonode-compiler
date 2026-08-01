// Fixture: key-only-trailing-spread
//
// A spread AFTER a static prop (`{ backgroundColor: 'yellow', ...extra }`)
// cannot be *partitioned* (`BailReason::TrailingSpread`) — the spread would
// need to win over the preceding static prop for correct precedence, which the
// emitted `{ ...passthroughCss, ...markerCss, ...css }` merge order can't
// express (compiler-bucketed static props always come after the spread in the
// emitted shape).
//
// It is still *keyed*, as schema 3: the call-site key is a hash of filename and
// span and needs no knowledge of the props. The marker is appended after the
// spread so the spread cannot shadow it. Props are classified at runtime
// exactly as they would be uncompiled, so the rendered output must be
// byte-identical to the original — which is what this fixture checks.
import { Div } from '@meonode/ui'

const extra = { padding: '5px', id: 'spread-target' }

export default function KeyOnlyTrailingSpread() {
  return Div({
    backgroundColor: 'yellow',
    ...extra,
  })
}
