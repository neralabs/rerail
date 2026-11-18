//! Plans made with the program from its relations' layouts, kept in its image (decision 0361 step 5).
//!
//! A rule whose every literal is an atom or a comparison, over relations whose facts give their sizes and the
//! distinct values of the columns it reads, is planned once a version: by the planner's own search, over a model of
//! the facts alone at a nominal scale (nothing measured, no store read). Its plans are kept by line (`line_key`: the
//! rule's text and its delta), with their work and rows per node, and a run takes them as they are: no search, no
//! pricing. Every other rule is planned at run time, as before (the exceptions).
use super::shapes::{Counts, Shapes};
use super::{Model, Step, Stats};
use crate::code_rule_dsl::logic::front::{Fan, LayoutFact, LayoutSize, Value};
use crate::code_rule_dsl::logic::program_image::Lit;
use crate::code_rule_dsl::logic::program_image::analyses::Strata;
use crate::code_rule_dsl::logic::program_image::manager::{Analysis, Cx, Reads, Row};
use crate::code_rule_dsl::logic::program_image::{TermRef, Version};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// The nominal scale a prepared plan is made at: its nodes, documents and kinds. A plan's work is kept per node, and a
/// run scales it to its own nodes.
pub const NOMINAL_NODES: f64 = 1_048_576.0;
pub const NOMINAL_DOCS: f64 = 300.0;
pub const NOMINAL_KINDS: f64 = 200.0;

/// One line's plan: its order, and its work and rows per node at the nominal scale.
#[derive(Clone, Debug, PartialEq)]
pub struct PreparedPlan {
    pub order: Vec<Step>,
    pub work_per_node: f64,
    pub rows_per_node: f64,
}

/// The version's prepared plans, by line (`super::line_key`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Prepared(pub HashMap<u128, PreparedPlan>);

/// Lines a run took from the prepared plans (`RERAIL_PLAN_CACHE_COUNTS`).
pub static TAKEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Versions whose prepared plans a run read from the store, and made (Q183; `RERAIL_PLAN_CACHE_COUNTS`).
pub static VERSIONS: [std::sync::atomic::AtomicU64; 2] = [const { std::sync::atomic::AtomicU64::new(0) }; 2];

impl Prepared {
    /// The version's prepared plans: its analysis, shared; none under `RERAIL_PREPARED_PLANS=off` or
    /// `RERAIL_LAYOUT_ESTIMATES=off` (the facts are then not read).
    pub fn of_version(v: &Arc<Version>) -> Arc<Prepared> {
        let off = |k: &str| std::env::var(k).is_ok_and(|x| x == "off");
        if off("RERAIL_PREPARED_PLANS") || off("RERAIL_LAYOUT_ESTIMATES") {
            return Default::default();
        }
        use crate::code_rule_dsl::logic::program_image::manager::{self, VersionAnalyses as _};
        // Held: this version's, as made or read.
        if v.analyses().holds::<Prepared>() {
            return manager::analysis::<Prepared>(v).unwrap_or_default();
        }
        // The run's kept plans' entry holds the plans an earlier process made of a version the same as this one (Q183):
        // read, not made.
        let Some(id) = super::run_holds_plans().then(|| manager::identity(v)) else { return manager::analysis::<Prepared>(v).unwrap_or_default() };
        if let Some(p) = super::prepared_stored(&id) {
            VERSIONS[0].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return v.analyses().hold(p);
        }
        let p = manager::analysis::<Prepared>(v).unwrap_or_default();
        VERSIONS[1].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        super::prepared_made(id, Arc::clone(&p));
        p
    }

