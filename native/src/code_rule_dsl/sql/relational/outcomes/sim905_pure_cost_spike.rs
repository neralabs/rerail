//! Pure self-tail helper summaries, without a Boolean/call control machine.
//! Event edges are child-search calls actually reached according to total truth.
#[path = "sim905_literal_cost_spike.rs"]
mod literal;
use super::{Compiled, cost_relation_spike};
use crate::code_rule_dsl::matcher::{AstMatcher, GrammarMatcher, StopBy, UtilRef};
use serde_json::json;
use std::sync::Arc;
fn calls_self(m: &AstMatcher, u: &UtilRef) -> bool {
    match m {
        AstMatcher::Util(v) => v.index == u.index && v.table.as_ptr() == u.table.as_ptr(),
        AstMatcher::All(v) | AstMatcher::Any(v) => v.iter().any(|m| calls_self(m, u)),
        AstMatcher::Not(m) => calls_self(m, u),
        AstMatcher::Has(r, _) => calls_self(r.rule(), u),
        _ => false,
    }
}
#[derive(Clone, Copy)]
struct Abstract {
    truth: i64,
    cost: i64,
    peak: u32,
}
/// Compile-time metadata partial evaluation, never an AST-node evaluator.
fn abstract_kind(
    m: &AstMatcher,
    kind: Option<u16>,
    incoming: u32,
    g: &GrammarMatcher,
    active: &mut std::collections::HashSet<(usize, usize)>,
) -> Option<Abstract> {
    let zero = |truth| Abstract {
        truth,
        cost: 0,
        peak: 0,
    };
    match m {
        AstMatcher::Never => Some(zero(0)),
        AstMatcher::Kind(_) => {
            let p = super::lower::Lower::new(g.language, true).plan(m).ok()?;
            let super::Op::Predicate(super::Set::Kinds(ids)) = p.op else {
                return None;
            };
            Some(zero(i64::from(kind.is_some_and(|k| ids.contains(&k)))))
        }
        AstMatcher::All(v) | AstMatcher::Any(v) => {
            let all = matches!(m, AstMatcher::All(_));
            let decisive = if all { 0 } else { 1 };
            let mut out = zero(if all { 1 } else { 0 });
            for child in v {
                let a = abstract_kind(child, kind, incoming, g, active)?;
                out.cost += a.cost;
                out.peak = out.peak.max(a.peak);
                if a.truth < 0 || a.truth == decisive {
                    out.truth = a.truth;
                    return Some(out);
                }
            }
            Some(out)
        }
        AstMatcher::Not(child) => {
            let mut a = abstract_kind(child, kind, incoming, g, active)?;
            a.truth = match a.truth {
                0 => 1,
                1 => 0,
                t => t,
            };
            Some(a)
        }
        AstMatcher::Util(u) => {
            if incoming >= 256 {
                return Some(Abstract {
                    truth: -1,
                    cost: 0,
                    peak: 0,
                });
            }
            let key = (u.table.as_ptr() as usize, u.index);
            if !active.insert(key) {
                return None;
            }
            let answer = u
                .with_target(|m| abstract_kind(m, kind, incoming + 1, g, active))
                .flatten();
            active.remove(&key);
            answer.map(|a| Abstract {
                truth: a.truth,
                cost: a.cost + 1,
                peak: (a.peak + 1).max(1),
            })
        }
        _ => None,
    }
}
fn collect_kinds(
    m: &AstMatcher,
    g: &GrammarMatcher,
    seen: &mut std::collections::HashSet<(usize, usize)>,
    out: &mut std::collections::BTreeSet<u16>,
) -> Result<(), String> {
    match m {
        AstMatcher::Kind(_) => {
            let p = super::lower::Lower::new(g.language, true).plan(m)?;
            let super::Op::Predicate(super::Set::Kinds(ids)) = p.op else {
                return Err("kind stratum proof failed".into());
            };
            out.extend(ids);
        }
        AstMatcher::All(v) | AstMatcher::Any(v) => {
            for m in v {
                collect_kinds(m, g, seen, out)?
            }
        }
        AstMatcher::Not(m) => collect_kinds(m, g, seen, out)?,
        AstMatcher::Has(r, _) => collect_kinds(r.rule(), g, seen, out)?,
        AstMatcher::Util(u) => {
            let key = (u.table.as_ptr() as usize, u.index);
            if seen.insert(key) {
                u.with_target(|m| collect_kinds(m, g, seen, out))
                    .ok_or("kind source missing")??;
            }
        }
        _ => {}
    }
    Ok(())
}
/// Offsets only atomic regex terminals accepted by the pure summary compiler.
pub(super) fn offset(sql: &str, offset: usize, count: usize) -> String {
    let mut s = sql.to_owned();
    for id in 0..count {
        s = s.replace(
            &format!("node_regex({id},"),
            &format!("node_regex(@SPIKE{}@,", id + offset),
        );
    }
    for id in offset..offset + count {
        s = s.replace(&format!("@SPIKE{id}@"), &id.to_string());
    }
    s
}
/// `name` identifies the explicit helper in this experiment's source manifest.
/// Source proof still checks its exact pure singleton structural-self-tail form.
pub(super) fn compile(
    grammar: &GrammarMatcher,
    name: &str,
    demand: &str,
    incoming: u32,
) -> Result<Compiled, String> {
    compile_mode(grammar, name, demand, incoming, false)
}
#[cfg(test)]
pub(super) fn compile_control(
    grammar: &GrammarMatcher,
    name: &str,
    demand: &str,
    incoming: u32,
) -> Result<Compiled, String> {
    compile_mode(grammar, name, demand, incoming, true)
}
fn compile_mode(
    grammar: &GrammarMatcher,
    name: &str,
    demand: &str,
    incoming: u32,
    generic: bool,
) -> Result<Compiled, String> {
    let index = grammar
        .utils
        .names
        .iter()
        .position(|n| n == name)
        .ok_or("pure helper missing")?;
    let util = UtilRef {
        table: Arc::downgrade(&grammar.utils),
        index,
    };
    let original = util
        .with_target(Clone::clone)
        .ok_or("pure helper unavailable")?;
    let AstMatcher::Any(items) = original else {
        return Err("pure helper needs ordered alternatives".into());
    };
    if items.is_empty()
        || items[..items.len() - 1]
            .iter()
            .any(|m| calls_self(m, &util))
    {
        return Err("pure helper needs one final self arm".into());
    }
    let tail = items.last().unwrap();
    fn exact_tail(m: &AstMatcher, u: &UtilRef) -> bool {
        match m {
        AstMatcher::All(v)=>v.iter().all(|m|matches!(m,AstMatcher::Kind(_))||matches!(m,AstMatcher::Has(r,StopBy::Neighbor) if matches!(r.rule(),AstMatcher::Util(v) if v.index==u.index&&v.table.as_ptr()==u.table.as_ptr()))),
        _=>false,
    }
    }
    if !exact_tail(tail, &util) {
        return Err("pure helper tail has additional work/conditions".into());
    }
    let AstMatcher::All(tail_items) = tail else {
        unreachable!()
    };
    if tail_items.len() != 2
        || !matches!(
            (&tail_items[0], &tail_items[1]),
            (AstMatcher::Kind(_), AstMatcher::Has(_, StopBy::Neighbor))
        )
    {
        return Err("pure helper requires kind guard before child call".into());
    }
    let paren = grammar.language.kinds().kind_id("parenthesized_expression");
    let guard = super::lower::Lower::new(grammar.language, true).plan(&tail_items[0])?;
    if !matches!(guard.op,super::Op::Predicate(super::Set::Kinds(ref kinds)) if kinds==&vec![paren])
    {
        return Err("pure helper requires parenthesis guard".into());
    }
    let bases = items[..items.len() - 1].to_vec();
    if bases.iter().any(|m| calls_self(m, &util)) {
        return Err("pure helper cyclic base".into());
    }
    let mut base = grammar.clone();
    base.matcher = AstMatcher::Any(bases);
    fn depth(
        m: &AstMatcher,
        active: &mut std::collections::HashSet<(usize, usize)>,
    ) -> Result<u32, String> {
        match m {
            AstMatcher::Kind(_) | AstMatcher::Regex(_) | AstMatcher::Never => Ok(0),
            AstMatcher::All(v) | AstMatcher::Any(v) => v
                .iter()
                .map(|m| depth(m, active))
                .try_fold(0, |a, b| b.map(|b| a.max(b))),
            AstMatcher::Not(m) => depth(m, active),
            AstMatcher::Has(r, StopBy::Neighbor) => depth(r.rule(), active),
            AstMatcher::Util(u) => {
                let k = (u.table.as_ptr() as usize, u.index);
                if !active.insert(k) {
                    return Err("pure base cycle".into());
                }
                let d = u
                    .with_target(|m| depth(m, active))
                    .ok_or("pure base missing")?;
                active.remove(&k);
                Ok(d? + 1)
            }
            _ => Err("pure base effect outside contract".into()),
        }
    }
    let max_depth = depth(&base.matcher, &mut std::collections::HashSet::new())?;
    if max_depth > 8 {
        return Err("pure base exceeds bounded acyclic call depth8".into());
    }
    let cutoff = 257 - max_depth;
    // The scalar producer derives atom/call order from this original AST.
    let mut definitions = vec![
        format!("requested AS MATERIALIZED ({demand})"),
        format!(
            "helper_nodes(file,node) AS (SELECT file,node FROM requested UNION SELECT c.file,c.id FROM helper_nodes h JOIN node p ON p.file=h.file AND p.id=h.node JOIN node c ON c.file=p.file AND c.parent=p.id WHERE p.kind={paren})"
        ),
        format!(
            "shape(file,node,level) AS (SELECT file,node,0 FROM requested UNION ALL SELECT c.file,c.id,s.level+1 FROM shape s JOIN node p ON p.file=s.file AND p.id=s.node JOIN node c ON c.file=p.file AND c.parent=p.id WHERE p.kind={paren} AND s.level<256)"
        ),
    ];
    definitions.push(format!("base_inputs AS MATERIALIZED(SELECT file,node,0 incoming FROM helper_nodes UNION SELECT DISTINCT file,node,level+{incoming}+1 FROM shape WHERE level+{incoming}+1>={cutoff} AND level+{incoming}+1<=256)"));
    let mut known = std::collections::BTreeSet::new();
    collect_kinds(
        &base.matcher,
        grammar,
        &mut std::collections::HashSet::new(),
        &mut known,
    )?;
    let known_list = known
        .iter()
        .map(u16::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let mut guards = vec![];
    let mut constant_rows = vec![];
    let mut strata = vec![];
    for kind in known.iter().copied().map(Some).chain(std::iter::once(None)) {
        let Some(normal) = abstract_kind(
            &base.matcher,
            kind,
            0,
            grammar,
            &mut std::collections::HashSet::new(),
        ) else {
            continue;
        };
        let guard = kind.map_or_else(
            || {
                if known.is_empty() {
                    "1".into()
                } else {
                    format!("n.kind NOT IN({known_list})")
                }
            },
            |k| format!("n.kind={k}"),
        );
        let mut levels = vec![];
        for level in 257 - normal.peak..=256 {
            let a = abstract_kind(
                &base.matcher,
                kind,
                level,
                grammar,
                &mut std::collections::HashSet::new(),
            )
            .ok_or("metadata stratum loses critical-depth proof")?;
            levels.push((level, a));
        }
        let field = |which: u8| {
            let value = |a: Abstract| match which {
                0 => a.truth,
                1 => a.cost,
                _ => i64::from(a.peak),
            };
            if levels.is_empty() {
                value(normal).to_string()
            } else {
                format!(
                    "CASE WHEN i.incoming<={} THEN {} {} END",
                    256 - normal.peak,
                    value(normal),
                    levels
                        .iter()
                        .map(|(level, a)| format!("WHEN i.incoming={level} THEN {}", value(*a)))
                        .collect::<Vec<_>>()
                        .join(" ")
                )
            }
        };
        constant_rows.push(format!("SELECT i.file,i.node,i.incoming,{} truth,{} cost,{} peak FROM base_inputs i JOIN node n ON n.file=i.file AND n.id=i.node WHERE {guard}",field(0),field(1),field(2)));
        strata.push(json!({"kind":kind,"normalBaseTruth":normal.truth,"normalBaseCost":normal.cost,"normalBasePeak":normal.peak,"proof":"compile-time original AST metadata partial evaluation; no per-node Rust evaluation"}));
        guards.push(format!("({guard})"));
    }
    let dynamic_demand = format!(
        "SELECT i.file,i.node,i.incoming FROM base_inputs i JOIN node n ON n.file=i.file AND n.id=i.node WHERE NOT({})",
        if guards.is_empty() {
            "0".into()
        } else {
            guards.join(" OR ")
        }
    );
    let dynamic = if generic {
        let d = cost_relation_spike::compile_summary_dynamic(
            &base,
            &dynamic_demand,
            &format!("base_{name}"),
        )?;
        literal::Projection {
            definitions: d.definitions,
            select: d.select,
            terminals: d.terminals,
            proof: json!({"genericDynamicControl":true}),
        }
    } else {
        literal::compile(
            grammar,
            &base.matcher,
            &dynamic_demand,
            &format!("base_{name}"),
        )?
    };
    let projection_proof = dynamic.proof;
    let terminals = dynamic.terminals;
    definitions.extend(dynamic.definitions);
    let mut base_rows = vec![dynamic.select];
    base_rows.extend(constant_rows);
    definitions.push(format!(
        "base_all AS MATERIALIZED ({})",
        base_rows.join(" UNION ALL ")
    ));
    definitions.push("demand AS MATERIALIZED (SELECT file,node FROM requested)".into());
    definitions.push(format!("helper_truth(file,node) AS (SELECT file,node FROM base_all WHERE incoming=0 AND truth=1 UNION SELECT p.file,p.id FROM helper_truth h JOIN node child ON child.file=h.file AND child.id=h.node JOIN node p ON p.file=child.file AND p.id=child.parent WHERE p.kind={paren})"));
    definitions.push(format!("edges AS MATERIALIZED (SELECT p.file,p.id parent,c.id child FROM node p JOIN base_all b ON b.file=p.file AND b.node=p.id AND b.incoming=0 JOIN node c ON c.file=p.file AND c.parent=p.id WHERE p.kind={paren} AND b.truth=0 AND NOT EXISTS(SELECT 1 FROM node earlier JOIN helper_truth h ON h.file=earlier.file AND h.node=earlier.id WHERE earlier.file=c.file AND earlier.parent=p.id AND earlier.id<c.id))"));
    definitions.push("events(file,root,node,level,path) AS (SELECT file,node,node,0,'' FROM demand UNION ALL SELECT e.file,e.root,k.child,e.level+1,e.path||printf('/%010d',k.child) FROM events e JOIN edges k ON k.file=e.file AND k.parent=e.node WHERE e.level<256)".into());
    definitions.push(format!("local AS MATERIALIZED (SELECT e.*,b.truth base_truth,b.cost base_cost,b.peak base_peak FROM events e JOIN base_all b ON b.file=e.file AND b.node=e.node AND b.incoming=CASE WHEN e.level+{incoming}+1<{cutoff} THEN 0 ELSE e.level+{incoming}+1 END WHERE e.level+{incoming}<256 UNION ALL SELECT e.*,-1,0,0 FROM events e WHERE e.level+{incoming}>=256)"));
    definitions.push(format!("charged AS MATERIALIZED (SELECT *,CASE WHEN level+{incoming}>=256 THEN -1 ELSE base_truth END local_truth,CASE WHEN level+{incoming}>=256 THEN 0 ELSE 1+base_cost END cost,CASE WHEN level+{incoming}>=256 THEN 0 ELSE level+1+base_peak END peak FROM local)"));
    definitions.push("first_failure AS MATERIALIZED (SELECT file,root,MIN(path) path FROM charged WHERE local_truth<0 GROUP BY file,root)".into());
    definitions.push("trace_reached AS MATERIALIZED (SELECT c.* FROM charged c LEFT JOIN first_failure f ON f.file=c.file AND f.root=c.root WHERE f.path IS NULL OR c.path<=f.path)".into());
    let select = "SELECT r.file,r.root node,CASE WHEN MIN(r.local_truth)<0 THEN -1 WHEN h.node IS NOT NULL THEN 1 ELSE 0 END truth,SUM(r.cost) cost,MAX(r.peak) peak FROM trace_reached r LEFT JOIN helper_truth h ON h.file=r.file AND h.node=r.root GROUP BY r.file,r.root";
    let names=regex::Regex::new(r"\b(requested|helper_nodes|shape|base_inputs|base_all|demand|helper_truth|edges|events|local|charged|first_failure|trace_reached)\b").unwrap();
    let definitions = definitions
        .into_iter()
        .map(|definition| {
            names
                .replace_all(&definition, |caps: &regex::Captures<'_>| {
                    format!("pure_{name}_{}", &caps[1])
                })
                .into_owned()
        })
        .collect::<Vec<_>>();
    let select = names
        .replace_all(select, |caps: &regex::Captures<'_>| {
            format!("pure_{name}_{}", &caps[1])
        })
        .into_owned();
    let sql = format!("WITH RECURSIVE {} {select}", definitions.join(","));
    if let Ok(path) = std::env::var("RERAIL_SIM905_FULL_SPIKE_EVIDENCE") {
        std::fs::write(format!("{path}.{name}.sql"), &sql).unwrap();
    }
    Ok(Compiled {
        sql,
        terminals,
        relations: json!({"scope":"test-only pure self-tail event summary","helper":name,"metadataStrata":strata,"literalProjection":projection_proof,"definitions":definitions,"select":select,"incomingDepth":incoming,"continuationCells":0,"reachedCallEdges":true,"path":"temporary relational DFS order, not capture state","limitations":["singleton final parenthesis self-tail","acyclic pure bases","no callbacks/capture effects","demand-closure acyclic bases; incoming depth is data"]}),
    })
}
