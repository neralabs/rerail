//! The planner's estimates from relations' layouts (decision 0361 step 4): a column's distinct values from its
//! relation's facts, declared or inferred (`program_image::layouts`), where they answer it, and no statistics made for
//! a relation whose every column they answer.
//!
//! Step 6, the checks: a probe's rows, as the executors count them (decision 0203), against the most its relation's key
//! or fan-out lets it return (`broken_at`). A fact the rows break by a large factor is the run's (`Broken`): the run's
//! shapes leave out its relation, and every relation whose facts were inferred from it (`for_run`), so they are
//! planned from statistics for the rest of the run, no prepared plan reading them is taken, and the profile names the
//! relation and the fact.
use crate::code_rule_dsl::logic::front::{FEW, Fan, LayoutFact, LayoutSize, LayoutValues};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// A `few` probe's rows, as the planner prices it: the class bounds it at `FEW` (16), and the trees' parents have
/// about three children (the batch estimate this replaced priced a parent column at a third of its rows).
pub const FEW_ROWS: f64 = 3.0;

/// Each relation's declared columns' names and its facts, by its name: every form of it (its deltas, its `__old`) is
/// named apart and takes none. Then, by relation, the relations its inferred facts were inferred from (the ones its
/// rules read); and the relations a run left out, their facts or those they were inferred from broken (step 6).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Shapes(pub HashMap<String, (Vec<String>, Vec<LayoutFact>)>, pub HashMap<String, BTreeSet<String>>, pub BTreeSet<String>);

/// How far a probe's rows pass the most its fact lets it return before the fact is broken: a large factor (decision
/// 0361: "a fan-out class broken by large factors"), as `joins::slow_factor` is a slow line's.
pub const BROKEN_BY: f64 = 8.0;

/// The fewest probes a check is made over: a few rows do not show a fact broken.
pub const LEAST_PROBES: u64 = 64;

/// The facts a run's rows broke (step 6), by relation: the first broken, as shown; and the probes checked against a
/// fact (`RERAIL_PLAN_CACHE_COUNTS`). A run's own (`run::local`).
#[derive(Default)]
pub struct Broken(parking_lot::Mutex<BTreeMap<String, String>>, pub std::sync::atomic::AtomicU64);

impl Broken {
    /// Notes `rel`'s fact `fact` broken; whether it is the first of `rel` this run.
    pub fn note(&self, rel: &str, fact: String) -> bool {
        let mut m = self.0.lock();
        if m.contains_key(rel) {
            return false;
        }
        m.insert(rel.to_owned(), fact);
        true
    }

    /// The relations whose facts the run's rows broke.
    pub fn relations(&self) -> BTreeSet<String> {
        self.0.lock().keys().cloned().collect()
    }

    /// Each broken relation and its fact.
    pub fn facts(&self) -> BTreeMap<String, String> {
        self.0.lock().clone()
    }
}

/// The run's `Broken`.
pub fn run_broken() -> std::sync::Arc<Broken> {
    crate::code_rule_dsl::logic::run::local::<Broken>()
}

/// Notes `rel`'s fact `fact` broken in the run (`Broken`), and the first time names it, `text`: in the profile, and on
/// stderr in a strict run. Whether it was the first. A probe's counts (`Model::layout_broken`) and a key's second row
/// where the rows are written (`setfix::Rel::set_key`) both break a fact here.
pub fn report_broken(rel: &str, fact: String, text: impl FnOnce() -> String) -> bool {
    if !run_broken().note(rel, fact) {
        return false;
    }
    let text = text();
    if crate::code_rule_dsl::logic::profile::strict() {
        eprintln!("rerail: a layout fact broken (decision 0361 step 6): {text}");
    }
    crate::code_rule_dsl::logic::profile::decision("broken layout facts (decision 0361 step 6)", || text);
    true
}

/// The rows a probe of a fan-out's columns returns, as priced; `None` for `many` (statistics price it).
fn probe_rows(fan: Fan) -> Option<f64> {
    match fan {
        Fan::One => Some(1.0),
        Fan::Few => Some(FEW_ROWS),
        Fan::About(n) => Some(f64::from(n.max(1))),
        Fan::Many => None,
    }
}

/// The most rows a probe of a fan-out's columns returns, by its class; `None` for `many`.
fn probe_bound(fan: Fan) -> Option<f64> {
    match fan.class() {
        Fan::One => Some(1.0),
        Fan::Few => Some(f64::from(FEW)),
        _ => None,
    }
}

/// The run's exact counts a `values` fact is priced by (decision 0361 item 5): the documents and the grammar's kinds
/// its statistics hold (`Stats::counts`); `None` where they are not known, and a fact of them answers nothing.
#[derive(Clone, Copy, Debug, Default)]
pub struct Counts {
    pub docs: Option<f64>,
    pub kinds: Option<f64>,
}