    /// The plans as rows, each a line's (`Analysis::rows`).
    pub fn to_rows(&self) -> Vec<Row> {
        let mut rows: Vec<Row> = self
            .0
            .iter()
            .map(|(line, p)| {
                let mut r = vec![Value::Int((line >> 64) as i64), Value::Int(*line as i64), Value::Int(p.work_per_node.to_bits() as i64), Value::Int(p.rows_per_node.to_bits() as i64)];
                r.extend(p.order.iter().map(|s| Value::Int(((s.at as i64) << 2) | mode_code(s.mode))));
                r
            })
            .collect();
        rows.sort_by_key(|r| match (&r[0], &r[1]) {
            (Value::Int(a), Value::Int(b)) => (*a, *b),
            _ => (0, 0),
        });
        rows
    }

    /// The plans read back from `rows` (`to_rows`'s).
    pub fn read_rows<'r>(rows: impl IntoIterator<Item = &'r [Value]>) -> Option<Prepared> {
        let int = |x: &Value| match x {
            Value::Int(i) => Some(*i),
            _ => None,
        };
        let mut out = HashMap::new();
        for r in rows {
            let ints: Vec<i64> = r.iter().map(int).collect::<Option<_>>()?;
            let [hi, lo, work, rows, steps @ ..] = ints.as_slice() else { return None };
            let line = (u128::from(*hi as u64) << 64) | u128::from(*lo as u64);
            let order = steps.iter().map(|&s| Some(Step { at: usize::try_from(s >> 2).ok()?, mode: mode_of(s & 3) })).collect::<Option<Vec<_>>>()?;
            out.insert(line, PreparedPlan { order, work_per_node: f64::from_bits(*work as u64), rows_per_node: f64::from_bits(*rows as u64) });
        }
        Some(Prepared(out))
    }

    /// Line `line`'s plan, scaled to a run of `nodes` nodes: its order, work and rows.
    pub fn take(&self, line: u128, nodes: f64) -> Option<super::Plan> {
        let p = self.0.get(&line)?;
        TAKEN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let n = nodes.max(1.0);
        Some((p.order.clone(), p.work_per_node * n, p.rows_per_node * n))
    }
}

/// A relation's rows from its facts at the nominal scale; `None` where no fact bounds them.
fn rows_of(facts: &[LayoutFact], node_col: &dyn Fn(usize) -> bool) -> Option<f64> {
    let nodes = |cols: &[usize]| cols.len() == 1 && node_col(cols[0]);
    facts
        .iter()
        .filter_map(|f| match f {
            LayoutFact::Size(LayoutSize::Per { most, col }) if node_col(*col) => Some(f64::from((*most).max(1)) * NOMINAL_NODES),
            LayoutFact::Size(LayoutSize::PerDocument) => Some(NOMINAL_DOCS),
            LayoutFact::Size(LayoutSize::PerKind) => Some(NOMINAL_KINDS),
            LayoutFact::Size(LayoutSize::Small) => Some(f64::from(crate::code_rule_dsl::logic::front::FEW)),
            LayoutFact::Key(cols) if nodes(cols) => Some(NOMINAL_NODES),
            LayoutFact::Fanout { from, fan, .. } if nodes(from) => match fan {
                Fan::One => Some(NOMINAL_NODES),
                Fan::Few => Some(super::shapes::FEW_ROWS * NOMINAL_NODES),
                Fan::About(n) => Some(f64::from((*n).max(1)) * NOMINAL_NODES),
                Fan::Many => None,
            },
            _ => None,
        })
        .reduce(f64::min)
}

