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
        if !test.exports.is_empty() || test.host_sound.is_some() {
            return Err(
                "parse outcomes: host hooks and exported spans need a typed contract".into(),
            );
        }
        self.parsed_depth += 1;
        let child_result = self.plan(&test.rule);
        self.parsed_depth -= 1;
        let child = child_result?;
        self.admit_parsed_child(&child)?;
        let term = self.pure.program.terminals.len();
        self.pure.program.terminals.push(Terminal::Fact(FactTerm {
            verdicts: Box::new(super::super::super::parse_facts::ParseAtomic {
                test: test.clone(),
                owner: self.scope_owner,
            }),
            reads: vec![],
        }));
        let op = Op::Parse {
            term,
            child: Box::new(child),
        };
        let mut effects = Effects::of(&op);
        effects.cache_fills |= test.grammar == Grammar::Annotation;
        Ok(Plan {
            effects,
            op,
            mentions: Slots::new(),
            writes: Slots::new(),
            node_writes: Slots::new(),
            walk_filter: None,
        })
    }
    /// Every admitted child must have an implementation for SubDoc identities.
    /// Unsupported capabilities are rejected during lowering, before any host parse runs.
    fn admit_parsed_child(&self, plan: &Plan) -> Result<(), String> {
        match &plan.op {
            Op::Predicate(_) | Op::Unknown(_) | Op::Bind(_) => Ok(()),
            Op::Value(term) => match self.pure.program.terminals.get(*term) {
                Some(Terminal::Fact(fact)) if fact.verdicts.supports_tree_budget() => Ok(()),
                _ => Err("parse outcomes: atomic child has no parsed-node budget contract".into()),
            },
            Op::All(items) | Op::Any(items) => {
                for child in items { self.admit_parsed_child(child)?; }
                Ok(())
            },
            Op::TransientCharge(child) => self.admit_parsed_child(child),
            Op::Charge(..) => Err("parse outcomes: parsed utilities must use transient entry charges".into()),
            Op::Not(child) | Op::FieldNeighbor { child, .. } | Op::FieldEnd { child, .. } => self.admit_parsed_child(child),
            Op::Search(_,_,child,stop) | Op::Count { rule:child, stop, .. } => {
                self.admit_parsed_child(child)?;
                if let Some(stop) = stop { self.admit_parsed_child(stop)?; }
                Ok(())
            },
            Op::Parse { child, .. } => self.admit_parsed_child(child),
            _ => Err("parse outcomes: child requires a parsed document capability that is not implemented".into()),
        }
    }
}
