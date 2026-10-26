//! Per-content symbol-table inputs: the rows of the X6 pack's per-file exports (`facts-a
//! xexport` of `symbols-merged`, the same file Y1 reads its `cgn_*` rows from), one `SymBlock`
//! per (path, content), with everything that is a function of the content alone computed here:
//! each definition candidate's traits (`dc__dead_traits`, `ef__traits` minus the declaration
//! selection), its annotation names (`proj_annotation_names` at that candidate), the import
//! tokens resolved against the path's module (`proj_imports` before its window), base-class
//! rows, `__all__` rows, module names, and the two per-content probes (base-name categories,
//! dynamic-import targets).
//!
//! Export ids (`facts-b gnorm --dump all` of the pack): 14 `proj_inventory`, 15 `py_parameters`,
//! 16 return annotations, 17 parameter annotations, 18 `py_imports` token seeds, 19 `py_bases`
//! with their start, 20 `py_decorators`, 21 `proj_explicit_exports`, 22 `py_all_names`, 23
//! `py_bases` (distinct), 24 `dc__module_names`, 25 `dc__raw_dynamic_imports` entries, 26
//! `ef__raw_execution_calls`.
use crate::y2::facts::{sym, Kind};
use rayon::prelude::*;
use regex::Regex;
use rerail_facts::sql::value as sv;
use rustc_hash::FxHashMap;
use serde_json::Value as Json;
use std::path::Path;
use std::sync::LazyLock;

/// Trait bits of a candidate.
pub const ABSTRACT: u16 = 1;
pub const PROPERTY: u16 = 2;
pub const IMPLICIT: u16 = 4;
pub const PROTOCOL: u16 = 8;
pub const STRUCTURAL: u16 = 16;
pub const LEGACY_SYNC: u16 = 32;
pub const LEGACY_ASYNC: u16 = 64;
pub const LAZY_DECORATED: u16 = 128;
pub const PRIVATE_API: u16 = 256;
pub const EXEC_SURFACE: u16 = 512;
pub const DIRECT_LAZY: u16 = 1024;

/// One `proj_inventory` row with its per-content traits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cand {
    pub start: i64,
    pub end: i64,
    pub name: u32,
    pub class_name: Option<u32>,
    pub ret_ann: Option<u32>,
    pub class_start: Option<i64>,
    pub body_start: Option<i64>,
    pub kind: Kind,
    pub id: u32,
    pub fqname: u32,
    pub base_name: u32,
    pub class_id: Option<u32>,
    pub bits: u16,
    /// `ef__traits.params`: the JSON array of the parameters' names
    pub params: u32,
    /// `proj_annotation_names` of the candidate's symbol (sorted, distinct)
    pub ann: Box<[u32]>,
}

/// One `proj_imports` token after `relative` (before the window).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tok {
    pub ordinal: i64,
    pub top: Option<i64>,
    pub name: u32,
    pub target: u32,
    /// `base IS NULL`
    pub module_kind: bool,
    pub position: i64,
    /// `module || '.' || name` of a symbol token (what `proj_resolved_names` joins on); 0 for
    /// a module token
    pub fq: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BaseRow {
    pub start: i64,
    pub owner: i64,
    /// `trim(rx_replace(rx_replace(name,'(?s)\[.*$',''),'^.*\.',''))`
    pub short: u32,
}

#[derive(Default, Debug)]
pub struct SymBlock {
    /// The path's module text (`replace(rx_replace(path,'/__init__\.py$|\.py$',''),'/','.')`).
    pub module: u32,
    pub cands: Vec<Cand>,
    pub toks: Vec<Tok>,
    pub bases: Vec<BaseRow>,
    /// `proj_explicit_exports` row with `selected = 1`: (start, array IS NOT NULL)
    pub explicit: Option<(i64, bool)>,
    /// `py_all_names` (name, owner)
    pub all_names: Vec<(u32, i64)>,
    /// `dc__module_names` names (their module is the block's)
    pub module_names: Vec<u32>,
    /// y2-probe rows: (owner, intent base, grammar base)
    pub probe: Vec<(i64, bool, bool)>,
    /// y2-dyn targets (distinct)
    pub dyn_fqs: Vec<u32>,
}

