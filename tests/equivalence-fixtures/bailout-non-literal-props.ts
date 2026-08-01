// Fixture: bailout-non-literal-props
//
// The suite's negative control: a call site the plugin leaves completely
// untouched, proving it does not vacuously mark everything.
//
// The props argument is a conditional, not an object literal
// (`BailReason::NotObjectLiteral`). The plugin cannot partition it — prop names
// are not statically knowable — and cannot key it either, because there is no
// object literal to append the marker to without either wrapping the expression
// in a spread (which would move when its getters run) or rewriting the call.
//
// This is the remaining gap in the compiler's memoization-collision immunity:
// such a call site keys off props exactly like an uncompiled one, so two
// structurally identical memoized subtrees written at different places still
// collide. `key` is the answer there. If this fixture ever starts emitting a
// marker, that gap has been closed and the docs should be revisited.
import { Div } from '@meonode/ui'

const useWide = true

export default function BailoutNonLiteralProps() {
  return Div(useWide ? { backgroundColor: 'yellow', padding: '5px', id: 'cond-target' } : { backgroundColor: 'yellow', id: 'cond-target' })
}
