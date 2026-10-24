//! Parsed children use private captures; only successful exports cross the document boundary.
use super::super::ParseSetup;
use super::{Demand, Output, Plan, Slots, Sql, fields, ordered, value};
use serde_json::json;

impl Sql {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn parsed(
        &mut self,
        term: usize,
        setup: &ParseSetup,
        child: &Plan,
        exports: &[(usize, usize)],
        hosted: bool,
        input: &Demand,
    ) -> Result<Output, String> {
        match setup {
            ParseSetup::Plain => self.parsed_body(term, false, child, exports, hosted, input),
            ParseSetup::Parsed => self.parsed_body(term, true, child, exports, hosted, input),
            ParseSetup::HostKinds(hosts) => {
                self.parsed_host_guard(term, hosts, child, exports, hosted, input)
            }
        }
    }
    #[allow(clippy::too_many_arguments)]
    fn parsed_host_guard(
        &mut self,
        term: usize,
        hosts: &[u16],
        child: &Plan,
        exports: &[(usize, usize)],
        hosted: bool,
        input: &Demand,
    ) -> Result<Output, String> {
        self.generation.operator_as("parsed host-kind rejection")?;
        let width = 2 + exports.len();
        let mut generation = self.generation.clone();
        generation.return_cells = generation
            .return_cells
            .checked_add(2 * width)
            .filter(|cells| *cells <= 128)
            .ok_or("guarded parsed return exceeds 128 active cells")?;
        generation
            .return_peak
            .set(generation.return_peak.get().max(generation.return_cells));
        let mut nested = Sql {
            definitions: vec![],
            profile: vec![],
            correlated: true,
            generation,
        };
        let columns = fields(&input.slots, |slot| value(input, "d", slot));
        let reached = Demand {
            name: nested.relation(
                "host-eligible parsed invocation",
                format!("SELECT 1 AS k,d.file,d.node,d.work{columns}"),
                true,
                "one reached host with original caller captures",
            ),
            slots: input.slots.clone(),
        };
        let answer = nested.parsed_body(term, false, child, exports, hosted, &reached)?;
        let payload = exports
            .iter()
            .map(|(_, outer)| format!(",o.c{outer}"))
            .collect::<String>();
        let query = format!(
            "WITH RECURSIVE {} SELECT o.truth,o.work{payload} FROM {} o",
            nested.definitions.join(",\n"),
            answer.name
        );
        let eligible =
            super::predicate::condition(&super::Set::Kinds(hosts.to_vec()), "parse_kind")?;
        let before = exports
            .iter()
            .map(|(_, outer)| format!(",{}", value(input, "d", *outer)))
            .collect::<String>();
        let returned = (0..width)
            .map(|index| format!("r.c{index}"))
            .collect::<Vec<_>>()
            .join(",");
        let choose = format!(
            "SELECT 0,d.work{before} WHERE NOT ({eligible}) UNION ALL SELECT {returned} FROM (SELECT 1 AS reached WHERE {eligible}) allowed {}",
            ordered::nested::bridge(&query, width)
        );
        let exported = exports
            .iter()
            .enumerate()
            .map(|(index, (_, outer))| format!(",r.c{} AS c{outer}", index + 2))
            .collect::<String>();
        let name=self.relation("parsed native host-kind dispatch",format!("SELECT d.k,r.c1 AS work,r.c0 AS truth{exported} FROM {} d JOIN node parse_kind ON parse_kind.file=d.file AND parse_kind.id=d.node {}",input.name,ordered::nested::bridge(&choose,width)),true,"native host-kind rejection before recursive child setup; outer utility entry and caller captures retained");
        self.profile.last_mut().unwrap()["childPrograms"] = json!({"eligibleParse":{"relations":nested.profile,"hostKinds":hosts,"singleReachedDemand":true,"typedReturnColumns":width}});
        Ok(Output { name })
    }

    #[allow(clippy::too_many_arguments)]
    fn parsed_body(
        &mut self,
        term: usize,
        defer_child: bool,
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
        if defer_child {
            self.generation
                .operator_as("parsed reached-child dispatch")?;
            generation.return_cells = generation
                .return_cells
                .checked_add(2 + inner.len())
                .filter(|cells| *cells <= 128)
                .ok_or("deferred parsed return exceeds 128 active cells")?;
            generation
                .return_peak
                .set(generation.return_peak.get().max(generation.return_cells));
        }
        let guard = if defer_child {
            "1".to_string()
        } else {
            format!("{alias}.parse_truth=1")
        };
        let (query, profile) = ordered::nested::program(
            child,
            input,
            true,
            &guard,
            &inner,
            (
                &format!("{alias}.parsed_file"),
                &format!("{alias}.parsed_node"),
                &format!("{alias}.parse_work"),
            ),
            generation,
        )?;
        if exports.is_empty() {
            let name=self.relation("parsed child outcome",format!("SELECT {alias}.k,COALESCE(r.c1,{alias}.parse_work) AS work,CASE WHEN {alias}.parse_truth=1 THEN r.c0 ELSE {alias}.parse_truth END AS truth FROM {parsed} {alias} {}",child_bridge(&query,2,defer_child,&format!("{alias}.parse_truth=1"))),true,"caller key,remaining work,parse or SQL child truth");
            self.profile.last_mut().unwrap()["childPrograms"] = json!({"parse":profile});
            return Ok(Output { name });
        }
        let returned = inner
            .iter()
            .enumerate()
            .map(|(index, slot)| format!(",r.c{} AS e{slot}", index + 2))
            .collect::<String>();
        let tested=self.relation("parsed child private outcome",format!("SELECT {alias}.*,r.c0 AS tested,r.c1 AS remaining{returned} FROM {parsed} {alias} {}",child_bridge(&query,2+inner.len(),defer_child,&format!("{alias}.parse_truth=1"))),true,"caller context,private child captures,child truth,remaining work");
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

/// Keep the authoritative atomic parse before the gate. The allowed row is the
/// first CROSS JOIN input, so a failed/unknown parse cannot initialize child SQL.
fn child_bridge(query: &str, width: usize, deferred: bool, guard: &str) -> String {
    if !deferred {
        return ordered::nested::bridge(query, width);
    }
    let columns = (0..width)
        .map(|index| format!("parsed_child.c{index}"))
        .collect::<Vec<_>>()
        .join(",");
    let gated = format!(
        "SELECT {columns} FROM (SELECT 1 AS reached WHERE {guard}) allowed CROSS JOIN relation_row parsed_child ON ({columns}) IS ({query})"
    );
    ordered::nested::bridge(&gated, width)
}