impl SymBlock {
    pub fn bytes(&self) -> usize {
        std::mem::size_of::<SymBlock>()
            + self.cands.capacity() * std::mem::size_of::<Cand>()
            + self.cands.iter().map(|c| c.ann.len() * 4).sum::<usize>()
            + self.toks.capacity() * std::mem::size_of::<Tok>()
            + self.bases.capacity() * std::mem::size_of::<BaseRow>()
            + self.all_names.capacity() * 16
            + self.module_names.capacity() * 4
            + self.probe.capacity() * 16
            + self.dyn_fqs.capacity() * 4
    }
}

/// Every (path, content)'s block.
#[derive(Default)]
pub struct Blocks {
    pub by_key: FxHashMap<(u32, u32), SymBlock>,
}

impl Blocks {
    pub fn get(&self, path: u32, content: u32) -> Option<&SymBlock> {
        self.by_key.get(&(path, content))
    }
    pub fn bytes(&self) -> usize {
        self.by_key.capacity() * (8 + std::mem::size_of::<SymBlock>() + 1) + self.by_key.values().map(|b| b.bytes() - std::mem::size_of::<SymBlock>()).sum::<usize>()
    }
}

struct Rxs {
    module_of: Regex,
    abstract_: Regex,
    property: Regex,
    dunder: Regex,
    implicit_dec: Regex,
    protocol_name: Regex,
    legacy_sync: Regex,
    legacy_async: Regex,
    lazy_dec: Regex,
    lazy_end: Regex,
    private_api: Regex,
    suffix_las: Regex,
    suffix_al: Regex,
    exec_paths: Regex,
    exec_class: Regex,
    direct_legacy: Regex,
    comment: Regex,
    first_word: Regex,
    alias: Regex,
    head: Regex,
    last_seg: Regex,
    bracket: Regex,
    to_dot: Regex,
    grammar: Regex,
    dyn_entry: Regex,
}

static RX: LazyLock<Rxs> = LazyLock::new(|| {
        let r = |p: &str| Regex::new(p).unwrap();
        Rxs {
            module_of: r(r"/__init__\.py$|\.py$"),
            abstract_: r("abstractmethod$"),
            property: r(r"property$|\.setter$|\.deleter$"),
            dunder: r("^__.*__$"),
            implicit_dec: r("validator$|overload$"),
            protocol_name: r("Protocol$"),
            legacy_sync: r("(?:legacy_sync|legacy_module_sync|direct_legacy_module_sync)$"),
            legacy_async: r("(?:legacy_async|legacy_module_async|direct_legacy_module_async)$"),
            lazy_dec: r(r"pure_lazy|apply_lazy_to_public_lazy_methods|(?:^|\.)lazy$"),
            lazy_end: r("_lazy$"),
            private_api: r(r"(?:^|\.)(?:internal_api|private_api|private_public_api)$"),
            suffix_las: r("_(?:lazy|async|sync)$"),
            suffix_al: r("_(?:async|lazy)$"),
            exec_paths: r("(?:/graph_query|/schema_transaction|/transaction|/versioned_transactions)"),
            exec_class: r("(?:connection|query|schematransaction|transaction|versioned)"),
            direct_legacy: r(r"(?:^|\.)direct_legacy[^.]*$"),
            comment: r("#[^\n]*"),
            first_word: r(r"^(\S+)"),
            alias: r(r"\s+as\s+(\S+)$"),
            head: r("^[^.]+"),
            last_seg: r(r"\.?[^.]+$"),
            bracket: r(r"(?s)\[.*$"),
            to_dot: r(r"^.*\."),
            grammar: r("Sql|Grammar"),
            dyn_entry: r(r#"(?:\bimport_module|\bload_python_module|\bimportlib\.import_module)\(\s*['"](?P<module>[^'"]+)['"]\s*\)\.(?P<attr>[A-Za-z_][A-Za-z0-9_]*)"#),
        }
});

fn rx() -> &'static Rxs {
    &RX
}

