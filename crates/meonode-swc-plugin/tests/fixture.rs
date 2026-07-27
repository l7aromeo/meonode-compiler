//! Fixture tests for Task 8 (factory call-site detection).
//!
//! Task 8 is detection-only — no rewriting happens yet (that's Task 9), so
//! every fixture case here has `output.js == input.js`: running the real
//! detection pass (`meonode_swc_plugin::detect::transform_program`) must
//! never change the program. This proves the visitor is wired up and
//! traverses these shapes without mutating anything; it does *not* prove
//! individual call sites are classified correctly (a case with an entirely
//! broken detector would look identical here too, since neither the
//! Compilable nor the Bail path mutates the AST in this task). Correctness
//! of the classification itself is covered by the decision-asserting unit
//! tests in `src/detect.rs`.
//!
//! Mirrors what the real swc plugin host does: the resolver
//! (`swc_ecma_transforms_base::resolver`) runs before detection, exactly as
//! `TransformPluginProgramMetadata` implies the host does before invoking
//! the plugin in production (see `detect.rs` module docs).

use std::path::PathBuf;

use meonode_swc_plugin::config::CompileConfig;
use meonode_swc_plugin::{detect, partition};
use swc_core::common::Mark;
use swc_core::ecma::ast::{Pass, Program};
use swc_core::ecma::parser::{EsSyntax, Syntax};
use swc_core::ecma::transforms::base::resolver;
use swc_core::ecma::transforms::testing::{test_fixture, FixtureTestConfig, Tester};

/// Fixed filename fed to `partition::transform_program` across every Task 9
/// fixture case. There's no plugin host metadata in a fixture test (see
/// `detect.rs`'s module docs on binding resolution for the same point about
/// the resolver), so `k`'s filename component is this constant rather than a
/// real file path — this only needs to be *some* stable string, not the
/// fixture's actual path on disk.
const FIXTURE_FILENAME: &str = "fixture.tsx";

/// A `Pass` that runs detection for its side effect (recording decisions)
/// and returns the program untouched — mirroring what Task 8's version of
/// `lib.rs::process_transform` did before Task 9 wired up the actual
/// rewrite. Still used by every Task 8 fixture case below, which assert
/// `output.js == input.js` to prove detection alone never mutates the tree.
struct DetectOnly {
    config: CompileConfig,
}

impl Pass for DetectOnly {
    fn process(&mut self, program: &mut Program) {
        let _decisions = detect::transform_program(program, &self.config);
    }
}

/// A `Pass` that runs the full Task 9 rewrite (detection + prop
/// partitioning), mirroring `lib.rs::process_transform` in the real plugin.
/// `config` carries Change 4's `factoryModules` list — real plugin config
/// (`TransformPluginProgramMetadata::get_transform_plugin_config`) is only
/// ever `Some` inside a real wasm32 plugin host (see `config.rs`'s module
/// docs), so config-driven fixture cases construct a `CompileConfig` here
/// directly rather than through that unavailable path.
struct Partition {
    config: CompileConfig,
}

impl Pass for Partition {
    fn process(&mut self, program: &mut Program) {
        partition::transform_program(program, FIXTURE_FILENAME, &self.config);
    }
}

fn tr(_tester: &mut Tester) -> impl Pass {
    // Fresh marks per fixture run, exactly like `resolver`'s own docs and
    // real-world swc plugin fixtures do. In production this pass isn't run
    // by the plugin at all — the host runs it before handing the program to
    // the plugin (see `detect.rs` module docs) — but `test_fixture` starts
    // from a freshly parsed, unresolved AST, so the fixture harness has to
    // do it explicitly.
    (
        resolver(Mark::new(), Mark::new(), false),
        DetectOnly {
            config: CompileConfig::default(),
        },
    )
}

fn tr_partition(_tester: &mut Tester) -> impl Pass {
    (
        resolver(Mark::new(), Mark::new(), false),
        Partition {
            config: CompileConfig::default(),
        },
    )
}

fn run(case: &str) {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixture")
        .join(case);
    test_fixture(
        Syntax::Es(EsSyntax::default()),
        &|t| tr(t),
        &dir.join("input.js"),
        &dir.join("output.js"),
        FixtureTestConfig::default(),
    );
}

