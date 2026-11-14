//! Text helpers of the translator that the passes share: masking strings and patterns, the variables a body
//! names or binds, calls of a relation, splitting at top-level separators, and `wrap` (a rule's lines).
//! Offsets are byte offsets: a masked text keeps every byte's place.

use super::patterns::Patterns;
use super::pyre::{captures, esc, findall, g, re, sub};
use fancy_regex::Regex;
use std::collections::BTreeSet;

pub type Set = BTreeSet<String>;


fn spaces(n: usize) -> String {
    " ".repeat(n)
}

/// `_mask_ticks`: backtick patterns blanked, every byte kept in place.
pub fn mask_ticks(px: &Patterns, s: &str) -> String {
    sub(&px.text_ticks, s, |c| spaces(g(c, 0).len()))
}

/// `_strip_strings`: each string literal as `""`.
pub fn strip_strings(px: &Patterns, s: &str) -> String {
    sub(&px.text_string, s, |_| "\"\"".to_owned())
}

/// `_strip_strings_keep`: each string literal blanked inside its quotes, every byte kept in place.
pub fn strip_strings_keep(px: &Patterns, s: &str) -> String {
    sub(&px.text_string, s, |c| format!("\"{}\"", spaces(g(c, 0).len() - 2)))
}

/// `_mask_ticks(_strip_strings_keep(s))`.
pub fn masked(px: &Patterns, s: &str) -> String {
    mask_ticks(px, &strip_strings_keep(px, s))
}

/// The metavariables a text names (outside strings and patterns, not called).
pub fn upper_vars(px: &Patterns, s: &str) -> Set {
    findall(&px.text_upper, &mask_ticks(px, &strip_strings(px, s))).into_iter().collect()
}

pub fn mv_refs(px: &Patterns, s: &str) -> Set {
    findall(&px.text_mv_ref, s).into_iter().collect()
}

/// `_binders`: metavariables a rule body binds itself (in a pattern, by a capture, by a named group), not inside an
/// aggregate; with `inside`, those the aggregates bind.
pub fn binders(px: &Patterns, s: &str, inside: bool) -> Set {
    if inside {
        let mut out = Set::new();
        for (a, b) in aggregate_spans(px, s) {
            out.extend(binders(px, &s[a..b].replace(['{', '}'], " "), false));
        }
        return out;
    }
    let mut out = Set::new();
    let whole = s;
    let s = drop_aggregates(px, s);
    for b in findall(&px.text_bind_arg, whole) {
        out.extend(findall(&px.text_bind_key, &b));
    }
    for c in captures(&px.text_pattern, &s) {
        out.extend(findall(&px.text_mv_ref, g(&c, 2)));
    }
    out.extend(findall(&px.text_capture, &mask_ticks(px, &strip_strings(px, &s))));
    out.extend(findall(&px.text_named_group, &s));
    out
}

/// `_drop_aggregates`: s with its aggregates' bodies blanked.
pub fn drop_aggregates(px: &Patterns, s: &str) -> String {
    let mut s = s.to_owned();
    for (a, b) in aggregate_spans(px, &s).into_iter().rev() {
        s = format!("{}{}{}", &s[..a], spaces(b - a), &s[b..]);
    }
    s
}

/// `_aggregate_spans`: the outermost aggregate bodies, as (start, end) offsets.
pub fn aggregate_spans(px: &Patterns, s: &str) -> Vec<(usize, usize)> {
    let m = masked(px, s);
    let mb = m.as_bytes();
    let mut spans: Vec<(usize, usize)> = Vec::new();
    for c in captures(&px.text_agg_start, &m) {
        let whole = c.get(0).expect("group 0");
        if spans.last().is_some_and(|l| whole.start() < l.1) {
            continue;
        }
        let mut d = 0i32;
        for (j, &ch) in mb.iter().enumerate().skip(whole.end() - 1) {
            d += (ch == b'{') as i32;
            d -= (ch == b'}') as i32;
            if d == 0 {
                spans.push((whole.end(), j));
                break;
            }
        }
    }
    spans
}

