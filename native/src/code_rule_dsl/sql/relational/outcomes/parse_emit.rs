//! Parsed children use private captures; only successful exports cross the document boundary.
use super::{Demand, Output, Plan, Slots, Sql, fields, ordered, value};
use serde_json::json;

impl Sql {
    pub(super) fn parsed(
        &mut self,
        term: usize,
        child: &Plan,
        exports: &[(usize, usize)],
        hosted: bool,
        input: &Demand,
    ) -> Result<Output, String> {
        let parsed=self.relation("reached literal parse",format!("SELECT d.*,p.truth AS parse_truth,p.work AS parse_work,p.parsed_file,p.parsed_node FROM {} d CROSS JOIN relation_parse_budget({term},d.file,d.node,d.work) p",input.name),true,"caller context,atomic parse status,remaining work,parsed document identity");
        let alias = if hosted {
            format!("parse_host_{term}")
        } else {
            "d".into()
        };
        let mut generation = self.generation.clone();
        generation.memo_bypass = true;
        if hosted {
            generation.parse_hosts.insert(
                term,
                super::parse_host_emit::Context {
                    file: format!("{alias}.file"),
                    node: format!("{alias}.node"),
                    alias: alias.clone(),
                    slots: input.slots.clone(),
                },
            );
        }
        let inner: Slots = exports.iter().map(|(inner, _)| *inner).collect();
        let (query, profile) = ordered::nested::program(
            child,
            input,
            true,
            &format!("{alias}.parse_truth=1"),
            &inner,
            (
                &format!("{alias}.parsed_file"),
                &format!("{alias}.parsed_node"),
                &format!("{alias}.parse_work"),
            ),
            generation,
        )?;
        if exports.is_empty() {
            let name=self.relation("parsed child outcome",format!("SELECT {alias}.k,COALESCE(r.c1,{alias}.parse_work) AS work,CASE WHEN {alias}.parse_truth=1 THEN r.c0 ELSE {alias}.parse_truth END AS truth FROM {parsed} {alias} {}",ordered::nested::bridge(&query,2)),true,"caller key,remaining work,parse or SQL child truth");
            self.profile.last_mut().unwrap()["childPrograms"] = json!({"parse":profile});
            return Ok(Output { name });
        }
        let returned = inner
            .iter()
            .enumerate()
            .map(|(index, slot)| format!(",r.c{} AS e{slot}", index + 2))
            .collect::<String>();
        let tested=self.relation("parsed child private outcome",format!("SELECT {alias}.*,r.c0 AS tested,r.c1 AS remaining{returned} FROM {parsed} {alias} {}",ordered::nested::bridge(&query,2+inner.len())),true,"caller context,private child captures,child truth,remaining work");
        self.profile.last_mut().unwrap()["childPrograms"] = json!({"parse":profile});
        let output_slots: Slots = exports.iter().map(|(_, outer)| *outer).collect();
        let columns = fields(&output_slots, |outer| {
            let inner = exports.iter().find(|(_, slot)| *slot == outer).unwrap().0;
            let before = value(input, "d", outer);
            format!(
                "CASE WHEN d.parse_truth=1 AND d.tested=1 THEN relation_capture_export({before},relation_parse_export(d.parsed_file,d.file,d.e{inner})) ELSE {before} END"
            )
        });
        let name=self.relation("parsed outcome and successful mapped exports",format!("SELECT d.k,COALESCE(d.remaining,d.parse_work) AS work,CASE WHEN d.parse_truth=1 THEN d.tested ELSE d.parse_truth END AS truth{columns} FROM {tested} d"),true,"caller key,remaining work,truth,committed decoded captures in host coordinates");
        Ok(Output { name })
    }
}
