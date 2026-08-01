#!/usr/bin/env bun
/**
 * Codegen: pulls the canonical CSS property name set from @meonode/ui and
 * emits crates/meonode-swc-plugin/src/css_props.rs.
 *
 * Source of truth: @meonode/ui's `bun run export:css-props` (defined in that
 * repo's package.json as `bun scripts/export-css-properties.ts`), which
 * prints a sorted, deduped JSON array of every camelCase CSS property name
 * the runtime's static/dynamic prop classifier recognizes. This script must
 * emit the *exact same set*, sorted in a way that is compatible with Rust's
 * `[&str]::binary_search`, so the compiler and the runtime never disagree
 * about what counts as a CSS prop.
 *
 * Usage:
 *   bun run scripts/codegen-css-set.ts
 *   MEONODE_UI_DIR=/path/to/ui bun run scripts/codegen-css-set.ts
 *
 * Env:
 *   MEONODE_UI_DIR  Path to the @meonode/ui checkout (default: "../ui",
 *                   relative to this repo's root).
 */

import { spawnSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const REPO_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const OUT_PATH = join(
  REPO_ROOT,
  "crates/meonode-swc-plugin/src/css_props.rs",
);

/**
 * Runs @meonode/ui's export script and returns the completed process.
 *
 * `bun run` writes its `$ <command>` banner to stderr, so stdout is pure JSON.
 * @param uiDir The @meonode/ui checkout.
 * @param args Extra arguments forwarded to the export script.
 */
function runExport(uiDir: string, args: string[]) {
  const label = ["export:css-props", ...args].join(" ");
  const result = spawnSync("bun", ["run", "export:css-props", ...args], {
    cwd: uiDir,
    encoding: "utf8",
  });
  if (result.error) {
    throw new Error(
      `Failed to spawn \`bun run ${label}\` in ${uiDir}: ${result.error.message}`,
    );
  }
  if (result.status !== 0) {
    throw new Error(
      `\`bun run ${label}\` in ${uiDir} exited with status ${result.status}.\n` +
        `stdout: ${result.stdout}\nstderr: ${result.stderr}`,
    );
  }
  return result;
}

/**
 * Parses one export script's stdout into a string array.
 * @param stdout Raw stdout.
 * @param label Command label, for error messages.
 */
function parseProps(stdout: string, label: string): string[] {
  const raw: unknown = JSON.parse(stdout);
  if (!Array.isArray(raw) || !raw.every((x) => typeof x === "string")) {
    throw new Error(`Expected \`bun run ${label}\` to print a JSON array of strings.`);
  }
  return raw as string[];
}

function main() {
  const uiDir = resolve(
    REPO_ROOT,
    process.env.MEONODE_UI_DIR ?? "../ui",
  );

  const result = runExport(uiDir, []);
  const lengthResult = runExport(uiDir, ["--length"]);

  const props = parseProps(result.stdout, "export:css-props");
  const lengthProps = parseProps(lengthResult.stdout, "export:css-props --length");

  // The length set decides which declarations reference the paired `--len`
  // theme variable. It must be a subset of the CSS set, or the plugin would
  // treat something as a length that it does not even recognise as a CSS prop.
  const cssSet = new Set(props);
  const orphans = lengthProps.filter((p) => !cssSet.has(p));
  if (orphans.length > 0) {
    throw new Error(
      `Length properties absent from the CSS property set: ${orphans.join(", ")}. ` +
        "The two sets are generated from the same source and must stay consistent.",
    );
  }

  // --- Sort-order verification -------------------------------------------
  // Rust's `&[&str]::binary_search` orders elements by `str`'s `Ord` impl,
  // which compares byte-by-byte (UTF-8 code units). JavaScript's default
  // `Array.prototype.sort()` (no comparator) orders strings by UTF-16 code
  // unit. These two orderings are NOT the same in general (they diverge for
  // any codepoint outside the Basic Multilingual Plane, and even within the
  // BMP, multi-byte UTF-8 sequences are ordered differently than UTF-16 code
  // units for non-ASCII characters). They ARE identical whenever every
  // string in the set is pure ASCII, because each ASCII character occupies
  // exactly one UTF-16 code unit AND exactly one UTF-8 byte with the same
  // numeric value (0x00-0x7F in both encodings).
  //
  // CSS property names (including vendor-prefixed ones like `MozAppearance`
  // or `WebkitTransform`) are always ASCII, so we assert that here rather
  // than merely assuming it — if @meonode/ui ever introduces a non-ASCII
  // property name, this script fails loudly instead of silently emitting a
  // Rust array that `binary_search` can't correctly probe.
  for (const p of [...props, ...lengthProps]) {
    if (!/^[\x00-\x7f]*$/.test(p)) {
      throw new Error(
        `Non-ASCII CSS property name encountered: ${JSON.stringify(p)}. ` +
          "JS UTF-16 sort order and Rust byte order are no longer " +
          "guaranteed to match; codegen aborted.",
      );
    }
  }

  // Sort (defensively — the source script already sorts, but codegen must
  // not depend on that) and dedupe.
  const sorted = [...new Set(props)].sort();

  // Sanity: JS sort must already be strictly ascending byte order for ASCII
  // input; verify before baking it into a Rust binary_search table.
  for (let i = 1; i < sorted.length; i++) {
    if (!(sorted[i - 1] < sorted[i])) {
      throw new Error(
        `Sort invariant violated at index ${i}: ${JSON.stringify(sorted[i - 1])} >= ${JSON.stringify(sorted[i])}`,
      );
    }
  }

  const sortedLengths = [...new Set(lengthProps)].sort();
  for (let i = 1; i < sortedLengths.length; i++) {
    if (!(sortedLengths[i - 1] < sortedLengths[i])) {
      throw new Error(
        `Length sort invariant violated at index ${i}: ${JSON.stringify(sortedLengths[i - 1])} >= ${JSON.stringify(sortedLengths[i])}`,
      );
    }
  }

  const rustFile = renderRust(sorted, sortedLengths);

  mkdirSync(dirname(OUT_PATH), { recursive: true });
  writeFileSync(OUT_PATH, rustFile, "utf8");

  console.log(
    `Wrote ${sorted.length} CSS property names and ${sortedLengths.length} length properties to ${OUT_PATH.replace(REPO_ROOT + "/", "")}`,
  );
}

function renderRust(sorted: string[], sortedLengths: string[]): string {
  const entries = sorted.map((p) => `    ${JSON.stringify(p)},`).join("\n");
  const lengthEntries = sortedLengths.map((p) => `    ${JSON.stringify(p)},`).join("\n");

  return `// @generated by scripts/codegen-css-set.ts — do not edit. Source: @meonode/ui css-properties.const.ts
//
// Sorted in byte order (Rust \`str\` \`Ord\`), which is guaranteed identical
// to JavaScript's default \`Array.prototype.sort()\` (UTF-16 code unit order)
// because every entry is pure ASCII. See scripts/codegen-css-set.ts for the
// full argument. \`is_css_prop\` relies on this invariant via
// \`binary_search\`; the sortedness unit test below is the enforcement point
// if that invariant is ever violated by a future regeneration.

pub static CSS_PROPS: &[&str] = &[
${entries}
];

/// Returns true if \`name\` is a recognized camelCase CSS property (including
/// vendor-prefixed and custom properties) per the set exported by
/// \`@meonode/ui\`'s \`css-properties.const.ts\`.
pub fn is_css_prop(name: &str) -> bool {
    CSS_PROPS.binary_search(&name).is_ok()
}

/// Properties whose value is a length, where a bare number is invalid.
///
/// A theme token used for one of these is rewritten to
/// \`var(--x--len, var(--x))\` so a numeric token value arrives with its unit.
/// The runtime makes the identical choice from the identical set, which is why
/// this is generated rather than hand-maintained: a disagreement would emit a
/// reference to a variable the other side never defined, and the browser drops
/// such a declaration silently.
///
/// Derived in \`@meonode/ui\` as the properties \`csstype\` parameterises by
/// \`TLength\`, intersected with the CSS property set above, minus
/// \`@emotion/unitless\`. That last subtraction keeps \`lineHeight\`, \`flex\`,
/// \`tabSize\` and \`strokeWidth\` out: they accept a length *and* a bare number,
/// and there the bare number is what the author meant.
pub static LENGTH_PROPS: &[&str] = &[
${lengthEntries}
];

/// Returns true if \`name\` is a length-valued CSS property, i.e. one where a
/// theme token must carry a unit.
pub fn is_length_prop(name: &str) -> bool {
    LENGTH_PROPS.binary_search(&name).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn css_props_is_sorted() {
        assert!(
            CSS_PROPS.windows(2).all(|w| w[0] < w[1]),
            "CSS_PROPS must be strictly sorted in byte order for binary_search to work"
        );
    }

    #[test]
    fn recognizes_known_css_props() {
        assert!(is_css_prop("backgroundColor"));
    }

    #[test]
    fn rejects_react_event_handlers() {
        assert!(!is_css_prop("onClick"));
    }

    #[test]
    fn rejects_non_css_props() {
        assert!(!is_css_prop("id"));
    }

    #[test]
    fn length_props_is_sorted() {
        assert!(
            LENGTH_PROPS.windows(2).all(|w| w[0] < w[1]),
            "LENGTH_PROPS must be strictly sorted in byte order for binary_search to work"
        );
    }

    #[test]
    fn length_props_is_a_subset_of_css_props() {
        for name in LENGTH_PROPS {
            assert!(
                is_css_prop(name),
                "{name} is a length property but not a recognized CSS property"
            );
        }
    }

    #[test]
    fn recognizes_length_props() {
        assert!(is_length_prop("padding"));
        assert!(is_length_prop("borderRadius"));
    }

    #[test]
    fn rejects_properties_where_a_bare_number_is_meaningful() {
        // Accept a length *and* a bare number; the bare number is the intent.
        assert!(!is_length_prop("lineHeight"));
        assert!(!is_length_prop("flex"));
        assert!(!is_length_prop("tabSize"));
        // Purely unitless.
        assert!(!is_length_prop("zIndex"));
        assert!(!is_length_prop("opacity"));
    }
}
`;
}

main();