/// `_calls`: spans (start, index of the closing paren) of calls to relation `name` (outside ticks and strings).
pub fn calls(px: &Patterns, s: &str, name: &str) -> Vec<(usize, usize)> {
    let m = masked(px, s);
    let mb = m.as_bytes();
    // `re.finditer(r"(?<![\w.])" + name + r"\(", masked)`, searched for directly: a pattern per call was most of a
    // large rule's translation (its compile).
    let needle = format!("{name}(");
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(k) = m[from..].find(&needle) {
        let start = from + k;
        from = start + needle.len();
        if m[..start].chars().next_back().is_some_and(|c| super::pyre::is_word(c) || c == '.') {
            from = start + m[start..].chars().next().map_or(1, char::len_utf8);
            continue;
        }
        let mut d = 0i32;
        for (j, &ch) in mb.iter().enumerate().skip(start + needle.len() - 1) {
            if ch == b'(' {
                d += 1;
            } else if ch == b')' {
                d -= 1;
                if d == 0 {
                    out.push((start, j));
                    break;
                }
            }
        }
    }
    out
}

/// The relations a text calls (`(?<![\w.])([a-z_]\w*)\(` over the masked text).
pub fn called(px: &Patterns, s: &str) -> Vec<String> {
    findall(&px.text_called, &masked(px, s))
}

/// `_split_args`: s split at top-level commas (quotes and ticks kept whole), each stripped; always one part more
/// than separators.
pub fn split_args(s: &str) -> Vec<String> {
    let (mut out, mut d, mut cur, mut ins) = (Vec::new(), 0i32, String::new(), None::<char>);
    for ch in s.chars() {
        if let Some(q) = ins {
            cur.push(ch);
            if ch == q {
                ins = None;
            }
            continue;
        }
        if ch == '"' || ch == '`' {
            ins = Some(ch);
        } else if "([{".contains(ch) {
            d += 1;
        } else if ")]}".contains(ch) {
            d -= 1;
        }
        if ch == ',' && d == 0 {
            out.push(cur.trim().to_owned());
            cur.clear();
        } else {
            cur.push(ch);
        }
    }
    out.push(cur.trim().to_owned());
    out
}