fn run_partition(case: &str) {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixture")
        .join(case);
    test_fixture(
        Syntax::Es(EsSyntax::default()),
        &|t| tr_partition(t),
        &dir.join("input.js"),
        &dir.join("output.js"),
        FixtureTestConfig::default(),
    );
}

/// Like [`run_partition`], but with a caller-supplied [`CompileConfig`] — for
/// Change 4's config-driven `factoryModules` fixtures.
fn run_partition_with_config(case: &str, config: CompileConfig) {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixture")
        .join(case);
    test_fixture(
        Syntax::Es(EsSyntax::default()),
        &|_t| {
            (
                resolver(Mark::new(), Mark::new(), false),
                Partition {
                    config: config.clone(),
                },
            )
        },
        &dir.join("input.js"),
        &dir.join("output.js"),
        FixtureTestConfig::default(),
    );
}

#[test]
fn named_import_div() {
    run("named_import_div");
}

#[test]
fn aliased_import() {
    run("aliased_import");
}

#[test]
fn children_first_p() {
    run("children_first_p");
}

#[test]
fn node_factory() {
    run("node_factory");
}

#[test]
fn local_create_node() {
    run("local_create_node");
}

#[test]
fn shadowed_div() {
    run("shadowed_div");
}

#[test]
fn namespace_import_bail() {
    run("namespace_import_bail");
}

#[test]
fn non_object_props_bail() {
    run("non_object_props_bail");
}

#[test]
fn spread_bail() {
    // Name predates Change 2 (v0.2): this leading-spread shape is actually
    // `Decision::Compilable` now (see `detect::tests::leading_spread_in_props_is_fine`
    // and the `transform_leading_spread` fixture below for the rewrite
    // itself), but this particular fixture only exercises the Task 8
    // detect-only pass, which never mutates the tree regardless of the
    // decision it records — so `output.js == input.js` still holds and this
    // case is still a valid (if staler-named) sanity check.
    run("spread_bail");
}

#[test]
fn computed_key_bail() {
    run("computed_key_bail");
}

#[test]
fn existing_marker_bail() {
    run("existing_marker_bail");
}

// --- Task 9: prop partitioning + call-site key emission ---
//
// Unlike the Task 8 cases above (which only ever assert `output.js ==
// input.js`), every case below drives the real rewrite pass
// (`partition::transform_program`) and asserts a genuinely different
// `output.js`.

#[test]
fn transform_static_only() {
    run_partition("transform_static_only");
}

#[test]
fn transform_dynamic_values() {
    run_partition("transform_dynamic_values");
}

#[test]
fn transform_special_keys() {
    run_partition("transform_special_keys");
}

#[test]
fn transform_children_first() {
    run_partition("transform_children_first");
}

#[test]
fn transform_local_create_node() {
    run_partition("transform_local_create_node");
}

#[test]
fn transform_shorthand_props() {
    run_partition("transform_shorthand_props");
}

#[test]
fn transform_single_effectful_call_compiles() {
    // v0.2 (Change 1): `x: f()` is the *only* effectful value (`padding:
    // "1px"` is a static literal) — trivially order-preserving. v1 bailed on
    // any effectful value at all (`EffectfulValue`); renamed from
    // `transform_effectful_bail` now that it legitimately compiles.
    run_partition("transform_single_effectful_call_compiles");
}

#[test]
fn transform_single_effectful_member_expr_compiles() {
    // Same reasoning as `transform_single_effectful_call_compiles`, for a
    // member-expression value (`x: a.b`) instead of a call. Renamed from
    // `transform_member_expr_bail`.
    run_partition("transform_single_effectful_member_expr_compiles");
}

#[test]
fn transform_empty_dyn_omitted() {
    run_partition("transform_empty_dyn_omitted");
}

#[test]
fn transform_all_special_keys() {
    run_partition("transform_all_special_keys");
}

#[test]
fn transform_nested_children() {
    run_partition("transform_nested_children");
}

