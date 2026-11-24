//! The view of a program image as the engine's inputs (decision 0662 §1, §5.1): each `img_` relation of
//! `rr/view.rerail` a source over the `Version`, read in place. A row is an id (a rule's place, a literal's number in
//! it, an atom); each row's values are read off the image when the engine asks for them. A name is the image's own
//! atom, a rule its id.

use super::Version;
use super::handles::Lit;
use super::{AtomId, RelId};
use crate::code_rule_dsl::facts::vtab::Source;
use std::sync::Arc;

/// The widest `img_` relation's columns.
const WIDEST: usize = 5;

/// One `img_` relation: its rows' ids, and how a row's values are read off the image (all of them at once: a
/// literal's are found by one walk of its rule's body).
struct ViewRel<R> {
    columns: Vec<String>,
    rows: Vec<R>,
    read: Box<dyn Fn(&R, &mut [i64; WIDEST]) + Send + Sync>,
}

impl<R: Send + Sync> Source for ViewRel<R> {
    fn columns(&self) -> Vec<String> {
        self.columns.clone()
    }
    fn len(&self) -> usize {
        self.rows.len()
    }
    fn estimated_matches(&self, _column: usize, _batch: bool) -> usize {
        self.rows.len()
    }
    fn indexed(&self, _column: usize) -> bool {
        false
    }
    fn lookup(&self, _column: usize, _values: &[i64]) -> Option<rerail_blocks::vector::Vector<usize>> {
        None
    }
    fn value(&self, row: usize, column: usize) -> i64 {
        let mut values = [0; WIDEST];
        (self.read)(&self.rows[row], &mut values);
        values[column]
    }
    fn row_at(&self, row: usize, columns: &[usize], out: &mut [i64]) {
        let mut values = [0; WIDEST];
        (self.read)(&self.rows[row], &mut values);
        for (o, &c) in out.iter_mut().zip(columns) {
            *o = values[c];
        }
    }
}

fn rel<R: Send + Sync + 'static>(name: &str, width: usize, rows: Vec<R>, read: impl Fn(&R, &mut [i64; WIDEST]) + Send + Sync + 'static) -> Arc<dyn Source> {
    let columns = (0..width).map(|c| crate::code_rule_dsl::logic::sql::select::sql_col(name, c)).collect();
    Arc::new(ViewRel { columns, rows, read: Box::new(read) })
}

/// The view of `v` (decision 0662 §1), by relation name: `img_rule`, `img_lit`, `img_decl`, `img_facts`, and
/// `img_kindset` of `kindsets` (the pass's kind sets, which are not yet the image's). `what` is the pass's program's
/// constants `"atom"` and `"agg"`, which `img_lit` states each literal as. The rules are read where they lie
/// (`Version::rule_places`), and each relation's name through one table of the image's relations' atoms.
pub fn sources(v: &Arc<Version>, kindsets: &[AtomId], what: [i64; 2]) -> Vec<(&'static str, Arc<dyn Source>)> {
    let places = Arc::new(v.rule_places());
    let names: Arc<Vec<u32>> = Arc::new((0..v.rel_end()).map(|r| v.rel_atom(RelId(r)).0).collect());
    let name = |names: &[u32], r: RelId| i64::from(names[r.index()]);
    // Each literal that names a relation, by its rule's place and its number in the body.
    let lits: Vec<(u32, u32)> = (0..places.len())
        .flat_map(|p| places.get(p).body().enumerate().filter(|(_, l)| matches!(l.view(), Lit::Atom(_) | Lit::Agg { .. })).map(move |(i, _)| (p as u32, i as u32)))
        .collect();
    let decls: Vec<(AtomId, usize)> = v.decls().into_iter().map(|d| (d.name(), d.arity())).collect();
    let facts: Vec<AtomId> = v.all_facts().into_iter().map(|f| f.name()).collect();
    let (pr, nr) = (Arc::clone(&places), Arc::clone(&names));
    let rules: Vec<u32> = (0..places.len() as u32).collect();
    vec![
        (
            "img_rule",
            rel("img_rule", 2, rules, move |&p, out| {
                let r = pr.get(p as usize);
                out[0] = i64::from(r.id().0);
                out[1] = name(&nr, r.head().rel());
            }),
        ),
        (
            "img_lit",
            rel("img_lit", 5, lits, move |&(p, i), out| {
                let r = places.get(p as usize);
                let (agg, a) = match r.body().nth(i as usize).map(|l| l.view()) {
                    Some(Lit::Atom(a)) => (false, a),
                    Some(Lit::Agg { atom: a, .. }) => (true, a),
                    _ => unreachable!("a literal that names a relation"),
                };
                *out = [i64::from(r.id().0), i64::from(i), what[usize::from(agg)], name(&names, a.rel()), i64::from(a.neg())];
            }),
        ),
        ("img_decl", rel("img_decl", 2, decls, move |&(a, n), out| (out[0], out[1]) = (i64::from(a.0), n as i64))),
        ("img_facts", rel("img_facts", 1, facts, move |&a, out| out[0] = i64::from(a.0))),
        ("img_kindset", rel("img_kindset", 1, kindsets.to_vec(), move |&a, out| out[0] = i64::from(a.0))),
    ]
}
