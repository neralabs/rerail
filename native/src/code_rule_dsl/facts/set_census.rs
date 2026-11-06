//! Decision 0045's measurement: what node sets look like per document chunk on a real repository.
//!
//! For every kind, and for "inside a node of kind K" for a few scope kinds, it records per document
//! (one chunk when the document has at most 2^16 nodes): the members and the runs of consecutive ids,
//! in the engine's id space (named, non-comment nodes in preorder). It then prices four forms of the
//! same sets: one dense bitmap over the whole space (0024 §1's dense form), a dense bitmap per document
//! (Metal's set steps), a sorted u32 list, and Roaring-style containers (array, bitmap or runs per
//! chunk, the smallest).
//!
//! `RERAIL_SET_CENSUS_ROOT=<dir>` names the repository; Python files only.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rayon::prelude::*;

use crate::code_rule_dsl::facts::core::{self, Built, kind_atom};
use crate::code_rule_dsl::language::{dialect_by_label, tape_tag};
use crate::code_rule_dsl::model::FileInput;

const CHUNK: u32 = 1 << 16;

/// One set's members in one document: count and runs, per chunk.
#[derive(Default, Clone, Copy)]
struct Part {
    count: u64,
    runs: u64,
}

/// Bytes of one chunk as Roaring would keep it, and which container that is.
fn container(p: Part) -> (u64, usize) {
    let array = 2 * p.count;
    let bitmap = 8192;
    let runs = 2 + 4 * p.runs;
    let best = array.min(bitmap).min(runs);
    let which = if best == runs { 2 } else if best == array { 0 } else { 1 };
    (best, which)
}

fn python_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        let name = e.file_name();
        let name = name.to_string_lossy();
        if p.is_dir() {
            if !name.starts_with('.') && name != "node_modules" {
                python_files(&p, out);
            }
        } else if name.ends_with(".py") {
            out.push(p);
        }
    }
}

/// Per document: node count, and per set (key) its parts per chunk.
struct Doc {
    nodes: u32,
    sets: HashMap<String, Vec<Part>>,
}

fn census(path: &Path, rel: &str, scopes: &[(String, u32)], names: &HashMap<u32, String>) -> Option<Doc> {
    let source = std::fs::read_to_string(path).ok()?;
    let file = FileInput::new(rel.to_owned(), "python".to_owned(), Arc::new(source), None, None, "core").ok()?;
    let Ok(Built::Facts(f)) = core::build(&file) else { return None };
    // The engine's ids: named, non-extra (comment) nodes in preorder. rank[id] = kept nodes before id.
    let n = f.kind.len();
    let keep: Vec<bool> = (0..n).map(|i| f.flags[i] & 1 == 1 && f.flags[i] & 2 == 0).collect();
    let mut rank = vec![0u32; n + 1];
    for i in 0..n {
        rank[i + 1] = rank[i] + u32::from(keep[i]);
    }
    let nodes = rank[n];
    let chunks = nodes.div_ceil(CHUNK).max(1) as usize;
    let mut sets: HashMap<String, Vec<Part>> = HashMap::new();
    let mut add = |key: String, ids: &mut dyn Iterator<Item = u32>| {
        let parts = sets.entry(key).or_insert_with(|| vec![Part::default(); chunks]);
        let mut last: Option<u32> = None;
        for id in ids {
            let c = (id / CHUNK) as usize;
            parts[c].count += 1;
            if last != Some(id.wrapping_sub(1)) || id % CHUNK == 0 {
                parts[c].runs += 1;
            }
            last = Some(id);
        }
    };
    // Kind sets: members in id order.
    let mut by_kind: HashMap<u32, Vec<u32>> = HashMap::new();
    for i in 0..n {
        if keep[i] {
            by_kind.entry(f.kind[i]).or_default().push(rank[i]);
        }
    }
    for (k, ids) in &by_kind {
        let name = names.get(k).cloned().unwrap_or_else(|| format!("kind{k}"));
        add(format!("kind:{name}"), &mut ids.iter().copied());
    }
    // Inside-sets: the union of the subtrees (strictly below) of the nodes of a scope kind.
    for (name, atom) in scopes {
        let mut intervals: Vec<(u32, u32)> = (0..n)
            .filter(|&i| keep[i] && f.kind[i] == *atom)
            .map(|i| (rank[i] + 1, rank[f.subtree_end[i] as usize]))
            .filter(|(a, b)| a < b)
            .collect();
        intervals.sort_unstable();
        let mut merged: Vec<(u32, u32)> = Vec::new();
        for (a, b) in intervals {
            match merged.last_mut() {
                Some(last) if a <= last.1 => last.1 = last.1.max(b),
                _ => merged.push((a, b)),
            }
        }
        if !merged.is_empty() {
            add(format!("inside:{name}"), &mut merged.iter().flat_map(|&(a, b)| a..b));
        }
    }
    Some(Doc { nodes, sets })
}