#[test]
fn transform_children_not_last_still_compiles() {
    // v0.2 (Change 1): `children: [f()]` is the *only* effectful value here
    // (`padding: '1px'` is a static literal) — trivially order-preserving,
    // so this now compiles even though `children` isn't source-final. Under
    // v1's blanket "every effectful value bails unless children is last"
    // rule this bailed (`EffectfulValue`); v0.2's general order rule
    // subsumes and strictly widens that exception. See
    // `detect::tests::effectful_children_not_last_but_only_effectful_value_is_fine`
    // for the equivalent unit-level assertion.
    run_partition("transform_children_not_last_still_compiles");
}

#[test]
fn transform_leading_spread() {
    // Change 2: a spread preceding every static prop compiles by leaving the
    // spread top-level and bucketing only the static props. Note `k`/`dyn`
    // are absent from the expected output — see
    // `transform_leading_spread_with_dynamic_prop` and the stable-key-hazard
    // doc comment on `partition::rewrite_object` for why.
    run_partition("transform_leading_spread");
}

#[test]
fn transform_leading_spread_with_dynamic_prop() {
    // Stable-key hazard fix: `onClick: handler` is dynamic (not a static
    // literal) but with a spread present it must stay flat/unbucketed
    // (never in `d`), rather than being hidden behind `d`'s
    // structural-only hash once `k`/`dyn` are omitted. `padding` (a static
    // literal) is unaffected and still bucketed into `c` normally.
    run_partition("transform_leading_spread_with_dynamic_prop");
}

#[test]
fn transform_multiple_leading_spreads() {
    // Multiple leading spreads are fine; their relative order is preserved.
    run_partition("transform_multiple_leading_spreads");
}

#[test]
fn transform_trailing_spread_bail() {
    // A spread *after* a static prop still bails (`TrailingSpread`) — see
    // `detect::tests::trailing_spread_bails`.
    run_partition("transform_trailing_spread_bail");
}

#[test]
fn transform_quoted_string_key() {
    // Change 3: non-identifier string keys (`'data-parallax'`) now bucket
    // normally instead of bailing with `NonIdentifierStringKey`.
    run_partition("transform_quoted_string_key");
}

#[test]
fn transform_key_member_expr() {
    // `key: item.id` (a MemberExpr, effectful) is the only effectful value —
    // trivially order-preserving — and `key` stays top-level, untouched,
    // exactly like any other special key (the ABSOLUTE CONSTRAINT: `key`
    // semantics never change).
    run_partition("transform_key_member_expr");
}

#[test]
fn transform_two_effectful_order_preserved() {
    // `padding` (Css, effectful) precedes `onClick` (Data, effectful) in both
    // source and emitted order (Css always emits before Data) — compiles.
    run_partition("transform_two_effectful_order_preserved");
}

#[test]
fn transform_two_effectful_order_violated_bail() {
    // `onClick` (Data, effectful) precedes `padding` (Css, effectful) in
    // source, but emission always puts Css before Data — a genuine
    // reordering, bails via `EffectfulReorder`.
    run_partition("transform_two_effectful_order_violated_bail");
}

#[test]
fn transform_factory_module_configured() {
    // Change 4: `Button` from a configured `factoryModules` entry
    // (`@meonode/mui`) is recognized as a props-at-arg-0 factory and
    // compiled, same as any `@meonode/ui` HTML factory.
    run_partition_with_config(
        "transform_factory_module_configured",
        CompileConfig {
            factory_modules: vec!["@meonode/mui".to_string()],
        },
    );
}

#[test]
fn transform_factory_module_unconfigured_bail() {
    // Identical source to `transform_factory_module_configured`, but without
    // `factoryModules` configured — Change 4 is opt-in, so `Button(...)` is
    // never even considered a candidate call site and is left untouched.
    run_partition("transform_factory_module_unconfigured_bail");
}

#[test]
fn transform_spread_before_props_bail() {
    run_partition("transform_spread_before_props_bail");
}

#[test]
fn transform_node_with_deps() {
    run_partition("transform_node_with_deps");
}

#[test]
fn transform_empty_props() {
    run_partition("transform_empty_props");
}

#[test]
fn transform_paren_wrapped_props() {
    run_partition("transform_paren_wrapped_props");
}
