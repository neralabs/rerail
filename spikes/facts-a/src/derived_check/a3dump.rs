//! The `views-derived` oracle's node tables (harness only; the engine never runs this).
//!
//! S4's check compares the schema's views with the policies' shared stage texts over the nodes
//! where a clause's selection gate holds, forced `true`. It builds those node tables as Spike
//! A3's dump groups did: a clause's `syntax_N` inputs that its `syntax_rows_k` stages union into
//! `nodes`, each input's rule split into its own selection, the shared gate (the selections'
//! common suffix, replaced by `true` here) and the captures (`$NODE`, the parent, first field
//! children, the nearest ancestor where the gate holds), read from the facts. This is the
//! construction moved out of the deleted `subst.rs` (S7, wave 3), unchanged.
use crate::ast::{self, Bound, Compiler, Env, Name, Names, Query, SyntaxMode};
use crate::eval::{FileCtx, Input, Proj, Rule};
use crate::rel::{RelPlan, StagePlan};
use crate::sql::engine::Table;
use crate::sql::value::V;
use serde_json::Value as Json;
use std::collections::BTreeMap;
use std::rc::Rc;

fn norm(sql: &str) -> String {
    sql.split_whitespace().collect::<Vec<_>>().join(" ")
}

const NODE_COLS: &str = "kind||':'||__start||':'||__end AS id, parent_kind||':'||parent_start||':'||parent_end AS parent, ancestor_kind||':'||ancestor_start||':'||ancestor_end AS edge_parent, row_number() OVER(PARTITION BY __path,parent_start,parent_end,parent_kind ORDER BY __start,__end,kind)-1 AS ordinal";
const UNION01: &str = "(SELECT * FROM syntax_rows_0 UNION ALL SELECT * FROM syntax_rows_1)";
const NODE_ORDINALS: &str = "SELECT __path,__start,__end,kind,row_number() OVER(PARTITION BY __path,parent_start,parent_end,parent_kind ORDER BY __start,__end,kind)-1 AS ordinal FROM (SELECT * FROM syntax_rows_0 UNION ALL SELECT * FROM syntax_rows_1)";
const NODES_JOIN: &str = "SELECT n.*,n.kind||':'||n.__start||':'||n.__end AS id, n.parent_kind||':'||n.parent_start||':'||n.parent_end AS parent, n.ancestor_kind||':'||n.ancestor_start||':'||n.ancestor_end AS edge_parent,o.ordinal FROM (SELECT * FROM syntax_rows_0 UNION ALL SELECT * FROM syntax_rows_1) n JOIN node_ordinals o ON o.__path=n.__path AND o.__start=n.__start AND o.__end=n.__end AND o.kind=n.kind";

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
    if inner.keys().any(|k| !["pattern", "stopBy", "field"].contains(&k.as_str())) {
        return None;
    }
    Some((rel.as_str(), name, field))
}

