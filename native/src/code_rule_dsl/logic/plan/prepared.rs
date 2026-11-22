//! Plans made with the program from its relations' layouts, kept in its image (decision 0361 step 5).
//!
//! A rule whose every literal is an atom or a comparison, over relations whose facts give their sizes and the
//! distinct values of the columns it reads, is planned once a version: by the planner's own search, over a model of
//! the facts alone at a nominal scale (nothing measured, no store read). Its plans are kept by line (`line_key`: the
//! rule's text and its delta), with their work and rows per node, and a run takes them as they are: no search, no
//! pricing. Every other rule is planned at run time, as before (the exceptions).
use rerail_blocks::vector::Vector;
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

/// One line's plan: its order, its work and rows per node at the nominal scale, and the rows per node it assumed of
/// each derived relation it reads that only an estimate sized (`Estimate`: no fact does). A run takes it only while
/// those relations, made by then, are near what it assumed (`take`).
#[derive(Clone, Debug, PartialEq)]
pub struct PreparedPlan {
    pub order: Vec<Step>,
    pub work_per_node: f64,
    pub rows_per_node: f64,
    pub assumed: Vec<(String, f64)>,
}

/// The version's prepared plans, by line (`super::line_key`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Prepared(pub HashMap<u128, PreparedPlan>);


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
        // A program of no rules (a cross part whose every stratum was kept) has no plan to make, read or keep: kept,
        // it was a write of every warm run, which makes it fresh each time.
        if v.rules().next().is_none() {
            return Default::default();
        }
        // The run's kept plans' entry holds the plans an earlier process made of a version the same as this one (Q228):
        // read, not made.
        let Some(id) = super::run_holds_plans().then(|| manager::identity(v)) else { return manager::analysis::<Prepared>(v).unwrap_or_default() };
        if let Some(p) = super::prepared_stored(&id) {
            run_counts().versions[0].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return v.analyses().hold(p);
        }
        let p = manager::analysis::<Prepared>(v).unwrap_or_default();
        run_counts().versions[1].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        super::prepared_made(id, Arc::clone(&p));
        p
    }

    /// The plans as rows, each a line's (`Analysis::rows`).
    pub fn to_rows(&self) -> Vec<Row> {
        let mut rows: Vec<Row> = self
            .0
            .iter()
            .map(|(line, p)| {
                let mut r = vec![Value::Int((line >> 64) as i64), Value::Int(*line as i64), Value::Int(p.work_per_node.to_bits() as i64), Value::Int(p.rows_per_node.to_bits() as i64), Value::Int(p.order.len() as i64)];
                r.extend(p.order.iter().map(|s| Value::Int(((s.at as i64) << 2) | mode_code(s.mode))));
                for (rel, per_node) in &p.assumed {
                    r.push(Value::Str(rel.as_str().into()));
                    r.push(Value::Int(per_node.to_bits() as i64));
                }
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
            let [hi, lo, work, rows, n, rest @ ..] = r else { return None };
            let (hi, lo, work, rows, n) = (int(hi)?, int(lo)?, int(work)?, int(rows)?, usize::try_from(int(n)?).ok()?);
            let line = (u128::from(hi as u64) << 64) | u128::from(lo as u64);
            let (steps, assumed) = (rest.get(..n)?, rest.get(n..)?);
            let order = steps.iter().map(|s| { let s = int(s)?; Some(Step { at: usize::try_from(s >> 2).ok()?, mode: mode_of(s & 3) }) }).collect::<Option<Vec<_>>>()?;
            let assumed = assumed
                .chunks(2)
                .map(|pair| match pair {
                    [Value::Str(rel), v] => Some((rel.to_string(), f64::from_bits(int(v)? as u64))),
                    _ => None,
                })
                .collect::<Option<Vec<_>>>()?;
            out.insert(line, PreparedPlan { order, work_per_node: f64::from_bits(work as u64), rows_per_node: f64::from_bits(rows as u64), assumed });
        }
        Some(Prepared(out))
    }

    /// The rows per node line `line`'s plan assumed of each relation only an estimate sized (none where it has no plan).
    pub fn assumed(&self, line: u128) -> &[(String, f64)] {
        self.0.get(&line).map_or(&[], |p| p.assumed.as_slice())
    }

    /// Line `line`'s plan, scaled to a run of `nodes` nodes: its order, work and rows. Taken as it is, estimates and
    /// all: the run's planning stays lean for the plans that are right, and one that turns out heavy is searched where
    /// it runs (`Model::prepared_heavy`). `None` where it has none, where this run searched it so, or where it reads
    /// a relation the run's `shapes` left out, its facts broken (step 6): that line is planned from statistics.
    pub fn take(&self, line: u128, nodes: f64, b: &super::body::Body<'_>, shapes: &Shapes) -> Option<super::Plan> {
        let p = self.0.get(&line)?;
        if super::prepared_searched(line) {
            return None;
        }
        if shapes.leaves_out() && ((0..b.len()).filter_map(|i| b.atom(i)).any(|a| shapes.left_out(b.rel(a))) || p.assumed.iter().any(|(r, _)| shapes.left_out(r))) {
            return None;
        }
        run_counts().taken.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
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
fn model_of(placed: &super::body::Placed, v: &Version, f: super::fields::Fields<'_>, shapes: &Shapes, layouts: &crate::code_rule_dsl::logic::program_image::layouts::Layouts) -> Model {
    let counts = Counts { docs: Some(NOMINAL_DOCS), kinds: Some(NOMINAL_KINDS) };
    let mut stats = Stats::empty_of(&f);
    stats.nodes = NOMINAL_NODES;
    stats.docs = Some(NOMINAL_DOCS as i64);
    let mut ndv = super::ColMap::default();
    let mut input_indexes: Vec<(String, Vec<Vec<String>>)> = Vec::new();
    for (rel, facts) in &layouts.0 {
        let name = v.rel_name(*rel);
        let node_col = |c: usize| v.decl_of(name).and_then(|d| d.cols().nth(c)).is_some_and(|(_, t)| matches!(v.text(t), "node" | "loc"));
        let facts: Vec<LayoutFact> = facts.iter().map(|(f, _)| f.clone()).collect();
        let Some(rows) = rows_of(&facts, &node_col) else { continue };
        stats.card.insert(name, rows);
        let width = v.decl_of(name).map_or(0, |d| d.cols().count());
        for c in 0..width {
            if let Some(n) = shapes.ndv(name, c, rows, counts) {
                ndv.insert((name.to_owned(), c), n);
                stats.fan.insert((name.to_owned(), c), (rows / n).max(1.0));
            }
        }
        if f.is_input(name) {
            input_indexes.push((name.to_owned(), super::vtab_indexes(name)));
        }
    }
    stats.indexes = stats.indexes.with(input_indexes);
    let given = super::Inputs { facts: "main.".to_owned(), stats: "main.".to_owned(), atoms: Some([0; 3]), native: Default::default(), batch: HashMap::new(), given: None, kept: Default::default() };
    Model { shapes: super::ShapeSource::of(placed), analysis: Default::default(), stats, ndv, docof: Default::default(), view_src: Default::default(), inputs: given.into(), docs_by_shift: false, derived: Default::default() }
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
        let mut model = model_of(&placed, v, f, &shapes, &layouts);
        model.derived = (0..placed.len()).map(|k| placed.body(k)).map(|b| b.rel(b.head()).to_owned()).collect();
        // Each relation's component, by name: a recursive rule's deltas are its atoms of its own.
        let scc: HashMap<&str, usize> = strata.sccs.iter().enumerate().flat_map(|(i, s)| s.iter().map(move |r| (v.rel_name(*r), i))).collect();
        let mut by_head: HashMap<&str, Vector<usize>> = HashMap::new();
        for k in 0..placed.len() {
            let b = placed.body(k);
            by_head.entry(b.rel(b.head())).or_default().push(k);
        }
        let derive = derived_estimates();
        let mut out = HashMap::new();
        // The derived relations sized by an estimate, not a fact: what a plan reading them assumes, checked when taken.
        let mut estimated: HashSet<String> = HashSet::new();
        // Components in dependency order: each derived relation the facts do not size is sized from its rules' plans
        // (their rows priced, summed over the rules), so the rules that read it are planned after it.
        for comp in &strata.sccs {
            let names: Vec<&str> = comp.iter().map(|r| v.rel_name(*r)).collect();
            let rules: Vector<usize> = names.iter().flat_map(|n| by_head.get(n).into_iter().flatten().copied()).collect();
            let reads_own = |k: usize| {
                let b = placed.body(k);
                (0..b.len()).any(|i| b.atom(i).is_some_and(|a| names.contains(&b.rel(a))))
            };
            let (steps, base): (Vector<usize>, Vector<usize>) = rules.iter().partition(|&&k| reads_own(k));
            let sized_by_facts: Vec<&str> = names.iter().copied().filter(|n| model.stats.card.contains_key(*n)).collect();
            let mut sums: HashMap<&str, Estimate> = HashMap::new();
            let mut all_known = true;
            for &k in &base {
                match prepare_rule(&mut model, f, &layouts, &placed.body(k), &scc, &estimated, &mut out) {
                    Some(e) => sums.entry(placed.body(k).rel(placed.body(k).head())).or_default().add(e),
                    None => all_known = false,
                }
            }
            if !derive {
                for &k in &steps {
                    prepare_rule(&mut model, f, &layouts, &placed.body(k), &scc, &estimated, &mut out);
                }
                continue;
            }
            // A recursion's relations: their base rules' rows, times a nominal depth of the rounds (an estimate, as the
            // rows of every derived relation here are: the facts give none).
            let recursive = !steps.is_empty();
            let set = |model: &mut Model, estimated: &mut HashSet<String>, name: &str, e: &Estimate, times: f64| {
                if sized_by_facts.contains(&name) {
                    return;
                }
                estimated.insert(name.to_owned());
                let width = v.decl_of(name).map_or(0, |d| d.cols().count());
                let estimate = (e.rows * times).max(1.0);
                let recursion = times > 1.0;
                // Q290's oracle (`sizes_from`): the rows per node a run held of it, where one did.
                let rows = sizes_from().and_then(|m| m.get(name).copied()).map_or(estimate, |per| (per * NOMINAL_NODES).max(1.0));
                let times = times * rows / estimate;
                if let Some(log) = sizes_log() {
                    use std::io::Write as _;
                    let _ = writeln!(log.lock(), "estimate\t{name}\t{}\t{}", rows / NOMINAL_NODES, u8::from(recursion));
                }
                model.stats.card.insert(name, rows);
                for c in 0..width {
                    let n = (e.ndv.get(c).copied().unwrap_or(e.rows) * times).clamp(1.0, rows);
                    model.ndv.insert((name.to_owned(), c), n);
                    model.stats.fan.insert((name.to_owned(), c), (rows / n).max(1.0));
                }
            };
            if recursive {
                if all_known {
                    for n in &names {
                        if let Some(e) = sums.get(n) {
                            set(&mut model, &mut estimated, n, e, RECURSION_DEPTH);
                        }
                    }
                }
                for &k in &steps {
                    prepare_rule(&mut model, f, &layouts, &placed.body(k), &scc, &estimated, &mut out);
                }
            } else if all_known {
                for n in &names {
                    if let Some(e) = sums.get(n) {
                        set(&mut model, &mut estimated, n, e, 1.0);
                    }
                }
            }
        }
        Ok(Prepared(out))
    }
    /// 2: each plan's assumed sizes of the derived relations an estimate sized. 3: no plan for a line with a constant
    /// on a column a `values` fact answers.
    const VERSION: u32 = 3;
    fn rows(&self, _v: &Version) -> Option<Vec<Row>> {
        Some(self.to_rows())
    }
    fn of_rows(_v: &Version, rows: &[Row]) -> Option<Prepared> {
        Prepared::read_rows(rows.iter().map(|v| v.as_slice()))
    }
}

/// Prepared plans by the identity of the version they were made of (`manager::identity`): what the kept plans'
/// entry holds of them (Q228).
pub type ByVersion = HashMap<[u8; 32], Arc<Prepared>>;

/// `by`'s rows for the kept plans' entry: each the version's identity as four ints, then a plan's row.
pub fn store_rows(by: &ByVersion) -> Vec<Row> {
    let mut out: Vec<Row> = Vec::new();
    for (id, p) in by {
        let head: Vec<Value> = id.chunks(8).map(|c| Value::Int(i64::from_le_bytes(c.try_into().expect("eight bytes")))).collect();
        // A version with no prepared plan is kept too, as its id alone: else it is not read back, and the next run
        // makes its (empty) plans again and writes them.
        if p.0.is_empty() {
            out.push(head);
            continue;
        }
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
        let Some(ids) = r.get(..4).and_then(|h| h.iter().map(|v| if let Value::Int(i) = v { Some(*i) } else { None }).collect::<Option<Vector<i64>>>()) else { continue };
        let mut id = [0u8; 32];
        for (k, i) in ids.iter().enumerate() {
            id[k * 8..k * 8 + 8].copy_from_slice(&i.to_le_bytes());
        }
        let plans = by.entry(id).or_default();
        // Its id alone: a version with no prepared plan (`store_rows`).
        if r.len() == 4 {
            continue;
        }
        plans.push(&r[4..]);
        if r.len() < 8 {
            bad.insert(id);
        }
    }
    by.into_iter().filter(|(id, _)| !bad.contains(id)).filter_map(|(id, rows)| Some((id, Arc::new(Prepared::read_rows(rows)?)))).collect()
}

/// A recursion's rows, as its base rules' times this: the rounds' nominal depth (an estimate; a tree's recursions run
/// about as deep as its documents' nesting).
const RECURSION_DEPTH: f64 = 8.0;

/// Whether derived relations the facts do not size are sized from their rules' plans, and the rules reading them
/// prepared: off (Q290, 2026-11-19). Plans prepared over derived relations cost the run more the more lines they take,
/// and right sizes do not save them: corpus0 x B-now, 957-969 G instructions off, 991-1,056 G on, 1,128 G sized from
/// what a run on another project held, 1,203 G from what the same corpus's run held (an oracle), the same findings in
/// each. A relation's size is not what such a plan lacks; the distributions it is priced by are the facts' alone.
/// `RERAIL_DERIVED_ESTIMATES=on`, or a caller's `DerivedEstimates(Some(true))`, turns them on: a measurement aid, with
/// `sizes_log` and `sizes_from` (`scripts/profiling/derived_sizes.py`).
fn derived_estimates() -> bool {
    rerail_prof::code_rule_dsl::logic::profile::bound::<DerivedEstimates>().0.unwrap_or_else(|| std::env::var("RERAIL_DERIVED_ESTIMATES").is_ok_and(|x| x == "on"))
}

/// A caller's choice of `derived_estimates`, bound in the run's context (a test's); `None`, the environment's.
#[derive(Clone, Copy, Default)]
pub struct DerivedEstimates(pub Option<bool>);

/// A relation's rows and its columns' distinct values as its rules' plans give them: summed over its rules.
#[derive(Clone, Debug, Default)]
struct Estimate {
    rows: f64,
    ndv: Vector<f64>,
}

impl Estimate {
    fn add(&mut self, e: Estimate) {
        self.rows += e.rows;
        if self.ndv.len() < e.ndv.len() {
            self.ndv.resize(e.ndv.len(), 0.0);
        }
        for (a, b) in self.ndv.iter_mut().zip(e.ndv) {
            *a += b;
        }
    }
}

/// Plans rule `b` into `out` where every literal is one the model prices (an atom of a sized relation whose columns it
/// reads have distinct values, or a comparison), with a recursive rule's deltas: its head's estimate from the plan
/// (`None` where it is not planned). The rows are the plan's; a head column's distinct values those of the body column
/// that binds it, at most the rows (one for a constant).
fn prepare_rule(model: &mut Model, f: super::fields::Fields<'_>, layouts: &crate::code_rule_dsl::logic::program_image::layouts::Layouts, b: &super::body::Body<'_>, scc: &HashMap<&str, usize>, estimated: &HashSet<String>, out: &mut HashMap<u128, PreparedPlan>) -> Option<Estimate> {
    let known = |model: &Model, i: usize| -> bool {
        match b.view(i) {
            Lit::Cmp { .. } => true,
            Lit::Atom(a) => {
                let rel = b.rel(a);
                model.stats.card.contains_key(rel) && b.args(i).iter().enumerate().all(|(c, t)| matches!(t, TermRef::Wild) || model.ndv.contains_key(&(rel.to_owned(), c)))
            }
            _ => false,
        }
    };
    if !(0..b.len()).all(|i| known(model, i)) {
        if let Some(log) = why_log() {
            let i = (0..b.len()).find(|&i| !known(model, i)).expect("a literal not known");
            let why = match b.view(i) {
                Lit::Agg { .. } => "aggregate".to_owned(),
                Lit::Nearest { .. } => "nearest".to_owned(),
                Lit::Ast { .. } => "ast".to_owned(),
                Lit::Cmp { .. } => "comparison".to_owned(),
                Lit::Atom(a) => {
                    let rel = b.rel(a);
                    let side = if f.is_input(rel) { "input" } else { "derived" };
                    match model.stats.card.contains_key(rel) {
                        false => format!("unsized {side} {rel}"),
                        true => {
                            let c = b.args(i).iter().enumerate().find(|(c, t)| !matches!(t, TermRef::Wild) && !model.ndv.contains_key(&(rel.to_owned(), *c))).map_or(0, |(c, _)| c);
                            format!("column {side} {rel}.{c}")
                        }
                    }
                }
            };
            use std::io::Write as _;
            let _ = writeln!(log.lock(), "{why}\t{}", b.rel(b.head()));
        }
        return None;
    }
    // A constant on a column a `values` fact answers: the fact gives the column's distinct values, not how many rows
    // hold this one, and the plan turns on it. PostHog's `param`: `kind($p, #k1025)` (an identifier, 16 % of the
    // nodes) priced as one kind in 200 led and read 10.4 M rows, where the search, from the kind's frequency, led with
    // `field($f, #f65559, $ps)` (0.4 %): 253 k. Such a line is planned at run time, from the constant's statistics
    // (decision 0361 item 5: what no fact answers); its rows still size its head here.
    let by_values = |i: usize| -> bool {
        let Lit::Atom(a) = b.view(i) else { return false };
        b.args(i).iter().enumerate().any(|(c, t)| {
            matches!(t, TermRef::Int(_) | TermRef::Str(_) | TermRef::Branded(_)) && layouts.of(a.rel()).any(|f| matches!(f, LayoutFact::Values { col, .. } if *col == c))
        })
    };
    let kept = !(0..b.len()).any(by_values);
    let own = scc.get(b.rel(b.head())).copied();
    let deltas: Vector<usize> = (0..b.len()).filter(|&i| matches!(b.view(i), Lit::Atom(a) if !a.neg() && own.is_some() && scc.get(b.rel(a)).copied() == own)).collect();
    let mut whole = None;
    let mut assumed: Vec<(String, f64)> = Vec::new();
    for i in 0..b.len() {
        if let Some(a) = b.atom(i)
            && estimated.contains(b.rel(a))
            && !assumed.iter().any(|(r, _)| r == b.rel(a))
        {
            assumed.push((b.rel(a).to_owned(), model.stats.card.get(b.rel(a)).copied().unwrap_or(1.0) / NOMINAL_NODES));
        }
    }
    for first in std::iter::once(None).chain(deltas.into_iter().map(Some)) {
        if let Some(d) = first.and_then(|i| b.atom(i)) {
            let rel = b.rel(d).to_owned();
            let rows = model.stats.card.get(&rel).copied().unwrap_or(1.0);
            model.stats.delta_rows.insert(rel, (rows / f64::from(crate::code_rule_dsl::logic::front::FEW)).max(1.0));
        }
        let Ok((order, work, rows)) = model.plan_rule_priced(f, b, first) else { continue };
        if first.is_none() {
            whole = Some(rows);
        }
        if kept {
            out.insert(super::line_key(b, first), PreparedPlan { order, work_per_node: work / NOMINAL_NODES, rows_per_node: rows / NOMINAL_NODES, assumed: assumed.clone() });
        }
    }
    let rows = whole?.max(1.0);
    let ndv = b
        .head()
        .args()
        .map(|t| match t {
            TermRef::Var(x) => (0..b.len())
                .filter_map(|i| match b.view(i) {
                    Lit::Atom(a) if !a.neg() => b.args(i).iter().position(|u| matches!(u, TermRef::Var(y) if *y == x)).and_then(|c| model.ndv.get(&(b.rel(a).to_owned(), c)).copied()),
                    _ => None,
                })
                .reduce(f64::min)
                .unwrap_or(rows)
                .min(rows),
            TermRef::Int(_) | TermRef::Str(_) | TermRef::Branded(_) => 1.0,
            _ => rows,
        })
        .collect();
    Some(Estimate { rows, ndv })
}

/// `RERAIL_PREPARED_WHY=FILE` (a measurement aid): each rule not prepared, appended as a version's plans are made:
/// why (the first literal that stops it: its kind, or the relation with no size or the column with no distinct
/// values) and its head.
/// The run's own (opened once a run, decision 0442).
fn why_log() -> Option<std::sync::Arc<parking_lot::Mutex<std::fs::File>>> {
    crate::code_rule_dsl::logic::run::setting(|| {
        let p = std::env::var_os("RERAIL_PREPARED_WHY")?;
        std::fs::OpenOptions::new().create(true).append(true).open(p).ok().map(|f| std::sync::Arc::new(parking_lot::Mutex::new(f)))
    })
}

/// `RERAIL_SIZES_LOG=FILE` (Q290's measurement): the derived relations' sizes, estimated and met, appended as tab-separated
/// lines. `estimate REL PER_NODE RECURSIVE`: what preparation sized a relation at, rows per node at the nominal scale
/// (`RECURSIVE` 1 for a recursion's, its base rules' times `RECURSION_DEPTH`). `seen REL ASSUMED HELD NODES`: a prepared
/// plan taken by a run, each relation it assumed a size of, rows per node, against what the run holds of it then (a
/// relation still being solved is left out). `scripts/profiling/derived_sizes.py` reads it. The run's own.
pub fn sizes_log() -> Option<std::sync::Arc<parking_lot::Mutex<std::fs::File>>> {
    crate::code_rule_dsl::logic::run::setting(|| {
        let p = std::env::var_os("RERAIL_SIZES_LOG")?;
        std::fs::OpenOptions::new().create(true).append(true).open(p).ok().map(|f| std::sync::Arc::new(parking_lot::Mutex::new(f)))
    })
}

/// `RERAIL_SIZES_FROM=LOG` (Q290's oracle, a measurement aid: a labelled bound, never a product path): each derived
/// relation an estimate sizes is sized instead at the rows per node an earlier run held of it (the median of its `seen`
/// lines in `LOG`, `sizes_log`'s), its columns' distinct values scaled with it. A version's prepared plans are kept by
/// its identity (Q228), so a run under it needs a new store.
fn sizes_from() -> Option<std::sync::Arc<HashMap<String, f64>>> {
    crate::code_rule_dsl::logic::run::setting(|| {
        let text = std::fs::read_to_string(std::env::var_os("RERAIL_SIZES_FROM")?).ok()?;
        let mut held: HashMap<String, Vector<f64>> = HashMap::new();
        for line in text.lines() {
            let f: Vec<&str> = line.split('\t').collect();
            if let ["seen", rel, _, h, ..] = f.as_slice()
                && let Ok(h) = h.parse::<f64>()
            {
                held.entry((*rel).to_owned()).or_default().push(h);
            }
        }
        Some(std::sync::Arc::new(
            held.into_iter()
                .map(|(r, mut xs)| {
                    xs.sort_by(f64::total_cmp);
                    (r, xs[xs.len() / 2])
                })
                .collect(),
        ))
    })
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

/// The run's counts of this module's work (decision 0442: a run's own, `run::local`).
#[derive(Default)]
pub struct PreparedCounts {
    /// Lines a run took from the prepared plans (`RERAIL_PLAN_CACHE_COUNTS`).
    pub taken: std::sync::atomic::AtomicU64,
    /// Lines a run found heavy while they ran their prepared plan, and searched once (`Model::prepared_heavy`).
    pub searched: std::sync::atomic::AtomicU64,
    /// Versions whose prepared plans a run read from the store, and made (Q183; `RERAIL_PLAN_CACHE_COUNTS`).
    pub versions: [std::sync::atomic::AtomicU64; 2],
}

/// The run's `PreparedCounts`.
#[inline]
pub fn run_counts() -> std::sync::Arc<PreparedCounts> {
    crate::code_rule_dsl::logic::run::local::<PreparedCounts>()
}
