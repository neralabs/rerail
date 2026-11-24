//! The data-layer rules in Rerail logic (t17): the hand-written programs, in `.rerail`, go through the front end.

use super::*;
use crate::code_rule_dsl::logic::program_image::{Lit, Version, about::About, analyses, fields::ProgramFields};
use crate::code_rule_dsl::logic::rr::parse_version as parse;
use std::path::PathBuf;

fn programs() -> PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").join("src/code_rule_dsl/logic/programs/data_layer")
}

/// The rules with recursion (E0's eight, t17), each with a program `<rule>.rerail` (`_` for `-`).
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

/// Each program, after `prelude.rerail`, parses and stratifies; or, if its first line is `// expect: <Error>`, fails with
/// that named error. Every relation, in rules and facts, is declared with its arity, every matcher atom holds JSON,
/// and every program's failure is reported. Each recursive rule has a program.
#[test]
fn data_layer_programs_parse() {
    let prelude = std::fs::read_to_string(programs().join("prelude.rerail")).unwrap();
    let (mut names, mut failures) = (Vec::new(), Vec::new());
    for entry in std::fs::read_dir(programs()).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if !name.ends_with(".rerail") || name == "prelude.rerail" {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        let expect = text.lines().next().and_then(|l| l.strip_prefix("// expect: ")).map(str::trim);
        let result = parse(&format!("{prelude}\n{text}")).and_then(|p| analyses::components_by_name(&p, &About::of(&p)).map(|_| p));
        let failure = match (expect, result) {
            (None, Ok(p)) => {
                let p: &Version = &p;
                let arity = |rel: &str| p.columns_of(rel).map(|c| c.len());
                let atoms = p.rules().flat_map(|r| std::iter::once(r.head()).chain(r.reads()).map(move |a| (r, a)));
                let undeclared = atoms.into_iter().find(|(_, a)| arity(p.rel_name(a.rel())) != Some(a.args().count()));
                let facts = p.fact_names();
                let fact = facts.iter().find(|rel| ProgramFields::fact_rows(p, rel).iter().any(|row| arity(rel) != Some(row.len())));
                let undeclared = undeclared.map(|(r, a)| p.show_atom(r, a)).or_else(|| fact.map(|rel| format!("a fact of {rel}")));
                undeclared.map(|a| format!("{a}: undeclared, or not the declared arity")).or_else(|| {
                    p.rules().flat_map(|r| r.body()).find_map(|l| match l.view() {
                        Lit::Ast { matcher, .. } => serde_json::from_str::<serde_json::Value>(p.text(matcher)).err().map(|e| format!("matcher {}: {e}", p.text(matcher))),
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
        assert!(names.contains(&format!("{rule}.rerail")), "no program for {rule}");
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
