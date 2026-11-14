//! The named-only tape's rule (RER-131's) on every `.rr` file of the repository. A token is a
//! **delimiter** (a bracket, `,`, `;`, `|`, `<`, `>`, a quote) or not. A delimiter is left out
//! where its gap is implied. Any other token (a keyword, an operator, `:`, `.`, `=>`) is left out
//! only where its parent's kind implies it in every gap it can stand in: under one parent kind,
//! it is always stored or never.
//!
//! The test lexes each file, finds each token's parent (the deepest node holding it) and whether
//! the tape stores it, and checks the second half: no token other than a delimiter is stored
//! under a kind in one place and left out under it in another.

use rerail_rr_syntax::syntax::lex;
use rerail_rr_syntax::tape::{self, FIRST_ANONYMOUS, KIND_NAMES};
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
    files.sort();
    files
}

/// RER-131's delimiters (`tape/tokens.rs`, `is_delimiter`).
fn is_delimiter(text: &str) -> bool {
    matches!(text, "(" | ")" | "[" | "]" | "{" | "}" | "<" | ">" | "," | ";" | "|" | "\"" | "'" | "`")
}

/// A token under its parent: its text, its start, and whether the tape stores it.
struct Tok {
    text: String,
    start: u32,
    stored: bool,
}

/// One gap of one node: its key (the parent's kind, the named child before it or `^`, the one
/// after or `$`), its tokens, and a place in the corpus.
struct Gap {
    key: (String, String, String),
    toks: Vec<Tok>,
    at: String,
}

fn label(n: &tape::Node) -> String {
    let f = tape::FIELD_NAMES.get(n.field as usize).copied().unwrap_or("");
    let k = KIND_NAMES[n.kind as usize];
    if f.is_empty() { k.to_owned() } else { format!("{f}:{k}") }
}

/// Every gap of every node, over the corpus (files with an ERROR left out: it keeps all).
fn gaps() -> Vec<Gap> {
    let mut out = Vec::new();
    for (path, src) in corpus() {
        let t = rerail_rr_syntax::engine_tape(&src);
        if t.iter().any(tape::Node::is_error) {
            continue;
        }
        let lexed = lex(&src);
        // Per node: its named children (start, label) and its tokens.
        let mut named: Vec<Vec<(u32, String)>> = vec![Vec::new(); t.len()];
        let mut toks: Vec<Vec<Tok>> = (0..t.len()).map(|_| Vec::new()).collect();
        for (i, n) in t.iter().enumerate() {
            if let Some(p) = n.parent
                && !n.extra && n.kind < FIRST_ANONYMOUS {
                    named[p as usize].push((n.start, label(n)));
                }
            let _ = i;
        }
        // The nodes on the path from the root to the last node started, by preorder.
        let (mut path_nodes, mut next): (Vec<usize>, usize) = (Vec::new(), 0);
        for &(_, start, end) in &lexed.toks {
            let (start, end) = (start as u32, end as u32);
            while next < t.len() && t[next].start <= start {
                if !t[next].extra {
                    while path_nodes.last().is_some_and(|&p| Some(p as u32) != t[next].parent) {
                        path_nodes.pop();
                    }
                    path_nodes.push(next);
                }
                next += 1;
            }
            let Some(&holder) = path_nodes.iter().rev().find(|&&p| t[p].start <= start && end <= t[p].end) else { continue };
            let h = &t[holder];
            let text = src[start as usize..end as usize].to_owned();
            let (parent, stored) = if h.start == start && h.end == end {
                if h.kind < FIRST_ANONYMOUS {
                    // A named token (an identifier, a string …).
                    continue;
                }
                (h.parent.expect("a token has a parent") as usize, true)
            } else if named[holder].is_empty() && toks[holder].iter().all(|t| !t.stored) && !matches!(KIND_NAMES[h.kind as usize], "argument_list" | "column_list" | "parenthesized_expression" | "group" | "io_relation" | "type" | "aggregate") {
                // Inside a leaf's own text (a `raw_text`, a `json_object`).
                continue;
            } else {
                (holder, false)
            };
            toks[parent].push(Tok { text, start, stored });
        }
        for (p, mut tk) in toks.into_iter().enumerate() {
            // An empty gap is a word too (the shortest), but only where a node has any.
            if tk.is_empty() && named[p].is_empty() {
                continue;
            }
            tk.sort_by_key(|t| t.start);
            let kind = KIND_NAMES[t[p].kind as usize].to_owned();
            let kids = &named[p];
            let mut tk = tk.into_iter().peekable();
            let mut before = "^".to_owned();
            for (at, after) in kids.iter().map(|(s, l)| (*s, l.clone())).chain(std::iter::once((u32::MAX, "$".to_owned()))) {
                let mut g = Vec::new();
                while tk.peek().is_some_and(|x| x.start < at) {
                    g.push(tk.next().expect("peeked"));
                }
                {
                    let line = src[..g.first().map_or(t[p].start, |x| x.start) as usize].matches('\n').count() + 1;
                    let _ = line;
                    let line = src[..g.first().map_or(t[p].start, |x| x.start) as usize].matches('\n').count() + 1;
                    out.push(Gap { key: (kind.clone(), before.clone(), after.clone()), toks: g, at: format!("{path}:{line}") });
                }
                before = after;
            }
        }
    }
    out
}

