//! The import closure (`components.rr`): each Python document's imports, the edges between documents, and each
//! document's closure (`rr_cx_reach`) and the modules it sees (`rr_cx_sees`). A program that reads an `rr_cx_`
//! relation has these rules added (`corpus.rs`, `reads_components`): a lookup by name kept to what the reader
//! imports. They were the cross-file memo's (decision 0076); the memo is removed and they stay (decision 0450 §3).
use super::{Error, Result, front};
use crate::code_rule_dsl::logic::program_image::{Builder, Version};
use std::collections::BTreeSet;
use std::sync::Arc;

/// `components.rr`, parsed.
fn parsed() -> Result<front::Program> {
    let path = crate::code_rule_dsl::logic::rr::library().join("components.rr");
    crate::code_rule_dsl::logic::rr::parse_file(&path).map_err(|e| Error::Plan(format!("{}: {e:?}", path.display())))
}

/// Adds `components.rr` to `prog`: its declarations, natives and rules. Core's relations and rules are the engine's,
/// taken once.
// The tree's edit, kept as a debug build's check of the version's (`run_typed`'s `Beside`).
#[cfg_attr(not(debug_assertions), allow(dead_code))]
pub(super) fn add(prog: &mut front::Program) -> Result<()> {
    let mut fields = parsed()?;
    let rules = taken(&mut fields, prog, |rel| prog.rules.iter().any(|r| r.head.rel.to_string() == rel));
    fields.rules = rules.clone();
    add_fields(prog, fields)?;
    for rule in rules {
        if !prog.rules.contains(&rule) {
            prog.rules.push(rule);
        }
    }
    Ok(())
}

/// `add` on the program's version, `head` its header (`Version::header`), edited as `add` edits the tree's fields:
/// the declarations, natives, qualifiers, facts and inputs by the header, and `components.rr`'s rules
/// copied from its image, each one the program does not hold already.
pub(super) fn add_to(v: &Arc<Version>, head: &mut front::Program) -> Result<Arc<Version>> {
    let mut fields = parsed()?;
    let rules = taken(&mut fields, head, |rel| v.rel(rel).is_some_and(|r| v.heads(r).next().is_some()));
    fields.rules = rules.clone();
    let own = front::Program { rules, decls: fields.decls.clone(), ..Default::default() };
    let was = head.clone();
    let mut edited = head.clone();
    add_fields(&mut edited, fields)?;
    let (image, _) = Builder::of_program(&own).and_then(|b| b.finish()).map_err(Error::Plan)?;
    let mut b = Builder::on(v);
    // The header first: a copied rule's relations are declared.
    b.set_header(&was, &edited);
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
    *head = edited;
    b.finish().map(|(v, _)| v).map_err(Error::Plan)
}

/// The rules taken from `components.rr`: its own (`rr_cx_*` heads), and those its lowering made of core.rr's views for
/// what they read (`kind` over `node`, `lower::projections`) where the program derives none: the program is pruned
/// before the closure is added, so a program that reads only `rr_cx_reach` holds no `kind` rule. Not what else
/// `core.rr` brings, and not a relation the program declares otherwise: it keeps its own declaration, and its input or
/// rule as its lowering made it.
fn taken(components: &mut front::Program, prog: &front::Program, derives: impl Fn(&str) -> bool) -> Vec<front::Rule> {
    let rules = std::mem::take(&mut components.rules);
    let heads = |rel: &str| rules.iter().any(|r| r.head.rel.to_string() == rel);
    let mut want: BTreeSet<String> = rules.iter().map(|r| r.head.rel.to_string()).filter(|h| h.starts_with("rr_cx_")).collect();
    let mut todo: Vec<String> = want.iter().cloned().collect();
    while let Some(rel) = todo.pop() {
        for r in rules.iter().filter(|r| r.head.rel.to_string() == rel) {
            for read in r.body.iter().filter_map(front::Literal::atom).map(|a| a.rel.to_string()) {
                let same_decl = prog.decls.get(read.as_str()).is_none_or(|d| components.decls.get(read.as_str()) == Some(d));
                if !want.contains(&read) && same_decl && heads(&read) && !derives(&read) {
                    want.insert(read.clone());
                    todo.push(read);
                }
            }
        }
    }
    rules.into_iter().filter(|r| want.contains(&r.head.rel.to_string())).collect()
}

/// `add`'s edit of everything but the rules: `components`' natives, declarations, qualifiers, facts and inputs the
/// program lacks, for the rules `taken` gives. A program that declares a base relation itself keeps its own
/// declaration, and what it keeps (`.decl rr_cx_edge(…) no_inline`, decision 0099). A view the program held as an
/// input it did not read (`kind`, core.rr's) is a rule now: no longer an input, as its lowering would have made it.
fn add_fields(prog: &mut front::Program, components: front::Program) -> Result<()> {
    let rules = components.rules;
    prog.inputs.retain(|i| !rules.iter().any(|r| !r.head.rel.starts_with("rr_cx_") && r.head.rel.to_string() == *i));
    let mut needed: BTreeSet<String> = BTreeSet::new();
    for r in &rules {
        needed.insert(r.head.rel.to_string());
        needed.extend(r.body.iter().filter_map(|l| l.atom()).map(|a| a.rel.to_string()));
    }
    let natives: Vec<_> = components.natives.into_iter().filter(|(rel, _)| needed.contains(rel)).collect();
    for (rel, native) in natives {
        needed.extend(native.reads().into_iter().map(str::to_owned));
        prog.natives.entry(rel).or_insert(native);
    }
    let missing: BTreeSet<String> = needed.iter().filter(|rel| !prog.decls.contains_key(rel.as_str())).cloned().collect();
    for (rel, cols) in components.decls.into_iter().filter(|(rel, _)| missing.contains(rel.as_str())) {
        prog.decls.insert(rel, cols);
    }
    for (rel, words) in components.qualifiers.into_iter().filter(|(rel, _)| missing.contains(rel)) {
        prog.qualifiers.entry(rel).or_default().extend(words);
    }
    for (rel, rows) in components.facts.into_iter().filter(|(rel, _)| missing.contains(rel)) {
        prog.facts.entry(rel).or_insert(rows);
    }
    for input in components.inputs.into_iter().filter(|rel| missing.contains(rel)) {
        if !prog.inputs.contains(&input) {
            prog.inputs.push(input);
        }
    }
    Ok(())
}
