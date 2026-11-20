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
pub mod on_request;
pub(super) mod whole;
pub(crate) mod lowering_tail;
pub mod demand;
pub mod magic;
pub mod prune;
mod rank;
pub mod simplify;
mod nearest;
pub mod parts;
pub mod text;
pub use merge::{DropRepeated, MergeIdentical, congruence};
pub use prune::Prune;
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
            w.on()?.fields().set_inputs(&kept);
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
/// `off` (the default) leaves the joins as written; `auto` and `adaptive` share a frequently requested endpoint's work
/// and admit small pair caches; `on` makes the static rewrites alone. Off by default (decision 0477): on corpus0 x
/// B-now `auto` took 7-12% more instructions for the same findings, and nothing prices the rewrite yet. A run that loads (`world::wanted`) keeps them:
/// a load hands exports back by location, which a new factor has none of.
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
        let mode = crate::code_rule_dsl::logic::profile::bound::<DeferredJoins>().0.map_or_else(|| std::env::var("RERAIL_DEFERRED_JOINS").unwrap_or_else(|_| "off".into()), str::to_owned);
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

/// The deferred joins' step (`defer_joins`).
const DEFERRED_PASS: &str = "joins deferred to their readers' requests";

/// `v` as the deferred joins pass (`product_demand::deferred`) leaves it under `d`, and what it decided: the
/// rewrite's plans, and each rewritten join for the authors' report (a factor read placed as `d` asks).
pub fn defer_joins(v: &Arc<Version>, d: &Deferral) -> Result<(Arc<Version>, Vec<manager::Decided>), String> {
    use crate::code_rule_dsl::logic::profile::author_joins::{Access, TOPIC};
    let (out, plans) = super::product_demand::deferred::defer(v, d.adaptive, d.cache_pairs)?;
    let pass = std::borrow::Cow::Borrowed(DEFERRED_PASS);
    let mut decided = vec![manager::Decided {
        pass: pass.clone(),
        what: "deferred joins".into(),
        detail: format!("pair demand: {:?}; endpoint demand: {:?}; reduced factors: {:?}; fanout sharing: {:?}", plans.pairs, plans.one_sided, plans.factors, plans.shared),
    }];
    for mut join in plans.author {
        if let Access::Factors { cache: None } = &join.access {
            join.placement = d.placement.get(&join.relation).cloned();
        }
        decided.push(manager::Decided { pass: pass.clone(), what: TOPIC.into(), detail: serde_json::to_string(&join).expect("join decision") });
    }
    Ok((out, decided))
}

/// `version` pruned to the rules its outputs read (`preparation::Prune`): what the bridged programs' check reads
/// (`bridge::wanted`), before the run prepares the program.
pub fn pruned(version: Arc<Version>) -> Result<Arc<Version>, String> {
    let mut p = Pipeline::new("pruned", version);
    p.program(&mut prune::Prune::default());
    Ok(p.current()?.clone())
}

/// The passes before lowering, on the version (decision 0281's path, steps 6c–6e): the functors only CPython gives
/// and the base relations as inputs, a prune, `simplify` and a prune again, the reach form, the demand for negated
/// tests, the program's own demand (`RERAIL_DEMAND=off` computes the listed relations in full) and a prune after
/// it, the reaches in cycles made rows again, the counts below a bound made by rank, then the text passes. Their decisions reach the run's profile.
pub fn front(version: Arc<Version>, dialect: crate::code_rule_dsl::language::GrammarDialect, deferral: &Deferral, last: Option<Run>) -> Result<(Arc<Version>, Front, Run), String> {
    front_of(version, dialect, deferral, last, true)
}

