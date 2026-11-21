//! Recover a tape edge already witnessed by a present location. The edge gives the split a document for the
//! location, even when the relation it came from can also return the unset value.

use super::{Program, plain, rewritten, same};
use crate::code_rule_dsl::logic::program_image::{CmpOp, Lit, LitRef, RuleId, RuleRef, T, TermRef, t_of};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Witness {
    anchor: usize,
    edge: &'static str,
    edge_anchor: usize,
    edge_value: usize,
}

type Key = (String, usize);
type Witnesses = BTreeSet<Witness>;

struct Proofs<'p, 'i> {
    program: &'p Program<'i>,
    producers: BTreeMap<&'p str, Vec<RuleRef<'p>>>,
    known: BTreeMap<Key, Witnesses>,
}

fn compared_to_unset(literal: LitRef<'_>, term: TermRef<'_>, comparison: CmpOp) -> bool {
    matches!(
        literal.view(),
        Lit::Cmp { l, op, r }
            if op == comparison
                && ((same(l, term) && matches!(r, TermRef::Int(-1))) || (same(r, term) && matches!(l, TermRef::Int(-1))))
    )
}

impl<'p, 'i> Proofs<'p, 'i> {
    fn location(&self, rel: &str, column: usize) -> bool {
        self.program.columns(rel).and_then(|cols| cols.get(column).cloned()).is_some_and(|(_, ty)| matches!(ty.as_str(), "loc" | "node"))
    }

