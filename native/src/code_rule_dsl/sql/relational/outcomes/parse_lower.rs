//! A parse boundary starts a fresh child environment and retains SQL composition.
use super::super::{Op, Plan, Slots, effects::Effects};
use super::Lower;
use crate::code_rule_dsl::{
    literal::Grammar,
    matcher::ParseTest,
    sql::terminals::{FactTerm, Terminal},
};

impl Lower {
    pub(super) fn parsed(&mut self, test: &ParseTest) -> Result<Plan, String> {
        let private = !test.exports.is_empty()
            || test.host_sound.is_some()
            || !crate::code_rule_dsl::matcher::CaptureAnalysis::default()
                .mentions(&test.rule)
                .is_empty();
        let outer_namespace = self.namespace.clone();
        let allocated = private.then(|| self.parse_atomic(test));
        if let Some(term) = allocated {
            let child_namespace = format!("{outer_namespace}@parse{term}:");
            self.parse_hosts.push(super::parse_host_lower::Scope {
                term,
                outer_namespace: outer_namespace.clone(),
                child_namespace: child_namespace.clone(),
            });
            self.namespace = child_namespace;
        }
        self.parsed_depth += 1;
        let child_result = self.plan(&test.rule);
        self.parsed_depth -= 1;
        self.namespace = outer_namespace;
        if private {
            self.parse_hosts.pop();
        }
        let term = allocated.unwrap_or_else(|| self.parse_atomic(test));
        let child = child_result?;
        self.admit_parsed_child(&child)?;
        let mut exports = Vec::new();
        for name in &test.exports {
            let qualified = format!("{}@parse{term}:{name}", self.namespace);
            let Some(inner) = self.slots.iter().position(|slot| {
                slot.name == qualified && slot.kind == super::super::CaptureKind::Node
            }) else {
                continue;
            };
            let outer = self.slot(name, super::super::CaptureKind::Node)?;
            self.span_slots.insert(outer);
            exports.push((inner, outer));
        }
        let writes: Slots = exports.iter().map(|(_, outer)| *outer).collect();
        let mut mentions = writes.clone();
        let child_prefix = format!("{}@parse{term}:", self.namespace);
        mentions.extend(
            child
                .mentions
                .iter()
                .copied()
                .filter(|slot| !self.slots[*slot].name.starts_with(&child_prefix)),
        );
        let expensive = contains_parsed_recursion(&child, &mut std::collections::HashSet::new());
        // Literal-only native candidate populations do not benefit from a host
        // kind dispatch. Broader populations reject cheap nonhosts before opening
        // the parser; literal-only populations defer setup until parsing succeeds.
        let setup = if !expensive {
            super::super::ParseSetup::Plain
        } else if self.parsed_depth == 0
            && self.hosted_depth == 0
            && self
                .root_kinds
                .as_ref()
                .is_some_and(|kinds| kinds.is_subset(&test.host_kinds))
        {
            super::super::ParseSetup::Parsed
        } else {
            super::super::ParseSetup::HostKinds(test.host_kinds.iter().copied().collect())
        };
        let op = Op::Parse {
            term,
            setup,
            child: Box::new(child),
            exports,
            hosted: test.host_sound.is_some(),
        };
        let mut effects = Effects::of(&op);
        effects.cache_fills |= test.grammar == Grammar::Annotation;
        Ok(Plan {
            effects,
            op,
            mentions,
            node_writes: writes.clone(),
            writes,
            walk_filter: None,
        })
    }
    pub(super) fn parse_atomic(&mut self, test: &ParseTest) -> usize {
        let term = self.pure.program.terminals.len();
        self.pure.program.terminals.push(Terminal::Fact(FactTerm {
            verdicts: Box::new(super::super::super::parse_facts::ParseAtomic {
                test: test.clone(),
                owner: self.scope_owner,
            }),
            reads: vec![],
        }));
        term
    }
    /// Every admitted child must have an implementation for SubDoc identities.
    /// Unsupported capabilities are rejected during lowering, before any host parse runs.
    pub(super) fn admit_parsed_child(&self, plan: &Plan) -> Result<(), String> {
        match &plan.op {
            Op::Predicate(_) | Op::Unknown(_) | Op::Bind(_) | Op::CaptureBind(_) | Op::Pattern(..) => Ok(()),
            Op::Value(term) => match self.pure.program.terminals.get(*term) {
                Some(Terminal::Fact(fact)) if fact.verdicts.supports_tree_budget() => Ok(()),
                _ => Err("parse outcomes: atomic child has no parsed-node budget contract".into()),
            },
            Op::All(items) | Op::Any(items) => {
                for child in items { self.admit_parsed_child(child)?; }
                Ok(())
            },
            Op::TransientCharge(child) | Op::Shared(child) => self.admit_parsed_child(child),
            Op::Charge(..) | Op::MemoPredicate(..) => Err("parse outcomes: parsed utilities must use transient entry charges".into()),
            Op::FieldRule { child,stop,.. } => { self.admit_parsed_child(child)?; self.admit_parsed_child(stop) },
            Op::Not(child) | Op::FieldNeighbor { child, .. } | Op::FieldEnd { child, .. } => self.admit_parsed_child(child),
            Op::Search(_,_,child,stop) | Op::Count { rule:child, stop, .. } => {
                self.admit_parsed_child(child)?;
                if let Some(stop) = stop { self.admit_parsed_child(stop)?; }
                Ok(())
            },
            Op::Parse { child, .. } => self.admit_parsed_child(child),
            Op::Host{..}|Op::PureRecursive(_)|Op::NthChild{..}|Op::ParsedRecursive{..}=>Ok(()),
            Op::CallFrame{body,..}=>self.admit_parsed_child(body),
            Op::MixedRecursive{program,..}=>{
                if !program.bypass{return Err("parse outcomes: recursive control must bypass native utility memo on parsed documents".into());}
                for region in &program.regions{self.admit_parsed_child(region)?;}Ok(())
            },
            Op::BoundLeaf{term,..}=>match self.pure.program.terminals.get(*term) {
                Some(Terminal::Fact(fact)) if fact.verdicts.supports_tree_rows()=>Ok(()),
                _=>Err("parse outcomes: typed atomic child has no parsed contract".into()),
            },
            Op::Constrained(core,constraints)=>{
                self.admit_parsed_child(core)?;for (_,child) in constraints{self.admit_parsed_child(child)?;}Ok(())
            },
            _ => Err("parse outcomes: child requires a parsed document capability that is not implemented".into()),
        }
    }
}

