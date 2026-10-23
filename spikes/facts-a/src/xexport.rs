//! `facts-a xexport` (S3): the exported relations of a rule's repository clauses, per (path,
//! content). The rule is compiled by `rerail_facts::compile`; each export's producer
//! (`Program.per_file[producer]`, sink `Sink::Export`) runs over one content. Its stages run on
//! this crate's engine, the path `repo` uses (`--stages engine`, the default): the clause's
//! relational plan (`rel::run`) over the content's inputs, its stored stage tables read by name
//! (`rel::TRACE`); a producer stage the engine did not store (it failed before it) fails the
//! content. With `--stages reference` the stages run on the reference evaluator instead (clauses
//! the engine cannot plan, e.g. over `reference_edges`). The export relations themselves (cut
//! subtrees over the stages) and `file(path)`/native call summaries (`cgn_*`, SCHEMA § 3.4,
//! through `cgnative`) go through the reference evaluator (`rerail_facts::reference`). Pattern
//! inputs come from the leaf executor (`eval::Input::rows` over facts). `--check-below BYTES`
//! (default 4096): on contents up to that size the reference also evaluates the stages, and an
//! export that differs fails the content (the reference stays the checker). Every relation the
//! exporter hands on (leaf inputs, engine stages, summaries, exports) is built from its IR schema
//! (`InputDef.schema`, `Stage.schema`, `ExportedRelation.schema`), columns matched by name, and
//! checked: a row whose width or values do not fit the schema's columns and affinities fails the
//! content. Output: one JSON line per job, `{"path", "hash", "exports": {"<id>": [[v, …], …]},
//! "failure"}`.
use crate::cgnative::{self, Variant};
use crate::eval;
use crate::facts;
use rerail_facts::ir::{self, FileSource, InputKind, Plan, Projection, Sink};
use rerail_facts::reference;
use rerail_facts::sql::value::V;
use serde_json::{Map, Value as Json, json};
use std::collections::BTreeMap;
use std::io::Write;
use std::rc::Rc;
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

/// The positions in `have` (a producer's column names) of `want`'s columns, matched by name (the
/// k-th column of a name to the k-th of that name), never by position. A column the producer
/// lacks is an error.
fn column_map(rel: &str, want: &[ir::Column], have: &[String]) -> Result<Vec<usize>, String> {
    let mut used = vec![false; have.len()];
    want.iter()
        .map(|c| {
            let i = (0..have.len()).find(|&i| !used[i] && have[i].eq_ignore_ascii_case(&c.name)).ok_or_else(|| format!("{rel}: column {} not produced (producer columns {have:?})", c.name))?;
            used[i] = true;
            Ok(i)
        })
        .collect()
}

/// A table's rows over the columns `map` selects, in the IR's column order.
fn table_rows(t: &crate::sql::engine::Table, map: &[usize]) -> Vec<Vec<V>> {
    (0..t.len()).map(|r| map.iter().map(|&c| t.value(r, c)).collect()).collect()
}

/// The same SQLite value: storage class and value (reals bitwise).
fn same(a: &V, b: &V) -> bool {
    match (a, b) {
        (V::Null, V::Null) => true,
        (V::Int(x), V::Int(y)) => x == y,
        (V::Real(x), V::Real(y)) => x.to_bits() == y.to_bits(),
        (V::Text(x), V::Text(y)) => x == y,
        _ => false,
    }
}

/// Every row of `rel` has `schema`'s width and holds each value as a column of that affinity
/// stores it (`apply_affinity` leaves it unchanged). Leaves are built that way, stages are stored
/// with their columns' affinities (CTAS), and an expression's affinity comes from those columns
/// or a `CAST`; so a row that does not conform is misbuilt and fails the content, never a row.
fn conform(rel: &str, schema: &[ir::Column], rows: &[Vec<V>]) -> Result<(), String> {
    for row in rows {
        if row.len() != schema.len() {
            return Err(format!("{rel}: row of {} values against {} columns", row.len(), schema.len()));
        }
        for (v, c) in row.iter().zip(schema) {
            if !same(&rerail_facts::sql::value::apply_affinity(v.clone(), c.aff), v) {
                return Err(format!("{rel}: column {} ({:?}) holds {v:?}", c.name, c.aff));
            }
        }
    }
    Ok(())
}

/// One worker's compiled state: the program, the leaf inputs it needs, Spike A's inputs for them.
struct Exporter {
    program: ir::Program,
    /// (export id, producer plan index, relation)
    exports: Vec<(u32, usize)>,
    rule: eval::Rule,
    /// IR input id -> (Spike A input index, Spike A's schema for it, the positions of the IR
    /// input's columns (`InputDef.schema`) in Spike A's table)
    inputs: BTreeMap<u32, (usize, crate::sql::engine::Schema, Vec<usize>)>,
    /// The rule as `repo` compiles it: its repository clauses' relational plans run the producers'
    /// stages (`--stages engine`); `None`: the reference evaluator runs them.
    engine: Option<eval::Rule>,
    /// Contents up to this many bytes are also exported through the reference and compared.
    check_below: usize,
}

