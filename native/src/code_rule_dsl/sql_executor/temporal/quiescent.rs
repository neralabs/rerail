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
                result |= matches!(*name, "count" | "sum" | "rr_join" | "rr_update_sum")
                    || (matches!(*name, "min" | "max") && arguments.len() == 1);
            }
        });
    }
    result
}

impl operator::Operator {
    pub(super) fn clear_empty_body(&mut self, store: &versions::Store) -> Result<bool> {
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
                                !store.contains(&name) || store.version(&name).len() > 0
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
        if protected.iter().any(|r| empty.contains(r)) {
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
                let version = store.version(&table);
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
        Ok(true)
    }
}
