//! Demand-keyed, pure SQL producer. Only utility and search boundaries are rows;
//! ordinary Boolean operators are scalar projections. No matcher callbacks.
use super::super::ir::Set;
use super::utility_projection_emit::{Summary, render, render_mode};
use super::utility_projection_kinds;
use super::utility_projection_lower::{Expr, Id, Program};
use super::{Axis, WalkFilter};
use std::collections::HashMap;

pub(super) struct MemoAnswers(String);
pub(super) struct LogicalEntries(String);
#[derive(Clone, Copy)]
enum CallUse {
    MemoAnswers,
    LogicalEntries,
}
pub(super) struct StoredSummary {
    pub index: usize,
    pub name: String,
    pub flags: usize,
    /// Private search reductions used only by this owner summary.
    pub reductions: Vec<usize>,
    pub cut: bool,
}
pub(super) struct Produced<C> {
    pub demand: String,
    pub roots: String,
    pub seed_definitions: Vec<String>,
    /// Explicit owner boundaries and dependencies for bounded physical storage.
    pub summaries: Vec<StoredSummary>,
    pub definitions: Vec<String>,
    /// file,node,incoming,truth,cost,peak
    pub summary_select: String,
    /// Memo answer sets and ordered logical entries are different contracts.
    calls: C,
}
impl Produced<MemoAnswers> {
    pub fn completion_select(&self) -> &str {
        &self.calls.0
    }
}
impl Produced<LogicalEntries> {
    pub fn entry_select(&self) -> &str {
        &self.calls.0
    }
}
impl<C> Produced<C> {
    fn map_calls<T>(self, f: impl FnOnce(C) -> T) -> Produced<T> {
        Produced {
            demand: self.demand,
            roots: self.roots,
            seed_definitions: self.seed_definitions,
            summaries: self.summaries,
            definitions: self.definitions,
            summary_select: self.summary_select,
            calls: f(self.calls),
        }
    }
}