/// An analysis of the version (`manager::analysis`): made once a version, and shared by every plan made of it (a
/// session's runs make tens of thousands; made per plan, it was a third of `make_plan`'s own CPU).
impl crate::code_rule_dsl::logic::program_image::manager::Analysis for Shapes {
    const NAME: &'static str = "shapes";
    const READS: crate::code_rule_dsl::logic::program_image::manager::Reads = crate::code_rule_dsl::logic::program_image::manager::Reads::RULES.or(crate::code_rule_dsl::logic::program_image::manager::Reads::ABOUT);
    fn compute(cx: &crate::code_rule_dsl::logic::program_image::manager::Cx<'_>) -> Result<Shapes, String> {
        let l = cx.get::<crate::code_rule_dsl::logic::program_image::layouts::Layouts>()?;
        Ok(Shapes::of_layouts(cx.v(), &l))
    }
}

impl Shapes {
    /// The facts of `v`'s relations (`Layouts`), by name: the version's analysis, shared.
    pub fn of_version(v: &std::sync::Arc<crate::code_rule_dsl::logic::program_image::Version>) -> std::sync::Arc<Shapes> {
        // `RERAIL_LAYOUT_ESTIMATES=off` plans from statistics alone, as before (a measurement aid).
        if std::env::var("RERAIL_LAYOUT_ESTIMATES").is_ok_and(|x| x == "off") {
            return Default::default();
        }
        // A program the analysis refuses (its strata) has no shapes: it plans from statistics, as before.
        crate::code_rule_dsl::logic::program_image::manager::analysis::<Shapes>(v).unwrap_or_default()
    }

    fn of_layouts(v: &crate::code_rule_dsl::logic::program_image::Version, l: &crate::code_rule_dsl::logic::program_image::layouts::Layouts) -> Shapes {
        use crate::code_rule_dsl::logic::program_image::layouts::Origin;
        let reads = |r: crate::code_rule_dsl::logic::program_image::RuleRef<'_>, out: &mut BTreeSet<String>| out.extend(r.body().filter_map(|l| l.atom()).map(|a| v.rel_name(a.rel()).to_owned()));
        let from = l
            .0
            .iter()
            .filter_map(|(rel, facts)| {
                let mut out = BTreeSet::new();
                for (_, o) in facts {
                    match o {
                        Origin::Rule(id) => v.rule(*id).into_iter().for_each(|r| reads(r, &mut out)),
                        Origin::Rules => v.rules().filter(|r| r.head().rel() == *rel).for_each(|r| reads(r, &mut out)),
                        Origin::Declared | Origin::Made => {}
                    }
                }
                (!out.is_empty()).then(|| (v.rel_name(*rel).to_owned(), out))
            })
            .collect();
        Shapes(
            l.0.iter()
                .map(|(rel, facts)| {
                    let name = v.rel_name(*rel);
                    let cols = v.decl_of(name).map_or_else(Vec::new, |d| d.cols().map(|(c, _)| v.text(c).to_owned()).collect());
                    (name.to_owned(), (cols, facts.iter().map(|(f, _)| f.clone()).collect()))
                })
                .collect(),
            from,
            BTreeSet::new(),
        )
    }

    /// These shapes in the run: without the relations whose facts its rows broke (`Broken`), nor any whose facts were
    /// inferred from one left out. The same shapes where the run broke none of theirs.
    pub fn for_run(s: std::sync::Arc<Shapes>) -> std::sync::Arc<Shapes> {
        // Outside a run (a plan made for a test, a tool's), no rows were checked.
        if s.0.is_empty() || crate::code_rule_dsl::logic::run::current().is_none() {
            return s;
        }
        let broken = run_broken().relations();
        if broken.is_empty() {
            return s;
        }
        s.without(&broken).map_or(s, std::sync::Arc::new)
    }

    /// These shapes without `broken`'s relations and every one whose facts were inferred from one left out; `None`
    /// where that leaves out none they hold.
    pub fn without(&self, broken: &BTreeSet<String>) -> Option<Shapes> {
        let mut out: BTreeSet<String> = broken.iter().filter(|r| self.0.contains_key(*r)).cloned().collect();
        loop {
            let more: Vec<String> = self.1.iter().filter(|(r, from)| !out.contains(*r) && from.iter().any(|f| out.contains(f))).map(|(r, _)| r.clone()).collect();
            if more.is_empty() {
                break;
            }
            out.extend(more);
        }
        out.retain(|r| self.0.contains_key(r));
        if out.is_empty() {
            return None;
        }
        let mut s = self.clone();
        for r in &out {
            s.0.remove(r);
        }
        s.2.extend(out);
        Some(s)
    }

    /// Whether `rel` is one the run left out (`for_run`): a prepared plan reading it is not taken.
    pub fn left_out(&self, rel: &str) -> bool {
        self.2.contains(rel)
    }

