//! Lowering of the composites a three-valued clause reads verdicts of (`plan`'s contract): `all`
//! with the withholding masks `match_all` reads, `any` with each branch's `may_bind`, and the
//! relations as strict or open searches.

use super::{Bound, Lowerer, capture_free};
use crate::code_rule_dsl::matcher::{AstMatcher, CaptureAnalysis, may_bind, reads_values};
use crate::code_rule_dsl::sql::plan::{AllTerm, Op, Unsupported, Walk};
use crate::code_rule_dsl::sql::recursion;
use std::collections::BTreeSet;

/// Whether matching a rule takes more than testing the node itself: it walks, counts, calls a
/// utility or reads a terminal. A capture-free such rule under a walk is worth a stage.
fn walks(rule: &AstMatcher) -> bool {
    match rule {
        AstMatcher::Kind(_) | AstMatcher::Regex(_) | AstMatcher::Pattern(_) | AstMatcher::Never => {
            false
        }
        AstMatcher::All(items) | AstMatcher::Any(items) => items.iter().any(walks),
        AstMatcher::Not(item) => walks(item),
        _ => true,
    }
}

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
        let mut analysis = CaptureAnalysis::default();
        let mut current = bound.clone();
        let mut held = BTreeSet::new();
        let mut lowered = Vec::with_capacity(items.len());
        for item in items {
            let (op, out) = self.lower(item, scope, &current)?;
            current = out;
            let mentions = analysis.mentions(item);
            let binds = analysis.binds(item);
            let checked = !mentions.is_disjoint(&held);
            let withholds = !item.binds_nothing();
            if checked || (withholds && reads_values(item, &mut BTreeSet::new())) {
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

    /// `match_any`'s branches, each with whether it may bind (`relation::may_bind`).
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
            branches.push((op, may_bind(item)));
            out.extend(branch);
        }
        Ok((Op::AnyOpen(branches), out))
    }

    /// A relation (with or without a field) as `Search` runs it: strict where its rule may bind.
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
        let (sub, out) = self.walked(rule, scope, bound)?;
        self.leaks(&sub)?;
        let binds = sub.binds();
        Ok((
            Op::Search {
                walk,
                sub: Box::new(sub),
                binds,
                strict: may_bind(rule),
            },
            out,
        ))
    }

    /// A walked rule (a relation's or a count's) of a three-valued clause: where it reads, binds
    /// and counts no capture and is more than a node test, its verdict at a node is the node's
    /// alone, so it is a per-batch stage, matched once per node however many walks reach the node
    /// (as the matcher remembers a node-local relation's answers per file, `Related::chains`). A
    /// rule that reaches a recursive utility whose units are being lowered stays in the unit's
    /// body, where its references read the loop's answers.
    pub(super) fn walked(
        &mut self,
        rule: &AstMatcher,
        scope: usize,
        bound: &Bound,
    ) -> Result<(Op, Bound), Unsupported> {
        if capture_free(rule) && walks(rule) && !recursion::reaches(rule, &self.program.in_progress)
        {
            return Ok((Op::Stage(self.stage(rule)?), bound.clone()));
        }
        self.lower(rule, scope, bound)
    }
}
