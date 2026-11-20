//! The preparation passes on the pass manager (decision 0281's path, step 5): each a pass over the prepared
//! program's version, run in one pipeline after the program's one image, from the last of the tree's passes
//! backward as they move.

use crate::code_rule_dsl::logic::program_image::manager::{self, Run, Cx, Pipeline, ProgramPass, ProgramW};
use crate::code_rule_dsl::logic::program_image::Version;
use std::collections::{BTreeSet, HashSet};
use std::sync::Arc;

mod merge;
pub(super) mod combining;
pub(super) mod hide;
pub(crate) mod on_request;
pub(super) mod whole;
pub(crate) mod lowering_tail;
mod demand;
mod magic;
mod prune;
mod rank;
mod simplify;
mod nearest;
pub mod parts;
pub mod text;
pub use merge::{DropRepeated, MergeIdentical};
pub use nearest::Nearest;

/// A check of the program that writes nothing (what every relation read is bound by, `corpus::bound`): its error
/// stops the pipeline.
pub struct Check<F>(pub F);

impl<F: FnMut(&Version) -> Result<(), String>> ProgramPass for Check<F> {
    const NAME: &'static str = "a check of the program";
    fn run(&mut self, cx: &Cx<'_>, _w: &mut ProgramW<'_>) -> Result<(), String> {
        (self.0)(cx.v())
    }
}

/// An input no rule reads is never bound or measured: it is not an input of what runs (an output stays one).
pub struct InputsRead;

impl ProgramPass for InputsRead {
    const NAME: &'static str = "the inputs the rules read";
    fn run(&mut self, cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<(), String> {
        let v = cx.v();
        let read: HashSet<u32> = v.rules().flat_map(|r| r.body().filter_map(|l| l.atom()).map(|a| a.rel().0).collect::<Vec<_>>()).collect();
        let outputs: HashSet<&str> = v.outputs().map(|a| v.text(a)).collect();
        let inputs: Vec<&str> = v.inputs().map(|a| v.text(a)).collect();
        let kept: Vec<&str> = inputs.iter().copied().filter(|i| v.rel(i).is_some_and(|r| read.contains(&r.0)) || outputs.contains(i)).collect();
        if kept.len() != inputs.len() {
            w.on()?.set_inputs(&kept);
        }
        Ok(())
    }
}

/// A measurement aid (decision 0173, open question 4): the rules reordered (`reverse`, `head`, `shuffle`), to find
/// what depends on their order.
pub struct RuleOrder(pub String);

impl ProgramPass for RuleOrder {
    const NAME: &'static str = "the rules reordered";
    fn run(&mut self, cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<(), String> {
        let v = cx.v();
        let mut rules: Vec<_> = v.rules().collect();
        match self.0.as_str() {
            "reverse" => rules.reverse(),
            "head" => rules.sort_by(|a, b| v.rel_name(a.head().rel()).cmp(v.rel_name(b.head().rel()))),
            "shuffle" => {
                let mut x: u64 = 0x0173;
                for i in (1..rules.len()).rev() {
                    x ^= x << 13;
                    x ^= x >> 7;
                    x ^= x << 17;
                    rules.swap(i, (x % (i as u64 + 1)) as usize);
                }
            }
            _ => return Ok(()),
        }
        let input = w.input().clone();
        let b = w.derive()?;
        for r in rules {
            b.keep(&input, r);
        }
        Ok(())
    }
}

/// What the passes found: the relations merged, the classes that went to the shared namespace, and the repeated
/// rules dropped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    /// The relations the run makes for the tree axes and `nearest` (`Nearest::sets`).
    pub sets: Vec<(String, Vec<i64>)>,
    /// The relations the file-local joins were pushed into (`file_local::PushLocalJoins`).
    pub pushed: Vec<String>,
    /// What the factorized store handed on as parts (`file_local::FactorizeExports`): a line per relation.
    pub factorized: Vec<String>,
    /// What the export demand cut (`file_local::DemandExports`): a line per export, or why it cut none.
    pub demand: Vec<String>,
    /// The relations located (`file_local::Locate`), or those left.
    pub located: Result<Vec<String>, Vec<String>>,
    pub merged: usize,
    pub shared: usize,
    pub repeated: usize,
}

/// What the passes before lowering found: the closures held as bitmaps and the relations through them, the
/// relations demanded for their negated tests, the program's own demand, the reaches made rows again, the matchers
/// and the vocabulary's tests.
pub struct Front {
    pub reaches: (usize, usize),
    pub demanded: Vec<String>,
    /// The relations the program lists (`.pragma "magic-transform"`), and the transformation's report or why it was
    /// not applied.
    pub demand: Option<(Vec<String>, Result<crate::code_rule_dsl::logic::front::ast::Report, String>)>,
    pub undone: Vec<String>,
    pub matchers: text::Matchers,
    pub vocabulary: crate::code_rule_dsl::logic::strings::Vocabulary,
    /// The relations the deferred joins pass gave a pair cache.
    pub cached: Vec<String>,
}

/// `RERAIL_READER_DEMAND=off` (a measurement aid): a node product is bounded by its direct readers' demand only,
/// not by demand projected from their own readers (`product_demand::rewrite`).
pub fn reader_demand() -> bool {
    !std::env::var("RERAIL_READER_DEMAND").is_ok_and(|v| v == "off")
}

/// How the deferred joins pass (`product_demand::deferred`) runs for one prepared program. `RERAIL_DEFERRED_JOINS`:
/// `auto` (the default) and `adaptive` share a frequently requested endpoint's work and admit small pair caches; `on`
/// makes the static rewrites alone; `off` leaves the joins as written. On by default (decision 0477): an insurance
/// against a join over-long for its readers' demand, at 1.0% more instructions on corpus0 x B-now (961.8 G against
/// 952.7 G, the same findings). A run that loads (`world::wanted`) keeps them: a load hands exports back by location,
/// which a new factor has none of.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Deferral {
    pub on: bool,
    pub adaptive: bool,
    /// Whether pair caches may be admitted: off once a cache drew a document-backed answer into the cross-file part.
    pub cache_pairs: bool,
    /// The rounds of `ON_REQUEST` made eager before the pass, as `answers_where_documents_are` makes them: the
    /// answers direct factor reads drew into the cross-file part, projected in their documents.
    pub undone: BTreeSet<usize>,
    /// The attribute each relation's pair cache was turned off for.
    pub placement: std::collections::BTreeMap<String, String>,
}

