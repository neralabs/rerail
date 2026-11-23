//! Stage independent summary changes for consumers in the same frozen SQL phase.
use super::*;
use combined::publication::{self, Publication};

#[derive(Clone)]
pub struct Summary {
    pub keys: String,
    pub active: String,
    pub changed_query: String,
    pub touched: String,
    pub changed: String,
    pub values: Option<(String,String)>,
}

struct Completion {
    reducer: Reducer,
    before: Option<transition::Pending>,
    summary: Summary,
    shared: bool,
}
struct Initialization {
    reducer: Reducer,
    before: Option<transition::InitialCursor>,
    queries: Vec<String>,
}

#[derive(Default)]
pub struct Batch {
    pub sources: BTreeMap<String, Source>,
    changed: BTreeMap<String, Option<String>>,
    pending: Vec<Completion>,
    initial: Vec<Initialization>,
    publications: Vec<Publication>,
}
pub struct Source {
    pub sql: String,
    pub keys: String,
    pub replacement: String,
}
impl Batch {
    pub fn initialize(
        &mut self,
        r: &Reducer,
        w: &mut Work<'_>,
        objects: &mut Vec<(String,bool)>,
        phase: &mut phase::Phase,
    ) -> Result<()> {
        if !r.shared_owner() || objects.iter().any(|(name,_)|name==&r.groups) { return Ok(()); }
        let before=match transition::prepare_initial(r,w,objects)? {
            transition::InitialPrepared::Reused => return Ok(()),
            transition::InitialPrepared::Evaluate(before) => before,
        };
        let shared=before.as_ref().and_then(|before|self.initial.iter().find(|previous|
            previous.before.as_ref().is_some_and(|p|before.equivalent(p))));
        let mut queries=vec![];
        let store=w.versions.unwrap();
        let bag=(shared.is_none() && r.combined.as_ref().is_some_and(|s|s.layout.min || s.layout.max))
            .then(||phase.relation(r.width(),&r.projection.render()));
        for (at,root) in super::initial::roots(r,bag.as_deref()).into_iter().enumerate() {
            owned_phase(w.db,objects,&root.name,&root.definition,Some(store),true)?;
            let query=shared.map_or_else(||phase.relation(root.width,&root.query),|p|p.queries[at].clone());
            phase.replace_all(store,&root.name,&query)?;
            queries.push(query);
        }
        if shared.is_some() { w.summary_reuses+=1; }
        if w.profiler.is_some() {
            phase.analyses.push(json!({"operation":"scalar summary initialization","group":r.groups,"shared_evaluation":shared.is_some(),"reuse_contract":"Equal projection, layout, and input snapshot; initialization has no prior maintenance cursor"}));
        }
        let keys=phase.view(r.keys(),&format!("SELECT {} FROM {}",names("",r.keys()),queries[0]));
        self.bind(r,&keys,&queries[0]);
        self.initial.push(Initialization {reducer:r.clone(),before,queries});
        Ok(())
    }

    fn bind(&mut self, r: &Reducer, keys: &str, active: &str) {
        let k=r.keys();
        let fields=if r.combined.is_some() {vec!["n","value","high","minimum","maximum"]} else {vec!["n","value"]};
        let renamed=fields.iter().enumerate().map(|(i,name)|format!("c{} AS {name}",k+i)).collect::<Vec<_>>().join(",");
        self.sources.insert(r.groups.clone(),Source {
            sql:format!("(SELECT g.* FROM {} g WHERE ({}) NOT IN(SELECT {} FROM {keys}) UNION ALL SELECT {},{renamed} FROM {active})",quote(&r.groups),names("g",k),names("",k),names("",k)),
            keys:keys.into(),replacement:format!("(SELECT {},{renamed} FROM {active})",names("",k)),
        });
    }

    pub fn append(
        &mut self,
        r: &Reducer,
        w: &mut Work<'_>,
        changes: &BTreeMap<String, Delta>,
        phase: &mut phase::Phase,
    ) -> Result<Option<String>> {
        if r.combined.as_ref().is_some_and(|s| !s.owner) {
            return Ok(None);
        }
        let Some(delta) = changes.get(&r.relation).filter(|d|d.inserted+d.deleted>0) else {
            return Ok(None);
        };
        if let Some(changed) = self.changed.get(&r.groups) {
            return Ok(changed.clone());
        }
        // Unsupported storage keeps the established publication boundary.
        if w.versions.is_none_or(|s| s.available(&r.groups).is_none())
            || r.combined.is_none() && !counts::eligible(r)
        {
            return r.apply(w, changes).map(|result| result.map(|(keys,_)|keys));
        }
        let before = match transition::prepare(r, w, changes)? {
            transition::Prepared::Reused(result) => {
                let changed = result.map(|(keys,_)|keys);
                self.changed.insert(r.groups.clone(), changed.clone());
                return Ok(changed);
            }
            transition::Prepared::Evaluate(before) => before,
        };
        let reusable = before.as_ref().and_then(|before|self.pending.iter().find(|p|
            p.before.as_ref().is_some_and(|previous|before.equivalent(previous))));
        let shared = reusable.is_some();
        let summary = if let Some(previous) = reusable {
            let summary = previous.summary.clone();
            let store=w.versions.unwrap();
            phase.patch_computed(store,&r.groups,&summary.keys,&summary.active)?;
            if let Some((keys,active))=&summary.values {
                phase.patch_computed(store,&r.values,keys,active)?;
            }
            w.summary_reuses+=1;
            if w.profiler.is_some() {
                phase.analyses.push(json!({"operation":"shared staged summary transition","source_group":previous.reducer.groups,"target_group":r.groups,"logicalAnalysis":crate::code_rule_dsl::logic::analysis::scalars::maintenance_equivalent(true,true)}));
            }
            summary
        } else if let Some(shared) = &r.combined {
            combined::phase::append(r, shared, w, delta, phase, &mut self.publications)?
        } else {
            counts::append(r, w, delta, phase)?
        };
        self.bind(r,&summary.keys,&summary.active);
        let changed = summary.changed_query.clone();
        self.changed.insert(r.groups.clone(), Some(changed.clone()));
        self.pending.push(Completion { reducer:r.clone(), before, summary, shared });
        Ok(Some(changed))
    }

    /// Remember transitions only after every output passed atomic publication.
    /// A phase failure leaves both native roots and the transition cache intact.
    pub fn finish(&mut self, w: &mut Work<'_>, counts: &BTreeMap<String, phase::Counts>) -> Result<u64> {
        publication::publish(w, &self.publications, counts)?;
        for p in self.initial.drain(..) {
            transition::remember_initial(&p.reducer,w,p.before);
        }
        let mut groups = 0;
        for p in self.pending.drain(..) {
            let touched = counts[&p.summary.touched].rows as u64;
            if !p.shared { groups += touched; }
            let result = (touched>0).then_some((p.summary.changed,touched));
            transition::remember(&p.reducer, w, p.before, &result)?;
        }
        self.sources.clear();
        self.changed.clear();
        self.publications.clear();
        Ok(groups)
    }
}
