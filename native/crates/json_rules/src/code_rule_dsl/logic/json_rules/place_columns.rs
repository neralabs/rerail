//! The place types of a translated policy's columns (decision 0724 §3): the passes before write each metavariable's
//! column `place`, since a metavariable is a pattern's node (`$X`) or a regex's group (`(?P<COMMA>,)`), a match. This
//! last pass gives each such column the one type its rows hold, through every rule's head to a fixpoint: `node`,
//! `match`, or `place` where a rule puts a node and another a match. A read of a value then takes its type's name
//! (`node_line_number`, `match_text`, `place_text`, …), so no column converts what it holds.

use std::collections::{BTreeMap, BTreeSet};

/// The columns typed `node` of the library's relations, by relation: what a rule's read of them binds.
pub type NodeColumns = BTreeMap<String, BTreeSet<usize>>;

/// The library's `.decl`s' node columns, read from its files.
pub fn library_node_columns(lib: &std::path::Path) -> NodeColumns {
    let mut out = NodeColumns::new();
    let mut stack = vec![lib.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "rerail")
                && let Ok(text) = std::fs::read_to_string(&p)
            {
                for d in decls(&blank(&text)) {
                    let cols = out.entry(d.rel).or_default();
                    cols.extend(d.cols.iter().enumerate().filter(|(_, (t, _))| t == "node").map(|(i, _)| i));
                }
            }
        }
    }
    out
}

const PLACES: [&str; 4] = ["node", "line", "match", "token"];

/// Engine reads whose given column binds a node, by relation and column.
const NODE_READS: &[(&str, &[usize])] = &[
    ("node", &[1]), ("kind", &[0]), ("field", &[0, 2]), ("child", &[0, 1]), ("within", &[0, 1]), ("next_sibling", &[0, 1]),
    ("precedes", &[0, 1]), ("earlier", &[0, 1]), ("node_text", &[0]), ("end_point", &[0, 1]), ("first_token", &[0]),
    ("last_token", &[0]), ("list_len", &[0]),
];

/// A read and its name for each place type (none: the read stays as written).
const TYPED: &[(&str, [&str; 5])] = &[
    // node, line, match, token, place
    ("text", ["", "line_text", "match_text", "token_text", "place_text"]),
    ("docof", ["", "line_document", "match_document", "token_document", "place_document"]),
    ("line", ["node_line_number", "line_number", "match_line_number", "token_line_number", "place_line_number"]),
    ("end_line", ["node_end_line_number", "line_number", "match_end_line_number", "token_end_line_number", "place_end_line_number"]),
    ("col", ["node_start_column", "", "match_start_column", "token_start_column", "place_start_column"]),
];

/// Comments and `#include` lines as spaces (newlines kept), so offsets stay the text's.
fn blank(text: &str) -> String {
    let b = text.as_bytes();
    let mut out = b.to_vec();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'"' {
            i = skip_string(b, i);
        } else if b[i..].starts_with(b"//") || (b[i] == b'#' && (i == 0 || b[i - 1] == b'\n')) {
            while i < b.len() && b[i] != b'\n' {
                out[i] = b' ';
                i += 1;
            }
        } else {
            i += 1;
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| text.to_owned())
}

fn skip_string(b: &[u8], i: usize) -> usize {
    let mut j = i + 1;
    while j < b.len() && b[j] != b'"' {
        j += if b[j] == b'\\' { 2 } else { 1 };
    }
    j + 1
}

