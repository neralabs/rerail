//! What a `.rr` tape's anonymous tokens are, in each gap between named children, over every
//! `.rr` file of the repository: the evidence for which of them a named-only tape may leave out
//! (a token the grammar implies, RER-131). Ignored; run by hand:
//!
//! `cargo test -p rerail-rr-syntax --test token_census -- --ignored --nocapture`

use rerail_rr_syntax::tape::{self, FIELD_NAMES, FIRST_ANONYMOUS, KIND_NAMES};
use std::collections::BTreeMap;

fn corpus() -> Vec<(String, String)> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let (mut files, mut work) = (Vec::new(), vec![root]);
    while let Some(dir) = work.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            let skip = p.file_name().is_some_and(|n| [".git", "target", "node_modules"].contains(&n.to_string_lossy().as_ref()));
            if p.is_dir() && !skip {
                work.push(p);
            } else if p.extension().is_some_and(|x| x == "rr") {
                files.extend(std::fs::read_to_string(&p).ok().map(|t| (p.display().to_string(), t)));
            }
        }
    }
    files
}

fn kind(k: u16) -> &'static str {
    if k == tape::ERROR { "ERROR" } else { KIND_NAMES.get(k as usize).copied().unwrap_or("?") }
}

fn label(n: &tape::Node) -> String {
    let f = FIELD_NAMES.get(n.field as usize).copied().unwrap_or("");
    if f.is_empty() { kind(n.kind).to_string() } else { format!("{f}:{}", kind(n.kind)) }
}

#[test]
#[ignore]
fn token_census() {
    let files = corpus();
    // Per parent kind and gap (the named child before it, or `^`; the one after, or `$`): the
    // token words seen there, with counts.
    let mut gaps: BTreeMap<(String, String, String), BTreeMap<String, usize>> = BTreeMap::new();
    let (mut tokens, mut nodes) = (0usize, 0usize);
    for (_, src) in &files {
        let t = rerail_rr_syntax::tape(src);
        nodes += t.len();
        let mut children: Vec<Vec<u32>> = vec![Vec::new(); t.len()];
        for (i, n) in t.iter().enumerate() {
            if let Some(p) = n.parent
                && !n.extra {
                    children[p as usize].push(i as u32);
                }
        }
        for (p, kids) in children.iter().enumerate() {
            if kids.is_empty() || t[p].is_error() {
                continue;
            }
            let (mut before, mut word) = ("^".to_string(), Vec::new());
            for &c in kids {
                let n = &t[c as usize];
                if n.kind >= FIRST_ANONYMOUS && n.kind != tape::ERROR {
                    tokens += 1;
                    word.push(src[n.start as usize..n.end as usize].to_string());
                } else {
                    let after = label(n);
                    *gaps.entry((kind(t[p].kind).into(), before, after.clone())).or_default().entry(word.join(" ")).or_default() += 1;
                    before = after;
                    word.clear();
                }
            }
            *gaps.entry((kind(t[p].kind).into(), before, "$".into())).or_default().entry(word.join(" ")).or_default() += 1;
        }
    }
    println!("{} files, {nodes} nodes, {tokens} anonymous tokens", files.len());
    for ((parent, before, after), words) in &gaps {
        let words: Vec<String> = words.iter().map(|(w, n)| format!("[{w}] {n}")).collect();
        println!("{parent}\t{before}\t{after}\t{}", words.join("  "));
    }
}