/// The model of the facts alone: each relation the facts size, its rows and its columns' distinct values; the base
/// relations' indexes as a run's; the program's `.shape` directives.
fn model_of(v: &Version, f: super::fields::Fields<'_>, shapes: &Shapes, layouts: &crate::code_rule_dsl::logic::program_image::layouts::Layouts) -> Model {
    let counts = Counts { docs: Some(NOMINAL_DOCS), kinds: Some(NOMINAL_KINDS) };
    let mut stats = Stats::empty_of(&f);
    stats.nodes = NOMINAL_NODES;
    stats.docs = Some(NOMINAL_DOCS as i64);
    let mut ndv = super::ColMap::default();
    for (rel, facts) in &layouts.0 {
        let name = v.rel_name(*rel);
        let node_col = |c: usize| v.decl_of(name).and_then(|d| d.cols().nth(c)).is_some_and(|(_, t)| matches!(v.text(t), "node" | "loc"));
        let facts: Vec<LayoutFact> = facts.iter().map(|(f, _)| f.clone()).collect();
        let Some(rows) = rows_of(&facts, &node_col) else { continue };
        stats.card.insert(name.to_owned(), rows);
        let width = v.decl_of(name).map_or(0, |d| d.cols().count());
        for c in 0..width {
            if let Some(n) = shapes.ndv(name, c, rows, counts) {
                ndv.insert((name.to_owned(), c), n);
                stats.fan.insert((name.to_owned(), c), (rows / n).max(1.0));
            }
        }
        if f.is_input(name) {
            Arc::make_mut(&mut stats.indexes).insert(name.to_owned(), super::vtab_indexes(name));
        }
    }
    let given = super::Inputs { facts: "main.".to_owned(), stats: "main.".to_owned(), atoms: Some([0; 3]), native: Default::default(), batch: HashMap::new(), given: None, kept: Default::default() };
    let shapes_0137 = Arc::new(
        f.shapes()
            .iter()
            .map(|s| crate::code_rule_dsl::logic::front::Shape {
                head: f.text(s.head).to_owned(),
                lit: f.text(s.lit).to_owned(),
                wait: match &s.wait {
                    crate::code_rule_dsl::logic::program_image::ShapeWaitRef::After(a) => crate::code_rule_dsl::logic::front::ShapeWait::After(f.text(*a).to_owned()),
                    crate::code_rule_dsl::logic::program_image::ShapeWaitRef::Bound(vs) => crate::code_rule_dsl::logic::front::ShapeWait::Bound(vs.iter().map(|&a| f.text(a).to_owned()).collect()),
                },
                text: f.text(s.text).to_owned(),
            })
            .collect::<Vec<_>>(),
    );
    Model { shapes: shapes_0137, analysis: Default::default(), stats, ndv, docof: Default::default(), view_src: Default::default(), inputs: given.into(), docs_by_shift: false, derived: Arc::new(HashSet::new()) }
}

impl Analysis for Prepared {
    const NAME: &'static str = "prepared plans";
    const READS: Reads = Reads::RULES.or(Reads::ABOUT);
    fn compute(cx: &Cx<'_>) -> Result<Prepared, String> {
        let _span = crate::code_rule_dsl::logic::profile::span(|| "prepared plans (decision 0361 step 5)".into());
        let v = cx.v();
        let shapes = cx.get::<Shapes>()?;
        let layouts = cx.get::<crate::code_rule_dsl::logic::program_image::layouts::Layouts>()?;
        let strata = cx.get::<Strata>()?;
        let placed = super::body::Placed::whole(Arc::clone(v));
        let f = placed.fields().map_err(|e| format!("{e:?}"))?;
        let mut model = model_of(v, f, &shapes, &layouts);
        model.derived = Arc::new((0..placed.len()).map(|k| placed.body(k)).map(|b| b.rel(b.head()).to_owned()).collect());
        // Each relation's component, by name: a recursive rule's deltas are its atoms of its own.
        let scc: HashMap<&str, usize> = strata.sccs.iter().enumerate().flat_map(|(i, s)| s.iter().map(move |r| (v.rel_name(*r), i))).collect();
        let mut out = HashMap::new();
        for k in 0..placed.len() {
            let b = placed.body(k);
            let known = |i: usize| -> bool {
                match b.view(i) {
                    Lit::Cmp { .. } => true,
                    Lit::Atom(a) => {
                        let rel = b.rel(a);
                        model.stats.card.contains_key(rel) && b.args(i).iter().enumerate().all(|(c, t)| matches!(t, TermRef::Wild) || model.ndv.contains_key(&(rel.to_owned(), c)))
                    }
                    _ => false,
                }
            };
            if !(0..b.len()).all(known) {
                continue;
            }
            let own = scc.get(b.rel(b.head())).copied();
            let deltas = (0..b.len()).filter(|&i| matches!(b.view(i), Lit::Atom(a) if !a.neg() && own.is_some() && scc.get(b.rel(a)).copied() == own));
            for first in std::iter::once(None).chain(deltas.map(Some)) {
                if let Some(d) = first.and_then(|i| b.atom(i)) {
                    let rel = b.rel(d).to_owned();
                    let rows = model.stats.card.get(&rel).copied().unwrap_or(1.0);
                    model.stats.delta_rows.insert(rel, (rows / f64::from(crate::code_rule_dsl::logic::front::FEW)).max(1.0));
                }
                let Ok((order, work, rows)) = model.plan_rule_priced(f, &b, first) else { continue };
                out.insert(super::line_key(&b, first), PreparedPlan { order, work_per_node: work / NOMINAL_NODES, rows_per_node: rows / NOMINAL_NODES });
            }
        }
        Ok(Prepared(out))
    }
    const VERSION: u32 = 1;
    fn rows(&self, _v: &Version) -> Option<Vec<Row>> {
        Some(self.to_rows())
    }
    fn of_rows(_v: &Version, rows: &[Row]) -> Option<Prepared> {
        Prepared::read_rows(rows.iter().map(Vec::as_slice))
    }
}

