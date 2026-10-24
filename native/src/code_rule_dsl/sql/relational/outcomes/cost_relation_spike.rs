//! Executable reference contract: immutable truth/work/relative-peak relations.
//! This is deliberately separate from production physical selection.
use super::{Compiled, Op, lower::Lower};
use crate::code_rule_dsl::matcher::{AstMatcher, GrammarMatcher, StopBy};
use serde_json::json;
use std::{
    collections::{HashMap, HashSet},
    rc::Rc,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Eligibility {
    pub captures: bool,
    pub callbacks: bool,
    pub cache_fills: bool,
    pub continuation_cells: usize,
}
pub(super) const CONTRACT: Eligibility = Eligibility {
    captures: false,
    callbacks: false,
    cache_fills: false,
    continuation_cells: 0,
};
#[derive(Clone)]
enum Expr {
    Atom(String),
    Call(Rc<Expr>),
    All(Vec<Rc<Expr>>),
    Any(Vec<Rc<Expr>>),
    Not(Rc<Expr>),
    Children(Rc<Expr>, Option<Vec<u16>>),
}
struct Source<'a> {
    lower: &'a mut Lower,
    active: HashSet<(usize, usize)>,
    utilities: HashMap<(usize, usize), Rc<Expr>>,
    three: bool,
}
impl Source<'_> {
    fn lower(&mut self, m: &AstMatcher) -> Result<Rc<Expr>, String> {
        let e = match m {
            AstMatcher::All(v) => Expr::All(v.iter().map(|m| self.lower(m)).collect::<Result<_,_>>()?),
            AstMatcher::Any(v) => Expr::Any(v.iter().map(|m| self.lower(m)).collect::<Result<_,_>>()?),
            AstMatcher::Not(m) => Expr::Not(self.lower(m)?),
            AstMatcher::Util(u) => {
                let key=(u.table.as_ptr() as usize,u.index);
                if !self.active.insert(key) { return Err("cost summary: recursive utility".into()); }
                let body=if let Some(body)=self.utilities.get(&key) { Rc::clone(body) } else {
                    let body=u.with_target(|m|self.lower(m)).ok_or("cost summary: missing utility body")??;
                    self.utilities.insert(key,Rc::clone(&body)); body
                };
                self.active.remove(&key);
                Expr::Call(body)
            }
            AstMatcher::Has(r, StopBy::Neighbor) => {
                let filter=if self.three { None } else { crate::code_rule_dsl::matcher::walk_candidate_filter(r.rule()).map(|(k,_)|k) };
                Expr::Children(self.lower(r.rule())?,filter)
            }
            AstMatcher::Kind(_) | AstMatcher::Regex(_) | AstMatcher::Never | AstMatcher::TestUnknown(_) => {
                let p=self.lower.plan(m)?;
                if !p.mentions.is_empty() || !p.writes.is_empty() { return Err("cost summary: capture effect".into()); }
                Expr::Atom(match p.op {
                    Op::Predicate(s) => format!("CASE WHEN {} THEN 1 ELSE 0 END",super::super::emit::condition(&s,"n")?),
                    Op::Unknown(t) => format!("CASE WHEN node_regex({t},n.file,n.id) THEN 2 ELSE 1 END"),
                    _ => return Err("cost summary: non-total atomic callback".into()),
                })
            }
            _ => return Err("cost summary: mutable capture, cache-fill, throwing callback, or unsupported structural effect".into()),
        };
        Ok(Rc::new(e))
    }
}
/// Each row is a completed pure trace or a depth-failure prefix. Cost counts
/// only reached utility entries. Peak is relative to the supplied incoming level.
struct Producer {
    nodes: &'static str,
    definitions: Vec<String>,
    summaries: HashMap<(usize, u32), String>,
}
impl Producer {
    fn relation(&mut self, body: String) -> String {
        let name = format!("cost{}", self.definitions.len());
        self.definitions
            .push(format!("{name} AS MATERIALIZED ({body})"));
        name
    }
    fn sequence(&mut self, rows: String, all: bool) -> String {
        let decisive = if all { "0" } else { "1" };
        let default = if all { 1 } else { 0 };
        self.relation(format!("WITH ordered AS (SELECT *,COALESCE(SUM(truth<0 OR truth={decisive}) OVER (PARTITION BY file,node ORDER BY ordinal ROWS BETWEEN UNBOUNDED PRECEDING AND 1 PRECEDING),0) blocked FROM ({rows})), reached AS (SELECT * FROM ordered WHERE blocked=0) SELECT file,node,CASE WHEN MIN(truth)<0 THEN MIN(truth) WHEN MAX(truth={decisive}) THEN {decisive} WHEN MAX(truth=2) THEN 2 ELSE {default} END truth,SUM(cost) cost,MAX(peak) peak FROM reached GROUP BY file,node"))
    }
    fn summary(&mut self, e: &Rc<Expr>, level: u32) -> Result<String, String> {
        let key = (Rc::as_ptr(e) as usize, level);
        if let Some(name) = self.summaries.get(&key) {
            return Ok(name.clone());
        }
        let nodes = self.nodes;
        let name = match e.as_ref() {
            Expr::Atom(t) => self.relation(format!(
                "SELECT n.file,n.id node,({t}) truth,0 cost,0 peak FROM {nodes} n"
            )),
            Expr::Call(_) if level >= 256 => self.relation(format!(
                "SELECT file,id node,-1 truth,0 cost,0 peak FROM {nodes}"
            )),
            Expr::Call(body) => {
                let b = self.summary(body, level + 1)?;
                self.relation(format!(
                    "SELECT file,node,truth,cost+1 cost,MAX(1,peak+1) peak FROM {b}"
                ))
            }
            Expr::Not(body) => {
                let b = self.summary(body, level)?;
                self.relation(format!("SELECT file,node,CASE truth WHEN 0 THEN 1 WHEN 1 THEN 0 ELSE truth END truth,cost,peak FROM {b}"))
            }
            Expr::All(v) | Expr::Any(v) => {
                if v.is_empty() {
                    self.relation(format!(
                        "SELECT file,id node,{} truth,0 cost,0 peak FROM {nodes}",
                        usize::from(matches!(e.as_ref(), Expr::All(_)))
                    ))
                } else {
                    let mut rows = vec![];
                    for (ord, b) in v.iter().enumerate() {
                        let b = self.summary(b, level)?;
                        rows.push(format!("SELECT *,{ord} ordinal FROM {b}"));
                    }
                    self.sequence(rows.join(" UNION ALL "), matches!(e.as_ref(), Expr::All(_)))
                }
            }
            Expr::Children(body, kinds) => {
                let b = self.summary(body, level)?;
                let filter = kinds.as_ref().map_or("1".into(), |k| {
                    if k.is_empty() {
                        "0".into()
                    } else {
                        format!(
                            "c.kind IN ({})",
                            k.iter().map(u16::to_string).collect::<Vec<_>>().join(",")
                        )
                    }
                });
                let rows = format!(
                    "SELECT c.file,c.parent node,b.truth,b.cost,b.peak,c.id ordinal FROM node c JOIN {b} b ON b.file=c.file AND b.node=c.id WHERE c.parent IS NOT NULL AND ({filter})"
                );
                let s = self.sequence(rows, false);
                self.relation(format!("SELECT n.file,n.id node,COALESCE(s.truth,0) truth,COALESCE(s.cost,0) cost,COALESCE(s.peak,0) peak FROM {nodes} n LEFT JOIN {s} s ON s.file=n.file AND s.node=n.id"))
            }
        };
        self.summaries.insert(key, name.clone());
        Ok(name)
    }
}
pub(super) fn compile(grammars: &[&GrammarMatcher], work: i64) -> Result<Compiled, String> {
    compile_at_depth(grammars, work, 0)
}
fn compile_at_depth(
    grammars: &[&GrammarMatcher],
    work: i64,
    depth: u32,
) -> Result<Compiled, String> {
    if work < 0 {
        return Err("cost summary: negative allowance".into());
    }
    if grammars.len() != 1 {
        return Err("cost summary: one grammar reference spike".into());
    }
    let g = grammars[0];
    if g.file.is_some() || g.transform.is_some() {
        return Err("cost summary: file/transform effect".into());
    }
    let mut lower = Lower::new(g.language, g.three_valued);
    let expr = Source {
        lower: &mut lower,
        active: HashSet::new(),
        utilities: HashMap::new(),
        three: g.three_valued,
    }
    .lower(&g.matcher)?;
    let mut p = Producer {
        nodes: "node",
        definitions: vec![],
        summaries: HashMap::new(),
    };
    let summary = p.summary(&expr, depth)?;
    let roots=p.relation(format!("SELECT s.*,SUM(s.cost) OVER(PARTITION BY s.file ORDER BY s.node ROWS UNBOUNDED PRECEDING) spent FROM {summary} s JOIN node n ON n.file=s.file AND n.id=s.node WHERE {} AND {}",usize::from(g.applies),super::root_filter(g,None)));
    let failure = p.relation(format!(
        "SELECT file,MIN(node) first FROM {roots} WHERE truth<0 OR spent>{work} GROUP BY file"
    ));
    let rows=p.relation(format!("SELECT 0 clause,r.file,r.node,'[]' captures,0 failure,{work}-r.spent work FROM {roots} r WHERE r.truth=1 AND NOT EXISTS(SELECT 1 FROM {failure} f WHERE f.file=r.file) UNION ALL SELECT 0,r.file,NULL,'[]',CASE WHEN r.spent>{work} THEN -2 ELSE r.truth END,CASE WHEN r.spent>{work} THEN 0 ELSE {work}-r.spent END FROM {roots} r JOIN {failure} f ON f.file=r.file AND f.first=r.node"));
    let sql = format!(
        "WITH {} SELECT * FROM {rows} ORDER BY clause,file,node",
        p.definitions.join(",")
    );
    let terminal_count = lower.pure.program.terminals.len();
    Ok(Compiled {
        sql,
        terminals: lower.pure.program.terminals,
        relations: json!({"scope":"test-only pure truth/cost/peak summaries","terminalOwners":vec![0;terminal_count],"effectContract":{"captures":CONTRACT.captures,"callbacks":CONTRACT.callbacks,"cacheFills":CONTRACT.cache_fills,"continuationCells":CONTRACT.continuation_cells},"summaryRelations":p.definitions.len(),"incomingDepth":depth}),
    })
}
#[path = "cost_relation_spike_tests.rs"]
mod tests;

