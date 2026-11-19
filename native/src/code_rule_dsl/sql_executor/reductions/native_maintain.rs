//! Preserve ordered maintenance fences while storing private state in native roots.
use super::*;

fn union(queries: Vec<String>, width: usize) -> String {
    if queries.is_empty() {format!("SELECT {} WHERE 0",vec!["NULL";width].join(","))}
    else {queries.join(" UNION ")}
}

/// Positive contexts change before scalar callbacks, as in the direct SQL path.
fn positive(rule: &Rule, w: &mut Work<'_>, changes: &BTreeMap<String,Delta>) -> Result<Option<String>> {
    let mut removed=vec![];
    let mut added=vec![];
    for atom in &rule.atoms {
        let Some(delta)=changes.get(&atom.relation) else {continue;};
        if delta.deleted>0 {
            let same=(0..atom.columns.len()).map(|c|format!("c.c{}=d.c{c}",atom.offset+c)).collect::<Vec<_>>().join(" AND ");
            removed.push(format!("SELECT c.* FROM {} d CROSS JOIN {} c WHERE {same}",quote(&delta.minus),quote(&rule.contexts)));
        }
        if delta.inserted>0 {
            let mut query=rule.context_query.clone();
            query.body.from[atom.at].relation=source(&delta.plus,&atom.columns);
            maintenance::anchor(&mut query,atom.at);
            added.push(query.render());
        }
    }
    if removed.is_empty() && added.is_empty() {return Ok(None);}
    let mut phase=phase::Phase::default();
    let removed=phase.relation(rule.width,&union(removed,rule.width));
    let added=phase.relation(rule.width,&union(added,rule.width));
    let dirty=phase.relation(rule.width,&format!("SELECT * FROM {removed} UNION SELECT * FROM {added}"));
    let output=w.output_table(rule.width)?;
    let store=w.versions.unwrap();
    phase.patch_computed(store,&rule.contexts,&dirty,&added)?;
    phase.load_empty(store,&output,&dirty)?;
    phase.execute(w)?;
    Ok((store.available(&output).unwrap().len()>0).then_some(output))
}

