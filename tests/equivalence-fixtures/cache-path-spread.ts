// Regression fixture: cache-path-spread
//
// Exercises the exact hazard described in the v0.2 design doc's stable-key
// section (see `partition.rs::rewrite_object`'s doc comment): the SAME call
// site, invoked twice with DIFFERENT spread contents, both times passing an
// (empty, shallow-equal) `deps` array. `Div`'s third argument is `deps` —
// every `@meonode/ui` factory (`Node`, `createNode`-derived factories,
// including plain HTML factories) accepts one.
//
// Before the fix, this call site's compiled marker still carried `k` (a pure
// function of call-site *source position*, identical across both
// invocations) and no `dyn` entry for the spread's contents (which aren't
// nameable at compile time) — so `BaseNode._getStableKey`'s fast path
// produced an IDENTICAL stable key for both invocations despite different
// `id` values coming from the spread. Combined with the (shallow-equal)
// `deps` array, `BaseNode.render()`'s `elementCache` lookup would then
// return the FIRST invocation's cached element for the SECOND call — a
// stale, wrong-props element.
//
// After the fix, `k`/`dyn` are never emitted when a spread is present, so
// `_getStableKey` falls back to `createPropSignature`, which hashes the
// spread-contributed `id` by its actual (now flat, top-level) value —
// producing a different stable key whenever `id` differs, so the second
// call's element is correctly rebuilt from its own props rather than reused
// from the cache.
import { Div } from '@meonode/ui'

export function makeRow(extra: Record<string, unknown>) {
  return Div({ ...extra, padding: '4px' }, [])
}
