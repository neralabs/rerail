//! Spike A3: fixed substitutions for the stages that rebuild the syntax tree in SQL, and for the
//! node-dumping AST inputs that feed only them.
//!
//! Two kinds of substitution, both chosen at compile time by exact shape, never by a planner:
//!
//! - **Dump groups** (`DumpGroup`): a clause's `syntax_N` inputs that `syntax_rows_k` unions into
//!   `nodes`. Each input's rule is `all[selection..., pattern $NODE, parent capture, field
//!   captures...]` with a capture-free selection whose tail is shared by every member (the gate).
//!   Their rows are produced in one pass over the facts: each member's own selection, then the
//!   shared gate once per node (one evaluator, one utility memo), then the captures read directly
//!   from the facts (parent, first child per field). Rows, order and anchors equal `Input::rows`.
//! - **Stage kernels** (`Kernel`): the SQL texts of `syntax_rows_k`, `nodes`, `node_ordinals`,
//!   `edges`, `ownership` and `descendants` that the policies generate, matched byte for byte after
//!   whitespace normalisation, run as native loops over the same tables with the engine's value,
//!   key, affinity and row-order rules.
//!
//! `FACTS_A3_CHECK=1` runs the original path next to every substitution and panics on any
//! difference in rows, order, values or anchors.
use crate::ast::{Compiler, Name, Names, Query, SyntaxMode};
use crate::eval::{Input, Proj};
use crate::rel::{RelPlan, StagePlan};
use crate::sql::engine::{Db, Schema, Table};
use crate::sql::value::{self as sv, Aff, DKey, V};
use rustc_hash::FxHashMap;
use serde_json::Value as Json;
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::rc::Rc;

pub fn check_mode() -> bool {
    static CHECK: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| std::env::var_os("FACTS_A3_CHECK").is_some());
    *CHECK
}

fn norm(sql: &str) -> String {
    sql.split_whitespace().collect::<Vec<_>>().join(" ")
}

const NODE_COLS: &str = "kind||':'||__start||':'||__end AS id, parent_kind||':'||parent_start||':'||parent_end AS parent, ancestor_kind||':'||ancestor_start||':'||ancestor_end AS edge_parent, row_number() OVER(PARTITION BY __path,parent_start,parent_end,parent_kind ORDER BY __start,__end,kind)-1 AS ordinal";
const UNION01: &str = "(SELECT * FROM syntax_rows_0 UNION ALL SELECT * FROM syntax_rows_1)";
const NODE_ORDINALS: &str = "SELECT __path,__start,__end,kind,row_number() OVER(PARTITION BY __path,parent_start,parent_end,parent_kind ORDER BY __start,__end,kind)-1 AS ordinal FROM (SELECT * FROM syntax_rows_0 UNION ALL SELECT * FROM syntax_rows_1)";
const NODES_JOIN: &str = "SELECT n.*,n.kind||':'||n.__start||':'||n.__end AS id, n.parent_kind||':'||n.parent_start||':'||n.parent_end AS parent, n.ancestor_kind||':'||n.ancestor_start||':'||n.ancestor_end AS edge_parent,o.ordinal FROM (SELECT * FROM syntax_rows_0 UNION ALL SELECT * FROM syntax_rows_1) n JOIN node_ordinals o ON o.__path=n.__path AND o.__start=n.__start AND o.__end=n.__end AND o.kind=n.kind";
const EDGES: &str = "SELECT __path,edge_parent AS src,id AS dst,ordinal FROM nodes WHERE edge_parent IS NOT NULL";
const OWNERSHIP: &str = "WITH RECURSIVE owned(__path,id,fn,cls,top_fn) AS ( SELECT n.__path,n.id,CASE WHEN n.kind='function_definition' THEN n.id END,CASE WHEN n.kind='class_definition' THEN n.id END,CASE WHEN n.kind='function_definition' THEN n.id END FROM nodes n LEFT JOIN nodes parent ON parent.__path=n.__path AND parent.id=n.edge_parent WHERE parent.id IS NULL UNION ALL SELECT child.__path,child.id,CASE WHEN child.kind='function_definition' THEN child.id ELSE owned.fn END,CASE WHEN child.kind='class_definition' THEN child.id ELSE owned.cls END,CASE WHEN owned.top_fn IS NULL AND child.kind='function_definition' THEN child.id ELSE owned.top_fn END FROM owned JOIN edges e ON e.__path=owned.__path AND e.src=owned.id JOIN nodes child ON child.__path=e.__path AND child.id=e.dst) SELECT * FROM owned";
const DESCENDANTS: &str = "WITH RECURSIVE walk(__path,root,node) AS (SELECT __path,id,id FROM nodes UNION SELECT w.__path,w.root,e.dst FROM walk w JOIN edges e ON e.__path=w.__path AND e.src=w.node) SELECT * FROM walk";
const DESCENDANTS_EXPAND_HEAD: &str = "WITH RECURSIVE walk(__path,root,node,expand) AS (SELECT __path,id,id,kind NOT IN (";
const DESCENDANTS_EXPAND_TAIL: &str = ") FROM nodes UNION SELECT w.__path,w.root,e.dst,w.expand FROM walk w JOIN edges e ON e.__path=w.__path AND e.src=w.node WHERE w.expand) SELECT __path,root,node FROM walk";
/// The engine's cap on recursive CTE rows (`Exec::recursive`).
const RECURSIVE_CAP: usize = 50_000_000;

