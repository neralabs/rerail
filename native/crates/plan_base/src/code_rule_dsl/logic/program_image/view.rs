//! The view of a program image as the engine's inputs (decision 0701 §1, §5.1): each `img_` relation of
//! `rr/view.rerail` a source over the image's view (`ImageView`), an analysis of the version: made by one walk of the
//! image, held with it, and read in place by every run over it. Each indexed column's rows are held in its values'
//! order, so a probe takes a slice of them (`Source::sorted_rows`), and each relation states its statistics, so a run
//! counts none. A name is the image's own atom, a rule its id.

use super::Version;
use super::handles::{Lit, RuleRef};
use super::manager::{Analysis, Cx, Reads};
use super::{AtomId, RelId};
use crate::code_rule_dsl::facts::vtab::Source;
use rerail_blocks::vector::Vector;
use std::sync::Arc;

/// The view's relations of the image, in `ImageView::tables`' order: name, columns, the columns a probe reads by.
const RELATIONS: [(&str, usize, &[usize]); 4] = [("img_rule", 2, &[0, 1]), ("img_lit", 5, &[0, 3]), ("img_decl", 2, &[0]), ("img_facts", 1, &[0])];

/// The columns that name a relation, in `RELATIONS`' order: each states its rows a value (`Source::column_counts`), so
/// a probe by it from a few names (a pass's kind sets) is priced by the rows those names hold, not by the column's
/// average. A name's rows are skewed (`node` is read by thousands of literals, a kind set by a few): priced by its
/// posting-weighted fan, 30 kind sets probing python's `img_lit` were 16,700 rows, so the run read all 8,625
/// literals and probed the kind sets with each, for 9 rows (1.5 ms of the step's 4.7).
const NAMES: [&[usize]; 4] = [&[1], &[3], &[0], &[0]];

/// `img_lit`'s column that says whether a literal is an aggregate's atom (1) or an atom (0): the source reads it as
/// the pass's own constant (`"agg"` or `"atom"`).
const WHAT: usize = 2;

/// The image's view (decision 0701 §5.1): `img_rule`, `img_lit`, `img_decl` and `img_facts`, each column held whole.
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
    /// By column: each value's rows, for a column that names a relation (`NAMES`).
    counts: Vec<Option<Vec<(i64, u64)>>>,
}

#[derive(Debug, PartialEq)]
struct Sorted {
    values: Vector<i64>,
    rows: Vector<u32>,
}

impl Table {
    fn of(columns: Vec<Vector<i64>>, indexed: &[usize], names: &[usize]) -> Table {
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
        let (stats, counts) = columns
            .iter()
            .enumerate()
            .map(|(c, col)| {
                let mut counts: rustc_hash::FxHashMap<i64, i64> = Default::default();
                for &x in col.iter() {
                    *counts.entry(x).or_default() += 1;
                }
                let stats = [n as i64, counts.len() as i64, counts.values().map(|k| k * k).sum()];
                (stats, names.contains(&c).then(|| counts.into_iter().map(|(v, k)| (v, k as u64)).collect()))
            })
            .unzip();
        Table { columns, sorted, stats, counts }
    }
}

/// Rule `r`'s `img_lit` rows: each body literal that names a relation, an atom or an aggregate's (`what` 0 or 1), by
/// its place in the body; `name` gives a relation's name.
fn lit_rows<'v>(r: RuleRef<'v>, name: impl Fn(RelId) -> i64 + 'v) -> impl Iterator<Item = [i64; 5]> + 'v {
    let id = i64::from(r.id().0);
    r.body().enumerate().filter_map(move |(i, l)| {
        let (agg, a) = match l.view() {
            Lit::Atom(a) => (0, a),
            Lit::Agg { atom: a, .. } => (1, a),
            _ => return None,
        };
        Some([id, i as i64, agg, name(a.rel()), i64::from(a.neg())])
    })
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
            rule[0].push(i64::from(r.id().0));
            rule[1].push(names[r.head().rel().index()]);
            for row in lit_rows(r, |rel| names[rel.index()]) {
                for (c, x) in row.into_iter().enumerate() {
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
                Table::of(columns, RELATIONS[at - 1].2, NAMES[at - 1])
            }),
        }
    }
}

