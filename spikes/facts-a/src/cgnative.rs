//! `facts-a cg-native`: native's per-file Python call summary (`calls::summarize`, the reference
//! graph's leaf; SCHEMA.md § 3.4), extracted per **content** by calling native with a symbolic
//! module path, and the differential that decides when that is exact.
//!
//! Native qualifies every name of a summary through the file's `ModulePath`, which depends on the
//! worktree's `__init__.py` set. To keep the export content-keyed, the summary is extracted once
//! per variant with a symbolic module whose segments are sentinels (`\u{1}0` … `\u{1}15`, then
//! `\u{1}S` for the stem, or `__init__`). Exported names spell each maximal run of sentinel
//! segments as one token `\u{1}<run length>` (`compact`). The repository plan substitutes the
//! real module in the dataflow (S3): a token is native's `qualified(count)` prefix of the real
//! module, where `count = run - symbolic length + real length` (`substitute`).
//!
//! `cg-native --root R --out-dir D [--export F.jsonl.zst]`: for every file of `R`, native with
//! the real module path (from the corpus's `__init__.py` files, as the host derives it) against
//! the symbolic variant the file's path selects, compacted and substituted. Both are compared
//! before and after `as_references` with the reference-graph rules' links (`siblingLinks`,
//! `constructorFields`), each relation as a multiset of rendered rows. A substitution that has no
//! real counterpart (a relative import above the real package root, a directory or stem that
//! keeps a dot) is a refusal: it is counted with its reason and never compared.
use crate::facts;
use crate::repo::walk;
use crate::arg;
use ast_grep_core::AstGrep;
use rerail_engine::code_rule_dsl::calls::contracts;
use rerail_engine::code_rule_dsl::calls::summary::Summary;
use rerail_engine::code_rule_dsl::calls::summarize;
use rerail_engine::code_rule_dsl::language::GrammarDialect;
use rerail_engine::code_rule_dsl::model::python_module_path_on_disk;
use rerail_engine::code_rule_dsl::module_path::ModulePath;
use serde_json::{Value as Json, json};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;
use parking_lot::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Directory sentinels of the symbolic module (more than any real package depth needs:
/// `MAX_DIRS` bounds the relative-import level that stays exact).
pub const MAX_DIRS: usize = 16;
pub const STEM: &str = "\u{1}S";

/// The symbolic module a content is summarised under.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Variant {
    /// A module of a package (`pkg/mod.py`): directories, then the stem.
    Module,
    /// A package's `__init__.py`: directories, then `__init__`.
    Init,
    /// A file outside any package: Ruff's stem fallback (`ModulePath::stem`, not a package).
    Unpackaged,
}

impl Variant {
    pub const ALL: [Variant; 3] = [Variant::Module, Variant::Init, Variant::Unpackaged];
    pub fn name(self) -> &'static str {
        match self {
            Variant::Module => "module",
            Variant::Init => "init",
            Variant::Unpackaged => "unpackaged",
        }
    }
    pub fn module(self) -> ModulePath {
        let dirs: Vec<String> = (0..MAX_DIRS).map(|i| format!("\u{1}{i}")).collect();
        let mut names: Vec<&str> = dirs.iter().map(String::as_str).collect();
        match self {
            Variant::Module => names.push(STEM),
            Variant::Init => names.push("__init__"),
            Variant::Unpackaged => return ModulePath::stem(&format!("{STEM}.py")).expect("stem"),
        }
        ModulePath::names(&names).expect("symbolic module")
    }
    /// The variant a real module path selects.
    pub fn of(real: &ModulePath) -> Variant {
        if !real.in_package() {
            Variant::Unpackaged
        } else if real.last() == "__init__" && real.segments.len() > 1 {
            Variant::Init
        } else {
            Variant::Module
        }
    }
}

/// A summary's relations rendered as text rows, names spelled out (SCHEMA § 3.4).
#[derive(Default, Debug)]
pub struct Rendered {
    pub rels: BTreeMap<&'static str, Vec<Vec<String>>>,
}

