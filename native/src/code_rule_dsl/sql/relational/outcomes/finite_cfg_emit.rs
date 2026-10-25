//! Physical adapter: static Boolean routes, original atomic function/search transactions.
use super::{
    finite_cfg::{Cfg, Target},
    finite_lower::{Function, Instruction, Node, Program},
};

pub(super) fn adapt(p: &Program, cfg: &Cfg) -> Option<Program> {
    let target = |t: Target| match t {
        Target::Site(i) => Some(i),
        _ => None,
    };
    let identity = |i: &usize| cfg.entries.get(i).copied().and_then(target);
    let code = cfg
        .sites
        .iter()
        .map(|site| {
            let n = &p.code[site.source];
            let op = match &n.op {
                Instruction::Eval(i) => Instruction::Eval(*i),
                Instruction::Call(i) => Instruction::Call(*i),
                Instruction::Boolean { all, children } => Instruction::Boolean {
                    all: *all,
                    children: children.iter().map(identity).collect::<Option<_>>()?,
                },
                Instruction::Not(_) => return None,
                Instruction::Search {
                    axis,
                    end,
                    field,
                    body,
                    stop,
                    bounds,
                    filter,
                } => Instruction::Search {
                    axis: *axis,
                    end: *end,
                    field: *field,
                    body: identity(body)?,
                    stop: stop.as_ref().map(identity).transpose_option()?,
                    bounds: *bounds,
                    filter: filter.clone(),
                },
                Instruction::Binding {
                    core,
                    children,
                    occurrence,
                } => Instruction::Binding {
                    core: core.clone(),
                    children: children
                        .iter()
                        .map(|(s, c)| Some((*s, identity(c)?)))
                        .collect::<Option<_>>()?,
                    occurrence: occurrence.as_ref().map(identity).transpose_option()?,
                },
            };
            Some(Node {
                op,
                effects: n.effects,
                mentions: n.mentions.clone(),
                writes: n.writes.clone(),
                node_writes: n.node_writes.clone(),
            })
        })
        .collect::<Option<_>>()?;
    let functions = p
        .functions
        .iter()
        .zip(&cfg.functions)
        .map(|(f, t)| {
            Some(Function {
                entry: target(*t)?,
                memo: f.memo,
                original: f.original.clone(),
                effects: f.effects,
                mentions: f.mentions.clone(),
                writes: f.writes.clone(),
            })
        })
        .collect::<Option<_>>()?;
    Some(Program {
        source: p.source.clone(),
        share_search: p.share_search,
        code,
        functions,
        regions: p.regions.clone(),
        entry: target(cfg.entry)?,
        mentions: p.mentions.clone(),
        writes: p.writes.clone(),
        node_writes: p.node_writes.clone(),
    })
}
trait TransposeOption<T> {
    fn transpose_option(self) -> Option<Option<T>>;
}
impl<T> TransposeOption<T> for Option<Option<T>> {
    fn transpose_option(self) -> Option<Option<T>> {
        match self {
            None => Some(None),
            Some(v) => v.map(Some),
        }
    }
}

// A single integer key prevents SQLite from indexing only the truth branch and
// scanning all source sites in that bucket for every correlated transition.
pub(super) fn metadata(cfg: &Cfg) -> String {
    let rows = cfg
        .sites
        .iter()
        .enumerate()
        .flat_map(|(site, s)| {
            s.routes.0.iter().enumerate().map(move |(branch, target)| {
                let (pc, truth) = match target {
                    Target::Site(i) => ((i + 10).to_string(), "NULL".into()),
                    Target::Return(v) => (
                        "0".into(),
                        if *v < 0 { "NULL".into() } else { v.to_string() },
                    ),
                };
                format!("({},{pc},{truth})", (site + 10) * 4 + branch)
            })
        })
        .collect::<Vec<_>>();
    format!(
        "SELECT column1 route_key,column2 target,column3 mapped FROM (VALUES {})",
        rows.join(",")
    )
}

/// Route only a completed transaction. Intermediate search/Boolean returns keep their frame.
/// An empty scalar RHS becomes an all-NULL relation_row; suppress that synthetic row
/// so an inapplicable handler cannot shadow a later UNION branch. Node0 remains valid.
pub(super) fn route(query: &str, table: &str, site: &str, parent: &str) -> String {
    format!(
        "SELECT r.c0,CASE WHEN r.c1=0 AND r.c4 IS {parent} THEN CASE WHEN route.route_key IS NULL THEN json('missing static CFG route') ELSE route.target END ELSE r.c1 END,r.c2,r.c3,r.c4,CASE WHEN r.c1=0 AND r.c4 IS {parent} THEN COALESCE(route.mapped,r.c5) ELSE r.c5 END,r.c6,r.c7 FROM (SELECT 1) cfg_seed CROSS JOIN relation_row r ON (r.c0,r.c1,r.c2,r.c3,r.c4,r.c5,r.c6,r.c7) IS ({query}) LEFT JOIN {table} route ON route.route_key=(({site})*4+CASE WHEN r.c5<0 THEN 3 ELSE r.c5 END) WHERE r.c0 IS NOT NULL"
    )
}
