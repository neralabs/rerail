//! Lowering of the composites a three-valued clause reads verdicts of (`plan`'s contract): `all`
//! with the withholding masks `match_all` reads, `any` with each branch's `may_bind`, and the
//! relations as strict or open searches.

use super::{Bound, Lowerer};
use crate::code_rule_dsl::matcher::AstMatcher;
use crate::code_rule_dsl::sql::plan::{AllTerm, Op, Unsupported, Walk};
use std::collections::BTreeSet;

impl Lowerer<'_> {
    /// `match_all`'s terms with what it withholds by: each term's mention and bind masks, whether
    /// an unknown verdict of it withholds (`!binds_nothing`), and whether a row can reach it with
    /// a capture it mentions withheld. A capture gets a mask bit where some term may withhold it:
    /// one that can be unknown (`reads_values`) and binds, or that is withheld itself.
    pub(super) fn all_open(
        &mut self,
        items: &[AstMatcher],
        scope: usize,
        bound: &Bound,
    ) -> Result<(Op, Bound), Unsupported> {
        let analysis = &mut self.program.analysis;
        let mut analyses = Vec::with_capacity(items.len());
        for item in items {
            analyses.push((
                analysis.captures.mentions(item),
                analysis.captures.binds(item),
                !item.binds_nothing() && analysis.reads_values(item),
            ));
        }
        let mut current = bound.clone();
        let mut held = BTreeSet::new();
        let mut lowered = Vec::with_capacity(items.len());
        for (item, (mentions, binds, unknown_binds)) in items.iter().zip(analyses) {
            let (op, out) = self.lower(item, scope, &current)?;
            current = out;
            let checked = !mentions.is_disjoint(&held);
            let withholds = !item.binds_nothing();
            if checked || unknown_binds {
                held.extend(binds.iter().cloned());
            }
            lowered.push((op, mentions, binds, withholds, checked));
        }
        for name in &held {
            self.bit(scope, name);
        }
        let terms = lowered
            .into_iter()
            .map(|(op, mentions, binds, withholds, checked)| AllTerm {
                op,
                mentions: self.mask(scope, &mentions),
                binds: self.mask(scope, &binds),
                withholds,
                checked,
            })
            .collect();
        Ok((Op::AllOpen(terms), current))
    }

    /// `match_any`'s branches, each with whether it may bind (`relation::may_bind`); under
    /// `violation.each`, each branch excluding (`emit/each.rs`).
    pub(super) fn any_open(
        &mut self,
        items: &[AstMatcher],
        scope: usize,
        bound: &Bound,
    ) -> Result<(Op, Bound), Unsupported> {
        let mut branches = Vec::with_capacity(items.len());
        let mut out = bound.clone();
        for item in items {
            let (op, branch) = self.lower(item, scope, bound)?;
            let may_bind = self.program.analysis.may_bind(item);
            branches.push((self.excluding(scope, op), may_bind));
            out.extend(branch);
        }
        Ok((Op::AnyOpen(branches), out))
    }

    /// A relation (with or without a field) as `Search` runs it: strict where its rule may bind;
    /// under `violation.each`, its rule excluding (`emit/each.rs`).
    pub(super) fn search(
        &mut self,
        matcher: &AstMatcher,
        scope: usize,
        bound: &Bound,
    ) -> Result<(Op, Bound), Unsupported> {
        let (walk, rule) = match matcher {
            AstMatcher::Field(relation) => {
                let stop = self.stop(&relation.stop)?;
                let walk = if relation.inside {
                    Walk::FieldInside(relation.field, stop)
                } else {
                    Walk::FieldHas(relation.field, stop)
                };
                (walk, &relation.rule)
            }
            _ => {
                let (walk, related) = self.relation(matcher)?.expect("a relation");
                (walk, related.rule())
            }
        };
        let (sub, out) = self.lower(rule, scope, bound)?;
        self.leaks(&sub)?;
        let sub = self.excluding(scope, sub);
        let binds = sub.binds();
        Ok((
            Op::Search {
                walk,
                sub: Box::new(sub),
                binds,
                strict: self.program.analysis.may_bind(rule),
                kinds: self.walk_kinds(rule),
            },
            out,
        ))
    }
}