#[test]
#[ignore = "a measurement over a repository on disk (RERAIL_SET_CENSUS_ROOT)"]
fn set_census() {
    let root = PathBuf::from(std::env::var("RERAIL_SET_CENSUS_ROOT").expect("RERAIL_SET_CENSUS_ROOT"));
    let mut files = Vec::new();
    python_files(&root, &mut files);
    files.sort();
    let python = dialect_by_label("python").unwrap();
    let (tag, catalog) = (tape_tag(python).unwrap(), python.kinds());
    let mut names = HashMap::new();
    for code in 0..catalog.kind_count() as u16 {
        if let Some(name) = catalog.kind_name(code) {
            names.insert(kind_atom(tag, code), name.to_owned());
        }
    }
    let scopes: Vec<(String, u32)> = ["function_definition", "class_definition", "except_clause", "lambda", "with_statement", "call"]
        .iter()
        .map(|s| ((*s).to_owned(), kind_atom(tag, catalog.kind_id(s))))
        .collect();
    let docs: Vec<Doc> = files
        .par_iter()
        .filter_map(|p| census(p, &p.strip_prefix(&root).unwrap().to_string_lossy(), &scopes, &names))
        .collect();
    let total: u64 = docs.iter().map(|d| u64::from(d.nodes)).sum();
    let big = docs.iter().filter(|d| d.nodes > CHUNK).count();
    let max = docs.iter().map(|d| d.nodes).max().unwrap_or(0);
    println!("files {} parsed {} nodes {total} (max {max} per file; {big} files over one chunk)", files.len(), docs.len());

    // Per set key: totals over the repository.
    #[derive(Default)]
    struct Tot {
        members: u64,
        docs: u64,
        chunks: u64,
        roaring: u64,
        per_file_dense: u64,
        kinds: [u64; 3],
        runs: u64,
    }
    let mut tots: HashMap<String, Tot> = HashMap::new();
    for d in &docs {
        for (key, parts) in &d.sets {
            let t = tots.entry(key.clone()).or_default();
            t.docs += 1;
            t.per_file_dense += u64::from(d.nodes.div_ceil(64)) * 8;
            for p in parts.iter().filter(|p| p.count > 0) {
                let (bytes, which) = container(*p);
                t.members += p.count;
                t.runs += p.runs;
                t.chunks += 1;
                t.roaring += bytes + 8; // + a directory entry
                t.kinds[which] += 1;
            }
        }
    }
    let dense_space = total.div_ceil(8);
    let mut keys: Vec<&String> = tots.keys().collect();
    keys.sort_by_key(|k| std::cmp::Reverse(tots[*k].members));
    let mut all = Tot::default();
    println!("\nset\tmembers\tdocs\tchunks array/bitmap/runs\truns\tdense(space) B\tdense(per file) B\tu32 list B\troaring B");
    for (i, k) in keys.iter().enumerate() {
        let t = &tots[*k];
        all.members += t.members;
        all.docs += t.docs;
        all.chunks += t.chunks;
        all.roaring += t.roaring;
        all.per_file_dense += t.per_file_dense;
        all.runs += t.runs;
        for j in 0..3 {
            all.kinds[j] += t.kinds[j];
        }
        if i < 25 || k.starts_with("inside:") {
            println!(
                "{k}\t{}\t{}\t{}/{}/{}\t{}\t{dense_space}\t{}\t{}\t{}",
                t.members, t.docs, t.kinds[0], t.kinds[1], t.kinds[2], t.runs, t.per_file_dense, 4 * t.members, t.roaring
            );
        }
    }
    let sets = tots.len() as u64;
    println!(
        "\nall {sets} sets: members {} chunks {} (array {} bitmap {} runs {}) runs {}\n  dense over the space: {} MB\n  dense per file (present files only): {} MB\n  u32 lists: {} MB\n  roaring: {} MB",
        all.members,
        all.chunks,
        all.kinds[0],
        all.kinds[1],
        all.kinds[2],
        all.runs,
        sets * dense_space / 1_000_000,
        all.per_file_dense / 1_000_000,
        4 * all.members / 1_000_000,
        all.roaring / 1_000_000
    );
    // How sparse sets are per document: the share of (set, document) pairs that are empty, which a
    // chunked operation skips and a dense one does not.
    let pairs = sets * docs.len() as u64;
    println!("  (set, document) pairs with members: {} of {pairs} ({:.1}%)", all.docs, 100.0 * all.docs as f64 / pairs as f64);
}

