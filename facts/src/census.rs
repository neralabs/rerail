//! The 60-rule census (diagnostics only): compiles, classification, refusals with counts,
//! exported relations, the sharing census (stage texts byte-identical across clauses; stages
//! equal by IR digest) and the order-dependent constructs of repository plans.
use crate::compile::compile_report;
use crate::front::rule::LeafCheck;
use crate::ir::{AggFn, AnySource, Expr, Partition, Rel, RepoSource, StageDef, WindowKind};
use serde_json::{Map, Value as Json, json};
use std::collections::BTreeMap;
use std::fmt::Write;
use std::path::Path;

fn part(p: Option<Partition>) -> &'static str {
    match p {
        None => "implicit",
        Some(Partition::File) => "file",
        Some(Partition::Repository) => "repository",
    }
}

/// A refusal's construct with its location detail removed, for counting.
fn normalise(c: &str) -> String {
    // Pattern refusals carry the rule path (`rule/all/0/...: `); SQL ones the token index.
    let c = match c.strip_prefix("pattern: ") {
        Some(rest) => format!("pattern: {}", rest.rsplit(": ").next().unwrap_or(rest)),
        None => c.to_owned(),
    };
    let re = regex::Regex::new(r" at token \d+").unwrap();
    re.replace_all(&c, "").into_owned()
}

/// Order-dependent constructs inside a repository relation.
fn order_constructs(r: &Rel<RepoSource>, out: &mut Vec<String>) {
    match r {
        Rel::Window { funcs, .. } => {
            for f in funcs {
                if matches!(f.kind, WindowKind::RowNumber) {
                    out.push(format!("row_number() (partition {}, order {})", f.partition.len(), f.order.len()));
                }
            }
        }
        Rel::Sort { keys, limit: Some(_), .. } => out.push(format!("ORDER BY ({} keys) LIMIT", keys.len())),
        Rel::Aggregate { aggs, .. } => {
            for a in aggs {
                if matches!(a.f, AggFn::GroupConcat | AggFn::JsonGroupArray) {
                    out.push(format!("{:?}", a.f));
                }
            }
        }
        _ => {}
    }
    visit_children(r, &mut |c| order_constructs(c, out));
}

pub fn visit_children<S>(r: &Rel<S>, f: &mut dyn FnMut(&Rel<S>)) {
    let sub = |e: &Expr<S>, f: &mut dyn FnMut(&Rel<S>)| {
        crate::front::sql::visit_expr(e, &mut |x| match x {
            Expr::Exists { rel, .. } | Expr::InRel { rel, .. } | Expr::Scalar(rel) => f(rel),
            _ => {}
        })
    };
    match r {
        Rel::JsonEach { input, arg, on, .. } => {
            sub(arg, f);
            on.iter().for_each(|e| sub(e, f));
            f(input);
        }
        Rel::Join { left, right, on, .. } => {
            on.iter().for_each(|e| sub(e, f));
            f(left);
            f(right);
        }
        Rel::Filter { input, pred } => {
            sub(pred, f);
            f(input);
        }
        Rel::Project { input, exprs, .. } => {
            exprs.iter().for_each(|e| sub(e, f));
            f(input);
        }
        Rel::Aggregate { input, keys, aggs } => {
            keys.iter().for_each(|e| sub(e, f));
            aggs.iter().for_each(|a| a.arg.iter().for_each(|e| sub(e, f)));
            f(input);
        }
        Rel::Window { input, funcs } => {
            for w in funcs {
                w.partition.iter().chain(w.order.iter().map(|o| &o.0)).for_each(|e| sub(e, f));
            }
            f(input);
        }
        Rel::Distinct { input, .. } | Rel::Sort { input, .. } => f(input),
        Rel::Union { left, right, .. } => {
            f(left);
            f(right);
        }
        Rel::With { bindings, body } => {
            for (_, b) in bindings {
                match b {
                    crate::ir::Binding::Plain(r) => f(r),
                    crate::ir::Binding::Fix { init, step, .. } => init.iter().chain(step.iter()).for_each(|r| f(r)),
                }
            }
            f(body);
        }
        Rel::Scan { .. } | Rel::Local { .. } | Rel::Unit => {}
    }
}

