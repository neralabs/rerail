//! A content's inputs at a path, shaped as the X6 exports the readers were written against (`facts-a
//! xexport` of the pack, ids of `facts-b gnorm --dump all`): Y1's `3`, `7`–`13`
//! (`ygraph::summary`) and the symbol table's `14`–`26` (`y3::input`). Each export is the pack's
//! per-file stage over the inputs, as the pack's SQL states it (`proj_inventory`, the seed of
//! `proj_imports`' tokens, `proj_explicit_exports`, `dc__module_names`) or a cut of one input's
//! columns. Out of the pack's scope a path has no rows; an input's path predicate (`pack::Input::
//! file`) keeps its rows only where the path matches.
use super::cgnative::Variant;
use super::pack::{INPUTS, Table};
use super::{Produced, Producer};
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use rusqlite::types::Value;
use serde_json::Value as Json;
use std::collections::{BTreeMap, BTreeSet};

/// Export id → rows, as the export file holds them.
pub(crate) type Exports = BTreeMap<u32, Vec<Vec<Json>>>;

/// The exports Y1 and the symbol table read.
pub(crate) const READ: [u32; 21] = [3, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26];

/// The pack's scope as the compiler builds a rule's (`literal_separator` globs): included and
/// not excluded.
pub(crate) struct Scope([GlobSet; 2]);

impl Scope {
    pub fn new(p: &Producer) -> Scope {
        Scope(p.pack.scope.clone().map(|globs| {
            let mut b = GlobSetBuilder::new();
            for g in &globs {
                b.add(GlobBuilder::new(g).literal_separator(true).build().expect("scope glob"));
            }
            b.build().expect("scope globs")
        }))
    }

    pub fn admits(&self, path: &str) -> bool {
        self.0[0].is_match(path) && !self.0[1].is_match(path)
    }
}

fn j(v: &Value) -> Json {
    match v {
        Value::Null => Json::Null,
        Value::Integer(i) => Json::from(*i),
        Value::Text(t) => Json::String(t.clone()),
        Value::Real(r) => Json::from(*r),
        Value::Blob(_) => panic!("a blob in a projection"),
    }
}

fn text(s: &str) -> Json {
    Json::String(s.to_owned())
}

/// `replace(rx_replace(path, '/__init__\.py$|\.py$', ''), '/', '.')`
pub(crate) fn module_of(path: &str) -> String {
    let stem = path.strip_suffix("/__init__.py").or_else(|| path.strip_suffix(".py")).unwrap_or(path);
    stem.replace('/', ".")
}

/// `rx_replace(items, '#[^\n]*', '')`
fn strip_comments(items: &str) -> String {
    let mut out = String::with_capacity(items.len());
    let mut rest = items;
    while let Some(at) = rest.find('#') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        rest = &rest[rest.find('\n').unwrap_or(rest.len())..];
    }
    out.push_str(rest);
    out
}