struct Owner {
    body: Id,
    function: Option<usize>,
}
struct Site {
    source: usize,
    expression: Id,
    target: usize,
    ordinal: usize,
    possible: String,
}
fn atom(s: &Set, n: &str) -> Result<String, String> {
    Ok(match s {
        Set::AllNodes => "1".into(),
        Set::Empty => "0".into(),
        Set::Kinds(k) => {
            if k.is_empty() {
                "0".into()
            } else {
                format!(
                    "{n}.kind IN ({})",
                    k.iter().map(u16::to_string).collect::<Vec<_>>().join(",")
                )
            }
        }
        Set::Regex(t) => format!("node_regex({t},{n}.file,{n}.id)"),
        Set::Nonempty => format!("{n}.\"end\">{n}.start"),
        Set::Complement(s) => format!("NOT ({})", atom(s, n)?),
        Set::Union(v) | Set::Intersect(v) => {
            let all = matches!(s, Set::Intersect(_));
            if v.is_empty() {
                if all { "1".into() } else { "0".into() }
            } else {
                format!(
                    "({})",
                    v.iter()
                        .map(|s| atom(s, n))
                        .collect::<Result<Vec<_>, _>>()?
                        .join(if all { ") AND (" } else { ") OR (" })
                )
            }
        }
        _ => return Err("projection atom needs a relational boundary".into()),
    })
}
fn edge(
    axis: Axis,
    field: Option<u16>,
    filter: Option<&WalkFilter>,
    origin: &str,
    child: &str,
) -> String {
    let mut sql = match axis {
        Axis::Child => format!("{child}.parent={origin}.node"),
        Axis::Parent => format!("{child}.id=n.parent"),
        Axis::Previous => format!("{child}.id=n.prev"),
        Axis::Next => format!("{child}.id=n.next"),
    };
    if let Some(f) = field {
        sql.push_str(&format!(
            " AND {}.field={f} AND {}.ffirst=1",
            if matches!(axis, Axis::Parent) {
                "n"
            } else {
                child
            },
            if matches!(axis, Axis::Parent) {
                "n"
            } else {
                child
            }
        ));
    }
    if let Some(filter) = filter {
        if filter.kinds.is_empty() {
            sql.push_str(" AND 0");
        } else {
            sql.push_str(&format!(
                " AND {child}.kind IN ({})",
                filter
                    .kinds
                    .iter()
                    .map(u16::to_string)
                    .collect::<Vec<_>>()
                    .join(",")
            ));
        }
        for t in &filter.text {
            sql.push_str(&format!(" AND node_regex({t},{child}.file,{child}.id)"));
        }
    }
    sql
}
/// Admission remains structural and source independent. The caller owns memo
/// keys/layout, work/depth fit, publication, and the original failed-prefix path.
pub(super) fn compile(
    p: &Program,
    demand_sql: &str,
    prefix: &str,
) -> Result<Produced<MemoAnswers>, String> {
    let mut produced = compile_forest(
        p,
        &[p.entry],
        &format!("SELECT 0 entry,file,node,incoming FROM ({demand_sql})"),
        prefix,
    )?;
    produced.summary_select = format!(
        "SELECT file,node,incoming,truth,cost,peak FROM ({}) WHERE entry=0",
        produced.summary_select
    );
    produced.calls.0 = format!(
        "SELECT root_file,root_node,root_incoming,path,utility,node,incoming,truth,cost,peak FROM ({}) WHERE root_entry=0",
        produced.calls.0
    );
    Ok(produced)
}
/// Forest demand additionally supplies an entry index. All roots share utility
/// and traversal relations; no helper is compiled once per caller.
pub(super) fn compile_forest(
    p: &Program,
    entries: &[Id],
    demand_sql: &str,
    prefix: &str,
) -> Result<Produced<MemoAnswers>, String> {
    Ok(
        compile_forest_mode(p, entries, demand_sql, prefix, false, CallUse::MemoAnswers)?
            .map_calls(MemoAnswers),
    )
}
pub(super) fn compile_complete(
    p: &Program,
    demand_sql: &str,
    prefix: &str,
) -> Result<Produced<LogicalEntries>, String> {
    let mut produced = compile_forest_mode(
        p,
        &[p.entry],
        &format!("SELECT 0 entry,file,node,0 incoming FROM ({demand_sql})"),
        prefix,
        true,
        CallUse::LogicalEntries,
    )?
    .map_calls(LogicalEntries);
    produced.summary_select = format!(
        "SELECT file,node,incoming,truth,cost,peak FROM ({}) WHERE entry=0",
        produced.summary_select
    );
    produced.calls.0 = format!(
        "SELECT root_file,root_node,root_incoming,path,utility,node,incoming,truth,cost,peak FROM ({}) WHERE root_entry=0",
        produced.calls.0
    );
    Ok(produced)
}
fn compile_forest_mode(
    p: &Program,
    entries: &[Id],
    demand_sql: &str,
    prefix: &str,
    complete: bool,
    call_use: CallUse,
) -> Result<Produced<String>, String> {
    if !prefix
        .as_bytes()
        .first()
        .is_some_and(|b| b.is_ascii_alphabetic() || *b == b'_')
        || !prefix
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
    {
        return Err("projection prefix must be an SQL identifier".into());
    }
    let mut owners = entries
        .iter()
        .map(|body| Owner {
            body: *body,
            function: None,
        })
        .collect::<Vec<_>>();
    let mut function_owners = Vec::new();
    for (f, fun) in p.functions.iter().enumerate() {
        function_owners.push(owners.len());
        owners.push(Owner {
            body: fun.body,
            function: Some(f),
        });
    }
    let mut searches = HashMap::new();
    for (id, e) in p.expressions.iter().enumerate() {
        if let Expr::Search { body, .. } = e {
            searches.insert(id, owners.len());
            owners.push(Owner {
                body: *body,
                function: None,
            });
        }
    }
    let mut sites = Vec::new();
    for (o, owner) in owners.iter().enumerate() {
        let possible = utility_projection_kinds::boundary_guards(p, owner.body, "n");
        render(
            p,
            owner.body,
            |_| Ok("0".into()),
            |id, e| {
                let target = match e {
                    Expr::Call(f) => function_owners[*f],
                    Expr::Search { .. } => searches[&id],
                    _ => unreachable!(),
                };
                let ordinal = sites.iter().filter(|s: &&Site| s.source == o).count();
                sites.push(Site {
                    source: o,
                    expression: id,
                    target,
                    possible: possible[ordinal].clone(),
                    ordinal,
                });
                Ok(Summary::atom("0".into()))
            },
        )?;
    }
    let demand = format!("{prefix}_demand");
    let roots = format!("{prefix}_roots");
    let summary = |o| format!("{prefix}_summary_{o}");
    let mut definitions = vec![format!(
        "{roots}(entry,file,node,incoming) AS MATERIALIZED (SELECT DISTINCT entry,file,node,incoming FROM ({demand_sql}))"
    )];
    let mut branches = vec![format!("SELECT entry,file,node,incoming FROM {roots}")];
    for site in &sites {
        let shift = usize::from(owners[site.source].function.is_some());
        let guard = format!(
            " AND ({}){}",
            site.possible,
            if shift == 1 {
                " AND d.incoming<256"
            } else {
                ""
            }
        );
        match &p.expressions[site.expression]{
            Expr::Call(_)=>branches.push(format!("SELECT {},d.file,d.node,d.incoming+{shift} FROM {demand} d JOIN node n ON n.file=d.file AND n.id=d.node WHERE d.owner={}{}",site.target,site.source,guard)),
            Expr::Search{axis,field,filter,..}=>branches.push(format!("SELECT {},d.file,c.id,d.incoming+{shift} FROM {demand} d JOIN node n ON n.file=d.file AND n.id=d.node JOIN node c ON c.file=d.file AND {} WHERE d.owner={}{}",site.target,edge(*axis,*field,filter.as_ref(),"d","c"),site.source,guard)),
            _=>unreachable!(),
        }
    }
    definitions.push(format!(
        "{demand}(owner,file,node,incoming) AS ({})",
        branches.join(" UNION ")
    ));
    fn dependency_order(
        owner: usize,
        sites: &[Site],
        seen: &mut [u8],
        out: &mut Vec<usize>,
    ) -> Result<(), String> {
        if seen[owner] == 2 {
            return Ok(());
        }
        if seen[owner] == 1 {
            return Err("projection has a cyclic summary dependency".into());
        }
        seen[owner] = 1;
        for site in sites.iter().filter(|s| s.source == owner) {
            dependency_order(site.target, sites, seen, out)?;
        }
        seen[owner] = 2;
        out.push(owner);
        Ok(())
    }
    let mut order = vec![];
    let mut seen = vec![0; owners.len()];
    for owner in 0..owners.len() {
        dependency_order(owner, &sites, &mut seen, &mut order)?;
    }
    let seed_definitions = definitions.clone();
    let mut summaries = Vec::new();
    let mut depths = vec![0usize; owners.len()];
    for &owner in &order {
        depths[owner] = sites
            .iter()
            .filter(|s| s.source == owner)
            .map(|s| depths[s.target] + 1)
            .max()
            .unwrap_or(0);
    }
    let mut recipe_branches = Vec::new();
    for o in order {
        let owner = &owners[o];
        let shift = usize::from(owner.function.is_some());
        let current = format!("d.incoming+{shift}");
        let mut joins = Vec::new();
        let mut search_rows = HashMap::new();
        let mut reductions = Vec::new();
        for site in sites.iter().filter(|s| s.source == o) {
            let alias = format!("b{}", site.ordinal);
            match &p.expressions[site.expression]{
                Expr::Call(_)=>joins.push(format!("LEFT JOIN {} {alias} ON {alias}.file=d.file AND {alias}.node=d.node AND {alias}.incoming={current}",summary(site.target))),
                Expr::Search{axis,field,filter,..}=>{
                    let rows=format!("{prefix}_walk_{o}_{}",site.ordinal);let reduced=format!("{rows}_reduced");
                    definitions.push(format!("{rows} AS MATERIALIZED (SELECT d.file,d.node,d.incoming,c.id candidate,s.truth,s.cost,s.peak,min(CASE WHEN s.truth=1 OR s.truth<0 THEN c.id END) OVER (PARTITION BY d.file,d.node,d.incoming) stop FROM {demand} d JOIN node n ON n.file=d.file AND n.id=d.node CROSS JOIN node c ON c.file=d.file AND {} CROSS JOIN {} s ON s.file=d.file AND s.node=c.id AND s.incoming={current} WHERE d.owner={o}{})",edge(*axis,*field,filter.as_ref(),"d","c"),summary(site.target),if shift==1{" AND d.incoming<256"}else{""}));
                    definitions.push(format!("{reduced} AS MATERIALIZED (SELECT file,node,incoming,coalesce(max(CASE WHEN candidate=stop THEN truth END),CASE WHEN max(truth=2) THEN 2 ELSE 0 END) truth,sum(cost) cost,max(peak) peak FROM {rows} WHERE stop IS NULL OR candidate<=stop GROUP BY file,node,incoming)"));
                    reductions.push(definitions.len()-1);
                    joins.push(format!("LEFT JOIN {reduced} {alias} ON {alias}.file=d.file AND {alias}.node=d.node AND {alias}.incoming=d.incoming"));search_rows.insert(site.ordinal,rows);
                },_=>unreachable!(),
            }
        }
        let mut cursor = 0;
        let projection = render_mode(
            p,
            owner.body,
            |s| atom(s, "n"),
            |_, _| {
                let alias = format!("b{cursor}");
                cursor += 1;
                Ok(Summary {
                    truth: format!("coalesce({alias}.truth,0)"),
                    cost: format!("coalesce({alias}.cost,0)"),
                    peak: format!("coalesce({alias}.peak,0)"),
                })
            },
            complete,
        )?;
        let sources = format!(
            "FROM {demand} d JOIN node n ON n.file=d.file AND n.id=d.node {}",
            joins.join(" ")
        );
        let from = format!("{sources} WHERE d.owner={o}");
        let s = projection.summary;
        let (truth, cost, peak) = if shift == 1 {
            (
                format!("CASE WHEN d.incoming>=256 THEN -1 ELSE ({}) END", s.truth),
                format!("CASE WHEN d.incoming>=256 THEN 0 ELSE 1+({}) END", s.cost),
                format!(
                    "CASE WHEN d.incoming>=256 THEN 0 ELSE max(1,1+({})) END",
                    s.peak
                ),
            )
        } else {
            (s.truth, s.cost, s.peak)
        };
        let flags = projection
            .reached
            .iter()
            .enumerate()
            .map(|(i, r)| {
                format!(
                    ",CASE WHEN {}({}) THEN 1 ELSE 0 END reached{i}",
                    if shift == 1 {
                        "d.incoming<256 AND "
                    } else {
                        ""
                    },
                    r.guard
                )
            })
            .collect::<String>();
        definitions.push(format!("{} AS MATERIALIZED (SELECT d.file,d.node,d.incoming,{truth} truth,{cost} cost,{peak} peak{flags} {from})",summary(o)));
        summaries.push(StoredSummary {
            index: definitions.len() - 1,
            name: summary(o),
            flags: projection.reached.len(),
            reductions,
            cut: depths[o] > 0 && depths[o] % 3 == 0,
        });
        for site in sites.iter().filter(|s| s.source == o) {
            let source = summary(o);
            let active = format!("d.reached{}", site.ordinal);
            let branch = if let Some(rows) = search_rows.get(&site.ordinal) {
                format!(
                    "SELECT {o} owner,d.file,d.node,d.incoming,{} target,w.candidate target_node,{current} target_incoming,printf('%06d.%012d',{},w.candidate) ordinal FROM {source} d JOIN {rows} w ON w.file=d.file AND w.node=d.node AND w.incoming=d.incoming AND (w.stop IS NULL OR w.candidate<=w.stop) WHERE {active}",
                    site.target, site.ordinal
                )
            } else {
                format!(
                    "SELECT {o} owner,d.file,d.node,d.incoming,{} target,d.node target_node,{current} target_incoming,printf('%06d',{}) ordinal FROM {source} d WHERE {active}",
                    site.target, site.ordinal
                )
            };
            recipe_branches.push(branch);
        }
    }
    let edges = format!("{prefix}_edges");
    definitions.push(format!("{edges} AS MATERIALIZED ({})",if recipe_branches.is_empty(){"SELECT 0 owner,0 file,0 node,0 incoming,0 target,0 target_node,0 target_incoming,'' ordinal WHERE 0".into()}else{recipe_branches.join(" UNION ALL ")}));
    let visits = format!("{prefix}_visits");
    // Memo writes commute, so their reachability is a set. Work/depth prefixes
    // need every logical invocation in source order, including repeated calls.
    let path = match call_use {
        CallUse::MemoAnswers => {
            definitions.push(format!("{visits}(root_entry,root_file,root_node,root_incoming,owner,file,node,incoming) AS (SELECT entry,file,node,incoming,entry,file,node,incoming FROM {roots} UNION SELECT v.root_entry,v.root_file,v.root_node,v.root_incoming,e.target,e.file,e.target_node,e.target_incoming FROM {visits} v JOIN {edges} e ON e.owner=v.owner AND e.file=v.file AND e.node=v.node AND e.incoming=v.incoming)"));
            None
        }
        CallUse::LogicalEntries => {
            definitions.push(format!("{visits}(root_entry,root_file,root_node,root_incoming,owner,file,node,incoming,path) AS (SELECT entry,file,node,incoming,entry,file,node,incoming,'' FROM {roots} UNION ALL SELECT v.root_entry,v.root_file,v.root_node,v.root_incoming,e.target,e.file,e.target_node,e.target_incoming,v.path||e.ordinal||'/' FROM {visits} v JOIN {edges} e ON e.owner=v.owner AND e.file=v.file AND e.node=v.node AND e.incoming=v.incoming)"));
            Some("v.path||'z'")
        }
    };
    let completions=function_owners.iter().enumerate().map(|(f,o)| {
        let path=path.map(str::to_owned).unwrap_or_else(||format!("printf('%06d.%012d.%06d',{},v.node,v.incoming)",depths[*o]));
        format!("SELECT v.root_entry,v.root_file,v.root_node,v.root_incoming,{path} path,{} utility,v.node,v.incoming,s.truth,s.cost,s.peak FROM {visits} v JOIN {} s ON s.file=v.file AND s.node=v.node AND s.incoming=v.incoming WHERE v.owner={o} AND s.truth IN (0,1)",p.functions[f].utility,summary(*o))
    }).collect::<Vec<_>>();
    Ok(Produced {
        demand,
        roots,
        seed_definitions,
        summaries,
        definitions,
        summary_select: (0..entries.len())
            .map(|entry| {
                format!(
                    "SELECT {entry} entry,file,node,incoming,truth,cost,peak FROM {}",
                    summary(entry)
                )
            })
            .collect::<Vec<_>>()
            .join(" UNION ALL "),
        calls: if completions.is_empty() {
            "SELECT 0 root_entry,0 root_file,0 root_node,0 root_incoming,'' path,0 utility,0 node,0 incoming,0 truth,0 cost,0 peak WHERE 0".into()
        } else {
            format!(
                "SELECT * FROM ({}) ORDER BY root_entry,root_file,root_node,root_incoming,path",
                completions.join(" UNION ALL ")
            )
        },
    })
}