impl Deferral {
    /// The setting for a program, from `RERAIL_DEFERRED_JOINS`, and what the run's earlier tries changed (`Replans`).
    pub fn of(loading_run: bool, key: &[u8; 32]) -> Deferral {
        let mode = crate::code_rule_dsl::logic::profile::bound::<DeferredJoins>().0.map_or_else(|| std::env::var("RERAIL_DEFERRED_JOINS").unwrap_or_else(|_| "auto".into()), str::to_owned);
        let on = !loading_run && matches!(mode.as_str(), "auto" | "on" | "adaptive");
        let adaptive = mode != "on";
        let base = Deferral { on, adaptive, cache_pairs: adaptive, ..Default::default() };
        crate::code_rule_dsl::logic::profile::with_bound::<Replans, _>(|r| r.0.as_ref().and_then(|r| r.lock().get(key).cloned())).filter(|_| on).unwrap_or(base)
    }

    /// What the next try changes, when `rel`, answered by a function of its node's document, landed in the cross-file
    /// part: the pair caches off (`cached` lists the relations that had one), else the round answering `rel` made
    /// eager. None when neither is left to try.
    pub fn after_placement(&self, rel: &str, cached: &[String]) -> Option<Deferral> {
        use crate::code_rule_dsl::logic::rr::lower::ON_REQUEST;
        if !self.on {
            return None;
        }
        let mut next = self.clone();
        if self.cache_pairs && !cached.is_empty() {
            next.cache_pairs = false;
            next.placement.extend(cached.iter().map(|c| (c.clone(), rel.to_owned())));
            crate::code_rule_dsl::logic::profile::decision("deferred join placement", || format!("{rel} needs the document; use direct factor reads"));
            return Some(next);
        }
        let round = ON_REQUEST.iter().position(|round| round.iter().any(|a| a.rel == rel))?;
        if !next.undone.insert(round) {
            return None;
        }
        crate::code_rule_dsl::logic::profile::decision("deferred join placement", || format!("project {rel} in its source document before reading its factors"));
        Some(next)
    }