/// One document's sets as positions: kind members and inside-scope runs.
struct DocSets {
    nodes: u32,
    kinds: HashMap<String, Vec<u32>>,
    inside: HashMap<String, Vec<(u32, u32)>>,
}

fn doc_sets(path: &Path, rel: &str, scopes: &[(String, u32)], names: &HashMap<u32, String>) -> Option<DocSets> {
    let source = std::fs::read_to_string(path).ok()?;
    let file = FileInput::new(rel.to_owned(), "python".to_owned(), Arc::new(source), None, None, "core").ok()?;
    let Ok(Built::Facts(f)) = core::build(&file) else { return None };
    let n = f.kind.len();
    let keep: Vec<bool> = (0..n).map(|i| f.flags[i] & 1 == 1 && f.flags[i] & 2 == 0).collect();
    let mut rank = vec![0u32; n + 1];
    for i in 0..n {
        rank[i + 1] = rank[i] + u32::from(keep[i]);
    }
    let mut kinds: HashMap<String, Vec<u32>> = HashMap::new();
    for i in (0..n).filter(|&i| keep[i]) {
        let name = names.get(&f.kind[i]).cloned().unwrap_or_default();
        kinds.entry(name).or_default().push(rank[i]);
    }
    let mut inside = HashMap::new();
    for (name, atom) in scopes {
        let mut iv: Vec<(u32, u32)> =
            (0..n).filter(|&i| keep[i] && f.kind[i] == *atom).map(|i| (rank[i] + 1, rank[f.subtree_end[i] as usize])).filter(|(a, b)| a < b).collect();
        iv.sort_unstable();
        let mut merged: Vec<(u32, u32)> = Vec::new();
        for (a, b) in iv {
            match merged.last_mut() {
                Some(last) if a <= last.1 => last.1 = last.1.max(b),
                _ => merged.push((a, b)),
            }
        }
        inside.insert(name.clone(), merged);
    }
    Some(DocSets { nodes: rank[n], kinds, inside })
}

fn thread_cpu() -> f64 {
    let mut t = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: a valid out-pointer for the calling thread's clock.
    unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut t) };
    t.tv_sec as f64 + t.tv_nsec as f64 * 1e-9
}

/// Decision 0045 step 2: the operations the prefilter's set programs run, on the dense sets (0024)
/// and on the containers (`space::NodeSet`), over the same repository. Thread CPU time, building
/// excluded; every result is checked equal between the two.
#[test]
#[ignore = "a measurement over a repository on disk (RERAIL_SET_CENSUS_ROOT)"]
fn set_operations() {
    let root = PathBuf::from(std::env::var("RERAIL_SET_CENSUS_ROOT").expect("RERAIL_SET_CENSUS_ROOT"));
    let mut files = Vec::new();
    python_files(&root, &mut files);
    files.sort();
    let python = dialect_by_label("python").unwrap();
    let (tag, catalog) = (tape_tag(python).unwrap(), python.kinds());
    let mut names = HashMap::new();
    for code in 0..catalog.kind_count() as u16 {
        if let Some(name) = catalog.kind_name(code) {
            names.insert(kind_atom(tag, code), name.to_owned());
        }
    }
    let scopes: Vec<(String, u32)> =
        ["function_definition", "class_definition", "except_clause"].iter().map(|s| ((*s).to_owned(), kind_atom(tag, catalog.kind_id(s)))).collect();
    let docs: Vec<DocSets> = files.par_iter().filter_map(|p| doc_sets(p, &p.strip_prefix(&root).unwrap().to_string_lossy(), &scopes, &names)).collect();
    // One space over the repository, and the runner's waves: files in order, up to 2^18 nodes each.
    let mut waves: Vec<std::ops::Range<usize>> = Vec::new();
    let (mut from, mut nodes) = (0, 0u32);
    for (at, d) in docs.iter().enumerate() {
        if nodes > 0 && nodes + d.nodes > 1 << 18 {
            waves.push(from..at);
            (from, nodes) = (at, 0);
        }
        nodes += d.nodes;
    }
    waves.push(from..docs.len());
    for (label, spans) in [("one space", vec![0..docs.len()]), ("waves of 2^18 nodes", waves)] {
        println!("\n{label}: {} spaces, files {} nodes {}", spans.len(), docs.len(), docs.iter().map(|d| u64::from(d.nodes)).sum::<u64>());
        operations(&docs, &spans);
    }
}

type Pair = (crate::code_rule_dsl::space::DenseSet, crate::code_rule_dsl::space::NodeSet);

