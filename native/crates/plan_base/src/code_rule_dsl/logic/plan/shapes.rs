//! The planner's estimates from relations' layouts (decision 0361 step 4): a column's distinct values from its
//! relation's facts, declared or inferred (`program_image::layouts`), where they answer it, and no statistics made for
//! a relation whose every column they answer.
//!
//! Step 6, the checks: a probe's rows, as the executors count them (decision 0203), against the most its relation's key
//! or fan-out lets it return (`broken_at`). A fact the rows break by a large factor is the run's (`Broken`): the run's
//! shapes leave out its relation, and every relation whose facts were inferred from it (`for_run`), so they are
//! planned from statistics for the rest of the run, no prepared plan reading them is taken, and the profile names the
//! relation and the fact.
use crate::code_rule_dsl::logic::front::{ArchivedLayoutFact, ArchivedLayoutSize, FEW, Fan, LayoutFact, LayoutValues};
use rerail_section::{Archive, Decode, Names, RelList, Section};
use std::collections::{BTreeMap, BTreeSet};

/// A `few` probe's rows, as the planner prices it: the class bounds it at `FEW` (16), and the trees' parents have
/// about three children (the batch estimate this replaced priced a parent column at a third of its rows).
pub const FEW_ROWS: f64 = 3.0;

/// Each relation's declared columns' names and its facts, by its name: every form of it (its deltas, its `__old`) is
/// named apart and takes none. Then, by relation, the relations its inferred facts were inferred from (the ones its
/// rules read); and the relations a run left out, their facts or those they were inferred from broken (step 6).
///
/// A section (decision 0628 §4), each list sorted by relation name and found by a search of it.
#[derive(Clone, PartialEq)]
pub struct Shapes(Section<Of>);

/// A relation's declared columns' names and its facts.
#[derive(Archive)]
#[archive(derive(Debug, PartialEq), decode)]
pub struct Rel {
    pub name: String,
    pub cols: Vec<String>,
    pub facts: Vec<LayoutFact>,
}

/// The relations a relation's inferred facts were inferred from.
#[derive(Archive)]
#[archive(derive(Debug, PartialEq), decode)]
pub struct Inferred {
    pub rel: String,
    pub from: Vec<String>,
}

/// `Shapes`' section: the relations' facts, what they were inferred from, and the relations a run left out, each
/// sorted by name.
#[derive(Archive, Default)]
#[archive(derive(Debug, PartialEq), decode)]
pub struct Of {
    pub rels: Vec<Rel>,
    pub from: Vec<Inferred>,
    pub left: Vec<String>,
}

impl Default for Shapes {
    fn default() -> Shapes {
        Shapes(Section::of(&Of::default()))
    }
}

impl std::fmt::Debug for Shapes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.get().fmt(f)
    }
}

/// How far a probe's rows pass the most its fact lets it return before the fact is broken: a large factor (decision
/// 0361: "a fan-out class broken by large factors"), as `joins::slow_factor` is a slow line's.
pub const BROKEN_BY: f64 = 8.0;

/// The fewest probes a check is made over: a few rows do not show a fact broken.
pub const LEAST_PROBES: u64 = 64;

/// The facts a run's rows broke (step 6), by relation: the first broken, as shown; and the probes checked against a
/// fact (`RERAIL_PLAN_CACHE_COUNTS`). A run's own (`run::local`). With them, the shapes `Shapes::for_run` last made:
/// the shapes it was given, how many relations were broken then, and what it made.
#[derive(Default)]
pub struct Broken(parking_lot::Mutex<BTreeMap<String, String>>, pub std::sync::atomic::AtomicU64, parking_lot::Mutex<Option<ForRun>>);

