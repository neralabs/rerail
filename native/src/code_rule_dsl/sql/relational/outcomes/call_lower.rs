//! Parameter calls use private utility captures and an explicit typed frame boundary.
use super::{CaptureKind, Lower, Op, Plan, Rc, Slots};
use crate::code_rule_dsl::matcher::UtilCall;

impl Lower {
    pub(super) fn call(&mut self, call: &UtilCall) -> Result<Plan, String> {
        self.utility_calls += 1;
        let key = (call.util.table.as_ptr() as usize, call.util.index);
        let (_, body) = if let Some((id, body)) = self.call_bodies.get(&key) {
            (*id, Rc::clone(body))
        } else {
            if !self.active.insert(key) {
                return Err(
                    "outcome relations: recursive parameter calls require a component contract"
                        .into(),
                );
            }
            let id = self.call_bodies.len();
            let caller_namespace =
                std::mem::replace(&mut self.namespace, format!("@call{}:{}:", key.0, key.1));
            let result = call.util.with_target(|rule| self.plan(rule));
            self.namespace = caller_namespace;
            self.active.remove(&key);
            let body = Rc::new(result.ok_or("outcome relations: missing parameter utility")??);
            if body.effects.contains_chain {
                return Err(
                    "outcome relations: recursive parameter calls require a component contract"
                        .into(),
                );
            }
            if body
                .mentions
                .union(&body.writes)
                .any(|s| self.slots[*s].kind != CaptureKind::Node)
            {
                return Err("outcome relations: parameter calls currently require node captures; list and count frames need typed contracts".into());
            }
            self.call_bodies.insert(key, (id, Rc::clone(&body)));
            (id, body)
        };
        let mut mapping = Vec::new();
        let mut mentions = Slots::new();
        let mut writes = Slots::new();
        for (param, caller) in &call.args {
            let outer = self.slot(caller, CaptureKind::Node)?;
            let caller_namespace =
                std::mem::replace(&mut self.namespace, format!("@call{}:{}:", key.0, key.1));
            let inner = self.slot(param, CaptureKind::Node);
            self.namespace = caller_namespace;
            let inner = inner?;
            mapping.push((inner, outer));
            mentions.insert(outer);
            if body.writes.contains(&inner) {
                writes.insert(outer);
            }
        }
        let frame_op = Op::CallFrame { body, mapping };
        let frame = Rc::new(Plan {
            effects: super::super::effects::Effects::of(&frame_op),
            op: frame_op,
            mentions: mentions.clone(),
            writes: writes.clone(),
            node_writes: writes.clone(),
            walk_filter: None,
        });
        // Native memo::call identifies the expression, not the utility it invokes.
        let address = std::ptr::from_ref(call) as usize;
        let next = 1_000_000 + self.call_sites.len();
        let identity = *self.call_sites.entry(address).or_insert(next);
        let op = Op::Charge(identity, frame);
        Ok(Plan {
            effects: super::super::effects::Effects::of(&op),
            op,
            mentions,
            writes: writes.clone(),
            node_writes: writes,
            walk_filter: None,
        })
    }
}