impl Analysis for ImageView {
    const NAME: &'static str = "the image's view (decision 0701)";
    const READS: Reads = Reads::RULES.or(Reads::DECLS).or(Reads::FACTS);
    fn compute(cx: &Cx<'_>) -> Result<ImageView, String> {
        let _s = crate::code_rule_dsl::logic::profile::span(|| "the image's view made".into());
        Ok(ImageView::of(cx.v()))
    }
}

/// What a run read of a view (decision 0701 §5.1), as a run's frame holds the leaves it read (decision 0659): each
/// probe, a relation's column and a value, with the digest of the rows it found, and whether the run read a row no
/// probe found (a scan). A run over another version whose every probe finds the same rows, and that scans nothing,
/// reads what this run read, so its outputs are this run's: the plan the run took is an evaluation of either.
#[derive(Default)]
pub struct Frame {
    probes: std::sync::Mutex<rustc_hash::FxHashMap<(usize, usize, i64), u64>>,
    scanned: std::sync::atomic::AtomicBool,
}

impl Frame {
    /// Whether a run over `v` reads what the run of this frame read: each probe found again by `v`'s own indexes
    /// (`found`), no view of `v` made.
    pub fn holds(&self, v: &Version) -> bool {
        if self.scanned.load(std::sync::atomic::Ordering::Relaxed) {
            return false;
        }
        let probes = self.probes.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        probes.iter().all(|(&(at, column, value), &digest)| found(v, at, column, value).is_some_and(|rows| digest_of(rows.iter().map(Vec::as_slice)) == digest))
    }
    /// Why it does not hold over `v` (`holds`): that its run scanned a relation, or its first probes (up to `n`) that
    /// find other rows in `v`, each with its rows there.
    pub fn misses(&self, v: &Version, n: usize) -> Vec<String> {
        if self.scanned.load(std::sync::atomic::Ordering::Relaxed) {
            return vec!["its run read a row no probe found".to_owned()];
        }
        let probes = self.probes.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut out: Vec<String> = probes
            .iter()
            .filter_map(|(&(at, column, value), &digest)| {
                let rows = found(v, at, column, value);
                (rows.as_ref().map(|r| digest_of(r.iter().map(Vec::as_slice))) != Some(digest)).then(|| format!("{}({column} = {value}): {rows:?}", RELATIONS[at].0))
            })
            .collect();
        out.sort();
        out.truncate(n);
        out
    }
}

/// The digest of a relation's rows, whatever their order: the sum of each row's hash.
fn digest_of<'r>(rows: impl Iterator<Item = &'r [i64]>) -> u64 {
    use std::hash::{Hash, Hasher};
    rows.fold(0u64, |d, row| {
        let mut h = rustc_hash::FxHasher::default();
        row.hash(&mut h);
        d.wrapping_add(h.finish())
    })
}

/// The rows of `v`'s view relation `at` whose `column` is `value`, as `ImageView::of` makes them (`img_lit`'s `what`
/// stored), found by `v`'s own indexes: a rule by its id, the rules of a head, the readers of a relation, a
/// declaration, a relation's facts. `None` for a column the view is not probed by.
fn found(v: &Version, at: usize, column: usize, value: i64) -> Option<Vec<Vec<i64>>> {
    let name = |rel: RelId| i64::from(v.rel_atom(rel).0);
    let atom = u32::try_from(value).ok().map(AtomId);
    let rule = u32::try_from(value).ok().and_then(|k| v.rule(super::RuleId(k)));
    Some(match (at, column) {
        (0, 0) => rule.map(|r| vec![value, name(r.head().rel())]).into_iter().collect(),
        (0, 1) => v.rel_of(atom?).map(|rel| v.heads(rel).map(|r| vec![i64::from(r.id().0), value]).collect()).unwrap_or_default(),
        (1, 0) => rule.map(|r| lit_rows(r, name).map(Vec::from).collect()).unwrap_or_default(),
        (1, 3) => {
            let Some(rel) = v.rel_of(atom?) else { return Some(Vec::new()) };
            let mut readers: Vec<RuleRef<'_>> = v.readers(rel).map(|(r, _)| r).collect();
            readers.sort_by_key(|r| r.id());
            readers.dedup_by_key(|r| r.id());
            readers.into_iter().flat_map(|r| lit_rows(r, name).filter(|row| row[3] == value).map(Vec::from).collect::<Vec<_>>()).collect()
        }
        (2, 0) => v.decl(atom?).map(|d| vec![value, d.arity() as i64]).into_iter().collect(),
        (3, 0) => v.facts(atom?).map(|_| vec![value]).into_iter().collect(),
        _ => return None,
    })
}

