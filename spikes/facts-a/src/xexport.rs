//! `facts-a xexport` (S3, interim until the per-file backend executes IR plans): the exported
//! relations of a rule's repository clauses, per (path, content). The rule is compiled by
//! `rerail_facts::compile`; each export's producer (`Program.per_file[producer]`, sink
//! `Sink::Export`) runs over one content: its pattern inputs through this crate's leaf executor
//! (`eval::Input::rows` over facts), `file(path)` and native's call summaries (`cgn_*`, SCHEMA
//! § 3.4, through `cgnative`) as fact relations, its stages and export relations through the
//! reference evaluator (`rerail_facts::reference`). Output: one JSON line per job,
//! `{"path", "hash", "exports": {"<id>": [[v, …], …]}, "failure"}`.
use crate::cgnative::{self, Variant};
use crate::eval;
use crate::facts;
use rerail_facts::ir::{self, FileSource, InputKind, Plan, Projection, Sink};
use rerail_facts::reference;
use rerail_facts::sql::value::V;
use serde_json::{Map, Value as Json, json};
use std::collections::BTreeMap;
use std::io::Write;
use std::sync::Arc;

fn projection_json(p: &Projection) -> Result<Json, String> {
    Ok(match p {
        Projection::Path => json!("file.path"),
        Projection::Directory => json!("file.directory"),
        Projection::Name => json!("file.name"),
        Projection::Stem => json!("file.stem"),
        Projection::Capture(n) => json!(format!("capture.{n}")),
        Projection::Text(n) => json!(format!("capture.{n}.text")),
        Projection::Kind(n) => json!(format!("capture.{n}.kind")),
        Projection::StartByte(n) => json!(format!("capture.{n}.startByte")),
        Projection::EndByte(n) => json!(format!("capture.{n}.endByte")),
        Projection::StartLine(n) => json!(format!("capture.{n}.startLine")),
        Projection::Regex { source, regex, group } => json!({"source": projection_json(source)?, "regex": regex, "group": group}),
        other => return Err(format!("projection {other:?} has no leaf executor in facts-a")),
    })
}

fn anchor_json(a: &ir::Anchor) -> Json {
    match a {
        ir::Anchor::Span => json!("witness.span"),
        ir::Anchor::Start => json!("witness.start"),
        ir::Anchor::End => json!("witness.end"),
        ir::Anchor::FileStart => json!("file.start"),
        ir::Anchor::Capture(n) => json!(format!("capture.{n}")),
    }
}

fn v_json(v: &V) -> Json {
    match v {
        V::Null => Json::Null,
        V::Int(i) => json!(i),
        V::Real(f) => json!(f),
        V::Text(s) => json!(&**s),
    }
}

/// One worker's compiled state: the program, the leaf inputs it needs, Spike A's inputs for them.
struct Exporter {
    program: ir::Program,
    /// (export id, producer plan index, relation)
    exports: Vec<(u32, usize)>,
    rule: eval::Rule,
    /// IR input id -> (Spike A input index, schema)
    inputs: BTreeMap<u32, (usize, crate::sql::engine::Schema)>,
}

fn compile(doc: &Json, config: &Map<String, Json>, clauses: &[String]) -> Result<Exporter, String> {
    let mut leaves = crate::leaf::Leaves::new();
    let c = rerail_facts::compile::compile_report(doc, config, &mut leaves)?;
    let program = c.program;
    let mut exports = Vec::new();
    for rp in &program.repository {
        if !clauses.is_empty() && !clauses.contains(&rp.clause) {
            continue;
        }
        let mut ids = std::collections::BTreeSet::new();
        reads(rp, &mut ids);
        for id in ids {
            let e = program.exported.iter().find(|e| e.id == id).ok_or(format!("export {id}"))?;
            exports.push((id, e.producer));
        }
    }
    // The pattern inputs the producers scan, as one synthetic relational clause for Spike A.
    let mut needed = std::collections::BTreeSet::new();
    for (_, p) in &exports {
        file_inputs(&program.per_file[*p], &mut needed);
    }
    let mut evidence = Map::new();
    for id in &needed {
        let d = &program.inputs[*id as usize];
        let InputKind::Pattern { query } = &d.kind else { return Err(format!("input {} is a regex clause", d.name)) };
        let mut project = Map::new();
        for (f, p) in &d.fields {
            project.insert(f.clone(), projection_json(p)?);
        }
        let mut ev = json!({"as": "row", "query": {"ast": query}, "project": project, "anchor": anchor_json(&d.anchor)});
        if let Some(e) = &d.each {
            ev["each"] = json!(e);
        }
        if let Some(p) = &d.paths {
            ev["paths"] = p.clone();
        }
        evidence.insert(format!("i{id:06}"), json!({"evidence": ev}));
    }
    let first = needed.iter().next().map(|i| format!("i{i:06}")).unwrap_or_default();
    let mut clauses_json: Vec<Json> = doc["clauses"].as_array().ok_or("clauses")?.iter().filter(|c| c.get("define").is_some()).cloned().collect();
    if !needed.is_empty() {
        clauses_json.push(json!({"id": "xexport", "relational": {"inputs": evidence, "query": format!("SELECT __path AS path, __start AS start_byte, __end AS end_byte FROM {first}")}, "violation": {"code": "x", "anchor": "witness.span"}}));
    }
    let mut synth = doc.clone();
    synth["clauses"] = Json::Array(clauses_json);
    let rule = eval::Rule::compile(&synth, config, None, &facts::GRAMMAR).map_err(|e| format!("leaf inputs: {e}"))?;
    let mut inputs = BTreeMap::new();
    if let Some(cl) = rule.clauses.iter().find(|c| c.id == "xexport") {
        let eval::Kind::Relational(plan) = &cl.kind else { unreachable!() };
        for (k, (name, idx)) in plan.inputs.iter().enumerate() {
            let id: u32 = name[1..].parse().unwrap();
            inputs.insert(id, (*idx, plan.input_schemas[k].clone()));
        }
    }
    Ok(Exporter { program, exports, rule, inputs })
}

