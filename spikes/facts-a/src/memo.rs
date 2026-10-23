//! The digest memo (docs/facts/LOOP.md § Parity on changed plans only): one clause's per-file
//! results, stored under `(content hash, plan digest, schema version)` (and the path: the
//! policies read `file.path`, through scopes, `paths` filters, path regexes and `__path`, so a
//! result is the content's at that path), so that `facts-a repo` re-evaluates a file only for the
//! clauses whose plan digest it has not seen; `--no-memo` evaluates everything.
//!
//! An entry holds what `eval::eval_file` produces for the clause on the file: findings, failures,
//! counts (`crate::counts`, the per-file part), and for a repository clause its per-file input
//! rows (`spill::FileRows`), for a budget its rows. The repository phase (`eval::finish`) always
//! runs over them.
//!
//! The plan digest (`digest`) is blake3 over: the memo's format and `ENGINE_VERSION`, the fact
//! schema version, whether the rewritten plan runs (`FACTS_REWRITES`), the resolved rule document
//! without its clauses (id, scope, options), its `define` clauses (utils, `report_when`), the
//! call's scope, the clause as resolved (its evidence inputs included), the physical plan of every
//! pattern input the clause reads (`lowered::Plan::digest_text`, or why it runs on the leaf
//! executor) and the physical `EXPLAIN` of each of its relational stages (`physplan::clause`), and
//! the algebraic budget's `A` (`rerail_facts::budget`: a `budget-exceeded` failure is a fact about
//! the content and the plan under that `A`). A rewrite that changes a plan changes that plan's
//! digest and no other. An operator whose results change while every plan's text stays the same
//! must bump `ENGINE_VERSION`.
//!
//! Layout: `<dir>/s<schema version>/<h[0..2]>/<content hash>-<path hash[0..16]>.json.zst`, one
//! JSON object per (content, path): `{"entries": {"<digest hex>": entry}}`. Each file is written
//! by the one worker that evaluates that path (atomic rename), so runs never contend. The memo is
//! off under `FACTS_MUTANT` (a seeded mutant changes no plan text).
use crate::closure::Val;
use crate::counts::Counts;
use crate::eval::{BudgetRow, Failure, Finding};
use crate::spill::FileRows;
use crate::sql::value::V;
use serde_json::{Value as Json, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// Bumped when an operator's results change without any plan text changing (stale entries then
/// never match again).
pub const ENGINE_VERSION: u32 = 1;
const FORMAT: &str = "rerail.facts-memo/v1";

/// One clause's results on one (content, path).
#[derive(Default)]
pub struct Entry {
    pub findings: Vec<Finding>,
    pub failures: Vec<Failure>,
    pub counts: Counts,
    pub rows: Option<FileRows>,
    pub budget: Vec<BudgetRow>,
}

/// The memo directory (`FACTS_MEMO_DIR`, default `~/rerail-bench/facts/memo`), `None` when the
/// memo is off (`--no-memo`, or a seeded mutant).
pub fn dir(args: &[String]) -> Option<PathBuf> {
    if args.iter().any(|a| a == "--no-memo") || std::env::var("FACTS_MUTANT").is_ok_and(|m| !matches!(m.trim(), "" | "0")) {
        return None;
    }
    let base = std::env::var_os("FACTS_MEMO_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(std::env::var("HOME").unwrap()).join("rerail-bench/facts/memo"));
    Some(base.join(format!("s{}", crate::facts::SCHEMA_VERSION)))
}

/// A clause's plan digest (see the module doc).
pub fn digest(rule_level: &Json, defines: &[&Json], scope: Option<&Json>, clause: &Json, inputs: &[String], stages: &[String]) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    let text = json!({
        "format": FORMAT,
        "engine": ENGINE_VERSION,
        "schema": crate::facts::SCHEMA_VERSION,
        "rewrites": crate::lowered::rewrites(),
        "rule": rule_level,
        "defines": defines,
        "scope": scope,
        "clause": clause,
        "inputs": inputs,
        "stages": stages,
        "budget": rerail_facts::budget::A,
    });
    h.update(serde_json::to_string(&text).unwrap().as_bytes());
    *h.finalize().as_bytes()
}

pub fn hex(d: &[u8]) -> String {
    d.iter().map(|b| format!("{b:02x}")).collect()
}

fn file_of(dir: &Path, content: &[u8; 32], path: &str) -> PathBuf {
    let c = hex(content);
    let p = hex(&blake3::hash(path.as_bytes()).as_bytes()[..8]);
    dir.join(&c[..2]).join(format!("{c}-{p}.json.zst"))
}

fn v_json(v: &V) -> Json {
    match v {
        V::Null => Json::Null,
        V::Int(i) => json!(i),
        V::Real(r) => json!({"r": format!("{:016x}", r.to_bits())}),
        V::Text(t) => json!(&**t),
    }
}

fn json_v(j: &Json) -> V {
    match j {
        Json::Null => V::Null,
        Json::Number(n) => V::Int(n.as_i64().expect("memo: integer")),
        Json::String(s) => V::text(s),
        Json::Object(o) => V::Real(f64::from_bits(u64::from_str_radix(o["r"].as_str().expect("memo: real"), 16).expect("memo: real bits"))),
        other => panic!("memo: value {other}"),
    }
}

fn val_json(v: &Val) -> Json {
    match v {
        Val::Null => Json::Null,
        Val::Int(i) => json!(i),
        Val::Real(r) => json!({"r": format!("{:016x}", r.to_bits())}),
        Val::Text(t) => json!(&**t),
        Val::Bool(b) => json!(b),
    }
}

fn json_val(j: &Json) -> Val {
    match j {
        Json::Bool(b) => Val::Bool(*b),
        other => match json_v(other) {
            V::Null => Val::Null,
            V::Int(i) => Val::Int(i),
            V::Real(r) => Val::Real(r),
            V::Text(t) => Val::Text(t.to_string().into_boxed_str()),
        },
    }
}

fn entry_json(e: &Entry) -> Json {
    json!({
        "findings": e.findings.iter().map(|f| json!([f.path, f.line, f.code, f.column])).collect::<Vec<_>>(),
        "failures": e.failures.iter().map(|f| json!([f.path, f.message])).collect::<Vec<_>>(),
        "counts": e.counts,
        "rows": e.rows.as_ref().map(|r| json!({
            "tables": r.tables.iter().map(|t| t.iter().map(|row| row.iter().map(v_json).collect::<Vec<_>>()).collect::<Vec<_>>()).collect::<Vec<_>>(),
            "anchors": r.anchors,
        })),
        "budget": e.budget.iter().map(|b| json!([b.key, b.line, b.values.iter().map(val_json).collect::<Vec<_>>()])).collect::<Vec<_>>(),
    })
}

fn json_entry(j: &Json, check: &str, clause: &str, path: &Rc<str>) -> Entry {
    let s = |v: &Json| v.as_str().expect("memo: text").to_owned();
    Entry {
        findings: j["findings"].as_array().unwrap().iter().map(|f| Finding { check: check.into(), clause: clause.into(), path: s(&f[0]), line: f[1].as_u64().unwrap() as u32, code: s(&f[2]), column: f[3].as_u64().map(|c| c as u32) }).collect(),
        failures: j["failures"].as_array().unwrap().iter().map(|f| Failure { check: check.into(), clause: clause.into(), path: s(&f[0]), message: s(&f[1]) }).collect(),
        counts: serde_json::from_value(j["counts"].clone()).expect("memo: counts"),
        rows: j["rows"].as_object().map(|r| FileRows {
            path: path.clone(),
            tables: r["tables"].as_array().unwrap().iter().map(|t| t.as_array().unwrap().iter().map(|row| row.as_array().unwrap().iter().map(json_v).collect::<Vec<_>>().into_boxed_slice()).collect()).collect(),
            anchors: serde_json::from_value(r["anchors"].clone()).expect("memo: anchors"),
        }),
        budget: j["budget"].as_array().unwrap().iter().map(|b| BudgetRow { path: path.to_string(), key: s(&b[0]), line: b[1].as_u64().unwrap() as u32, values: b[2].as_array().unwrap().iter().map(json_val).collect() }).collect(),
    }
}

/// One (content, path)'s stored entries, by digest (raw JSON, decoded on use).
pub struct FileMemo {
    file: PathBuf,
    entries: serde_json::Map<String, Json>,
    added: bool,
}

impl FileMemo {
    pub fn load(dir: &Path, content: &[u8; 32], path: &str) -> FileMemo {
        let file = file_of(dir, content, path);
        let entries = std::fs::read(&file)
            .ok()
            .and_then(|z| zstd::decode_all(&z[..]).ok())
            .and_then(|b| serde_json::from_slice::<Json>(&b).ok())
            .and_then(|mut j| j.get_mut("entries").map(Json::take))
            .and_then(|e| match e {
                Json::Object(m) => Some(m),
                _ => None,
            })
            .unwrap_or_default();
        FileMemo { file, entries, added: false }
    }

    pub fn has(&self, digest: &[u8; 32]) -> bool {
        self.entries.contains_key(&hex(digest))
    }

    pub fn get(&self, digest: &[u8; 32], check: &str, clause: &str, path: &Rc<str>) -> Option<Entry> {
        self.entries.get(&hex(digest)).map(|j| json_entry(j, check, clause, path))
    }

    pub fn put(&mut self, digest: &[u8; 32], e: &Entry) {
        self.entries.insert(hex(digest), entry_json(e));
        self.added = true;
    }

    /// Writes the file back when entries were added (temporary file, then rename).
    pub fn save(self) {
        if !self.added {
            return;
        }
        let dir = self.file.parent().unwrap();
        std::fs::create_dir_all(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
        let body = serde_json::to_vec(&json!({"format": FORMAT, "entries": self.entries})).unwrap();
        let z = zstd::encode_all(&body[..], 3).unwrap();
        let tmp = self.file.with_extension(format!("tmp{}", std::process::id()));
        std::fs::write(&tmp, z).unwrap_or_else(|e| panic!("{}: {e}", tmp.display()));
        std::fs::rename(&tmp, &self.file).unwrap_or_else(|e| panic!("{}: {e}", self.file.display()));
    }
}

/// Per clause of a run: lookups served by the memo and evaluated (reported in `results.json`).
pub type Tally = BTreeMap<(usize, usize), (u64, u64)>;