/// One `img_` relation read off the view in place, `img_lit`'s `what` as the pass's constants; with a frame, what a
/// run reads of it recorded there (`framed_sources`), each row a probe found marked `handed`.
struct ViewRel {
    view: Arc<ImageView>,
    at: usize,
    name: &'static str,
    what: [i64; 2],
    frame: Option<(Arc<Frame>, std::sync::Mutex<Vec<bool>>)>,
}

impl ViewRel {
    fn table(&self) -> &Table {
        &self.view.tables[self.at]
    }
    fn read(&self, row: usize, column: usize) -> i64 {
        if let Some((frame, handed)) = &self.frame
            && !handed.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get(row).copied().unwrap_or(false)
        {
            frame.scanned.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        let x = self.table().columns[column][row];
        if self.at == 1 && column == WHAT { self.what[x as usize] } else { x }
    }
    /// A probe's rows, recorded in the frame.
    fn probed(&self, column: usize, value: i64, rows: &[u32]) {
        let Some((frame, handed)) = &self.frame else { return };
        let t = self.table();
        let stored: Vec<Vec<i64>> = rows.iter().map(|&r| t.columns.iter().map(|c| c[r as usize]).collect()).collect();
        frame.probes.lock().unwrap_or_else(std::sync::PoisonError::into_inner).insert((self.at, column, value), digest_of(stored.iter().map(Vec::as_slice)));
        let mut handed = handed.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        for &r in rows {
            handed[r as usize] = true;
        }
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
        self.probed(column, value, &s.rows[lo..hi]);
        Some(&s.rows[lo..hi])
    }
    fn sorts_rows(&self, column: usize) -> bool {
        self.indexed(column)
    }
    fn statistics(&self) -> Option<Vec<(String, i64, i64, i64, i64)>> {
        Some(self.columns().into_iter().zip(&self.table().stats).map(|(name, &[rows, ndv, sq])| (name, rows, ndv, sq, sq)).collect())
    }
    fn column_counts(&self, c: usize) -> Option<Vec<(i64, u64)>> {
        self.table().counts.get(c)?.clone()
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
    fn column_counts(&self, _c: usize) -> Option<Vec<(i64, u64)>> {
        Some(self.sets.iter().map(|&s| (s, 1)).collect())
    }
    fn value(&self, row: usize, _column: usize) -> i64 {
        self.sets[row]
    }
}

/// The view of `v` (decision 0701 §1), by relation name: `img_rule`, `img_lit`, `img_decl`, `img_facts` (its view,
/// `ImageView`, made once for the version), and `img_kindset` of `kindsets` (the pass's kind sets, which are not yet
/// the image's). `what` is the pass's program's constants `"atom"` and `"agg"`, which `img_lit` states each literal as.
pub fn sources(v: &Arc<Version>, kindsets: &[AtomId], what: [i64; 2]) -> Result<Vec<(&'static str, Arc<dyn Source>)>, String> {
    made_sources(v, kindsets, what, None)
}

/// `sources`, what a run reads of them recorded in a frame (`Frame`).
pub fn framed_sources(v: &Arc<Version>, kindsets: &[AtomId], what: [i64; 2]) -> Result<(Vec<(&'static str, Arc<dyn Source>)>, Arc<Frame>), String> {
    let frame = Arc::new(Frame::default());
    Ok((made_sources(v, kindsets, what, Some(&frame))?, frame))
}

fn made_sources(v: &Arc<Version>, kindsets: &[AtomId], what: [i64; 2], frame: Option<&Arc<Frame>>) -> Result<Vec<(&'static str, Arc<dyn Source>)>, String> {
    let view = super::manager::analysis::<ImageView>(v)?;
    let mut out: Vec<(&'static str, Arc<dyn Source>)> = RELATIONS
        .iter()
        .enumerate()
        .map(|(at, &(name, _, _))| {
            let frame = frame.map(|f| (Arc::clone(f), std::sync::Mutex::new(vec![false; view.tables[at].columns.first().map_or(0, Vector::len)])));
            (name, Arc::new(ViewRel { view: Arc::clone(&view), at, name, what, frame }) as Arc<dyn Source>)
        })
        .collect();
    // The kind sets are given, not the image's: a run over another version is given the same ones (`Kept`'s key).
    out.push(("img_kindset", Arc::new(Given { sets: kindsets.iter().map(|a| i64::from(a.0)).collect() })));
    Ok(out)
}

/// The runs of rules passes over a library's union, kept with it (decision 0701 §5.1): each by its key (the pass's
/// program, the kind sets it was given, its outputs), with its frame and its outputs' rows. A program linked on the
/// union (`Version::extends`) whose version the frame holds over has those outputs, and runs nothing.
#[derive(Default)]
pub struct Kept {
    runs: std::sync::Mutex<Vec<(u64, Arc<Frame>, Arc<Vec<Vec<Vec<i64>>>>)>>,
}

impl Kept {
    /// The run kept under `key`.
    pub fn get(&self, key: u64) -> Option<(Arc<Frame>, Arc<Vec<Vec<Vec<i64>>>>)> {
        self.runs.lock().unwrap_or_else(std::sync::PoisonError::into_inner).iter().find(|(k, _, _)| *k == key).map(|(_, f, o)| (Arc::clone(f), Arc::clone(o)))
    }
    /// Whether it keeps no run: no rules pass ran over the union.
    pub fn is_empty(&self) -> bool {
        self.runs.lock().unwrap_or_else(std::sync::PoisonError::into_inner).is_empty()
    }
    /// Whether a run it keeps answers `v` (its frame holds over `v`).
    pub fn answers(&self, v: &Version) -> bool {
        self.runs.lock().unwrap_or_else(std::sync::PoisonError::into_inner).iter().any(|(_, f, _)| f.holds(v))
    }
    /// Why each run it keeps does not answer `v` (`Frame::misses`), up to `n` reasons a run.
    pub fn misses(&self, v: &Version, n: usize) -> Vec<String> {
        self.runs.lock().unwrap_or_else(std::sync::PoisonError::into_inner).iter().flat_map(|(_, f, _)| f.misses(v, n)).collect()
    }
    /// Keeps a run under `key`.
    pub fn keep(&self, key: u64, frame: Arc<Frame>, outputs: Arc<Vec<Vec<Vec<i64>>>>) {
        let mut runs = self.runs.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if !runs.iter().any(|(k, _, _)| *k == key) {
            runs.push((key, frame, outputs));
        }
    }
}

impl Kept {
    fn keys(&self) -> Vec<u64> {
        self.runs.lock().unwrap_or_else(std::sync::PoisonError::into_inner).iter().map(|(k, _, _)| *k).collect()
    }
}

impl std::fmt::Debug for Kept {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Kept {{ runs: {:?} }}", self.keys())
    }
}

/// Two hold the same runs when they hold runs of the same keys.
impl PartialEq for Kept {
    fn eq(&self, other: &Kept) -> bool {
        self.keys() == other.keys()
    }
}

impl Analysis for Kept {
    const NAME: &'static str = "the rules passes' runs kept with a library (decision 0701)";
    const READS: Reads = Reads::RULES.or(Reads::DECLS).or(Reads::FACTS);
    fn compute(_cx: &Cx<'_>) -> Result<Kept, String> {
        Ok(Kept::default())
    }
}
