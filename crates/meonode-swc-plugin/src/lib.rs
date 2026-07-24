//! meonode-swc-plugin
//!
//! SWC WASM plugin for `@meonode/compiler`. Rewrites `@meonode/ui` component
//! call sites into pre-partitioned marker props at build time:
//!
//! ```text
//! { __meo$: 1, c: { ... }, d: { ... }, k: '...', dyn: [ ... ] }
//! ```
//!
//! This file is intentionally a no-op passthrough with respect to output:
//! Task 8 adds call-site *detection* (see `detect.rs`), but no rewriting
//! happens yet. That lands in Task 9.

use swc_core::ecma::ast::Program;
use swc_core::plugin::metadata::TransformPluginProgramMetadata;
use swc_core::plugin::plugin_transform;

// Generated CSS property set (see scripts/codegen-css-set.ts). Not yet
// consumed by the passthrough transform below — wired up in Task 9.
#[allow(dead_code)]
mod css_props;

// Generated @meonode/ui HTML factory table (see
// scripts/codegen-factories.ts), consumed by `detect`.
mod factories;

// Factory call-site detection (no rewriting yet — see module docs). Public
// so `tests/fixture.rs` (an external integration test crate) can drive the
// real detection pass, matching what `process_transform` does below.
pub mod detect;

/// Entry point invoked by the SWC plugin host (next-swc / @swc/core).
///
/// Runs call-site detection (see `detect::transform_program`) and returns
/// the program unchanged. The host has already run the SWC resolver before
/// invoking this plugin (see `TransformPluginProgramMetadata` and
/// `detect`'s module docs), so `Ident`s here already carry resolved
/// `SyntaxContext`s — detection does not need to (and must not) run the
/// resolver itself.
#[plugin_transform]
pub fn process_transform(program: Program, _metadata: TransformPluginProgramMetadata) -> Program {
    let _decisions = detect::transform_program(&program);
    program
}
