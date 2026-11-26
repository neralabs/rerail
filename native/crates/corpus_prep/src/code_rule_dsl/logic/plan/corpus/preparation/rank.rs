//! A count below a bound made by rank (decision 0361 step 7), on the version (decision 0281's path): its plan made by
//! `rank_form::plan` over the version's readers and the counted relations' rules, read in the image, and written
//! into a derivation of the version (`rank_form::write`).

use super::demand::{about_p, edit_native_rows, is_input, qualified};
use crate::code_rule_dsl::logic::program_image::manager::{Cx, ProgramPass, ProgramW};
use crate::code_rule_dsl::logic::program_image::{About, Cell, Rows, Version};
use crate::code_rule_dsl::logic::rank_form::{self, NO_RANK};
use std::collections::HashSet;

type Result<T> = std::result::Result<T, String>;

/// The readers rewritten, and the ranks made.
#[derive(Default)]
pub struct RankForm {
    pub found: usize,
}

/// The version as the form reads it.
struct VersionOf<'v> {
    v: &'v Version,
    about: &'v About,
}

impl rank_form::Of for VersionOf<'_> {
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
}

impl ProgramPass for RankForm {
    const NAME: &'static str = "counts below a bound";
    fn run(&mut self, _cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<()> {
        if !rank_form::on() {
            return Ok(());
        }
        let input = w.input().clone();
        let v: &Version = &input;
        let about = About::of(v);
        let of = VersionOf { v, about: &about };
        let plan = rank_form::plan(v, &of);
        let found = plan.found();
        self.found += found;
        if found == 0 {
            return Ok(());
        }
        let b = w.derive()?;
        let made = rank_form::write(v, &of, &plan, b)?;
        let p = about_p(b);
        let mut rows = Rows::new();
        for (rel, query, counted, group, op) in &made {
            rows.push([p, Cell::Str(b.atom(rel)), Cell::Str(b.atom(query)), Cell::Str(b.atom(counted)), Cell::Int(*group as i64), Cell::Str(b.atom(op))]);
        }
        edit_native_rows(b, "native_rank", &HashSet::new(), rows);
        Ok(())
    }
}

// cratify: `tests` is rerail-logic's (it mounts it: it reads the language, `rr::parse_version`, or the corpus's root)
