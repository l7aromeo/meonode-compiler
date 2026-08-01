// WASM artifact smoke tests (Task 10).
//
// These tests load the *actual built artifact* at npm/meonode_swc_plugin.wasm
// through @swc/core's real plugin host (the same wasmer-based loader/ABI
// bridge next-swc and Vite's swc plugin use), not through the crate's own
// Rust-side unit/fixture tests. That distinction is the whole point: the
// crate is built against swc_core 74.0.2, and @swc/core embeds its own
// swc_core-derived ABI. If those two versions' plugin ABI (rkyv-encoded
// Program, bytecheck-validated) had drifted apart, @swc/core would reject
// the module at load/deserialize time with a bytecheck/ABI error — the
// exact failure mode this suite exists to catch. A green run here is
// evidence the artifact loads and executes correctly against a real host,
// not just that the Rust source compiles.
//
// Run via `bun run build:wasm` (produces the artifact this suite loads)
// followed by `bun run test`.

import { existsSync } from 'node:fs';
import path from 'node:path';
import { transform } from '@swc/core';
import { describe, expect, it } from 'vitest';

const WASM_PATH = path.resolve(import.meta.dirname, '../npm/meonode_swc_plugin.wasm');

if (!existsSync(WASM_PATH)) {
  throw new Error(
    `wasm artifact not found at ${WASM_PATH} — run \`bun run build:wasm\` first`,
  );
}

/** Runs the real plugin (loaded from the built .wasm artifact) over `src` via @swc/core. */
async function run(src: string, filename = 'test.tsx'): Promise<string> {
  const result = await transform(src, {
    filename,
    jsc: {
      target: 'es2022',
      parser: { syntax: 'typescript', tsx: true },
      experimental: {
        plugins: [[WASM_PATH, {}]],
      },
    },
  });
  return result.code;
}

