//! Conformance: every shipped `.mui` / `.crm` example must parse cleanly.
//!
//! The examples live in `examples/mui/` (one subfolder per example) and
//! double as the parser's regression corpus. If the grammar ever drifts from
//! the very files a new user opens first, this test fails — keeping "what we
//! ship" and "what we parse" in lockstep.

use std::path::PathBuf;

fn examples_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("examples")
        .join("mui")
}

fn parse_ok(file: &str) {
    let path = examples_dir().join(file);
    let src = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    let doc = mui_syntax::parse(&src);
    assert!(
        doc.errors.is_empty(),
        "{file} parsed with errors: {:#?}",
        doc.errors
    );
    assert!(!doc.views.is_empty(), "{file} produced no views");
}

#[test]
fn hello_mui_parses() {
    parse_ok("hello/hello.mui");
}

#[test]
fn counter_mui_parses() {
    parse_ok("counter/counter.mui");
}

#[test]
fn app_crm_parses() {
    // `.crm` mixes Copper items (struct / impl / func) ahead of the view; the
    // structural pass skips them and still recovers the `App` view cleanly.
    parse_ok("crm/app.crm");
}
