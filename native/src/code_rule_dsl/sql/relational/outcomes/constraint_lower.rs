//! Constraints are ordered tests on captured nodes, with one commit after all succeed.
use super::{CaptureKind, Lower, Op, Plan};
use crate::code_rule_dsl::matcher::Constrained;

impl Lower {
    /// The caller has already charged the Constrained operator to the lowering budget.
    pub(super) fn constrained(&mut self, constrained: &Constrained) -> Result<Plan, String> {
        let core = self.plan(&constrained.rule)?;
        let mut mentions = core.mentions.clone();
        let mut writes = core.writes.clone();
        let mut node_writes = core.node_writes.clone();
        let mut constraints = Vec::new();
        for (name, matcher) in &constrained.constraints {
            let qualified = format!("{}{name}", self.namespace);
            let existing = self.slots.iter().position(|slot| slot.name == qualified);
            let target = match existing {
                Some(slot) => slot,
                None => self.slot(name, CaptureKind::Node)?,
            };
            let child = self.plan(matcher)?;
            if child.writes.iter().any(|slot| {
                self.slots[*slot].kind == CaptureKind::Count && !core.writes.contains(slot)
            }) {
                return Err("outcome constraints: constraint-only count exports need an internal/publication contract".into());
            }
            mentions.insert(target);
            mentions.extend(&child.mentions);
            writes.extend(&child.writes);
            node_writes.extend(&child.node_writes);
            // Native get_match ignores list and transformed count values. Their constraint body is
            // unreachable; keep its declared schema, but do not execute it or its effects.
            if self.slots[target].kind == CaptureKind::Node {
                constraints.push((target, child));
            }
        }
        let op = Op::Constrained(Box::new(core), constraints);
        Ok(Plan {
            effects: super::super::effects::Effects::of(&op),
            op,
            mentions,
            writes,
            node_writes,
            walk_filter: None,
        })
    }
}
