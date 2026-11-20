//! A program's `ast(d, n, { … })` atoms, run by the transducer's matchers (decision 0106).
//!
//! Each atom becomes an input relation, one per distinct matcher and captured names: `rr_ast_<k>(n)` without
//! captures, `rr_ast_<k>_<m>(n, x1, …, xm)` with `m` (decision 0107), and `docof(n, d)` where the rule reads `d`
//! elsewhere. Each chunk fills the relations by running the matchers' wave over its documents' tapes
//! (`facts::vtab::syntax::with_matchers`); a captured matcher's rows read W's bindings at each matched root. The
//! joins read the matches as any base relation.
use std::collections::{BTreeSet, HashMap};
use crate::code_rule_dsl::logic::program_image::{Lit, TermRef, Version};

pub use crate::code_rule_dsl::facts::grammar_shape::single_field;
#[cfg(any(test, feature = "test-hooks"))]
use crate::code_rule_dsl::facts::grammar_shape::{field_kinds, leaf_kind};

pub use crate::code_rule_dsl::facts::core::ast_matches::{MOST_CAPTURES, PREFIX, columns};

/// Each relation's matcher compiled for the transducer, over the program's first `.policy` (its utils), by the
/// registered compiler (`facts::vtab::syntax::AstCompiler`), with the matchers whose roots bound its rows
/// ([`demanded_by_roots`]). A matcher that does not lower fails the run, naming it.
pub(in crate::code_rule_dsl) fn compiled(
    matchers: &[(String, String, Vec<String>)],
    demand: &[Option<Vec<usize>>],
    policies: &[serde_json::Value],
) -> Result<crate::code_rule_dsl::facts::vtab::syntax::Matchers, String> {
    let _span = crate::code_rule_dsl::logic::profile::span(|| "ast matchers lowered".into());
    crate::code_rule_dsl::facts::vtab::syntax::compile_matchers(matchers, demand, policies)
}

/// Per matcher, the other matchers whose roots hold every node its rows are read at, where the rules say so
/// (decision 0459): each rule that reads its relation joins its node with a column that only other matchers' roots
/// fill (`raw_module_name(w, …)`, made from `rr_ast_11(w)`, beside `rr_ast_14(w, f)`). The wave knows every
/// matcher's roots before it binds any capture, so a matcher's captures are bound only at the nodes in those roots:
/// internal_dead_code's `mn_function` bound 737,806 roots for the 27,849 its join keeps (t45 §2). The magic rule of
/// the atom is widened to the document where its body is the batch's own relations; this is that rule at node grain,
/// where its body is the same wave's matchers. `None` where any read is not so joined: negated, in an aggregate, an
/// output, or joined only with what the batch's rows or the inputs fill.
pub fn demanded_by_roots(v: &Version, matchers: &[(String, String, Vec<String>)]) -> Vec<Option<Vec<usize>>> {
    use crate::code_rule_dsl::logic::program_image::fields::ProgramFields;
    let outputs = v.output_names();
    // The relations something besides matchers' roots fills: the inputs, native and lattice relations, and the
    // relations with fact rows (read once, for every matcher's walk).
    let filled_elsewhere: BTreeSet<String> = v
        .input_names()
        .into_iter()
        .chain(v.natives().into_keys())
        .chain(v.lattice().into_keys())
        .chain(v.fact_names().into_iter().filter(|rel| v.has_fact_rows(rel)))
        .collect();
    let index: HashMap<&str, usize> = matchers.iter().enumerate().map(|(k, (rel, _, _))| (rel.as_str(), k)).collect();
    let mut known = HashMap::new();
    matchers
        .iter()
        .enumerate()
        .map(|(k, (rel, _, _))| {
            if outputs.iter().any(|out| out == rel) {
                return None;
            }
            let mut by = BTreeSet::new();
            let mut read = false;
            for rule in v.rules() {
                let body: Vec<Lit<'_>> = rule.body().map(|literal| literal.view()).collect();
                for (at, literal) in body.iter().enumerate() {
                    let atom = match *literal {
                        Lit::Atom(atom) if v.rel_name(atom.rel()) == rel.as_str() => atom,
                        Lit::Agg { atom, .. } if v.rel_name(atom.rel()) == rel.as_str() => return None,
                        _ => continue,
                    };
                    let Some(node) = atom.args().next().and_then(TermRef::var).filter(|_| !atom.neg()) else { return None };
                    read = true;
                    // Another positive atom of the body that binds the node where only matchers' roots fill it.
                    let roots = body.iter().enumerate().filter(|(other, _)| *other != at).find_map(|(_, other)| {
                        let Lit::Atom(other) = *other else { return None };
                        if other.neg() {
                            return None;
                        }
                        other.args().enumerate().find_map(|(column, arg)| {
                            (arg.var() == Some(node)).then(|| filled_by_roots(&filled_elsewhere, v, &index, v.rel_name(other.rel()), column, &mut known, &mut Vec::new())).flatten()
                        })
                    });
                    match roots {
                        Some(roots) if !roots.contains(&k) => by.extend(roots),
                        _ => return None,
                    }
                }
            }
            (read && !by.is_empty()).then(|| by.into_iter().collect())
        })
        .collect()
}

/// The matchers whose roots hold every value of `rel`'s `column`, where only they fill it: a matcher's own relation's
/// node, or a derived relation whose every rule binds the column from such a column of a positive atom.
fn filled_by_roots(
    filled_elsewhere: &BTreeSet<String>,
    v: &Version,
    index: &HashMap<&str, usize>,
    rel: &str,
    column: usize,
    known: &mut HashMap<(String, usize), Option<BTreeSet<usize>>>,
    visiting: &mut Vec<(String, usize)>,
) -> Option<BTreeSet<usize>> {
    use crate::code_rule_dsl::logic::program_image::fields::ProgramFields as _;
    if let Some(&k) = index.get(rel) {
        return (column == 0).then(|| BTreeSet::from([k]));
    }
    let key = (rel.to_owned(), column);
    if let Some(found) = known.get(&key) {
        return found.clone();
    }
    // A cycle, an input, a native or lattice relation, or a relation with facts: not only matchers' roots.
    if visiting.contains(&key) || filled_elsewhere.contains(rel) {
        return None;
    }
    visiting.push(key.clone());
    let mut found = Some(BTreeSet::new());
    let mut rules = 0;
    for rule in v.rules().filter(|rule| v.rel_name(rule.head().rel()) == rel) {
        rules += 1;
        let Some(var) = rule.head().args().nth(column).and_then(TermRef::var) else {
            found = None;
            break;
        };
        let from = rule.body().find_map(|literal| {
            let Lit::Atom(atom) = literal.view() else { return None };
            if atom.neg() {
                return None;
            }
            atom.args().enumerate().find_map(|(c, arg)| (arg.var() == Some(var)).then(|| filled_by_roots(filled_elsewhere, v, index, v.rel_name(atom.rel()), c, known, visiting)).flatten())
        });
        match (from, found.as_mut()) {
            (Some(from), Some(found)) => found.extend(from),
            _ => {
                found = None;
                break;
            }
        }
    }
    visiting.pop();
    let found = found.filter(|_| rules > 0);
    known.insert(key, found.clone());
    found
}

// cratify: its tests (`matchers/tests.rs`) are rerail-engine's, which mounts them: they run what this crate is below.
