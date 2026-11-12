//! Bound SQLite's expansion of a CTE graph, without scheduling one query per relation.
use super::*;
use crate::code_rule_dsl::{facts::vtab, logic::native::component};

// SQLite counts references while copying CTE definitions even when they are MATERIALIZED.
// Leave room below its 65,535-reference limit for local walks and planner rewrites.
const REFERENCES: usize = 8192;
const STAGES: usize = 32;

pub fn bind(prog: &Program, db: &Connection, inputs: &mut Inputs, rel: &str, source: Arc<dyn vtab::Source>) -> Result<()> {
    if inputs.given.is_none() { inputs.given = Some(Stats::read(db,prog,inputs,false)?); }
    let stats = inputs.given.as_mut().unwrap();
    stats.measured(rel,vtab::syntax::statistics(source.as_ref()).into_iter().map(|(c,r,n,s,d)|(c,[r,n,s,d])),[]);
    stats.card.insert(rel.to_owned(),source.len() as f64);
    if !vtab::replace_bound_source(db,rel,source.clone())? { connection::bind(db,rel,source)?; }
    Ok(())
}

fn input(prog: &mut Program, rel: &str) {
    prog.rules.retain(|r|r.head.rel!=rel);
    prog.facts.remove(rel);
    prog.natives.remove(rel);
    if !prog.inputs.iter().any(|r|r==rel) { prog.inputs.push(rel.to_owned()); }
}

/// Prove empty positive joins before planning. Negation and aggregates are never treated as
/// empty merely because their input is empty. A quiet maintenance relation shares its old fact.
fn simplify(prog: &mut Program, db: &Connection, inputs: &mut Inputs, resident: &mut BTreeSet<String>) -> Result<()> {
    let mut empty: BTreeSet<String> = prog.inputs.iter().filter(|r|vtab::bound_source(db,r).is_some_and(|s|s.len()==0)).cloned().collect();
    loop {
        let before = (empty.len(),prog.rules.len(),prog.inputs.len());
        prog.rules.retain(|r|!r.body.iter().any(|l|matches!(l,Literal::Atom(a) if !a.neg && empty.contains(a.rel.as_str()))));
        for rel in prog.decls.keys() {
            if !prog.inputs.contains(rel) && !prog.natives.contains_key(rel)
                && prog.facts.get(rel).is_none_or(Vec::is_empty) && !prog.rules.iter().any(|r|r.head.rel==*rel) {
                empty.insert(rel.clone());
            }
        }
        let native: BTreeSet<_> = prog.inputs.iter().cloned().collect();
        let mut quiet = Vec::new();
        for scc in strata(prog)?.0 {
            if scc.len()!=1 { continue; }
            let rel=scc.iter().next().unwrap();
            if prog.inputs.contains(rel) || prog.natives.contains_key(rel) || prog.facts.get(rel).is_some_and(|f|!f.is_empty()) { continue; }
            let rules: Vec<_> = prog.rules.iter().filter(|r|r.head.rel==*rel).collect();
            if let [rule] = rules.as_slice() {
                if let Some((old,gone)) = survivors(rule,&scc,&native) {
                    if empty.contains(&gone) {
                        if let Some(held)=component::held(db,&old).filter(|h|h.none_stored()) { quiet.push((rel.clone(),held)); }
                    }
                }
            }
        }
        for (rel,held) in quiet {
            component::hold_as(db,&rel,held);
            bind(prog,db,inputs,&rel,component::derived(db,&rel).unwrap())?;
            input(prog,&rel);
            resident.insert(rel.clone());
            QUIET_STRATA.fetch_add(1,std::sync::atomic::Ordering::Relaxed);
            super::super::super::profile::decision("composed quiet relation",||format!("{rel}: shares its retained rows; no deletions or insertions"));
        }
        if before==(empty.len(),prog.rules.len(),prog.inputs.len()) { break; }
    }
    Ok(())
}

impl Compiled {
    /// A topological prefix whose expanded definitions and output reads fit the budget.
    /// Export only values used after the cut, together in one statement.
    pub fn boundary(&self) -> Option<BTreeSet<String>> {
        let mut costs = HashMap::<String,usize>::new();
        let mut used=0usize;
        let mut prefix=BTreeSet::new();
        for (rel,reads) in &self.reads {
            let cost=reads.iter().filter(|(r,_)|*r!=rel).fold(1usize,|a,(r,n)|a.saturating_add(costs.get(r).copied().unwrap_or(1).saturating_mul(*n)));
            // Account for both the definition and an exported read of every relation.
            if used.saturating_add(cost.saturating_mul(2))>REFERENCES {
                let outside: BTreeSet<_> = self.reads.iter().filter(|(r,_)|!prefix.contains(r)).flat_map(|(_,ds)|ds.keys().cloned()).chain(self.outputs.iter().cloned()).collect();
                return Some(prefix.intersection(&outside).cloned().collect());
            }
            used+=cost*2;
            costs.insert(rel.clone(),cost);
            prefix.insert(rel.clone());
        }
        None
    }
}

