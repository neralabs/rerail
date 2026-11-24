//! The view of a program image as the engine's inputs (decision 0662 §1, §5.1): each `img_` relation of
//! `rr/view.rerail` a source over the image's view (`ImageView`), an analysis of the version: made by one walk of the
//! image, held with it, and read in place by every run over it. Each indexed column's rows are held in its values'
//! order, so a probe takes a slice of them (`Source::sorted_rows`), and each relation states its statistics, so a run
//! counts none. A name is the image's own atom, a rule its id.

use super::Version;
use super::handles::Lit;
use super::manager::{Analysis, Cx, Reads};
use super::{AtomId, RelId};
use crate::code_rule_dsl::facts::vtab::Source;
use rerail_blocks::vector::Vector;
use std::sync::Arc;

/// The view's relations of the image, in `ImageView::tables`' order: name, columns, the columns a probe reads by.
const RELATIONS: [(&str, usize, &[usize]); 4] = [("img_rule", 2, &[0, 1]), ("img_lit", 5, &[0, 3]), ("img_decl", 2, &[0]), ("img_facts", 1, &[0])];

/// `img_lit`'s column that says whether a literal is an aggregate's atom (1) or an atom (0): the source reads it as
/// the pass's own constant (`"agg"` or `"atom"`).
const WHAT: usize = 2;

/// The image's view (decision 0662 §5.1): `img_rule`, `img_lit`, `img_decl` and `img_facts`, each column held whole.
#[derive(Debug, PartialEq)]
pub struct ImageView {
    tables: [Table; 4],
}

/// One relation of the view: its columns, the rows of each indexed column in its values' order, and its statistics.
#[derive(Debug, PartialEq)]
struct Table {
    columns: Vec<Vector<i64>>,
    /// By column: its values in order and their rows (`Sorted`), for a column a probe reads by.
    sorted: Vec<Option<Sorted>>,
    /// By column: rows, distinct values, and the sum of each value's rows squared (`syntax::statistics`' `sq`; one
    /// document, so `sqd` is the same).
    stats: Vec<[i64; 3]>,
}

#[derive(Debug, PartialEq)]
struct Sorted {
    values: Vector<i64>,
    rows: Vector<u32>,
}

impl Table {
    fn of(columns: Vec<Vector<i64>>, indexed: &[usize]) -> Table {
        let n = columns.first().map_or(0, Vector::len);
        let sorted = (0..columns.len())
            .map(|c| {
                indexed.contains(&c).then(|| {
                    let mut rows: Vector<u32> = (0..n as u32).collect();
                    rows.sort_by_key(|&r| columns[c][r as usize]);
                    Sorted { values: rows.iter().map(|&r| columns[c][r as usize]).collect(), rows }
                })
            })
            .collect();
        let stats = columns
            .iter()
            .map(|col| {
                let mut counts: rustc_hash::FxHashMap<i64, i64> = Default::default();
                for &x in col.iter() {
                    *counts.entry(x).or_default() += 1;
                }
                [n as i64, counts.len() as i64, counts.values().map(|k| k * k).sum()]
            })
            .collect();
        Table { columns, sorted, stats }
    }
}

impl ImageView {
    /// `v`'s view, by one walk of its rules, declarations and facts.
    pub fn of(v: &Version) -> ImageView {
        let places = v.rule_places();
        let names: Vec<i64> = (0..v.rel_end()).map(|r| i64::from(v.rel_atom(RelId(r)).0)).collect();
        let mut rule: [Vector<i64>; 2] = Default::default();
        let mut lit: [Vector<i64>; 5] = Default::default();
        for p in 0..places.len() {
            let r = places.get(p);
            let id = i64::from(r.id().0);
            rule[0].push(id);
            rule[1].push(names[r.head().rel().index()]);
            for (i, l) in r.body().enumerate() {
                let (agg, a) = match l.view() {
                    Lit::Atom(a) => (0, a),
                    Lit::Agg { atom: a, .. } => (1, a),
                    _ => continue,
                };
                for (c, x) in [id, i as i64, agg, names[a.rel().index()], i64::from(a.neg())].into_iter().enumerate() {
                    lit[c].push(x);
                }
            }
        }
        let (mut decl, mut facts): ([Vector<i64>; 2], Vector<i64>) = Default::default();
        for d in v.decls() {
            decl[0].push(i64::from(d.name().0));
            decl[1].push(d.arity() as i64);
        }
        facts.extend(v.all_facts().into_iter().map(|f| i64::from(f.name().0)));
        let made: [Vec<Vector<i64>>; 4] = [Vec::from(rule), Vec::from(lit), Vec::from(decl), vec![facts]];
        let mut at = 0;
        ImageView {
            tables: made.map(|columns| {
                at += 1;
                Table::of(columns, RELATIONS[at - 1].2)
            }),
        }
    }
}

