//! The import closure (`components.rerail`): each Python document's imports, the edges between documents, and each
//! document's closure (`rr_cx_reach`) and the modules it sees (`rr_cx_sees`). A program that reads an `rr_cx_`
//! relation has these rules added (`corpus.rs`, `reads_components`): a lookup by name kept to what the reader
//! imports. They were the cross-file memo's (decision 0076); the memo is removed and they stay (decision 0450 §3).
use super::{Error, Result, front};
use crate::code_rule_dsl::logic::program_image::{Builder, Version};
use std::collections::BTreeSet;
use std::sync::Arc;

/// `components.rerail`, parsed.
fn parsed() -> Result<front::Program> {
    let path = crate::code_rule_dsl::logic::rr::library().join("components.rerail");
    crate::code_rule_dsl::logic::rr::parse_file(&path).map_err(|e| Error::Plan(format!("{}: {e:?}", path.display())))
}

/// Adds `components.rerail` to the program's version: the rules `taken` gives, copied from its image, each one the
/// program does not hold already, and what they need of its declarations, natives, qualifiers, facts and inputs.
/// Core's relations and rules are the engine's, taken once. A view the program held as an input it did not read
/// (`kind`, core.rerail's) is a rule now: no longer an input, as its lowering would have made it.
pub(super) fn add_to(v: &Arc<Version>) -> Result<Arc<Version>> {
    use crate::code_rule_dsl::logic::program_facts::Field;
    use crate::code_rule_dsl::logic::program_image::fields::ProgramFields;
    let mut components = parsed()?;
    let rules = taken(&mut components, |rel| v.columns_of(rel), |rel| v.rel(rel).is_some_and(|r| v.heads(r).next().is_some()));
    let own = front::Program { rules: rules.clone(), decls: components.decls.clone(), ..Default::default() };
    let (image, _) = Builder::of_program(&own).and_then(|b| b.finish()).map_err(Error::Plan)?;
    let mut needed: BTreeSet<String> = BTreeSet::new();
    for r in &rules {
        needed.insert(r.head.rel.to_string());
        needed.extend(r.body.iter().filter_map(|l| l.atom()).map(|a| a.rel.to_string()));
    }
    let mut b = Builder::on(v);
    b.retain_inputs(|i| !rules.iter().any(|r| !r.head.rel.starts_with("rr_cx_") && r.head.rel.to_string() == i));
    // The fields first: a copied rule's relations are declared.
    let mut natives = v.native_map().map_err(Error::Plan)?;
    let natives_were = natives.len();
    let taken: Vec<_> = components.natives.iter().filter(|(rel, _)| needed.contains(*rel)).collect();
    for (rel, native) in taken {
        needed.extend(native.reads().into_iter().map(str::to_owned));
        natives.entry(rel.clone()).or_insert_with(|| native.clone());
    }
    if natives.len() != natives_were {
        b.set_field(Field::Natives(&natives));
    }
    let missing: BTreeSet<String> = needed.iter().filter(|rel| !v.declares(rel)).cloned().collect();
    for (rel, cols) in components.decls.iter().filter(|(rel, _)| missing.contains(rel.as_str())) {
        let pairs: Vec<(&str, &str)> = cols.iter().map(|(n, t)| (n.as_str(), t.as_str())).collect();
        b.declare(rel, &pairs);
    }
    let mut qualifiers = v.qualifier_map().map_err(Error::Plan)?;
    let mut qualified = false;
    for (rel, words) in components.qualifiers.iter().filter(|(rel, _)| missing.contains(*rel)) {
        let had = qualifiers.entry(rel.clone()).or_default();
        let n = had.len();
        had.extend(words.iter().cloned());
        qualified |= had.len() != n;
    }
    if qualified {
        b.set_field(Field::Qualifiers(&qualifiers));
    }
    for (rel, rows) in components.facts.iter().filter(|(rel, _)| missing.contains(*rel)) {
        if !ProgramFields::has_fact_rows(&**v, rel) {
            b.add_facts(rel, rows);
        }
    }
    for input in components.inputs.iter().filter(|rel| missing.contains(*rel)) {
        b.add_input(input);
    }
    // A rule the program holds already is taken once (by its hash and its text).
    let held: std::collections::HashMap<u64, Vec<String>> = v.rules().fold(std::collections::HashMap::new(), |mut m, r| {
        m.entry(r.hash()).or_insert_with(Vec::new).push(v.show_rule(r));
        m
    });
    let mut added: std::collections::HashSet<String> = std::collections::HashSet::new();
    for r in image.rules() {
        let text = image.show_rule(r);
        if held.get(&r.hash()).is_some_and(|ts| ts.contains(&text)) || !added.insert(text) {
            continue;
        }
        b.copy(&image, r);
    }
    b.finish().map(|(v, _)| v).map_err(Error::Plan)
}

/// The rules taken from `components.rerail`: its own (`rr_cx_*` heads), and those its lowering made of core.rerail's views for
/// what they read (`kind` over `node`, `lower::projections`) where the program derives none: the program is pruned
/// before the closure is added, so a program that reads only `rr_cx_reach` holds no `kind` rule. Not what else
/// `core.rerail` brings, and not a relation the program declares otherwise: it keeps its own declaration, and its input or
/// rule as its lowering made it.
fn taken(components: &mut front::Program, columns: impl Fn(&str) -> Option<Vec<(String, String)>>, derives: impl Fn(&str) -> bool) -> Vec<front::Rule> {
    let rules = std::mem::take(&mut components.rules);
    let heads = |rel: &str| rules.iter().any(|r| r.head.rel.to_string() == rel);
    let mut want: BTreeSet<String> = rules.iter().map(|r| r.head.rel.to_string()).filter(|h| h.starts_with("rr_cx_")).collect();
    let mut todo: Vec<String> = want.iter().cloned().collect();
    while let Some(rel) = todo.pop() {
        for r in rules.iter().filter(|r| r.head.rel.to_string() == rel) {
            for read in r.body.iter().filter_map(front::Literal::atom).map(|a| a.rel.to_string()) {
                let theirs = components.decls.get(read.as_str()).map(|d| d.iter().map(|(n, t)| (n.to_string(), t.to_string())).collect::<Vec<_>>());
                let same_decl = columns(&read).is_none_or(|d| theirs == Some(d));
                if !want.contains(&read) && same_decl && heads(&read) && !derives(&read) {
                    want.insert(read.clone());
                    todo.push(read);
                }
            }
        }
    }
    rules.into_iter().filter(|r| want.contains(&r.head.rel.to_string())).collect()
}
