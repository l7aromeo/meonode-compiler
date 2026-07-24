// Fixture 4: css-prop-merge
//
// The same CSS property (`padding`) appears both as a direct style prop and
// inside the `css` prop object. `css` must win in both the legacy and
// compiled-marker runtime paths (`{ ...markerCssProps, ...css }` /
// `{ ...cachedCssProps, ...nonCachedCssProps, ...css }`) — this fixture makes
// that merge order externally observable via the rendered output.
import { Div } from '@meonode/ui'

export default function CssPropMerge() {
  return Div({
    padding: '10px',
    color: 'black',
    css: { padding: '24px', border: '1px solid red' },
    children: 'Merged styles',
  })
}