/// Stage text normalised for the byte-identity census (whitespace runs collapsed).
fn norm_text(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// IR digest of every stage of a relational clause: the lowered relation with stage scans
/// replaced by the read stage's digest and input scans by the input's evidence, so equal
/// digests compute equal relations whatever the stage and input names.
fn stage_digests(inputs: &[crate::ir::InputDef], front_stages: &[(String, Rel<AnySource>)]) -> Vec<String> {
    let mut digests: Vec<String> = Vec::new();
    for (_, rel) in front_stages {
        let mapped = crate::ir::map_rel(rel.clone(), &mut |s| {
            Ok::<_, ()>(match s {
                AnySource::Stage(i) => format!("stage:{}", digests[i as usize]),
                AnySource::Input(i) => {
                    let d = &inputs[i as usize];
                    format!("input:{}", blake3::hash(format!("{:?}{:?}{:?}{:?}{:?}", d.kind, d.fields, d.anchor, d.each, d.paths).as_bytes()).to_hex())
                }
                AnySource::Syntax(t) => format!("syntax:{t:?}"),
                AnySource::Members(c) => format!("members:{}", c.name()),
                AnySource::Summaries { rel, links } => format!("summaries:{}:{links}", rel.name()),
            })
        })
        .unwrap();
        let d = blake3::hash(format!("{mapped:?}").as_bytes()).to_hex()[..16].to_owned();
        digests.push(d);
    }
    digests
}

/// A leaf check that accepts every pattern and regex: the census's IR-level pass, which shows
/// what the IR, the front ends and the classifier decide independently of the leaf executor's
/// coverage. Never used to evaluate.
pub struct IrOnly;
impl LeafCheck for IrOnly {
    fn pattern(&mut self, _: &Json, _: &std::collections::BTreeMap<String, Json>) -> Result<Vec<String>, String> {
        Ok(vec![])
    }
    fn regex(&mut self, _: &Json) -> Result<(), String> {
        Ok(())
    }
}

/// Stage digest with inputs abstracted to their columns: equal shapes compute the same algebra
/// over (possibly different) leaves, i.e. one relation over all nodes filtered per clause.
fn stage_shapes(front_stages: &[(String, Rel<AnySource>)], inputs: &[crate::ir::InputDef]) -> Vec<String> {
    let mut digests: Vec<String> = Vec::new();
    for (_, rel) in front_stages {
        let mapped = crate::ir::map_rel(rel.clone(), &mut |s| {
            Ok::<_, ()>(match s {
                AnySource::Stage(i) => format!("stage:{}", digests[i as usize]),
                AnySource::Input(i) => format!("input[{}]", inputs[i as usize].schema.iter().map(|c| c.name.as_str()).collect::<Vec<_>>().join(",")),
                AnySource::Syntax(t) => format!("syntax:{t:?}"),
                AnySource::Members(c) => format!("members:{}", c.name()),
                AnySource::Summaries { rel, links } => format!("summaries:{}:{links}", rel.name()),
            })
        })
        .unwrap();
        digests.push(blake3::hash(format!("{mapped:?}").as_bytes()).to_hex()[..16].to_owned());
    }
    digests
}

pub struct Census {
    pub json: Json,
    pub markdown: String,
}

pub fn run(dir: &Path, config: &Map<String, Json>, leaves: &mut dyn LeafCheck) -> Result<Census, String> {
    let mut paths: Vec<_> = std::fs::read_dir(dir).map_err(|e| e.to_string())?.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|x| x == "json")).collect();
    paths.sort();
    let mut rules = Vec::new();
    let mut constructs: BTreeMap<String, (usize, BTreeMap<String, usize>)> = BTreeMap::new();
    let mut runtime: BTreeMap<String, usize> = BTreeMap::new();
    let mut texts: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut digests: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut shapes: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut exports_json = Vec::new();
    let mut classification = Vec::new();
    let mut orders = Vec::new();
    let mut compiled_n = 0;
    // The pattern front end (I3): pattern inputs lowered to algebra over core relations, and the
    // first construct refused per input otherwise.
    let (mut pattern_inputs, mut lowered) = (0usize, 0usize);
    let mut not_lowered: BTreeMap<String, usize> = BTreeMap::new();
    for p in &paths {
        let doc: Json = serde_json::from_slice(&std::fs::read(p).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        // The configuration's options that this document declares.
        let declared = crate::policy::option_values(&doc, &Map::new())?;
        let config: &Map<String, Json> = &config.iter().filter(|(k, _)| declared.contains_key(*k)).map(|(k, v)| (k.clone(), v.clone())).collect();
        let c = compile_report(&doc, config, leaves)?;
        let rule = c.program.rule.clone();
        let short = rule.trim_start_matches("data-layer-").to_owned();
        if c.refusals.is_empty() {
            compiled_n += 1;
        }
        for r in &c.refusals {
            let e = constructs.entry(normalise(&r.construct)).or_default();
            e.0 += 1;
            *e.1.entry(short.clone()).or_default() += 1;
        }
        for i in c.program.inputs.iter().filter(|i| matches!(i.kind, crate::ir::InputKind::Pattern { .. })) {
            pattern_inputs += 1;
            match &i.algebra {
                Ok(_) => lowered += 1,
                Err(e) => {
                    // Drop the rule path (`rule/all/0/...: `) so equal constructs count together.
                    let k = match e.split_once(": ") {
                        Some((at, what)) if at.starts_with("rule") || at.starts_with("utils/") => what,
                        _ => e.as_str(),
                    };
                    *not_lowered.entry(k.to_owned()).or_default() += 1
                }
            }
        }
        for r in &c.program.runtime_refusals {
            *runtime.entry(format!("{} ({short}/{})", r.construct, r.clause)).or_default() += 1;
        }
        let resolved = crate::policy::resolve(&doc, config)?;
        // Sharing: stage texts and IR digests across clauses.
        let front = crate::front::rule::front(&doc, config, leaves)?;
        for (fc, clause) in front.clauses.iter().zip(resolved["clauses"].as_array().unwrap().iter().filter(|c| c.get("define").is_none())) {
            let Some(rel) = clause.get("relational") else { continue };
            for st in rel.get("stages").and_then(Json::as_array).into_iter().flatten() {
                if let Some(q) = st.get("query").and_then(Json::as_str) {
                    texts.entry(norm_text(q)).or_default().push(format!("{short}/{}/{}", fc.id, st["name"].as_str().unwrap_or("")));
                }
            }
            if let Some(crate::front::rule::ClauseBody::Relational { stages, .. }) = &fc.body {
                let fs: Vec<(String, Rel<AnySource>)> = stages
                    .iter()
                    .map(|s| match &s.def {
                        StageDef::Query { rel } => (s.name.clone(), rel.clone()),
                        StageDef::Reach { roots, .. } => (s.name.clone(), roots.clone()),
                    })
                    .collect();
                for ((name, _), d) in fs.iter().zip(stage_digests(&front.inputs, &fs)) {
                    digests.entry(d).or_default().push(format!("{short}/{}/{name}", fc.id));
                }
                for ((name, _), d) in fs.iter().zip(stage_shapes(&fs, &front.inputs)) {
                    shapes.entry(d).or_default().push(format!("{short}/{}/{name}", fc.id));
                }
            }
        }
        let mut forms: BTreeMap<&str, usize> = BTreeMap::new();
        for rep in &c.reports {
            *forms.entry(rep.form).or_default() += 1;
            if rep.form != "forbid" || rep.inferred == Some(Partition::Repository) {
                if rep.form == "refused" && rep.declared.is_none() {
                    continue;
                }
                classification.push(json!({
                    "rule": short, "clause": rep.clause, "form": rep.form, "declared": part(rep.declared),
                    "inferred": rep.inferred.map(|p| part(Some(p))).unwrap_or("refused"),
                    "file_stages": rep.file_stages, "repository_stages": rep.repo_stages, "exports": rep.exports.len(),
                }));
            }
        }
        for e in &c.program.exported {
            exports_json.push(json!({
                "rule": short, "name": e.name, "columns": e.schema.iter().map(|c| c.name.clone()).collect::<Vec<_>>(),
                "distinct": e.distinct, "ordered": e.ordered,
            }));
        }
        for plan in &c.program.repository {
            let mut found = Vec::new();
            for st in &plan.stages {
                let mut f = Vec::new();
                match &st.def {
                    StageDef::Query { rel } => order_constructs(rel, &mut f),
                    StageDef::Reach { roots, edges, .. } => {
                        order_constructs(roots, &mut f);
                        if let Some(e) = edges {
                            order_constructs(e, &mut f);
                        }
                    }
                }
                found.extend(f.into_iter().map(|x| format!("{}: {x}", st.name)));
            }
            if let crate::ir::Sink::Findings { rel, .. } | crate::ir::Sink::Budget { rel, .. } = &plan.sink {
                let mut f = Vec::new();
                order_constructs(rel, &mut f);
                found.extend(f.into_iter().map(|x| format!("query: {x}")));
            }
            let refused: Vec<String> = c.refusals.iter().filter(|r| r.clause == plan.clause && r.construct.starts_with("order-dependent")).map(|r| format!("{}: {}", r.at, r.construct)).collect();
            let marks: Vec<Json> = c.reports.iter().filter(|r| r.clause == plan.clause).flat_map(|r| r.order.iter()).map(|o| json!({ "at": o.at, "construct": o.construct, "total": format!("{:?}", o.total).to_lowercase() })).collect();
            orders.push(json!({ "rule": short, "clause": plan.clause, "constructs": found, "refused": refused, "marks": marks }));
        }
        rules.push(json!({
            "rule": short,
            "compiles": c.refusals.is_empty(),
            "forms": forms,
            "disabled_by_when": c.disabled.len(),
            "per_file_plans": c.program.per_file.len(),
            "repository_plans": c.program.repository.len(),
            "exports": c.program.exported.len(),
            "refusals": c.refusals.len(),
            "runtime_refusals": c.program.runtime_refusals.len(),
            "report_when": c.program.report_when,
            "refusal_list": c.refusals.iter().map(|r| json!({"clause": r.clause, "at": r.at, "construct": r.construct})).collect::<Vec<_>>(),
        }));
    }
    let shared_texts: Vec<Json> = texts
        .iter()
        .filter(|(_, v)| {
            let mut clauses: Vec<&str> = v.iter().map(|s| s.rsplit_once('/').unwrap().0).collect();
            clauses.dedup();
            clauses.len() > 1
        })
        .map(|(t, v)| json!({ "bytes": t.len(), "stages": v }))
        .collect();
    let across = |v: &Vec<String>| {
        let mut clauses: Vec<&str> = v.iter().map(|s| s.rsplit_once('/').unwrap().0).collect();
        clauses.sort();
        clauses.dedup();
        clauses.len() > 1
    };
    let shared_digests: Vec<Json> = digests.iter().filter(|(_, v)| across(v)).map(|(d, v)| json!({ "digest": d, "stages": v })).collect();
    let shared_shapes: Vec<Json> = shapes.iter().filter(|(_, v)| across(v)).map(|(d, v)| json!({ "digest": d, "stages": v })).collect();
    let json = json!({
        "rules": rules, "compiled": compiled_n, "total": paths.len(),
        "constructs": constructs.iter().map(|(k, (n, by))| json!({"construct": k, "count": n, "rules": by})).collect::<Vec<_>>(),
        "runtime_refusals": runtime, "classification": classification, "exports": exports_json,
        "sharing": { "byte_identical_texts": shared_texts, "ir_digest_groups": shared_digests, "ir_shape_groups": shared_shapes },
        "repository_order": orders,
        "pattern_front_end": { "pattern_inputs": pattern_inputs, "lowered": lowered, "not_lowered": not_lowered },
    });
    let markdown = render(&json);
    Ok(Census { json, markdown })
}

