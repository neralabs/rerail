//! Reuse materialized outputs while leaving internal relational composition in one SQL query.
use super::*;
use crate::code_rule_dsl::logic::native::component;

pub fn run(prog: &Program, db: &Connection, mut inputs: Inputs, lowered: Option<&lower::Lowered>, hooks: &mut Hooks<'_>) -> Result<Outcome> {
    let Some(ks) = hooks.strata.take() else { return stages::run(prog,db,inputs,lowered,hooks) };
    let roots: BTreeSet<String> = prog.outputs.iter().chain(&inputs.kept).cloned().collect();
    let needed = super::super::super::front::needed(prog,roots.iter().cloned().collect());
    let sccs = strata(prog)?.0;
    let rules: Vec<Vec<&Rule>> = sccs.iter().map(|s|prog.rules.iter().filter(|r|s.contains(r.head.rel.as_str())).collect()).collect();

    // A composed boundary's provenance is its transitive input facts. Internal CTEs are not
    // exported just to cache them. This also permits a different program to reuse the same output.
    let mut provenance = BTreeMap::new();
    let mut clean = BTreeSet::new();
    for root in &roots {
        let reads = super::super::super::front::needed(prog,vec![root.clone()]);
        // Native and tree leaves can read document state beyond their explicit demand relation.
        // Until that state has versioned input provenance, do not certify their outputs as unchanged.
        let explicit = !reads.iter().any(|r|prog.natives.contains_key(r)) && !prog.rules.iter()
            .filter(|r|reads.contains(r.head.rel.as_str())).any(|r|r.body.iter().any(|l|matches!(l,Literal::Ast{..}|Literal::Nearest{..})));
        let digests: Option<Vec<(String,u64)>> = explicit.then(|| reads.iter().filter(|r|prog.inputs.contains(*r))
            .map(|r|ks.input(r).map(|d|(r.clone(),d))).collect()).flatten();
        provenance.insert(root.clone(),digests);
    }
    let mut ids = identities(prog,&sccs,&rules);
    ids.retain(|r,_|provenance.get(r).is_some_and(Option::is_some));
    ks.program(&ids);
    for (root,digests) in &provenance {
        if ks.has_rows(root) && digests.as_ref().is_some_and(|ds|ds.iter().all(|(r,d)|ks.read_was(root,r)==Some(*d))) { clean.insert(root.clone()); }
    }
    ks.admit(&clean.iter().cloned().collect::<Vec<_>>());
    let mut restored = BTreeMap::new();
    for rel in &clean {
        if ks.has_rows(rel) {
            if let Some((types,flat,n))=ks.rows(rel) {
                component::hold_rows(db,rel,types,flat,n);
                restored.insert(rel.clone(),component::derived(db,rel).expect("held rows"));
            }
        }
    }
    clean.retain(|r|restored.contains_key(r));
    let mut cut = prog.clone();
    cut.rules.retain(|r|!clean.contains(r.head.rel.as_str()));
    for rel in &clean {
        cut.facts.remove(rel);
        cut.natives.remove(rel);
        if !cut.inputs.contains(rel) { cut.inputs.push(rel.clone()); }
    }
    let all = sccs.iter().filter(|s|s.iter().any(|r|needed.contains(r)&&!prog.inputs.contains(r))).count();
    let remaining = super::super::super::front::needed(&cut,roots.iter().cloned().collect());
    let ran = sccs.iter().filter(|s|s.iter().any(|r|remaining.contains(r)&&!cut.inputs.contains(r))).count();
    let out = if roots.iter().all(|r|clean.contains(r)) {
        let values = roots.iter().map(|r| {
            let source=&restored[r];
            (r.clone(),(0..source.len()).map(|row|(0..prog.decls[r].len()).map(|c|SqlValue::Integer(source.value(row,c))).collect()).collect())
        }).collect();
        let plan=Compiled {sql:String::new(),outputs:roots.iter().cloned().collect(),kept:inputs.kept,reads:Vec::new(),aliases:HashMap::new(),resident:clean.clone(),prepares:Default::default()};
        plan.finish(db,prog,lowered,hooks,values)?
    } else {
        for (rel,source) in &restored { stages::bind(&cut,db,&mut inputs,rel,source.clone())?; }
        stages::run_with(&cut,db,inputs,lowered,hooks,clean.clone())?
    };
    for root in roots.iter().filter(|r|!clean.contains(*r)) {
        let got=component::kept_rows(db,root).zip(provenance.remove(root).flatten())
            .map(|((types,flat,n),reads)|(rows_digest(types.len(),&flat,n),types,flat,n,reads));
        ks.ran(root,got);
    }
    ks.counted(ran,all);
    super::super::super::profile::decision("composed fact reuse",||format!("{} of {} outputs reused; {ran} of {all} strata evaluated in a composed plan",clean.len(),roots.len()));
    Ok(out)
}
