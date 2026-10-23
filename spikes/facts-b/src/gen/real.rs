//! Real repository plans: rules compiled by `rerail_facts::compile` (S1), their repository plans
//! normalised by the generator, their exported relations read per (path, content) from the
//! per-file producer's output (`facts-a xexport` JSON lines), on demand.
//!
//! Cross-plan sharing (R37, REWRITES.md): every exported relation gets one graph-wide id per
//! per-content computation (`export_digest`: the producer's per-file plan closure the relation
//! reads, its inputs with the utilities they reference, the rule's scope), and all plans are
//! normalised through one hash-consing context (`norm::Plans`), so a stage equal in two plans is
//! one node. `FACTS_SHARE=0`: one graph per plan, one id per (program, export) (the differential
//! test's other side).
use super::norm::{self, Graph};
use super::val::{self, Row, Val};
use rerail_facts::front::rule::LeafCheck;
use rerail_facts::ir::{self, ExportedRelation, FileSource, Plan, Program, Rel, RepoSource, Sink, StageDef};
use rustc_hash::{FxHashMap, FxHashSet};
use serde_json::{Map, Value as Json};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::io::{BufRead, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// Graph-wide export ids start here (below: the test vectors' exports).
pub const STRIDE: u32 = 1000;

/// The leaf check is the per-file executor's; the repository side only needs the plans.
pub(super) struct Accept;
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
    /// The (path, hash) the producer failed on.
    failed: FxHashSet<(String, String)>,
    /// The last line read, parsed (a content's exports are read one after the other).
    last: RefCell<Option<((u64, u32), Rc<Json>)>>,
}

pub struct Real {
    /// (name, program index, plan, exported relations)
    pub plans: Vec<(String, usize, Plan<RepoSource>, Vec<ExportedRelation>)>,
    /// One entry per plan; with sharing on, every entry holds the same graph.
    pub graphs: Vec<(String, Rc<Graph>)>,
    pub refused: Vec<Json>,
    files: Vec<ExportFile>,
    /// Graph-wide id − STRIDE → its producers (file index, program-local id); the first is read.
    exports: Vec<Vec<(usize, u32)>>,
    /// Per program: local export id → graph-wide id.
    ids: Vec<FxHashMap<u32, u32>>,
    /// The sharing report: relations shared, and digests not shared because their producers'
    /// outputs disagree on which contents failed (the side condition checked on the data).
    pub sharing: Json,
}

