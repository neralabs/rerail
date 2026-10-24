//! A reached atomic parse publishes a document identity, then SQL evaluates its root.
use super::{Demand, Output, Plan, Slots, Sql, ordered};
use serde_json::json;

impl Sql {
    pub(super) fn parsed(
        &mut self,
        term: usize,
        child: &Plan,
        input: &Demand,
    ) -> Result<Output, String> {
        let parsed = self.relation(
            "reached literal parse",
            format!("SELECT d.*,p.truth AS parse_truth,p.work AS parse_work,p.parsed_file,p.parsed_node FROM {} d CROSS JOIN relation_parse_budget({term},d.file,d.node,d.work) p", input.name),
            true, "caller context,atomic parse status,remaining work,parsed document identity",
        );
        // A separate fresh demand is essential: host capture node IDs belong to another file.
        let (query, profile) = ordered::nested::program(
            child,
            input,
            true,
            "d.parse_truth=1",
            &Slots::new(),
            ("d.parsed_file", "d.parsed_node", "d.parse_work"),
            self.generation.clone(),
        )?;
        let name = self.relation(
            "parsed child outcome",
            format!("SELECT d.k,COALESCE(r.c1,d.parse_work) AS work,CASE WHEN d.parse_truth=1 THEN r.c0 ELSE d.parse_truth END AS truth FROM {parsed} d {}", ordered::nested::bridge(&query, 2)),
            true, "caller key,remaining work,parse or SQL child truth",
        );
        self.profile.last_mut().unwrap()["childPrograms"] = json!({"parse":profile});
        Ok(Output { name })
    }
}
