//! Tests of this module that read what the engine registers as it loads (the languages): the engine's facade of
//! the module mounts them.
use super::*;

/// T's programs split into file-local relations and the rest: what the cross-file rest reads.
#[test]
fn t_programs_split_into_file_local_and_cross_file() {
    use crate::code_rule_dsl::logic::rr::parse;
    for name in ["calls.rr", "calls_rust.rr", "workload.rr"] {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/code_rule_dsl/typing/programs").join(name);
        let prog = parse(&crate::code_rule_dsl::logic::settings::program_text(&path).unwrap()).unwrap();
        let local = split(&prog);
        let derived: std::collections::BTreeSet<&str> = prog.rules.iter().map(|r| r.head.rel.as_str()).collect();
        let global: Vec<&&str> = derived.iter().filter(|r| !local.contains(**r)).collect();
        let mut reads_base = std::collections::BTreeSet::new();
        let mut reads_local = std::collections::BTreeSet::new();
        for rule in prog.rules.iter().filter(|r| !local.contains(r.head.rel.as_str())) {
            for l in &rule.body {
                if let Some(a) = l.atom() {
                    if prog.inputs.iter().any(|x| x == &a.rel) { reads_base.insert(a.rel.to_string()); }
                    if local.contains(a.rel.as_str()) { reads_local.insert(a.rel.to_string()); }
                }
            }
        }
        // Projections of the tree are file-local; what an import binds is not.
        assert!(local.contains("kind"), "{name}");
        assert!(!local.contains("calls") && !local.contains("res") || name == "workload.rr", "{name}");
        eprintln!("SPLIT {name}: {} local, {} cross-file {global:?}\n  cross-file reads base {reads_base:?}\n  and local {reads_local:?}", local.len(), global.len());
    }
}
