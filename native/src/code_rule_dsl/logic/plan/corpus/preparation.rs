//! The preparation passes on the pass manager (decision 0281's path, step 5): each a pass over the prepared
//! program's version, run in one pipeline after the program's one image, from the last of the tree's passes
//! backward as they move.

use crate::code_rule_dsl::logic::program_image::manager::{Cx, Pipeline, ProgramPass, ProgramW};
use crate::code_rule_dsl::logic::program_image::Version;
use std::collections::HashSet;
use std::sync::Arc;

mod merge;
pub use merge::{DropRepeated, MergeIdentical};

/// Every head variable of every rule is bound in its body: by a positive atom, a matched pattern or a nearest, a
/// comparison that defines it from bound terms (`Body::cmp_binds`), or an aggregate's result. A variable named only in a
/// negation, or not at all, has no value: the rule's rows are not the rule's meaning, and its finding has no place
/// (decision 0652: a `$$$X` captures nothing, and RUF016's fallback read its `I` from a negation alone).
pub fn head_bound(v: &Version) -> Result<(), String> {
    use crate::code_rule_dsl::logic::plan::body::{Body, VarSet, lit_vars, term_vars};
    use crate::code_rule_dsl::logic::program_image::Lit as L;
    let mut wrong = Vec::new();
    for rule in v.rules() {
        // An aggregate's context (`rr_ctx…`) names the outer rule's variables, some of which the outer rule binds.
        if crate::code_rule_dsl::logic::rr::aggregate_demand::helper(v.rel_name(rule.head().rel())) {
            continue;
        }
        let lits: Vec<_> = rule.body().collect();
        let mut bound = VarSet::new();
        let mut rest = Vec::new();
        for l in &lits {
            match l.view() {
                L::Atom(a) if !a.neg() => bound.extend(&lit_vars(*l)),
                L::Nearest { .. } | L::Ast { .. } => bound.extend(&lit_vars(*l)),
                L::Agg { var, .. } => {
                    bound.insert(var);
                }
                L::Cmp { .. } => rest.push(*l),
                L::Atom(_) => {}
            }
        }
        let body = Body::of(v, rule);
        loop {
            let before = rest.len();
            rest.retain(|l| match l.view() {
                // `x = t` or `t = x`: either side defines the other.
                L::Cmp { l: a, op, r } => match body.cmp_binds(a, op, r, &bound).or_else(|| body.cmp_binds(r, op, a, &bound)) {
                    Some(new) => {
                        bound.extend(&new);
                        false
                    }
                    None => true,
                },
                _ => false,
            });
            if rest.len() == before {
                break;
            }
        }
        let mut head = VarSet::new();
        rule.head().args().for_each(|t| term_vars(t, &mut head));
        let free: Vec<&str> = head.minus(&bound).iter().map(|s| v.text(rule.var(s))).collect();
        if !free.is_empty() {
            wrong.push(format!("  {}: {} bound by no positive literal\n      {}", v.rel_name(rule.head().rel()), free.iter().map(|x| format!("${x}")).collect::<Vec<_>>().join(", "), v.show_rule(rule).chars().take(400).collect::<String>()));
        }
    }
    if wrong.is_empty() {
        return Ok(());
    }
    Err(format!("{} rule(s) leave a head variable unbound: give each a value in a positive literal, or leave it out of the head:\n{}", wrong.len(), wrong.join("\n")))
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

/// The passes that follow the program's image, over `version`: the program that runs, a base (what the store keeps),
/// with what they found; the pipeline's stages traced. The joins pushed are made per rule, so two programs' copies
/// of one library rule made two: after the rows are located, they are merged again, and so are the relations over
/// them, before the program splits. The merges (`merge`) run as `corpus::merge_and_drop_repeats`
/// runs them, or the merge alone with `RERAIL_SAME=off`.
pub fn run(version: Arc<Version>, factorize: bool) -> Result<(Arc<Version>, Found), String> {
    // Every head variable has a value (decision 0652).
    head_bound(&version)?;
    let mut p = Pipeline::new("preparation", version);
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
    let found = Found { pushed: pushed.made, factorized: factorized.lines, demand: demand.lines, located, merged: merge.merged, shared: merge.shared, repeated: repeats.0 };
    p.program(&mut InputsRead);
    if let Ok(order) = std::env::var("RERAIL_RULE_ORDER") {
        p.program(&mut RuleOrder(order));
    }
    let (out, stages) = p.finish()?;
    for s in &stages {
        crate::code_rule_dsl::logic::profile::decision("preparation pass (decision 0281)", || format!("{}: {:?}, {:?}", s.pass, s.ran, s.form));
    }
    // The store keeps a base's bytes (`program_text::encode_version`): an edit's version is folded into one.
    let out = match out.base_bytes() {
        Some(_) => out,
        None => out.fold()?,
    };
    Ok((out, found))
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
        let (out, _) = run(v.clone(), true).unwrap();
        assert!(out.base_bytes().is_some(), "a base");
        let inputs: Vec<&str> = out.inputs().map(|a| out.text(a)).collect();
        assert_eq!(inputs, ["a", "c"], "b is read by no rule; c is an output");
        let mut pl = Pipeline::new("test", v);
        pl.program(&mut RuleOrder("reverse".into()));
        let (rev, _) = pl.finish().unwrap();
        let heads: Vec<&str> = rev.rules().map(|r| rev.rel_name(r.head().rel())).collect();
        assert_eq!(heads, ["q", "p"]);
    }
}
