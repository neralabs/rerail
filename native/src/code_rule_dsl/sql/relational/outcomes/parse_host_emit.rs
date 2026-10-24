//! Lexical host snapshots are immutable SQL expressions, never native matcher hooks.
use super::{Demand, Output, Plan, Slots, Sql, fields, needed, ordered, value};
use serde_json::json;

#[derive(Clone)]
pub(super) struct Context {
    pub file: String,
    pub node: String,
    pub alias: String,
    pub slots: Slots,
}
impl Sql {
    pub(super) fn hosted(
        &mut self,
        parse: usize,
        child: &Plan,
        mapping: &[(usize, usize)],
        input: &Demand,
    ) -> Result<Output, String> {
        let context = self
            .generation
            .parse_hosts
            .get(&parse)
            .cloned()
            .ok_or("host emission has no lexical parse snapshot")?;
        let slots = needed(child);
        let columns = fields(&slots, |slot| {
            let before = if context.slots.contains(&slot) {
                format!("{}.c{slot}", context.alias)
            } else {
                "NULL".into()
            };
            if let Some((inner, _)) = mapping.iter().find(|(_, outer)| *outer == slot) {
                let capture = value(input, "d", *inner);
                format!(
                    "relation_capture_export({before},relation_parse_export(d.file,{},{}))",
                    context.file, capture
                )
            } else {
                before
            }
        });
        let demand=Demand{name:self.relation("host snapshot with current parsed exports",format!("SELECT d.k,{} AS file,{} AS node,d.work{columns} FROM {} d",context.file,context.node,input.name),true,"candidate key,original host location,work,original host captures,current parsed exports"),slots};
        let mut generation = self.generation.clone();
        generation.memo_bypass = true;
        let (query, profile) = ordered::nested::program(
            child,
            &demand,
            false,
            "1",
            &Slots::new(),
            ("d.file", "d.node", "d.work"),
            generation,
        )?;
        let name = self.relation(
            "host child read-only outcome",
            format!(
                "SELECT d.k,r.c1 AS work,r.c0 AS truth FROM {} d {}",
                demand.name,
                ordered::nested::bridge(&query, 2)
            ),
            true,
            "candidate key,remaining work,host verdict; host bindings discarded",
        );
        self.profile.last_mut().unwrap()["childPrograms"] = json!({"host":profile});
        Ok(Output { name })
    }
}