pub(super) fn index(p: &Path) -> ExportFile {
    let f = std::fs::File::open(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
    let mut r = std::io::BufReader::new(f);
    let mut index = FxHashMap::default();
    let mut failed = FxHashSet::default();
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
        let key = (j["path"].as_str().unwrap().to_owned(), j["hash"].as_str().unwrap().to_owned());
        if j.get("failure").is_some_and(|f| !f.is_null()) {
            failed.insert(key.clone());
        }
        index.insert(key, (off, n as u32));
        off += n as u64;
    }
    ExportFile { file: RefCell::new(std::fs::File::open(p).unwrap()), index, failed, last: RefCell::new(None) }
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

/// Utility ids a pattern query references (`matches`), recursively through the utilities.
fn util_refs(v: &Json, utils: &Map<String, Json>, out: &mut BTreeSet<String>) {
    match v {
        Json::Object(o) => {
            for (k, x) in o {
                if k == "matches" {
                    if let Some(id) = x.as_str() {
                        if out.insert(id.to_owned()) {
                            if let Some(u) = utils.get(id) {
                                util_refs(u, utils, out);
                            }
                        }
                    }
                }
                util_refs(x, utils, out);
            }
        }
        Json::Array(a) => a.iter().for_each(|x| util_refs(x, utils, out)),
        _ => {}
    }
}

/// The per-content computation of exported relation `e` as text: equal texts compute equal rows
/// for every (path, content) (xexport evaluates exactly these parts). The export's definition,
/// the producer stages it reads (transitively; renumbered by first read, names dropped: stages
/// are read by index), its inputs (renumbered, names dropped) with the rule utilities they
/// reference and the options those mention, the producer's syntax spec, the rule's scope and
/// `define.language`. Whether a content fails is a property of the whole producer output, not of
/// this text: it is checked on the data (`load`).
pub fn export_digest(doc: &Json, config: &Map<String, Json>, prog: &Program, e: &ExportedRelation) -> String {
    let plan = &prog.per_file[e.producer];
    let Sink::Export(list) = &plan.sink else { return format!("unshared: producer {} has no export sink", e.producer) };
    let Some((_, rel)) = list.iter().find(|(i, _)| *i == e.id) else { return format!("unshared: export {} not in its producer", e.id) };
    let (mut stages, mut inputs): (Vec<u32>, Vec<u32>) = (Vec::new(), Vec::new());
    let pos = |v: &mut Vec<u32>, i: u32| -> u32 {
        match v.iter().position(|x| *x == i) {
            Some(p) => p as u32,
            None => {
                v.push(i);
                v.len() as u32 - 1
            }
        }
    };
    let mut canon = |r: &Rel<FileSource>, stages: &mut Vec<u32>, inputs: &mut Vec<u32>| -> Rel<FileSource> {
        ir::map_rel(r.clone(), &mut |s| {
            Ok::<_, ()>(match s {
                FileSource::Stage(i) => FileSource::Stage(pos(stages, i)),
                FileSource::Input(i) => FileSource::Input(pos(inputs, i)),
                o => o,
            })
        })
        .unwrap()
    };
    let mut text = String::new();
    writeln!(text, "export {:?} {:?}", e.schema, canon(rel, &mut stages, &mut inputs)).unwrap();
    let mut k = 0;
    while k < stages.len() {
        let st = &plan.stages[stages[k] as usize];
        let def = match &st.def {
            StageDef::Query { rel } => format!("query {:?}", canon(rel, &mut stages, &mut inputs)),
            StageDef::Reach { roots, edges, graph } => {
                let r = canon(roots, &mut stages, &mut inputs);
                let e = edges.as_ref().map(|x| canon(x, &mut stages, &mut inputs));
                format!("reach {r:?} {e:?} {graph:?}")
            }
        };
        writeln!(text, "stage {k} {:?} {def}", st.schema).unwrap();
        k += 1;
    }
    // The rule's utilities (every `define`), and the options with their resolved values.
    let mut utils = Map::new();
    let mut options = Map::new();
    let mut language = Vec::new();
    for c in doc["clauses"].as_array().map(Vec::as_slice).unwrap_or_default() {
        if let Some(d) = c.get("define") {
            if let Some(u) = d.get("utils").and_then(Json::as_object) {
                utils.extend(u.clone());
            }
            if let Some(o) = d.get("options").and_then(Json::as_object) {
                options.extend(o.clone());
            }
            if let Some(l) = d.get("language") {
                language.push(l.clone());
            }
        }
    }
    let mut used = BTreeSet::new();
    // Mutant 52: the inputs and the rule's scope are left out of the digest.
    let blind = crate::mutant::on(52);
    for (k, i) in inputs.iter().enumerate() {
        let mut d = prog.inputs[*i as usize].clone();
        d.name = String::new();
        if let ir::InputKind::Pattern { query } = &d.kind {
            util_refs(query, &utils, &mut used);
        }
        if !blind {
            writeln!(text, "input {k} {d:?}").unwrap();
        }
    }
    for id in &used {
        let u = utils.get(id).cloned().unwrap_or(Json::Null);
        let us = u.to_string();
        writeln!(text, "util {id} {us}").unwrap();
        for (name, spec) in &options {
            if us.contains(name.as_str()) {
                writeln!(text, "option {name} {spec} {}", config.get(name).cloned().unwrap_or(Json::Null)).unwrap();
            }
        }
    }
    writeln!(text, "syntax {:?}", plan.syntax).unwrap();
    if !blind {
        writeln!(text, "scope {}", doc.get("scope").cloned().unwrap_or(Json::Null)).unwrap();
    }
    writeln!(text, "language {}", Json::Array(language)).unwrap();
    text
}

/// The side condition of sharing an exported relation that `export_digest` cannot see: its
/// producers' outputs index the same contents and fail on the same ones (a content fails for its
/// whole producer output, whichever relation is read).
pub(super) fn producers_agree(files: &[&ExportFile]) -> bool {
    // Mutant 54: the failures are not compared.
    let f0 = files[0];
    files.iter().all(|f| (crate::mutant::on(54) || f.failed == f0.failed) && f.index.len() == f0.index.len() && f.index.keys().all(|x| f0.index.contains_key(x)))
}

/// `spec`: `[{"rule": FILE, "config": {..}, "clauses": [..], "exports": JSONL}]`.
pub fn load(spec: &Path) -> Real {
    let share = std::env::var("FACTS_SHARE").map_or(true, |v| v != "0");
    let items: Vec<Json> = serde_json::from_str(&std::fs::read_to_string(spec).unwrap()).unwrap();
    let mut out = Real { plans: Vec::new(), graphs: Vec::new(), refused: Vec::new(), files: Vec::new(), exports: Vec::new(), ids: Vec::new(), sharing: Json::Null };
    let mut programs = Vec::new();
    for it in &items {
        let doc: Json = serde_json::from_str(&std::fs::read_to_string(it["rule"].as_str().unwrap()).unwrap()).unwrap();
        let config: Map<String, Json> = it.get("config").and_then(Json::as_object).cloned().unwrap_or_default();
        let clauses: Vec<String> = it["clauses"].as_array().unwrap().iter().map(|c| c.as_str().unwrap().to_owned()).collect();
        let c = rerail_facts::compile::compile_report(&doc, &config, &mut Accept).unwrap();
        out.files.push(index(&PathBuf::from(it["exports"].as_str().unwrap())));
        programs.push((doc, config, clauses, c));
    }
    // Graph-wide export ids: one per digest whose producers agree on the failed contents.
    let mut groups: BTreeMap<String, Vec<(usize, u32)>> = BTreeMap::new();
    for (k, (doc, config, _, c)) in programs.iter().enumerate() {
        for e in &c.program.exported {
            let d = if share { export_digest(doc, config, &c.program, e) } else { format!("program {k} export {}", e.id) };
            groups.entry(d).or_default().push((k, e.id));
        }
    }
    out.ids = vec![FxHashMap::default(); programs.len()];
    let (mut shared, mut split) = (Vec::new(), Vec::new());
    for (d, members) in groups {
        // Side condition on the data (`producers_agree`); otherwise each member keeps its own id.
        let agree = producers_agree(&members.iter().map(|(k, _)| &out.files[*k]).collect::<Vec<_>>());
        let names = |ms: &[(usize, u32)]| -> Vec<String> { ms.iter().map(|(k, e)| programs[*k].3.program.exported.iter().find(|x| x.id == *e).map_or(format!("{k}:{e}"), |x| format!("{}:{}", programs[*k].0["id"].as_str().unwrap_or("?"), x.name))).collect() };
        let parts: Vec<Vec<(usize, u32)>> = if agree { vec![members.clone()] } else { members.iter().map(|m| vec![*m]).collect() };
        if members.len() > 1 {
            if parts.len() == 1 {
                shared.push(Json::from(names(&members)));
            } else {
                let _ = &d;
                split.push(Json::from(names(&members)));
            }
        }
        for part in parts {
            let id = STRIDE + out.exports.len() as u32;
            for (k, e) in &part {
                out.ids[*k].insert(*e, id);
            }
            out.exports.push(part);
        }
    }
    out.sharing = serde_json::json!({"on": share, "exports": out.exports.len(), "shared": shared, "not_shared_failures_differ": split});
    // Plans, normalised through one context (sharing on) or one each.
    let mut all = norm::Plans::default();
    let mut names = Vec::new();
    for (k, (_, _, clauses, c)) in programs.iter().enumerate() {
        for r in &c.refusals {
            if clauses.contains(&r.clause) {
                out.refused.push(serde_json::json!({"clause": r.clause, "at": r.at, "construct": r.construct}));
            }
        }
        for p in &c.program.repository {
            if !clauses.contains(&p.clause) {
                continue;
            }
            let ids = &out.ids[k];
            let map = |e: u32| ids[&e];
            let mut own = norm::Plans::default();
            match (if share { &mut all } else { &mut own }).add(&p.clause, p, &c.program.exported, &map) {
                Ok(()) => {
                    if !share {
                        out.graphs.push((p.clause.clone(), Rc::new(own.graph())));
                    }
                    names.push(p.clause.clone());
                    out.plans.push((p.clause.clone(), k, p.clone(), c.program.exported.clone()));
                }
                Err(rs) => {
                    for r in rs {
                        out.refused.push(serde_json::json!({"clause": p.clause, "at": r.at, "construct": r.construct, "by": "generator"}));
                    }
                }
            }
        }
    }
    if share {
        let g = Rc::new(all.graph());
        out.graphs = names.into_iter().map(|n| (n, g.clone())).collect();
    }
    out
}

/// `facts-b gnorm`: compile and normalise the spec's repository plans without reading their
/// exports; print every refusal and the node count of each plan that normalises; with `--dump X`,
/// the program's EXPLAIN text (`X` = `all`) or its lines from the first containing `X` on.
pub fn check(spec: &Path, dump: Option<&str>) {
    let items: Vec<Json> = serde_json::from_str(&std::fs::read_to_string(spec).unwrap()).unwrap();
    // Constants are evaluated while normalising; their text values need a string table.
    val::install(Rc::new(RefCell::new(crate::derive::World::new(crate::store::Store::default()))));
    for it in &items {
        let doc: Json = serde_json::from_str(&std::fs::read_to_string(it["rule"].as_str().unwrap()).unwrap()).unwrap();
        let config: Map<String, Json> = it.get("config").and_then(Json::as_object).cloned().unwrap_or_default();
        let clauses: Vec<String> = it["clauses"].as_array().unwrap().iter().map(|c| c.as_str().unwrap().to_owned()).collect();
        let c = rerail_facts::compile::compile_report(&doc, &config, &mut Accept).unwrap();
        for r in c.refusals.iter().filter(|r| clauses.contains(&r.clause)) {
            println!("refused (compiler) {} {}: {}", r.clause, r.at, r.construct);
        }
        if let Some(d) = dump {
            let text = rerail_facts::explain::explain(&c.program);
            let from = if d == "all" { 0 } else { text.find(d).map(|i| text[..i].rfind('\n').map_or(0, |j| j + 1)).unwrap_or(text.len()) };
            println!("{}", &text[from..]);
        }
        for p in c.program.repository.iter().filter(|p| clauses.contains(&p.clause)) {
            match norm::plan(p, &c.program.exported) {
                Ok(g) => println!("ok {}: {} nodes", p.clause, g.nodes.len()),
                Err(rs) => rs.iter().for_each(|r| println!("refused (generator) {} {}: {}", p.clause, r.at, r.construct)),
            }
        }
    }
    // `FACTS_SHARING=1`: the spec's plans loaded as greplay loads them (exports indexed): the
    // exported relations shared across programs and the nodes of the merged graph.
    if std::env::var_os("FACTS_SHARING").is_some() {
        let r = load(spec);
        let mut graphs: Vec<&Rc<Graph>> = r.graphs.iter().map(|(_, g)| g).collect();
        graphs.dedup_by(|a, b| Rc::ptr_eq(a, b));
        println!("{}", serde_json::to_string_pretty(&r.sharing).unwrap());
        println!("graphs {}, nodes {}", graphs.len(), graphs.iter().map(|g| g.nodes.len()).sum::<usize>());
    }
}

impl Real {
    /// The graph-wide id of program `k`'s export `e`.
    pub fn global(&self, k: usize, e: u32) -> u32 {
        self.ids[k][&e]
    }

    /// Rows of graph-wide export `e` for (path, content hash): read from its (first) producer's
    /// output. A content the producer failed on is an error (its clause fails; never an empty
    /// guess).
    pub fn rows(&self, e: u32, path: &str, hash: &str) -> Result<Vec<Row>, String> {
        let (fi, local) = self.exports[(e - STRIDE) as usize][0];
        let f = &self.files[fi];
        let Some(&at) = f.index.get(&(path.to_owned(), hash.to_owned())) else {
            return Err(format!("no exports for {path} {hash}"));
        };
        let cached = f.last.borrow().as_ref().filter(|(k, _)| *k == at).map(|(_, j)| j.clone());
        let j = match cached {
            Some(j) => j,
            None => {
                let mut buf = vec![0u8; at.1 as usize];
                {
                    let mut file = f.file.borrow_mut();
                    file.seek(SeekFrom::Start(at.0)).unwrap();
                    file.read_exact(&mut buf).unwrap();
                }
                let j: Rc<Json> = Rc::new(serde_json::from_slice(&buf).unwrap());
                *f.last.borrow_mut() = Some((at, j.clone()));
                j
            }
        };
        if let Some(fail) = j["failure"].as_str() {
            return Err(format!("{path}: per-file producer failed: {fail}"));
        }
        Ok(match j["exports"].get(local.to_string()) {
            None => Vec::new(),
            Some(rows) => rows.as_array().unwrap().iter().map(|r| r.as_array().unwrap().iter().map(json_val).collect()).collect(),
        })
    }

    /// Spec items (programs) with an exports file.
    pub fn programs(&self) -> usize {
        self.files.len()
    }

    /// Every export of program `k` for (path, content hash), its JSON line parsed once: (graph-wide
    /// export id, rows), for the exports a plan reads whose first producer is this program's (a
    /// shared relation is read from one producer, as `rows` reads it). A content the producer
    /// failed on, or without a line, is an error.
    pub fn program_rows(&self, k: usize, path: &str, hash: &str) -> Result<Vec<(u32, Vec<Row>)>, String> {
        let f = &self.files[k];
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
        let Some(m) = j["exports"].as_object() else { return Ok(Vec::new()) };
        Ok(m.iter()
            .filter_map(|(id, rows)| {
                let g = *self.ids[k].get(&id.parse::<u32>().unwrap())?;
                (self.exports[(g - STRIDE) as usize][0].0 == k).then(|| (g, rows.as_array().unwrap().iter().map(|r| r.as_array().unwrap().iter().map(json_val).collect()).collect()))
            })
            .collect())
    }
}
