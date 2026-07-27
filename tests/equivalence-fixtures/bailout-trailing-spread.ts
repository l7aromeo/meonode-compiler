// Fixture: bailout-trailing-spread
//
// A spread AFTER a static prop (`{ backgroundColor: 'yellow', ...extra }`)
// still bails (`BailReason::TrailingSpread`) — the spread would need to win
// over the preceding static prop for correct precedence, which the emitted
// `{ ...passthroughCss, ...markerCss, ...css }` merge order can't express
// (compiler-bucketed static props always come after the spread in the
// emitted shape). This is the suite's negative control, proving it doesn't
// vacuously mark everything compilable now that leading spreads (see
// `leading-spread.ts`) are allowed.
import { Div } from '@meonode/ui'

const extra = { padding: '5px', id: 'spread-target' }

export default function BailoutTrailingSpread() {
  return Div({
    backgroundColor: 'yellow',
    ...extra,
  })
}
