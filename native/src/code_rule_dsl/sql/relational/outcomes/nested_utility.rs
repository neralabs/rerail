//! Utility entry, reached body, and measured answer in one dependency chain.
use super::*;

impl Shared<'_> {
    pub(super) fn utility(
        &mut self,
        utility: usize,
        body: &Plan,
        guard: &str,
    ) -> Result<(String, Option<(usize, usize)>), String> {
        // Keep input bindings for both the memo key and rollback. Unwritten slots
        // keep their original value throughout the body and need no snapshot.
        let saved = self.save(&body.writes)?;
        *self
            .memo
            .remaining
            .get_mut(&utility)
            .expect("nested utility") -= 1;
        let keep = self.memo.remaining[&utility] > 0;
        let reached = self.name("reached");
        let start = self.name("start");
        let hit = self.name("hit");
        let cost = self.name("memoCost");
        let truth = self.name("truth");
        let outputs: BTreeMap<_, _> = body
            .writes
            .iter()
            .map(|s| (*s, self.name("export")))
            .collect();
        let prior: Vec<_> = self
            .memo
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.utility == utility)
            .map(|(id, _)| id)
            .collect();
        let choose = |column: &str| {
            if prior.is_empty() {
                return "NULL".into();
            }
            let arms = prior
                .iter()
                .map(|id| {
                    let key = self
                        .memo
                        .keys
                        .iter()
                        .map(|s| format!(" AND d.m{id}i{s} IS d.c{s}"))
                        .collect::<String>();
                    format!("WHEN d.m{id}truth>=0{key} THEN d.m{id}{column}")
                })
                .collect::<Vec<_>>()
                .join(" ");
            format!("CASE WHEN {guard} THEN CASE {arms} END END")
        };
        let mut lookup = BTreeMap::from([
            (reached.clone(), format!("COALESCE({guard},0)")),
            (start.clone(), "d.work".into()),
            (hit.clone(), choose("truth")),
            (cost.clone(), choose("cost")),
        ]);
        lookup.extend(
            outputs
                .iter()
                .map(|(s, n)| (n.clone(), choose(&format!("o{s}")))),
        );
        // A first miss needs only constants and the input allowance. A dynamic
        // lookup stays materialized so its capture-key comparisons run once.
        self.project(
            "nested utility answer lookup",
            lookup,
            &[],
            !prior.is_empty(),
        )?;
        self.project("nested utility entry or replay charge",BTreeMap::from([
            (truth.clone(),format!("CASE WHEN NOT d.{reached} THEN NULL WHEN d.{hit} IS NOT NULL THEN CASE WHEN d.work<d.{cost} THEN -2 ELSE d.{hit} END WHEN d.work=0 THEN -2 END")),
            ("work".into(),format!("CASE WHEN NOT d.{reached} THEN d.work WHEN d.{hit} IS NOT NULL THEN MAX(0,d.work-d.{cost}) ELSE MAX(0,d.work-1) END")),
        ]),&[],true)?;
        let miss = format!("d.{reached} AND d.{hit} IS NULL AND d.{start}>0");
        let inherited = self.memo.proven.clone();
        let (answer, _) = self.emit(body, &miss)?;
        // A body is optional on an outer memo hit. Its compile-time replay proofs
        // cannot escape this boundary; its runtime typed answers do escape.
        self.memo.proven = inherited;
        let result = format!("CASE WHEN {miss} THEN d.{answer} ELSE d.{truth} END");
        let mut updates = BTreeMap::from([(truth.clone(), result.clone())]);
        for s in &body.writes {
            updates.insert(format!("c{s}"),format!("CASE WHEN ({result})=1 THEN CASE WHEN d.{hit} IS NOT NULL THEN d.{} ELSE d.c{s} END ELSE d.{} END",outputs[s],saved[s]));
        }
        self.project(
            "nested utility outcome and rollback",
            updates,
            &[answer],
            false,
        )?;
        let entry = if keep {
            let id = self.memo.allocate(utility, &body.writes)?;
            let mut columns = format!(
                ",CASE WHEN d.{truth}>=0 THEN d.{truth} END AS m{id}truth,d.{start}-d.work AS m{id}cost"
            );
            for s in &self.memo.keys {
                columns.push_str(&format!(
                    ",{} AS m{id}i{s}",
                    saved
                        .get(s)
                        .map_or_else(|| format!("d.c{s}"), |n| format!("d.{n}"))
                ));
            }
            for s in &body.writes {
                columns.push_str(&format!(",d.c{s} AS m{id}o{s}"));
            }
            self.demand.name = self.sql.relation(
                "nested typed utility answer and total cost",
                format!("SELECT d.*{columns} FROM {} d", self.demand.name),
                false,
                "utility truth,total measured cost,original input bindings,exports",
            );
            let p = self.sql.profile.last_mut().unwrap();
            p["memoKey"] = json!({"utility":utility,"context":["file","node"],"captures":self.memo.keys,"entry":id,"scope":"shared root demand through utility bodies"});
            p["memoColumns"] = json!(self.memo.entries[id].columns.len());
            Some(id)
        } else {
            None
        };
        let retired: Vec<_> = saved
            .into_values()
            .chain(outputs.into_values())
            .chain([reached, start, hit, cost])
            .collect();
        self.project("nested utility return", BTreeMap::new(), &retired, true)?;
        self.memo.wrote(body);
        Ok((
            truth,
            if self.demand.slots.is_empty() {
                entry.map(|id| (utility, id))
            } else {
                None
            },
        ))
    }
}
