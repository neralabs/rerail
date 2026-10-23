//! The digest memo (docs/facts/LOOP.md § Parity on changed plans only), in the content store
//! (`rerail_engine::content_store`, `facts_memo.rs`): one clause's per-file results under
//! `(content hash, plan digest, fact schema version)`, plus the path for a clause whose result
//! reads it, so that `facts-a repo` and the daemon re-evaluate a file only for the clauses whose
//! plan digest the store has not seen on its content; `--no-memo` evaluates everything.
//!
//! An entry holds what `eval::eval_file` produces for the clause on the file: findings, failures,
//! counts (`crate::counts`, the per-file part), and for a repository clause its per-file input
//! rows (`spill::FileRows`), for a budget its rows. The repository phase (`eval::finish`) always
//! runs over them. A clause run with nothing to report is a completion bit of the run's plan set
//! with its counts; anything else is a result row holding the encoded entry.
//!
//! Path: a clause `eval::Rule::content_pure` admits is stored once per content, its paths elided
//! (a finding's, failure's and budget row's path, and the first column of its repository rows,
//! which is the path by construction) and filled in on read, so identical content at two paths
//! is evaluated once; any other clause is stored under its path. The rule's scope picks clauses by
//! path before the memo is read.
//!
//! The plan digest (`digest`) is blake3 over: the memo's format and `ENGINE_VERSION`, the fact
//! schema version, whether the rewritten plan runs (`FACTS_REWRITES`), the resolved rule document
//! without its clauses (id, scope, options), its `define` clauses (utils, `report_when`), the
//! call's scope, the clause as resolved (its evidence inputs included), the physical plan of every
//! pattern input the clause reads (`lowered::Plan::digest_text`, or why it runs on the leaf
//! executor) and the physical `EXPLAIN` of each of its relational stages (`physplan::clause`). A
//! rewrite that changes a plan changes that plan's digest and no other. An operator whose results
//! change while every plan's text stays the same must bump `ENGINE_VERSION`.
//!
//! The store: `FACTS_STORE` (a content store file), else `FACTS_MEMO_DIR`'s (default
//! `~/rerail-bench/facts/memo`) `STORE_FILE_NAME`. The memo is off under `FACTS_MUTANT` (a seeded
//! mutant changes no plan text).
use crate::closure::Val;
use crate::counts::Counts;
use crate::eval::{BudgetRow, Failure, Finding, Rule};
use crate::spill::FileRows;
use crate::sql::value::V;
use rerail_engine::content_store::{ContentStore, FactsCompletion, FactsMemo, FactsMemoKey, FactsPlanSet, FactsResult, SourceId, STORE_FILE_NAME};
use serde_json::{Value as Json, json};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;

/// Bumped when an operator's results change without any plan text changing (stale entries then
/// never match again).
pub const ENGINE_VERSION: u32 = 1;
/// Part of every plan digest: the memo key is fixed across the store move (PHASE-3.md § Rules).
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

impl Entry {
    /// Nothing to report: stored as a completion bit with its counts.
    fn is_empty(&self) -> bool {
        self.findings.is_empty() && self.failures.is_empty() && self.rows.is_none() && self.budget.is_empty()
    }
}

/// The store file the memo lives in, `None` when the memo is off (`--no-memo`, or a seeded
/// mutant).
pub fn store_path(args: &[String]) -> Option<PathBuf> {
    if args.iter().any(|a| a == "--no-memo") || std::env::var("FACTS_MUTANT").is_ok_and(|m| !matches!(m.trim(), "" | "0")) {
        return None;
    }
    if let Some(file) = std::env::var_os("FACTS_STORE") {
        return Some(PathBuf::from(file));
    }
    let base = std::env::var_os("FACTS_MEMO_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(std::env::var("HOME").unwrap()).join("rerail-bench/facts/memo"));
    Some(base.join(STORE_FILE_NAME))
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
    });
    h.update(serde_json::to_string(&text).unwrap().as_bytes());
    *h.finalize().as_bytes()
}

pub fn hex(d: &[u8]) -> String {
    d.iter().map(|b| format!("{b:02x}")).collect()
}

/// The memo of one compiled policy set: the store, the set's plan digests in clause order, and per
/// clause its ordinal in the set and whether it is content-pure.
pub struct Memo {
    pub store: ContentStore,
    pub path: PathBuf,
    set: FactsPlanSet,
    /// Per rule, the ordinal of its first clause.
    first: Vec<u32>,
    pure: Vec<Vec<bool>>,
}