/// What the corpus cannot show, from the grammar (`21-named-only.bitlex`): a setting's arguments
/// and a declaration's or a functor's modifiers may be any punctuation (`punctuation`,
/// `setting_punctuation`), or none. So the gaps around them imply the empty word, and every
/// punctuation but a delimiter is kept under those kinds.
const FREE_KINDS: [&str; 3] = ["setting", "declaration", "functor_declaration"];
const PUNCTUATION: [&str; 32] = ["(", ")", ",", ".", ";", ":", "=", "<", ">", "!", "|", "@", "*", "[", "]", "-", "+", "/", "%", "~", "^", "&", "{", "}", ":-", "=>", "!=", "<=", ">=", "<:", "->", "#include"];

fn free(key: &(String, String, String)) -> bool {
    let around = |l: &str| l.starts_with("argument:") || l.starts_with("modifier:");
    FREE_KINDS.contains(&key.0.as_str())
        && (around(&key.1) || around(&key.2) || (key.0 != "setting" && key.2 == "$" && (key.1.starts_with("columns:") || key.1.starts_with("return_type:"))) || (key.0 == "setting" && key.2 == "$" && key.1.starts_with("name:")))
}

fn word(g: &Gap) -> String {
    g.toks.iter().map(|t| t.text.as_str()).collect::<Vec<_>>().join(" ")
}

#[test]
fn the_tape_follows_the_named_only_rule() {
    let gaps = gaps();
    // Each gap's words, and its implied word: the only shortest seen there.
    let mut words: BTreeMap<&(String, String, String), BTreeMap<String, usize>> = BTreeMap::new();
    for g in &gaps {
        *words.entry(&g.key).or_default().entry(word(g)).or_default() += 1;
    }
    let implied: BTreeMap<&(String, String, String), Option<String>> = words
        .iter()
        .map(|(k, ws)| {
            let len = |w: &str| if w.is_empty() { 0 } else { w.split(' ').count() };
            let least = ws.keys().map(|w| len(w)).min().expect("a word");
            let shortest: Vec<&String> = ws.keys().filter(|w| len(w) == least).collect();
            (*k, if free(k) { Some(String::new()) } else { (shortest.len() == 1).then(|| shortest[0].clone()) })
        })
        .collect();
    // Per kind, the tokens other than delimiters a gap holds beside a choice: kept everywhere.
    let mut kept: BTreeMap<&str, std::collections::BTreeSet<&str>> = BTreeMap::new();
    for k in FREE_KINDS {
        kept.entry(k).or_default().extend(PUNCTUATION.iter().filter(|p| !is_delimiter(p)));
    }
    for g in &gaps {
        if implied[&g.key].as_deref() != Some(word(g).as_str()) {
            kept.entry(g.key.0.as_str()).or_default().extend(g.toks.iter().map(|t| t.text.as_str()).filter(|t| !is_delimiter(t)));
        }
    }
    let mut wrong: BTreeMap<String, (usize, String)> = BTreeMap::new();
    for g in &gaps {
        let whole = implied[&g.key].as_deref() != Some(word(g).as_str());
        let want: Vec<bool> = g.toks.iter().map(|t| whole || kept.get(g.key.0.as_str()).is_some_and(|k| k.contains(t.text.as_str()))).collect();
        let got: Vec<bool> = g.toks.iter().map(|t| t.stored).collect();
        if want != got {
            let show = |v: &[bool]| g.toks.iter().zip(v).map(|(t, &s)| if s { t.text.clone() } else { format!("({})", t.text) }).collect::<Vec<_>>().join(" ");
            let e = wrong.entry(format!("{} {}..{}: want {}, got {}", g.key.0, g.key.1, g.key.2, show(&want), show(&got))).or_insert((0, g.at.clone()));
            e.0 += 1;
        }
    }
    if std::env::var_os("RR_TOKEN_RULE_ALL").is_some() {
        for (k, ws) in &words {
            println!("{}\t{}\t{}\t{:?}\t{:?}", k.0, k.1, k.2, implied[k], ws);
        }
        println!("kept {kept:?}");
    }
    let lines: Vec<String> = wrong.iter().map(|(k, (n, at))| format!("{k} ({n}, as at {at})")).collect();
    assert!(lines.is_empty(), "{} gaps break the rule (stored tokens bare, left out in parentheses):\n{}", lines.len(), lines.join("\n"));
}
