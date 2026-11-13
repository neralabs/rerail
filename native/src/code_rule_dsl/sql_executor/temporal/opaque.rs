//! Opaque SQL runs at a resolved gate. Checked neighbors keep their retained state.
use super::*;

#[derive(Clone)]
pub struct Plan {
    pub select: String,
    pub inputs: BTreeMap<String, String>,
}
impl Plan {
    pub fn new(step: &schedule::Step, inputs: &BTreeMap<String, String>) -> Result<Self> {
        let prefix = format!("INSERT OR IGNORE INTO new_{} ", step.output);
        let select = step
            .sql
            .strip_prefix(&prefix)
            .ok_or_else(|| error("SQL producer has no emitted SELECT body"))?
            .to_owned();
        Ok(Self {
            select,
            inputs: step
                .reads
                .iter()
                .filter_map(|r| inputs.get(r).map(|table| (r.clone(), table.clone())))
                .collect(),
        })
    }
    pub fn publish_inputs(
        &self,
        w: &mut Work<'_>,
        store: &versions::Store,
        publications: &mut Publications,
    ) -> Result<()> {
        let _context=w.profile_scope(||json!({"stage":"opaque producer input publication","relations":self.inputs.keys().collect::<Vec<_>>()}));
        for (logical, table) in &self.inputs {
            publications.publish(w, logical, store.version(table))?;
        }
        Ok(())
    }
}

/// Exact versions currently visible in SQL-owned logical result tables.
/// An early producer publication must not cause repeated final writes or trigger calls.
#[derive(Clone, Default)]
pub struct Publications {
    roots: BTreeMap<String, versions::Version>,
}
impl Publications {
    pub fn publish(
        &mut self,
        w: &mut Work<'_>,
        relation: &str,
        next: versions::Version,
    ) -> Result<()> {
        let previous = self
            .roots
            .get(relation)
            .cloned()
            .unwrap_or_else(|| next.empty_like());
        if !previous.same(&next) {
            let change = delta(w, &previous, &next)?;
            let columns = names("", next.columns().len());
            if change.deleted > 0 {
                w.db.execute(
                    &format!(
                        "DELETE FROM {} WHERE ({columns}) IN(SELECT {columns} FROM {})",
                        quote(relation),
                        change.minus
                    ),
                    [],
                )
                .map_err(error)?;
            }
            if change.inserted > 0 {
                w.db.execute(
                    &format!(
                        "INSERT INTO {} SELECT * FROM {}",
                        quote(relation),
                        change.plus
                    ),
                    [],
                )
                .map_err(error)?;
            }
        }
        self.roots.insert(relation.into(), next);
        Ok(())
    }
}
