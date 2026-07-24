'use client'

// Shared test tree (Task 12) — kept identical in e2e/vite-app/src/App.jsx so
// the two fixtures exercise the same shape:
//   - static styles              (Div/P/Span literal CSS props)
//   - dynamic values             (identifier + arrow-function onClick)
//   - nested children            (children-last compile rule)
//   - custom createNode factory  (Card)
//   - `as` polymorphism          (Div rendered as a <section>)
//
// The outer node carries id="root-marker" so the orchestrator can extract
// just this subtree's outerHTML from the prerendered document, ignoring
// build-hashed asset URLs elsewhere on the page.
import { Div, P, Span, createNode } from '@meonode/ui'

const Card = createNode('div', { borderRadius: 8, padding: '8px' })

const SPACING = '12px'
const ACCENT_COLOR = '#3b82f6'

export default function Page() {
  return Div({
    id: 'root-marker',
    padding: '20px',
    backgroundColor: '#f0f0f0',
    children: [
      Div({
        color: ACCENT_COLOR,
        padding: SPACING,
        onClick: () => {
          // Intentionally a no-op: only present so this prop is bucketed as
          // dynamic; SSR/build output never invokes it.
        },
        children: [
          P('Hello world', { color: '#333333', fontSize: '16px' }),
          Span('Second paragraph', { fontWeight: 'bold' }),
        ],
      }),
      Card({
        backgroundColor: '#ffffff',
        id: 'card-1',
        children: 'Card body',
      }),
      Div({ as: 'section', padding: '4px', children: 'Polymorphic section' }),
    ],
  }).render()
}
