//! meonode-swc-plugin
//!
//! SWC WASM plugin for `@meonode/compiler`. Rewrites `@meonode/ui` component
//! call sites into pre-partitioned marker props at build time:
//!
//! ```text
//! { __meo$: 1, c: { ... }, d: { ... }, k: '...', dyn: [ ... ] }
//! ```
//!
//! Call-site *detection* (see `detect.rs`) and the effect-safety classifier
//! it relies on (`effect.rs`) decide which call sites are safe to rewrite;
//! `partition.rs` performs the actual rewrite for every call site it accepts.

use swc_core::ecma::ast::Program;
use swc_core::plugin::metadata::{
    TransformPluginMetadataContextKind, TransformPluginProgramMetadata,
};
use swc_core::plugin::plugin_transform;

// Generated CSS property set (see scripts/codegen-css-set.ts), consumed by
// `partition` to decide `c`- vs `d`-bucket membership.
mod css_props;

// Generated @meonode/ui HTML factory table (see
// scripts/codegen-factories.ts), consumed by `detect`.
mod factories;

// Effect-safety classifier shared by `detect` (bail decisions) and
// `partition` (dyn-list classification).
mod effect;

// Shared prop-key utilities (special-key set, key-name extraction) used by
// both `detect` and `partition` — see module docs for why they must agree.
mod keys;

// Evaluation-order safety analysis (v0.2 rule, Change 1) — the standalone,
// AST-agnostic core of "would compiling this reorder an effectful value
// relative to another effectful value". Public so its unit tests double as
// the documented contract for the rule; `detect` is the only real caller.
pub mod order;

// Plugin configuration (`factoryModules`, Change 4). Public so
// `tests/fixture.rs` can construct a `CompileConfig` directly for
// config-driven fixtures (the plugin-metadata path itself is untestable
// outside a real wasm32 host — see its module docs).
pub mod config;

// Factory call-site detection (no rewriting itself — see module docs).
// Public so `tests/fixture.rs` (an external integration test crate) can
// drive the real detection pass directly.
pub mod detect;

// Prop partitioning + call-site key emission (Task 9). Public for the same
// reason as `detect`.
pub mod partition;

/// Entry point invoked by the SWC plugin host (next-swc / @swc/core).
///
/// Runs call-site detection (see `detect::transform_program`) and rewrites
/// every accepted call site (see `partition::transform_program`). The host
/// has already run the SWC resolver before invoking this plugin (see
/// `TransformPluginProgramMetadata` and `detect`'s module docs), so `Ident`s
/// here already carry resolved `SyntaxContext`s.
///
/// The plugin's own config (the `{}` second element of the `swcPlugins`
/// tuple, e.g. `{ factoryModules: ['@meonode/mui'] }` — Change 4) is read via
/// `get_transform_plugin_config` and parsed by `config::CompileConfig`, which
/// degrades to its default (no extra modules, current behavior) on a
/// missing or malformed config rather than failing the build.
#[plugin_transform]
pub fn process_transform(
    mut program: Program,
    metadata: TransformPluginProgramMetadata,
) -> Program {
    let filename = resolve_filename(&metadata);
    let plugin_config =
        config::CompileConfig::from_json(metadata.get_transform_plugin_config().as_deref());
    partition::transform_program(&mut program, &filename, &plugin_config);
    program
}

/// Resolves the filename used to compute each call site's `k` value: the
/// host-provided filename (see `TransformPluginMetadataContextKind::Filename`),
/// with the host-provided cwd stripped as a prefix if both are available so
/// `k` doesn't depend on where the project happens to be checked out on
/// disk. Falls back to `"unknown"` if the host doesn't provide a filename at
/// all (e.g. some non-file virtual sources).
fn resolve_filename(metadata: &TransformPluginProgramMetadata) -> String {
    let Some(filename) = metadata.get_context(&TransformPluginMetadataContextKind::Filename) else {
        return "unknown".to_string();
    };

    match metadata.get_context(&TransformPluginMetadataContextKind::Cwd) {
        Some(cwd) if !cwd.is_empty() => filename
            .strip_prefix(&cwd)
            .map(|rest| rest.trim_start_matches(['/', '\\']).to_string())
            .unwrap_or(filename),
        _ => filename,
    }
}
