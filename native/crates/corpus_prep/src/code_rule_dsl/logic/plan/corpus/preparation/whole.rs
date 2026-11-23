//! The passes on the whole combined program, once its parts are linked (decision 0281's path, step 7a-1), on its
//! version: the combine's tail (`corpus::tail_version`).

use crate::code_rule_dsl::logic::program_image::manager::{Cx, Last, ProgramPass, ProgramW, Reads, edit_as_is};
use crate::code_rule_dsl::logic::program_image::{Lit, RuleRef, TermRef, Version};
use std::collections::BTreeMap;

type Result<T> = std::result::Result<T, String>;

/// Whether an edit of rules alone (no header) neither removes nor adds a rule `heads` names: a pass deciding from
/// those heads' rules and the header decides as it did (decision 0385).
fn edit_misses(cx: &Cx<'_>, last: &Last<'_>, heads: impl Fn(&str) -> bool) -> bool {
    let v = cx.v();
    let of = |at: &Version, r: RuleRef<'_>| heads(at.rel_name(r.head().rel()));
    last.delta.touched == Reads::RULES
        && last.delta.removed.iter().all(|id| last.input.rule(id).is_some_and(|r| !of(last.input, r)))
        && last.delta.added.iter().all(|&id| v.rule(id).is_some_and(|r| !of(v, r)))
}

/// core.rerail's `kind(n, k)` and `docof(n, d)` are projections of `node` in a part that reads them (`rr::lower`'s
/// `projections`) and inputs in one that does not: once one part projects them, they are no inputs of the combined
/// program. Both would give `kind` two domains.
#[derive(Default)]
pub struct Projected;

impl ProgramPass for Projected {
    const NAME: &'static str = "projected inputs";
    /// Over an edit (decision 0385): the edit as it is, where it has no rule of `kind` or `docof` and leaves the
    /// header (the inputs) as it was.
    fn again(&mut self, cx: &Cx<'_>, last: &Last<'_>, w: &mut ProgramW<'_>) -> Result<bool> {
        if !edit_misses(cx, last, |h| h == "kind" || h == "docof") {
            return Ok(false);
        }
        edit_as_is(cx, last, w)
    }
    fn run(&mut self, cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<()> {
        let v = cx.v();
        let projected: Vec<&str> = ["kind", "docof"]
            .into_iter()
            .filter(|rel| {
                v.rel(rel).is_some_and(|id| {
                    v.heads(id).any(|r| {
                        let mut body = r.body();
                        matches!((body.next().map(|l| l.view()), body.next()), (Some(Lit::Atom(a)), None) if v.rel_name(a.rel()) == "node")
                    })
                })
            })
            .collect();
        let inputs: Vec<String> = v.inputs().map(|a| v.text(a).to_owned()).collect();
        if inputs.iter().any(|i| projected.contains(&i.as_str())) {
            let kept: Vec<String> = inputs.into_iter().filter(|i| !projected.contains(&i.as_str())).collect();
            w.on()?.fields().set_inputs(&kept);
        }
        Ok(())
    }
}

/// The helpers' columns typed again once the library's declarations are known (the combined program).
///
/// A program is lowered without the library: a helper column bound only by a library relation (`kind(n, …)`,
/// `child(p, n, _)`) finds no declaration then, and is a symbol by default. It holds nodes, which the library says;
/// and a column read through it (a demand's, `rr_demand_agg_…`, copied from the aggregate's) is then a symbol beside
/// the node the rule's own atoms make it, which no native plan compares. A helper's symbol column every rule fills
/// from one typed column takes that type; a column a symbol column binds stays one.
#[derive(Default)]
pub struct RetypeHelpers;

impl ProgramPass for RetypeHelpers {
    const NAME: &'static str = "retype helpers";
    /// Over an edit (decision 0385): the edit as it is, where it has no helper's rule and leaves the declarations as
    /// they were; a helper's type is read from its rules and the declarations alone.
    fn again(&mut self, cx: &Cx<'_>, last: &Last<'_>, w: &mut ProgramW<'_>) -> Result<bool> {
        use crate::code_rule_dsl::logic::rr::aggregate_demand::helper;
        if !edit_misses(cx, last, helper) {
            return Ok(false);
        }
        edit_as_is(cx, last, w)
    }
    fn run(&mut self, cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<()> {
        use crate::code_rule_dsl::logic::rr::aggregate_demand::helper;
        let v = cx.v();
        let mut decls: BTreeMap<String, Vec<(String, String)>> =
            v.decls().into_iter().map(|d| (v.text(d.name()).to_owned(), d.cols().map(|(n, t)| (v.text(n).to_owned(), v.text(t).to_owned())).collect())).collect();
        let mut retyped: std::collections::BTreeSet<String> = Default::default();
        let mut changed = true;
        while changed {
            changed = false;
            let helpers: Vec<String> = decls.keys().filter(|r| helper(r)).cloned().collect();
            for rel in helpers {
                let width = decls[&rel].len();
                for i in 0..width {
                    if decls[&rel][i].1 != "sym" {
                        continue;
                    }
                    let (mut found, mut symbol) = (None::<String>, false);
                    for rule in v.rel(&rel).into_iter().flat_map(|id| v.heads(id)) {
                        let Some(TermRef::Var(x)) = rule.head().args().nth(i) else {
                            symbol = true;
                            continue;
                        };
                        for l in rule.body() {
                            match l.view() {
                                Lit::Atom(a) if !a.neg() => {
                                    let name = v.rel_name(a.rel());
                                    for (j, t) in a.args().enumerate() {
                                        if t.var() != Some(x) {
                                            continue;
                                        }
                                        let ty = if crate::code_rule_dsl::logic::front::is_axis(name) {
                                            Some("node".to_owned())
                                        } else {
                                            decls.get(name).and_then(|c| c.get(j)).map(|(_, t)| t.clone())
                                        };
                                        match ty.as_deref() {
                                            Some("sym") if !helper(name) => symbol = true,
                                            Some("sym") | None => {}
                                            Some(t) => found = Some(t.to_owned()),
                                        }
                                    }
                                }
                                Lit::Nearest { n, a, .. } if n == x || a == x => found = Some("node".to_owned()),
                                _ => {}
                            }
                        }
                    }
                    if let (Some(ty), false) = (found, symbol) {
                        decls.get_mut(&rel).expect("a helper")[i].1 = ty;
                        retyped.insert(rel.clone());
                        changed = true;
                    }
                }
            }
        }
        if !retyped.is_empty() {
            let b = w.on()?;
            for rel in &retyped {
                let cols: Vec<(&str, &str)> = decls[rel].iter().map(|(n, t)| (n.as_str(), t.as_str())).collect();
                b.fields().declare(rel, &cols);
            }
        }
        Ok(())
    }
}