/// `p`'s inputs at `path` as the exports Y1 and the symbol table read (`READ`).
pub(crate) fn exports(p: &Producer, scope: &Scope, path: &str, x: &Produced) -> Exports {
    let mut out: Exports = READ.iter().map(|&id| (id, Vec::new())).collect();
    if !scope.admits(path) {
        return out;
    }
    let pj = text(path);
    out.insert(3, vec![vec![pj.clone()]]);
    // Y1: the summaries, every variant, in `Variant::ALL` order.
    for (v, r) in Variant::ALL.iter().zip(&x.cgn) {
        for (id, rel, cols) in [(7, "cgn_def", 5), (8, "cgn_edge", 2), (9, "cgn_alias", 2), (10, "cgn_opaque", 1), (11, "cgn_module", 1), (12, "cgn_open", 1), (13, "cgn_reference_alias", 2)] {
            let rows = out.get_mut(&id).unwrap();
            for row in r.rels.get(rel).map(Vec::as_slice).unwrap_or_default() {
                let mut e = vec![pj.clone(), text(v.name())];
                e.extend(row[..cols].iter().map(|c| text(c)));
                rows.push(e);
            }
        }
    }
    // The symbol table: the pack's inputs (`file` predicates applied at this path).
    let input = |name: &str| -> (&super::pack::Input, &Table) {
        let i = INPUTS.iter().position(|n| *n == name).unwrap();
        (&p.pack.inputs[i], &x.inputs[i])
    };
    let rows_of = |name: &str| -> Vec<&(i64, i64, Vec<Value>)> {
        let (inp, t) = input(name);
        if inp.file.as_ref().is_some_and(|rx| !rx.is_match(path)) {
            return Vec::new();
        }
        t.rows.iter().collect()
    };
    let col = |name: &str, c: &str| Table::col(input(name).0, c);
    let module = module_of(path);
    // 14 proj_inventory: functions, then classes (UNION ALL).
    {
        let (n, cn, ra, cs, bs) = (col("py_functions", "name"), col("py_functions", "class_name"), col("py_functions", "return_annotation"), col("py_functions", "class_start"), col("py_functions", "body_start"));
        let mut rows = Vec::new();
        let qualified = |class: &Json, name: &Json, sep: &str| -> Json {
            let local = match class {
                Json::Null => name.as_str().unwrap().to_owned(),
                c => format!("{}.{}", c.as_str().unwrap(), name.as_str().unwrap()),
            };
            text(&format!("{module}{sep}{local}"))
        };
        for (s, e, v) in rows_of("py_functions") {
            let (name, class) = (j(&v[n]), j(&v[cn]));
            let kind = if class.is_null() { "function" } else { "method" };
            rows.push(vec![pj.clone(), Json::from(*s), Json::from(*e), name.clone(), class.clone(), j(&v[ra]), j(&v[cs]), j(&v[bs]), text(&module), text(kind), qualified(&class, &name, "::"), qualified(&class, &name, ".")]);
        }
        let (n, bs) = (col("py_classes", "name"), col("py_classes", "body_start"));
        for (s, e, v) in rows_of("py_classes") {
            let name = j(&v[n]);
            rows.push(vec![pj.clone(), Json::from(*s), Json::from(*e), name.clone(), Json::Null, Json::Null, Json::Null, j(&v[bs]), text(&module), text("class"), qualified(&Json::Null, &name, "::"), qualified(&Json::Null, &name, ".")]);
        }
        out.insert(14, rows);
    }
    // 15 py_parameters: __start, __end, annotation, name, owner.
    let (a, n, o) = (col("py_parameters", "annotation"), col("py_parameters", "name"), col("py_parameters", "owner"));
    out.insert(15, rows_of("py_parameters").iter().map(|(s, e, v)| vec![pj.clone(), Json::from(*s), Json::from(*e), j(&v[a]), j(&v[n]), j(&v[o])]).collect());
    // 16 return annotations: name, owner.
    let (n, o) = (col("py_return_annotations", "name"), col("py_return_annotations", "owner"));
    out.insert(16, rows_of("py_return_annotations").iter().map(|(_, _, v)| vec![pj.clone(), j(&v[n]), j(&v[o])]).collect());
    // 17 parameter annotations: name, parameter.
    let (n, pa) = (col("py_parameter_annotations", "name"), col("py_parameter_annotations", "parameter"));
    out.insert(17, rows_of("py_parameter_annotations").iter().map(|(_, _, v)| vec![pj.clone(), j(&v[n]), j(&v[pa])]).collect());
    // 18 proj_imports' token seeds: (path, ordinal, top, base, '', items less comments || ',', 0).
    let (b, it, t) = (col("py_imports", "base"), col("py_imports", "items"), col("py_imports", "top"));
    out.insert(
        18,
        rows_of("py_imports")
            .iter()
            .map(|(s, _, v)| {
                let items = match &v[it] {
                    Value::Text(x) => text(&format!("{},", strip_comments(x))),
                    _ => Json::Null,
                };
                vec![pj.clone(), Json::from(*s), j(&v[t]), j(&v[b]), text(""), items, Json::from(0)]
            })
            .collect(),
    );
    // 19 py_bases with their start; 23 the distinct (name, owner).
    let (n, o) = (col("py_bases", "name"), col("py_bases", "owner"));
    let bases = rows_of("py_bases");
    out.insert(19, bases.iter().map(|(s, _, v)| vec![pj.clone(), Json::from(*s), j(&v[n]), j(&v[o])]).collect());
    let mut seen = BTreeSet::new();
    out.insert(23, bases.iter().filter(|(_, _, v)| seen.insert((j(&v[n]).to_string(), j(&v[o]).to_string()))).map(|(_, _, v)| vec![pj.clone(), j(&v[n]), j(&v[o])]).collect());
    // 20 decorators: name, owner.
    let (n, o) = (col("py_decorators", "name"), col("py_decorators", "owner"));
    out.insert(20, rows_of("py_decorators").iter().map(|(_, _, v)| vec![pj.clone(), j(&v[n]), j(&v[o])]).collect());
    // 21 proj_explicit_exports: __start, array, selected (row_number by __start).
    let ar = col("py_all", "array");
    let mut all: Vec<_> = rows_of("py_all");
    all.sort_by_key(|r| r.0);
    out.insert(21, all.iter().enumerate().map(|(k, (s, _, v))| vec![pj.clone(), Json::from(*s), j(&v[ar]), Json::from(k as i64 + 1)]).collect());
    // 22 py_all_names: name, owner.
    let (n, o) = (col("py_all_names", "name"), col("py_all_names", "owner"));
    out.insert(22, rows_of("py_all_names").iter().map(|(_, _, v)| vec![pj.clone(), j(&v[n]), j(&v[o])]).collect());
    // 24 dc__module_names: the content's names under the path's module.
    out.insert(24, x.module_names.iter().map(|n| vec![text(&module), Json::String(n.clone())]).collect());
    // 25 dc__raw_dynamic_imports: entry.
    let en = col("dc__raw_dynamic_imports", "entry");
    out.insert(25, rows_of("dc__raw_dynamic_imports").iter().map(|(_, _, v)| vec![j(&v[en])]).collect());
    // 26 ef__raw_execution_calls: name, owner.
    let (n, o) = (col("ef__raw_execution_calls", "name"), col("ef__raw_execution_calls", "owner"));
    out.insert(26, rows_of("ef__raw_execution_calls").iter().map(|(_, _, v)| vec![pj.clone(), j(&v[n]), j(&v[o])]).collect());
    // The cuts of the annotation and decorator inputs are distinct (as the stages read them through
    // `UNION` and `EXISTS`): first occurrence kept.
    for id in [16, 17, 20] {
        let mut seen = BTreeSet::new();
        out.get_mut(&id).unwrap().retain(|r| seen.insert(Json::Array(r.clone()).to_string()));
    }
    out
}

