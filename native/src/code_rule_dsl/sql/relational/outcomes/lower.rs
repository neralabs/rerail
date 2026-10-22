use super::{Axis, Op, Plan, Slot, Slots};
use crate::code_rule_dsl::{
    language::GrammarDialect,
    matcher::{AstMatcher, Semantic, StopBy},
};
use ast_grep_core::{matcher::PatternNode, meta_var::MetaVariable};
use std::collections::HashSet;

pub(super) struct Lower {
    pub pure: super::super::lower::Lowerer,
    pub slots: Vec<Slot>,
    active: HashSet<(usize, usize)>,
    operators: usize,
}

impl Lower {
    pub fn new(dialect: GrammarDialect) -> Self {
        Self {
            pure: super::super::lower::Lowerer::new(dialect),
            slots: vec![],
            active: HashSet::new(),
            operators: 0,
        }
    }

    fn slot(&mut self, name: &str, count: bool) -> Result<usize, String> {
        if let Some(id) = self.slots.iter().position(|slot| slot.name == name) {
            if self.slots[id].count != count {
                return Err("outcome relations: capture and count names must be distinct".into());
            }
            return Ok(id);
        }
        // The bound is structural, not a machine-word mask. Wider contracts need a measured cut.
        if self.slots.len() >= 128 {
            return Err("outcome relations: binding width exceeds 128 columns".into());
        }
        let id = self.slots.len();
        self.slots.push(Slot {
            name: name.into(),
            count,
        });
        Ok(id)
    }

    fn walk<'a>(
        &self,
        matcher: &'a AstMatcher,
    ) -> Result<(Axis, &'a StopBy, &'a AstMatcher), String> {
        let (axis, rule, stop) = match matcher {
            AstMatcher::Inside(r, s) => (Axis::Parent, r, s),
            AstMatcher::Has(r, s) => (Axis::Child, r, s),
            AstMatcher::Follows(r, s) => (Axis::Previous, r, s),
            AstMatcher::Precedes(r, s) => (Axis::Next, r, s),
            _ => return Err("outcome relations: count requires a structural walk".into()),
        };
        Ok((axis, stop, rule.rule()))
    }

    fn stop(&mut self, by: &StopBy) -> Result<Option<Box<Plan>>, String> {
        match by {
            StopBy::Rule(rule) => Ok(Some(Box::new(self.plan(rule)?))),
            _ => Ok(None),
        }
    }

    pub fn plan(&mut self, matcher: &AstMatcher) -> Result<Plan, String> {
        self.operators += 1;
        if self.operators > 256 {
            return Err("outcome relations: expanded operator count exceeds 256".into());
        }
        let mut mentions = Slots::new();
        let mut writes = Slots::new();
        let mut node_writes = Slots::new();
        let op = match matcher {
            AstMatcher::Kind(_) | AstMatcher::Regex(_) | AstMatcher::Never => {
                Op::Predicate(self.pure.set(matcher)?)
            }
            AstMatcher::Pattern(pattern) => {
                if let PatternNode::MetaVar {
                    meta_var: MetaVariable::Capture(name, true),
                } = &pattern.node
                {
                    if !matches!(pattern.strictness, ast_grep_core::MatchStrictness::Smart) {
                        return Err(
                            "outcome relations: whole captures require Smart matching".into()
                        );
                    }
                    let slot = self.slot(name, false)?;
                    mentions.insert(slot);
                    writes.insert(slot);
                    node_writes.insert(slot);
                    Op::Bind(slot)
                } else {
                    Op::Predicate(self.pure.set(matcher)?)
                }
            }
            AstMatcher::Semantic(Semantic::Value(test))
                if !test.reading.follow && !test.reading.literal =>
            {
                use crate::code_rule_dsl::sql::terminals::{FactTerm, Leaf, Terminal};
                let id = self.pure.program.terminals.len();
                self.pure.program.terminals.push(Terminal::Fact(FactTerm {
                    verdicts: Box::new(Leaf(matcher.clone())),
                    reads: vec![],
                }));
                Op::Value(id)
            }
            AstMatcher::TestUnknown(regex) => {
                let super::super::ir::Set::Regex(term) =
                    self.pure.set(&AstMatcher::Regex(regex.clone()))?
                else {
                    unreachable!()
                };
                Op::Unknown(term)
            }
            AstMatcher::All(items) | AstMatcher::Any(items) => {
                let children = items
                    .iter()
                    .map(|m| self.plan(m))
                    .collect::<Result<Vec<_>, _>>()?;
                for child in &children {
                    mentions.extend(&child.mentions);
                    writes.extend(&child.writes);
                    node_writes.extend(&child.node_writes);
                }
                if matches!(matcher, AstMatcher::All(_)) {
                    Op::All(children)
                } else {
                    Op::Any(children)
                }
            }
            AstMatcher::Not(inner) => {
                let child = self.plan(inner)?;
                mentions = child.mentions.clone();
                Op::Not(Box::new(child))
            }
            AstMatcher::Inside(..)
            | AstMatcher::Has(..)
            | AstMatcher::Follows(..)
            | AstMatcher::Precedes(..) => {
                let (axis, by, rule) = self.walk(matcher)?;
                let end = !matches!(by, StopBy::Neighbor);
                let stop = self.stop(by)?;
                let child = self.plan(rule)?;
                mentions = child.mentions.clone();
                writes = child.writes.clone();
                node_writes = child.node_writes.clone();
                Op::Search(axis, end, Box::new(child), stop)
            }
            AstMatcher::Count(count) => {
                let (axis, by, rule) = self.walk(&count.relation)?;
                let end = !matches!(by, StopBy::Neighbor);
                let stop = self.stop(by)?;
                let child = self.plan(rule)?;
                mentions = child.mentions.clone();
                let binding = if let Some(binding) = &count.binding {
                    binding.offset.checked_add(u32::MAX as i64).ok_or(
                        "outcome relations: count offset can exceed signed integer storage",
                    )?;
                    let slot = self.slot(&binding.name, true)?;
                    writes.insert(slot);
                    Some((slot, binding.offset))
                } else {
                    None
                };
                Op::Count {
                    axis,
                    end,
                    rule: Box::new(child),
                    stop,
                    min: i64::try_from(count.min).map_err(
                        |_| "outcome relations: count minimum exceeds signed integer storage",
                    )?,
                    max: count.max.map(i64::try_from).transpose().map_err(
                        |_| "outcome relations: count maximum exceeds signed integer storage",
                    )?,
                    binding,
                }
            }
            AstMatcher::Util(util) => {
                let key = (util.table.as_ptr() as usize, util.index);
                if !self.active.insert(key) {
                    return Err(
                        "outcome relations: recursive outcomes need a component contract".into(),
                    );
                }
                let result = util
                    .with_target(|rule| self.plan(rule))
                    .ok_or("outcome relations: missing utility")?;
                self.active.remove(&key);
                return result;
            }
            _ => return Err(
                "outcome relations: leaf or operator lacks a total, effect-free outcome contract"
                    .into(),
            ),
        };
        Ok(Plan {
            op,
            mentions,
            writes,
            node_writes,
        })
    }
}
