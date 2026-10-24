//! Incoming logical depth is a demand column, not a cloned SQL program.
use super::{Expr, Source};
use crate::code_rule_dsl::{matcher::GrammarMatcher, sql::terminals::Terminal};
use std::{
    collections::{HashMap, HashSet},
    rc::Rc,
};
pub(in crate::code_rule_dsl::sql::relational::outcomes) struct DynamicSummary {
    pub definitions: Vec<String>,
    pub select: String,
    pub terminals: Vec<Terminal>,
}
struct Producer {
    prefix: String,
    nodes: String,
    definitions: Vec<String>,
    summaries: HashMap<(usize, u32), String>,
}
impl Producer {
    fn relation(&mut self, body: String) -> String {
        let name = format!("{}_{}", self.prefix, self.definitions.len());
        self.definitions
            .push(format!("{name} AS MATERIALIZED ({body})"));
        name
    }
    fn sequence(&mut self, rows: String, all: bool) -> String {
        let decisive = if all { 0 } else { 1 };
        let default = if all { 1 } else { 0 };
        self.relation(format!("WITH ordered AS (SELECT *,COALESCE(SUM(truth<0 OR truth={decisive}) OVER(PARTITION BY file,node,incoming ORDER BY ordinal ROWS BETWEEN UNBOUNDED PRECEDING AND 1 PRECEDING),0) blocked FROM ({rows})),reached AS(SELECT * FROM ordered WHERE blocked=0) SELECT file,node,incoming,CASE WHEN MIN(truth)<0 THEN MIN(truth) WHEN MAX(truth={decisive}) THEN {decisive} WHEN MAX(truth=2) THEN 2 ELSE {default} END truth,SUM(cost) cost,MAX(peak) peak FROM reached GROUP BY file,node,incoming"))
    }
    fn summary(&mut self, e: &Rc<Expr>, offset: u32) -> Result<String, String> {
        let key = (Rc::as_ptr(e) as usize, offset);
        if let Some(name) = self.summaries.get(&key) {
            return Ok(name.clone());
        }
        let nodes = self.nodes.clone();
        let name = match e.as_ref() {
            Expr::Atom(t) => self.relation(format!(
                "SELECT n.file,n.id node,n.incoming,({t}) truth,0 cost,0 peak FROM {nodes} n"
            )),
            Expr::Call(_) if offset >= 256 => self.relation(format!(
                "SELECT file,id node,incoming,-1 truth,0 cost,0 peak FROM {nodes}"
            )),
            Expr::Call(body) => {
                let b = self.summary(body, offset + 1)?;
                self.relation(format!("SELECT file,node,incoming,CASE WHEN incoming+{offset}>=256 THEN -1 ELSE truth END truth,CASE WHEN incoming+{offset}>=256 THEN 0 ELSE cost+1 END cost,CASE WHEN incoming+{offset}>=256 THEN 0 ELSE MAX(1,peak+1) END peak FROM {b}"))
            }
            Expr::Not(body) => {
                let b = self.summary(body, offset)?;
                self.relation(format!("SELECT file,node,incoming,CASE truth WHEN 0 THEN 1 WHEN 1 THEN 0 ELSE truth END truth,cost,peak FROM {b}"))
            }
            Expr::All(v) | Expr::Any(v) => {
                if v.is_empty() {
                    self.relation(format!(
                        "SELECT file,id node,incoming,{} truth,0 cost,0 peak FROM {nodes}",
                        usize::from(matches!(e.as_ref(), Expr::All(_)))
                    ))
                } else {
                    let mut rows = vec![];
                    for (ordinal, body) in v.iter().enumerate() {
                        let b = self.summary(body, offset)?;
                        rows.push(format!("SELECT *,{ordinal} ordinal FROM {b}"));
                    }
                    self.sequence(rows.join(" UNION ALL "), matches!(e.as_ref(), Expr::All(_)))
                }
            }
            Expr::Children(body, kinds) => {
                let b = self.summary(body, offset)?;
                let filter = kinds.as_ref().map_or("1".into(), |ks| {
                    if ks.is_empty() {
                        "0".into()
                    } else {
                        format!(
                            "c.kind IN({})",
                            ks.iter().map(u16::to_string).collect::<Vec<_>>().join(",")
                        )
                    }
                });
                let s=self.sequence(format!("SELECT c.file,c.parent node,b.incoming,b.truth,b.cost,b.peak,c.id ordinal FROM node c JOIN {b} b ON b.file=c.file AND b.node=c.id WHERE c.parent IS NOT NULL AND ({filter})"),false);
                self.relation(format!("SELECT n.file,n.id node,n.incoming,COALESCE(s.truth,0) truth,COALESCE(s.cost,0) cost,COALESCE(s.peak,0) peak FROM {nodes} n LEFT JOIN {s} s ON s.file=n.file AND s.node=n.id AND s.incoming=n.incoming"))
            }
        };
        self.summaries.insert(key, name.clone());
        Ok(name)
    }
}
pub(in crate::code_rule_dsl::sql::relational::outcomes) fn compile_summary_dynamic(
    g: &GrammarMatcher,
    demand_sql: &str,
    prefix: &str,
) -> Result<DynamicSummary, String> {
    if g.file.is_some() || g.transform.is_some() {
        return Err("dynamic cost: file/transform effect".into());
    }
    if prefix.is_empty()
        || !prefix
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return Err("dynamic cost: invalid internal CTE prefix".into());
    }
    let mut lower = super::super::lower::Lower::new(g.language, g.three_valued);
    let expr = Source {
        lower: &mut lower,
        active: HashSet::new(),
        utilities: HashMap::new(),
        three: g.three_valued,
    }
    .lower(&g.matcher)?;
    let demand = format!("{prefix}_demands");
    let closure = format!("{prefix}_closure");
    let nodes = format!("{prefix}_nodes");
    let mut p = Producer {
        prefix: prefix.into(),
        nodes: nodes.clone(),
        summaries: HashMap::new(),
        definitions: vec![
            format!(
                "{demand} AS MATERIALIZED(SELECT DISTINCT file,node,incoming FROM ({demand_sql}) WHERE incoming BETWEEN 0 AND 256)"
            ),
            format!(
                "{closure}(file,node,incoming) AS(SELECT file,node,incoming FROM {demand} UNION SELECT n.file,n.id,d.incoming FROM {closure} d JOIN node n ON n.file=d.file AND n.parent=d.node)"
            ),
            format!(
                "{nodes} AS MATERIALIZED(SELECT n.*,d.incoming FROM {closure} d JOIN node n ON n.file=d.file AND n.id=d.node)"
            ),
        ],
    };
    let s = p.summary(&expr, 0)?;
    Ok(DynamicSummary {
        definitions: p.definitions,
        select: format!(
            "SELECT s.file,s.node,s.incoming,s.truth,s.cost,s.peak FROM {s} s JOIN {demand} d ON d.file=s.file AND d.node=s.node AND d.incoming=s.incoming"
        ),
        terminals: lower.pure.program.terminals,
    })
}