    /// The fact of `rel` that bounds a probe on its columns `bound`, and the most rows it lets a probe return: a key
    /// within them, one; a fan-out from within them, its class's bound (`one`; `few`, 16; `many` bounds nothing).
    pub fn probe_limit(&self, rel: &str, bound: &[usize]) -> Option<(f64, &LayoutFact)> {
        let (_, facts) = self.0.get(rel)?;
        let within = |cols: &[usize]| !cols.is_empty() && cols.iter().all(|c| bound.contains(c));
        facts
            .iter()
            .filter_map(|f| match f {
                LayoutFact::Key(k) if within(k) => Some((1.0, f)),
                LayoutFact::Fanout { from, fan, .. } if within(from) => probe_bound(*fan).map(|n| (n, f)),
                _ => None,
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
    }

    /// Step 6's check of a probe of `rel` on its columns `bound`: `into` probes made `made` rows. The fact it breaks,
    /// shown, where they passed the most it lets them return by `BROKEN_BY`, over `LEAST_PROBES` probes at least.
    pub fn broken_at(&self, rel: &str, bound: &[usize], into: u64, made: u64) -> Option<String> {
        if into < LEAST_PROBES {
            return None;
        }
        let (most, fact) = self.probe_limit(rel, bound)?;
        run_broken().1.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if (made as f64) <= BROKEN_BY * most * into as f64 {
            return None;
        }
        let cols = &self.0.get(rel)?.0;
        Some(fact.show(&|c| cols.get(c).cloned().unwrap_or_else(|| format!("${c}"))))
    }

    /// Column `c` of `rel`'s distinct values, of `rows` rows, from its facts: a key's column has one a row; a
    /// size `per c` one per `most` rows; a fan-out's source one per probe's rows; a dense order within a fan-out's
    /// source as many as a probe returns at most; a `values` fact by its class, of the run's `counts`. `None` where no
    /// fact answers it.
    pub fn ndv(&self, rel: &str, c: usize, rows: f64, counts: Counts) -> Option<f64> {
        let (_, facts) = self.0.get(rel)?;
        let rows = rows.max(1.0);
        let by = |f: &LayoutFact| -> Option<f64> {
            match f {
                LayoutFact::Key(k) if k.as_slice() == [c] => Some(rows),
                LayoutFact::Size(LayoutSize::Per { most, col }) if *col == c => Some((rows / f64::from((*most).max(1))).max(1.0)),
                LayoutFact::Fanout { from, fan, .. } if from.as_slice() == [c] => probe_rows(*fan).map(|n| (rows / n).max(1.0)),
                LayoutFact::Order { col, within, dense: true } if *col == c => facts.iter().find_map(|g| match g {
                    LayoutFact::Fanout { from, fan, .. } if from == within => probe_bound(*fan).map(|n| rows.min(n)),
                    _ => None,
                }),
                LayoutFact::Values { col, values } if *col == c => match values {
                    LayoutValues::Documents => counts.docs.map(|d| rows.min(d.max(1.0))),
                    LayoutValues::Kinds => counts.kinds.map(|k| rows.min(k.max(1.0))),
                    LayoutValues::WithinDocument => counts.docs.map(|d| (rows / d.max(1.0)).max(1.0)),
                    LayoutValues::Many => Some(rows),
                    LayoutValues::About(n) => Some(rows.min(f64::from((*n).max(1)))),
                    LayoutValues::OneIn(n) => Some((rows / f64::from((*n).max(1))).max(1.0)),
                },
                _ => None,
            }
        };
        // The most distinct values any fact gives (a key and a fan-out of one column: the key's).
        facts.iter().filter_map(by).reduce(f64::max)
    }

    /// Whether every one of `rel`'s columns is answered (`ndv`): its statistics are not made.
    pub fn answers(&self, rel: &str, counts: Counts) -> bool {
        self.0.get(rel).is_some_and(|(cols, _)| !cols.is_empty() && (0..cols.len()).all(|c| self.ndv(rel, c, 2.0, counts).is_some()))
    }

    /// The place of `rel`'s declared column `name` (a source's spelling: `end_` is `end`).
    pub fn place(&self, rel: &str, name: &str) -> Option<usize> {
        let (cols, _) = self.0.get(rel)?;
        cols.iter().position(|c| c == name).or_else(|| cols.iter().position(|c| c == name.trim_end_matches('_')))
    }

    /// A column's statistic as `syntax::measured` gives it, `(rows, ndv, sq, sqd)`, from its facts where they answer
    /// it: postings taken even (`sq = rows² / ndv`), and within documents a document's share of them (one document's
    /// alone where the column is the documents).
    pub fn statistic(&self, rel: &str, name: &str, rows: i64, counts: Counts) -> Option<(i64, i64, i64, i64)> {
        let c = self.place(rel, name)?;
        let r = rows.max(0) as f64;
        let ndv = self.ndv(rel, c, r, counts)?.max(1.0);
        let sq = (r * r / ndv).max(r);
        let documents = self.0.get(rel).is_some_and(|(_, fs)| fs.iter().any(|f| matches!(f, LayoutFact::Values { col, values: LayoutValues::Documents } if *col == c)));
        let sqd = if documents { sq } else { counts.docs.map_or(sq, |d| (sq / d.max(1.0)).max(r)) };
        Some((rows, ndv.round() as i64, sq.round() as i64, sqd.round() as i64))
    }
}