// ---------------------------------------------------------------------------------------------
// Dump groups

/// Captures of a dump member's tail, read from the facts.
#[derive(Clone)]
pub struct Tail {
    pub node: Name,
    pub parent: Option<Name>,
    /// Field captures: name and grammar field code (`None`: unknown field, never binds).
    pub fields: Vec<(Name, Option<u16>)>,
}

pub struct DumpMember {
    /// Position in `RelPlan::inputs` (and the rule's input index).
    pub plan_input: usize,
    pub rule_input: usize,
    /// The member's own selection conjuncts (before the shared gate).
    pub sel: Query,
    pub tail: Tail,
}

pub struct DumpGroup {
    pub members: Vec<DumpMember>,
    /// The selection suffix every member shares (capture-free).
    pub gate: Query,
    /// `any[inside{all[X, pattern $A], stopBy: X}, pattern $_]`: `A` is the nearest proper
    /// ancestor where X holds, bound when it is named (X two-valued, checked).
    pub ancestor: Option<(Name, Query)>,
}

/// Flattens nested `all`s; a `matches` conjunct of a policy utility is inlined (a utility that
/// binds captures is not memoised, so inlining it is its evaluation).
fn flat_all(v: &Json, utils: &BTreeMap<String, Json>, out: &mut Vec<Json>, inlined: &mut bool, depth: u32) {
    if let Some(o) = v.as_object() {
        if o.len() == 1 {
            if let Some(items) = o.get("all").and_then(Json::as_array) {
                for x in items {
                    flat_all(x, utils, out, inlined, depth);
                }
                return;
            }
            if let Some(u) = o.get("matches").and_then(Json::as_str).and_then(|n| utils.get(n)) {
                if depth < 4 {
                    *inlined = true;
                    flat_all(u, utils, out, inlined, depth + 1);
                    return;
                }
            }
        }
    }
    out.push(v.clone());
}

/// `any[inside{all[X, pattern $A], stopBy: X}, pattern $_]` -> (A, X).
fn ancestor_capture(v: &Json) -> Option<(&str, &Json)> {
    let items = v.as_object().filter(|o| o.len() == 1)?.get("any")?.as_array()?;
    if items.len() != 2 || metavar(&items[1]) != Some("_") {
        return None;
    }
    let inside = items[0].as_object().filter(|o| o.len() == 1)?.get("inside")?.as_object()?;
    if inside.len() != 2 {
        return None;
    }
    let all = inside.get("all")?.as_array()?;
    let stop = inside.get("stopBy")?;
    if all.len() != 2 || &all[0] != stop || !stop.is_object() {
        return None;
    }
    let name = metavar(&all[1]).filter(|n| *n != "_")?;
    Some((name, stop))
}

/// `$NAME` (named-node metavariable) -> `NAME` (`_` included).
fn meta_name(p: &str) -> Option<&str> {
    let rest = p.strip_prefix('$')?;
    (!rest.is_empty() && rest.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')).then_some(rest)
}

fn metavar(v: &Json) -> Option<&str> {
    let o = v.as_object()?;
    if o.len() != 1 {
        return None;
    }
    meta_name(o.get("pattern")?.as_str()?)
}

/// `any[REL{pattern $X, stopBy neighbor[, field f]}, pattern $_]` -> (relation, X, field).
fn optional_capture(v: &Json) -> Option<(&str, &str, Option<&str>)> {
    let items = v.as_object().filter(|o| o.len() == 1)?.get("any")?.as_array()?;
    if items.len() != 2 || metavar(&items[1]) != Some("_") {
        return None;
    }
    let o = items[0].as_object().filter(|o| o.len() == 1)?;
    let (rel, inner) = o.iter().next()?;
    let inner = inner.as_object()?;
    if inner.get("stopBy").and_then(Json::as_str) != Some("neighbor") {
        return None;
    }
    let field = inner.get("field").map(|f| f.as_str()).unwrap_or(None);
    let expected = if field.is_some() { 3 } else { 2 };
    if inner.len() != expected {
        return None;
    }
    let name = meta_name(inner.get("pattern")?.as_str()?)?;
    if name == "_" {
        return None;
    }
    // Only the keys `pattern`, `stopBy` and `field` may appear.
    if inner.keys().any(|k| !["pattern", "stopBy", "field"].contains(&k.as_str())) {
        return None;
    }
    Some((rel.as_str(), name, field))
}

/// A member's evidence split, or `None` when it is not a dump.
struct Split {
    /// Capture-free selection conjuncts, in order.
    sel: Vec<Json>,
    /// `$NODE` and the neighbour captures after it.
    tail: Vec<Json>,
    /// The ancestor capture (name, X) taken out of the selection.
    ancestor: Option<(String, Json)>,
    /// A utility was inlined.
    inlined: bool,
    ast: Json,
}

fn split_member(ev: &Json, utils: &BTreeMap<String, Json>) -> Option<Split> {
    let o = ev.as_object()?;
    if o.keys().any(|k| !["as", "query", "project"].contains(&k.as_str())) || o.get("as").and_then(Json::as_str) != Some("row") {
        return None;
    }
    let ast = o.get("query")?.as_object()?.get("ast")?.as_object()?;
    if ast.keys().any(|k| !["language", "syntax", "rule"].contains(&k.as_str())) {
        return None;
    }
    let mut conj = Vec::new();
    let mut inlined = false;
    flat_all(ast.get("rule")?, utils, &mut conj, &mut inlined, 0);
    let at = conj.iter().position(|c| metavar(c).is_some_and(|n| n != "_"))?;
    let (sel, tail) = conj.split_at(at);
    // The ancestor capture, if any, is the selection's last conjunct (nothing after it can read
    // its binding); every other selection conjunct is checked capture-free by the caller.
    let mut kept = sel.to_vec();
    let mut ancestor = None;
    if let Some((name, x)) = sel.last().and_then(ancestor_capture) {
        ancestor = Some((name.to_owned(), x.clone()));
        kept.pop();
    }
    Some(Split { sel: kept, tail: tail.to_vec(), ancestor, inlined, ast: Json::Object(ast.clone()) })
}

/// Columns of `SELECT <c or NULL AS c>, ... FROM <table>` arms of a `UNION ALL`.
fn parse_union_all(sql: &str) -> Option<Vec<(String, Vec<(String, bool)>)>> {
    let mut arms = Vec::new();
    for arm in norm(sql).split(" UNION ALL ") {
        let rest = arm.strip_prefix("SELECT ")?;
        let (items, table) = rest.rsplit_once(" FROM ")?;
        if !table.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return None;
        }
        let mut cols = Vec::new();
        for it in items.split(',') {
            let it = it.trim();
            if let Some(n) = it.strip_prefix("NULL AS ") {
                cols.push((n.to_string(), true));
            } else if it.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                cols.push((it.to_string(), false));
            } else {
                return None;
            }
        }
        arms.push((table.to_string(), cols));
    }
    Some(arms)
}

