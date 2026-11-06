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