/// A recursive parsed child has dynamic document/continuation setup worth
/// deferring until the authoritative atomic parse has returned a tree.
fn contains_parsed_recursion(plan: &Plan, seen: &mut std::collections::HashSet<usize>) -> bool {
    if !seen.insert(std::ptr::from_ref(plan) as usize) {
        return false;
    }
    match &plan.op {
        Op::ParsedRecursive { .. } => true,
        Op::All(items) | Op::Any(items) => items.iter().any(|p| contains_parsed_recursion(p, seen)),
        Op::Charge(_, p)
        | Op::MemoPredicate(_, p)
        | Op::TransientCharge(p)
        | Op::Shared(p)
        | Op::CallFrame { body: p, .. } => contains_parsed_recursion(p, seen),
        Op::Not(p)
        | Op::Parse { child: p, .. }
        | Op::Host { child: p, .. }
        | Op::FieldNeighbor { child: p, .. }
        | Op::FieldEnd { child: p, .. } => contains_parsed_recursion(p, seen),
        Op::FieldRule { child, stop, .. } => {
            contains_parsed_recursion(child, seen) || contains_parsed_recursion(stop, seen)
        }
        Op::Search(_, _, child, stop)
        | Op::Count {
            rule: child, stop, ..
        } => {
            contains_parsed_recursion(child, seen)
                || stop
                    .as_ref()
                    .is_some_and(|p| contains_parsed_recursion(p, seen))
        }
        Op::Constrained(core, terms) => {
            contains_parsed_recursion(core, seen)
                || terms
                    .iter()
                    .any(|(_, p)| contains_parsed_recursion(p, seen))
        }
        Op::MixedRecursive { program, .. } => program
            .regions
            .iter()
            .any(|p| contains_parsed_recursion(p, seen)),
        Op::RecursiveSearch { component, .. } => contains_parsed_recursion(&component.base, seen),
        Op::FormatArgs {
            unused, missing, ..
        } => [unused, missing]
            .into_iter()
            .flatten()
            .any(|p| contains_parsed_recursion(p, seen)),
        Op::Binding {
            core,
            children,
            occurrence,
            ..
        } => {
            contains_parsed_recursion(core, seen)
                || children
                    .iter()
                    .any(|(_, p)| contains_parsed_recursion(p, seen))
                || occurrence
                    .as_ref()
                    .is_some_and(|p| contains_parsed_recursion(p, seen))
        }
        _ => false,
    }
}

#[cfg(test)]
mod setup_tests {
    use super::super::super::ParseSetup;
    use super::*;
    use crate::code_rule_dsl::{matcher::Semantic, test_support::grammar_matchers};
    use serde_json::json;
    #[test]
    fn native_anchor_cost_choice_stays_in_original_document_context() {
        let policy = super::super::super::parse_tests::policy(
            "python",
            json!({"matches":"walk"}),
            json!({"walk":{"parse":{"grammar":"annotation","has":{"stopBy":"end","any":[{"kind":"identifier","capture":"T"},{"all":[{"kind":"string"},{"matches":"walk"}]}]}}}}),
        );
        let grammar = &grammar_matchers(&policy)[0];
        let crate::code_rule_dsl::matcher::AstMatcher::Util(util) = &grammar.matcher else {
            panic!("utility fixture");
        };
        let hosts = util
            .with_target(|body| match body {
                crate::code_rule_dsl::matcher::AstMatcher::Semantic(Semantic::Parse(test)) => {
                    test.host_kinds.as_ref().clone()
                }
                _ => panic!("parse fixture"),
            })
            .unwrap();
        let select = |parsed_depth, hosted_depth, kinds| {
            util.with_target(|body| {
                let crate::code_rule_dsl::matcher::AstMatcher::Semantic(Semantic::Parse(test)) =
                    body
                else {
                    panic!("parse fixture");
                };
                let mut lower = Lower::new(grammar.language, grammar.three_valued);
                lower.root_kinds = kinds;
                lower.parsed_depth = parsed_depth;
                lower.hosted_depth = hosted_depth;
                match lower.parsed(test).unwrap().op {
                    Op::Parse { setup, .. } => setup,
                    _ => panic!("parse plan"),
                }
            })
            .unwrap()
        };
        assert!(matches!(
            select(0, 0, Some(hosts.clone())),
            ParseSetup::Parsed
        ));
        assert!(matches!(select(0, 0, None), ParseSetup::HostKinds(_)));
        let mut broad = hosts.clone();
        broad.insert(u16::MAX);
        assert!(matches!(
            select(0, 0, Some(broad)),
            ParseSetup::HostKinds(_)
        ));
        // Equal u16 values in another document do not establish kind identity.
        assert!(matches!(
            select(1, 0, Some(hosts.clone())),
            ParseSetup::HostKinds(_)
        ));
        assert!(matches!(
            select(0, 1, Some(hosts)),
            ParseSetup::HostKinds(_)
        ));
    }
}