/// `_rename_vars`: variables renamed by `mapping` outside string and pattern literals.
pub fn rename_vars(px: &Patterns, text: &str, mapping: &[(String, String)]) -> String {
    if mapping.is_empty() {
        return text.to_owned();
    }
    let mut names: Vec<&String> = mapping.iter().map(|(k, _)| k).collect();
    names.sort_by_key(|k| std::cmp::Reverse(k.chars().count()));
    let alt = names.iter().map(|k| esc(k)).collect::<Vec<_>>().join("|");
    let pat = re(&format!(r#"(?<![\w$@"])({alt})\b(?!\s*\()"#));
    super::pyre::split1(&px.text_pieces, text)
        .into_iter()
        .map(|p| if p.starts_with('"') || p.starts_with('`') { p } else { sub(&pat, &p, |c| mapping.iter().find(|(k, _)| k == g(c, 1)).map(|(_, v)| v.clone()).unwrap_or_default()) })
        .collect()
}

/// `split_alts`: the top-level ` ; `-separated alternatives of a disjunction body.
pub fn split_alts(s: &str) -> Vec<String> {
    let b = s.as_bytes();
    let (mut out, mut d, mut start, mut ins) = (Vec::new(), 0i32, 0usize, false);
    for i in 0..b.len() {
        let ch = b[i];
        if ch == b'"' && (i == 0 || b[i - 1] != b'\\') {
            ins = !ins;
        } else if !ins && (ch == b'(' || ch == b'{') {
            d += 1;
        } else if !ins && (ch == b')' || ch == b'}') {
            d -= 1;
        } else if !ins && d == 0 && s[i..].starts_with(" ; ") {
            out.push(if start <= i { s[start..i].trim().to_owned() } else { String::new() });
            start = i + 3;
        }
    }
    out.push(s.get(start..).unwrap_or("").trim().to_owned());
    out
}

/// Python's `s[a:-b]` over characters.
pub fn chars_slice(s: &str, a: usize, b: usize) -> String {
    let n = s.chars().count();
    if a + b >= n { String::new() } else { s.chars().skip(a).take(n - a - b).collect() }
}

fn clen(s: &str) -> usize {
    s.chars().count()
}

/// `wrap`: a rule's text, lines filled to `width` (88), a long disjunction one alternative per line.
pub fn wrap(head: &str, lits: &[String], edits: Option<&[String]>) -> String {
    let width = 88;
    let edits = edits.filter(|e| !e.is_empty());
    if lits.is_empty() {
        return format!("{head}{}", edits.map_or(".".to_owned(), |e| format!(" => {}.", e.join(", "))));
    }
    let mut lits: Vec<String> = lits.to_vec();
    if let Some(e) = edits {
        let last = lits.pop().expect("a literal");
        lits.push(format!("{last} => {}", e.join(", ")));
    }
    let out = format!("{head} :- ");
    let mut line = out.clone();
    let mut lines: Vec<String> = Vec::new();
    let n = lits.len();
    for (i, lit) in lits.iter().enumerate() {
        let piece = format!("{lit}{}", if i < n - 1 { "," } else { "." });
        if clen(&piece) > width - 4 && lit.starts_with('(') && lit.contains(" ; ") {
            let alts = split_alts(&chars_slice(lit, 1, 1));
            if alts.len() > 1 && lit.ends_with(')') {
                lines.push(line.trim_end().to_owned());
                lines.push(format!("    ( {}", alts[0]));
                lines.extend(alts[1..].iter().map(|a| format!("    ; {a}")));
                lines.push(format!("    ){}", &piece[lit.len()..]));
                line = "    ".to_owned();
                continue;
            }
        }
        if clen(&line) + clen(&piece) + 1 > width && line.trim() != format!("{head} :-") {
            lines.push(line.trim_end().to_owned());
            line = format!("    {piece} ");
        } else {
            line.push_str(&piece);
            line.push(' ');
        }
    }
    lines.push(line.trim_end().to_owned());
    lines.join("\n")
}

/// `_balanced`: the brackets of s balance, and the first one opened closes at the end.
pub fn balanced(s: &str) -> bool {
    let b = s.as_bytes();
    let (mut d, mut instr) = (0i32, false);
    for i in 0..b.len() {
        let ch = b[i];
        if ch == b'"' && (i == 0 || b[i - 1] != b'\\') {
            instr = !instr;
        }
        if instr {
            continue;
        }
        if b"({[".contains(&ch) {
            d += 1;
        } else if b")}]".contains(&ch) {
            d -= 1;
            if d == 0 && i != b.len() - 1 {
                return false;
            }
        }
    }
    d == 0
}


/// `is_atom`: `name(…)`, one balanced call.
pub fn is_atom(px: &Patterns, s: &str) -> bool {
    super::pyre::search(&px.text_atom_start, s) && s.ends_with(')') && balanced(&s[s.find('(').expect("a paren")..])
}

/// `disj_atoms`: the branches of `(a ; b ; c)` when each is a single atom.
pub fn disj_atoms(px: &Patterns, s: &str) -> Option<Vec<String>> {
    if !(s.starts_with('(') && s.ends_with(')') && balanced(s)) {
        return None;
    }
    let inner = &s[1..s.len() - 1];
    let (mut parts, mut d, mut cur, mut instr) = (Vec::new(), 0i32, String::new(), false);
    for ch in inner.chars() {
        if ch == '"' {
            instr = !instr;
        }
        if !instr {
            if "({[".contains(ch) {
                d += 1;
            } else if ")}]".contains(ch) {
                d -= 1;
            } else if ch == ';' && d == 0 {
                parts.push(cur.trim().to_owned());
                cur.clear();
                continue;
            }
        }
        cur.push(ch);
    }
    parts.push(cur.trim().to_owned());
    (parts.len() > 1 && parts.iter().all(|p| is_atom(px, p))).then_some(parts)
}

/// `head_span_ok`: t's first `(` closes at its last character.
pub fn head_span_ok(px: &Patterns, t: &str) -> bool {
    let m = masked(px, t);
    let b = m.as_bytes();
    let mut d = 0i32;
    for (i, &ch) in b.iter().enumerate() {
        d += (ch == b'(') as i32;
        d -= (ch == b')') as i32;
        if d == 0 {
            return i == b.len() - 1;
        }
    }
    false
}

/// `_split_top_text`: s split at top-level `sep` (quotes, ticks and brackets kept whole), stripped, empties dropped.
pub fn split_top_text(s: &str, sep: char) -> Vec<String> {
    let cs: Vec<char> = s.chars().collect();
    let (mut out, mut d, mut cur, mut ins) = (Vec::new(), 0i32, String::new(), None::<char>);
    for i in 0..cs.len() {
        let ch = cs[i];
        if let Some(q) = ins {
            cur.push(ch);
            if ch == q && cs[i - 1] != '\\' {
                ins = None;
            }
            continue;
        }
        if ch == '"' || ch == '`' {
            ins = Some(ch);
        } else if "([{".contains(ch) {
            d += 1;
        } else if ")]}".contains(ch) {
            d -= 1;
        }
        if ch == sep && d == 0 {
            out.push(cur.trim().to_owned());
            cur.clear();
        } else {
            cur.push(ch);
        }
    }
    out.push(cur.trim().to_owned());
    out.into_iter().filter(|x| !x.is_empty()).collect()
}

const READS: [&str; 6] = ["text(", "value(", "line(", "end_line(", "col(", "@doc("];

fn is_top(l: &str) -> bool {
    !l.starts_with('(') && !l.starts_with('!') && !l.contains(" : {")
}

/// `bind_read_nodes`: a metavariable the body's own positive literals only read is bound first by `node(_, X, …)`.
pub fn bind_read_nodes(px: &Patterns, lits: Vec<String>) -> Vec<String> {
    let top: Vec<String> = lits.iter().filter(|l| is_top(l)).map(|l| strip_strings_keep(px, l)).collect();
    let joined = top.iter().map(|t| mask_ticks(px, t)).collect::<Vec<_>>().join(" ");
    let xs: Set = findall(&px.text_top_upper, &joined).into_iter().collect();
    let mut add = Vec::new();
    for x in xs {
        let ex = esc(&x);
        let use_re = re(&format!(r"(@?\w*\()?(?<!\w)\$*{ex}\b(?!\s*\()"));
        let read_re = re(&format!(r"(?<![\w$@])(?:text|value)\({ex}\b"));
        let uses: Vec<String> = top.iter().flat_map(|t| findall(&use_re, t)).collect();
        if !uses.is_empty() && uses.iter().all(|u| READS.contains(&u.as_str())) && top.iter().any(|t| super::pyre::search(&read_re, t)) {
            add.push(format!("node(_, {x}, _, _, _)"));
        }
    }
    if add.is_empty() {
        lits
    } else {
        add.extend(lits);
        add
    }
}

/// `_top_binders`: captures the body's own literals bind (not in a disjunction, under `!` or in an aggregate).
pub fn top_binders(px: &Patterns, lits: &[String]) -> Set {
    let top = lits.iter().filter(|l| is_top(l)).cloned().collect::<Vec<_>>().join(", ");
    let mut out = Set::new();
    for x in binders(px, &top, false) {
        let r = re(&format!(r"(\w*\()?(?<![\w$])\$?{}\b", esc(&x)));
        let uses = findall(&r, &top);
        if uses.is_empty() || !uses.iter().all(|u| matches!(u.as_str(), "text(" | "value(" | "str(" | "line(")) {
            out.insert(x);
        }
    }
    out
}


/// `transform_defs_for`: the transform literals a helper needs for the transformed metavariables it reads but does
/// not define, each from its source.
pub fn transform_defs_for(px: &Patterns, defs: &indexmap::IndexMap<String, (String, Vec<String>)>, lits: &[String]) -> Vec<String> {
    let text = lits.join(", ");
    let mut have: Set = findall(&px.text_have, &mask_ticks(px, &strip_strings(px, &text))).into_iter().collect();
    let mut out = Vec::new();
    fn need(px: &Patterns, t: &str, depth: usize, defs: &indexmap::IndexMap<String, (String, Vec<String>)>, have: &mut Set, out: &mut Vec<String>) {
        if have.contains(t) || !defs.contains_key(t) || depth > 20 {
            return;
        }
        let (src, body) = &defs[t];
        if defs.contains_key(src) {
            need(px, src, depth + 1, defs, have, out);
        }
        have.insert(t.to_owned());
        for l in body {
            out.push(super::pyre::split1(&px.text_str_piece, l).into_iter().map(|q| if q.starts_with('"') { q } else { sub(&px.text_td_var, &q, |c| format!("{}_td", g(c, 1))) }).collect());
        }
    }
    let uv = upper_vars(px, &text);
    let mut ts: Vec<&String> = defs.keys().filter(|k| uv.contains(*k)).collect();
    ts.sort();
    for t in ts {
        need(px, t, 0, defs, &mut have, &mut out);
    }
    out
}

/// A pattern for whole words of `names` (longest first), as the passes build them.
pub fn alternation(names: &[String]) -> String {
    let mut ns: Vec<&String> = names.iter().collect();
    ns.sort_by_key(|k| std::cmp::Reverse(k.chars().count()));
    ns.iter().map(|k| esc(k)).collect::<Vec<_>>().join("|")
}
