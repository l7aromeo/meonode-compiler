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

use meonode_swc_plugin::detect;
use swc_core::common::Mark;
use swc_core::ecma::ast::{Pass, Program};
use swc_core::ecma::parser::{EsSyntax, Syntax};
use swc_core::ecma::transforms::base::resolver;
use swc_core::ecma::transforms::testing::{test_fixture, FixtureTestConfig, Tester};

/// A `Pass` that runs detection for its side effect (recording decisions)
/// and returns the program untouched — mirroring
/// `lib.rs::process_transform`, which does exactly this in the real plugin.
struct DetectOnly;

impl Pass for DetectOnly {
    fn process(&mut self, program: &mut Program) {
        let _decisions = detect::transform_program(program);
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
