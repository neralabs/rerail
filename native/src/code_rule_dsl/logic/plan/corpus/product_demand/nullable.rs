//! Recover a tape edge already witnessed by a present location. The edge gives the split a document for the
//! location, even when the relation it came from can also return the unset value.

use super::{atom, plain};
use crate::code_rule_dsl::logic::front::{Literal, Program, Rule, Term};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Witness {
    anchor: usize,
    edge: &'static str,
    edge_anchor: usize,
    edge_value: usize,
}

type Key<'a> = (&'a str, usize);
type Witnesses = BTreeSet<Witness>;

struct Proofs<'a> {
    program: &'a Program,
    producers: BTreeMap<&'a str, Vec<&'a Rule>>,
    known: BTreeMap<Key<'a>, Witnesses>,
}

fn compared_to_unset(literal: &Literal, term: &Term, comparison: &str) -> bool {
    matches!(
        literal,
        Literal::Cmp { l, op, r }
            if *op == comparison
                && ((l == term && *r == Term::Int(-1)) || (r == term && *l == Term::Int(-1)))
    )
}

impl<'a> Proofs<'a> {
    fn location(&self, rel: &str, column: usize) -> bool {
        self.program
            .decls
            .get(rel)
            .and_then(|cols| cols.get(column))
            .is_some_and(|(_, ty)| matches!(ty.as_str(), "loc" | "node"))
    }

    fn for_column(&mut self, rel: &'a str, column: usize, fuel: usize) -> Witnesses {
        let key = (rel, column);
        if let Some(found) = self.known.get(&key) {
            return found.clone();
        }
        if fuel == 0
            || !self.location(rel, column)
            || self.program.facts.contains_key(rel)
            || self.program.inputs.iter().any(|input| input == rel)
            || self.program.natives.contains_key(rel)
            || self.program.answers.values().flatten().any(|answer| answer == rel)
        {
            return Witnesses::new();
        }
        let Some(rules) = self.producers.get(rel) else {
            return Witnesses::new();
        };
        self.known.insert(key, Witnesses::new());
        let mut common: Option<Witnesses> = None;
        for rule in rules.clone() {
            let Some(value) = rule.head.args.get(column) else {
                return Witnesses::new();
            };
            if *value == Term::Int(-1) || rule.body.iter().any(|l| compared_to_unset(l, value, "=")) {
                continue;
            }
            let mut here = Witnesses::new();
            if matches!(value, Term::Var(_)) {
                for body in rule.body.iter().filter_map(|lit| match lit {
                    Literal::Atom(a) if !a.neg && !a.gated => Some(a),
                    _ => None,
                }) {
                    let witnesses = body
                        .args
                        .iter()
                        .enumerate()
                        .filter(|(_, t)| *t == value)
                        .flat_map(|(i, _)| {
                            let rel = body.rel.as_str();
                            let edge = match (rel, i, body.args.len()) {
                                ("field", 0, 3) => Some(("field", 2)),
                                ("field", 2, 3) => Some(("field", 0)),
                                ("child", 0, 3) => Some(("child", 1)),
                                ("child", 1, 3) => Some(("child", 0)),
                                _ => None,
                            };
                            if let Some((edge, anchor)) = edge {
                                return Witnesses::from([Witness {
                                    anchor,
                                    edge,
                                    edge_anchor: anchor,
                                    edge_value: i,
                                }]);
                            }
                            self.for_column(rel, i, fuel - 1)
                        });
                    for witness in witnesses {
                        let Some(anchor) = body.args.get(witness.anchor) else {
                            continue;
                        };
                        if !matches!(anchor, Term::Var(_)) || anchor == value {
                            continue;
                        }
                        for (i, _) in rule.head.args.iter().enumerate().filter(|(_, t)| *t == anchor) {
                            here.insert(Witness { anchor: i, ..witness });
                        }
                    }
                }
            }
            common = Some(common.map_or_else(|| here.clone(), |before| before.intersection(&here).copied().collect()));
            if common.as_ref().is_some_and(BTreeSet::is_empty) {
                return Witnesses::new();
            }
        }
        let found = common.unwrap_or_default();
        self.known.insert(key, found.clone());
        found
    }
}