impl Analysis for ImageView {
    const NAME: &'static str = "the image's view (decision 0662)";
    const READS: Reads = Reads::RULES.or(Reads::DECLS).or(Reads::FACTS);
    fn compute(cx: &Cx<'_>) -> Result<ImageView, String> {
        let _s = crate::code_rule_dsl::logic::profile::span(|| "the image's view made".into());
        Ok(ImageView::of(cx.v()))
    }
}

/// One `img_` relation read off the view in place, `img_lit`'s `what` as the pass's constants.
struct ViewRel {
    view: Arc<ImageView>,
    at: usize,
    name: &'static str,
    what: [i64; 2],
}

impl ViewRel {
    fn table(&self) -> &Table {
        &self.view.tables[self.at]
    }
    fn read(&self, row: usize, column: usize) -> i64 {
        let x = self.table().columns[column][row];
        if self.at == 1 && column == WHAT { self.what[x as usize] } else { x }
    }
}

impl Source for ViewRel {
    fn columns(&self) -> Vec<String> {
        (0..self.table().columns.len()).map(|c| crate::code_rule_dsl::logic::sql::select::sql_col(self.name, c)).collect()
    }
    fn len(&self) -> usize {
        self.table().columns.first().map_or(0, Vector::len)
    }
    fn estimated_matches(&self, column: usize, _batch: bool) -> usize {
        let [rows, ndv, _] = self.table().stats[column];
        (rows / ndv.max(1)).max(1) as usize
    }
    fn indexed(&self, column: usize) -> bool {
        self.table().sorted[column].is_some()
    }
    fn lookup(&self, column: usize, values: &[i64]) -> Option<Vector<usize>> {
        Some(values.iter().flat_map(|&v| self.sorted_rows(column, v).into_iter().flatten().map(|&r| r as usize)).collect())
    }
    fn sorted_rows(&self, column: usize, value: i64) -> Option<&[u32]> {
        let s = self.table().sorted.get(column)?.as_ref()?;
        let (lo, hi) = (s.values.partition_point(|&x| x < value), s.values.partition_point(|&x| x <= value));
        Some(&s.rows[lo..hi])
    }
    fn sorts_rows(&self, column: usize) -> bool {
        self.indexed(column)
    }
    fn statistics(&self) -> Option<Vec<(String, i64, i64, i64, i64)>> {
        Some(self.columns().into_iter().zip(&self.table().stats).map(|(name, &[rows, ndv, sq])| (name, rows, ndv, sq, sq)).collect())
    }
    fn value(&self, row: usize, column: usize) -> i64 {
        self.read(row, column)
    }
    fn row_at(&self, row: usize, columns: &[usize], out: &mut [i64]) {
        for (o, &c) in out.iter_mut().zip(columns) {
            *o = self.read(row, c);
        }
    }
}

/// `img_kindset`: the pass's kind sets, which are not yet the image's.
struct Given {
    sets: Vector<i64>,
}

impl Source for Given {
    fn columns(&self) -> Vec<String> {
        vec![crate::code_rule_dsl::logic::sql::select::sql_col("img_kindset", 0)]
    }
    fn len(&self) -> usize {
        self.sets.len()
    }
    fn estimated_matches(&self, _column: usize, _batch: bool) -> usize {
        1
    }
    fn indexed(&self, _column: usize) -> bool {
        false
    }
    fn lookup(&self, _column: usize, _values: &[i64]) -> Option<Vector<usize>> {
        None
    }
    fn statistics(&self) -> Option<Vec<(String, i64, i64, i64, i64)>> {
        let n = self.sets.len() as i64;
        Some(vec![(self.columns().remove(0), n, n, n, n)])
    }
    fn value(&self, row: usize, _column: usize) -> i64 {
        self.sets[row]
    }
}

/// The view of `v` (decision 0662 §1), by relation name: `img_rule`, `img_lit`, `img_decl`, `img_facts` (its view,
/// `ImageView`, made once for the version), and `img_kindset` of `kindsets` (the pass's kind sets, which are not yet
/// the image's). `what` is the pass's program's constants `"atom"` and `"agg"`, which `img_lit` states each literal as.
pub fn sources(v: &Arc<Version>, kindsets: &[AtomId], what: [i64; 2]) -> Result<Vec<(&'static str, Arc<dyn Source>)>, String> {
    let view = super::manager::analysis::<ImageView>(v)?;
    let mut out: Vec<(&'static str, Arc<dyn Source>)> = RELATIONS.iter().enumerate().map(|(at, &(name, _, _))| (name, Arc::new(ViewRel { view: Arc::clone(&view), at, name, what }) as Arc<dyn Source>)).collect();
    out.push(("img_kindset", Arc::new(Given { sets: kindsets.iter().map(|a| i64::from(a.0)).collect() })));
    Ok(out)
}