fn reads(p: &Plan<ir::RepoSource>, out: &mut std::collections::BTreeSet<u32>) {
    let s = format!("{p:?}");
    // Exported(id) occurrences: the plan's leaves.
    let mut rest = s.as_str();
    while let Some(i) = rest.find("Exported(") {
        let tail = &rest[i + 9..];
        let end = tail.find(')').unwrap();
        if let Ok(id) = tail[..end].parse() {
            out.insert(id);
        }
        rest = &tail[end..];
    }
}

fn file_inputs(p: &Plan<FileSource>, out: &mut std::collections::BTreeSet<u32>) {
    let s = format!("{p:?}");
    let mut rest = s.as_str();
    while let Some(i) = rest.find("Input(") {
        let tail = &rest[i + 6..];
        let end = tail.find(')').unwrap();
        if let Ok(id) = tail[..end].parse() {
            out.insert(id);
        }
        rest = &tail[end..];
    }
}

impl Exporter {
    fn run(&self, path: &str, source: Arc<String>) -> Result<BTreeMap<u32, Vec<Vec<V>>>, String> {
        let mut out = BTreeMap::new();
        // Outside the rule's scope a content has no inputs and no summary; it is still a member
        // (`file(path)`), which views over the path set read (module paths, package markers).
        let in_scope = self.rule.scope.matches(path);
        let f = if in_scope {
            match facts::build(path, source.clone())? {
                facts::Built::Facts(f) => Some(f),
                facts::Built::Unparsed => return Err("unparsed".into()),
            }
        } else {
            None
        };
        // Native's summaries of this content under each variant, per links, in the path's reading.
        let summaries: std::cell::RefCell<BTreeMap<Box<str>, Vec<(Variant, cgnative::Rendered)>>> = Default::default();
        let summary_rows = |rel: ir::SummaryRel, links: &Box<str>| -> Result<Vec<Vec<V>>, String> {
            let Some(f) = &f else { return Ok(Vec::new()) };
            let mut cache = summaries.borrow_mut();
            if !cache.contains_key(links) {
                let key = cgnative::links_key(links)?;
                let tape = f.tape.as_ref().ok_or("no tape")?;
                let reading = if path.ends_with(".pyi") { "x.pyi" } else { "x.py" };
                let all = Variant::ALL.iter().map(|v| (*v, cgnative::compacted(&cgnative::render(&cgnative::summary(tape, reading, &v.module(), f.syntax_errors, source.len()).as_references(key))))).collect();
                cache.insert(links.clone(), all);
            }
            let mut out = Vec::new();
            for (v, r) in &cache[links] {
                for row in r.rels.get(rel.name()).map(Vec::as_slice).unwrap_or_default() {
                    let mut x = vec![V::text(path), V::text(v.name())];
                    x.extend(row.iter().map(|c| V::text(c)));
                    out.push(x);
                }
            }
            Ok(out)
        };
        let text: std::rc::Rc<str> = std::rc::Rc::from(source.as_str());
        let mut rows: BTreeMap<u32, Vec<Vec<V>>> = BTreeMap::new();
        let memo = crate::ast::FileMemo::default();
        for (id, (idx, schema)) in &self.inputs {
            let Some(f) = &f else {
                rows.insert(*id, Vec::new());
                continue;
            };
            let ctx = eval::FileCtx { f, path, source: &source };
            let r = self.rule.inputs[*idx].rows(&ctx, schema, &memo, &text);
            if let Some(u) = r.unsupported {
                return Err(format!("input {id}: {u}"));
            }
            rows.insert(*id, (0..r.table.len()).map(|i| r.table.row(i)).collect());
        }
        for (eid, p) in &self.exports {
            let plan = &self.program.per_file[*p];
            let Sink::Export(list) = &plan.sink else { return Err("producer without an Export sink".into()) };
            let (_, rel) = list.iter().find(|(i, _)| i == eid).ok_or("export not in its producer")?;
            if plan.syntax.is_some() {
                return Err("per-file producer with a syntax spec (not wired in xexport)".into());
            }
            let one = Plan { clause: plan.clause.clone(), syntax: None, stages: plan.stages.clone(), sink: Sink::Findings { rel: rel.clone(), key: vec![], code: None, locations: vec![], violation: ir::Violation { code: String::new(), anchor: ir::Anchor::Span, each: None } } };
            let r = reference::evaluate_plan(
                &one,
                |s| match s {
                    FileSource::Stage(i) => Some(*i),
                    _ => None,
                },
                &|s| match s {
                    FileSource::Input(i) => rows.get(i).cloned().ok_or_else(|| format!("input {i} not computed")),
                    FileSource::Core(ir::CoreRel::File) => Ok(vec![vec![V::text(path)]]),
                    FileSource::Summary { rel, links } => summary_rows(*rel, links),
                    other => Err(format!("{other:?} not wired in xexport")),
                },
            )?;
            out.insert(*eid, r);
        }
        Ok(out)
    }
}

