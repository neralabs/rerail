//! The producer against facts-engine's X6 exports: every (path, content) of E3's replay corpus,
//! each export Y1 and the symbol table read, as a multiset and in order. Ignored (minutes, data
//! outside the repository):
//!
//! ```text
//! cargo test --profile profiling --lib --features metal-tests -- --ignored --nocapture xinputs::tests::xinputs_equal_x6_exports
//! ```
//!
//! Environment: `RERAIL_XINPUTS_JOBS` (`path \t blake3 \t source file`, default the replay's
//! jobs), `RERAIL_XINPUTS_EXPORTS` (default `replay-symbols-merged.jsonl`), `RERAIL_XINPUTS_EVERY`
//! (compare every n-th line, default 1), `RERAIL_XINPUTS_OUT` (run directory for `results.json`).
use super::exports::{READ, Scope, exports};
use super::Producer;
use rayon::prelude::*;
use serde_json::{Value as Json, json};
use std::collections::{BTreeMap, HashMap};
use std::io::BufRead;
use std::path::PathBuf;
use std::sync::Arc;

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

fn bench() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap()).join("rerail-bench/facts")
}

#[derive(Default)]
struct Tally {
    lines: usize,
    contents_failed: usize,
    failures: Vec<String>,
    /// export id → (rows want, rows got, lines whose multiset differs, lines whose order differs)
    by_id: BTreeMap<u32, [usize; 4]>,
    examples: Vec<Json>,
}

fn sorted(rows: &[Json]) -> Vec<String> {
    let mut v: Vec<String> = rows.iter().map(Json::to_string).collect();
    v.sort_unstable();
    v
}

#[test]
fn compiles_and_produces_a_small_file() {
    let p = Producer::new().unwrap();
    let src = Arc::new("import os\nfrom .a import b as c\n\nclass K(Base):\n    @property\n    def m(self, x: int) -> str:\n        return os.path.join(x)\n".to_owned());
    let image = p.image(&src).unwrap();
    let x = p.produce(Arc::clone(&src), &image).unwrap();
    let scope = Scope::new(&p);
    let e = exports(&p, &scope, "data_layer/k.py", &x);
    assert_eq!(e[&3].len(), 1);
    assert_eq!(e[&14].len(), 2, "{:?}", e[&14]);
    assert_eq!(e[&18].len(), 2, "{:?}", e[&18]);
    assert!(!e[&7].is_empty());
    assert!(exports(&p, &scope, "elsewhere/k.py", &x).values().all(Vec::is_empty));
}