/// A reader rejects unset explicitly. Probe the same tape edge its surviving rows must have read.
pub(super) fn restore(program: &mut Program) -> usize {
    let mut proofs = Proofs {
        program,
        producers: BTreeMap::new(),
        known: BTreeMap::new(),
    };
    for rule in &program.rules {
        proofs.producers.entry(&rule.head.rel).or_default().push(rule);
    }
    let mut edits = Vec::new();
    for (rule_index, rule) in program.rules.iter().enumerate() {
        let mut fresh = None;
        for (index, lit) in rule.body.iter().enumerate() {
            let Literal::Atom(a) = lit else { continue };
            if a.neg || a.gated {
                continue;
            }
            for (column, value) in a
                .args
                .iter()
                .enumerate()
                .filter(|(_, value)| matches!(value, Term::Var(_)))
            {
                if !proofs.location(&a.rel, column) || !rule.body.iter().any(|l| compared_to_unset(l, value, "!=")) {
                    continue;
                }
                let candidates = proofs.for_column(&a.rel, column, 8);
                let Some(w) = candidates
                    .iter()
                    .find(|w| a.args.get(w.anchor).is_some_and(|t| plain(t) || *t == Term::Wild))
                else {
                    continue;
                };
                if fresh.is_none() {
                    fresh = Some(
                        rule.head
                            .vars()
                            .into_iter()
                            .chain(rule.body.iter().flat_map(Literal::vars))
                            .collect::<BTreeSet<_>>(),
                    );
                }
                let mut reader = a.clone();
                if reader.args.make_mut()[w.anchor] == Term::Wild {
                    let names = fresh.as_mut().expect("rule variables");
                    let mut n = names.len();
                    let name = loop {
                        let name = format!("rr_present{n}");
                        if names.insert(name.clone()) {
                            break name;
                        }
                        n += 1;
                    };
                    reader.args.make_mut()[w.anchor] = Term::Var(name.into());
                }
                let mut args = [Term::Wild, Term::Wild, Term::Wild];
                args[w.edge_anchor] = reader.args[w.anchor].clone();
                args[w.edge_value] = value.clone();
                edits.push((rule_index, index, reader, atom(w.edge.to_owned(), &args)));
                break;
            }
        }
    }
    let count = edits.len();
    for (rule, index, reader, edge) in edits {
        program.rules[rule].body.make_mut()[index] = Literal::Atom(reader);
        program.rules[rule].body.push(Literal::Atom(edge));
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_rule_dsl::logic::rr::parse;

    #[test]
    fn a_present_location_uses_the_edge_shared_by_its_producers() {
        let mut p = parse(
            r#"
            .decl field(p: node, name: symbol, c: node)
            .decl missing(p: node)
            .input field, missing
            .decl parts(p: node, a: loc)
            parts(p, a) :- field(p, "type", a).
            parts(p, a) :- missing(p), unset(a).
            .decl signature(p: node, a: loc)
            signature(p, a) :- parts(p, a).
            .decl present(a: loc)
            .output present
            present(a) :- signature(_, a), !unset(a).
        "#,
        )
        .unwrap();
        assert_eq!(restore(&mut p), 1);
        let present = p.rules.iter().find(|r| r.head.rel == "present").unwrap().to_string();
        assert!(present.contains("field($rr_present"), "{present}");
        assert!(present.contains("signature($rr_present"), "{present}");
    }

    #[test]
    fn an_alternative_location_without_the_edge_blocks_the_proof() {
        let mut p = parse(
            r#"
            .decl field(p: node, name: symbol, c: node)
            .decl other(p: node, c: node)
            .input field, other
            .decl parts(p: node, a: loc)
            parts(p, a) :- field(p, "type", a).
            parts(p, a) :- other(p, a).
            .decl present(a: loc)
            .output present
            present(a) :- parts(_, a), !unset(a).
        "#,
        )
        .unwrap();
        let before = p.rules.clone();
        assert_eq!(restore(&mut p), 0);
        assert_eq!(p.rules, before);
    }
}
