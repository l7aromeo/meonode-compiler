// Fixture 6: nested-children
//
// A 3-level tree with array children and mixed text/node entries, to
// exercise the compiler rewriting nested factory calls inside `children`
// independently (see partition.rs's `children`-last exception) alongside
// plain string children siblings.
import { Div, P, Span } from '@meonode/ui'

export default function NestedChildren() {
  return Div({
    padding: '16px',
    children: [
      Div({
        display: 'flex',
        children: [P('First', { color: 'red' }), Span('Second', { color: 'blue' })],
      }),
      'Trailing text node',
      P('Bottom paragraph', { fontStyle: 'italic' }),
    ],
  })
}