/// `replace(rx_replace(path, '/__init__\.py$|\.py$', ''), '/', '.')`
pub fn module_of(path: &str) -> String {
    rx().module_of.replace_all(path, "").replace('/', ".")
}

/// `rx_replace(s, '\.?[^.]+$', '')`
fn strip_last(s: &str) -> String {
    rx().last_seg.replace_all(s, "").into_owned()
}

const EXEC_BASES: [&str; 46] = [
    "aggregate", "commit", "commit_branch", "count", "create_branch", "cursor_paginate", "cursor_paginate_with_total", "delete_by_id", "delete_by_key", "delete_by_key_many", "delete_many", "execute",
    "execute_one", "exists", "find_object_branches", "find_object_revisions", "first", "fts_search", "get_history", "graph", "graph_query", "insert", "insert_many", "insert_many_raw", "keyset_paginate",
    "keyset_paginate_with_total", "query", "query_unfiltered", "read", "read_version", "rollback", "search", "select_at_revision", "select_many", "select_one", "table_exists", "traverse", "update",
    "update_by_key", "update_by_key_many", "update_many", "upsert", "upsert_many", "vector_search", "write", "write_version",
];
const EXEC_ANN: [&str; 9] = ["databaseconnection", "lazyresult", "lazyseq", "readtransaction", "serverplan", "versioneddatabaseconnection", "versionedreadtransaction", "versionedwritetransaction", "writetransaction"];
const LAZY_ANN: [&str; 3] = ["lazyresult", "lazyseq", "serverplan"];
const EXEC_CALLS: [&str; 21] = [
    "_fetchall", "_fetchone", "execute", "execute_async", "execute_fire_and_forget", "execute_grammar", "execute_grammar_fetchall", "execute_grammar_fetchone", "execute_raw", "fetchall", "fetchall_raw",
    "fetchone", "fetchone_raw", "grammar_effect_lazy", "grammar_fetchall", "grammar_fetchall_async", "grammar_fetchone", "grammar_fetchone_async", "grammar_row_lazy", "grammar_rows_lazy", "raw_execute",
];

/// One export line's rows, parsed (strings not interned yet).
struct Parsed {
    path: String,
    hash: String,
    rows: [Vec<Vec<Json>>; 13],
}

fn parse_line(line: &[u8]) -> Parsed {
    let mut j: Json = serde_json::from_slice(line).expect("export line");
    let path = j["path"].as_str().unwrap().to_owned();
    let hash = j["hash"].as_str().unwrap().to_owned();
    let ex = j["exports"].as_object_mut().unwrap();
    let mut rows: [Vec<Vec<Json>>; 13] = Default::default();
    for (i, id) in (14..=26).enumerate() {
        if let Some(Json::Array(rs)) = ex.remove(&id.to_string()) {
            rows[i] = rs.into_iter().map(|r| match r {
                Json::Array(v) => v,
                o => panic!("export row {o}"),
            }).collect();
        }
    }
    Parsed { path, hash, rows }
}

fn s(v: &Json) -> &str {
    v.as_str().unwrap_or_else(|| panic!("expected text, got {v}"))
}
fn os(v: &Json) -> Option<&str> {
    match v {
        Json::Null => None,
        o => Some(s(o)),
    }
}
fn i(v: &Json) -> i64 {
    v.as_i64().unwrap_or_else(|| panic!("expected integer, got {v}"))
}
fn oi(v: &Json) -> Option<i64> {
    match v {
        Json::Null => None,
        o => Some(i(o)),
    }
}

