//! Host programs are lowered in the original host namespace and run as SQL children.
use super::super::{CaptureKind, Op, Plan, Slots, effects::Effects};
use super::Lower;
use crate::code_rule_dsl::matcher::HostTest;

#[derive(Clone)]
pub(super) struct Scope {
    pub term: usize,
    pub outer_namespace: String,
    pub child_namespace: String,
}
impl Lower {
    pub(super) fn hosted(&mut self, test: &HostTest) -> Result<Plan, String> {
        let scope = self
            .parse_hosts
            .last()
            .cloned()
            .ok_or("parse host has no lexical parse owner")?;
        let incoming: Vec<_> = self
            .slots
            .iter()
            .enumerate()
            .filter_map(|(id, slot)| {
                let name = self.logical_names[id].as_ref()?;
                (slot.name == format!("{}{name}", scope.child_namespace)
                    && slot.kind == CaptureKind::Node)
                    .then(|| (id, name.clone()))
            })
            .collect();
        let namespace = std::mem::replace(&mut self.namespace, scope.outer_namespace);
        let depth = std::mem::replace(&mut self.parsed_depth, 0);
        // A host rule's own nested parse establishes its own host context.
        let mut mapping = Vec::new();
        for (inner, name) in incoming {
            let outer = self.slot(&name, CaptureKind::Node)?;
            self.span_slots.insert(outer);
            mapping.push((inner, outer));
        }
        self.hosted_depth += 1;
        let child_result = self.plan(&test.rule);
        self.hosted_depth -= 1;
        self.namespace = namespace;
        self.parsed_depth = depth;
        let child = child_result?;
        let mut mentions: Slots = mapping.iter().map(|(inner, _)| *inner).collect();
        // Caller captures are supplied through the lexical host snapshot, not the parsed env.
        mentions.extend(&child.mentions);
        let op = Op::Host {
            parse: scope.term,
            child: Box::new(child),
            mapping,
        };
        Ok(Plan {
            effects: Effects::of(&op),
            op,
            mentions,
            writes: Slots::new(),
            node_writes: Slots::new(),
            walk_filter: None,
        })
    }
}
