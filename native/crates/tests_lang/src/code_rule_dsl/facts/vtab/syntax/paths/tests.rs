//! Node paths' tables and codes (decision 0722).

use crate::code_rule_dsl::language::{Family, GrammarDialect, base, tape_tag};

/// Every name a family's node path tables hold is a kind or field of one of its grammars: a table written for another
/// version of a grammar fails here, not as paths made longer by escapes.
#[test]
fn every_name_in_a_familys_path_tables_is_one_of_its_grammars() {
    let mut families: Vec<(Family, Vec<GrammarDialect>)> = Vec::new();
    for &d in GrammarDialect::all() {
        if tape_tag(d).is_none() {
            continue;
        }
        match families.iter_mut().find(|(f, _)| *f == Family::of(d)) {
            Some((_, ds)) => ds.push(d),
            None => families.push((Family::of(d), vec![d])),
        }
    }
    let mut wrong = Vec::new();
    for (family, dialects) in &families {
        let tables = base(*family).path_tables();
        let kind = |k: &str| dialects.iter().any(|d| d.kinds().kind_id(k) != 0);
        let field = |f: &str| f.is_empty() || dialects.iter().any(|d| d.kinds().field_id(f).is_some());
        for p in tables.parents {
            if !kind(p.kind) {
                wrong.push(format!("{family:?}: parent {}", p.kind));
            }
            for s in p.slots {
                if !field(s.field) {
                    wrong.push(format!("{family:?}: {}'s field {}", p.kind, s.field));
                }
                if !s.grouped && !s.kinds.is_empty() {
                    wrong.push(format!("{family:?}: {}.{} lists kinds but is not kind-grouped", p.kind, s.field));
                }
                wrong.extend(s.kinds.iter().filter(|k| !kind(k)).map(|k| format!("{family:?}: {}.{}'s kind {k}", p.kind, s.field)));
            }
        }
        wrong.extend(tables.lists.iter().filter(|k| !kind(k)).map(|k| format!("{family:?}: list {k}")));
        wrong.extend(tables.labels.iter().filter(|(n, _)| !n.strip_prefix('.').map_or_else(|| kind(n), field)).map(|(n, _)| format!("{family:?}: label of {n}")));
    }
    assert!(wrong.is_empty(), "names no grammar of the family has:\n{}", wrong.join("\n"));
}
