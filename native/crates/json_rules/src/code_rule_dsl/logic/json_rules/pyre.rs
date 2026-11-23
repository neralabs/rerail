//! Python's `re` calls the translator makes, over `fancy_regex` (the Python's look-arounds and back-references):
//! `sub` with a function, `findall`, `split` keeping groups, `escape`. Offsets are byte offsets; every pattern is
//! the translator's own, so a pattern that does not compile is a bug (a panic), and a match that fails (fancy-regex's
//! backtrack limit) counts as no match.

use fancy_regex::{Captures, Regex};

/// A pattern of the translator's.
pub fn re(p: &str) -> Regex {
    Regex::new(p).unwrap_or_else(|e| panic!("json_rules: bad regex {p:?}: {e}"))
}

/// `re.escape(s)` as Python 3.7+ writes it (text the translator prints).
pub fn py_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if "()[]{}?*+-|^$\\.&~# \t\n\r\u{b}\u{c}".contains(ch) {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// A literal for a pattern of the translator's own (not printed).
pub fn esc(s: &str) -> String {
    fancy_regex::escape(s).into_owned()
}

/// Every match's captures, in order (Python's `finditer`).
pub fn captures<'t>(r: &Regex, s: &'t str) -> Vec<Captures<'t>> {
    let mut out = Vec::new();
    let mut at = 0;
    while at <= s.len() {
        let Ok(Some(c)) = r.captures_from_pos(s, at) else { break };
        let m = c.get(0).expect("group 0");
        let (start, end) = (m.start(), m.end());
        out.push(c);
        at = if end == start { next_char(s, end) } else { end };
    }
    out
}

fn next_char(s: &str, i: usize) -> usize {
    if i >= s.len() { s.len() + 1 } else { i + s[i..].chars().next().map_or(1, char::len_utf8) }
}

/// `re.search(p, s)`.
pub fn search(r: &Regex, s: &str) -> bool {
    matches!(r.is_match(s), Ok(true))
}

/// `re.search` with its captures.
pub fn find<'t>(r: &Regex, s: &'t str) -> Option<Captures<'t>> {
    r.captures(s).ok().flatten()
}

/// `re.findall(p, s)` for a pattern of no group or one: each match's text, or its group.
pub fn findall(r: &Regex, s: &str) -> Vec<String> {
    let groups = r.captures_len();
    captures(r, s).into_iter().map(|c| c.get(if groups > 1 { 1 } else { 0 }).map_or(String::new(), |m| m.as_str().to_owned())).collect()
}

/// `re.findall` of a pattern with groups: each match's groups ("" where one is unset).
pub fn findall_groups(r: &Regex, s: &str) -> Vec<Vec<String>> {
    let n = r.captures_len();
    captures(r, s).into_iter().map(|c| (1..n).map(|i| c.get(i).map_or(String::new(), |m| m.as_str().to_owned())).collect()).collect()
}

/// `re.sub(p, f, s)`: each match replaced by `f` of its captures. An empty match is replaced too, also right where
/// a non-empty one ended (Python 3.7's rule).
pub fn sub(r: &Regex, s: &str, mut f: impl FnMut(&Captures) -> String) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last = 0;
    let mut at = 0;
    while at <= s.len() {
        let Ok(Some(c)) = r.captures_from_pos(s, at) else { break };
        let m = c.get(0).expect("group 0");
        let (start, end) = (m.start(), m.end());
        out.push_str(&s[last..start]);
        out.push_str(&f(&c));
        last = end;
        at = if end == start { next_char(s, end) } else { end };
    }
    out.push_str(&s[last.min(s.len())..]);
    out
}

/// `re.sub(p, s_rep, s)` with a fixed replacement.
pub fn sub_str(r: &Regex, s: &str, rep: &str) -> String {
    sub(r, s, |_| rep.to_owned())
}

/// `re.split(p, s)` for a pattern with groups: [text, group 1, …, text, …], `None` for an unset group.
pub fn split(r: &Regex, s: &str) -> Vec<Option<String>> {
    let n = r.captures_len();
    let mut out = Vec::new();
    let mut last = 0;
    for c in captures(r, s) {
        let m = c.get(0).expect("group 0");
        out.push(Some(s[last..m.start()].to_owned()));
        for i in 1..n {
            out.push(c.get(i).map(|g| g.as_str().to_owned()));
        }
        last = m.end();
    }
    out.push(Some(s[last..].to_owned()));
    out
}

/// The pieces of `re.split` with one group, unset groups as "".
pub fn split1(r: &Regex, s: &str) -> Vec<String> {
    split(r, s).into_iter().map(Option::unwrap_or_default).collect()
}

/// `re.fullmatch(p, s)`: the pattern wrapped as `^(?:p)$` (with no newline before the end, as Python reads it).
pub fn fullmatch<'t>(p: &str, s: &'t str) -> Option<Captures<'t>> {
    let r = re(&format!(r"^(?:{p})\z"));
    r.captures(s).ok().flatten()
}

pub fn is_full(p: &str, s: &str) -> bool {
    fullmatch(p, s).is_some()
}

/// A group's text, or "".
pub fn g<'a>(c: &'a Captures, i: usize) -> &'a str {
    c.get(i).map_or("", |m| m.as_str())
}

/// Whether a group matched.
pub fn has(c: &Captures, i: usize) -> bool {
    c.get(i).is_some()
}

/// The `[A-Z][A-Z0-9_]*` test (a metavariable).
pub fn is_meta(s: &str) -> bool {
    let mut cs = s.chars();
    cs.next().is_some_and(|c| c.is_ascii_uppercase()) && cs.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

/// `[a-z]\w*`.
pub fn is_lower_word(s: &str) -> bool {
    let mut cs = s.chars();
    cs.next().is_some_and(|c| c.is_ascii_lowercase()) && cs.all(is_word)
}

/// Python's `\w` on one character.
pub fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