fn render(j: &Json) -> String {
    let mut m = String::new();
    let rules = j["rules"].as_array().unwrap();
    writeln!(m, "Compiled without refusal: **{} / {}** rules.\n", j["compiled"], j["total"]).unwrap();
    let pf = &j["pattern_front_end"];
    writeln!(m, "Pattern front end (matcher inputs as algebra over core relations): **{} / {}** pattern inputs of compiled clauses lowered.\n", pf["lowered"], pf["pattern_inputs"]).unwrap();
    let mut nl: Vec<(&String, u64)> = pf["not_lowered"].as_object().unwrap().iter().map(|(k, v)| (k, v.as_u64().unwrap())).collect();
    nl.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    for (k, v) in nl.iter().take(15) {
        writeln!(m, "- {v} × {k}").unwrap();
    }
    writeln!(m).unwrap();
    writeln!(m, "| rule | forbid | budget | relational | disabled by `when` | per-file plans | repository plans | exports | refusals | runtime refusals | compiles |\n|---|--:|--:|--:|--:|--:|--:|--:|--:|--:|---|").unwrap();
    let mut tot = [0u64; 8];
    for r in rules {
        let f = |k: &str| r["forms"].get(k).and_then(Json::as_u64).unwrap_or(0);
        let refused = f("refused");
        let vals = [f("forbid"), f("budget"), f("relational"), r["disabled_by_when"].as_u64().unwrap(), r["per_file_plans"].as_u64().unwrap(), r["repository_plans"].as_u64().unwrap(), r["exports"].as_u64().unwrap(), r["refusals"].as_u64().unwrap()];
        for (t, v) in tot.iter_mut().zip(vals) {
            *t += v;
        }
        writeln!(
            m,
            "| `{}` | {} | {} | {} | {} | {} | {} | {} | {}{} | {} | {} |",
            r["rule"].as_str().unwrap(),
            vals[0],
            vals[1],
            vals[2],
            vals[3],
            vals[4],
            vals[5],
            vals[6],
            vals[7],
            if refused > 0 { format!(" ({refused} clauses)") } else { String::new() },
            r["runtime_refusals"],
            if r["compiles"].as_bool().unwrap() { "yes" } else { "**refused**" }
        )
        .unwrap();
    }
    writeln!(m, "| **total** | {} | {} | {} | {} | {} | {} | {} | {} | | |\n", tot[0], tot[1], tot[2], tot[3], tot[4], tot[5], tot[6], tot[7]).unwrap();
    writeln!(m, "Refused constructs (count; rules):\n\n| construct | count | rules |\n|---|--:|---|").unwrap();
    for c in j["constructs"].as_array().unwrap() {
        let by: Vec<String> = c["rules"].as_object().unwrap().iter().map(|(k, v)| format!("`{k}` {v}")).collect();
        writeln!(m, "| {} | {} | {} |", c["construct"].as_str().unwrap().replace('|', "\\|"), c["count"], by.join(", ")).unwrap();
    }
    writeln!(m, "\nRuntime refusals (compiled; the leaf executor refuses per content):\n").unwrap();
    for (k, v) in j["runtime_refusals"].as_object().unwrap() {
        writeln!(m, "- {k}: {v}").unwrap();
    }
    writeln!(m, "\nClassification (every relational and budget clause, and every clause inferred cross-file):\n\n| rule | clause | form | declared | inferred | per-file stages | repository stages | exports |\n|---|---|---|---|---|--:|--:|--:|").unwrap();
    for c in j["classification"].as_array().unwrap() {
        if c["form"] == "relational" && c["declared"] == "file" && c["inferred"] == "file" && c["file_stages"] == 0 {
            continue;
        }
        writeln!(m, "| `{}` | `{}` | {} | {} | {} | {} | {} | {} |", c["rule"].as_str().unwrap(), c["clause"].as_str().unwrap(), c["form"].as_str().unwrap(), c["declared"].as_str().unwrap(), c["inferred"].as_str().unwrap(), c["file_stages"], c["repository_stages"], c["exports"]).unwrap();
    }
    let trivial = j["classification"].as_array().unwrap().iter().filter(|c| c["form"] == "relational" && c["declared"] == "file" && c["inferred"] == "file" && c["file_stages"] == 0).count();
    writeln!(m, "\nNot listed: {trivial} stage-less `partitionBy: file` relational clauses, all inferred per file.").unwrap();
    let sh = &j["sharing"];
    let count = |k: &str| {
        let g = sh[k].as_array().unwrap();
        let rules = g.iter().filter(|x| {
            let mut r: Vec<&str> = x["stages"].as_array().unwrap().iter().map(|s| s.as_str().unwrap().split('/').next().unwrap()).collect();
            r.sort();
            r.dedup();
            r.len() > 1
        });
        (g.len(), g.iter().map(|x| x["stages"].as_array().unwrap().len()).sum::<usize>(), rules.count())
    };
    let (tg, ts, tr) = count("byte_identical_texts");
    let (dg, ds, dr) = count("ir_digest_groups");
    let (sg, ss, sr) = count("ir_shape_groups");
    writeln!(m, "\nSharing census (stages recurring in more than one clause):\n\n| measure | groups | stages in them | groups spanning rules |\n|---|--:|--:|--:|").unwrap();
    writeln!(m, "| byte-identical stage text (whitespace-normalised) | {tg} | {ts} | {tr} |").unwrap();
    writeln!(m, "| equal IR digest (same algebra over the same inputs) | {dg} | {ds} | {dr} |").unwrap();
    writeln!(m, "| equal IR shape (same algebra, inputs abstracted to their columns) | {sg} | {ss} | {sr} |").unwrap();
    let all_marks: Vec<&Json> = j["repository_order"].as_array().unwrap().iter().flat_map(|o| o["marks"].as_array().unwrap().iter()).collect();
    let proven = all_marks.iter().filter(|x| x["total"] == "proven").count();
    writeln!(m, "\nOrder-dependent constructs in repository plans (after the cut): {} marked, {proven} `total: proven`, {} `total: checked` (a tie between distinct rows is a run-time refusal):\n", all_marks.len(), all_marks.len() - proven).unwrap();
    for o in j["repository_order"].as_array().unwrap() {
        let ms: Vec<String> = o["marks"].as_array().unwrap().iter().map(|x| format!("{}: {} ({})", x["at"].as_str().unwrap(), x["construct"].as_str().unwrap(), x["total"].as_str().unwrap())).collect();
        let refused = o["refused"].as_array().unwrap().len();
        writeln!(m, "- `{}`/`{}`: {}{}", o["rule"].as_str().unwrap(), o["clause"].as_str().unwrap(), if ms.is_empty() { "none".to_string() } else { ms.join("; ") }, if refused > 0 { format!(" (refused: {refused})") } else { String::new() }).unwrap();
    }
    m
}