/// The block of one parsed line.
fn block(p: &Parsed) -> SymBlock {
    let r = rx();
    let path = p.path.as_str();
    let module = module_of(path);
    let [inv, params, ret_ann, par_ann, imps, bases_s, decs, explicit, all_names, bases_d, modnames, dyns, execs] = &p.rows;
    let decs: Vec<(&str, i64)> = decs.iter().map(|x| (s(&x[1]), i(&x[2]))).collect();
    let bases_d: Vec<(&str, i64)> = bases_d.iter().map(|x| (s(&x[1]), i(&x[2]))).collect();
    let execs: Vec<(&str, i64)> = execs.iter().map(|x| (s(&x[1]), i(&x[2]))).collect();
    let dec_at = |o: i64, re: &Regex| decs.iter().any(|(n, w)| *w == o && re.is_match(n));
    let dec_at2 = |o: i64, c: Option<i64>, re: &Regex| decs.iter().any(|(n, w)| (*w == o || Some(*w) == c) && re.is_match(n));
    let base_at = |o: i64, f: &dyn Fn(&str) -> bool| bases_d.iter().any(|(n, w)| *w == o && f(n));
    // (start, name, owner) of every parameter; annotation names by parameter start
    let params: Vec<(i64, &str, i64)> = params.iter().map(|x| (i(&x[1]), s(&x[4]), i(&x[5]))).collect();
    let mut cands = Vec::with_capacity(inv.len());
    for x in inv {
        assert_eq!(s(&x[0]), path);
        assert_eq!(s(&x[8]), module, "{path}: inventory module");
        let (start, end, name) = (i(&x[1]), i(&x[2]), s(&x[3]));
        let class_name = os(&x[4]);
        let ret = os(&x[5]);
        let class_start = oi(&x[6]);
        let kind = match s(&x[9]) {
            "function" => Kind::Function,
            "method" => Kind::Method,
            "class" => Kind::Class,
            o => panic!("kind {o}"),
        };
        let mut bits = 0u16;
        let abstract_ = if kind == Kind::Class { base_at(start, &|n| n.contains("ABC")) } else { dec_at(start, &r.abstract_) };
        let property = dec_at(start, &r.property);
        let implicit = r.dunder.is_match(name) || ["main", "pytest_configure", "pytest_collection_modifyitems"].contains(&name) || dec_at(start, &r.implicit_dec);
        let structural = base_at(start, &|n| n.contains("Protocol"));
        let protocol = r.protocol_name.is_match(name) || structural;
        // proj_parameters at the candidate (if it is the selected declaration of its id)
        let mine: Vec<&(i64, &str, i64)> = {
            let mut v: Vec<&(i64, &str, i64)> = params.iter().filter(|p| p.2 == start && (class_name.is_none() || !["self", "cls"].contains(&p.1))).collect();
            v.sort_by_key(|p| p.0);
            v
        };
        let mut ann: Vec<&str> = ret_ann.iter().filter(|a| i(&a[2]) == start).map(|a| s(&a[1])).collect();
        ann.extend(par_ann.iter().filter(|a| mine.iter().any(|p| p.0 == i(&a[2]))).map(|a| s(&a[1])));
        ann.sort_unstable();
        ann.dedup();
        let ann_lower = |set: &[&str]| ann.iter().any(|a| set.contains(&a.to_ascii_lowercase().as_str()));
        let legacy_sync = dec_at(start, &r.legacy_sync);
        let legacy_async = dec_at(start, &r.legacy_async);
        let lazy_decorated = dec_at(start, &r.lazy_dec) || (!name.starts_with('_') && r.lazy_end.is_match(name) && dec_at2(start, class_start, &r.lazy_dec));
        let private_api = dec_at2(start, class_start, &r.private_api);
        let base = r.suffix_las.replace_all(name, "");
        let not_lazy_fn = !(kind == Kind::Function
            && ["l2055_lazy_result/lazy_reads.py", "l2055_lazy_result/lazy_writes.py", "l2055_lazy_result/transform.py"].contains(&path)
            && name.starts_with("lazy_")
            && (r.lazy_end.is_match(name) || ann_lower(&LAZY_ANN)));
        let ctx_class = ["l2055_lazy_result/execution_context.py", "l2055_lazy_result/lazy_result.py", "l2055_lazy_result/lazy_seq.py", "l2055_lazy_result/transform.py"].contains(&path)
            && class_name.is_some_and(|c| ["BatchContext", "EagerContext", "LazyResult", "LazySeq", "OnAccessContext"].contains(&c));
        let self_return = match (class_name, ret) {
            (Some(c), Some(ra)) => {
                let t = sv::trim(ra, Some(" '\""), true, true);
                t == c || t.starts_with(&format!("{c}["))
            }
            _ => false,
        };
        let exec_surface = !dec_at(start, &r.abstract_)
            && !private_api
            && not_lazy_fn
            && !ctx_class
            && !self_return
            && ((EXEC_BASES.contains(&base.as_ref())
                && (r.exec_paths.is_match(path) || r.exec_class.is_match(&class_name.unwrap_or("").to_ascii_lowercase()) || ann_lower(&EXEC_ANN)))
                || (r.lazy_end.is_match(name) || ann_lower(&LAZY_ANN))
                || legacy_sync
                || legacy_async);
        let direct_lazy = dec_at(start, &r.direct_legacy)
            && execs.iter().any(|(n, w)| {
                *w == start && (EXEC_CALLS.contains(n) || n.starts_with("execute_grammar") || n.starts_with("grammar_fetch")) && (r.lazy_end.is_match(n) || n.starts_with("lazy_"))
            });
        for (b, f) in [
            (abstract_, ABSTRACT), (property, PROPERTY), (implicit, IMPLICIT), (protocol, PROTOCOL), (structural, STRUCTURAL), (legacy_sync, LEGACY_SYNC), (legacy_async, LEGACY_ASYNC),
            (lazy_decorated, LAZY_DECORATED), (private_api, PRIVATE_API), (exec_surface, EXEC_SURFACE), (direct_lazy, DIRECT_LAZY),
        ] {
            if b {
                bits |= f;
            }
        }
        let params_json = serde_json::to_string(&mine.iter().map(|p| p.1).collect::<Vec<_>>()).unwrap();
        cands.push(Cand {
            start,
            end,
            name: sym(name),
            class_name: class_name.map(sym),
            ret_ann: ret.map(sym),
            class_start,
            body_start: oi(&x[7]),
            kind,
            id: sym(s(&x[10])),
            fqname: sym(s(&x[11])),
            base_name: sym(&r.suffix_al.replace_all(name, "")),
            class_id: class_name.map(|c| sym(&format!("{module}::{c}"))),
            bits,
            params: sym(&params_json),
            ann: ann.into_iter().map(sym).collect(),
        });
    }
    // proj_imports: tokens, parts, relative, resolved (per path)
    let mut toks = Vec::new();
    let init_path = sv::like("%/__init__.py", path);
    for x in imps {
        let (ordinal, top, base) = (i(&x[1]), oi(&x[2]), os(&x[3]));
        let mut rest = s(&x[5]).to_owned();
        let mut position = 0i64;
        // `rest` always ends with ',' (the seed appends one): each step takes the text up to
        // the first comma, until nothing is left.
        while !rest.is_empty() {
            let k = rest.find(',').unwrap_or_else(|| panic!("{path}: import items without a trailing comma"));
            let item = sv::trim(&rest[..k], Some(" ()\n\r"), true, true);
            rest = rest[k + 1..].to_owned();
            position += 1;
            if item.is_empty() {
                continue;
            }
            let Some(original) = r.first_word.captures(&item).and_then(|c| c.get(1)).map(|m| m.as_str().to_owned()) else { continue };
            if original == "*" {
                continue;
            }
            let alias = r.alias.captures(&item).and_then(|c| c.get(1)).map(|m| m.as_str().to_owned());
            // relative
            let mut prefix = if init_path { module.clone() } else { strip_last(&module) };
            let mut remaining = match base {
                None => -1,
                Some(b) => b.chars().count() as i64 - b.trim_start_matches('.').chars().count() as i64 - 1,
            };
            while remaining > 0 {
                prefix = strip_last(&prefix);
                remaining -= 1;
            }
            let target = match base {
                None => original.clone(),
                Some(b) if !b.starts_with('.') => format!("{b}.{original}"),
                Some(b) => {
                    let lb = b.trim_start_matches('.');
                    format!("{}{}{original}", if prefix.is_empty() { String::new() } else { format!("{prefix}.") }, if lb.is_empty() { String::new() } else { format!("{lb}.") })
                }
            };
            let name = match alias {
                Some(a) => a,
                None => match base {
                    None => r.head.find(&original).map(|m| m.as_str().to_owned()).unwrap_or_else(|| panic!("{path}: import name of {original:?} is NULL")),
                    Some(_) => original.clone(),
                },
            };
            let fq = if base.is_none() { 0 } else { sym(&format!("{module}.{name}")) };
            toks.push(Tok { ordinal, top, name: sym(&name), target: sym(&target), module_kind: base.is_none(), position, fq });
        }
    }
    let bases = bases_s
        .iter()
        .map(|x| {
            let n = s(&x[2]);
            let short = sv::trim(&r.to_dot.replace_all(&r.bracket.replace_all(n, ""), ""), None, true, true);
            BaseRow { start: i(&x[1]), owner: i(&x[3]), short: sym(&short) }
        })
        .collect();
    let explicit = explicit.iter().find(|x| i(&x[3]) == 1).map(|x| (i(&x[1]), !x[2].is_null()));
    let all_names = all_names.iter().map(|x| (sym(s(&x[1])), i(&x[2]))).collect();
    let module_names = modnames
        .iter()
        .map(|x| {
            assert_eq!(s(&x[0]), module, "{path}: module names' module");
            sym(s(&x[1]))
        })
        .collect();
    let mut owners: Vec<i64> = bases_d.iter().map(|b| b.1).collect();
    owners.sort_unstable();
    owners.dedup();
    let probe = owners
        .into_iter()
        .filter_map(|o| {
            let intent = base_at(o, &|n| n.contains("ServerPlanStep"));
            let grammar = base_at(o, &|n| r.grammar.is_match(n));
            (intent || grammar).then_some((o, intent, grammar))
        })
        .collect();
    let mut dyn_fqs: Vec<u32> = dyns
        .iter()
        .filter_map(|x| {
            let c = r.dyn_entry.captures(s(&x[0]))?;
            Some(sym(&format!("{}.{}", c.name("module")?.as_str(), c.name("attr")?.as_str())))
        })
        .collect();
    dyn_fqs.sort_unstable();
    dyn_fqs.dedup();
    SymBlock { module: sym(&module), cands, toks, bases, explicit, all_names, module_names, probe, dyn_fqs }
}