fn parse_tail(tail: &[Json], names: &mut Names, grammar: &rerail_engine::code_rule_dsl::tape::Grammar) -> Option<Tail> {
    let node = names.id(metavar(&tail[0])?);
    let mut out = Tail { node, parent: None, fields: Vec::new() };
    let mut seen = vec![node];
    for (i, t) in tail[1..].iter().enumerate() {
        let (rel, name, field) = optional_capture(t)?;
        let id = names.id(name);
        // Every capture is bound at most once (a repeated name would compare, not bind).
        if seen.contains(&id) {
            return None;
        }
        seen.push(id);
        match (rel, field) {
            ("inside", None) if i == 0 => out.parent = Some(id),
            ("has", Some(f)) => out.fields.push((id, grammar.field_code(f))),
            _ => return None,
        }
    }
    Some(out)
}

// ---------------------------------------------------------------------------------------------
// Stage kernels

/// One arm of the `nodes` source union: a table and, per union column, its column there (`None`:
/// `NULL AS`), plus the affinities of the stored `syntax_rows_k` stage it passed through.
#[derive(Clone)]
pub struct Arm {
    pub table: String,
    pub cols: Vec<Option<usize>>,
    pub stage_affs: Option<Vec<Aff>>,
}

#[derive(Clone)]
pub enum Kernel {
    /// A stage absorbed by a later kernel (`syntax_rows_k`, `node_ordinals`): not materialised.
    Absorbed,
    /// `nodes`: the union of `arms`, plus `id`, `parent`, `edge_parent` and `ordinal`. `join`: the
    /// `node_ordinals` variant (each row once per ordinals row with its path, span and kind).
    Nodes { arms: Vec<Arm>, join: bool },
    Edges,
    Ownership,
    Descendants { expand: Option<Vec<String>> },
}

