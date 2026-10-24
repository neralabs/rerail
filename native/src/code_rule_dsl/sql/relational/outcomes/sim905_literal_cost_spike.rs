//! Source-proved literal helper projections. Materialize utility/traversal
//! fanout, not a row relation for every finite Boolean expression.
use crate::code_rule_dsl::{
    matcher::{AstMatcher, GrammarMatcher, StopBy, UtilRef},
    sql::terminals::Terminal,
};
use serde_json::{Value, json};
use std::sync::Arc;
pub(super) struct Projection {
    pub definitions: Vec<String>,
    pub select: String,
    pub terminals: Vec<Terminal>,
    pub proof: Value,
}
fn body(g: &GrammarMatcher, name: &str) -> Result<AstMatcher, String> {
    let index = g
        .utils
        .names
        .iter()
        .position(|n| n == name)
        .ok_or("literal utility missing")?;
    UtilRef {
        table: Arc::downgrade(&g.utils),
        index,
    }
    .with_target(Clone::clone)
    .ok_or_else(|| "literal source missing".into())
}
fn util(m: &AstMatcher, g: &GrammarMatcher, name: &str) -> bool {
    matches!(m,AstMatcher::Util(u) if u.table.as_ptr()==Arc::as_ptr(&g.utils)&&g.utils.names[u.index]==name)
}
fn kind(m: &AstMatcher, g: &GrammarMatcher, name: &str) -> bool {
    let Ok(p) = super::super::lower::Lower::new(g.language, true).plan(m) else {
        return false;
    };
    matches!(p.op,super::super::Op::Predicate(super::super::Set::Kinds(v)) if v==vec![g.language.kinds().kind_id(name)])
}
fn probe(m: &AstMatcher, g: &GrammarMatcher, part: &str) -> bool {
    let AstMatcher::Any(v) = m else {
        return false;
    };
    if v.len() != 2 {
        return false;
    }
    let (AstMatcher::All(a), AstMatcher::All(b)) = (&v[0], &v[1]) else {
        return false;
    };
    if a.len() != 2
        || b.len() != 2
        || !kind(&a[0], g, "string")
        || !util(&a[1], g, part)
        || !kind(&b[0], g, "concatenated_string")
    {
        return false;
    }
    let AstMatcher::Has(r, StopBy::Neighbor) = &b[1] else {
        return false;
    };
    let AstMatcher::All(c) = r.rule() else {
        return false;
    };
    c.len() == 2 && kind(&c[0], g, "string") && util(&c[1], g, part)
}
fn base_arm(m: &AstMatcher, g: &GrammarMatcher) -> bool {
    let AstMatcher::All(v) = m else {
        return false;
    };
    v.len() == 2
        && matches!(&v[0],AstMatcher::Not(n) if util(n,g,"bytes-string"))
        && util(&v[1], g, "plain-string")
}
fn unwrap_one(m: &AstMatcher) -> &AstMatcher {
    if let AstMatcher::Any(v) = m
        && v.len() == 1
    {
        unwrap_one(&v[0])
    } else {
        m
    }
}
fn validate(g: &GrammarMatcher, base: &AstMatcher) -> Result<Vec<Terminal>, String> {
    let mut terminals = vec![];
    for name in ["part-t", "part-f", "part-b"] {
        let m = body(g, name)?;
        let AstMatcher::Has(r, StopBy::Neighbor) = m else {
            return Err("part helper shape changed".into());
        };
        let AstMatcher::All(v) = r.rule() else {
            return Err("part guard shape changed".into());
        };
        if v.len() != 2 || !kind(&v[0], g, "string_start") {
            return Err("part kind guard changed".into());
        }
        let AstMatcher::Regex(regex) = &v[1] else {
            return Err("part regex source changed".into());
        };
        terminals.push(Terminal::Regex(regex.clone()));
    }
    if !probe(&body(g, "t-string")?, g, "part-t") {
        return Err("t-string call order changed".into());
    }
    let f = body(g, "f-string")?;
    let AstMatcher::All(f) = f else {
        return Err("f-string shape changed".into());
    };
    if f.len() != 2
        || !probe(&f[0], g, "part-f")
        || !matches!(&f[1],AstMatcher::Not(n) if util(n,g,"t-string"))
    {
        return Err("f-string normalized probe-before-not order changed".into());
    }
    let p = body(g, "plain-string")?;
    let AstMatcher::All(p) = p else {
        return Err("plain-string shape changed".into());
    };
    let plain_ok = if p.len() == 2 {
        matches!(&p[0],AstMatcher::Any(v) if v.len()==2&&kind(&v[0],g,"string")&&kind(&v[1],g,"concatenated_string"))
            && matches!(&p[1],AstMatcher::Not(n) if matches!(n.as_ref(),AstMatcher::Any(v) if v.len()==2&&util(&v[0],g,"t-string")&&util(&v[1],g,"f-string")))
    } else {
        false
    };
    if !plain_ok {
        return Err("plain-string normalized t-before-f order changed".into());
    }
    let b = body(g, "bytes-string")?;
    let AstMatcher::All(b) = b else {
        return Err("bytes-string shape changed".into());
    };
    if b.len() != 2 || !probe(&b[0], g, "part-b") || !util(&b[1], g, "plain-string") {
        return Err("bytes-string normalized probe-before-plain order changed".into());
    }
    let base = unwrap_one(base);
    let base_ok = base_arm(base, g)
        || matches!(base,AstMatcher::Any(v) if v.len()==2&&kind(&v[0],g,"none")&&base_arm(&v[1],g));
    if !base_ok {
        return Err("literal base normalized not-bytes-before-plain changed".into());
    }
    Ok(terminals)
}
/// Demand columns file,node,incoming. Callers have already excluded all
/// metadata-constant strata; this proof accepts only string/concat demands.
pub(super) fn compile(
    g: &GrammarMatcher,
    base: &AstMatcher,
    demand: &str,
    prefix: &str,
) -> Result<Projection, String> {
    let terminals = validate(g, base)?;
    let st = g.language.kinds().kind_id("string");
    let ct = g.language.kinds().kind_id("concatenated_string");
    let start = g.language.kinds().kind_id("string_start");
    let n = |suffix: &str| format!("{prefix}_{suffix}");
    let input = n("input");
    let roots = n("roots");
    let parts = n("parts");
    let flags = n("flags");
    let numbered = n("numbered");
    let stats = n("stats");
    let data = n("data");
    let t = n("t");
    let f = n("f");
    let p = n("plain");
    let b = n("bytes");
    let answer = n("answer");
    let mut definitions = vec![
        format!("{input} AS MATERIALIZED(SELECT DISTINCT file,node,incoming FROM ({demand}))"),
        format!("{roots} AS MATERIALIZED(SELECT DISTINCT file,node FROM {input})"),
        format!(
            "{parts} AS MATERIALIZED(SELECT d.file,d.node,n.id part FROM {roots} d JOIN node n ON n.file=d.file AND n.id=d.node WHERE n.kind={st} UNION ALL SELECT d.file,d.node,c.id FROM {roots} d JOIN node n ON n.file=d.file AND n.id=d.node AND n.kind={ct} JOIN node c ON c.file=n.file AND c.parent=n.id AND c.kind={st})"
        ),
        format!(
            "{flags} AS MATERIALIZED(SELECT x.*,EXISTS(SELECT 1 FROM node s WHERE s.file=x.file AND s.parent=x.part AND s.kind={start} AND node_regex(0,s.file,s.id)) tf,EXISTS(SELECT 1 FROM node s WHERE s.file=x.file AND s.parent=x.part AND s.kind={start} AND node_regex(1,s.file,s.id)) ff,EXISTS(SELECT 1 FROM node s WHERE s.file=x.file AND s.parent=x.part AND s.kind={start} AND node_regex(2,s.file,s.id)) bf FROM {parts} x)"
        ),
        format!(
            "{numbered} AS MATERIALIZED(SELECT *,ROW_NUMBER() OVER(PARTITION BY file,node ORDER BY part) ordinal FROM {flags})"
        ),
        format!(
            "{stats} AS MATERIALIZED(SELECT r.file,r.node,COUNT(x.part) count,COALESCE(MAX(x.tf),0) tf,COALESCE(MAX(x.ff),0) ff,COALESCE(MAX(x.bf),0) bf,COALESCE(MIN(CASE WHEN x.tf THEN x.ordinal END),COUNT(x.part)) rt,COALESCE(MIN(CASE WHEN x.ff THEN x.ordinal END),COUNT(x.part)) rf,COALESCE(MIN(CASE WHEN x.bf THEN x.ordinal END),COUNT(x.part)) rb FROM {roots} r LEFT JOIN {numbered} x ON x.file=r.file AND x.node=r.node GROUP BY r.file,r.node)"
        ),
        format!(
            "{data} AS MATERIALIZED(SELECT i.*,s.count,s.tf,s.ff,s.bf,s.rt,s.rf,s.rb FROM {input} i JOIN {stats} s ON s.file=i.file AND s.node=i.node)"
        ),
    ];
    definitions.push(format!("{t} AS MATERIALIZED(SELECT x.file,x.node,x.incoming,o.off,CASE WHEN x.incoming+o.off>=256 OR (x.count>0 AND x.incoming+o.off+1>=256) THEN -1 ELSE x.tf END truth,CASE WHEN x.incoming+o.off>=256 THEN 0 WHEN x.count>0 AND x.incoming+o.off+1>=256 THEN 1 ELSE 1+x.rt END cost,CASE WHEN x.incoming+o.off>=256 THEN 0 WHEN x.count>0 AND x.incoming+o.off+1>=256 THEN 1 ELSE 1+(x.count>0) END peak FROM {data} x CROSS JOIN(SELECT column1 off FROM(VALUES(1),(2),(3))) o)"));
    definitions.push(format!("{f} AS MATERIALIZED(SELECT x.file,x.node,x.incoming,o.off,CASE WHEN x.incoming+o.off>=256 OR (x.count>0 AND x.incoming+o.off+1>=256) THEN -1 WHEN NOT x.ff THEN 0 WHEN t.truth<0 THEN t.truth ELSE 1-t.truth END truth,CASE WHEN x.incoming+o.off>=256 THEN 0 WHEN x.count>0 AND x.incoming+o.off+1>=256 THEN 1 ELSE 1+x.rf+CASE WHEN x.ff THEN t.cost ELSE 0 END END cost,CASE WHEN x.incoming+o.off>=256 THEN 0 WHEN x.count>0 AND x.incoming+o.off+1>=256 THEN 1 ELSE MAX(1+(x.count>0),CASE WHEN x.ff THEN 1+t.peak ELSE 0 END) END peak FROM {data} x CROSS JOIN(SELECT column1 off FROM(VALUES(1),(2))) o JOIN {t} t ON t.file=x.file AND t.node=x.node AND t.incoming=x.incoming AND t.off=o.off+1)"));
    definitions.push(format!("{p} AS MATERIALIZED(SELECT x.file,x.node,x.incoming,o.off,CASE WHEN x.incoming+o.off>=256 THEN -1 WHEN t.truth<0 THEN t.truth WHEN t.truth=1 THEN 0 WHEN f.truth<0 THEN f.truth ELSE 1-f.truth END truth,CASE WHEN x.incoming+o.off>=256 THEN 0 ELSE 1+t.cost+CASE WHEN t.truth=0 THEN f.cost ELSE 0 END END cost,CASE WHEN x.incoming+o.off>=256 THEN 0 ELSE 1+MAX(t.peak,CASE WHEN t.truth=0 THEN f.peak ELSE 0 END) END peak FROM {data} x CROSS JOIN(SELECT column1 off FROM(VALUES(0),(1))) o JOIN {t} t ON t.file=x.file AND t.node=x.node AND t.incoming=x.incoming AND t.off=o.off+1 JOIN {f} f ON f.file=x.file AND f.node=x.node AND f.incoming=x.incoming AND f.off=o.off+1)"));
    definitions.push(format!("{b} AS MATERIALIZED(SELECT x.file,x.node,x.incoming,CASE WHEN x.incoming>=256 OR(x.count>0 AND x.incoming+1>=256) THEN -1 WHEN NOT x.bf THEN 0 ELSE p.truth END truth,CASE WHEN x.incoming>=256 THEN 0 WHEN x.count>0 AND x.incoming+1>=256 THEN 1 ELSE 1+x.rb+CASE WHEN x.bf THEN p.cost ELSE 0 END END cost,CASE WHEN x.incoming>=256 THEN 0 WHEN x.count>0 AND x.incoming+1>=256 THEN 1 ELSE MAX(1+(x.count>0),CASE WHEN x.bf THEN 1+p.peak ELSE 0 END) END peak FROM {data} x JOIN {p} p ON p.file=x.file AND p.node=x.node AND p.incoming=x.incoming AND p.off=1)"));
    definitions.push(format!("{answer} AS MATERIALIZED(SELECT b.file,b.node,b.incoming,CASE WHEN b.truth<0 THEN b.truth WHEN b.truth=1 THEN 0 ELSE p.truth END truth,b.cost+CASE WHEN b.truth=0 THEN p.cost ELSE 0 END cost,MAX(b.peak,CASE WHEN b.truth=0 THEN p.peak ELSE 0 END) peak FROM {b} b JOIN {p} p ON p.file=b.file AND p.node=b.node AND p.incoming=b.incoming AND p.off=0)"));
    Ok(Projection {
        definitions,
        select: format!("SELECT file,node,incoming,truth,cost,peak FROM {answer}"),
        terminals,
        proof: json!({"sourceProof":"original normalized Kind/Regex/Has/Any/Not/utility shapes checked","materializedBoundaries":["ordered string parts","t-string offsets","f-string offsets","plain-string offsets","bytes-string"],"booleanExpressionRelations":0,"incomingDepth":"data","atomicRegexes":"cloned original source part-t/part-f/part-b"}),
    })
}
