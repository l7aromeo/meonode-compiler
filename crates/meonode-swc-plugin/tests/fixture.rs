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
struct DetectOnly;

impl Pass for DetectOnly {
    fn process(&mut self, program: &mut Program) {
        let _decisions = detect::transform_program(program);
    }
}

/// A `Pass` that runs the full Task 9 rewrite (detection + prop
/// partitioning), mirroring `lib.rs::process_transform` in the real plugin.
struct Partition;

impl Pass for Partition {
    fn process(&mut self, program: &mut Program) {
        partition::transform_program(program, FIXTURE_FILENAME);
    }
}

fn tr(_tester: &mut Tester) -> impl Pass {
    // Fresh marks per fixture run, exactly like `resolver`'s own docs and
    // real-world swc plugin fixtures do. In production this pass isn't run
    // by the plugin at all — the host runs it before handing the program to
    // the plugin (see `detect.rs` module docs) — but `test_fixture` starts
    // from a freshly parsed, unresolved AST, so the fixture harness has to
    // do it explicitly.
    (resolver(Mark::new(), Mark::new(), false), DetectOnly)
}

fn tr_partition(_tester: &mut Tester) -> impl Pass {
    (resolver(Mark::new(), Mark::new(), false), Partition)
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
fn transform_effectful_bail() {
    run_partition("transform_effectful_bail");
}

#[test]
fn transform_member_expr_bail() {
    run_partition("transform_member_expr_bail");
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
fn transform_children_not_last_bail() {
    run_partition("transform_children_not_last_bail");
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