/// Load every line of `file` (only (path, hash) pairs `want` admits, when given) into `out`.
/// Lines are parsed in parallel; blocks are built (and strings interned) on this thread.
pub fn load(file: &Path, out: &mut Blocks, want: Option<&(dyn Fn(&str, &str) -> bool + Sync)>) -> usize {
    let data = std::fs::read(file).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
    let mut lines: Vec<&[u8]> = Vec::new();
    let mut at = 0;
    for nl in memchr::memchr_iter(b'\n', &data) {
        lines.push(&data[at..nl]);
        at = nl + 1;
    }
    if at < data.len() {
        lines.push(&data[at..]);
    }
    let mut loaded = 0;
    for chunk in lines.chunks(4096) {
        let parsed: Vec<Parsed> = chunk
            .par_iter()
            .filter(|l| !l.is_empty())
            .filter(|l| match want {
                None => true,
                Some(f) => {
                    let s = std::str::from_utf8(l).unwrap();
                    let h = s.rfind("\"hash\":\"").map(|i| &s[i + 8..i + 72]).unwrap();
                    let p = s.rfind("\"path\":").map(|i| &s[i + 7..s.len() - 1]).unwrap();
                    let p: String = serde_json::from_str(p).unwrap();
                    f(&p, h)
                }
            })
            .map(|l| parse_line(l))
            .collect();
        for p in parsed {
            let key = (sym(&p.path), sym(&p.hash));
            if out.by_key.contains_key(&key) {
                continue;
            }
            out.by_key.insert(key, block(&p));
            loaded += 1;
        }
    }
    loaded
}
