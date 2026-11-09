//! Compose positive support and result queries into a caller's SQL phase.
use super::*;

impl Retained {
    pub(in crate::code_rule_dsl::sql_executor) fn phase_context_rows(&self, counts: &BTreeMap<String,phase::Counts>) -> u64 {
        self.rules.iter().filter_map(|r|counts.get(&r.contexts)).map(|c|c.keys as u64).sum()
    }

    pub(in crate::code_rule_dsl::sql_executor) fn phase_arms(&self) -> usize {
        self.rules
            .iter()
            .map(|r| if r.contribution_view { 2 } else { 4 })
            .sum::<usize>()
            + 6
    }
    pub(in crate::code_rule_dsl::sql_executor) fn declare_phase(
        &self,
        db: &rusqlite::Connection,
        objects: &mut Vec<(String, bool)>,
        store: &versions::Store,
    ) -> Result<()> {
        owned_phase(
            db,
            objects,
            &self.heads,
            &format!(
                "{},n INTEGER NOT NULL CHECK(n>=0),PRIMARY KEY({})",
                schema(self.width),
                names("", self.width)
            ),
            Some(store),
            true,
        )?;
        for rule in self.rules.iter() {
            owned_phase(
                db,
                objects,
                &rule.contexts,
                &format!(
                    "{},PRIMARY KEY({})",
                    schema(rule.width),
                    names("", rule.width)
                ),
                Some(store),
                true,
            )?;
            let heads = (0..self.width)
                .map(|i| format!("h{i} INTEGER NOT NULL"))
                .collect::<Vec<_>>()
                .join(",");
            if !rule.contribution_view {
                owned_phase(
                    db,
                    objects,
                    &rule.contributions,
                    &format!(
                        "{},{heads},PRIMARY KEY({})",
                        schema(rule.width),
                        names("", rule.width)
                    ),
                    Some(store),
                    true,
                )?;
            } else {
                let columns = rule
                    .contribution_storage
                    .heads
                    .iter()
                    .filter_map(|e| match e {
                        crate::code_rule_dsl::logic::analysis::scalars::Expression::Column(c) => {
                            Some(*c)
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                rule.index(db, &columns)?;
            }
            for atom in &rule.atoms {
                rule.index(
                    db,
                    &(atom.offset..atom.offset + atom.columns.len()).collect::<Vec<_>>(),
                )?;
            }
        }
        Ok(())
    }

    pub(in crate::code_rule_dsl::sql_executor) fn append_phase(
        &self,
        w: &mut Work<'_>,
        changes: &BTreeMap<String, Delta>,
        phase: &mut crate::code_rule_dsl::sql_executor::phase::Phase,
        full: bool,
    ) -> Result<(String, String)> {
        if w.profiler.is_some() {
            phase
                .analyses
                .push(json!({"full_rebuild":full,"analysis":self.explain()}));
        }
        if !full {
            return fused::append(self, w, changes, phase);
        }
        let store = w.versions.unwrap();
        let mut contributions = vec![];
        for rule in self.rules.iter() {
            let contexts = phase.relation(rule.width, &rule.context_query.render());
            let evaluated = phase.relation(
                rule.width + self.width,
                &rule.evaluation(&contexts).render(),
            );
            phase.replace_all(store, &rule.contexts, &contexts)?;
            if !rule.contribution_view {
                phase.replace_all(store, &rule.contributions, &evaluated)?;
            }
            contributions.push(format!(
                "SELECT {} FROM {evaluated}",
                (rule.width..rule.width + self.width)
                    .map(|c| format!("c{c}"))
                    .collect::<Vec<_>>()
                    .join(",")
            ));
        }
        let cols = names("", self.width);
        let values = if contributions.is_empty() {
            format!("SELECT {} WHERE 0", vec!["NULL"; self.width].join(","))
        } else {
            contributions.join(" UNION ALL ")
        };
        let values = phase.view(self.width, &values);
        let heads = phase.relation(self.width + 1, &self.head_query(&cols, &values));
        phase.replace_all(store, &self.heads, &heads)?;
        let result = phase.relation(
            self.width,
            &self.initial_result_from(&heads, &format!("c{}", self.width)),
        );
        phase.replace_all(store, &self.output, &result)?;
        let keys = self.lattice.as_ref().map_or(self.width, |l| l.keys);
        let old = if store.version(&self.output).len() > 0 {
            format!(
                " UNION SELECT {} FROM {}",
                names("", keys),
                quote(&self.output)
            )
        } else {
            String::new()
        };
        let keys = phase.view(
            keys,
            &format!("SELECT {} FROM {result}{old}", names("", keys)),
        );
        Ok((keys, result))
    }
}
