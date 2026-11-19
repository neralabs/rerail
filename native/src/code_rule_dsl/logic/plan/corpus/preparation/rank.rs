//! A count below a bound made by rank (decision 0361 step 7), on the version (decision 0281's path): its plan made by
//! `rank_form::plan` over the version's readers and the counted relations' rules, each read as a tree rule, and
//! written into the version.

use super::demand::{about_p, edit_native_rows, is_input, qualified};
use crate::code_rule_dsl::logic::front::{Native, Rule};
use crate::code_rule_dsl::logic::program_image::manager::{Cx, ProgramPass, ProgramW};
use crate::code_rule_dsl::logic::program_image::{About, Cell, RuleId, Tree, Version};
use crate::code_rule_dsl::logic::rank_form::{self, NO_RANK};
use std::collections::{HashMap, HashSet};

type Result<T> = std::result::Result<T, String>;

/// The readers rewritten, and the ranks made.
#[derive(Default)]
pub struct RankForm {
    pub found: usize,
}

/// The version as the form reads it.
struct VersionOf<'v, 't> {
    v: &'v Version,
    about: &'v About,
    tree: &'t mut Tree<'v>,
}

impl rank_form::Of for VersionOf<'_, '_> {
    fn types(&self, rel: &str) -> Option<Vec<String>> {
        self.v.decl_of(rel).map(|d| d.cols().map(|(_, t)| self.v.text(t).to_owned()).collect())
    }
    fn kept(&self, rel: &str) -> bool {
        let (v, about) = (self.v, self.about);
        about.natives.iter().any(|n| v.rel_name(n.rel) == rel)
            || about.lattice.iter().any(|l| v.rel_name(l.rel) == rel)
            || is_input(v, rel)
            || qualified(v, about, rel, NO_RANK)
    }
    fn one_rule(&mut self, rel: &str) -> Option<Rule> {
        let id = self.v.rel(rel)?;
        let mut heads = self.v.heads(id);
        let (one, none) = (heads.next()?, heads.next().is_none());
        none.then(|| self.tree.rule(one))
    }
}