/// `front`, with the node products bounded by their readers (`product_demand`) where `whole`: a whole program's; a
/// part's are bounded once the parts are linked (`parts::front`), where the products' readers are.
pub(super) fn front_of(version: Arc<Version>, dialect: crate::code_rule_dsl::language::GrammarDialect, deferral: &Deferral, last: Option<Run>, whole: bool) -> Result<(Arc<Version>, Front, Run), String> {
    let mut p = kept(FRONT, version, last)?;
    // @py_unparse(n) and @ast_dump(n) read the relations the batches bind them as; the base relations the program
    // reads and does not derive are inputs, as an authored file's are.
    p.program(&mut simplify::CpythonFunctors);
    p.program(&mut simplify::BaseInputs);
    // Only the rules an output reads, directly or through other rules (T's front prunes the same way).
    p.program(&mut prune::Prune::default());
    // Bodies simplified (decision 0038), then pruned again.
    let simplified = simplify::passes(&mut p)?;
    simplify::decide(&simplified);
    p.program(&mut prune::Prune::default());
    if whole {
        p.program(&mut super::product_demand::BoundProducts::Whole);
    }
    // Joins deferred to what their readers request (`product_demand::deferred`, decision 0477): the requested pairs
    // or endpoints drive the producer, or the two factors meet in each reader. A part prepared alone lacks its
    // readers. The rounds of attributes answered on request that an earlier try made eager are undone first.
    if deferral.on && whole {
        for &round in &deferral.undone {
            p.made("a round of requests made eager (deferred joins)", 1, &[round as u8], |v| {
                use crate::code_rule_dsl::logic::rr::lower::{ON_REQUEST, undo_round, undone_with};
                Ok((undone_with(v, |head, rules| undo_round(head, rules, ON_REQUEST[round]))?, Vec::new()))
            });
        }
        if !deferral.undone.is_empty() {
            p.program(&mut prune::Prune::default());
        }
        p.made(DEFERRED_PASS, 1, &deferral.settings(), |v| defer_joins(v, deferral));
    }
    let mut reach = demand::ReachForm::default();
    p.program(&mut reach);
    let mut negated = demand::NegatedDemand::default();
    p.program(&mut negated);
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
    let mut undo = demand::UndoInCycles::default();
    p.program(&mut undo);
    // Counts below a bound made without the pairs (decision 0361 step 7), once demand has placed its readers: a
    // relation the form replaced is unread, and pruned.
    let mut ranks = rank::RankForm::default();
    p.program(&mut ranks);
    if ranks.found > 0 {
        p.program(&mut prune::Prune::default());
        crate::code_rule_dsl::logic::profile::decision("counts below a bound (decision 0361 step 7)", || format!("{} counts made by rank over their asked and counted rows, not their pairs", ranks.found));
    }
    let text::Found { matchers, vocabulary } = text::passes(&mut p, dialect)?;
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
/// them, before the program splits. The merges (`merge`) run as `MergeIdentical` between two `DropRepeated`, or
/// the merge alone with `RERAIL_SAME=off`. One merge finds every class at once: its congruence is the greatest, and
/// it compares a relation's rules as a set, so a repeated rule does not keep a relation from its twin. The merge's
/// renaming can make rules repeat; they are dropped after it, and make nothing more mergeable. (A loop of merges
/// until no rule repeated was 2-3 congruences a call, 1.5 s of the dl1 batch's 13.8 s CPU.)
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
    let located = crate::code_rule_dsl::typing::file_local::locate_all(&mut p);
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
    // Its versions a few edits deep made flat (decision 0385), so a ninth edit in a row runs again too.
    if let Some(last) = last.map(crate::code_rule_dsl::logic::program_image::manager::Run::shallow)
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

    /// `p` with the readers of each of `q`'s rules, in order (its `asked` rows).
    fn asked(p: &Arc<Version>, readers: Vec<std::collections::BTreeSet<String>>) -> Arc<Version> {
        use crate::code_rule_dsl::logic::program_facts::Field;
        use crate::code_rule_dsl::logic::program_image::{Builder, fields::FieldsMut};
        let mut b = Builder::on(p);
        b.fields().set_field(Field::Asked(&std::collections::BTreeMap::from([("q".to_owned(), readers)])));
        b.finish().unwrap().0
    }

    /// An input no rule reads leaves the inputs (an output stays one); the rules reordered keep each rule; the
    /// program that comes out is a base, which the store keeps.
    #[test]
    fn the_passes_after_the_image_prune_the_inputs_and_reorder_the_rules() {
        let v = crate::code_rule_dsl::logic::rr::parse_version(
            ".decl a(x: number)\n.decl b(x: number)\n.decl c(x: number)\n.input a, b, c\n.decl p(x: number)\n.decl q(x: number)\n\
             p(x) :- a(x).\nq(x) :- p(x).\n.output p, q, c\n",
        )
        .unwrap();
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
        let p = crate::code_rule_dsl::logic::rr::parse_version(
            ".decl k(x: number)\n.input k\n.decl q(x: number)\n.decl a(x: number)\n.decl b(x: number)\n\
             q(x) :- k(x), x < 3.\nq(x) :- k(x), x > 5.\na(x) :- q(x).\nb(x) :- q(x), x != 1.\n.output a, b\n",
        )
        .unwrap();
        let by = |r: &[&str]| r.iter().map(|s| (*s).to_owned()).collect::<std::collections::BTreeSet<String>>();
        let v = asked(&p, vec![by(&["a"]), by(&["b"])]);
        let (out, _, _) = run(v, true, None, |_| Ok(())).unwrap();
        assert_eq!(out.asked_against_rules(), [("q".to_owned(), 2, 2)]);
        assert_eq!(out.asked()["q"], [by(&["a"]), by(&["b"])]);
        // Two equal readers merged: the record names the class (`MergeIdentical`'s rename).
        let p = crate::code_rule_dsl::logic::rr::parse_version(
            ".decl k(x: number)\n.input k\n.decl q(x: number)\n.decl a(x: number)\n.decl a2(x: number)\n.decl o(x: number)\n\
             q(x) :- k(x), x < 3.\nq(x) :- k(x), x > 5.\na(x) :- q(x).\na2(x) :- q(x).\no(x) :- a(x), a2(x).\n.output o\n",
        )
        .unwrap();
        let v = asked(&p, vec![by(&["a"]), by(&["a2"])]);
        let (out, _, _) = run(v, true, None, |_| Ok(())).unwrap();
        assert!(out.rel("a2").is_none_or(|r| out.heads(r).next().is_none()), "a2 merged into a");
        assert_eq!(out.asked()["q"], [by(&["a"]), by(&["a"])]);
    }
}
