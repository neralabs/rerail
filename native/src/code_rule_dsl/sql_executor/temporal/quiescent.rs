//! Empty positive frontiers need no context deletion scan. Scalar input indexes survive.
use super::super::super::logic::sql::sql_ast::SqlExpression as E;
use super::*;

fn aggregate(query: &Query) -> bool {
    let mut result = false;
    for (e, _) in &query.body.columns {
        // Expression visitors do not enter scalar subqueries. Only an aggregate
        // in this SELECT can produce a row from an empty positive body.
        e.visit(&mut |e| {
            if let E::Call { name, arguments } = e {
                result |= matches!(
                    *name,
                    "count" | "sum" | "rr_exact_sum" | "rr_join" | "rr_update_sum"
                ) || (matches!(*name, "min" | "max") && arguments.len() == 1);
            }
        });
    }
    result
}

impl operator::Operator {
    pub fn clear_empty_body(
        &mut self,
        db: &rusqlite::Connection,
        store: &versions::Store,
    ) -> Result<bool> {
        self.empty_body = false;
        if self.opaque.is_some() {return Ok(false);}
        let native_empty = self
            .run
            .reductions
            .as_ref()
            .is_none_or(|states| states.values().all(|s| s.reducer_inputs().next().is_none()));
        let emitted = &self.run.emitted;
        let internal = emitted
            .program
            .relations
            .iter()
            .map(|(r, _)| r.clone())
            .collect::<BTreeSet<_>>();
        let mut possible = BTreeSet::new();
        loop {
            let before = possible.len();
            for (step, query) in emitted.program.steps.iter().zip(&emitted.queries) {
                let nonempty = query.as_ref().is_none_or(|q| {
                    !q.ctes.is_empty()
                        || aggregate(q)
                        || q.body.from.iter().all(|i| {
                            let Some(name) = read(&i.relation) else {
                                return true;
                            };
                            if internal.contains(&name) {
                                possible.contains(&name)
                            } else {
                                if store.contains(&name) {
                                    store.version(&name).len() > 0
                                } else if native_empty {
                                    // Estimates only avoid expensive exact counts. Only an
                                    // exact zero proves emptiness. Scalar cursors must still
                                    // consume their journals, even with an empty outer body.
                                    super::super::super::facts::vtab::bound_source(db, &name)
                                        .is_none_or(|s| s.estimated_len() > 0 || s.len() > 0)
                                } else {
                                    true
                                }
                            }
                        })
                });
                if nonempty {
                    possible.insert(step.output.clone());
                }
            }
            if possible.len() == before {
                break;
            }
        }
        if possible.contains(&self.result) {
            return Ok(false);
        }
        let empty = internal
            .difference(&possible)
            .cloned()
            .collect::<BTreeSet<_>>();
        let mut protected = self
            .run
            .reductions
            .iter()
            .flat_map(|states| states.values())
            .flat_map(|s| s.reducer_inputs())
            .map(str::to_owned)
            .collect::<BTreeSet<_>>();
        // Clearing an internal scalar source would require propagating its deletion
        // into the surviving reducer index. Leave such operators to normal maintenance.
        if protected.iter().any(|r| empty.contains(r))
            || emitted
                .program
                .steps
                .iter()
                .zip(&emitted.queries)
                .any(|(step, q)| {
                    empty.contains(&step.output)
                        && q.as_ref().is_some_and(|q| {
                            q.body
                                .from
                                .iter()
                                .filter_map(|i| read(&i.relation))
                                .any(|r| protected.contains(&r))
                        })
                })
        {
            // A preserved scalar cursor may return to an identical source version.
            // Its positive contexts must survive too, or reactivation sees no delta
            // and never reconstructs them.
            return Ok(false);
        }
        for (step, query) in emitted.program.steps.iter().zip(&emitted.queries) {
            if !empty.contains(&step.output) {
                if let Some(query) = query {
                    protected.extend(query.reads().iter().filter_map(|r| read(r)));
                }
            }
        }
        for name in &empty {
            if let Some(state) = self.run.reductions.as_ref().and_then(|s| s.get(name)) {
                state.clear_contexts(store)?;
            }
            for table in [name.clone(), format!("new_{name}"), format!("delta_{name}")] {
                let Some(version) = store.available(&table) else { continue; };
                if version.len() > 0 {
                    store.replace(&table, version.empty_like())?;
                }
            }
            self.run.counts.insert(name.clone(), 0);
        }
        for (r, seen) in &mut self.seen {
            if !protected.contains(r) {
                *seen = store.version(r);
            }
        }
        for (r, pending) in &self.pending {
            if !protected.contains(r) {
                pending.clear(store)?;
            }
        }
        self.empty_body = true;
        Ok(true)
    }
}