fn compile(doc: &Json, config: &Map<String, Json>, clauses: &[String], engine: bool, check_below: usize) -> Result<Exporter, String> {
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
            let d = &program.inputs[id as usize];
            let map = column_map(&format!("input {}", d.name), &d.schema, &plan.input_schemas[k].cols)?;
            inputs.insert(id, (*idx, plan.input_schemas[k].clone(), map));
        }
    }
    let engine = if engine { Some(eval::Rule::compile(doc, config, None, &facts::GRAMMAR).map_err(|e| format!("engine: {e}"))?) } else { None };
    Ok(Exporter { program, exports, rule, inputs, engine, check_below })
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
            conform(rel.name(), &rel.schema(), &out)?;
            Ok(out)
        };
        let text: std::rc::Rc<str> = std::rc::Rc::from(source.as_str());
        let mut rows: BTreeMap<u32, Vec<Vec<V>>> = BTreeMap::new();
        let memo = crate::ast::FileMemo::default();
        for (id, (idx, schema, map)) in &self.inputs {
            let inp = &self.rule.inputs[*idx];
            // `evidence.paths`: outside it the input has no rows (as `eval_file` reads it).
            let (Some(f), true) = (&f, inp.paths.as_ref().is_none_or(|p| p.matches(path))) else {
                rows.insert(*id, Vec::new());
                continue;
            };
            let ctx = eval::FileCtx { f, path, source: &source };
            let r = inp.rows(&ctx, schema, &memo, &text);
            if let Some(u) = r.unsupported {
                return Err(format!("input {id}: {u}"));
            }
            let d = &self.program.inputs[*id as usize];
            let got = table_rows(&r.table, map);
            conform(&format!("input {}", d.name), &d.schema, &got)?;
            rows.insert(*id, got);
        }
        let scan = |s: &FileSource| -> Result<Vec<Vec<V>>, String> {
            match s {
                FileSource::Input(i) => rows.get(i).cloned().ok_or_else(|| format!("input {i} not computed")),
                FileSource::Core(ir::CoreRel::File) => {
                    let r = vec![vec![V::text(path)]];
                    conform("file", &ir::CoreRel::File.schema(), &r)?;
                    Ok(r)
                }
                FileSource::Summary { rel, links } => summary_rows(*rel, links),
                other => Err(format!("{other:?} not wired in xexport")),
            }
        };
        let reference_stages = |plan: &Plan<FileSource>| -> Result<Vec<Rc<Vec<Vec<V>>>>, String> {
            let (done, refusal) = reference::evaluate_stages(&plan.stages, |s| if let FileSource::Stage(i) = s { Some(*i) } else { None }, &scan);
            refusal.map_or(Ok(done), Err)
        };
        // Each producer's stages, computed once and read by all of its exports.
        let mut producers: Vec<usize> = self.exports.iter().map(|(_, p)| *p).collect();
        producers.sort();
        producers.dedup();
        let mut stages: BTreeMap<usize, Vec<Rc<Vec<Vec<V>>>>> = BTreeMap::new();
        for &p in &producers {
            let plan = &self.program.per_file[p];
            if plan.syntax.is_some() {
                return Err("per-file producer with a syntax spec (not wired in xexport)".into());
            }
            let st = match &self.engine {
                Some(e) => self.engine_stages(e, plan, f.as_ref(), path, &source, &text, &memo)?,
                None => reference_stages(plan)?,
            };
            stages.insert(p, st);
        }
        let exports_over = |stages: &BTreeMap<usize, Vec<Rc<Vec<Vec<V>>>>>| -> Result<BTreeMap<u32, Vec<Vec<V>>>, String> {
            let mut out = BTreeMap::new();
            for (eid, p) in &self.exports {
                let plan = &self.program.per_file[*p];
                let Sink::Export(list) = &plan.sink else { return Err("producer without an Export sink".into()) };
                let (_, rel) = list.iter().find(|(i, _)| i == eid).ok_or("export not in its producer")?;
                let one = Plan { clause: plan.clause.clone(), syntax: None, stages: vec![], sink: Sink::Findings { rel: rel.clone(), key: vec![], code: None, locations: vec![], violation: ir::Violation { code: String::new(), anchor: ir::Anchor::Span, each: None } } };
                let st = &stages[p];
                let r = reference::evaluate_plan(&one, |_| None, &|s| match s {
                    FileSource::Stage(i) => st.get(*i as usize).map(|r| (**r).clone()).ok_or_else(|| format!("stage {i} not computed")),
                    other => scan(other),
                })?;
                let def = self.program.exported.iter().find(|e| e.id == *eid).ok_or_else(|| format!("export {eid} without a definition"))?;
                conform(&def.name, &def.schema, &r)?;
                out.insert(*eid, r);
            }
            Ok(out)
        };
        let out = exports_over(&stages)?;
        if self.engine.is_some() && source.len() <= self.check_below {
            // The reference stays the checker: the same exports with the stages on the reference.
            let mut want = BTreeMap::new();
            for &p in &producers {
                want.insert(p, reference_stages(&self.program.per_file[p]).map_err(|e| format!("reference check: {e}"))?);
            }
            for (eid, rows) in exports_over(&want)? {
                if reference::multiset(&rows) != reference::multiset(&out[&eid]) {
                    return Err(format!("engine stages differ from the reference on export {eid}: {} vs {} rows", out[&eid].len(), rows.len()));
                }
            }
        }
        Ok(out)
    }

    /// A producer's stages on the engine: its clause's relational plan (`rel::run`, as `repo`
    /// runs it) over this content's inputs; each producer stage is the engine's stored table of
    /// the same name (`rel::TRACE`). A stage the engine did not store fails the content.
    #[allow(clippy::too_many_arguments)]
    fn engine_stages(&self, e: &eval::Rule, plan: &Plan<FileSource>, f: Option<&facts::Facts>, path: &str, source: &Arc<String>, text: &Rc<str>, memo: &crate::ast::FileMemo) -> Result<Vec<Rc<Vec<Vec<V>>>>, String> {
        let cl = e.clauses.iter().find(|c| c.id == plan.clause).ok_or_else(|| format!("engine: no clause {}", plan.clause))?;
        let eval::Kind::Relational(rp) = &cl.kind else { return Err(format!("engine: clause {} is not relational", plan.clause)) };
        let mut inputs = Vec::new();
        for (k, (_, idx)) in rp.inputs.iter().enumerate() {
            let inp = &e.inputs[*idx];
            inputs.push(match f {
                Some(f) if inp.paths.as_ref().is_none_or(|p| p.matches(path)) => {
                    let ctx = eval::FileCtx { f, path, source: source.as_str() };
                    let r = inp.rows(&ctx, &rp.input_schemas[k], memo, text);
                    if let Some(u) = r.unsupported {
                        return Err(format!("engine input {k}: {u}"));
                    }
                    r.table
                }
                _ => Rc::new(crate::sql::engine::Table::empty(rp.input_schemas[k].clone())),
            });
        }
        let traced: Rc<std::cell::RefCell<Option<crate::rel::Traced>>> = Rc::default();
        let sink = traced.clone();
        crate::rel::TRACE.with(|h| *h.borrow_mut() = Some(Box::new(move |_, _, t: &crate::rel::Traced| *sink.borrow_mut() = Some(t.clone()))));
        let r = crate::rel::run(rp, crate::rel::Partition { inputs, anchors: Default::default(), facts: f.map(|f| (f, path)), source: source.as_str(), path });
        crate::rel::TRACE.with(|h| *h.borrow_mut() = None);
        let tables = traced.borrow_mut().take().unwrap_or_default();
        plan.stages
            .iter()
            .map(|st| {
                let name = st.name.to_ascii_lowercase();
                let t = tables.iter().find(|(n, _)| *n == name).map(|(_, t)| t).ok_or_else(|| format!("engine: stage {} not stored ({})", st.name, r.failure.as_deref().unwrap_or("not in the clause")))?;
                let rows = table_rows(t, &column_map(&format!("engine stage {}", st.name), &st.schema, &t.schema.cols)?);
                conform(&format!("engine stage {}", st.name), &st.schema, &rows)?;
                Ok(Rc::new(rows))
            })
            .collect()
    }
}

