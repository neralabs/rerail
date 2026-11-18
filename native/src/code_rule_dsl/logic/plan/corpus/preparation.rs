//! The preparation passes on the pass manager (decision 0281's path, step 5): each a pass over the prepared
//! program's version, run in one pipeline after the program's one image, from the last of the tree's passes
//! backward as they move.

use crate::code_rule_dsl::logic::program_image::manager::{Run, Cx, Pipeline, ProgramPass, ProgramW};
use crate::code_rule_dsl::logic::program_image::Version;
use std::collections::{BTreeSet, HashSet};
use std::sync::Arc;

mod merge;
pub(super) mod combining;
pub(super) mod hide;
pub(crate) mod on_request;
pub(super) mod whole;
pub(crate) mod lowering_tail;
pub mod demand;
mod magic;
mod prune;
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
/// it, the reaches in cycles made rows again, then the text passes. Their decisions reach the run's profile.
pub fn front(version: Arc<Version>, dialect: crate::code_rule_dsl::language::GrammarDialect, last: Option<Run>) -> Result<(Arc<Version>, Front, Run), String> {
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
    let text::Found { matchers, vocabulary } = text::passes(&mut p, dialect)?;
    for d in p.decisions() {
        crate::code_rule_dsl::logic::profile::decision(&d.what, || d.detail.clone());
    }
    let run = p.done()?;
    let demand = own.outcome.map(|o| (own.listed, o));
    Ok((run.output().clone(), Front { reaches: (reach.reaches, reach.throughs), demanded: negated.demanded, demand, undone: undo.undone, matchers, vocabulary }, run))
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
        // Two equal readers merged: the record names the class (`MergeIdentical`'s rename).
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