/// Prepared plans by the identity of the version they were made of (`manager::identity`): what the kept plans'
/// entry holds of them (Q183).
pub type ByVersion = HashMap<[u8; 32], Arc<Prepared>>;

/// `by`'s rows for the kept plans' entry: each the version's identity as four ints, then a plan's row.
pub fn store_rows(by: &ByVersion) -> Vec<Row> {
    let mut out: Vec<Row> = Vec::new();
    for (id, p) in by {
        let head: Vec<Value> = id.chunks(8).map(|c| Value::Int(i64::from_le_bytes(c.try_into().expect("eight bytes")))).collect();
        out.extend(p.to_rows().into_iter().map(|r| head.iter().cloned().chain(r).collect()));
    }
    out.sort_by(|a, b| format!("{a:?}").cmp(&format!("{b:?}")));
    out
}

/// The kept plans' entry's rows read back (`store_rows`'s); a row it cannot read is left out with its version's.
pub fn of_store_rows(rows: &[Row]) -> ByVersion {
    let mut by: HashMap<[u8; 32], Vec<&[Value]>> = HashMap::new();
    let mut bad = HashSet::new();
    for r in rows {
        let Some(ids) = r.get(..4).and_then(|h| h.iter().map(|v| if let Value::Int(i) = v { Some(*i) } else { None }).collect::<Option<Vec<i64>>>()) else { continue };
        let mut id = [0u8; 32];
        for (k, i) in ids.iter().enumerate() {
            id[k * 8..k * 8 + 8].copy_from_slice(&i.to_le_bytes());
        }
        by.entry(id).or_default().push(&r[4..]);
        if r.len() < 8 {
            bad.insert(id);
        }
    }
    by.into_iter().filter(|(id, _)| !bad.contains(id)).filter_map(|(id, rows)| Some((id, Arc::new(Prepared::read_rows(rows)?)))).collect()
}

fn mode_code(m: Option<super::Mode>) -> i64 {
    match m {
        None => 0,
        Some(super::Mode::Range) => 1,
        Some(super::Mode::Climb) => 2,
        Some(super::Mode::Descend) => 3,
    }
}

fn mode_of(c: i64) -> Option<super::Mode> {
    match c {
        1 => Some(super::Mode::Range),
        2 => Some(super::Mode::Climb),
        3 => Some(super::Mode::Descend),
        _ => None,
    }
}