impl Memo {
    pub fn open(path: PathBuf, rules: &[Rule]) -> Memo {
        let store = ContentStore::open(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let mut first = Vec::with_capacity(rules.len());
        let mut plans = Vec::new();
        for rule in rules {
            first.push(plans.len() as u32);
            plans.extend_from_slice(&rule.digests);
        }
        Memo { store, path, set: FactsPlanSet::new(plans), first, pure: rules.iter().map(Rule::content_pure).collect() }
    }

    pub fn pure(&self, ri: usize, ci: usize) -> bool {
        self.pure[ri][ci]
    }

    /// The store alone (the daemon's memory attribution drops the memo's own index first).
    pub fn into_store(self) -> ContentStore {
        self.store
    }

    /// The content's source, if the store holds it.
    pub fn source(&self, content: &[u8; 32]) -> Option<SourceId> {
        self.store.facts_source(*content).unwrap_or_else(|e| panic!("memo: {e}"))
    }

    /// What the store holds for (content, path); the content is stored on `save` if it is new.
    pub fn file(&self, content: [u8; 32], path: &str) -> FileMemo<'_> {
        match self.source(&content) {
            Some(source) => self.file_at(source, path),
            None => FileMemo { memo: self, content: Err(content), path: path.to_owned(), held: FactsMemo::default(), results: Vec::new(), completions: Vec::new() },
        }
    }

    /// What the store holds for (a stored content, path).
    pub fn file_at(&self, source: SourceId, path: &str) -> FileMemo<'_> {
        let held = self.store.facts_memo(FactsMemoKey { source, fact_schema: crate::facts::SCHEMA_VERSION, path }).unwrap_or_else(|e| panic!("memo: {e}"));
        FileMemo { memo: self, content: Ok(source), path: path.to_owned(), held, results: Vec::new(), completions: Vec::new() }
    }

    /// Clause `(ri, ci)`'s entry encoded for the store at `path` (`Send`, unlike `Entry`, so an
    /// evaluation thread can hand it over): counts alone for an entry with nothing to report.
    pub fn encode(&self, ri: usize, ci: usize, e: &Entry, path: &str) -> Encoded {
        let pure = self.pure(ri, ci);
        if e.is_empty() {
            Encoded { empty: true, pure, bytes: serde_json::to_vec(&e.counts).unwrap() }
        } else {
            Encoded { empty: false, pure, bytes: serde_json::to_vec(&entry_json(e, pure.then_some(path))).unwrap() }
        }
    }
}

/// An entry as the store holds it (`Memo::encode`).
pub struct Encoded {
    empty: bool,
    pure: bool,
    bytes: Vec<u8>,
}

/// An entry's findings and failures alone (the daemon's per-file results), read without building
/// its repository or budget rows.
#[derive(serde::Deserialize)]
struct Reported {
    findings: Vec<(Option<String>, u32, String, Option<u32>)>,
    failures: Vec<(Option<String>, String)>,
}

/// One (content, path)'s stored entries and the entries a fresh evaluation adds.
pub struct FileMemo<'m> {
    memo: &'m Memo,
    /// The stored source, or the blake3 hash of a content the store does not hold yet.
    content: Result<SourceId, [u8; 32]>,
    path: String,
    held: FactsMemo,
    results: Vec<FactsResult>,
    completions: Vec<FactsCompletion>,
}

impl FileMemo<'_> {
    pub fn has(&self, digest: &[u8; 32]) -> bool {
        self.held.results.contains_key(digest) || self.held.completed.contains_key(digest)
    }

    pub fn get(&self, digest: &[u8; 32], check: &str, clause: &str, path: &Rc<str>) -> Option<Entry> {
        if let Some(r) = self.held.results.get(digest) {
            let j: Json = serde_json::from_slice(&r.entry).expect("memo: entry");
            return Some(json_entry(&j, check, clause, path));
        }
        self.held.completed.get(digest).map(|(_, counts)| Entry { counts: serde_json::from_slice(counts).expect("memo: counts"), ..Entry::default() })
    }

    /// The clause's stored findings and failures (`get` without the rows and counts).
    pub fn get_reported(&self, digest: &[u8; 32], check: &str, clause: &str, path: &str) -> Option<(Vec<Finding>, Vec<Failure>)> {
        if let Some(r) = self.held.results.get(digest) {
            let j: Reported = serde_json::from_slice(&r.entry).expect("memo: entry");
            let p = |v: Option<String>| v.unwrap_or_else(|| path.to_owned());
            return Some((
                j.findings.into_iter().map(|(fp, line, code, column)| Finding { check: check.into(), clause: clause.into(), path: p(fp), line, code, column }).collect(),
                j.failures.into_iter().map(|(fp, message)| Failure { check: check.into(), clause: clause.into(), path: p(fp), message }).collect(),
            ));
        }
        self.held.completed.contains_key(digest).then(|| (Vec::new(), Vec::new()))
    }

    /// Records clause `(ri, ci)`'s fresh entry (written by `save`).
    pub fn put(&mut self, ri: usize, ci: usize, digest: &[u8; 32], e: &Entry) {
        let encoded = self.memo.encode(ri, ci, e, &self.path);
        self.put_encoded(ri, ci, digest, encoded);
    }

    pub fn put_encoded(&mut self, ri: usize, ci: usize, digest: &[u8; 32], e: Encoded) {
        if e.empty {
            self.completions.push(FactsCompletion { ordinal: self.memo.first[ri] + ci as u32, path_dependent: !e.pure, counts: e.bytes });
        } else {
            self.results.push(FactsResult { plan: *digest, path_dependent: !e.pure, entry: e.bytes });
        }
    }

    /// Writes the fresh entries (storing the content first if the store lacks it).
    pub fn save(self, bytes: &[u8]) {
        if self.results.is_empty() && self.completions.is_empty() {
            return;
        }
        let store = &self.memo.store;
        let source = match self.content {
            Ok(s) => s,
            Err(content) => store.put_facts_source(bytes, content, &[]).unwrap_or_else(|e| panic!("memo: {e}")),
        };
        store
            .put_facts_memo(FactsMemoKey { source, fact_schema: crate::facts::SCHEMA_VERSION, path: &self.path }, &self.memo.set, &self.results, &self.completions)
            .unwrap_or_else(|e| panic!("memo: {e}"));
    }
}

