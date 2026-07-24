// Fixture 2: dynamic-values
//
// Module-level consts referenced as prop values (identifiers, not literals) —
// the compiler must mark these as dynamic (`dyn`) since it can't prove their
// value is stable at the call site, even though the actual runtime value is
// constant here. Also includes an arrow-function `onClick`, which is dynamic
// for the same reason (function identity/body isn't a literal).
import { Div } from '@meonode/ui'

const SPACING = '12px'
const ACCENT_COLOR = '#3b82f6'

export default function DynamicValues() {
  return Div({
    padding: SPACING,
    color: ACCENT_COLOR,
    onClick: () => {
      // Intentionally a no-op: only identity/bucketing matters for this
      // fixture, never invoked during a renderToString pass.
    },
    children: 'Dynamic content',
  })
}