pub fn render(s: &Summary) -> Rendered {
    let n = |id: u32| s.name(id).to_owned();
    let i = |x: u32| x.to_string();
    let mut r = Rendered::default();
    let mut put = |rel: &'static str, rows: Vec<Vec<String>>| {
        r.rels.insert(rel, rows);
    };
    put("cgn_module", s.module.iter().map(|&m| vec![n(m)]).collect());
    put("cgn_def", s.defs.iter().enumerate().map(|(k, d)| vec![i(k as u32), n(d.name), i(d.start), i(d.end), i(d.flags as u32)]).collect());
    put("cgn_edge", s.edges.iter().map(|&(d, c)| vec![i(d), n(c)]).collect());
    put("cgn_reference", s.references.iter().map(|&(d, c)| vec![i(d), n(c)]).collect());
    put("cgn_reference_target", s.reference_targets.iter().map(|&c| vec![n(c)]).collect());
    put("cgn_reference_alias", s.reference_aliases.iter().map(|&(a, b)| vec![n(a), n(b)]).collect());
    put("cgn_reference_decorator", s.reference_decorators.iter().map(|&(d, c)| vec![i(d), n(c)]).collect());
    put("cgn_target", s.targets.iter().map(|&c| vec![n(c)]).collect());
    put("cgn_alias", s.aliases.iter().map(|&(a, b)| vec![n(a), n(b)]).collect());
    put("cgn_opaque", s.opaque.iter().map(|&c| vec![n(c)]).collect());
    put("cgn_star", s.stars.iter().map(|&c| vec![n(c)]).collect());
    put("cgn_super", s.supers.iter().map(|&(a, b, k)| vec![n(a), n(b), i(k as u32)]).collect());
    put("cgn_open", vec![vec![i(s.open as u32), i(s.open_sites as u32)]]);
    r
}

/// Why a symbolic name has no real counterpart.
pub const BEYOND_ROOT: &str = "relative import above the real package root";
pub const DOTTED_SEGMENT: &str = "real module segment keeps a dot (native `qualified` is None)";
pub const CONSTANT_MODULE: &str = "real module named like a module native compares names with (abc, typing, typing_extensions, functools)";
pub const TOO_DEEP: &str = "relative import level above the symbolic module's 17 segments";
/// Modules whose qualified names native's summary compares with constants (`TRANSPARENT`,
/// `functools.wraps`): a symbolic module never equals them, a real one may.
pub const CONSTANT_MODULES: &[&str] = &["abc", "typing", "typing_extensions", "functools"];

/// The exported spelling of a symbolic name: each maximal run of sentinel segments
/// (`\u{1}0.\u{1}1. … .\u{1}S`) becomes one token `\u{1}<run length>`, the only thing the
/// substitution reads.
pub fn compact(name: &str) -> String {
    if !name.contains('\u{1}') {
        return name.to_owned();
    }
    let mut out = String::with_capacity(name.len());
    let mut rest = name;
    while let Some(at) = rest.find('\u{1}') {
        out.push_str(&rest[..at]);
        let mut run = 0usize;
        let mut end = at;
        loop {
            let seg = &rest[end..];
            let len = seg.find('.').unwrap_or(seg.len());
            let token = &seg[..len];
            let is_sentinel = token.strip_prefix('\u{1}').is_some_and(|t| t == "S" || t.parse::<usize>().is_ok_and(|d| d < MAX_DIRS));
            assert!(is_sentinel || run > 0, "stray sentinel in {name:?}");
            if !is_sentinel {
                break;
            }
            run += 1;
            end += len;
            if rest[end..].starts_with(".\u{1}") {
                end += 1;
            } else {
                break;
            }
        }
        out.push('\u{1}');
        out.push_str(&run.to_string());
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

/// Replaces each token `\u{1}<r>` of a compact name by the real module's `qualified(r -
/// symbolic length + real length)` prefix (native's own `ModulePath::qualified`); an empty
/// prefix takes its following `.` with it (`relative` with an empty base starts at `rest`).
pub fn substitute(name: &str, real: &ModulePath, symbolic_len: usize) -> Result<String, &'static str> {
    if !name.contains('\u{1}') {
        return Ok(name.to_owned());
    }
    let mut out = String::with_capacity(name.len());
    let mut rest = name;
    while let Some(at) = rest.find('\u{1}') {
        out.push_str(&rest[..at]);
        let digits = rest[at + 1..].bytes().take_while(u8::is_ascii_digit).count();
        let run: isize = rest[at + 1..at + 1 + digits].parse().expect("sentinel run length");
        rest = &rest[at + 1 + digits..];
        let count = run - symbolic_len as isize + real.segments.len() as isize;
        if count < 0 {
            return Err(BEYOND_ROOT);
        }
        let prefix = real.qualified(count as usize).ok_or(DOTTED_SEGMENT)?;
        if prefix.is_empty() {
            rest = rest.strip_prefix('.').unwrap_or(rest);
        } else {
            out.push_str(&prefix.join("."));
        }
    }
    out.push_str(rest);
    Ok(out)
}

fn compacted(r: &Rendered) -> Rendered {
    Rendered { rels: r.rels.iter().map(|(rel, rows)| (*rel, rows.iter().map(|row| row.iter().map(|c| compact(c)).collect()).collect())).collect() }
}

fn substituted(r: &Rendered, real: &ModulePath, symbolic_len: usize) -> Result<Rendered, &'static str> {
    let mut out = Rendered::default();
    for (rel, rows) in &r.rels {
        let mut v = Vec::with_capacity(rows.len());
        for row in rows {
            v.push(row.iter().map(|c| substitute(c, real, symbolic_len)).collect::<Result<Vec<_>, _>>()?);
        }
        out.rels.insert(rel, v);
    }
    Ok(out)
}

