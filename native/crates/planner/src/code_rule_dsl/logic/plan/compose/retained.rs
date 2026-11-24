//! Reuse materialized outputs while leaving internal relational composition in one SQL query.
use super::*;
use crate::code_rule_dsl::logic::native::component;

pub fn run(p: &Placed, db: &Connection, mut inputs: Inputs, lowered: Option<lower::Lowering<'_>>, hooks: &mut Hooks<'_>) -> Result<Outcome> {
    let Some(ks) = hooks.strata.take() else { return stages::run(p,db,inputs,lowered,hooks) };
    let f = p.fields()?;
    let roots: BTreeSet<String> = f.outputs().map(str::to_owned).chain(inputs.kept.iter().cloned()).collect();
    let needed = needed(f,roots.iter().map(String::as_str));
    let sccs = p.components()?;
    // Each component's rules, by key, in the program's order.
    let rules: Vec<Vec<RuleKey>> = sccs.iter().map(|s|(0..p.len()).filter(|&k|s.contains(head(p,k))).map(RuleKey::program).collect()).collect();

    // A composed boundary's provenance is its transitive input facts. Internal CTEs are not
    // exported just to cache them. This also permits a different program to reuse the same output.
    let mut provenance = BTreeMap::new();
    let mut clean = BTreeSet::new();
    for root in &roots {
        let reads = super::needed(f,[root.as_str()]);
        // Native and tree leaves can read document state beyond their explicit demand relation.
        // Until that state has versioned input provenance, do not certify their outputs as unchanged.
        let explicit = !reads.iter().any(|r|f.native(r).is_some()) && !(0..p.len())
            .filter(|&k|reads.contains(head(p,k))).any(|k|p.rule(k).body().any(|l|matches!(l.view(),Lit::Ast{..}|Lit::Nearest{..})));
        let digests: Option<Vec<(String,u64)>> = explicit.then(|| reads.iter().filter(|r|f.is_input(r))
            .map(|r|ks.input(r).map(|d|(r.clone(),d))).collect()).flatten();
        provenance.insert(root.clone(),digests);
    }
    let mut ids = identities(f,&Versions::of_placed(p.clone()),&sccs,&rules);
    ids.retain(|r,_|provenance.get(r).is_some_and(Option::is_some));
    ks.program(&ids);
    for (root,digests) in &provenance {
        if ks.has_rows(root) && digests.as_ref().is_some_and(|ds|ds.iter().all(|(r,d)|ks.read_was(root,r)==Some(*d))) { clean.insert(root.clone()); }
    }
    let mut restored = BTreeMap::new();
    for rel in &clean {
        if ks.has_rows(rel) {
            let types = vec![b'i'; f.width(rel)];
            if let Some((_, held))=ks.rows(rel, &types) {
                component::hold_bound(db,rel,held);
                restored.insert(rel.clone(),component::derived(db,rel).expect("held rows"));
            }
        }
    }
    clean.retain(|r|restored.contains_key(r));
    let cut = stages::inputs_made(p,&clean.iter().cloned().collect::<Vec<_>>())?;
    let cf = cut.fields()?;
    let all = sccs.iter().filter(|s|s.iter().any(|r|needed.contains(r)&&!f.is_input(r))).count();
    let remaining = super::needed(cf,roots.iter().map(String::as_str));
    let ran = sccs.iter().filter(|s|s.iter().any(|r|remaining.contains(r)&&!cf.is_input(r))).count();
    let out = if roots.iter().all(|r|clean.contains(r)) {
        let values = roots.iter().map(|r| {
            let source=&restored[r];
            (r.clone(),(0..source.len()).map(|row|(0..f.width(r)).map(|c|SqlValue::Integer(source.value(row,c))).collect()).collect())
        }).collect();
        let plan=Compiled {sql:String::new(),outputs:roots.iter().cloned().collect(),kept:inputs.kept,reads:Vec::new(),aliases:HashMap::new(),resident:clean.clone(),prepares:Default::default()};
        plan.finish(db,f,lowered,hooks,values)?
    } else {
        for (rel,source) in &restored { stages::bind(&cut,db,&mut inputs,rel,source.clone())?; }
        stages::run_with(&cut,db,inputs,lowered,hooks,clean.clone())?
    };
    for root in roots.iter().filter(|r|!clean.contains(*r)) {
        // The engine's relation, its digest taken where its rows lie (decision 0353 step 8).
        let got=component::kept_rel(db,root).zip(provenance.remove(root).flatten()).and_then(|(rel,reads)| {
            let atoms=crate::content_store::atoms::Atoms::for_connection(db);
            let cells=crate::code_rule_dsl::logic::plan::Cells::new(&atoms,&crate::code_rule_dsl::logic::plan::stored_types(f,root,rel.types().to_vec()));
            let d=crate::code_rule_dsl::logic::plan::source_rows::held_digest(&cells,&rel)?;
            Some((d,rel.types().to_vec(),crate::code_rule_dsl::logic::plan::RanRows::Held(Arc::clone(&rel)),rel.rows(),reads))
        });
        ks.ran(root,got);
    }
    ks.counted(ran,all);
    super::super::super::profile::decision("composed fact reuse",||format!("{} of {} outputs reused; {ran} of {all} strata evaluated in a composed plan",clean.len(),roots.len()));
    Ok(out)
}