/// `xexport --rule FILE [--config JSON] [--clauses a,b] --jobs TSV --out FILE [--threads N]
/// [--stages engine|reference] [--check-below BYTES]`; jobs: `path \t hash \t source file` per line.
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
    let engine = match arg("--stages").as_deref() {
        None | Some("engine") => true,
        Some("reference") => false,
        Some(o) => panic!("--stages {o}: engine | reference"),
    };
    let check_below: usize = arg("--check-below").map(|t| t.parse().unwrap()).unwrap_or(4096);
    let summary = {
        let e = compile(&doc, &config, &clauses, engine, check_below).unwrap_or_else(|e| panic!("compile: {e}"));
        json!({"exports": e.exports.iter().map(|(id, p)| json!({"id": id, "producer": p, "name": e.program.exported.iter().find(|x| x.id == *id).map(|x| x.name.clone())})).collect::<Vec<_>>(), "inputs": e.inputs.len(), "repository": e.program.repository.iter().map(|p| p.clause.clone()).collect::<Vec<_>>()})
    };
    eprintln!("xexport: {summary}");
    let next = std::sync::atomic::AtomicUsize::new(0);
    // Contents whose engine exports the reference re-derived equal (a difference fails the job).
    let checked = std::sync::atomic::AtomicUsize::new(0);
    let lines: parking_lot::Mutex<Vec<(usize, String)>> = parking_lot::Mutex::new(Vec::new());
    let t0 = std::time::Instant::now();
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| {
                let e = compile(&doc, &config, &clauses, engine, check_below).unwrap();
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
                        let small = src.len() <= check_below;
                        let r = e.run(path, Arc::new(src))?;
                        if engine && small {
                            checked.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        }
                        Ok(r)
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
    sum["stages"] = json!(if engine { "engine" } else { "reference" });
    sum["reference_checked"] = json!(checked.into_inner());
    sum["check_below_bytes"] = json!(check_below);
    sum["seconds"] = json!(t0.elapsed().as_secs_f64());
    std::fs::write(format!("{out_path}.summary.json"), serde_json::to_string_pretty(&sum).unwrap()).unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;
    use rerail_facts::ir::{Aff, Column};

    /// The dead-code `imports` defect (S3 wave 6): an input projecting a same-node regex's named
    /// groups (`capture.BASE`), evaluated outside `eval_file` as the exporter does, read NULL and
    /// '' because the group names were bound per thread by `eval_file` only. The input now carries
    /// them.
    #[test]
    fn regex_groups_read_outside_eval_file() {
        let doc = json!({"id": "t", "clauses": [{"id": "c", "relational": {
            "inputs": {"raw_imports": {"evidence": {"as": "row", "query": {"ast": {"language": "python", "syntax": "valid", "rule": {"all": [
                {"any": [{"kind": "import_from_statement"}, {"kind": "import_statement"}]},
                {"regex": "(?s)^(?:from\\s+(?P<BASE>[^\\s]+)\\s+)?import\\s+(?P<ITEMS>.*)$"}]}}},
                "project": {"base": "capture.BASE", "items": "capture.ITEMS"}}}},
            "query": "SELECT __path AS path, __start AS start_byte, __end AS end_byte FROM raw_imports"},
            "violation": {"code": "x", "anchor": "witness.span"}}]});
        let rule = eval::Rule::compile(&doc, &Map::new(), None, &facts::GRAMMAR).unwrap();
        let eval::Kind::Relational(plan) = &rule.clauses[0].kind else { panic!("relational") };
        let src = "from a.b import c, d\nimport e\n";
        let source = Arc::new(src.to_owned());
        let Ok(facts::Built::Facts(f)) = facts::build("t.py", source.clone()) else { panic!("facts") };
        let ctx = eval::FileCtx { f: &f, path: "t.py", source: &source };
        let r = rule.inputs[plan.inputs[0].1].rows(&ctx, &plan.input_schemas[0], &crate::ast::FileMemo::default(), &Rc::from(src));
        let rows: Vec<Vec<V>> = (0..r.table.len()).map(|i| r.table.row(i)).collect();
        assert_eq!(format!("{rows:?}"), format!("{:?}", vec![vec![V::text("t.py"), V::Int(0), V::Int(20), V::text("a.b"), V::text("c, d")], vec![V::text("t.py"), V::Int(21), V::Int(29), V::Null, V::text("e")]]));
    }

    /// Columns are matched by name, never by position; a missing column is an error.
    #[test]
    fn columns_by_name() {
        let want = [Column::new("__path", Aff::Text), Column::new("top", Aff::None), Column::new("base", Aff::None)];
        let have: Vec<String> = ["__path", "base", "top", "extra"].iter().map(|s| s.to_string()).collect();
        assert_eq!(column_map("r", &want, &have).unwrap(), vec![0, 2, 1]);
        assert!(column_map("r", &[Column::new("items", Aff::None)], &have).is_err());
    }

    /// A row of the wrong width, or a value its column's affinity would store differently, fails.
    #[test]
    fn rows_conform_to_the_schema() {
        let s = [Column::new("__path", Aff::Text), Column::new("__start", Aff::Integer), Column::new("x", Aff::None)];
        assert!(conform("r", &s, &[vec![V::text("p"), V::Int(3), V::text("4")]]).is_ok());
        assert!(conform("r", &s, &[vec![V::text("p"), V::Int(3)]]).is_err());
        assert!(conform("r", &s, &[vec![V::text("p"), V::text("3"), V::Null]]).is_err());
        assert!(conform("r", &s, &[vec![V::Int(1), V::Int(3), V::Null]]).is_err());
    }
}
