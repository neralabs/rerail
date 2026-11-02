//! The data-layer rules in Rerail logic (t17): the hand-written programs go through the front end.

use super::*;
use std::path::{Path, PathBuf};

fn programs() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src/code_rule_dsl/logic/programs/data_layer")
}

fn error_name(e: &Error) -> &'static str {
    match e {
        Error::Syntax(_) => "Syntax",
        Error::NotStratifiable { .. } => "NotStratifiable",
        Error::AggregationInRecursion { .. } => "AggregationInRecursion",
        Error::UnboundedLattice { .. } => "UnboundedLattice",
        Error::UnboundDemand { .. } => "UnboundDemand",
    }
}

/// Each program, after `prelude.rdl`, parses and stratifies; or, if its first line is `// expect: <Error>`, fails with
/// that named error. Every matcher atom holds JSON.
#[test]
fn data_layer_programs_parse() {
    let prelude = std::fs::read_to_string(programs().join("prelude.rdl")).unwrap();
    let mut names = Vec::new();
    for entry in std::fs::read_dir(programs()).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if !name.ends_with(".rdl") || name == "prelude.rdl" {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        let expect = text.lines().next().and_then(|l| l.strip_prefix("// expect: ")).map(str::trim);
        let result = parse(&format!("{prelude}\n{text}")).and_then(|p| strata(&p).map(|_| p));
        match (expect, result) {
            (None, Ok(p)) => {
                for l in p.rules.iter().flat_map(|r| &r.body) {
                    if let Literal::Ast { matcher, .. } = l {
                        serde_json::from_str::<serde_json::Value>(matcher).unwrap_or_else(|e| panic!("{name}: matcher {matcher}: {e}"));
                    }
                }
            }
            (None, Err(e)) => panic!("{name}: {e}"),
            (Some(want), Ok(_)) => panic!("{name}: parsed, expected {want}"),
            (Some(want), Err(e)) => assert_eq!(error_name(&e), want, "{name}: {e}"),
        }
        names.push(name);
    }
    assert!(!names.is_empty(), "no programs");
}