    fn settings(&self) -> Vec<u8> {
        let mut k = vec![u8::from(self.adaptive), u8::from(self.cache_pairs)];
        k.extend(self.undone.iter().map(|&r| r as u8 + 2));
        for (rel, attribute) in &self.placement {
            k.extend_from_slice(format!("\0{rel}={attribute}").as_bytes());
        }
        k
    }
}

/// A caller's choice of `RERAIL_DEFERRED_JOINS`'s mode for its runs (a test's), over the variable's.
#[derive(Clone, Default)]
pub struct DeferredJoins(pub Option<&'static str>);

/// The run's changed deferrals, by prepared program: a placement that failed is tried again with the next one
/// (`Deferral::after_placement`). Bound by `corpus::run_observed` for its run.
#[derive(Clone, Default)]
pub struct Replans(pub Option<Arc<parking_lot::Mutex<std::collections::BTreeMap<[u8; 32], Deferral>>>>);

/// The stages of `front`, in order, at which a debug build checks the version against the tree's passes.
pub const STAGES: [&str; 12] = ["inputs", "prune", "simplify", "prune again", "products", "deferred", "reach", "negated demand", "demand", "undo", "rank", "text"];

/// `prog` as the deferred joins pass leaves it under `d`, the rounds `d` makes eager undone first: what it decided,
/// and the relations it gave a pair cache.
pub fn defer_joins(prog: &mut crate::code_rule_dsl::logic::front::Program, d: &Deferral) -> (Vec<manager::Decided>, Vec<String>) {
    use crate::code_rule_dsl::logic::profile::author_joins::{Access, TOPIC};
    use crate::code_rule_dsl::logic::rr::lower::{ON_REQUEST, undo_round};
    for &round in &d.undone {
        let mut rules = std::mem::take(&mut prog.rules);
        undo_round(prog, &mut rules, ON_REQUEST[round]);
        prog.rules = rules;
    }
    if !d.undone.is_empty() {
        *prog = crate::code_rule_dsl::logic::front::prune(std::mem::take(prog));
    }
    let plans = super::product_demand::deferred::rewrite(prog, d.adaptive, d.cache_pairs);
    let pass = std::borrow::Cow::Borrowed("joins deferred to their readers' requests");
    let mut decided = vec![manager::Decided { pass: pass.clone(), what: "deferred joins".into(), detail: format!("pair demand: {:?}; endpoint demand: {:?}; reduced factors: {:?}; fanout sharing: {:?}; left as written, read across files: {:?}", plans.pairs, plans.one_sided, plans.factors, plans.shared, plans.cross) }];
    let mut cached = Vec::new();
    for mut join in plans.author {
        match &join.access {
            Access::Factors { cache: Some(_) } => cached.push(join.relation.clone()),
            Access::Factors { cache: None } => join.placement = d.placement.get(&join.relation).cloned(),
            _ => {}
        }
        decided.push(manager::Decided { pass: pass.clone(), what: TOPIC.into(), detail: serde_json::to_string(&join).expect("join decision") });
    }
    (decided, cached)
}

/// `version` pruned to the rules its outputs read (`front::prune`): what the bridged programs' check reads
/// (`bridge::wanted`), before the run prepares the program.
pub fn pruned(version: Arc<Version>) -> Result<Arc<Version>, String> {
    let mut p = Pipeline::new("pruned", version);
    p.program(&mut prune::Prune::default());
    Ok(p.current()?.clone())
}

/// The passes before lowering, on the version (decision 0281's path, steps 6c–6e): the functors only CPython gives
/// and the base relations as inputs, a prune, `simplify` and a prune again, the reach form, the demand for negated
/// tests, the program's own demand (`RERAIL_DEMAND=off` computes the listed relations in full) and a prune after
/// it, the reaches in cycles made rows again, the counts below a bound made by rank, then the text passes. Their decisions reach the run's profile, as the
/// tree's passes made them. `check` is called after each of `STAGES` with the version so far.
pub fn front(version: Arc<Version>, dialect: crate::code_rule_dsl::language::GrammarDialect, deferral: &Deferral, last: Option<Run>, check: &mut dyn FnMut(&'static str, &Arc<Version>)) -> Result<(Arc<Version>, Front, Run), String> {
    let mut p = kept(FRONT, version, last)?;
    let mut at = |p: &mut Pipeline<'_>, stage: &'static str| -> Result<(), String> {
        let v = p.current()?.clone();
        check(stage, &v);
        Ok(())
    };
    // @py_unparse(n) and @ast_dump(n) read the relations the batches bind them as; the base relations the program
    // reads and does not derive are inputs, as an authored file's are.
    p.program(&mut simplify::CpythonFunctors);
    p.program(&mut simplify::BaseInputs);
    at(&mut p, "inputs")?;
    // Only the rules an output reads, directly or through other rules (T's front prunes the same way).
    p.program(&mut prune::Prune::default());
    at(&mut p, "prune")?;
    // Bodies simplified (decision 0038), then pruned again.
    let simplified = simplify::passes(&mut p)?;
    simplify::decide(&simplified);
    at(&mut p, "simplify")?;
    p.program(&mut prune::Prune::default());
    at(&mut p, "prune again")?;
    // A rule whose head pairs node ranges no join connects (a Cartesian product of the corpus's nodes) is bounded by
    // the tuples its readers can use, or refused (`product_demand`). Found on the version; the rewrite reads the
    // tree, decoded only for a program with such a product. A part prepared alone (`parts`) lacks its readers.
    if !parts::on() {
        let v = p.current()?.clone();
        let d = p.get::<crate::code_rule_dsl::logic::plan::fields::FieldsData>()?;
        if super::product_demand::has_products(&v, &crate::code_rule_dsl::logic::plan::fields::Fields::over(&v, &d)) {
            let narrow = reader_demand();
            p.made("node products bound by their readers' demand", 1, &[u8::from(narrow)], |v| {
                let mut prog = v.program(&Default::default())?;
                let r = super::product_demand::rewrite(&mut prog, narrow)?;
                let mut decided = Vec::new();
                let pass = std::borrow::Cow::Borrowed("node products bound by their readers' demand");
                if !r.products.is_empty() {
                    decided.push(manager::Decided { pass: pass.clone(), what: "node Cartesian products".into(), detail: format!("restricted to connected reader demands: {}", r.products.into_iter().collect::<Vec<_>>().join(", ")) });
                }
                if !r.readers.is_empty() {
                    decided.push(manager::Decided { pass, what: "projected reader demand".into(), detail: format!("{} ({} ordered node and {} present location witnesses)", r.readers.into_iter().collect::<Vec<_>>().join(", "), r.order_witnesses, r.present_witnesses) });
                }
                let (out, _) = crate::code_rule_dsl::logic::program_image::Builder::of_program(&prog).and_then(|b| b.finish())?;
                Ok((out, decided))
            });
        }
    }
    at(&mut p, "products")?;
    // Joins deferred to what their readers request (`product_demand::deferred`): the requested pairs or endpoints
    // drive the producer, or the two factors meet in each reader. A part prepared alone lacks its readers.
    if deferral.on && !parts::on() {
        p.made("joins deferred to their readers' requests", 1, &deferral.settings(), |v| {
            let mut prog = v.program(&Default::default())?;
            let (decided, _) = defer_joins(&mut prog, deferral);
            let (out, _) = crate::code_rule_dsl::logic::program_image::Builder::of_program(&prog).and_then(|b| b.finish())?;
            Ok((out, decided))
        });
    }
    at(&mut p, "deferred")?;
    let mut reach = demand::ReachForm::default();
    p.program(&mut reach);
    at(&mut p, "reach")?;
    let mut negated = demand::NegatedDemand::default();
    p.program(&mut negated);
    at(&mut p, "negated demand")?;
    let mut own = magic::ProgramDemand::default();
    if std::env::var("RERAIL_DEMAND").map_or(true, |x| x != "off") {
        p.program(&mut own);
        if matches!(own.outcome, Some(Ok(_))) {
            p.program(&mut prune::Prune::default());
            // A measurement aid: `RERAIL_DEMAND_DUMP=<file>` writes the rules as demand left them.
            if let Some(path) = std::env::var_os("RERAIL_DEMAND_DUMP")
                && let Ok(v) = p.current()
            {
                let _ = std::fs::write(path, v.rules().map(|r| format!("{}\n", v.show_rule(r))).collect::<String>());
            }
        }
    }
    at(&mut p, "demand")?;
    let mut undo = demand::UndoInCycles::default();
    p.program(&mut undo);
    at(&mut p, "undo")?;
    // Counts below a bound made without the pairs (decision 0361 step 7), once demand has placed its readers: a
    // relation the form replaced is unread, and pruned.
    let mut ranks = rank::RankForm::default();
    p.program(&mut ranks);
    if ranks.found > 0 {
        p.program(&mut prune::Prune::default());
        crate::code_rule_dsl::logic::profile::decision("counts below a bound (decision 0361 step 7)", || format!("{} counts made by rank over their asked and counted rows, not their pairs", ranks.found));
    }
    at(&mut p, "rank")?;
    let text::Found { matchers, vocabulary } = text::passes(&mut p, dialect)?;
    at(&mut p, "text")?;
    let mut cached = Vec::new();
    for d in p.decisions() {
        if d.what == crate::code_rule_dsl::logic::profile::author_joins::TOPIC
            && let Ok(j) = serde_json::from_str::<crate::code_rule_dsl::logic::profile::author_joins::Join>(&d.detail)
            && matches!(j.access, crate::code_rule_dsl::logic::profile::author_joins::Access::Factors { cache: Some(_) })
        {
            cached.push(j.relation);
        }
        crate::code_rule_dsl::logic::profile::decision(&d.what, || d.detail.clone());
    }
    let run = p.done()?;
    let demand = own.outcome.map(|o| (own.listed, o));
    Ok((run.output().clone(), Front { reaches: (reach.reaches, reach.throughs), demanded: negated.demanded, demand, undone: undo.undone, matchers, vocabulary, cached }, run))
}

/// The passes that follow the program's image, over `version`: the program that runs, a base (what the store keeps),
/// with what they found; the pipeline's stages traced. The joins pushed are made per rule, so two programs' copies
/// of one library rule made two: after the rows are located, they are merged again, and so are the relations over
/// them, before the program splits. The merges (`merge`) run as `corpus::merge_and_drop_repeats`
/// runs them, or the merge alone with `RERAIL_SAME=off`.
/// `bound` checks every relation read is bound, after `nearest` is read from the transducer.
pub fn run(version: Arc<Version>, factorize: bool, last: Option<Run>, bound: impl FnMut(&Version) -> Result<(), String>) -> Result<(Arc<Version>, Found, Run), String> {
    let mut p = kept(PREPARATION, version, last)?;
    let mut nearest = Nearest::default();
    p.rules(&mut nearest);
    p.program(&mut Check(bound));
    // The file-local part of each cross-file rule runs in the chunks, which export its rows, not their base relations.
    let mut pushed = crate::code_rule_dsl::typing::file_local::PushLocalJoins::default();
    p.program(&mut pushed);
    // An export that is a join of parts meeting at a key it projects away is handed on as its parts (decision 0099).
    let mut factorized = crate::code_rule_dsl::typing::file_local::FactorizeExports::new(factorize);
    p.program(&mut factorized);
    // Each export cut to what its cross-file readers can use: their demand, made file-local.
    let mut demand = crate::code_rule_dsl::typing::file_local::DemandExports::default();
    p.program(&mut demand);
    for line in &demand.lines {
        crate::code_rule_dsl::logic::profile::decision("export demand", || line.clone());
    }
    // Every relation the chunks hand on names its document per row, so a file's rows can be memoised (decision 0052).
    // A relation a round leaves (its rows name no document) is moved to the cross-file part, its file-local readers
    // following, and the program is located again until every row handed on is located (decision 0290, as
    // `file_local::locate` does on the tree).
    let mut moved: Vec<String> = Vec::new();
    let mut located: Vec<String> = Vec::new();
    let located = loop {
        let mut locate = crate::code_rule_dsl::typing::file_local::Locate::default();
        p.program(&mut locate);
        located.extend(locate.located);
        if locate.blocked.is_empty() {
            break Ok(located);
        }
        let fresh: Vec<String> = locate.blocked.into_iter().filter(|r| !moved.contains(r)).collect();
        if fresh.is_empty() {
            moved.sort();
            break Err(moved);
        }
        moved.extend(fresh.iter().cloned());
        p.program(&mut crate::code_rule_dsl::typing::file_local::MoveCrossFile(fresh));
    };
    let mut merge = MergeIdentical::default();
    let mut repeats = DropRepeated::default();
    if std::env::var("RERAIL_SAME").is_ok_and(|v| v == "off") {
        p.program(&mut merge);
    } else {
        p.program(&mut repeats);
        p.program(&mut merge);
        p.program(&mut repeats);
    }
    let found = Found { sets: nearest.sets, pushed: pushed.made, factorized: factorized.lines, demand: demand.lines, located, merged: merge.merged, shared: merge.shared, repeated: repeats.0 };
    p.program(&mut InputsRead);
    if let Ok(order) = std::env::var("RERAIL_RULE_ORDER") {
        p.program(&mut RuleOrder(order));
    }
    let run = p.done()?;
    let (out, stages) = (run.output().clone(), run.stages());
    for s in stages {
        crate::code_rule_dsl::logic::profile::decision("preparation pass (decision 0281)", || format!("{}: {:?}, {:?}", s.pass, s.ran, s.form));
    }
    // The store keeps a base's bytes (`program_text::encode_version`): an edit's version is folded into one.
    let out = match out.base_bytes() {
        Some(_) => out,
        None => out.fold()?,
    };
    Ok((out, found, run))
}

/// The pipelines a run keeps, by name (decision 0333 §5): the passes before lowering, and those after.
pub const FRONT: &str = "before lowering";
pub const PREPARATION: &str = "preparation";

/// The key a pipeline's run is kept under (decision 0333 §2): its name, and the digest the run's manifest names.
pub fn run_key(name: &str, named: &[u8; 32]) -> [u8; 32] {
    use sha2::Digest as _;
    sha2::Sha256::new().chain_update(b"rerail pipeline run\0").chain_update(name.as_bytes()).chain_update(b"\0").chain_update(named).finalize().into()
}

/// The digest a run's manifest names its pipelines' runs by: of the first pipeline's input, so two runs of one
/// program name one.
pub fn runs_digest(front: &Run) -> [u8; 32] {
    use sha2::Digest as _;
    sha2::Sha256::new().chain_update(b"rerail pipeline runs\0").chain_update(crate::code_rule_dsl::logic::program_image::manager::identity(front.input())).finalize().into()
}

/// A pipeline named `name` over `version`, kept to be stored: run `again` from `last` when `version` is an edit of
/// its input (`rebase`), else from scratch.
fn kept(name: &'static str, version: Arc<Version>, last: Option<Run>) -> Result<Pipeline<'static>, String> {
    if let Some(last) = last
        && let Some(v) = crate::code_rule_dsl::logic::program_image::manager::rebase(last.input(), &version)?
    {
        crate::code_rule_dsl::logic::profile::decision("the last run (decision 0333)", || format!("{name}: run again over an edit of the last run's input ({} rules)", crate::code_rule_dsl::logic::program_image::manager::Delta::of(last.input(), &v).size()));
        return Ok(Pipeline::again(last, v));
    }
    Ok(Pipeline::keeping(name, version))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An input no rule reads leaves the inputs (an output stays one); the rules reordered keep each rule; the
    /// program that comes out is a base, which the store keeps.
    #[test]
    fn the_passes_after_the_image_prune_the_inputs_and_reorder_the_rules() {
        let p = crate::code_rule_dsl::logic::rr::parse(
            ".decl a(x: number)\n.decl b(x: number)\n.decl c(x: number)\n.input a, b, c\n.decl p(x: number)\n.decl q(x: number)\n\
             p(x) :- a(x).\nq(x) :- p(x).\n.output p, q, c\n",
        )
        .unwrap();
        let (v, _) = crate::code_rule_dsl::logic::program_image::Builder::of_program(&p).unwrap().finish().unwrap();
        let (out, _, _) = run(v.clone(), true, None, |_| Ok(())).unwrap();
        assert!(out.base_bytes().is_some(), "a base");
        let inputs: Vec<&str> = out.inputs().map(|a| out.text(a)).collect();
        assert_eq!(inputs, ["a", "c"], "b is read by no rule; c is an output");
        let mut pl = Pipeline::new("test", v);
        pl.program(&mut RuleOrder("reverse".into()));
        let (rev, _) = pl.finish().unwrap();
        let heads: Vec<&str> = rev.rules().map(|r| rev.rel_name(r.head().rel())).collect();
        assert_eq!(heads, ["q", "p"]);
    }

    /// The request record (decision 0331) follows the steps' rules, the derivations' too (by each rule's source), and
    /// holds one entry a rule after the pipeline; readers merged are named by their class.
    #[test]
    fn the_request_record_follows_the_preparation_steps() {
        let mut p = crate::code_rule_dsl::logic::rr::parse(
            ".decl k(x: number)\n.input k\n.decl q(x: number)\n.decl a(x: number)\n.decl b(x: number)\n\
             q(x) :- k(x), x < 3.\nq(x) :- k(x), x > 5.\na(x) :- q(x).\nb(x) :- q(x), x != 1.\n.output a, b\n",
        )
        .unwrap();
        let by = |r: &[&str]| r.iter().map(|s| (*s).to_owned()).collect::<std::collections::BTreeSet<String>>();
        p.asked_by.insert("q".into(), vec![by(&["a"]), by(&["b"])]);
        let (v, _) = crate::code_rule_dsl::logic::program_image::Builder::of_program(&p).unwrap().finish().unwrap();
        let (out, _, _) = run(v, true, None, |_| Ok(())).unwrap();
        assert_eq!(out.asked_against_rules(), [("q".to_owned(), 2, 2)]);
        assert_eq!(out.asked()["q"], [by(&["a"]), by(&["b"])]);
        // Two equal readers merged: the record names the class (`simplify::merge_identical`'s rename, the tree's).
        let mut p = crate::code_rule_dsl::logic::rr::parse(
            ".decl k(x: number)\n.input k\n.decl q(x: number)\n.decl a(x: number)\n.decl a2(x: number)\n.decl o(x: number)\n\
             q(x) :- k(x), x < 3.\nq(x) :- k(x), x > 5.\na(x) :- q(x).\na2(x) :- q(x).\no(x) :- a(x), a2(x).\n.output o\n",
        )
        .unwrap();
        p.asked_by.insert("q".into(), vec![by(&["a"]), by(&["a2"])]);
        let (v, _) = crate::code_rule_dsl::logic::program_image::Builder::of_program(&p).unwrap().finish().unwrap();
        let (out, _, _) = run(v, true, None, |_| Ok(())).unwrap();
        assert!(out.rel("a2").is_none_or(|r| out.heads(r).next().is_none()), "a2 merged into a");
        assert_eq!(out.asked()["q"], [by(&["a"]), by(&["a"])]);
    }
}