/// The reference-graph links of batch 6 (`internal-dead-code`'s `references`, which
/// `imperative-private-functions`' `calls` term repeats), compiled by native.
fn links() -> Vec<(String, contracts::Key)> {
    let v = json!({
        "siblingLinks": [
            {"decorator": "(?:legacy_sync|legacy_module_sync|direct_legacy_module_sync)$", "name": {"replace": "$", "by": "_lazy"}},
            {"decorator": "(?:legacy_async|legacy_module_async|direct_legacy_module_async)$", "name": {"replace": "(?:_async)?$", "by": "_lazy"}}
        ],
        "constructorFields": {"fieldToParameter": {"replace": "^_+", "by": ""}}
    });
    let key = contracts::compile(v.get("siblingLinks"), v.get("constructorFields"), "links", 1 << 20).expect("links");
    vec![("none".into(), 0), (blake3::hash(v.to_string().as_bytes()).to_hex()[..16].to_owned(), key)]
}

/// Native's summary of one document under `module` (`calls::summarize`, as the workspace calls it).
fn summary(doc: &rerail_engine::code_rule_dsl::tape::TapeDoc, path: &str, module: &ModulePath, has_errors: bool, bytes: usize) -> Summary {
    let grep = AstGrep::doc(doc.clone());
    summarize(grep.root(), GrammarDialect::Python, path, Some(module), has_errors, bytes, None).0
}

/// The host's module path for a corpus file (`python_module_path_on_disk`, then the stem
/// fallback), as `FileInput::new` derives it.
fn real_module(root: &Path, path: &str) -> Option<ModulePath> {
    match python_module_path_on_disk(&root.join(path)) {
        Some(Json::String(m)) => ModulePath::dotted(&m),
        Some(Json::Array(names)) => ModulePath::names(&names.iter().map(|n| n.as_str()).collect::<Option<Vec<_>>>()?),
        Some(_) => None,
        None => ModulePath::stem(path),
    }
}

#[derive(Default)]
struct Stat {
    rows: u64,
    files_differing: u64,
    examples: Vec<String>,
}

#[derive(Default)]
struct Out {
    files: u64,
    compared: u64,
    refused: BTreeMap<&'static str, u64>,
    refused_examples: Vec<String>,
    variants: BTreeMap<&'static str, u64>,
    stats: BTreeMap<String, Stat>,
    extract_ns: Vec<u64>,
}

