//! The data-layer rules in Rerail logic (t17): the hand-written programs, in `.rr`, go through the front end.

use super::*;
use crate::code_rule_dsl::logic::rr::parse;
use std::path::{Path, PathBuf};

fn programs() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src/code_rule_dsl/logic/programs/data_layer")
}

/// The rules with recursion (E0's eight, t17), each with a program `<rule>.rr` (`_` for `-`).
const RECURSIVE: [&str; 8] = [
    "execution_authority_bypass",
    "internal_dead_code",
    "legacy_execution_surfaces",
    "public_api_triples",
    "serverplan_dispatch_architecture",
    "serverplan_quality",
    "serverplan_state_tripwires",
    "serverplan_steps",
];

fn error_name(e: &Error) -> &'static str {
    match e {
        Error::Syntax(_) => "Syntax",
        Error::NotStratifiable { .. } => "NotStratifiable",
        Error::AggregationInRecursion { .. } => "AggregationInRecursion",
        Error::UnboundedLattice { .. } => "UnboundedLattice",
        Error::UnboundDemand { .. } => "UnboundDemand",
        Error::UnboundedValueCycle { .. } => "UnboundedValueCycle",
        Error::UndeclaredInput { .. } => "UndeclaredInput",
    }
}

/// Each program, after `prelude.rr`, parses and stratifies; or, if its first line is `// expect: <Error>`, fails with
/// that named error. Every relation, in rules and facts, is declared with its arity, every matcher atom holds JSON,
/// and every program's failure is reported. Each recursive rule has a program.
#[test]
fn data_layer_programs_parse() {
    let prelude = std::fs::read_to_string(programs().join("prelude.rr")).unwrap();
    let (mut names, mut failures) = (Vec::new(), Vec::new());
    for entry in std::fs::read_dir(programs()).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if !name.ends_with(".rr") || name == "prelude.rr" {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        let expect = text.lines().next().and_then(|l| l.strip_prefix("// expect: ")).map(str::trim);
        let result = parse(&format!("{prelude}\n{text}")).and_then(|p| strata(&p).map(|_| p));
        let failure = match (expect, result) {
            (None, Ok(p)) => {
                let atoms = p.rules.iter().flat_map(|r| std::iter::once(&r.head).chain(r.body.iter().filter_map(Literal::atom)));
                let undeclared = atoms.into_iter().find(|a| p.decls.get(&a.rel).is_none_or(|c| c.len() != a.args.len()));
                let fact = p.facts.iter().find(|(rel, rows)| rows.iter().any(|row| p.decls.get(*rel).is_none_or(|c| c.len() != row.len())));
                let undeclared = undeclared.map(|a| a.to_string()).or_else(|| fact.map(|(rel, _)| format!("a fact of {rel}")));
                undeclared.map(|a| format!("{a}: undeclared, or not the declared arity")).or_else(|| {
                    p.rules.iter().flat_map(|r| &r.body).find_map(|l| match l {
                        Literal::Ast { matcher, .. } => serde_json::from_str::<serde_json::Value>(matcher).err().map(|e| format!("matcher {matcher}: {e}")),
                        _ => None,
                    })
                })
            }
            (None, Err(e)) => Some(e.to_string()),
            (Some(want), Ok(_)) => Some(format!("parsed, expected {want}")),
            (Some(want), Err(e)) => (error_name(&e) != want).then(|| format!("expected {want}, got {e}")),
        };
        failures.extend(failure.map(|f| format!("{name}: {f}")));
        names.push(name);
    }
    for rule in RECURSIVE {
        assert!(names.contains(&format!("{rule}.rr")), "no program for {rule}");
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