pub fn run(prog: &Program, db: &Connection, inputs: Inputs, lowered: Option<&lower::Lowered>, hooks: &mut Hooks<'_>) -> Result<Outcome> {
    run_with(prog,db,inputs,lowered,hooks,BTreeSet::new())
}

/// A session's kept outputs can share their old fact when their change sets are empty.
/// Expose those change sets together, not one scheduler query for each stratum.
fn changes(prog: &Program, inputs: &Inputs) -> BTreeSet<String> {
    let roots: BTreeSet<_> = prog.outputs.iter().chain(&inputs.kept).cloned().collect();
    let native: BTreeSet<_> = prog.inputs.iter().cloned().collect();
    let mut out=BTreeSet::new();
    for rel in roots {
        if prog.inputs.contains(&rel) || prog.facts.get(&rel).is_some_and(|f|!f.is_empty()) { continue; }
        let scc=BTreeSet::from([rel.clone()]);
        let rules: Vec<_> = prog.rules.iter().filter(|r|r.head.rel==rel).collect();
        let Some(gone)=rules.iter().find_map(|r|survivors(r,&scc,&native).map(|(_,g)|g)) else { continue };
        let mut demands=BTreeSet::from([gone]);
        let simple=rules.iter().all(|r| {
            if survivors(r,&scc,&native).is_some() { return true; }
            let [Literal::Atom(a)]=r.body.as_slice() else { return false };
            if a.neg || a.args!=r.head.args { return false; }
            demands.insert(a.rel.to_string()); true
        });
        // Do not split a recursive component across a publication boundary.
        let reads=super::super::super::front::needed(prog,demands.iter().cloned().collect());
        if simple && !reads.contains(&rel) { out.extend(demands.into_iter().filter(|r|!prog.inputs.contains(r))); }
    }
    out
}

pub(super) fn run_with(prog: &Program, db: &Connection, mut inputs: Inputs, lowered: Option<&lower::Lowered>, hooks: &mut Hooks<'_>, mut resident: BTreeSet<String>) -> Result<Outcome> {
    let mut remaining=prog.clone();
    simplify(&mut remaining,db,&mut inputs,&mut resident)?;
    let changes=changes(&remaining,&inputs);
    let mut executions=0;
    if !changes.is_empty() {
        let mut part=remaining.clone();
        part.outputs=changes.iter().cloned().collect();
        let kept=std::mem::replace(&mut inputs.kept,changes.clone());
        super::super::super::profile::decision("composed change admission",||format!("{} retained change relations requested together",changes.len()));
        let got=run_stages(&part,db,&mut inputs,None,&mut Hooks::default(),resident.clone(),&mut executions);
        inputs.kept=kept;
        got?;
        for rel in changes {
            bind(&remaining,db,&mut inputs,&rel,component::derived(db,&rel).expect("retained changes"))?;
            input(&mut remaining,&rel);
            resident.insert(rel);
        }
    }
    run_stages(&remaining,db,&mut inputs,lowered,hooks,resident,&mut executions)
}

fn run_stages(prog: &Program, db: &Connection, inputs: &mut Inputs, lowered: Option<&lower::Lowered>, hooks: &mut Hooks<'_>, mut resident: BTreeSet<String>, executions: &mut usize) -> Result<Outcome> {
    let mut remaining=prog.clone();
    while *executions<STAGES {
        simplify(&mut remaining,db,inputs,&mut resident)?;
        let mut plan=planned(&remaining,db,inputs)?;
        plan.resident=resident.clone();
        let Some(boundary)=plan.boundary() else {
            *executions+=1;
            return plan.execute(db,&remaining,lowered,hooks)
        };
        if plan.prepares(db)? {
            *executions+=1;
            return plan.execute(db,&remaining,lowered,hooks)
        }
        if boundary.is_empty() { return Err(Error::Plan("a composed relation exceeds SQLite's reference budget; it needs an explicit physical boundary".into())); }
        let mut part=remaining.clone();
        part.outputs=boundary.iter().cloned().collect();
        let kept=std::mem::replace(&mut inputs.kept,boundary.clone());
        let query=planned(&part,db,inputs);
        inputs.kept=kept;
        let query=query?;
        if query.boundary().is_some() { return Err(Error::Plan("composed materialization does not fit the reference budget".into())); }
        *executions+=1;
        super::super::super::profile::decision("composed materialization",||format!("stage {}: {} shared relations; bounded CTE expansion",executions,boundary.len()));
        query.execute(db,&part,None,&mut Hooks::default())?;
        for rel in boundary {
            bind(&remaining,db,inputs,&rel,component::derived(db,&rel).expect("materialized boundary"))?;
            input(&mut remaining,&rel);
            resident.insert(rel);
        }
    }
    Err(Error::Plan(format!("composed job exceeds the {STAGES}-stage budget")))
}
