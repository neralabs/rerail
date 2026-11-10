//! Bound SQL result arms without changing frozen wave inputs or losing key journals.
use super::*;

#[derive(Default)]
pub(super) struct Batches {
    pub counts: BTreeMap<String, phase::Counts>,
    pub phases: usize,
    pub cache_batches: usize,
    decisions: usize,
    keys: BTreeMap<String, Vec<versions::Version>>,
}
impl Batches {
    pub fn execute(
        &mut self,
        phase: &mut phase::Phase,
        w: &Work<'_>,
        decisions: &[Value],
    ) -> Result<()> {
        if phase.arm_bound() == 0 {
            return Ok(());
        }
        if w.profiler.is_some() {
            phase
                .analyses
                .extend(decisions[self.decisions..].iter().cloned());
        }
        self.decisions = decisions.len();
        let _context=w.profile_scope(||json!({"phaseBatch":self.phases,"waveInputs":"Frozen until every operator cache has evaluated"}));
        self.counts.extend(std::mem::take(phase).execute(w)?);
        self.phases += 1;
        Ok(())
    }
    pub fn cache(
        &mut self,
        phase: &mut phase::Phase,
        touched: &mut BTreeMap<String, Vec<String>>,
        relations: &BTreeMap<String, Relation>,
        w: &mut Work<'_>,
        store: &versions::Store,
        decisions: &[Value],
    ) -> Result<()> {
        let mut slots = vec![];
        for (r, queries) in touched.iter_mut() {
            let mut local = vec![];
            for source in queries.drain(..) {
                if phase.defines(&source) {
                    local.push(source);
                } else if let Some(version) = store.available(&source) {
                    if version.len() > 0 {
                        let roots=self.keys.entry(r.clone()).or_default();
                        if !roots.iter().any(|old|old.same(&version)) {roots.push(version);}
                    }
                } else {
                    return Err(error("wave key journal has no immutable native source"));
                }
            }
            if !local.is_empty() {
                let width = relations[r].keys;
                let keys = phase.union_keys(width, &local);
                let unique = phase.relation(
                    width,
                    &format!("SELECT DISTINCT {} FROM {keys}", names("", width)),
                );
                let slot = w.output_table(width)?;
                phase.load_empty(store, &slot, &unique)?;
                slots.push((r.clone(), slot));
            }
        }
        self.execute(phase, w, decisions)?;
        for (r, slot) in slots {
            let version = store
                .available(&slot)
                .ok_or_else(|| error("missing staged wave keys"))?;
            if version.len() > 0 {
                self.keys.entry(r).or_default().push(version);
            }
        }
        self.cache_batches += 1;
        Ok(())
    }
    pub fn bind(
        &self,
        touched: &mut BTreeMap<String, Vec<String>>,
        w: &mut Work<'_>,
        store: &versions::Store,
    ) -> Result<()> {
        for (r, versions) in &self.keys {
            for version in versions {
                let slot = w.output_table(version.columns().len())?;
                if !store.clone_scratch(&slot, version)? {
                    return Err(error("staged wave key schema changed"));
                }
                touched.get_mut(r).unwrap().push(slot);
            }
        }
        Ok(())
    }
}
