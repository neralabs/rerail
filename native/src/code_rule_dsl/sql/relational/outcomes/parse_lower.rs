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
        let op = Op::Parse {
            term,
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
    fn parse_atomic(&mut self, test: &ParseTest) -> usize {
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
    fn admit_parsed_child(&self, plan: &Plan) -> Result<(), String> {
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
            Op::TransientCharge(child) => self.admit_parsed_child(child),
            Op::Charge(..) => Err("parse outcomes: parsed utilities must use transient entry charges".into()),
            Op::Not(child) | Op::FieldNeighbor { child, .. } | Op::FieldEnd { child, .. } => self.admit_parsed_child(child),
            Op::Search(_,_,child,stop) | Op::Count { rule:child, stop, .. } => {
                self.admit_parsed_child(child)?;
                if let Some(stop) = stop { self.admit_parsed_child(stop)?; }
                Ok(())
            },
            Op::Parse { child, .. } => self.admit_parsed_child(child),
            Op::Host{..}|Op::PureRecursive(_)|Op::NthChild{..}=>Ok(()),
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