/// Reusable producer hook: `demand_sql` yields distinct `(file,node)` keys.
/// Computation currently covers the whole document, then projects the requested
/// rows. Moving that demand into the producer requires proving closure through
/// each structural edge; this API does not claim that optimization.
pub(super) fn compile_summary(
    grammar: &GrammarMatcher,
    incoming_depth: u32,
    demand_sql: &str,
) -> Result<Compiled, String> {
    compile_summary_mode(grammar, incoming_depth, demand_sql, false)
}
/// The supported source language can read only the current node or descendants.
/// Restrict summary construction to the closure of the requested node keys.
pub(super) fn compile_summary_demanded(
    grammar: &GrammarMatcher,
    incoming_depth: u32,
    demand_sql: &str,
) -> Result<Compiled, String> {
    compile_summary_mode(grammar, incoming_depth, demand_sql, true)
}
fn compile_summary_mode(
    grammar: &GrammarMatcher,
    incoming_depth: u32,
    demand_sql: &str,
    restricted: bool,
) -> Result<Compiled, String> {
    if grammar.file.is_some() || grammar.transform.is_some() {
        return Err("cost summary: file/transform effect".into());
    }
    let mut lower = Lower::new(grammar.language, grammar.three_valued);
    let expr = Source {
        lower: &mut lower,
        active: HashSet::new(),
        utilities: HashMap::new(),
        three: grammar.three_valued,
    }
    .lower(&grammar.matcher)?;
    let definitions = if restricted {
        vec![
            format!("summary_demands AS MATERIALIZED (SELECT DISTINCT file,node FROM ({demand_sql}))"),
            "summary_closure(file,node) AS (SELECT file,node FROM summary_demands UNION SELECT n.file,n.id FROM summary_closure d JOIN node n ON n.file=d.file AND n.parent=d.node)".into(),
            "summary_nodes AS MATERIALIZED (SELECT n.* FROM summary_closure d JOIN node n ON n.file=d.file AND n.id=d.node)".into(),
        ]
    } else {
        vec![]
    };
    let mut producer = Producer {
        nodes: if restricted { "summary_nodes" } else { "node" },
        definitions,
        summaries: HashMap::new(),
    };
    let summary = producer.summary(&expr, incoming_depth)?;
    let demand = if restricted {
        "summary_demands".into()
    } else {
        producer.relation(demand_sql.into())
    };
    Ok(Compiled {
        sql: format!(
            "WITH RECURSIVE {} SELECT s.file,s.node,s.truth,s.cost,s.peak FROM {summary} s JOIN {demand} d ON d.file=s.file AND d.node=s.node",
            producer.definitions.join(",")
        ),
        terminals: lower.pure.program.terminals,
        relations: json!({"scope":"test-only projected pure summary producer","incomingDepth":incoming_depth,"columns":["file","node","truth","cost","relativePeak"],"allNodeConstruction":!restricted,"demandClosure":"current node and descendants only","continuationCells":0}),
    })
}

#[path = "cost_relation_dynamic_spike.rs"]
mod dynamic;
pub(super) use dynamic::compile_summary_dynamic;
