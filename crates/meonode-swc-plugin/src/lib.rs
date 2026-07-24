//! meonode-swc-plugin
//!
//! SWC WASM plugin for `@meonode/compiler`. Rewrites `@meonode/ui` component
//! call sites into pre-partitioned marker props at build time:
//!
//! ```text
//! { __meo$: 1, c: { ... }, d: { ... }, k: '...', dyn: [ ... ] }
//! ```
//!
//! This file is intentionally a no-op passthrough. The real call-site
//! rewrite (visitor + partitioning logic) lands in Tasks 8-9.

use swc_core::ecma::ast::Program;
use swc_core::plugin::metadata::TransformPluginProgramMetadata;
use swc_core::plugin::plugin_transform;

// Generated CSS property set (see scripts/codegen-css-set.ts). Not yet
// consumed by the passthrough transform below — wired up in Task 9.
#[allow(dead_code)]
mod css_props;

/// Entry point invoked by the SWC plugin host (next-swc / @swc/core).
///
/// Returns the program unchanged. Kept as a minimal, verifiable scaffold so
/// CI can build the wasm32-wasip1 artifact before any transform logic exists.
#[plugin_transform]
pub fn process_transform(program: Program, _metadata: TransformPluginProgramMetadata) -> Program {
    program
}
