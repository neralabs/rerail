//! Bound SQLite's expansion of a CTE graph, without scheduling one query per relation.
//!
//! A stage's program is a version (t36 item 7, step 6): each edit (relations made inputs, rules proved empty, the
//! outputs a part publishes) an overlay of the version before it, made with the program image's builder.
use super::*;
use crate::code_rule_dsl::logic::program_image::handles::Terms;
use crate::code_rule_dsl::logic::program_image::{TermRef, Version};
use crate::code_rule_dsl::{facts::vtab, logic::native::component};

// SQLite counts references while copying CTE definitions even when they are MATERIALIZED.
// Leave room below its 65,535-reference limit for local walks and planner rewrites.
const REFERENCES: usize = 8192;
const STAGES: usize = 32;

pub(super) fn bind(p: &Placed, db: &Connection, inputs: &mut Inputs, rel: &str, source: Arc<dyn vtab::Source>) -> Result<()> {
    if inputs.given.is_none() { inputs.given = Some(Stats::read(db,p,inputs,false)?); }
    let stats = inputs.given.as_mut().unwrap();
    stats.measured(rel,vtab::syntax::statistics(source.as_ref()).into_iter().map(|(c,r,n,s,d)|(c,[r,n,s,d])),[]);
    stats.card.insert(rel,source.len() as f64);
    if !vtab::replace_bound_source(db,rel,source.clone())? { connection::bind(db,rel,source)?; }
    Ok(())
}

/// `p` edited by `b`, a builder on `p.v` that only removes rules (each rule left where it was) and states parts. The
/// maintained program's parts stay as they were: no edit here touches them, as the tree's edits did not.
fn edited(_: &Placed, b: Builder<'_>) -> Result<Placed> {
    let (v, _) = b.finish().map_err(Error::Plan)?;
    let ids = v.rules().map(|r| r.id()).collect();
    Ok(Placed::new(v, ids, true))
}

/// `p` with `rels` made inputs, one edit: their rules, facts and natives gone, and each an input after those it has.
pub(super) fn inputs_made(p: &Placed, rels: &[String]) -> Result<Placed> {
    if rels.is_empty() {
        return Ok(p.clone());
    }
    let f = p.fields()?;
    let heads: BTreeSet<RelId> = rels.iter().filter_map(|r| f.v.rel(r)).collect();
    let mut b = Builder::on(&p.v);
    b.retain(|r| !heads.contains(&r.head().rel()));
    let mut inputs: Vec<&str> = f.inputs().collect();
    for rel in rels {
        if f.facts(rel).is_some() {
            b.drop_facts(rel);
        }
        if f.native(rel).is_some() {
            b.drop_native(rel);
        }
        if !inputs.contains(&rel.as_str()) {
            inputs.push(rel);
        }
    }
    if inputs.len() != f.inputs().len() {
        b.set_inputs(&inputs);
    }
    edited(p, b)
}

/// `p` less its rules `gone` (by their places).
fn without(p: &Placed, gone: &[usize]) -> Result<Placed> {
    let mut b = Builder::on(&p.v);
    for &k in gone {
        b.remove(p.id(k));
    }
    edited(p, b)
}

/// `p` with outputs `outs`, in order.
fn with_outputs(p: &Placed, outs: &[String]) -> Result<Placed> {
    let mut b = Builder::on(&p.v);
    b.set_outputs(outs);
    edited(p, b)
}

/// Prove empty positive joins before planning. Negation and aggregates are never treated as
/// empty merely because their input is empty.
fn simplify(p: &mut Placed, db: &Connection) -> Result<()> {
    let mut empty: BTreeSet<String> = p.fields()?.inputs().filter(|r|vtab::bound_source(db,r).is_some_and(|s|s.len()==0)).map(str::to_owned).collect();
    loop {
        let before = (empty.len(),p.len(),p.fields()?.inputs().len());
        let gone: Vec<usize> = (0..p.len()).filter(|&k| p.rule(k).body().any(|l|matches!(l.view(),Lit::Atom(a) if !a.neg() && empty.contains(p.v.rel_name(a.rel()))))).collect();
        if !gone.is_empty() { *p = without(p,&gone)?; }
        let f = p.fields()?;
        for d in f.decls() {
            let rel = f.text(d.name());
            if !f.is_input(rel) && f.native(rel).is_none() && !f.has_facts(rel) && !f.derives(rel) {
                empty.insert(rel.to_owned());
            }
        }
        if before==(empty.len(),p.len(),p.fields()?.inputs().len()) { break; }
    }
    Ok(())
}