fn compare(out: &mut Out, scope: &str, path: &str, real: &Rendered, sym: &Rendered) {
    for (rel, rows) in &real.rels {
        let st = out.stats.entry(format!("{scope}.{rel}")).or_default();
        st.rows += rows.len() as u64;
        let mut a: Vec<&Vec<String>> = rows.iter().collect();
        let mut b: Vec<&Vec<String>> = sym.rels.get(rel).map(|r| r.iter().collect()).unwrap_or_default();
        a.sort();
        b.sort();
        if a != b {
            st.files_differing += 1;
            if st.examples.len() < 5 {
                let only_real: Vec<_> = a.iter().filter(|x| b.binary_search(x).is_err()).take(3).collect();
                let only_sym: Vec<_> = b.iter().filter(|x| a.binary_search(x).is_err()).take(3).collect();
                st.examples.push(format!("{path}: real {} rows, symbolic {} rows; only real {only_real:?}; only symbolic {only_sym:?}", a.len(), b.len()));
            }
        }
    }
}

fn json_of(r: &Rendered) -> Json {
    json!(r.rels.iter().map(|(k, v)| (k.to_string(), json!(v))).collect::<BTreeMap<_, _>>())
}

pub fn run(args: &[String]) {
    let root = arg(args, "--root").expect("--root");
    let threads: usize = arg(args, "--threads").map_or(4, |t| t.parse().unwrap());
    let out_dir = arg(args, "--out-dir").expect("--out-dir");
    let export = arg(args, "--export");
    let limit: Option<usize> = arg(args, "--limit").map(|x| x.parse().unwrap());
    std::fs::create_dir_all(&out_dir).unwrap();
    let root = Path::new(&root);
    let mut paths = walk(root);
    if let Some(l) = limit {
        paths.truncate(l);
    }
    let links = links();
    let next = AtomicUsize::new(0);
    let exported: Mutex<std::collections::BTreeSet<(String, bool)>> = Default::default();
    // The export is zstd-compressed JSONL (level 3, as fact files v1).
    let sink: Option<Mutex<zstd::stream::write::Encoder<'static, std::io::BufWriter<std::fs::File>>>> =
        export.as_ref().map(|p| Mutex::new(zstd::stream::write::Encoder::new(std::io::BufWriter::new(std::fs::File::create(p).unwrap()), 3).unwrap()));
    let t_all = std::time::Instant::now();
    let outs: Vec<Out> = std::thread::scope(|s| {
        let hs: Vec<_> = (0..threads)
            .map(|_| {
                let (paths, next, links, exported, sink) = (&paths, &next, &links, &exported, &sink);
                s.spawn(move || {
                    let mut out = Out::default();
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(path) = paths.get(i) else { break };
                        let Ok(src) = String::from_utf8(std::fs::read(root.join(path)).unwrap()) else { continue };
                        let source = std::sync::Arc::new(src);
                        let Ok(facts::Built::Facts(f)) = facts::build(path, source.clone()) else { continue };
                        let tape = f.tape.as_ref().expect("tape");
                        out.files += 1;
                        let stub = path.ends_with(".pyi");
                        // The export: every variant of each distinct (content, stub reading), once.
                        if let Some(sink) = sink {
                            let hash = blake3::hash(source.as_bytes()).to_hex().to_string();
                            if exported.lock().insert((hash.clone(), stub)) {
                                let t = std::time::Instant::now();
                                let mut lines = String::new();
                                for v in Variant::ALL {
                                    let s = summary(tape, if stub { "x.pyi" } else { "x.py" }, &v.module(), f.syntax_errors, source.len());
                                    // `as_references` per links key of the reference-graph rules (both use the
                                    // same one); key 0 (no links) is compared but not exported.
                                    let refs: BTreeMap<String, Json> = links.iter().filter(|(_, key)| *key != 0).map(|(k, key)| (k.clone(), json_of(&compacted(&render(&s.as_references(*key)))))).collect();
                                    lines.push_str(&json!({"hash": hash, "variant": v.name(), "symbolic_len": v.module().segments.len(), "stub": stub, "summary": json_of(&compacted(&render(&s))), "as_references": refs}).to_string());
                                    lines.push('\n');
                                }
                                out.extract_ns.push(t.elapsed().as_nanos() as u64);
                                sink.lock().write_all(lines.as_bytes()).unwrap();
                            }
                        }
                        // The differential.
                        let Some(real) = real_module(root, path) else {
                            *out.refused.entry("no real module path").or_default() += 1;
                            continue;
                        };
                        // SCHEMA § 3.4's refusals that no name shows: a module named like the
                        // modules native compares names with, and a relative import deeper than
                        // the symbolic module.
                        if CONSTANT_MODULES.contains(&real.segments[0].as_ref()) {
                            *out.refused.entry(CONSTANT_MODULE).or_default() += 1;
                            continue;
                        }
                        let prefix = facts::lookup("import_prefix");
                        let deepest = prefix.map_or(0, |k| f.of_kind(k).iter().map(|&p| f.text(p).matches('.').count()).max().unwrap_or(0));
                        if deepest > MAX_DIRS + 1 {
                            *out.refused.entry(TOO_DEEP).or_default() += 1;
                            continue;
                        }
                        let v = Variant::of(&real);
                        *out.variants.entry(v.name()).or_default() += 1;
                        let sym_module = v.module();
                        let real_s = summary(tape, path, &real, f.syntax_errors, source.len());
                        let sym_s = summary(tape, path, &sym_module, f.syntax_errors, source.len());
                        let mut refusal = None;
                        let mut pairs = vec![("summary".to_string(), real_s.clone(), sym_s.clone())];
                        for (k, key) in links.iter() {
                            pairs.push((format!("as_references[{k}]"), real_s.as_references(*key), sym_s.as_references(*key)));
                        }
                        let mut rendered = Vec::new();
                        for (scope, r, s) in &pairs {
                            match substituted(&compacted(&render(s)), &real, sym_module.segments.len()) {
                                Ok(sub) => rendered.push((scope.clone(), render(r), sub)),
                                Err(why) => {
                                    refusal = Some(why);
                                    break;
                                }
                            }
                        }
                        if let Some(why) = refusal {
                            *out.refused.entry(why).or_default() += 1;
                            if out.refused_examples.len() < 10 {
                                out.refused_examples.push(format!("{path}: {why}"));
                            }
                            continue;
                        }
                        out.compared += 1;
                        for (scope, r, s) in &rendered {
                            compare(&mut out, scope, path, r, s);
                        }
                    }
                    out
                })
            })
            .collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    if let Some(sink) = sink {
        sink.into_inner().finish().unwrap().flush().unwrap();
    }
    let wall = t_all.elapsed().as_secs_f64();
    let mut total = Out::default();
    for o in outs {
        total.files += o.files;
        total.compared += o.compared;
        for (k, v) in o.refused {
            *total.refused.entry(k).or_default() += v;
        }
        total.refused_examples.extend(o.refused_examples);
        for (k, v) in o.variants {
            *total.variants.entry(k).or_default() += v;
        }
        for (k, v) in o.stats {
            let e = total.stats.entry(k).or_default();
            e.rows += v.rows;
            e.files_differing += v.files_differing;
            e.examples.extend(v.examples);
        }
        total.extract_ns.extend(o.extract_ns);
    }
    let differing: u64 = total.stats.values().map(|s| s.files_differing).sum();
    let res = json!({
        "files": total.files,
        "compared": total.compared,
        "wall_s": wall,
        "variants": total.variants,
        "refused": total.refused,
        "refused_examples": total.refused_examples,
        "differing_total": differing,
        "relations": total.stats.iter().map(|(k, v)| (k.clone(), json!({"rows": v.rows, "files_differing": v.files_differing, "examples": v.examples}))).collect::<BTreeMap<_, _>>(),
        "export_contents": total.extract_ns.len(),
        "export_ns_per_content_3_variants": crate::repo::summary(total.extract_ns.clone()),
        "links": links.iter().map(|(k, _)| k.clone()).collect::<Vec<_>>(),
    });
    std::fs::write(Path::new(&out_dir).join("results.json"), serde_json::to_string_pretty(&res).unwrap()).unwrap();
    println!("{}", json!({"files": total.files, "compared": total.compared, "refused": res["refused"], "differing_total": differing, "wall_s": wall}));
}
