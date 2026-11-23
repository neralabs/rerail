//! Pass staged relation values and signed changes to acyclic consumers without a SQL boundary.
use super::*;

#[derive(Default)]
pub struct Pipeline {
    pub bindings: BTreeMap<String, String>,
    pub changes: BTreeMap<String, Delta>,
    roots: BTreeMap<String, String>,
    depths: BTreeMap<String, usize>,
}
impl Pipeline {
    // SQLite expands CTE references during preparation even behind materialized
    // fences. Bound dependent incremental graphs separately from result arms.
    pub const MAX_DEPTH: usize = 3;
    pub fn depth(&self, inputs: &BTreeSet<String>) -> usize {
        inputs.iter().filter_map(|r| self.depths.get(r)).copied().max().unwrap_or(0)
    }
    pub fn retain_from(&mut self, at: usize, last_reads: &BTreeMap<String, usize>) {
        self.bindings.retain(|r,_| last_reads.get(r).is_some_and(|last| *last >= at));
        self.changes.retain(|r,_| self.bindings.contains_key(r));
        self.roots.retain(|r,_| self.bindings.contains_key(r));
        self.depths.retain(|r,_| self.bindings.contains_key(r));
    }
    pub fn unchanged(&mut self, input: &str, result: &str) {
        self.bindings.insert(input.into(), result.into());
        self.roots.insert(input.into(), result.into());
        self.depths.insert(input.into(), 0);
    }
    pub fn native(&mut self, input: &str, result: &str, d: Delta) {
        self.unchanged(input, result);
        if d.inserted + d.deleted > 0 {
            self.changes.insert(input.into(), d);
        }
    }
    pub fn append(
        &mut self,
        phase: &mut phase::Phase,
        input: &str,
        result: &str,
        width: usize,
        keys: &str,
        next: &str,
        full: bool,
        depth: usize,
    ) {
        let old = phase.relation(
            width,
            &format!(
                "SELECT r.* FROM {keys} k CROSS JOIN {} r WHERE {}",
                quote(result),
                same("r", "k", width)
            ),
        );
        let plus = phase.difference_keys(width, next, &old);
        let minus = phase.difference_keys(width, &old, next);
        let value = if full {
            next.into()
        } else {
            phase.view(width, &format!("SELECT r.* FROM {} r WHERE NOT EXISTS(SELECT 1 FROM {keys} k WHERE {}) UNION ALL SELECT * FROM {next}",quote(result),same("r","k",width)))
        };
        self.bindings.insert(input.into(), value);
        self.roots.insert(input.into(), result.into());
        self.depths.insert(input.into(), depth);
        // Cardinality is unknown until the shared statement executes. Nonzero
        // bounds schedule SQL operands; empty CTEs still evaluate no contexts.
        self.changes.insert(
            input.into(),
            Delta {
                plus,
                minus,
                inserted: 1,
                deleted: 1,
            },
        );
    }
    pub fn stage(
        &mut self,
        phase: &mut phase::Phase,
        w: &mut Work<'_>,
        store: &versions::Store,
    ) -> Result<Vec<(String, String, String)>> {
        let mut slots = vec![];
        for (input, d) in &self.changes {
            let width = store.version(&self.roots[input]).columns().len();
            let plus = w.output_table(width)?;
            let minus = w.output_table(width)?;
            phase.load_empty(store, &plus, &d.plus)?;
            phase.load_empty(store, &minus, &d.minus)?;
            slots.push((input.clone(), plus, minus));
        }
        Ok(slots)
    }
    pub fn resume(
        &mut self,
        w: &mut Work<'_>,
        store: &versions::Store,
        snapshots: Vec<(String, versions::Version, versions::Version)>,
    ) -> Result<()> {
        self.bindings = self.roots.clone();
        self.depths.clear();
        self.changes.clear();
        for (input, added, removed) in snapshots {
            let inserted = added.len() as u64;
            let deleted = removed.len() as u64;
            if inserted + deleted == 0 {
                continue;
            }
            let plus = w.output_table(added.columns().len())?;
            let minus = w.output_table(removed.columns().len())?;
            store.clone_scratch(&plus, &added)?;
            store.clone_scratch(&minus, &removed)?;
            self.changes.insert(
                input,
                Delta {
                    plus,
                    minus,
                    inserted,
                    deleted,
                },
            );
        }
        Ok(())
    }
}