pub fn apply(state: &Retained, w: &mut Work<'_>, changes: &BTreeMap<String,Delta>) -> Result<(Delta,u64,u64)> {
    let store=w.versions.unwrap();
    let width=state.width;
    let before=store.available(&state.heads).unwrap();
    let old_heads=w.allocate_readonly(format!("{},n INTEGER NOT NULL CHECK(n>=0),PRIMARY KEY({},n)",schema(width),names("",width)))?;
    if !store.clone_scratch(&old_heads,&before)? {return Err(error("Head support snapshot changed shape"));}
    let mut touched=vec![];
    let (mut contexts,mut groups)=(0,0);
    let _context=w.profile_scope(||json!({"stage":"isolated native support maintenance","logicalAnalysis":state.explain(),"evaluation_fences":["positive contexts","scalar transitions","dependent-key refresh","ordered head evaluation","support reconciliation","lattice fold"]}));
    for rule in state.rules.iter() {
        let mark=w.mark();
        let base=positive(rule,w,changes)?;
        let mut queries=base.into_iter().map(|table|format!("SELECT * FROM {}",quote(&table))).collect::<Vec<_>>();
        for reducer in &rule.reducers {
            if let Some((keys,count))=reducer.apply(w,changes)? {
                groups+=count;
                if store.available(&keys).is_none_or(|v|v.len()>0) {
                    queries.push(reducer.affected_contexts(rule,&keys));
                }
            }
        }
        if queries.is_empty() {w.rewind(mark)?;continue;}
        let mut phase=phase::Phase::default();
        let dirty=phase.relation(rule.width,&union(queries,rule.width));
        let active=phase.relation(rule.width,&format!("SELECT d.* FROM {dirty} d WHERE EXISTS(SELECT 1 FROM {} c WHERE {})",quote(&rule.contexts),equal("c","d",rule.width)));
        let dirty_rows=w.output_table(rule.width)?;
        let active_rows=w.output_table(rule.width)?;
        phase.load_empty(store,&dirty_rows,&dirty)?;
        phase.load_empty(store,&active_rows,&active)?;
        let counts=phase.execute(w)?;
        contexts+=counts[&dirty_rows].rows as u64;
        if counts[&dirty_rows].rows==0 {w.rewind(mark)?;continue;}
        for reducer in &rule.reducers {
            reducer.refresh_dependencies(w,rule,&dirty_rows,&active_rows)?;
        }
        let mut phase=phase::Phase::default();
        let keys=fused::append_support(state,rule,w,&mut phase,&fused::Contexts {dirty:dirty_rows,active:active_rows})?;
        let selected=w.output_table(width)?;
        phase.load_empty(store,&selected,&keys)?;
        phase.execute(w)?;
        let keys=store.available(&selected).unwrap();
        if keys.len()>0 {touched.push(keys);}
        w.rewind(mark)?;
    }
    if touched.is_empty() {
        return Ok((Delta {plus:w.readonly_rows(width,&[])?,minus:w.readonly_rows(width,&[])?,inserted:0,deleted:0},contexts,groups));
    }
    let mut queries=vec![];
    for keys in touched {
        let name=w.output_table(width)?;
        if !store.clone_scratch(&name,&keys)? {return Err(error("Changed support keys changed shape"));}
        queries.push(format!("SELECT * FROM {}",quote(&name)));
    }
    let mut phase=phase::Phase::default();
    let head_keys=phase.relation(width,&union(queries,width));
    let key_width=state.lattice.as_ref().map_or(width,|l|l.keys);
    let keys=phase.relation(key_width,&format!("SELECT DISTINCT {} FROM {head_keys}",names("",key_width)));
    let next=if let Some(lattice)=&state.lattice {
        if let Some(tree)=&lattice.tree {
            let totals=phase.relation(width+2,&format!("SELECT {},coalesce(n.n,0),coalesce(o.n,0) FROM {head_keys} k LEFT JOIN {} n ON {} LEFT JOIN {} o ON {}",names("k",width),quote(&state.heads),equal("n","k",width),quote(&old_heads),equal("o","k",width)));
            let changed=w.allocate_readonly(format!("{},n INTEGER NOT NULL,old_n INTEGER NOT NULL,PRIMARY KEY({})",schema(width),names("",width)))?;
            let selected=w.output_table(key_width)?;
            phase.load_empty(store,&changed,&totals)?;
            phase.load_empty(store,&selected,&keys)?;
            phase.execute(w)?;
            let operation=w.profiler.map(|p|p.operation("registered lattice support index",("sql-extension",0),0,vec![]));
            tree.apply(w.db,&changed,Some(store))?;
            if let Some(operation)=operation {operation.finish_unobserved();}
            phase=phase::Phase::default();
            let keys=phase.view(key_width,&format!("SELECT * FROM {}",quote(&selected)));
            let next=phase.relation(width,&format!("SELECT {},t.summary FROM {keys} k CROSS JOIN {} t WHERE {}",names("k",key_width),quote(&tree.roots),equal("t","k",key_width)));
            return publish(state,w,phase,keys,next,contexts,groups);
        }
        let candidates=format!("FROM {} h WHERE {} AND h.n>0",quote(&state.heads),equal("h","k",key_width));
        let value=format!("h.c{}",width-1);
        let folded=if let Some(aggregate)=&lattice.aggregate {
            format!("(SELECT {aggregate}({value}) {candidates})")
        } else {
            format!("CASE WHEN (SELECT min({value}) {candidates})=(SELECT max({value}) {candidates}) THEN (SELECT min({value}) {candidates}) ELSE {} END",lattice.top)
        };
        phase.relation(width,&format!("SELECT {},{folded} FROM {keys} k WHERE EXISTS(SELECT 1 {candidates})",names("k",key_width)))
    } else {
        phase.relation(width,&format!("SELECT {} FROM {keys} k CROSS JOIN {} h WHERE {} AND h.n>0",names("h",width),quote(&state.heads),equal("h","k",width)))
    };
    publish(state,w,phase,keys,next,contexts,groups)
}

fn publish(state: &Retained,w: &mut Work<'_>,mut phase: phase::Phase,keys:String,next:String,contexts:u64,groups:u64) -> Result<(Delta,u64,u64)> {
    let store=w.versions.unwrap();
    let width=state.width;
    let key_width=state.lattice.as_ref().map_or(width,|l|l.keys);
    let old=phase.relation(width,&format!("SELECT o.* FROM {keys} k CROSS JOIN {} o WHERE {}",quote(&state.output),equal("o","k",key_width)));
    let added=phase.difference_keys(width,&next,&old);
    let removed=phase.difference_keys(width,&old,&next);
    let plus=w.output_table(width)?;
    let minus=w.output_table(width)?;
    phase.load_empty(store,&plus,&added)?;
    phase.load_empty(store,&minus,&removed)?;
    if store.available(&state.output).is_some() {phase.patch_computed(store,&state.output,&keys,&next)?;}
    let counts=phase.execute(w)?;
    let (inserted,deleted)=(counts[&plus].rows as u64,counts[&minus].rows as u64);
    if store.available(&state.output).is_none() {
        if deleted>0 {execute(w.db,&format!("DELETE FROM {} WHERE ({}) IN(SELECT {} FROM {})",quote(&state.output),names("",width),names("",width),quote(&minus)))?;}
        if inserted>0 {execute(w.db,&format!("INSERT INTO {} SELECT * FROM {}",quote(&state.output),quote(&plus)))?;}
    }
    Ok((Delta {plus,minus,inserted,deleted},contexts,groups))
}
