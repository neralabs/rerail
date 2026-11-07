//! Output ownership and transitive invalidation. Persistent user tables are never owned here.
use super::*;

#[derive(Clone)]
pub(super) struct Execution {
    pub inputs: BTreeSet<String>,
    pub objects: Vec<(String, bool)>,
    pub emitted: Arc<plan::emit::Emitted>,
    /// The program as declared (pruned, before lowering), for the catalog.
    pub program: Arc<front::Program>,
    pub counts: BTreeMap<String, u64>,
    pub outputs: Vec<String>,
    pub max_waves: u32,
    pub reductions: Option<BTreeMap<String, reductions::Retained>>,
}

impl CodeRuleSqlExecutor {
    pub(super) fn affected(&self, changed: &BTreeSet<String>) -> BTreeSet<usize> {
        let mut changed = changed.clone();
        let mut affected = BTreeSet::new();
        loop {
            let before = affected.len();
            for (i, execution) in self.executions.iter().enumerate() {
                if !affected.contains(&i)
                    && (execution.inputs.iter().any(|input| changed.contains(input))
                        || execution
                            .objects
                            .iter()
                            .any(|(name, _)| changed.contains(name)))
                {
                    affected.insert(i);
                    changed.extend(execution.objects.iter().map(|(name, _)| name.clone()));
                }
            }
            if before == affected.len() {
                return affected;
            }
        }
    }

    pub(super) fn invalidate(&mut self, changed: &BTreeSet<String>) -> Result<()> {
        let affected = self.affected(changed);
        self.remove_executions(&affected)
    }

    pub(super) fn remove_executions(&mut self, affected: &BTreeSet<usize>) -> Result<()> {
        if affected.is_empty() {
            return Ok(());
        }
        let objects: BTreeSet<_> = affected
            .iter()
            .flat_map(|i| self.executions[*i].objects.iter().cloned())
            .collect();
        for table in [false, true] {
            for (name, is_table) in objects.iter().filter(|(_, is_table)| *is_table == table) {
                self.db
                    .execute_batch(&format!(
                        "DROP {} IF EXISTS temp.{}",
                        if *is_table { "TABLE" } else { "VIEW" },
                        quote(name)
                    ))
                    .map_err(error)?;
            }
        }
        let mut at = 0;
        self.executions.retain(|_| {
            let keep = !affected.contains(&at);
            at += 1;
            keep
        });
        self.generated.retain(|object| !objects.contains(object));
        self.clear_control()?;
        self.db.reset_execution();
        self.refresh_catalog()
    }

    pub(super) fn result_index(&self, relation: &str, columns: &[u32]) -> Result<()> {
        if !self
            .generated
            .iter()
            .any(|(name, table)| name == relation && *table)
        {
            return Err(error("indexes require a session-owned result table"));
        }
        index(&self.db, relation, columns).map_err(error)
    }
}

pub(super) fn index(
    db: &rusqlite::Connection,
    relation: &str,
    columns: &[u32],
) -> rusqlite::Result<()> {
    let count = db
        .prepare(&format!("SELECT * FROM {} LIMIT 0", quote(relation)))?
        .column_count();
    if columns.is_empty()
        || columns.iter().any(|i| *i as usize >= count)
        || columns.iter().collect::<BTreeSet<_>>().len() != columns.len()
    {
        return Err(rusqlite::Error::InvalidParameterName(
            "index columns must be distinct valid positions".into(),
        ));
    }
    let suffix = columns
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("_");
    let name = format!("__sql_index_{relation}_{suffix}");
    let cols = columns
        .iter()
        .map(|i| format!("c{i}"))
        .collect::<Vec<_>>()
        .join(",");
    db.execute_batch(&format!(
        "CREATE INDEX IF NOT EXISTS temp.{} ON {}({cols})",
        quote(&name),
        quote(relation)
    ))
}