/// (start, end) of each clause or directive of blanked text.
fn statements(t: &str) -> Vec<(usize, usize)> {
    let b = t.as_bytes();
    let (mut out, mut start, mut depth, mut i) = (Vec::new(), 0, 0i32, 0);
    while i < b.len() {
        match b[i] {
            b'"' => {
                i = skip_string(b, i);
                continue;
            }
            b'(' | b'{' | b'[' => depth += 1,
            b')' | b'}' | b']' => depth -= 1,
            b'.' if depth == 0 => {
                let prev = if i > 0 { b[i - 1] } else { b' ' };
                let next = b.get(i + 1).copied().unwrap_or(b' ');
                if !(prev.is_ascii_digit() && next.is_ascii_digit()) && next.is_ascii_whitespace() {
                    if !t[start..i].trim().is_empty() {
                        out.push((start, i));
                    }
                    start = i + 1;
                }
            }
            b'\n' if depth == 0 && t[start..i].trim_start().starts_with('.') => {
                out.push((start, i));
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    out
}

/// The top-level arguments of b[a..e], each as (start, end) with its spaces trimmed.
fn args(t: &str, a: usize, e: usize) -> Vec<(usize, usize)> {
    let b = t.as_bytes();
    let (mut out, mut depth, mut i, mut s) = (Vec::new(), 0i32, a, a);
    let push = |s: usize, e: usize, out: &mut Vec<(usize, usize)>| {
        let x = &t[s..e];
        let l = s + (x.len() - x.trim_start().len());
        let r = e - (x.len() - x.trim_end().len());
        if l < r {
            out.push((l, r));
        }
    };
    while i < e {
        match b[i] {
            b'"' => {
                i = skip_string(b, i);
                continue;
            }
            b'(' | b'{' | b'[' => depth += 1,
            b')' | b'}' | b']' => depth -= 1,
            b',' if depth == 0 => {
                push(s, i, &mut out);
                s = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    push(s, e, &mut out);
    out
}

struct Atom {
    name: String,
    at: usize,
    args: Vec<String>,
    neg: bool,
}

/// Every `name(args)` in t[a..e]; functors (`@f(…)`) and the insides of strings left out.
fn atoms(t: &str, a: usize, e: usize) -> Vec<Atom> {
    let b = t.as_bytes();
    let mut out = Vec::new();
    let mut i = a;
    while i < e {
        if b[i] == b'"' {
            i = skip_string(b, i);
            continue;
        }
        let word = (b[i].is_ascii_alphabetic() || b[i] == b'_') && (i == 0 || !(b[i - 1].is_ascii_alphanumeric() || matches!(b[i - 1], b'_' | b'@' | b'.' | b'$')));
        if !word {
            i += 1;
            continue;
        }
        let mut j = i;
        while j < e && (b[j].is_ascii_alphanumeric() || b[j] == b'_') {
            j += 1;
        }
        let mut k = j;
        while k < e && b[k] == b' ' {
            k += 1;
        }
        if k >= e || b[k] != b'(' {
            i = j;
            continue;
        }
        let (mut depth, mut m) = (1, k + 1);
        while m < e && depth > 0 {
            match b[m] {
                b'"' => {
                    m = skip_string(b, m);
                    continue;
                }
                b'(' => depth += 1,
                b')' => depth -= 1,
                _ => {}
            }
            m += 1;
        }
        let mut p = i;
        while p > a && matches!(b[p - 1], b' ' | b'\t') {
            p -= 1;
        }
        let neg = p > a && b[p - 1] == b'!';
        let xs = args(t, k + 1, m.saturating_sub(1)).into_iter().map(|(s, e)| t[s..e].to_owned()).collect();
        out.push(Atom { name: t[i..j].to_owned(), at: i, args: xs, neg });
        i = k + 1;
    }
    out
}

struct Decl {
    rel: String,
    /// Each column's type as written, and where the type starts.
    cols: Vec<(String, usize)>,
}

fn decls(t: &str) -> Vec<Decl> {
    let mut out = Vec::new();
    for (a, e) in statements(t) {
        let a = a + (t[a..e].len() - t[a..e].trim_start().len());
        let s = &t[a..e];
        let Some(rest) = s.strip_prefix(".decl ").or_else(|| s.strip_prefix(".violation ")) else { continue };
        let at = a + (s.len() - rest.len());
        let at = at + (rest.len() - rest.trim_start().len());
        let rest = rest.trim_start();
        let Some(open) = rest.find('(') else { continue };
        let rel = rest[..open].trim().to_owned();
        let Some(close) = t[at + open..].find(')') else { continue };
        let cols = args(t, at + open + 1, at + open + close)
            .into_iter()
            .map(|(s, e)| match t[s..e].split_once(':') {
                Some((n, ty)) => {
                    let ty_at = s + n.len() + 1 + (ty.len() - ty.trim_start().len());
                    (ty.trim().to_owned(), ty_at)
                }
                None => (String::new(), e),
            })
            .collect();
        out.push(Decl { rel, cols });
    }
    out
}

fn is_var(a: &str) -> bool {
    a.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') && a.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// A regex literal's named groups.
fn groups(lit: &str) -> Vec<String> {
    let mut out = Vec::new();
    let b = lit.as_bytes();
    let mut i = 0;
    while let Some(k) = lit[i..].find("(?") {
        let mut j = i + k + 2;
        if b.get(j) == Some(&b'P') {
            j += 1;
        }
        if b.get(j) == Some(&b'<') && b.get(j + 1).is_some_and(|c| c.is_ascii_alphabetic() || *c == b'_') {
            let s = j + 1;
            let mut e = s;
            while e < b.len() && (b[e].is_ascii_alphanumeric() || b[e] == b'_') {
                e += 1;
            }
            if b.get(e) == Some(&b'>') {
                out.push(lit[s..e].to_owned());
            }
        }
        i = j;
    }
    out
}

struct Clause {
    heads: Vec<Atom>,
    body: Vec<Atom>,
    /// The body's text, for its equalities.
    text: String,
}

type Kinds = BTreeMap<String, BTreeSet<&'static str>>;
type Carry = BTreeMap<(String, usize), BTreeSet<&'static str>>;

fn place(k: &str) -> Option<&'static str> {
    PLACES.iter().copied().find(|p| *p == k)
}

struct Program<'a> {
    lib: &'a NodeColumns,
    /// This text's declarations: each relation's column types.
    own: BTreeMap<String, Vec<String>>,
}

impl Program<'_> {
    fn node_col(&self, rel: &str, i: usize) -> bool {
        // The text's own columns are inferred from what their heads are given (`place_columns`): a metavariable's
        // `node` column can hold a regex group's match.
        if self.own.contains_key(rel) {
            return false;
        }
        NODE_READS.iter().any(|(r, cs)| *r == rel && cs.contains(&i)) || self.lib.get(rel).is_some_and(|c| c.contains(&i))
    }

    fn kinds(&self, c: &Clause, carry: &Carry) -> Kinds {
        let mut k = Kinds::new();
        for at in c.body.iter().filter(|a| !a.neg) {
            for (i, a) in at.args.iter().enumerate() {
                if !is_var(a) {
                    continue;
                }
                if self.node_col(&at.name, i) {
                    k.entry(a.clone()).or_default().insert("node");
                } else if let Some(ks) = carry.get(&(at.name.clone(), i)) {
                    k.entry(a.clone()).or_default().extend(ks);
                }
            }
        }
        for _ in 0..3 {
            for at in &c.body {
                if at.name == "match" && at.args.len() == 2 && at.args[0].starts_with('"') && k.get(&at.args[1]).is_some_and(|s| !s.is_empty()) {
                    for g in groups(&at.args[0]) {
                        k.entry(g).or_default().insert("match");
                    }
                }
            }
            for (x, y) in equalities(&c.text) {
                let u: BTreeSet<_> = k.get(&x).into_iter().chain(k.get(&y)).flatten().copied().collect();
                k.insert(x, u.clone());
                k.insert(y, u);
            }
        }
        k
    }
}

/// `x = y` between two variables.
fn equalities(t: &str) -> Vec<(String, String)> {
    let b = t.as_bytes();
    let mut out = Vec::new();
    for (i, _) in t.match_indices('=') {
        if i == 0 || matches!(b[i - 1], b'<' | b'>' | b'!' | b'=') || b.get(i + 1) == Some(&b'=') {
            continue;
        }
        let l = t[..i].trim_end();
        let x: String = l.chars().rev().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect::<Vec<_>>().into_iter().rev().collect();
        let r = t[i + 1..].trim_start();
        let y: String = r.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
        let after = r[y.len()..].trim_start();
        let before = l[..l.len() - x.len()].chars().last();
        if is_var(&x) && is_var(&y) && !after.starts_with('(') && !before.is_some_and(|c| c == '@' || c == '.') {
            out.push((x, y));
        }
    }
    out
}

/// The translated text with each `place` column of its own declarations narrowed to the type its rows hold, and each
/// read of a value named for that value's type.
pub fn place_columns(text: String, lib: &NodeColumns) -> String {
    let t = blank(&text);
    let ds = decls(&t);
    let own: BTreeMap<String, Vec<String>> = ds.iter().map(|d| (d.rel.clone(), d.cols.iter().map(|(t, _)| t.clone()).collect())).collect();
    let prog = Program { lib, own };
    let mut clauses = Vec::new();
    for (a, e) in statements(&t) {
        if t[a..e].trim_start().starts_with('.') {
            continue;
        }
        let k = t[a..e].find(":-").map(|k| a + k);
        let heads = atoms(&t, a, k.unwrap_or(e));
        let body = k.map(|k| atoms(&t, k + 2, e)).unwrap_or_default();
        clauses.push(Clause { heads, body, text: k.map(|k| t[k + 2..e].to_owned()).unwrap_or_default() });
    }
    let mut carry = Carry::new();
    for _ in 0..40 {
        let mut changed = false;
        for c in &clauses {
            let k = prog.kinds(c, &carry);
            for h in &c.heads {
                for (i, a) in h.args.iter().enumerate() {
                    if let Some(ks) = k.get(a).filter(|_| is_var(a)) {
                        let col = carry.entry((h.name.clone(), i)).or_default();
                        let n = col.len();
                        col.extend(ks.iter().filter(|x| place(x).is_some()));
                        changed |= col.len() != n;
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }
    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    for d in &ds {
        for (i, (ty, at)) in d.cols.iter().enumerate() {
            if ty != "place" && ty != "node" {
                continue;
            }
            let ks = carry.get(&(d.rel.clone(), i)).cloned().unwrap_or_default();
            // A `node` column holds what its heads give it: a match's, or a match's and a node's (`place`).
            if ty == "node" && ks.iter().all(|k| *k == "node") {
                continue;
            }
            let new = match ks.len() {
                0 => "node",
                1 => ks.iter().next().copied().unwrap_or("node"),
                _ => "place",
            };
            if new != ty {
                edits.push((*at, at + ty.len(), new.to_owned()));
            }
        }
    }
    for c in &clauses {
        let k = prog.kinds(c, &carry);
        for at in &c.body {
            let Some((_, names)) = TYPED.iter().find(|(r, _)| *r == at.name).filter(|_| at.args.len() == 2) else { continue };
            let ks = k.get(&at.args[0]).cloned().unwrap_or_default();
            let i = match ks.len() {
                0 => 0,
                1 => PLACES.iter().position(|p| ks.contains(p)).unwrap_or(0),
                _ => 4,
            };
            if !names[i].is_empty() {
                edits.push((at.at, at.at + at.name.len(), names[i].to_owned()));
            }
        }
        for at in &c.body {
            if at.name == "end_point" && at.args.len() == 2 {
                edits.push((at.at, at.at + at.name.len(), "node_to_end_position".to_owned()));
            }
        }
    }
    edits.sort();
    edits.dedup();
    let mut out = text;
    for (a, e, rep) in edits.into_iter().rev() {
        out.replace_range(a..e, &rep);
    }
    out
}
