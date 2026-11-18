//! Tests of this module that read what the engine registers as it loads (the languages): the engine's facade of
//! the module mounts them.
use super::*;

/// T's programs split into file-local relations and the rest: what the cross-file rest reads.
#[test]
fn t_programs_split_into_file_local_and_cross_file() {
    use crate::code_rule_dsl::logic::program_image::fields::ProgramFields;
    for name in ["calls.rr", "calls_rust.rr", "workload.rr"] {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/code_rule_dsl/typing/programs").join(name);
        let v = crate::code_rule_dsl::logic::rr::parse_version(&crate::code_rule_dsl::logic::settings::program_text(&path).unwrap()).unwrap();
        let local = crate::code_rule_dsl::logic::program_image::manager::analysis::<FileLocal>(&v).unwrap().local.clone();
        let inputs = v.input_names();
        let derived: std::collections::BTreeSet<&str> = v.rules().map(|r| v.rel_name(r.head().rel())).collect();
        let global: Vec<&&str> = derived.iter().filter(|r| !local.contains(**r)).collect();
        let mut reads_base = std::collections::BTreeSet::new();
        let mut reads_local = std::collections::BTreeSet::new();
        for rule in v.rules().filter(|r| !local.contains(v.rel_name(r.head().rel()))) {
            for a in rule.body().filter_map(|l| l.atom()) {
                let rel = v.rel_name(a.rel());
                if inputs.iter().any(|x| x == rel) { reads_base.insert(rel.to_owned()); }
                if local.contains(rel) { reads_local.insert(rel.to_owned()); }
            }
        }
        // Projections of the tree are file-local; what an import binds is not.
        assert!(local.contains("kind"), "{name}");
        assert!(!local.contains("calls") && !local.contains("res") || name == "workload.rr", "{name}");
        eprintln!("SPLIT {name}: {} local, {} cross-file {global:?}\n  cross-file reads base {reads_base:?}\n  and local {reads_local:?}", local.len(), global.len());
    }
}
