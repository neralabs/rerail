//! A count below a bound made by rank (decision 0361 step 7), on the version (decision 0281's path): the tree's
//! `rank_form::find`, its plan made by the same function (`rank_form::plan`) over the version's readers and the
//! counted relations' rules, each read as a tree rule, and written into the version.

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
    use super::*;
    use crate::code_rule_dsl::logic::program_image::Builder;
    use crate::code_rule_dsl::logic::program_image::manager::Pipeline;

    /// The version's pass makes what the tree's makes: the same image, the reader a sum over the rank.
    #[test]
    fn the_versions_rank_form_is_the_trees() {
        let text = concat!(
            ".decl edge(p: number, c: number, i: number)\n.decl flag(n: number)\n.decl top(b: number)\n.input edge, flag, top\n",
            ".decl place(b: number, s: number, i: number)\n.output place\n",
            "place(b, s, i) :- top(b), edge(b, s, _), !flag(s), i = count : { edge(b, s2, _), !flag(s2), s2 < s }.\n",
        );
        let mut tree = crate::code_rule_dsl::logic::rr::parse(text).unwrap();
        let (v, _) = Builder::of_program(&tree).unwrap().finish().unwrap();
        assert_eq!(rank_form::find(&mut tree), 1);
        let mut p = Pipeline::new("test", v);
        let mut pass = RankForm::default();
        p.program(&mut pass);
        assert_eq!(pass.found, 1);
        let out = p.current().unwrap().clone();
        let (want, _) = Builder::of_program(&tree).unwrap().finish().unwrap();
        assert_eq!((out.size(), out.digest()), (want.size(), want.digest()), "{}", crate::code_rule_dsl::logic::program_image::differences(&want, &out));
    }
}
