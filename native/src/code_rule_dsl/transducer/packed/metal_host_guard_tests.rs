//! No host-side evaluation on the Metal path: what Metal's kernels answer, the device computes.
//! The code that plans and runs them (every non-test `metal_*.rs` beside this file, and the Metal
//! entry points of `compile/matcher.rs`) may not call a host evaluator: a pattern matched, a token
//! recognized, a fact found or a program run on the CPU. A pass that needs one is ported to a
//! kernel (as patterns over named-only documents were, `token_parts`), never computed on the host
//! and handed over.
//!
//! `REMAINING` lists what still is, each to be ported and then removed here; adding a call to any
//! of `HOST_EVALUATORS` fails this test, and so does porting one without removing its entry.

const HOST_EVALUATORS: [&str; 14] = [
    "execute_ast_pattern",
    "pattern_children",
    "transducer_parts",
    "transducer_tokens",
    "record_tokens",
    "RecordTokens",
    "TapeTokens",
    ".sequence(",
    "tokens::unfold",
    "execute_roots",
    "execute_program",
    "run_cpu",
    "fact_rows(",
    ".found(",
];

/// (where, evaluator) still on the host. `metal`: the facts' rows are made on the host and tested
/// on the device (`Pass::Fact`).
const REMAINING: [(&str, &str); 1] = [("compile/matcher.rs::metal", "fact_rows(")];

/// Code with its comments removed (line comments, doc comments; strings are kept).
fn code(source: &str) -> String {
    source
        .lines()
        .map(|line| {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") {
                return "";
            }
            line.find(" // ").map_or(line, |at| &line[..at])
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The body of `fn name` (from its line to the first line that is a lone `}` at its indent).
fn function<'s>(source: &'s str, name: &str) -> Vec<&'s str> {
    let mut found = Vec::new();
    let lines: Vec<&str> = source.lines().collect();
    for (at, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        let signature = trimmed.strip_prefix("pub(in crate::code_rule_dsl) ").unwrap_or(trimmed);
        if !(signature.starts_with(&format!("fn {name}(")) || signature.starts_with(&format!("fn {name}<"))) {
            continue;
        }
        let indent = line.len() - trimmed.len();
        let end = lines[at..].iter().position(|l| l.len() == indent + 1 && l.trim() == "}").map_or(lines.len(), |e| at + e);
        found.push(lines[at..=end.min(lines.len() - 1)].join("\n").leak() as &str);
    }
    found
}

#[test]
fn the_metal_path_calls_no_host_evaluator() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/code_rule_dsl/transducer/packed");
    let mut sources: Vec<(String, String)> = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("the packed directory") {
        let path = entry.expect("an entry").path();
        let name = path.file_name().and_then(|name| name.to_str()).unwrap_or_default().to_owned();
        if !name.starts_with("metal_") || !name.ends_with(".rs") || name.contains("tests") || name.contains("conformance") {
            continue;
        }
        sources.push((name, std::fs::read_to_string(&path).expect("a source")));
    }
    assert!(sources.len() > 20, "the Metal sources");
    let matcher = std::fs::read_to_string(dir.join("compile/matcher.rs")).expect("compile/matcher.rs");
    for name in ["metal", "roots_metal", "labelled_metal", "warm_metal"] {
        let bodies = function(&matcher, name);
        assert!(!bodies.is_empty(), "compile/matcher.rs has fn {name}");
        for body in bodies {
            sources.push((format!("compile/matcher.rs::{name}"), body.to_owned()));
        }
    }
    let mut found = Vec::new();
    for (name, source) in &sources {
        let code = code(source);
        for evaluator in HOST_EVALUATORS {
            if code.contains(evaluator) && !found.contains(&(name.clone(), evaluator)) {
                found.push((name.clone(), evaluator));
            }
        }
    }
    found.sort();
    let mut remaining: Vec<(String, &str)> = REMAINING.iter().map(|(name, evaluator)| ((*name).to_owned(), *evaluator)).collect();
    remaining.sort();
    assert_eq!(found, remaining, "host evaluation on the Metal path: port it to a kernel (see this module)");
}
