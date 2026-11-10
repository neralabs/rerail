//! Compose positive support and result queries into a caller's SQL phase.
use super::*;

impl Retained {
    /// A single positive conjunction has no contexts if any of its atom sources is empty.
    pub(in crate::code_rule_dsl::sql_executor) fn empty_positive_inputs(&self) -> Vec<String> {
        if self.rules.len() == 1 && self.rules[0].reducers.is_empty() {
            self.rules[0]
                .atoms
                .iter()
                .map(|a| a.relation.clone())
                .collect()
        } else {
            vec![]
        }
    }
    pub(in crate::code_rule_dsl::sql_executor) fn version_loop_eligible(&self) -> bool {
        self.lattice.is_none() && self.can_fuse() && self.rules.iter().all(|r| !r.atoms.is_empty())
    }


    pub(in crate::code_rule_dsl::sql_executor) fn common_phase(&self) -> crate::code_rule_dsl::logic::analysis::Decision {
        let support = self.can_fuse() || self.isolated_phase().proven || self.scalar_phase_eligible();
        let bounded = self.phase_arms()<400;
        crate::code_rule_dsl::logic::analysis::Decision::new(support && bounded, if !bounded {
            "The support transition exceeds the common phase result-arm bound"
        } else if support {
            "Total or fenced checked evaluation stays at its scheduler gate; independent summaries, support, and wave folds share atomic publication"
        } else {
            "Dependent, ordered, nullable, or opaque evaluation retains a separate transition boundary"
        })
    }
    pub(in crate::code_rule_dsl::sql_executor) fn set_step(&mut self, step: usize) {
        for rule in Arc::make_mut(&mut self.rules) { rule.step=step; }
    }
    /// Bind new predecessor values inside the current phase while preserving
    /// the logical input names used by signed deltas and retained cursors.
    pub(in crate::code_rule_dsl::sql_executor) fn bind_inputs(&self, inputs: &BTreeMap<String, String>) -> Self {
        let mut next = self.clone();
        for rule in Arc::make_mut(&mut next.rules) {
            rule.context_query.body.relations_mut(&mut |r| {
                if let Some((name, table)) = r.program_relation().and_then(|name| inputs.get(name).map(|table| (name.to_owned(), table.clone()))) {
                    *r = Rel::Bound { rel: name, table: quote(&table) };
                }
            });
        }
        next
    }
    pub(in crate::code_rule_dsl::sql_executor) fn phase_context_rows(&self, counts: &BTreeMap<String,phase::Counts>) -> u64 {
        self.rules.iter().filter_map(|r|counts.get(&r.contexts)).map(|c|c.keys as u64).sum()
    }

    pub(in crate::code_rule_dsl::sql_executor) fn phase_arms(&self) -> usize {
        self.rules
            .iter()
            .map(|r| if r.contribution_view { 2 } else { 4 })
            .sum::<usize>()
            + 6
            + self.rules.iter().map(|r|r.reducers.iter().filter(|r|r.shared_owner()).count()*6).sum::<usize>()
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
        if self.rules.iter().any(|r|!r.reducers.is_empty()) {
            return Err(error("Scalar phases must retain their summary completions until publication"));
        }
        self.append_phase_with(w, changes, phase, full, &mut reducer::staged::Batch::default())
    }

    pub(in crate::code_rule_dsl::sql_executor) fn initialize_summaries(
        &self, w: &mut Work<'_>, objects: &mut Vec<(String,bool)>, store: &versions::Store,
        phase: &mut phase::Phase, summaries: &mut reducer::staged::Batch,
    ) -> Result<()> {
        for rule in self.rules.iter() {
            for reducer in &rule.reducers {
                summaries.initialize(reducer,w,objects,phase)?;
                reducer.index_context(w.db, rule, Some(store))?;
            }
        }
        Ok(())
    }

    pub(in crate::code_rule_dsl::sql_executor) fn append_phase_with(
        &self,
        w: &mut Work<'_>,
        changes: &BTreeMap<String, Delta>,
        phase: &mut crate::code_rule_dsl::sql_executor::phase::Phase,
        full: bool,
        summaries: &mut reducer::staged::Batch,
    ) -> Result<(String,String)> {
        if w.profiler.is_some() {
            phase
                .analyses
                .push(json!({"full_rebuild":full,"analysis":self.explain()}));
        }
        let mut extra = BTreeMap::new();
        for rule in self.rules.iter() {
            let mut queries = vec![];
            for reducer in &rule.reducers {
                if let Some(keys) = summaries.append(reducer,w,changes,phase)? {
                    queries.push(reducer.affected_contexts(rule,&keys));
                }
            }
            if !queries.is_empty() { extra.insert(rule.step,queries); }
        }
        if !full && self.scalar_phase_eligible() {
            let mut pending = vec![];
            let (keys,next,_) = fused::append_with(self,w,changes,phase,&extra,&mut pending,&summaries.sources)?;
            if !pending.is_empty() { return Err(error("common scalar phase requires native support roots")); }
            return Ok((keys,next));
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
                &rule.evaluation_with(&contexts, &summaries.sources).render(),
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
