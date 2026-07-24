// Fixture 7: bailout-spread
//
// A spread prop (`...extra`) in the props object literal is one of
// detect.rs's `BailReason`s (`SpreadProp`) — the plugin must leave this call
// site completely untouched. This fixture is the negative control: it
// asserts the pipeline itself (parse -> (no) transform -> codegen) doesn't
// corrupt an already-valid call site it decided not to rewrite.
import { Div } from '@meonode/ui'

const extra = { padding: '5px', id: 'spread-target' }

export default function BailoutSpread() {
  return Div({
    ...extra,
    backgroundColor: 'yellow',
  })
}