/// A member's evidence split, or `None` when it is not a dump.
struct Split {
    sel: Vec<Json>,
    tail: Vec<Json>,
    ancestor: Option<(String, Json)>,
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

/// A relational clause's dump group: every arm table of its `nodes` source union is a
/// node-dumping input read by nothing else (A3's `subst::plan_clause`, dump part).
pub fn plan(rel: &Json, plan: &RelPlan, inputs: &[Input], names: &mut Names, grammar: &rerail_engine::code_rule_dsl::tape::Grammar, utils: &BTreeMap<String, Json>) -> Result<Option<DumpGroup>, String> {
    let stages_json: Vec<&Json> = rel.get("stages").and_then(Json::as_array).map(|a| a.iter().collect()).unwrap_or_default();
    let n = plan.stages.len();
    let name_of = |i: usize| match &plan.stages[i] {
        StagePlan::Query { name, .. } | StagePlan::Reach { name, .. } => name.to_ascii_lowercase(),
    };
    let sql_of = |i: usize| stages_json.get(i).and_then(|s| s.get("query")).and_then(Json::as_str).map(norm);
    let plain = |i: usize| matches!(&plan.stages[i], StagePlan::Query { deferred, .. } if deferred.is_empty());
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
    // The `nodes` stage's source arms, and the stages they pass through (read only by it).
    let Some(ni) = stage_idx("nodes").filter(|&i| plain(i)) else { return Ok(None) };
    let sql = sql_of(ni).unwrap_or_default();
    let window_src = sql.strip_prefix(&format!("SELECT *,{NODE_COLS} FROM ")).map(str::to_owned);
    let join = sql == norm(NODES_JOIN);
    let ord_idx = stage_idx("node_ordinals");
    let mut through: Vec<usize> = Vec::new();
    let mut arm_tables: Vec<String> = Vec::new();
    let mut width = None;
    if window_src.as_deref() == Some(UNION01) || join {
        for k in ["syntax_rows_0", "syntax_rows_1"] {
            let Some(si) = stage_idx(k).filter(|&i| plain(i) && i < ni) else { return Ok(None) };
            let allowed: Vec<usize> = if join { vec![ni, ord_idx.unwrap_or(usize::MAX)] } else { vec![ni] };
            if readers(k).iter().any(|r| !allowed.contains(r)) {
                return Ok(None);
            }
            let Some(parsed) = sql_of(si).and_then(|s| parse_union_all(&s)) else { return Ok(None) };
            let stage_schema = schema_of(si).unwrap();
            for (table, cols) in parsed {
                let Some(ip) = input_pos(&table) else { return Ok(None) };
                let in_schema = &plan.input_schemas[ip];
                if cols.iter().any(|(c, null)| !*null && !in_schema.cols.iter().any(|x| x == c)) || cols.len() != stage_schema.cols.len() {
                    return Ok(None);
                }
                if *width.get_or_insert(cols.len()) != cols.len() {
                    return Ok(None);
                }
                arm_tables.push(table.to_ascii_lowercase());
            }
            through.push(si);
        }
        if join {
            match ord_idx {
                Some(oi) if plain(oi) && oi < ni && sql_of(oi).as_deref() == Some(&norm(NODE_ORDINALS)) && readers("node_ordinals").iter().all(|&r| r == ni) => through.push(oi),
                _ => return Ok(None),
            }
        }
    } else if let Some(src) = window_src.filter(|s| s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')) {
        if input_pos(&src).is_none() {
            return Ok(None);
        }
        width = Some(plan.input_schemas[input_pos(&src).unwrap()].cols.len());
        arm_tables.push(src.to_ascii_lowercase());
    } else {
        return Ok(None);
    }
    // The union's columns must be the ones the dump reads, and `nodes` = union + 4.
    let ucols: Vec<String> = match &plan.stages[ni] {
        StagePlan::Query { schema, .. } => schema.cols.clone(),
        _ => unreachable!(),
    };
    let w = width.unwrap_or(0);
    if ucols.len() != w + 4 || ucols[w..] != ["id", "parent", "edge_parent", "ordinal"] {
        return Ok(None);
    }
    for c in ["__path", "__start", "__end", "kind", "parent_start", "parent_end", "parent_kind", "ancestor_start", "ancestor_end", "ancestor_kind"] {
        if !ucols[..w].iter().any(|x| x == c) {
            return Ok(None);
        }
    }
    let ev_by_name: BTreeMap<String, &Json> = rel["inputs"].as_object().ok_or("inputs")?.iter().map(|(k, v)| (k.to_ascii_lowercase(), &v["evidence"])).collect();
    let mut splits = Vec::new();
    for t in &arm_tables {
        if readers(t).iter().any(|&r| r != ni && !through.contains(&r)) {
            return Ok(None);
        }
        let Some(ev) = ev_by_name.get(t) else { return Ok(None) };
        let Some(split) = split_member(ev, utils) else { return Ok(None) };
        let ip = input_pos(t).unwrap();
        let rule_input = plan.inputs[ip].1;
        let inp = &inputs[rule_input];
        if !matches!(inp.anchor, crate::eval::Anchor::Span) || inp.each.is_some() || inp.paths.is_some() || !inp.query.decoded.is_empty() || inp.query.syntax != SyntaxMode::Valid {
            return Ok(None);
        }
        splits.push((ip, rule_input, split));
    }
    let s0 = &splits[0].2;
    if splits.iter().any(|(_, _, s)| s.ancestor != s0.ancestor || s.ast.get("syntax") != s0.ast.get("syntax") || s.ast.get("language") != s0.ast.get("language")) {
        return Ok(None);
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
        return Ok(None);
    }
    let ast_with = |ast: &Json, rule: Vec<Json>| -> Json {
        let mut a = ast.as_object().unwrap().clone();
        a.insert("rule".into(), serde_json::json!({ "all": rule }));
        Json::Object(a)
    };
    let gate_json = ast_with(&s0.ast, s0.sel[s0.sel.len() - common..].to_vec());
    let gate = Compiler::new(names, grammar, utils).query(&gate_json)?;
    if ast::binds_any(&gate) {
        return Ok(None);
    }
    let strict = s0.ancestor.is_some() || splits.iter().any(|(_, _, s)| s.inlined);
    if strict && gate.three_valued {
        return Ok(None);
    }
    let ancestor = match &s0.ancestor {
        None => None,
        Some((name, x)) => {
            let xq = Compiler::new(names, grammar, utils).query(&ast_with(&s0.ast, vec![x.clone()]))?;
            if ast::binds_any(&xq) || xq.three_valued {
                return Ok(None);
            }
            Some((names.id(name), xq))
        }
    };
    let mut members = Vec::new();
    for (ip, ri, s) in &splits {
        let own = s.sel[..s.sel.len() - common].to_vec();
        let q = Compiler::new(names, grammar, utils).query(&ast_with(&s.ast, own))?;
        let Some(tail) = parse_tail(&s.tail, names, grammar) else { return Ok(None) };
        if ast::binds_any(&q) || q.three_valued {
            return Ok(None);
        }
        let caps: Vec<Name> = std::iter::once(tail.node).chain(tail.parent).chain(tail.fields.iter().map(|f| f.0)).chain(ancestor.as_ref().map(|a| a.0)).collect();
        if ancestor.as_ref().is_some_and(|a| caps.iter().filter(|&&c| c == a.0).count() > 1) {
            return Ok(None);
        }
        fn proj_ok(p: &Proj, caps: &[Name]) -> bool {
            match p {
                Proj::Path | Proj::Directory | Proj::Name | Proj::Stem => true,
                Proj::Capture(n) | Proj::Text(n) | Proj::Kind(n) | Proj::Start(n) | Proj::End(n) | Proj::Line(n) => caps.contains(n),
                Proj::Regex(src, _, _) => proj_ok(src, caps),
            }
        }
        let groups = inputs[*ri].node_regexes.iter().any(|rx| rx.capture_names().flatten().next().is_some());
        if groups || !inputs[*ri].projs.iter().all(|p| proj_ok(p, &caps)) {
            return Ok(None);
        }
        members.push(DumpMember { plan_input: *ip, rule_input: *ri, sel: q, tail });
    }
    Ok(Some(DumpGroup { members, gate, ancestor }))
}

const PENDING: u32 = u32::MAX - 1;

/// The nearest proper ancestor of `n` where `x` holds, memoised in `near`.
fn nearest_where(f: &crate::facts::Facts, x: &ast::Eval<'_>, n: u32, near: &mut [u32], xval: &mut [u8], chain: &mut Vec<u32>) -> u32 {
    chain.clear();
    let mut cur = n;
    let ans = loop {
        if near[cur as usize] != PENDING {
            break near[cur as usize];
        }
        chain.push(cur);
        match f.parent(cur) {
            None => break crate::facts::NONE,
            Some(p) => {
                let v = &mut xval[p as usize];
                if *v == 0 {
                    *v = if x.match_root(p).is_some() { 1 } else { 2 };
                }
                if *v == 1 {
                    break p;
                }
                cur = p;
            }
        }
    };
    for &c in chain.iter() {
        near[c as usize] = ans;
    }
    ans
}

/// A dump group's member tables in one pass over the facts: per member, its own selection over
/// its candidate roots in preorder; the gate once per node; the captures from the facts. `None`
/// when an evaluation refused or decoded a named escape.
pub fn rows(rule: &Rule, g: &DumpGroup, plan: &RelPlan, ctx: &FileCtx<'_>) -> Option<Vec<Table>> {
    let f = ctx.f;
    let schemas = || g.members.iter().map(|m| plan.input_schemas[m.plan_input].clone());
    let mut tables: Vec<Vec<Box<[V]>>> = g.members.iter().map(|_| Vec::new()).collect();
    if f.syntax_errors {
        return Some(schemas().map(Table::empty).collect());
    }
    let saved = crate::pyvalue::NAMED_ESCAPES.with(|n| n.replace(0));
    let gate = ast::Eval::new(f, ctx.path, &g.gate);
    let mut verdict = vec![0u8; f.len()];
    let path: Rc<str> = Rc::from(ctx.path);
    let mut refused = false;
    let mut env = Env::default();
    let anc_ev = g.ancestor.as_ref().map(|(_, xq)| ast::Eval::new(f, ctx.path, xq));
    let mut near = if anc_ev.is_some() { vec![PENDING; f.len()] } else { Vec::new() };
    let mut xval = if anc_ev.is_some() { vec![0u8; f.len()] } else { Vec::new() };
    let mut chain: Vec<u32> = Vec::new();
    let mut selected: Vec<u32> = Vec::new();
    let mut ancestors: Vec<u32> = Vec::new();
    for (mi, m) in g.members.iter().enumerate() {
        let inp = &rule.inputs[m.rule_input];
        let ev = ast::Eval::new(f, ctx.path, &m.sel);
        selected.clear();
        for root in ev.roots() {
            if ev.match_root(root).is_none() {
                continue;
            }
            let v = &mut verdict[root as usize];
            if *v == 0 {
                *v = if gate.match_root(root).is_some() { 1 } else { 2 };
            }
            if *v == 1 && f.named(root) {
                selected.push(root);
            }
        }
        if ev.unsupported.borrow().is_some() {
            refused = true;
            break;
        }
        ancestors.clear();
        if let Some(xev) = &anc_ev {
            for &root in &selected {
                ancestors.push(nearest_where(f, xev, root, &mut near, &mut xval, &mut chain));
            }
        }
        let rows = &mut tables[mi];
        rows.reserve(selected.len());
        for (k, &root) in selected.iter().enumerate() {
            env.vars.clear();
            env.vars.push((m.tail.node, Bound::Node(root)));
            if let Some(p) = m.tail.parent {
                if let Some(par) = f.parent(root).filter(|&x| f.named(x)) {
                    env.vars.push((p, Bound::Node(par)));
                }
            }
            for &(name, code) in &m.tail.fields {
                let Some(code) = code else { continue };
                if let Some(c) = f.children(root).iter().copied().find(|&c| f.field[c as usize] == code) {
                    if f.named(c) {
                        env.vars.push((name, Bound::Node(c)));
                    }
                }
            }
            if let Some((aname, _)) = &g.ancestor {
                let a = ancestors[k];
                if a != crate::facts::NONE && f.named(a) {
                    env.vars.push((*aname, Bound::Node(a)));
                }
            }
            let (s, e) = (f.start[root as usize], f.end[root as usize]);
            let mut row = Vec::with_capacity(3 + inp.projs.len());
            row.push(V::Text(path.clone()));
            row.push(V::Int(s as i64));
            row.push(V::Int(e as i64));
            for p in &inp.projs {
                row.push(match p {
                    Proj::Capture(n) | Proj::Text(n) => match env.get(*n) {
                        Some(Bound::Node(x)) => V::text(f.text(x)),
                        _ => V::Null,
                    },
                    Proj::Kind(n) => match env.get(*n) {
                        Some(Bound::Node(x)) => V::Text(crate::facts::name(f.kind[x as usize])),
                        _ => V::Null,
                    },
                    Proj::Start(n) => env.get(*n).map_or(V::Null, |b| V::Int(gate.range_of(b).0 as i64)),
                    Proj::End(n) => env.get(*n).map_or(V::Null, |b| V::Int(gate.range_of(b).1 as i64)),
                    Proj::Line(n) => env.get(*n).map_or(V::Null, |b| V::Int(f.lines.line(gate.range_of(b).0 as usize) as i64)),
                    _ => crate::eval::project(p, &gate, &env, &[], ctx, &inp.query),
                });
            }
            rows.push(row.into_boxed_slice());
        }
    }
    if gate.unsupported.borrow().is_some() || anc_ev.as_ref().is_some_and(|x| x.unsupported.borrow().is_some()) {
        refused = true;
    }
    if crate::pyvalue::NAMED_ESCAPES.with(|n| n.replace(saved)) > 0 {
        refused = true;
    }
    (!refused).then(|| schemas().zip(tables).map(|(s, rows)| Table::from_rows(s, rows)).collect())
}