describe('wasm artifact smoke (@swc/core loading meonode_swc_plugin.wasm)', () => {
  it('transforms a static call site into the partitioned marker-prop shape', async () => {
    const code = await run(
      "import { Div } from '@meonode/ui'\nexport const x = Div({ padding: '20px', id: 'a' })\n",
    );

    expect(code).toContain('__meo$');
    expect(code).toMatch(/c:\s*{\s*padding:\s*'20px'\s*}/);
    expect(code).toMatch(/d:\s*{\s*id:\s*'a'\s*}/);
    expect(code).toMatch(/k:\s*"m/);
  });

  it('marks dynamic (non-literal) prop values in `dyn`', async () => {
    const code = await run(
      "import { Div } from '@meonode/ui'\nDiv({ onClick: handleClick, padding: '1px' })\n",
    );

    expect(code).toContain('dyn');
    expect(code).toMatch(/dyn:\s*\[\s*"onClick"\s*\]/);
  });

  it('compiles a leading spread, leaving it top-level (v0.2 Change 2)', async () => {
    const src =
      "import { Div } from '@meonode/ui'\nconst rest = {}\nDiv({ ...rest, padding: 1 })\n";
    const code = await run(src);

    expect(code).toContain('__meo$');
    expect(code).toContain('...rest');
    expect(code).toMatch(/c:\s*{\s*padding:\s*1\s*}/);
    // Stable-key hazard fix: `k`/`dyn` must never be emitted when a spread
    // is present, however static everything else is — a spread's contents
    // are invisible to `k` (a pure function of call-site source position),
    // so two evaluations of this call site with different spread contents
    // would otherwise collide on the same stable key.
    expect(code).not.toMatch(/\bk:\s*"m/);
    expect(code).not.toMatch(/\bdyn:\s*\[/);
  });

  it('leaves a dynamic prop flat (unbucketed) alongside a leading spread', async () => {
    const src =
      "import { Div } from '@meonode/ui'\nconst rest = {}\nDiv({ ...rest, onClick: handler, padding: 1 })\n";
    const code = await run(src);

    expect(code).toContain('__meo$');
    expect(code).toContain('...rest');
    expect(code).toMatch(/c:\s*{\s*padding:\s*1\s*}/);
    // `onClick` must stay flat, not bucketed into `d` — bucketing it would
    // hide its actual value behind `d`'s structural-only hash once the
    // legacy stable-key fallback is in play (no `k`/`dyn` to rely on).
    expect(code).not.toMatch(/d:\s*{/);
    expect(code).toMatch(/onClick:\s*handler/);
  });

  it('keys but does not partition a trailing spread', async () => {
    // Partitioning is refused (`TrailingSpread`), so no `c`/`d` buckets — but
    // the call-site key is a hash of filename and span and needs no knowledge
    // of the props, so schema 3 is still stamped. Appended *after* the spread,
    // so the spread cannot shadow it.
    const src =
      "import { Div } from '@meonode/ui'\nconst rest = {}\nDiv({ padding: 1, ...rest })\n";
    const code = await run(src);

    expect(code).toMatch(/__meo\$:\s*3/);
    expect(code).toContain('__meo$k');
    expect(code).not.toMatch(/__meo\$c:/);
    expect(code).not.toMatch(/__meo\$d:/);
    expect(code).toContain('...rest');
    // Order matters: marker after the spread.
    expect(code.indexOf('...rest')).toBeLessThan(code.indexOf('__meo$'));
  });

  it('leaves a non-object-literal props argument completely untouched', async () => {
    // The negative control, and the remaining gap: there is no object literal
    // to append a marker to, so this call site keys off props exactly like an
    // uncompiled one and can still collide with a structurally identical one
    // elsewhere. `key` is the answer there.
    const src = "import { Div } from '@meonode/ui'\nDiv(cond ? { padding: 1 } : { padding: 2 })\n";
    const code = await run(src);

    expect(code).not.toContain('__meo$');
  });

  it('compiles nested factory calls inside `children` independently', async () => {
    const code = await run(
      "import { Div } from '@meonode/ui'\nDiv({ padding: '20px', onClick: h, children: [Div({ color: 'red' })] })\n",
    );

    // Both the outer and the nested Div call site should be rewritten.
    // Count the schema marker itself (`__meo$:`), not every `__meo$`-prefixed
    // bucket key — schema 2 names all buckets with that prefix.
    const markerCount = (code.match(/__meo\$:/g) ?? []).length;
    expect(markerCount).toBe(2);
  });

  describe('call-site key (`k`) determinism', () => {
    const src = "import { Div } from '@meonode/ui'\nDiv({ padding: '20px' })\n";

    it('is identical for the same source + filename compiled twice', async () => {
      const code1 = await run(src, 'same.tsx');
      const code2 = await run(src, 'same.tsx');

      const k1 = code1.match(/k:\s*"(m[^"]+)"/)?.[1];
      const k2 = code2.match(/k:\s*"(m[^"]+)"/)?.[1];

      expect(k1).toBeDefined();
      expect(k1).toBe(k2);
    });

    it('differs when the filename differs', async () => {
      const codeA = await run(src, 'a.tsx');
      const codeB = await run(src, 'b.tsx');

      const kA = codeA.match(/k:\s*"(m[^"]+)"/)?.[1];
      const kB = codeB.match(/k:\s*"(m[^"]+)"/)?.[1];

      expect(kA).toBeDefined();
      expect(kB).toBeDefined();
      expect(kA).not.toBe(kB);
    });
  });

  describe('output snapshots', () => {
    it('matches snapshot for a static-only call', async () => {
      const code = await run(
        "import { Div } from '@meonode/ui'\nexport const x = Div({ padding: '20px', id: 'a' })\n",
        'snapshot-static.tsx',
      );
      expect(code).toMatchInlineSnapshot(`
        "import { Div } from '@meonode/ui';
        export const x = Div({
            __meo$: 2,
            __meo$c: {
                padding: '20px'
            },
            __meo$d: {
                id: 'a'
            },
            __meo$k: "m1j2i941a490nl"
        });
        "
      `);
    });

    it('matches snapshot for a call with dynamic values', async () => {
      const code = await run(
        "import { Div } from '@meonode/ui'\nDiv({ width, onClick: () => {}, color: someColor })\n",
        'snapshot-dynamic.tsx',
      );
      expect(code).toMatchInlineSnapshot(`
        "import { Div } from '@meonode/ui';
        Div({
            __meo$: 2,
            __meo$c: {
                width,
                color: someColor
            },
            __meo$d: {
                onClick: ()=>{}
            },
            __meo$k: "m3ohe2cbdqutli",
            __meo$dyn: [
                "width",
                "color"
            ]
        });
        "
      `);
    });

    it('matches snapshot for a leading spread (compiled, spread left top-level)', async () => {
      const code = await run(
        "import { Div } from '@meonode/ui'\nconst rest = {}\nDiv({ ...rest, padding: 1 })\n",
        'snapshot-leading-spread.tsx',
      );
      expect(code).toMatchInlineSnapshot(`
        "import { Div } from '@meonode/ui';
        const rest = {};
        Div({
            __meo$: 2,
            ...rest,
            __meo$c: {
                padding: 1
            }
        });
        "
      `);
    });

    it('matches snapshot for a key-only call site (trailing spread)', async () => {
      const code = await run(
        "import { Div } from '@meonode/ui'\nconst rest = {}\nDiv({ padding: 1, ...rest })\n",
        'snapshot-bailout.tsx',
      );
      expect(code).toMatchInlineSnapshot(`
        "import { Div } from '@meonode/ui';
        const rest = {};
        Div({
            padding: 1,
            ...rest,
            __meo$: 3,
            __meo$k: "m1wgxf8d9tgibj"
        });
        "
      `);
    });
  });
});