    fn for_column(&mut self, rel: &str, column: usize, fuel: usize) -> Witnesses {
        let key = (rel.to_owned(), column);
        if let Some(found) = self.known.get(&key) {
            return found.clone();
        }
        let f = &self.program.f;
        if fuel == 0
            || !self.location(rel, column)
            || f.facts.contains(rel)
            || f.inputs.iter().any(|input| input == rel)
            || f.natives.contains_key(rel)
            || f.answers.values().flatten().any(|answer| answer == rel)
        {
            return Witnesses::new();
        }
        let Some(rules) = self.producers.get(rel).cloned() else {
            return Witnesses::new();
        };
        let v = self.program.v();
        self.known.insert(key.clone(), Witnesses::new());
        let mut common: Option<Witnesses> = None;
        for rule in rules {
            let head: Vec<TermRef<'_>> = rule.head().args().collect();
            let Some(&value) = head.get(column) else {
                return Witnesses::new();
            };
            if matches!(value, TermRef::Int(-1)) || rule.body().any(|l| compared_to_unset(l, value, CmpOp::Eq)) {
                continue;
            }
            let mut here = Witnesses::new();
            if matches!(value, TermRef::Var(_)) {
                for body in rule.body().filter_map(|lit| match lit.view() {
                    Lit::Atom(a) if !a.neg() && !a.gated() => Some(a),
                    _ => None,
                }) {
                    let args: Vec<TermRef<'_>> = body.args().collect();
                    let rel = v.rel_name(body.rel());
                    let mut witnesses = Witnesses::new();
                    for (i, _) in args.iter().enumerate().filter(|(_, t)| same(**t, value)) {
                        let edge = match (rel, i, args.len()) {
                            ("field", 0, 3) => Some(("field", 2)),
                            ("field", 2, 3) => Some(("field", 0)),
                            ("child", 0, 3) => Some(("child", 1)),
                            ("child", 1, 3) => Some(("child", 0)),
                            _ => None,
                        };
                        if let Some((edge, anchor)) = edge {
                            witnesses.insert(Witness { anchor, edge, edge_anchor: anchor, edge_value: i });
                        } else {
                            witnesses.extend(self.for_column(rel, i, fuel - 1));
                        }
                    }
                    for witness in witnesses {
                        let Some(&anchor) = args.get(witness.anchor) else {
                            continue;
                        };
                        if !matches!(anchor, TermRef::Var(_)) || same(anchor, value) {
                            continue;
                        }
                        for (i, _) in head.iter().enumerate().filter(|(_, t)| same(**t, anchor)) {
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

/// An edit `restore` makes in a rule: its atom at literal `index` reads its anchor column `anchor` as variable
/// `fresh` where it was `_`, and the rule reads `edge(args)`, `edge_anchor` its anchor and `edge_value` the location
/// (literal `index`'s column `column`).
struct Edit {
    index: usize,
    anchor: usize,
    fresh: Option<String>,
    witness: Witness,
    column: usize,
}

/// The arguments of edit `e`'s atom in rule `r` (`body`: its literals), its anchor a variable.
fn read<'a>(r: RuleRef<'a>, body: &[LitRef<'a>], e: &'a Edit) -> Vec<T<'a>> {
    let a = body[e.index].atom().expect("an atom");
    a.args()
        .enumerate()
        .map(|(k, t)| match (&e.fresh, k == e.anchor) {
            (Some(name), true) => T::Var(name.as_str()),
            _ => t_of(r, t),
        })
        .collect()
}

/// A reader rejects unset explicitly. Probe the same tape edge its surviving rows must have read.
pub(super) fn restore(program: &mut Program<'_>) -> Result<usize, String> {
    let mut edits: BTreeMap<RuleId, Vec<Edit>> = BTreeMap::new();
    {
        let v = program.v();
        let mut proofs = Proofs { program, producers: BTreeMap::new(), known: BTreeMap::new() };
        for rule in v.rules() {
            proofs.producers.entry(v.rel_name(rule.head().rel())).or_default().push(rule);
        }
        for rule in v.rules() {
            let mut fresh: Option<BTreeSet<String>> = None;
            for (index, lit) in rule.body().enumerate() {
                let Lit::Atom(a) = lit.view() else { continue };
                if a.neg() || a.gated() {
                    continue;
                }
                let rel = v.rel_name(a.rel());
                let args: Vec<TermRef<'_>> = a.args().collect();
                for (column, &value) in args.iter().enumerate().filter(|(_, value)| matches!(value, TermRef::Var(_))) {
                    if !proofs.location(rel, column) || !rule.body().any(|l| compared_to_unset(l, value, CmpOp::Ne)) {
                        continue;
                    }
                    let candidates = proofs.for_column(rel, column, 8);
                    let Some(w) = candidates.iter().find(|w| args.get(w.anchor).is_some_and(|t| plain(*t) || matches!(t, TermRef::Wild))) else {
                        continue;
                    };
                    let names = fresh.get_or_insert_with(|| rule.vars().map(|x| v.text(x).to_owned()).collect());
                    let name = matches!(args[w.anchor], TermRef::Wild).then(|| {
                        let mut n = names.len();
                        loop {
                            let name = format!("rr_present{n}");
                            if names.insert(name.clone()) {
                                break name;
                            }
                            n += 1;
                        }
                    });
                    edits.entry(rule.id()).or_default().push(Edit { index, anchor: w.anchor, fresh: name, witness: *w, column });
                    break;
                }
            }
        }
    }
    let count = edits.values().map(Vec::len).sum();
    if count == 0 {
        return Ok(0);
    }
    let made = program.derive(
        &mut |first, v, r, b| {
            let Some(here) = edits.get(&r.id()) else { return Ok(false) };
            let body: Vec<LitRef<'_>> = r.body().collect();
            let mut rb = rewritten(first, v, r, b);
            for (i, l) in body.iter().enumerate() {
                rb = match here.iter().find(|e| e.index == i) {
                    Some(e) => rb.atom(v.rel_name(l.atom().expect("an atom").rel()), &read(r, &body, e)),
                    None => rb.lit(v, r, *l),
                };
            }
            for e in here {
                let args = read(r, &body, e);
                let a = body[e.index].atom().expect("an atom");
                let mut edge = [T::Wild, T::Wild, T::Wild];
                edge[e.witness.edge_anchor] = args[e.anchor].clone();
                edge[e.witness.edge_value] = t_of(r, a.args().nth(e.column).expect("the location"));
                rb = rb.atom(e.witness.edge, &edge);
            }
            rb.finish().map(|_| true)
        },
        &mut |_, _| Ok(()),
    )?;
    program.set(made);
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::tests::parse;

    #[test]
    fn a_present_location_uses_the_edge_shared_by_its_producers() {
        let v = parse(
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
        let mut p = Program::of(&v);
        assert_eq!(restore(&mut p).unwrap(), 1);
        let present = p.shown(Some("present")).remove(0);
        assert!(present.contains("field($rr_present"), "{present}");
        assert!(present.contains("signature($rr_present"), "{present}");
    }

    #[test]
    fn an_alternative_location_without_the_edge_blocks_the_proof() {
        let v = parse(
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
        let mut p = Program::of(&v);
        assert_eq!(restore(&mut p).unwrap(), 0);
        assert!(!p.changed());
    }
}
