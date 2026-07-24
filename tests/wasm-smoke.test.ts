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

  it('bails out (leaves the call untouched) on spread props', async () => {
    const src =
      "import { Div } from '@meonode/ui'\nconst rest = {}\nDiv({ ...rest, padding: 1 })\n";
    const code = await run(src);

    expect(code).not.toContain('__meo$');
    expect(code).toContain('...rest');
  });

  it('compiles nested factory calls inside `children` independently', async () => {
    const code = await run(
      "import { Div } from '@meonode/ui'\nDiv({ padding: '20px', onClick: h, children: [Div({ color: 'red' })] })\n",
    );

    // Both the outer and the nested Div call site should be rewritten.
    const markerCount = (code.match(/__meo\$/g) ?? []).length;
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
            __meo$: 1,
            c: {
                padding: '20px'
            },
            d: {
                id: 'a'
            },
            k: "m1j2i941a490nl"
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
            __meo$: 1,
            c: {
                width,
                color: someColor
            },
            d: {
                onClick: ()=>{}
            },
            k: "m3ohe2cbdqutli",
            dyn: [
                "width",
                "onClick",
                "color"
            ]
        });
        "
      `);
    });

    it('matches snapshot for a bailout (spread props, untouched)', async () => {
      const code = await run(
        "import { Div } from '@meonode/ui'\nconst rest = {}\nDiv({ ...rest, padding: 1 })\n",
        'snapshot-bailout.tsx',
      );
      expect(code).toMatchInlineSnapshot(`
        "import { Div } from '@meonode/ui';
        const rest = {};
        Div({
            ...rest,
            padding: 1
        });
        "
      `);
    });
  });
});