#[test]
#[ignore = "differential over the replay corpus; minutes and data outside the repository (see the module docs)"]
fn xinputs_equal_x6_exports() {
    let jobs_p = env("RERAIL_XINPUTS_JOBS").map_or_else(|| bench().join("phase-2/s3-crossfile/exports/replay-jobs.tsv"), PathBuf::from);
    let exports_p = env("RERAIL_XINPUTS_EXPORTS").map_or_else(|| bench().join("explore/x6-rules/exports/replay-symbols-merged.jsonl"), PathBuf::from);
    let every: usize = env("RERAIL_XINPUTS_EVERY").map_or(1, |x| x.parse().unwrap());
    let sources: HashMap<(String, String), String> = std::fs::read_to_string(&jobs_p)
        .unwrap()
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| {
            let mut it = l.split('\t');
            let (p, h, s) = (it.next().unwrap(), it.next().unwrap(), it.next().unwrap());
            ((p.to_owned(), h.to_owned()), s.to_owned())
        })
        .collect();
    let t0 = std::time::Instant::now();
    let producer = Producer::new().unwrap();
    let scope = Scope::new(&producer);
    let reader = std::io::BufReader::with_capacity(1 << 24, std::fs::File::open(&exports_p).unwrap());
    let mut tally = Tally::default();
    let mut chunk: Vec<String> = Vec::new();
    let mut n = 0usize;
    let flush = |chunk: &mut Vec<String>, tally: &mut Tally| {
        let results: Vec<Result<(Json, Vec<(u32, [usize; 4], Option<Json>)>), String>> = chunk
            .par_iter()
            .map(|line| {
                let j: Json = serde_json::from_str(line).unwrap();
                let (path, hash) = (j["path"].as_str().unwrap(), j["hash"].as_str().unwrap());
                let src = sources.get(&(path.to_owned(), hash.to_owned())).ok_or_else(|| format!("{path} {hash}: no job"))?;
                let source = Arc::new(String::from_utf8(std::fs::read(src).unwrap()).map_err(|_| format!("{path}: not UTF-8"))?);
                let image = producer.image(&source).map_err(|e| format!("{path} {hash}: {e}"))?;
                let x = producer.produce(Arc::clone(&source), &image).map_err(|e| format!("{path} {hash}: {e}"))?;
                let got = exports(&producer, &scope, path, &x);
                let mut per = Vec::new();
                for id in READ {
                    let want = j["exports"].get(id.to_string()).and_then(Json::as_array).cloned().unwrap_or_default();
                    let mine: Vec<Json> = got[&id].iter().map(|r| Json::Array(r.clone())).collect();
                    let mine = &mine;
                    let same_set = sorted(&want) == sorted(mine);
                    let same_order = &want == mine;
                    let example = (!same_set).then(|| {
                        let (w, g) = (sorted(&want), sorted(mine));
                        let only_want: Vec<&String> = w.iter().filter(|r| !g.contains(r)).take(3).collect();
                        let only_got: Vec<&String> = g.iter().filter(|r| !w.contains(r)).take(3).collect();
                        json!({"path": path, "hash": hash, "export": id, "want": want.len(), "got": mine.len(), "only_want": only_want, "only_got": only_got})
                    });
                    per.push((id, [want.len(), mine.len(), usize::from(!same_set), usize::from(!same_order)], example));
                }
                Ok((json!([path, hash]), per))
            })
            .collect();
        for r in results {
            tally.lines += 1;
            match r {
                Err(e) => {
                    tally.contents_failed += 1;
                    if tally.failures.len() < 20 {
                        tally.failures.push(e);
                    }
                }
                Ok((_, per)) => {
                    for (id, c, ex) in per {
                        let e = tally.by_id.entry(id).or_default();
                        for k in 0..4 {
                            e[k] += c[k];
                        }
                        if let Some(ex) = ex {
                            if tally.examples.len() < 40 {
                                tally.examples.push(ex);
                            }
                        }
                    }
                }
            }
        }
        chunk.clear();
    };
    for line in reader.lines() {
        let line = line.unwrap();
        n += 1;
        if line.is_empty() || (n - 1) % every != 0 {
            continue;
        }
        chunk.push(line);
        if chunk.len() == 256 {
            flush(&mut chunk, &mut tally);
            eprintln!("xinputs: {} lines compared at {:.1}s", tally.lines, t0.elapsed().as_secs_f64());
        }
    }
    flush(&mut chunk, &mut tally);
    let differing: usize = tally.by_id.values().map(|c| c[2]).sum();
    let res = json!({
        "jobs": jobs_p, "exports": exports_p, "every": every, "lines": tally.lines, "seconds": t0.elapsed().as_secs_f64(),
        "contents_failed": tally.contents_failed, "failures": tally.failures,
        "by_export": tally.by_id.iter().map(|(id, c)| (id.to_string(), json!({"rows_x6": c[0], "rows_produced": c[1], "lines_multiset_differs": c[2], "lines_order_differs": c[3]}))).collect::<BTreeMap<_, _>>(),
        "lines_multiset_differs_total": differing,
        "examples": tally.examples,
    });
    if let Some(out) = env("RERAIL_XINPUTS_OUT") {
        std::fs::create_dir_all(&out).unwrap();
        std::fs::write(PathBuf::from(out).join("results.json"), serde_json::to_string_pretty(&res).unwrap()).unwrap();
    }
    println!("{}", serde_json::to_string_pretty(&res).unwrap());
    assert_eq!(tally.contents_failed, 0);
    assert_eq!(differing, 0);
}
