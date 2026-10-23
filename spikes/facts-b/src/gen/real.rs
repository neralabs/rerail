//! Real repository plans: rules compiled by `rerail_facts::compile` (S1), their repository plans
//! normalised by the generator, their exported relations read per (path, content) from the
//! per-file producer's output (`facts-a xexport` JSON lines), on demand.
use super::norm::{self, Graph, Op};
use super::val::{self, Row, Val};
use rerail_facts::front::rule::LeafCheck;
use rerail_facts::ir::{ExportedRelation, Plan, RepoSource};
use rustc_hash::FxHashMap;
use serde_json::{Map, Value as Json};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::io::{BufRead, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// Global export ids: (program index + 1) × STRIDE + the program's export id (below STRIDE: the
/// test vectors' exports).
pub const STRIDE: u32 = 1000;

/// The leaf check is the per-file executor's; the repository side only needs the plans.
struct Accept;
impl LeafCheck for Accept {
    fn pattern(&mut self, _ast: &Json, _utils: &BTreeMap<String, Json>) -> Result<Vec<String>, String> {
        Ok(vec![])
    }
    fn regex(&mut self, _spec: &Json) -> Result<(), String> {
        Ok(())
    }
}

pub struct ExportFile {
    file: RefCell<std::fs::File>,
    /// (path, hash hex) -> (offset, length) of its JSON line.
    index: FxHashMap<(String, String), (u64, u32)>,
}

pub struct Real {
    /// (name, program index, plan, exported relations)
    pub plans: Vec<(String, usize, Plan<RepoSource>, Vec<ExportedRelation>)>,
    pub graphs: Vec<(String, Rc<Graph>)>,
    pub refused: Vec<Json>,
    files: Vec<ExportFile>,
}

fn index(p: &Path) -> ExportFile {
    let f = std::fs::File::open(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
    let mut r = std::io::BufReader::new(f);
    let mut index = FxHashMap::default();
    let mut off = 0u64;
    let mut line = String::new();
    loop {
        line.clear();
        let n = r.read_line(&mut line).unwrap();
        if n == 0 {
            break;
        }
        // The line starts with {"exports":…, so read the keys from the end: …"hash":"…","path":"…"}
        let j: Json = serde_json::from_str(&line).unwrap();
        index.insert((j["path"].as_str().unwrap().to_owned(), j["hash"].as_str().unwrap().to_owned()), (off, n as u32));
        off += n as u64;
    }
    ExportFile { file: RefCell::new(std::fs::File::open(p).unwrap()), index }
}

fn json_val(v: &Json) -> Val {
    match v {
        Json::Null => Val::NULL,
        Json::Number(n) => match n.as_i64() {
            Some(i) => Val::int(i),
            None => Val::real(n.as_f64().unwrap().to_bits()),
        },
        Json::String(s) => Val::text(val::sym(s)),
        Json::Bool(b) => Val::int(*b as i64),
        other => Val::text(val::sym(&other.to_string())),
    }
}

/// `spec`: `[{"rule": FILE, "config": {..}, "clauses": [..], "exports": JSONL}]`.
pub fn load(spec: &Path) -> Real {
    let items: Vec<Json> = serde_json::from_str(&std::fs::read_to_string(spec).unwrap()).unwrap();
    let mut out = Real { plans: Vec::new(), graphs: Vec::new(), refused: Vec::new(), files: Vec::new() };
    for (k, it) in items.iter().enumerate() {
        let doc: Json = serde_json::from_str(&std::fs::read_to_string(it["rule"].as_str().unwrap()).unwrap()).unwrap();
        let config: Map<String, Json> = it.get("config").and_then(Json::as_object).cloned().unwrap_or_default();
        let clauses: Vec<String> = it["clauses"].as_array().unwrap().iter().map(|c| c.as_str().unwrap().to_owned()).collect();
        let c = rerail_facts::compile::compile_report(&doc, &config, &mut Accept).unwrap();
        for r in &c.refusals {
            if clauses.contains(&r.clause) {
                out.refused.push(serde_json::json!({"clause": r.clause, "at": r.at, "construct": r.construct}));
            }
        }
        for p in &c.program.repository {
            if !clauses.contains(&p.clause) {
                continue;
            }
            match norm::plan(p, &c.program.exported) {
                Ok(mut g) => {
                    for op in g.nodes.iter_mut() {
                        if let Op::Exported(e) = op {
                            *e += (k as u32 + 1) * STRIDE;
                        }
                    }
                    out.graphs.push((p.clause.clone(), Rc::new(g)));
                    out.plans.push((p.clause.clone(), k, p.clone(), c.program.exported.clone()));
                }
                Err(rs) => {
                    for r in rs {
                        out.refused.push(serde_json::json!({"clause": p.clause, "at": r.at, "construct": r.construct, "by": "generator"}));
                    }
                }
            }
        }
        out.files.push(index(&PathBuf::from(it["exports"].as_str().unwrap())));
    }
    out
}

impl Real {
    /// Rows of global export `e` for (path, content hash): read from its producer's output.
    /// A content the producer failed on is an error (its clause fails; never an empty guess).
    pub fn rows(&self, e: u32, path: &str, hash: &str) -> Result<Vec<Row>, String> {
        let f = &self.files[(e / STRIDE - 1) as usize];
        let Some(&(off, len)) = f.index.get(&(path.to_owned(), hash.to_owned())) else {
            return Err(format!("no exports for {path} {hash}"));
        };
        let mut buf = vec![0u8; len as usize];
        {
            let mut file = f.file.borrow_mut();
            file.seek(SeekFrom::Start(off)).unwrap();
            file.read_exact(&mut buf).unwrap();
        }
        let j: Json = serde_json::from_slice(&buf).unwrap();
        if let Some(fail) = j["failure"].as_str() {
            return Err(format!("{path}: per-file producer failed: {fail}"));
        }
        Ok(match j["exports"].get((e % STRIDE).to_string()) {
            None => Vec::new(),
            Some(rows) => rows.as_array().unwrap().iter().map(|r| r.as_array().unwrap().iter().map(json_val).collect()).collect(),
        })
    }
}