/// Chooses substitutions for one relational clause. `ev_json`: the clause's input evidence by
/// name. Returns the dump group (if any) and sets `plan.kernels`.
pub fn plan_clause(
    rel: &Json,
    plan: &mut RelPlan,
    inputs: &[Input],
    names: &mut Names,
    grammar: &rerail_engine::code_rule_dsl::tape::Grammar,
    utils: &BTreeMap<String, Json>,
) -> Result<(), String> {
    if std::env::var_os("FACTS_A3_OFF").is_some() {
        return Ok(());
    }
    let stages_json: Vec<&Json> = rel.get("stages").and_then(Json::as_array).map(|a| a.iter().collect()).unwrap_or_default();
    let n = plan.stages.len();
    let mut kernels: Vec<Option<Kernel>> = vec![None; n];
    let name_of = |i: usize| match &plan.stages[i] {
        StagePlan::Query { name, .. } | StagePlan::Reach { name, .. } => name.to_ascii_lowercase(),
    };
    let sql_of = |i: usize| stages_json.get(i).and_then(|s| s.get("query")).and_then(Json::as_str).map(norm);
    let plain = |i: usize| matches!(&plan.stages[i], StagePlan::Query { deferred, .. } if deferred.is_empty());
    // Which statements read a table (stages and the final query).
    let readers = |table: &str| -> Vec<usize> {
        let mut out = Vec::new();
        for (i, st) in plan.stages.iter().enumerate() {
            let reads = match st {
                StagePlan::Query { stmt, deferred, .. } => stmt.reads.iter().chain(deferred.iter().flat_map(|(_, _, s)| s.reads.iter())).any(|r| r.eq_ignore_ascii_case(table)),
                StagePlan::Reach { roots, edges, .. } => roots.reads.iter().chain(edges.iter().flat_map(|e| e.reads.iter())).any(|r| r.eq_ignore_ascii_case(table)),
            };
            if reads {
                out.push(i);
            }
        }
        if plan.fin.reads.iter().any(|r| r.eq_ignore_ascii_case(table)) {
            out.push(usize::MAX);
        }
        out
    };
    let stage_idx = |name: &str| (0..n).find(|&i| name_of(i) == name);
    let input_pos = |name: &str| plan.inputs.iter().position(|(n, _)| n.eq_ignore_ascii_case(name));
    let schema_of = |i: usize| match &plan.stages[i] {
        StagePlan::Query { schema, .. } => Some(schema.clone()),
        _ => None,
    };

    // `nodes`: the window variant over `raw_nodes`/the two `syntax_rows_k`, or the ordinals-join
    // variant.
    let mut dump_tables: Vec<String> = Vec::new();
    if let Some(ni) = stage_idx("nodes").filter(|&i| plain(i)) {
        let sql = sql_of(ni).unwrap_or_default();
        let window_src = sql.strip_prefix(&format!("SELECT *,{NODE_COLS} FROM ")).map(str::to_owned);
        let join = sql == norm(NODES_JOIN);
        let ord_idx = stage_idx("node_ordinals");
        let union_src = window_src.as_deref() == Some(UNION01) || join;
        let mut arms: Option<Vec<Arm>> = None;
        if union_src {
            // Both `syntax_rows_k` must be plain UNION ALLs read only by `nodes` (and
            // `node_ordinals` in the join variant).
            let mut all = Vec::new();
            let mut ok = true;
            let mut width = None;
            for k in ["syntax_rows_0", "syntax_rows_1"] {
                let Some(si) = stage_idx(k).filter(|&i| plain(i) && i < ni) else { ok = false; break };
                let allowed: Vec<usize> = if join { vec![ni, ord_idx.unwrap_or(usize::MAX)] } else { vec![ni] };
                if readers(k).iter().any(|r| !allowed.contains(r)) {
                    ok = false;
                    break;
                }
                let Some(parsed) = sql_of(si).and_then(|s| parse_union_all(&s)) else { ok = false; break };
                let stage_schema = schema_of(si).unwrap();
                for (table, cols) in parsed {
                    let Some(ip) = input_pos(&table) else { ok = false; break };
                    let in_schema = &plan.input_schemas[ip];
                    let mut map = Vec::new();
                    for (c, null) in &cols {
                        if *null {
                            map.push(None);
                        } else {
                            match in_schema.cols.iter().position(|x| x == c) {
                                Some(p) => map.push(Some(p)),
                                None => {
                                    ok = false;
                                    break;
                                }
                            }
                        }
                    }
                    if !ok || map.len() != stage_schema.cols.len() {
                        ok = false;
                        break;
                    }
                    if *width.get_or_insert(map.len()) != map.len() {
                        ok = false;
                        break;
                    }
                    all.push(Arm { table: table.to_ascii_lowercase(), cols: map, stage_affs: Some(stage_schema.affs.clone()) });
                    kernels[si] = Some(Kernel::Absorbed);
                }
                if !ok {
                    break;
                }
            }
            if ok {
                arms = Some(all);
            } else {
                kernels.iter_mut().for_each(|k| *k = None);
            }
        } else if let Some(src) = window_src.filter(|s| s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')) {
            if let Some(ip) = input_pos(&src) {
                let w = plan.input_schemas[ip].cols.len();
                arms = Some(vec![Arm { table: src.to_ascii_lowercase(), cols: (0..w).map(Some).collect(), stage_affs: None }]);
            }
        }
        if let Some(arms) = arms {
            // The ordinals stage of the join variant is absorbed when only `nodes` reads it.
            let mut ok = true;
            if join {
                match ord_idx {
                    Some(oi) if plain(oi) && oi < ni && sql_of(oi).as_deref() == Some(&norm(NODE_ORDINALS)) && readers("node_ordinals").iter().all(|&r| r == ni) => kernels[oi] = Some(Kernel::Absorbed),
                    _ => ok = false,
                }
            }
            // The union's columns must be the ones the kernel reads, and `nodes` = union + 4.
            let ucols: Vec<String> = match &plan.stages[ni] {
                StagePlan::Query { schema, .. } => schema.cols.clone(),
                _ => unreachable!(),
            };
            let w = arms[0].cols.len();
            if ucols.len() != w + 4 || ucols[w..] != ["id", "parent", "edge_parent", "ordinal"] {
                ok = false;
            }
            for c in ["__path", "__start", "__end", "kind", "parent_start", "parent_end", "parent_kind", "ancestor_start", "ancestor_end", "ancestor_kind"] {
                if !ucols[..w].iter().any(|x| x == c) {
                    ok = false;
                }
            }
            if ok {
                dump_tables = arms.iter().map(|a| a.table.clone()).collect();
                kernels[ni] = Some(Kernel::Nodes { arms, join });
            } else {
                kernels.iter_mut().for_each(|k| *k = None);
            }
        }
    }
    if let Some(ni) = stage_idx("nodes").filter(|&i| kernels[i].is_some()) {
        if let Some(ei) = stage_idx("edges").filter(|&i| plain(i) && i > ni && sql_of(i).as_deref() == Some(&norm(EDGES))) {
            kernels[ei] = Some(Kernel::Edges);
            if let Some(oi) = stage_idx("ownership").filter(|&i| plain(i) && i > ei && sql_of(i).as_deref() == Some(&norm(OWNERSHIP))) {
                kernels[oi] = Some(Kernel::Ownership);
            }
            if let Some(di) = stage_idx("descendants").filter(|&i| plain(i) && i > ei) {
                let sql = sql_of(di).unwrap_or_default();
                if sql == norm(DESCENDANTS) {
                    kernels[di] = Some(Kernel::Descendants { expand: None });
                } else if let Some(kinds) = sql.strip_prefix(DESCENDANTS_EXPAND_HEAD).and_then(|s| s.strip_suffix(DESCENDANTS_EXPAND_TAIL)) {
                    let list: Option<Vec<String>> = kinds
                        .split(',')
                        .map(|k| k.strip_prefix('\'').and_then(|k| k.strip_suffix('\'')).filter(|k| !k.contains('\'')).map(str::to_owned))
                        .collect();
                    if let Some(list) = list {
                        kernels[di] = Some(Kernel::Descendants { expand: Some(list) });
                    }
                }
            }
        }
    }
    // A dump group: every arm table of `nodes` is a node-dumping input read by nothing else.
    let why = |n: u32| {
        if std::env::var_os("FACTS_A3_PLAN").is_some() {
            eprintln!("a3 plan: no dump group (check {n})");
        }
    };
    let dump = 'dump: {
        if dump_tables.is_empty() {
            break 'dump None;
        }
        let ev_by_name: BTreeMap<String, &Json> = rel["inputs"].as_object().ok_or("inputs")?.iter().map(|(k, v)| (k.to_ascii_lowercase(), &v["evidence"])).collect();
        let nodes_idx = stage_idx("nodes");
        let mut splits = Vec::new();
        for t in &dump_tables {
            if readers(t).iter().any(|&r| Some(r) != nodes_idx && !matches!(kernels.get(r), Some(Some(Kernel::Absorbed)))) {
                why(1);
                break 'dump None;
            }
            let Some(ev) = ev_by_name.get(t) else { break 'dump None };
            let Some(split) = split_member(ev, utils) else {
                why(2);
                break 'dump None;
            };
            let ip = input_pos(t).unwrap();
            let rule_input = plan.inputs[ip].1;
            let inp = &inputs[rule_input];
            if !matches!(inp.anchor, crate::eval::Anchor::Span) || inp.each.is_some() || inp.paths.is_some() || !inp.query.decoded.is_empty() || inp.query.syntax != SyntaxMode::Valid {
                why(3);
                break 'dump None;
            }
            splits.push((ip, rule_input, split));
        }
        let s0 = &splits[0].2;
        // One ancestor capture shared by every member, or none.
        if splits.iter().any(|(_, _, s)| s.ancestor != s0.ancestor || s.ast.get("syntax") != s0.ast.get("syntax") || s.ast.get("language") != s0.ast.get("language")) {
            why(4);
            break 'dump None;
        }
        // The shared gate: the longest common suffix of the members' selections.
        let mut common = s0.sel.len();
        for (_, _, s) in &splits[1..] {
            let mut k = 0;
            while k < common.min(s.sel.len()) && s0.sel[s0.sel.len() - 1 - k] == s.sel[s.sel.len() - 1 - k] {
                k += 1;
            }
            common = k;
        }
        if common == 0 {
            why(5);
            break 'dump None;
        }
        let ast_with = |ast: &Json, rule: Vec<Json>| -> Json {
            let mut a = ast.as_object().unwrap().clone();
            a.insert("rule".into(), serde_json::json!({ "all": rule }));
            Json::Object(a)
        };
        let gate_json = ast_with(&s0.ast, s0.sel[s0.sel.len() - common..].to_vec());
        let gate = Compiler::new(names, grammar, utils).query(&gate_json)?;
        if crate::ast::binds_any(&gate) {
            why(6);
            break 'dump None;
        }
        // An inlined utility or an ancestor capture: the whole selection must be two-valued (no
        // unknown verdicts whose propagation would differ).
        let strict = s0.ancestor.is_some() || splits.iter().any(|(_, _, s)| s.inlined);
        if strict && gate.three_valued {
            why(7);
            break 'dump None;
        }
        let ancestor = match &s0.ancestor {
            None => None,
            Some((name, x)) => {
                let xq = Compiler::new(names, grammar, utils).query(&ast_with(&s0.ast, vec![x.clone()]))?;
                if crate::ast::binds_any(&xq) || xq.three_valued {
                    why(8);
                    break 'dump None;
                }
                Some((names.id(name), xq))
            }
        };
        let mut members = Vec::new();
        for (ip, ri, s) in &splits {
            // A member's own selection must be two-valued: then splitting `all[own, gate]` at the
            // gate changes no verdict and no refusal.
            let own = s.sel[..s.sel.len() - common].to_vec();
            let q = Compiler::new(names, grammar, utils).query(&ast_with(&s.ast, own))?;
            let Some(tail) = parse_tail(&s.tail, names, grammar) else {
                why(9);
                break 'dump None;
            };
            if crate::ast::binds_any(&q) || q.three_valued {
                why(10);
                break 'dump None;
            }
            let caps: Vec<Name> = std::iter::once(tail.node).chain(tail.parent).chain(tail.fields.iter().map(|f| f.0)).chain(ancestor.as_ref().map(|a| a.0)).collect();
            if ancestor.as_ref().is_some_and(|a| caps.iter().filter(|&&c| c == a.0).count() > 1) {
                why(11);
                break 'dump None;
            }
            fn proj_ok(p: &Proj, caps: &[Name]) -> bool {
                match p {
                    Proj::Path | Proj::Directory | Proj::Name | Proj::Stem => true,
                    Proj::Capture(n) | Proj::Text(n) | Proj::Kind(n) | Proj::Start(n) | Proj::End(n) | Proj::Line(n) => caps.contains(n),
                    Proj::Regex(src, _, _) => proj_ok(src, caps),
                }
            }
            // No same-node regex group may stand in for an unbound capture.
            let groups = inputs[*ri].node_regexes.iter().any(|rx| rx.capture_names().flatten().next().is_some());
            if groups || !inputs[*ri].projs.iter().all(|p| proj_ok(p, &caps)) {
                why(12);
                break 'dump None;
            }
            members.push(DumpMember { plan_input: *ip, rule_input: *ri, sel: q, tail });
        }
        Some(DumpGroup { members, gate, ancestor })
    };
    plan.kernels = kernels;
    plan.dump = dump;
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Kernel execution

fn col(schema: &Schema, name: &str) -> Result<usize, String> {
    schema.cols.iter().position(|c| c == name).ok_or_else(|| format!("kernel: column {name} missing"))
}

fn table<'t>(db: &'t Db<'_>, name: &str) -> Result<&'t Rc<Table>, String> {
    db.tables.get(name).ok_or_else(|| format!("kernel: table {name} missing"))
}

/// `a||':'||b||':'||c` (NULL when any operand is NULL).
fn concat3(a: &V, b: &V, c: &V) -> V {
    match (sv::to_text(a), sv::to_text(b), sv::to_text(c)) {
        (Some(x), Some(y), Some(z)) => {
            let mut s = String::with_capacity(x.len() + y.len() + z.len() + 2);
            s.push_str(&x);
            s.push(':');
            s.push_str(&y);
            s.push(':');
            s.push_str(&z);
            V::Text(Rc::from(s))
        }
        _ => V::Null,
    }
}

/// CTAS storage of one row (`rel::store`): each column converted by its declared affinity.
fn store_row(row: &mut [V], affs: &[(usize, Aff)]) {
    for &(i, a) in affs {
        let v = std::mem::replace(&mut row[i], V::Null);
        row[i] = sv::apply_affinity(v, a);
    }
}

fn non_none(affs: &[Aff]) -> Vec<(usize, Aff)> {
    affs.iter().enumerate().filter(|(_, a)| **a != Aff::None).map(|(i, a)| (i, *a)).collect()
}

fn key2(a: &V, b: &V) -> Option<(sv::Key, sv::Key)> {
    Some((sv::key(a)?, sv::key(b)?))
}

pub fn run(k: &Kernel, db: &Db<'_>, out_schema: &Schema) -> Result<Table, String> {
    let rows = match k {
        Kernel::Absorbed => Vec::new(),
        Kernel::Nodes { arms, join } => nodes(db, arms, *join, out_schema)?,
        Kernel::Edges => edges(db)?,
        Kernel::Ownership => ownership(db)?,
        Kernel::Descendants { expand } => descendants(db, expand.as_deref())?,
    };
    let mut rows = rows;
    let affs = non_none(&out_schema.affs);
    if !affs.is_empty() {
        for r in rows.iter_mut() {
            store_row(r, &affs);
        }
    }
    Ok(Table { schema: out_schema.clone(), rows })
}

fn nodes(db: &Db<'_>, arms: &[Arm], join: bool, out: &Schema) -> Result<Vec<Box<[V]>>, String> {
    let w = out.cols.len() - 4;
    let ucol = |n: &str| out.cols[..w].iter().position(|c| c == n).ok_or_else(|| format!("kernel: column {n} missing"));
    let (path, start, end, kind) = (ucol("__path")?, ucol("__start")?, ucol("__end")?, ucol("kind")?);
    let (ps, pe, pk) = (ucol("parent_start")?, ucol("parent_end")?, ucol("parent_kind")?);
    let (as_, ae, ak) = (ucol("ancestor_start")?, ucol("ancestor_end")?, ucol("ancestor_kind")?);
    // The union, each arm through its stored stage's affinities.
    let mut u: Vec<Vec<V>> = Vec::new();
    for arm in arms {
        let t = table(db, &arm.table)?;
        let affs = arm.stage_affs.as_deref().map(non_none).unwrap_or_default();
        u.reserve(t.rows.len());
        for r in &t.rows {
            let mut row: Vec<V> = Vec::with_capacity(w + 4);
            for c in &arm.cols {
                row.push(match c {
                    Some(i) => r[*i].clone(),
                    None => V::Null,
                });
            }
            store_row(&mut row, &affs);
            u.push(row);
        }
    }
    // row_number() OVER(PARTITION BY __path,parent_start,parent_end,parent_kind ORDER BY
    // __start,__end,kind) - 1, ties in union order (the engine's window rule).
    let mut order: Vec<u32> = (0..u.len() as u32).collect();
    let part_cols = [path, ps, pe, pk];
    let ord_cols = [start, end, kind];
    order.sort_by(|&a, &b| {
        let (ra, rb) = (&u[a as usize], &u[b as usize]);
        for &c in &part_cols {
            let o = sv::cmp(&ra[c], &rb[c]);
            if o != Ordering::Equal {
                return o;
            }
        }
        for &c in &ord_cols {
            let o = sv::cmp(&ra[c], &rb[c]);
            if o != Ordering::Equal {
                return o;
            }
        }
        a.cmp(&b)
    });
    let mut ordinal = vec![0i64; u.len()];
    let mut n = 0i64;
    let mut prev: Option<u32> = None;
    for &i in &order {
        let same = prev.is_some_and(|p| part_cols.iter().all(|&c| sv::cmp(&u[p as usize][c], &u[i as usize][c]) == Ordering::Equal));
        n = if same { n + 1 } else { 1 };
        prev = Some(i);
        ordinal[i as usize] = n - 1;
    }
    if !join {
        let mut rows = Vec::with_capacity(u.len());
        for (i, mut r) in u.into_iter().enumerate() {
            let id = concat3(&r[kind], &r[start], &r[end]);
            let parent = concat3(&r[pk], &r[ps], &r[pe]);
            let edge = concat3(&r[ak], &r[as_], &r[ae]);
            r.push(id);
            r.push(parent);
            r.push(edge);
            r.push(V::Int(ordinal[i]));
            rows.push(r.into_boxed_slice());
        }
        return Ok(rows);
    }
    // The join variant: `n JOIN node_ordinals o ON path, start, end, kind equal`, n in union
    // order, o in its own (union) order.
    let mut by_key: FxHashMap<[sv::Key; 4], Vec<u32>> = FxHashMap::default();
    let k4 = |r: &[V]| -> Option<[sv::Key; 4]> { Some([sv::key(&r[path])?, sv::key(&r[start])?, sv::key(&r[end])?, sv::key(&r[kind])?]) };
    for (i, r) in u.iter().enumerate() {
        if let Some(k) = k4(r) {
            by_key.entry(k).or_default().push(i as u32);
        }
    }
    let mut rows = Vec::with_capacity(u.len());
    for r in &u {
        let Some(k) = k4(r) else { continue };
        let Some(matches) = by_key.get(&k) else { continue };
        for &o in matches {
            let mut row = r.clone();
            row.push(concat3(&r[kind], &r[start], &r[end]));
            row.push(concat3(&r[pk], &r[ps], &r[pe]));
            row.push(concat3(&r[ak], &r[as_], &r[ae]));
            row.push(V::Int(ordinal[o as usize]));
            rows.push(row.into_boxed_slice());
        }
    }
    Ok(rows)
}

fn edges(db: &Db<'_>) -> Result<Vec<Box<[V]>>, String> {
    let t = table(db, "nodes")?;
    let (p, e, id, o) = (col(&t.schema, "__path")?, col(&t.schema, "edge_parent")?, col(&t.schema, "id")?, col(&t.schema, "ordinal")?);
    Ok(t.rows.iter().filter(|r| !r[e].is_null()).map(|r| vec![r[p].clone(), r[e].clone(), r[id].clone(), r[o].clone()].into_boxed_slice()).collect())
}

fn is_text(v: &V, s: &str) -> bool {
    matches!(v, V::Text(t) if &**t == s)
}

fn ownership(db: &Db<'_>) -> Result<Vec<Box<[V]>>, String> {
    let nodes = table(db, "nodes")?;
    let edges = table(db, "edges")?;
    let (np, nid, nk, ne) = (col(&nodes.schema, "__path")?, col(&nodes.schema, "id")?, col(&nodes.schema, "kind")?, col(&nodes.schema, "edge_parent")?);
    let (ep, es, ed) = (col(&edges.schema, "__path")?, col(&edges.schema, "src")?, col(&edges.schema, "dst")?);
    let mut node_ix: FxHashMap<(sv::Key, sv::Key), Vec<u32>> = FxHashMap::default();
    for (i, r) in nodes.rows.iter().enumerate() {
        if let Some(k) = key2(&r[np], &r[nid]) {
            node_ix.entry(k).or_default().push(i as u32);
        }
    }
    let mut edge_ix: FxHashMap<(sv::Key, sv::Key), Vec<u32>> = FxHashMap::default();
    for (i, r) in edges.rows.iter().enumerate() {
        if let Some(k) = key2(&r[ep], &r[es]) {
            edge_ix.entry(k).or_default().push(i as u32);
        }
    }
    const FN: &str = "function_definition";
    const CLS: &str = "class_definition";
    let mut queue: Vec<Box<[V]>> = Vec::new();
    for r in &nodes.rows {
        let has_parent = key2(&r[np], &r[ne]).is_some_and(|k| node_ix.contains_key(&k));
        if has_parent {
            continue;
        }
        let f = if is_text(&r[nk], FN) { r[nid].clone() } else { V::Null };
        let c = if is_text(&r[nk], CLS) { r[nid].clone() } else { V::Null };
        queue.push(vec![r[np].clone(), r[nid].clone(), f.clone(), c, f].into_boxed_slice());
    }
    let mut result = Vec::new();
    let mut steps = 0usize;
    while !queue.is_empty() {
        steps += queue.len();
        if steps > RECURSIVE_CAP {
            return Err("recursive CTE exceeded 50M rows".into());
        }
        let batch = std::mem::take(&mut queue);
        for o in &batch {
            let Some(ek) = key2(&o[0], &o[1]) else { continue };
            let Some(es_) = edge_ix.get(&ek) else { continue };
            for &e in es_ {
                let er = &edges.rows[e as usize];
                let Some(ck) = key2(&er[ep], &er[ed]) else { continue };
                let Some(cs) = node_ix.get(&ck) else { continue };
                for &c in cs {
                    let ch = &nodes.rows[c as usize];
                    let is_fn = is_text(&ch[nk], FN);
                    let f = if is_fn { ch[nid].clone() } else { o[2].clone() };
                    let cl = if is_text(&ch[nk], CLS) { ch[nid].clone() } else { o[3].clone() };
                    let top = if o[4].is_null() && is_fn { ch[nid].clone() } else { o[4].clone() };
                    queue.push(vec![ch[np].clone(), ch[nid].clone(), f, cl, top].into_boxed_slice());
                }
            }
        }
        result.extend(batch);
    }
    Ok(result)
}

/// Interned `DKey`s: the recursion's distinctness over small integers.
struct Interner {
    ids: FxHashMap<DKey, u32>,
}
impl Interner {
    fn id(&mut self, v: &V) -> u32 {
        let k = sv::dkey(v);
        let n = self.ids.len() as u32;
        *self.ids.entry(k).or_insert(n)
    }
}

fn descendants(db: &Db<'_>, expand: Option<&[String]>) -> Result<Vec<Box<[V]>>, String> {
    let nodes = table(db, "nodes")?;
    let edges = table(db, "edges")?;
    let (np, nid, nk) = (col(&nodes.schema, "__path")?, col(&nodes.schema, "id")?, col(&nodes.schema, "kind")?);
    let (ep, es, ed) = (col(&edges.schema, "__path")?, col(&edges.schema, "src")?, col(&edges.schema, "dst")?);
    let mut it = Interner { ids: FxHashMap::default() };
    // Edges by (path, src) key, in table order; `=` never holds for NULL.
    let mut edge_ix: FxHashMap<(u32, u32), Vec<u32>> = FxHashMap::default();
    for (i, r) in edges.rows.iter().enumerate() {
        if r[ep].is_null() || r[es].is_null() {
            continue;
        }
        let k = (it.id(&r[ep]), it.id(&r[es]));
        edge_ix.entry(k).or_default().push(i as u32);
    }
    // walk(path, root, node[, expand]) rows as interned keys plus their values.
    struct W {
        path: u32,
        root: u32,
        node: u32,
        expand: u32,
        vals: [V; 3],
        flag: V,
    }
    let mut seen: rustc_hash::FxHashSet<(u32, u32, u32, u32)> = Default::default();
    let mut queue: Vec<W> = Vec::new();
    let mut admit = |w: W, queue: &mut Vec<W>| {
        if seen.insert((w.path, w.root, w.node, w.expand)) {
            queue.push(w);
        }
    };
    for r in &nodes.rows {
        let flag = match expand {
            None => V::Null,
            Some(list) => match &r[nk] {
                V::Null => V::Null,
                k => V::bool(!list.iter().any(|x| sv::cmp(k, &V::text(x)) == Ordering::Equal && matches!(k, V::Text(_)))),
            },
        };
        let w = W { path: it.id(&r[np]), root: it.id(&r[nid]), node: it.id(&r[nid]), expand: it.id(&flag), vals: [r[np].clone(), r[nid].clone(), r[nid].clone()], flag };
        admit(w, &mut queue);
    }
    let mut result = Vec::new();
    let mut steps = 0usize;
    while !queue.is_empty() {
        steps += queue.len();
        if steps > RECURSIVE_CAP {
            return Err("recursive CTE exceeded 50M rows".into());
        }
        let batch = std::mem::take(&mut queue);
        for w in &batch {
            if expand.is_some() && sv::truth(&w.flag) != Some(true) {
                continue;
            }
            if w.vals[0].is_null() || w.vals[2].is_null() {
                continue;
            }
            let Some(es_) = edge_ix.get(&(w.path, w.node)) else { continue };
            for &e in es_ {
                let er = &edges.rows[e as usize];
                let nw = W { path: w.path, root: w.root, node: it.id(&er[ed]), expand: w.expand, vals: [w.vals[0].clone(), w.vals[1].clone(), er[ed].clone()], flag: w.flag.clone() };
                admit(nw, &mut queue);
            }
        }
        result.extend(batch.into_iter().map(|w| {
            let [a, b, c] = w.vals;
            vec![a, b, c].into_boxed_slice()
        }));
    }
    Ok(result)
}

/// Rows equal as the engine's results (`FACTS_A3_CHECK`): same length, order, value types and
/// values (texts byte-equal, numbers bit-equal).
pub fn same_rows(a: &[Box<[V]>], b: &[Box<[V]>]) -> Result<(), String> {
    if a.len() != b.len() {
        return Err(format!("{} rows vs {}", a.len(), b.len()));
    }
    for (i, (x, y)) in a.iter().zip(b).enumerate() {
        let eq = x.len() == y.len()
            && x.iter().zip(y.iter()).all(|(p, q)| match (p, q) {
                (V::Null, V::Null) => true,
                (V::Int(m), V::Int(n)) => m == n,
                (V::Real(m), V::Real(n)) => m.to_bits() == n.to_bits(),
                (V::Text(m), V::Text(n)) => m == n,
                _ => false,
            });
        if !eq {
            return Err(format!("row {i}: {x:?} vs {y:?}"));
        }
    }
    Ok(())
}