/// The operations on each space of `spans` (files of `docs`), summed over the spaces.
fn operations(docs: &[DocSets], spans: &[std::ops::Range<usize>]) {
    use crate::code_rule_dsl::space::{DenseSet, NodeSet, NodeSpace};
    let names = ["call", "identifier", "raise_statement", "await", "decorator", "inside:function_definition", "inside:class_definition", "inside:except_clause"];
    let built: Vec<(NodeSpace, Vec<Pair>)> = spans
        .iter()
        .map(|span| {
            let docs = &docs[span.clone()];
            let space = NodeSpace::new(docs.iter().map(|d| d.nodes).collect());
            let sets = names
                .iter()
                .map(|name| {
                    let (mut dense, mut chunked) = (DenseSet::empty(&space), NodeSet::empty(&space));
                    for (f, d) in docs.iter().enumerate() {
                        if let Some(scope) = name.strip_prefix("inside:") {
                            let runs = &d.inside[scope];
                            runs.iter().for_each(|&(a, b)| dense.fill(space.base(f) + a..space.base(f) + b));
                            chunked.insert_runs(f, runs.iter().map(|&(a, b)| a..b));
                        } else if let Some(ps) = d.kinds.get(*name) {
                            ps.iter().for_each(|&p| dense.insert(space.base(f) + p));
                            chunked.insert_file(f, ps.iter().copied());
                        }
                    }
                    (dense, chunked)
                })
                .collect();
            (space, sets)
        })
        .collect();
    let at = |name: &str| names.iter().position(|n| *n == name).unwrap();
    println!("op\tresult\tdense ms\tcontainers ms\tdense/containers");
    let rounds = 5;
    let time = |label: &str, dense_op: &dyn Fn(&NodeSpace, &[Pair]) -> DenseSet, chunked_op: &dyn Fn(&NodeSpace, &[Pair]) -> NodeSet| {
        let mut members = 0;
        for (space, sets) in &built {
            let (d, c) = (dense_op(space, sets), chunked_op(space, sets));
            assert_eq!(&NodeSet::from_words(space, d.words()), &c, "{label}");
            members += c.len();
        }
        let t = thread_cpu();
        (0..rounds).for_each(|_| built.iter().for_each(|(space, sets)| drop(std::hint::black_box(dense_op(space, sets)))));
        let dense = (thread_cpu() - t) / f64::from(rounds) * 1e3;
        let t = thread_cpu();
        (0..rounds).for_each(|_| built.iter().for_each(|(space, sets)| drop(std::hint::black_box(chunked_op(space, sets)))));
        let chunked = (thread_cpu() - t) / f64::from(rounds) * 1e3;
        println!("{label}\t{members}\t{dense:.2}\t{chunked:.2}\t{:.2}", dense / chunked);
    };
    for (a, b) in [
        ("call", "inside:function_definition"),
        ("raise_statement", "inside:except_clause"),
        ("await", "inside:function_definition"),
        ("identifier", "inside:class_definition"),
        ("decorator", "call"),
    ] {
        let (a_, b_) = (at(a), at(b));
        time(&format!("{a} ∩ {b}"), &|_, s| { let mut x = s[a_].0.clone(); x.intersect_with(&s[b_].0); x }, &|_, s| { let mut x = s[a_].1.clone(); x.intersect_with(&s[b_].1); x });
        time(&format!("{a} − {b}"), &|_, s| { let mut x = s[a_].0.clone(); x.difference_with(&s[b_].0); x }, &|_, s| { let mut x = s[a_].1.clone(); x.difference_with(&s[b_].1); x });
    }
    for (a, b) in [("call", "await"), ("raise_statement", "decorator"), ("inside:function_definition", "inside:class_definition")] {
        let (a_, b_) = (at(a), at(b));
        time(&format!("{a} ∪ {b}"), &|_, s| { let mut x = s[a_].0.clone(); x.union_with(&s[b_].0); x }, &|_, s| { let mut x = s[a_].1.clone(); x.union_with(&s[b_].1); x });
    }
    for (a, b) in [("call", "raise_statement"), ("identifier", "await")] {
        let (a_, b_) = (at(a), at(b));
        time(&format!("{a} within files of {b}"), &|sp, s| { let mut x = s[a_].0.clone(); x.within_files(sp, &[&s[b_].0]); x }, &|sp, s| { let mut x = s[a_].1.clone(); x.within_files(sp, &[&s[b_].1]); x });
    }
    let c = at("call");
    time("clone call", &|_, s| s[c].0.clone(), &|_, s| s[c].1.clone());
    println!("set\tdense B\tcontainers B");
    for (i, name) in names.iter().enumerate() {
        let dense: usize = built.iter().map(|(space, _)| space.words() * 8).sum();
        let chunked: usize = built.iter().map(|(_, sets)| sets[i].1.bytes()).sum();
        println!("{name}\t{dense}\t{chunked}");
    }
}