impl ProgramPass for RankForm {
    const NAME: &'static str = "counts below a bound";
    fn run(&mut self, cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<()> {
        if !rank_form::on() {
            return Ok(());
        }
        let v = cx.v();
        let about = About::of(v);
        let lists = Default::default();
        let mut tree = Tree::new(v, &lists);
        // The readers: the rules with a count, read as the tree's.
        let readers: Vec<(RuleId, Rule)> = v
            .rules()
            .filter(|r| r.body().any(|l| matches!(l.view(), crate::code_rule_dsl::logic::program_image::Lit::Agg { .. })))
            .map(|r| (r.id(), tree.rule(r)))
            .filter(|(_, r)| rank_form::counts(r))
            .collect();
        if readers.is_empty() {
            return Ok(());
        }
        let (changed, made) = rank_form::plan(&mut VersionOf { v, about: &about, tree: &mut tree }, &readers);
        self.found += made.found;
        if made.found == 0 {
            return Ok(());
        }
        let changed: HashMap<RuleId, Rule> = changed.into_iter().collect();
        let input = w.input().clone();
        let b = w.derive()?;
        for (rel, cols) in &made.decls {
            let cols: Vec<(&str, &str)> = cols.iter().map(|(n, t)| (n.as_str(), t.as_str())).collect();
            b.declare(rel, &cols);
        }
        for r in input.rules() {
            match changed.get(&r.id()) {
                Some(rule) => {
                    b.add(rule)?;
                }
                None => {
                    b.keep(&input, r);
                }
            }
        }
        for rule in &made.rules {
            b.add(rule)?;
        }
        let p = about_p(b);
        let mut rows = Vec::new();
        for (rel, native) in &made.natives {
            let Native::Rank { query, counted, group, op } = native else { unreachable!("the form makes ranks") };
            rows.push(vec![p, Cell::Str(b.atom(rel)), Cell::Str(b.atom(query)), Cell::Str(b.atom(counted)), Cell::Int(*group as i64), Cell::Str(b.atom(op))]);
        }
        edit_native_rows(b, "native_rank", &HashSet::new(), rows);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    //! The rank form is found by the rules' form alone, whatever their relations are called.
    use super::*;
    use crate::code_rule_dsl::logic::program_image::manager::Pipeline;
    /// The reader's rules after the form, as text.
    fn reader(v: &Version, head: &str) -> String {
        v.rules().filter(|r| v.rel_name(r.head().rel()) == head).map(|r| v.show_rule(r)).collect::<Vec<_>>().join("\n")
    }

    /// `text`'s version through the pass: how many readers it rewrote, and the version.
    fn found(text: &str) -> (usize, std::sync::Arc<Version>) {
        let mut p = Pipeline::new("test", crate::code_rule_dsl::logic::rr::parse_version(text).unwrap());
        let mut pass = RankForm::default();
        p.program(&mut pass);
        (pass.found, p.finish().unwrap().0)
    }

    /// Its natives.
    fn natives(v: &Version) -> std::collections::BTreeMap<String, Native> {
        crate::code_rule_dsl::logic::program_image::fields::ProgramFields::natives(v)
    }

    /// A count of earlier rows of a group, over relations of any names: made by rank, its reader a sum over the rank.
    #[test]
    fn a_count_below_a_bound_is_a_rank_whatever_its_relations() {
        let (n, p) = found(&concat!(
            ".decl edge(p: number, c: number, i: number)\n.decl flag(n: number)\n.decl top(b: number)\n.input edge, flag, top\n",
            ".decl place(b: number, s: number, i: number)\n.output place\n",
            "place(b, s, i) :- top(b), edge(b, s, _), !flag(s), i = count : { edge(b, s2, _), !flag(s2), s2 < s }.\n",
        ));
        assert_eq!(n, 1, "{}", reader(&p, "place"));
        let (rel, native) = natives(&p).into_iter().find(|(_, n)| matches!(n, Native::Rank { .. })).expect("a rank");
        let Native::Rank { group, op, .. } = &native else { unreachable!() };
        assert_eq!((*group, op.as_str()), (1, "<"), "grouped by b, counting below");
        let place = reader(&p, "place");
        assert!(place.contains(&format!("sum")) && place.contains(rel.as_str()), "{place}");
    }

    /// Written the other way round (`x > y`), and counting above (`y > x`): the op as the counted key sees it.
    #[test]
    fn a_rank_takes_its_op_from_the_counted_side() {
        for (cmp, want) in [("s > s2", "<"), ("s2 > s", ">"), ("s2 <= s", "<="), ("s >= s2", "<=")] {
            let (n, p) = found(&format!(concat!(
                ".decl edge(p: number, c: number, i: number)\n.decl top(b: number)\n.input edge, top\n",
                ".decl place(b: number, s: number, i: number)\n.output place\n",
                "place(b, s, i) :- top(b), edge(b, s, _), i = count : {{ edge(b, s2, _), {} }}.\n",
            ), cmp));
            assert_eq!(n, 1, "{cmp}");
            let op = natives(&p).into_values().find_map(|n| if let Native::Rank { op, .. } = n { Some(op) } else { None });
            assert_eq!(op.as_deref(), Some(want), "{cmp}");
        }
    }

    /// The counted rows read an asked variable (`near(s2, s)`): it is in the group, so each asked row is a group of its
    /// own, counted as exactly as before.
    #[test]
    fn a_count_tied_beyond_its_comparison_groups_by_what_ties_it() {
        let (n, p) = found(&concat!(
            ".decl edge(p: number, c: number, i: number)\n.decl near(a: number, b: number)\n.decl top(b: number)\n.input edge, near, top\n",
            ".decl place(b: number, s: number, i: number)\n.output place\n",
            "place(b, s, i) :- top(b), edge(b, s, _), i = count : { edge(b, s2, _), near(s2, s), s2 < s }.\n",
        ));
        assert_eq!(n, 1, "{}", reader(&p, "place"));
        let group = natives(&p).into_values().find_map(|n| if let Native::Rank { group, .. } = n { Some(group) } else { None });
        assert_eq!(group, Some(2), "grouped by b and s");
    }

    /// Not the form: both sides of the comparison are the counted rows' own (`j < s2`), so no bound comes from the
    /// asked rows.
    #[test]
    fn a_comparison_within_the_counted_rows_is_left_as_rows() {
        let (n, p) = found(&concat!(
            ".decl edge(p: number, c: number, i: number)\n.decl top(b: number)\n.input edge, top\n",
            ".decl place(b: number, i: number)\n.output place\n",
            "place(b, i) :- top(b), i = count : { edge(b, s2, j), j < s2 }.\n",
        ));
        assert_eq!(n, 0, "{}", reader(&p, "place"));
        assert!(natives(&p).into_values().all(|n| !matches!(n, Native::Rank { .. })));
    }
}
