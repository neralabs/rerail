//! The import closure (`components.rr`): each Python document's imports, the edges between documents, and each
//! document's closure (`rr_cx_reach`) and the modules it sees (`rr_cx_sees`). A program that reads an `rr_cx_`
//! relation has these rules added (`corpus.rs`, `reads_components`): a lookup by name kept to what the reader
//! imports. They were the cross-file memo's (decision 0076); the memo is removed and they stay (decision 0450 §3).
use super::{Error, Result};
use crate::code_rule_dsl::logic::program_image::{Builder, Version};
use std::collections::BTreeSet;
use std::sync::Arc;

/// `components.rr`, parsed.
fn parsed() -> Result<Arc<Version>> {
    let path = crate::code_rule_dsl::logic::rr::library().join("components.rr");
    crate::code_rule_dsl::logic::rr::parse_file_version(&path).map_err(|e| Error::Plan(format!("{}: {e:?}", path.display())))
}

/// Adds `components.rr` to the program's version: the rules `taken` gives, copied from its image, each one the
/// program does not hold already, and what they need of its declarations, natives, qualifiers, facts and inputs.
/// Core's relations and rules are the engine's, taken once. A view the program held as an input it did not read
/// (`kind`, core.rr's) is a rule now: no longer an input, as its lowering would have made it.
pub(super) fn add_to(v: &Arc<Version>) -> Result<Arc<Version>> {
    use crate::code_rule_dsl::logic::program_facts::Field;
    use crate::code_rule_dsl::logic::program_image::fields::ProgramFields;
    let image = parsed()?;
    let want = taken(&image, |rel| v.columns_of(rel), |rel| v.rel(rel).is_some_and(|r| v.heads(r).next().is_some()));
    let rules: Vec<_> = image.rules().filter(|r| want.contains(image.rel_name(r.head().rel()))).collect();
    let mut needed: BTreeSet<String> = BTreeSet::new();
    for r in &rules {
        needed.insert(image.rel_name(r.head().rel()).to_owned());
        needed.extend(r.reads().map(|a| image.rel_name(a.rel()).to_owned()));
    }
    let mut b = Builder::on(v);
    b.fields().retain_inputs(|i| !want.iter().any(|h| !h.starts_with("rr_cx_") && h == i));
    // The fields first: a copied rule's relations are declared.
    let mut natives = v.native_map().map_err(Error::Plan)?;
    let natives_were = natives.len();
    let theirs = image.native_map().map_err(Error::Plan)?;
    let taken: Vec<_> = theirs.iter().filter(|(rel, _)| needed.contains(*rel)).collect();
    for (rel, native) in taken {
        needed.extend(native.reads().into_iter().map(str::to_owned));
        natives.entry(rel.clone()).or_insert_with(|| native.clone());
    }
    if natives.len() != natives_were {
        b.fields().set_field(Field::Natives(&natives));
    }
    let missing: BTreeSet<String> = needed.iter().filter(|rel| !v.declares(rel)).cloned().collect();
    for rel in &missing {
        let Some(cols) = image.columns_of(rel) else { continue };
        let pairs: Vec<(&str, &str)> = cols.iter().map(|(n, t)| (n.as_str(), t.as_str())).collect();
        b.fields().declare(rel, &pairs);
    }
    let mut qualifiers = v.qualifier_map().map_err(Error::Plan)?;
    let mut qualified = false;
    for (rel, words) in image.qualifier_map().map_err(Error::Plan)?.iter().filter(|(rel, _)| missing.contains(*rel)) {
        let had = qualifiers.entry(rel.clone()).or_default();
        let n = had.len();
        had.extend(words.iter().cloned());
        qualified |= had.len() != n;
    }
    if qualified {
        b.fields().set_field(Field::Qualifiers(&qualifiers));
    }
    for rel in ProgramFields::fact_names(&*image).iter().filter(|rel| missing.contains(*rel)) {
        if !ProgramFields::has_fact_rows(&**v, rel) {
            b.fields().add_facts(rel, &ProgramFields::fact_rows(&*image, rel));
        }
    }
    for input in image.input_names().iter().filter(|rel| missing.contains(*rel)) {
        b.fields().add_input(input);
    }
    // A rule the program holds already is taken once (by its hash and its text).
    let held: std::collections::HashMap<u64, Vec<String>> = v.rules().fold(std::collections::HashMap::new(), |mut m, r| {
        m.entry(r.hash()).or_insert_with(Vec::new).push(v.show_rule(r));
        m
    });
    let mut added: std::collections::HashSet<String> = std::collections::HashSet::new();
    for r in rules {
        let text = image.show_rule(r);
        if held.get(&r.hash()).is_some_and(|ts| ts.contains(&text)) || !added.insert(text) {
            continue;
        }
        b.copy(&image, r);
    }
    b.finish().map(|(v, _)| v).map_err(Error::Plan)
}

/// The heads of the rules taken from `components.rr`: its own (`rr_cx_*` heads), and those its lowering made of core.rr's views for
/// what they read (`kind` over `node`, `lower::projections`) where the program derives none: the program is pruned
/// before the closure is added, so a program that reads only `rr_cx_reach` holds no `kind` rule. Not what else
/// `core.rr` brings, and not a relation the program declares otherwise: it keeps its own declaration, and its input or
/// rule as its lowering made it.
fn taken(image: &Version, columns: impl Fn(&str) -> Option<Vec<(String, String)>>, derives: impl Fn(&str) -> bool) -> BTreeSet<String> {
    let name = |r: &crate::code_rule_dsl::logic::program_image::RuleRef<'_>| image.rel_name(r.head().rel()).to_owned();
    let heads = |rel: &str| image.rel(rel).is_some_and(|r| image.heads(r).next().is_some());
    let mut want: BTreeSet<String> = image.rules().map(|r| name(&r)).filter(|h| h.starts_with("rr_cx_")).collect();
    let mut todo: Vec<String> = want.iter().cloned().collect();
    while let Some(rel) = todo.pop() {
        let Some(id) = image.rel(&rel) else { continue };
        for r in image.heads(id) {
            for read in r.reads().map(|a| image.rel_name(a.rel()).to_owned()) {
                let same_decl = columns(&read).is_none_or(|d| image.columns_of(&read) == Some(d));
                if !want.contains(&read) && same_decl && heads(&read) && !derives(&read) {
                    want.insert(read.clone());
                    todo.push(read);
                }
            }
        }
    }
    want
}
