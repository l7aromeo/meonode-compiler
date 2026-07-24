// Fixture 5: custom-factory
//
// `createNode('div', { borderRadius: 8, padding: '8px' })` produces a local
// factory (`Card`) with baked-in initial props. The compiler recognizes
// `Card(...)` call sites (via `LocalFactoryCollector` in detect.rs) and
// compiles them the same way as any other @meonode/ui HTML factory. The
// runtime then shallow-merges `{ ...initialProps, ...props }` *before*
// calling `Node(...)` — this fixture exercises that merge with the compiled
// marker shape on the `props` side.
import { createNode } from '@meonode/ui'

const Card = createNode('div', { borderRadius: 8, padding: '8px' })

export default function CustomFactory() {
  return Card({
    backgroundColor: '#ffffff',
    id: 'card-1',
    children: 'Card body',
  })
}