/// `xexport --rule FILE [--config JSON] [--clauses a,b] --jobs TSV --out FILE [--threads N]`;
/// jobs: `path \t hash \t source file` per line.
pub fn main(args: &[String]) {
    let arg = |n: &str| args.iter().position(|a| a == n).and_then(|i| args.get(i + 1)).cloned();
    let doc: Json = serde_json::from_str(&std::fs::read_to_string(arg("--rule").expect("--rule")).unwrap()).unwrap();
    let config: Map<String, Json> = arg("--config").map(|c| serde_json::from_str(&c).unwrap()).unwrap_or_default();
    let clauses: Vec<String> = arg("--clauses").map(|c| c.split(',').map(str::to_owned).collect()).unwrap_or_default();
    let jobs: Vec<(String, String, String)> = std::fs::read_to_string(arg("--jobs").expect("--jobs"))
        .unwrap()
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| {
            let mut it = l.split('\t');
            (it.next().unwrap().to_owned(), it.next().unwrap().to_owned(), it.next().unwrap().to_owned())
        })
        .collect();
    let threads: usize = arg("--threads").map(|t| t.parse().unwrap()).unwrap_or(4);
    let out_path = arg("--out").expect("--out");
    let summary = {
        let e = compile(&doc, &config, &clauses).unwrap_or_else(|e| panic!("compile: {e}"));
        json!({"exports": e.exports.iter().map(|(id, p)| json!({"id": id, "producer": p, "name": e.program.exported.iter().find(|x| x.id == *id).map(|x| x.name.clone())})).collect::<Vec<_>>(), "inputs": e.inputs.len(), "repository": e.program.repository.iter().map(|p| p.clause.clone()).collect::<Vec<_>>()})
    };
    eprintln!("xexport: {summary}");
    let next = std::sync::atomic::AtomicUsize::new(0);
    let lines: parking_lot::Mutex<Vec<(usize, String)>> = parking_lot::Mutex::new(Vec::new());
    let t0 = std::time::Instant::now();
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| {
                let e = compile(&doc, &config, &clauses).unwrap();
                loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some((path, hash, file)) = jobs.get(i) else { break };
                    let res = (|| -> Result<BTreeMap<u32, Vec<Vec<V>>>, String> {
                        let bytes = std::fs::read(file).map_err(|e| format!("{file}: {e}"))?;
                        let h = blake3::hash(&bytes).to_hex().to_string();
                        if &h != hash {
                            return Err(format!("{file}: content hash {h} != {hash}"));
                        }
                        let src = String::from_utf8(bytes).map_err(|_| "not utf-8".to_string())?;
                        e.run(path, Arc::new(src))
                    })();
                    let line = match res {
                        Ok(m) => json!({"path": path, "hash": hash, "exports": m.iter().map(|(k, rows)| (k.to_string(), Json::Array(rows.iter().map(|r| Json::Array(r.iter().map(v_json).collect())).collect()))).collect::<Map<_, _>>(), "failure": Json::Null}),
                        Err(f) => json!({"path": path, "hash": hash, "exports": {}, "failure": f}),
                    };
                    lines.lock().push((i, line.to_string()));
                }
            });
        }
    });
    let mut lines = lines.into_inner();
    lines.sort();
    let mut w = std::io::BufWriter::new(std::fs::File::create(&out_path).unwrap());
    let mut failures = 0;
    for (_, l) in &lines {
        if !l.contains("\"failure\":null") {
            failures += 1;
        }
        writeln!(w, "{l}").unwrap();
    }
    w.flush().unwrap();
    eprintln!("xexport: {} jobs, {} failures, {:.1}s", lines.len(), failures, t0.elapsed().as_secs_f64());
    let mut sum = summary;
    sum["jobs"] = json!(lines.len());
    sum["failures"] = json!(failures);
    sum["seconds"] = json!(t0.elapsed().as_secs_f64());
    std::fs::write(format!("{out_path}.summary.json"), serde_json::to_string_pretty(&sum).unwrap()).unwrap();
}