impl Compiled {
    /// A topological prefix whose expanded definitions and output reads fit the budget.
    /// Export only values used after the cut, together in one statement.
    pub(super) fn boundary(&self) -> Option<BTreeSet<String>> {
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

pub(super) fn run(p: &Placed, db: &Connection, inputs: Inputs, lowered: Option<lower::Lowering<'_>>, hooks: &mut Hooks<'_>) -> Result<Outcome> {
    run_with(p,db,inputs,lowered,hooks,BTreeSet::new())
}

/// Whether terms `a` and `b` of one rule of `v` are equal, term by term (`front::Term`'s equality).
fn same_terms(v: &Version, mut a: Terms<'_>, mut b: Terms<'_>) -> bool {
    loop {
        match (a.next(), b.next()) {
            (None, None) => return true,
            (Some(x), Some(y)) if same_term(v, x, y) => {}
            _ => return false,
        }
    }
}

fn same_term(v: &Version, x: TermRef<'_>, y: TermRef<'_>) -> bool {
    match (x, y) {
        (TermRef::Var(s), TermRef::Var(t)) => s == t,
        (TermRef::Wild, TermRef::Wild) | (TermRef::Top, TermRef::Top) => true,
        (TermRef::Int(i), TermRef::Int(j)) => i == j,
        (TermRef::Branded(a), TermRef::Branded(b)) => a == b,
        (TermRef::Str(a), TermRef::Str(b)) => v.text(a) == v.text(b),
        (TermRef::Call(f, xs), TermRef::Call(g, ys)) => v.text(f) == v.text(g) && same_terms(v, xs, ys),
        _ => false,
    }
}

pub(super) fn run_with(p: &Placed, db: &Connection, mut inputs: Inputs, lowered: Option<lower::Lowering<'_>>, hooks: &mut Hooks<'_>, resident: BTreeSet<String>) -> Result<Outcome> {
    let mut remaining=p.clone();
    simplify(&mut remaining,db)?;
    run_stages(&remaining,db,&mut inputs,lowered,hooks,resident,&mut 0)
}

fn run_stages(p: &Placed, db: &Connection, inputs: &mut Inputs, lowered: Option<lower::Lowering<'_>>, hooks: &mut Hooks<'_>, mut resident: BTreeSet<String>, executions: &mut usize) -> Result<Outcome> {
    let mut remaining=p.clone();
    while *executions<STAGES {
        simplify(&mut remaining,db)?;
        let mut plan=planned(&remaining,db,inputs)?;
        plan.resident=resident.clone();
        let Some(boundary)=plan.boundary() else {
            *executions+=1;
            return plan.execute(db,remaining.fields()?,lowered,hooks)
        };
        if plan.prepares(db)? {
            *executions+=1;
            return plan.execute(db,remaining.fields()?,lowered,hooks)
        }
        if boundary.is_empty() { return Err(Error::Plan("a composed relation exceeds SQLite's reference budget; it needs an explicit physical boundary".into())); }
        let rels: Vec<String>=boundary.iter().cloned().collect();
        let part=with_outputs(&remaining,&rels)?;
        let kept=std::mem::replace(&mut inputs.kept,boundary.clone());
        let query=planned(&part,db,inputs);
        inputs.kept=kept;
        let query=query?;
        if query.boundary().is_some() { return Err(Error::Plan("composed materialization does not fit the reference budget".into())); }
        *executions+=1;
        super::super::super::profile::decision("composed materialization",||format!("stage {}: {} shared relations; bounded CTE expansion",executions,boundary.len()));
        query.execute(db,part.fields()?,None,&mut Hooks::default())?;
        for rel in boundary {
            bind(&remaining,db,inputs,&rel,component::derived(db,&rel).expect("materialized boundary"))?;
            resident.insert(rel);
        }
        remaining=inputs_made(&remaining,&rels)?;
    }
    Err(Error::Plan(format!("composed job exceeds the {STAGES}-stage budget")))
}
