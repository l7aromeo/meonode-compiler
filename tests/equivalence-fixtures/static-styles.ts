// Fixture 1: static-styles
//
// A Div with plain literal style props (padding/backgroundColor) and nested
// P(...) children, also with literal style props. Every prop value here is a
// literal, so the compiled variant should bucket everything into `c` (all
// CSS props) with no `dyn` array at all.
import { Div, P } from '@meonode/ui'

export default function StaticStyles() {
  return Div({
    padding: '20px',
    backgroundColor: '#f0f0f0',
    children: [
      P('Hello world', { color: '#333333', fontSize: '16px' }),
      P('Second paragraph', { fontWeight: 'bold', marginTop: '8px' }),
    ],
  })
}