fn v_json(v: &V) -> Json {
    match v {
        V::Null => Json::Null,
        V::Int(i) => json!(i),
        V::Real(r) => json!({"r": format!("{:016x}", r.to_bits())}),
        V::Text(t) => json!(&**t),
    }
}

/// `{"p": 0}`: the reading path (a content-pure entry's elided path column).
fn json_v(j: &Json, path: &Rc<str>) -> V {
    match j {
        Json::Null => V::Null,
        Json::Number(n) => V::Int(n.as_i64().expect("memo: integer")),
        Json::String(s) => V::text(s),
        Json::Object(o) if o.contains_key("p") => V::Text(path.clone()),
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

fn json_val(j: &Json, path: &Rc<str>) -> Val {
    match j {
        Json::Bool(b) => Val::Bool(*b),
        other => match json_v(other, path) {
            V::Null => Val::Null,
            V::Int(i) => Val::Int(i),
            V::Real(r) => Val::Real(r),
            V::Text(t) => Val::Text(t.to_string().into_boxed_str()),
        },
    }
}

/// The entry as JSON. `elide`: a content-pure clause's path, written as null (findings, failures,
/// budget rows) and `{"p": 0}` (the repository rows' first column); a path anywhere else would
/// contradict `Rule::content_pure` and panics.
fn entry_json(e: &Entry, elide: Option<&str>) -> Json {
    let path = |p: &str| -> Json {
        match elide {
            Some(own) => {
                assert_eq!(p, own, "memo: a content-pure clause reported another path");
                Json::Null
            }
            None => json!(p),
        }
    };
    json!({
        "findings": e.findings.iter().map(|f| json!([path(&f.path), f.line, f.code, f.column])).collect::<Vec<_>>(),
        "failures": e.failures.iter().map(|f| json!([path(&f.path), f.message])).collect::<Vec<_>>(),
        "counts": e.counts,
        "rows": e.rows.as_ref().map(|r| json!({
            "tables": r.tables.iter().map(|t| t.iter().map(|row| row.iter().enumerate().map(|(c, v)| match (elide, c, v) {
                (Some(own), 0, V::Text(t)) if &**t == own => json!({"p": 0}),
                (Some(_), 0, other) => panic!("memo: a content-pure clause's rows hold {other:?} in the path column"),
                _ => v_json(v),
            }).collect::<Vec<_>>()).collect::<Vec<_>>()).collect::<Vec<_>>(),
            "anchors": r.anchors,
        })),
        "budget": e.budget.iter().map(|b| json!([path(&b.path), b.key, b.line, b.values.iter().map(val_json).collect::<Vec<_>>()])).collect::<Vec<_>>(),
    })
}

fn json_entry(j: &Json, check: &str, clause: &str, path: &Rc<str>) -> Entry {
    let s = |v: &Json| v.as_str().expect("memo: text").to_owned();
    let p = |v: &Json| v.as_str().map_or_else(|| path.to_string(), str::to_owned);
    Entry {
        findings: j["findings"].as_array().unwrap().iter().map(|f| Finding { check: check.into(), clause: clause.into(), path: p(&f[0]), line: f[1].as_u64().unwrap() as u32, code: s(&f[2]), column: f[3].as_u64().map(|c| c as u32) }).collect(),
        failures: j["failures"].as_array().unwrap().iter().map(|f| Failure { check: check.into(), clause: clause.into(), path: p(&f[0]), message: s(&f[1]) }).collect(),
        counts: serde_json::from_value(j["counts"].clone()).expect("memo: counts"),
        rows: j["rows"].as_object().map(|r| FileRows {
            path: path.clone(),
            tables: r["tables"].as_array().unwrap().iter().map(|t| t.as_array().unwrap().iter().map(|row| row.as_array().unwrap().iter().map(|v| json_v(v, path)).collect::<Vec<_>>().into_boxed_slice()).collect()).collect(),
            anchors: serde_json::from_value(r["anchors"].clone()).expect("memo: anchors"),
        }),
        budget: j["budget"].as_array().unwrap().iter().map(|b| BudgetRow { path: p(&b[0]), key: s(&b[1]), line: b[2].as_u64().unwrap() as u32, values: b[3].as_array().unwrap().iter().map(|v| json_val(v, path)).collect() }).collect(),
    }
}

/// Per clause of a run: lookups served by the memo and evaluated (reported in `results.json`).
pub type Tally = BTreeMap<(usize, usize), (u64, u64)>;