/// `dc__module_names`' names of a content (its `module` is the path's): the distinct names of
/// `dc__raw_module_names` the stage keeps, in first-row order. Every condition reads the content
/// alone (`py_functions` at the same path), so this is computed once per content.
pub(crate) fn module_names(p: &Producer, inputs: &[Table]) -> Vec<String> {
    let at = |name: &str| INPUTS.iter().position(|n| *n == name).unwrap();
    let (fi, ri) = (at("py_functions"), at("dc__raw_module_names"));
    let fcn = Table::col(&p.pack.inputs[fi], "class_name");
    let fs = &inputs[fi].rows;
    let method_at = |at: &Value| match at {
        Value::Integer(a) => fs.iter().any(|(s, _, v)| s == a && !matches!(v[fcn], Value::Null)),
        _ => false,
    };
    let c = |name: &str| Table::col(&p.pack.inputs[ri], name);
    let (nm, dfn, dcl, fun, cls, cb, dv, nc, ctl) = (c("name"), c("decorated_function"), c("decorated_class"), c("function"), c("class"), c("class_body"), c("default_value"), c("nested_class"), c("control"));
    let some = |v: &Value| !matches!(v, Value::Null);
    let mut seen = BTreeSet::new();
    let mut names = Vec::new();
    for (_, _, v) in &inputs[ri].rows {
        let keep = some(&v[ctl])
            || some(&v[nc])
            || ((!some(&v[dfn]) || method_at(&v[dfn]))
                && (!some(&v[dcl]) || some(&v[cb]))
                && (!some(&v[cls]) || some(&v[cb]))
                && (!some(&v[fun]) || (some(&v[dv]) && method_at(&v[fun]))));
        if let (true, Value::Text(name)) = (keep, &v[nm]) {
            if seen.insert(name.clone()) {
                names.push(name.clone());
            }
        }
    }
    names
}