/// `Shapes::for_run`'s last answer: its shapes (held, so no other can take their address), the broken relations'
/// count it saw (they only grow, so the count names the set), and the shapes without them.
type ForRun = (std::sync::Arc<Shapes>, usize, std::sync::Arc<Shapes>);

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
    /// Kept by the version's identity (`plan::kept_analyses`, t33 K4): a relation's row is `rel`, its name, its
    /// columns' count, its columns and its facts (`LayoutFact::placed`); a `from` row the relations its facts were
    /// inferred from; a `left` row the relations left out.
    fn rows(&self, _v: &crate::code_rule_dsl::logic::program_image::Version) -> Option<Vec<crate::code_rule_dsl::logic::program_image::manager::Row>> {
        use crate::code_rule_dsl::logic::front::Value;
        let s = |x: &str| Value::Str(x.into());
        // Its section's lists, each sorted by name already.
        let of = self.0.decode();
        let mut out = Vec::new();
        for rel in &of.rels {
            let mut r = vec![s("rel"), s(&rel.name), Value::Int(rel.cols.len() as i64)];
            r.extend(rel.cols.iter().map(|c| s(c)));
            r.extend(rel.facts.iter().map(|f| s(&f.placed())));
            out.push(r);
        }
        for inferred in &of.from {
            out.push(std::iter::once(s("from")).chain(std::iter::once(s(&inferred.rel))).chain(inferred.from.iter().map(|r| s(r))).collect());
        }
        if !of.left.is_empty() {
            out.push(std::iter::once(s("left")).chain(of.left.iter().map(|r| s(r))).collect());
        }
        Some(out)
    }
    fn of_rows(_v: &crate::code_rule_dsl::logic::program_image::Version, rows: &[crate::code_rule_dsl::logic::program_image::manager::Row]) -> Option<Shapes> {
        use crate::code_rule_dsl::logic::front::Value;
        let text = |v: &Value| match v {
            Value::Str(x) => Some(x.to_string()),
            _ => None,
        };
        // The rows in `rows`' order: each list sorted by name, as the section keeps it.
        let mut of = Of::default();
        for r in rows {
            match text(r.first()?)?.as_str() {
                "rel" => {
                    let Value::Int(n) = r.get(2)? else { return None };
                    let n = usize::try_from(*n).ok()?;
                    let cols = r.get(3..3 + n)?.iter().map(text).collect::<Option<Vec<_>>>()?;
                    let facts = r.get(3 + n..)?.iter().map(|f| LayoutFact::of_placed(&text(f)?).ok()).collect::<Option<Vec<_>>>()?;
                    of.rels.push(Rel { name: text(r.get(1)?)?, cols, facts });
                }
                "from" => of.from.push(Inferred { rel: text(r.get(1)?)?, from: r.get(2..)?.iter().map(text).collect::<Option<_>>()? }),
                "left" => of.left = r.get(1..)?.iter().map(text).collect::<Option<_>>()?,
                _ => return None,
            }
        }
        let out = Shapes(Section::of(&of));
        Some(out)
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
        // Each relation's rules, by its head: found once, not by a scan of every rule for each relation.
        let mut by_head: std::collections::HashMap<_, Vec<crate::code_rule_dsl::logic::program_image::RuleRef<'_>>> = std::collections::HashMap::new();
        for r in v.rules() {
            by_head.entry(r.head().rel()).or_default().push(r);
        }
        let from = l
            .0
            .iter()
            .filter_map(|(rel, facts)| {
                let mut out = BTreeSet::new();
                for (_, o) in facts {
                    match o {
                        Origin::Rule(id) => v.rule(*id).into_iter().for_each(|r| reads(r, &mut out)),
                        Origin::Rules => by_head.get(rel).into_iter().flatten().for_each(|r| reads(*r, &mut out)),
                        Origin::Declared | Origin::Made => {}
                    }
                }
                (!out.is_empty()).then(|| (v.rel_name(*rel).to_owned(), out))
            })
            .collect();
        let from: BTreeMap<String, BTreeSet<String>> = from;
        let mut rels: Vec<Rel> = l
            .0
            .iter()
            .map(|(rel, facts)| {
                let name = v.rel_name(*rel);
                let cols = v.decl_of(name).map_or_else(Vec::new, |d| d.cols().map(|(c, _)| v.text(c).to_owned()).collect());
                Rel { name: name.to_owned(), cols, facts: facts.iter().map(|(f, _)| f.clone()).collect() }
            })
            .collect();
        rels.sort_by(|a, b| a.name.cmp(&b.name));
        rels.dedup_by(|a, b| a.name == b.name);
        let from = from.into_iter().map(|(rel, from)| Inferred { rel, from: from.into_iter().collect() }).collect();
        Shapes(Section::of(&Of { rels, from, left: Vec::new() }))
    }

    /// `rel`'s columns and facts, found by a search of the sorted relations.
    fn rel(&self, rel: &str) -> Option<&ArchivedRel> {
        let rels = &self.0.rels;
        rels.binary_search_by(|r| r.name.as_str().cmp(rel)).ok().map(|i| &rels[i])
    }

    /// `rel`'s facts, if it has a layout.
    pub fn facts(&self, rel: &str) -> Option<&RelList<ArchivedLayoutFact>> {
        self.rel(rel).map(|r| &r.facts)
    }

    /// The name of `rel`'s declared column `c`.
    pub fn column(&self, rel: &str, c: usize) -> Option<&str> {
        self.rel(rel)?.cols.get(c).map(|n| n.as_str())
    }

    /// Each relation with inferred facts and the relations they were inferred from.
    pub fn inferred(&self) -> &RelList<ArchivedInferred> {
        &self.0.from
    }

    /// Whether no relation has facts.
    pub fn is_empty(&self) -> bool {
        self.0.rels.is_empty()
    }

    /// Whether the run left out any relation (`for_run`).
    pub fn leaves_out(&self) -> bool {
        !self.0.left.is_empty()
    }

    /// These shapes in the run: without the relations whose facts its rows broke (`Broken`), nor any whose facts were
    /// inferred from one left out. The same shapes where the run broke none of theirs. Made once for the same shapes
    /// and broken relations: each chunk asks it of every source it measures, and each `without` decodes and encodes
    /// the whole section (1.4 s of the 300-file bench's main thread, `sample` on the M1 Max).
    pub fn for_run(s: std::sync::Arc<Shapes>) -> std::sync::Arc<Shapes> {
        // Outside a run (a plan made for a test, a tool's), no rows were checked.
        if s.is_empty() || crate::code_rule_dsl::logic::run::current().is_none() {
            return s;
        }
        let run = run_broken();
        let broken = run.relations();
        if broken.is_empty() {
            return s;
        }
        let mut last = run.2.lock();
        if let Some((given, n, made)) = last.as_ref()
            && std::sync::Arc::ptr_eq(given, &s)
            && *n == broken.len()
        {
            return std::sync::Arc::clone(made);
        }
        let made = s.without(&broken).map_or_else(|| std::sync::Arc::clone(&s), std::sync::Arc::new);
        *last = Some((s, broken.len(), std::sync::Arc::clone(&made)));
        made
    }

    /// These shapes without `broken`'s relations and every one whose facts were inferred from one left out; `None`
    /// where that leaves out none they hold.
    pub fn without(&self, broken: &BTreeSet<String>) -> Option<Shapes> {
        let mut out: BTreeSet<String> = broken.iter().filter(|r| self.rel(r).is_some()).cloned().collect();
        loop {
            let more: Vec<String> = self.0.from.iter().filter(|i| !out.contains(i.rel.as_str()) && i.from.iter().any(|f| out.contains(f.as_str()))).map(|i| i.rel.as_str().to_owned()).collect();
            if more.is_empty() {
                break;
            }
            out.extend(more);
        }
        out.retain(|r| self.rel(r).is_some());
        if out.is_empty() {
            return None;
        }
        // A new section of what is left: the rest of the relations, the left out ones added to the run's.
        let mut of = Of::decode(self.0.get(), &Names::NONE);
        of.rels.retain(|r| !out.contains(&r.name));
        of.left.extend(out);
        of.left.sort();
        of.left.dedup();
        Some(Shapes(Section::of(&of)))
    }

    /// Whether `rel` is one the run left out (`for_run`): a prepared plan reading it is not taken.
    pub fn left_out(&self, rel: &str) -> bool {
        self.0.left.binary_search_by(|r| r.as_str().cmp(rel)).is_ok()
    }

    /// The fact of `rel` that bounds a probe on its columns `bound`, and the most rows it lets a probe return: a key
    /// within them, one; a fan-out from within them, its class's bound (`one`; `few`, 16; `many` bounds nothing).
    pub fn probe_limit(&self, rel: &str, bound: &[usize]) -> Option<(f64, &ArchivedLayoutFact)> {
        let facts = self.facts(rel)?;
        let within = |cols: &[usize]| !cols.is_empty() && cols.iter().all(|c| bound.contains(c));
        facts
            .iter()
            .filter_map(|f| match f {
                ArchivedLayoutFact::Key(k) if within(k) => Some((1.0, f)),
                ArchivedLayoutFact::Fanout { from, fan, .. } if within(from) => probe_bound(*fan).map(|n| (n, f)),
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
        let cols = &self.rel(rel)?.cols;
        // The fact made again to be shown: a broken fact is reported once a run.
        Some(LayoutFact::decode(fact, &Names::NONE).show(&|c| cols.get(c).map_or_else(|| format!("${c}"), |n| n.as_str().to_owned())))
    }

    /// Column `c` of `rel`'s distinct values, of `rows` rows, from its facts: a key's column has one a row; a
    /// size `per c` one per `most` rows; a fan-out's source one per probe's rows; a dense order within a fan-out's
    /// source as many as a probe returns at most; a `values` fact by its class, of the run's `counts`. `None` where no
    /// fact answers it.
    pub fn ndv(&self, rel: &str, c: usize, rows: f64, counts: Counts) -> Option<f64> {
        let facts = self.facts(rel)?;
        let rows = rows.max(1.0);
        let by = |f: &ArchivedLayoutFact| -> Option<f64> {
            match f {
                ArchivedLayoutFact::Key(k) if k.as_slice() == [c] => Some(rows),
                ArchivedLayoutFact::Size(ArchivedLayoutSize::Per { most, col }) if *col == c => Some((rows / f64::from((*most).max(1))).max(1.0)),
                ArchivedLayoutFact::Fanout { from, fan, .. } if from.as_slice() == [c] => probe_rows(*fan).map(|n| (rows / n).max(1.0)),
                ArchivedLayoutFact::Order { col, within, dense: true } if *col == c => facts.iter().find_map(|g| match g {
                    ArchivedLayoutFact::Fanout { from, fan, .. } if from.as_slice() == within.as_slice() => probe_bound(*fan).map(|n| rows.min(n)),
                    _ => None,
                }),
                ArchivedLayoutFact::Values { col, values } if *col == c => match values {
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
        self.rel(rel).is_some_and(|r| !r.cols.is_empty() && (0..r.cols.len()).all(|c| self.ndv(rel, c, 2.0, counts).is_some()))
    }

    /// The place of `rel`'s declared column `name` (a source's spelling: `end_` is `end`).
    pub fn place(&self, rel: &str, name: &str) -> Option<usize> {
        let cols = &self.rel(rel)?.cols;
        cols.iter().position(|c| c.as_str() == name).or_else(|| cols.iter().position(|c| c.as_str() == name.trim_end_matches('_')))
    }

    /// A column's statistic as `syntax::measured` gives it, `(rows, ndv, sq, sqd)`, from its facts where they answer
    /// it: postings taken even (`sq = rows² / ndv`), and within documents a document's share of them (one document's
    /// alone where the column is the documents).
    pub fn statistic(&self, rel: &str, name: &str, rows: i64, counts: Counts) -> Option<(i64, i64, i64, i64)> {
        let c = self.place(rel, name)?;
        let r = rows.max(0) as f64;
        let ndv = self.ndv(rel, c, r, counts)?.max(1.0);
        let sq = (r * r / ndv).max(r);
        let documents = self.facts(rel).is_some_and(|fs| fs.iter().any(|f| matches!(f, ArchivedLayoutFact::Values { col, values: LayoutValues::Documents } if *col == c)));
        let sqd = if documents { sq } else { counts.docs.map_or(sq, |d| (sq / d.max(1.0)).max(r)) };
        Some((rows, ndv.round() as i64, sq.round() as i64, sqd.round() as i64))
    }
}
