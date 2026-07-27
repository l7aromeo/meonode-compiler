// Fixture: keyed-list
//
// `key: item.id` inside a `.map()` callback is a `MemberExpr` — effectful per
// `effect::is_effect_free` — but v0.2's order-analysis rule (Change 1) only
// bails on a *reordering* between two effectful values; with nothing else
// effectful in the object (`color` is a literal), there's nothing to reorder
// it relative to, so this compiles. `key` also stays top-level, untouched,
// exactly like `children` — the ABSOLUTE CONSTRAINT that `key`/`children`
// semantics never change under compilation.
import { Div } from '@meonode/ui'

const items = [
  { id: 'a', label: 'Alpha' },
  { id: 'b', label: 'Beta' },
  { id: 'c', label: 'Gamma' },
]

export default function KeyedList() {
  return Div({
    padding: '4px',
    children: items.map(item =>
      Div({
        key: item.id,
        color: 'blue',
        children: item.label,
      }),
    ),
  })
}
