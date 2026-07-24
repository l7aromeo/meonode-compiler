// Fixture 8: as-polymorphism
//
// `as` is a MeoNode-specific special key (swaps the rendered DOM tag) that
// must stay top-level, never bucketed into `c`/`d`. Confirms the compiled
// marker's special-key handling doesn't disturb polymorphic rendering.
import { Div } from '@meonode/ui'

export default function AsPolymorphism() {
  return Div({ as: 'section', padding: '4px', children: 'Polymorphic section' })
}
