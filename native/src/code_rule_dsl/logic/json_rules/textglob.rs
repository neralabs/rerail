//! A whole-string regex as the glob-family text test that matches exactly the same strings, or
//! none. Indices are over characters, as Python's are.

const GLOB_SPECIAL: &str = "*?[]{},\\";

struct No;

type R<T> = Result<T, No>;

fn lit(ch: char) -> String {
    if GLOB_SPECIAL.contains(ch) { format!("\\{ch}") } else { ch.to_string() }
}

fn at(s: &[char], i: usize) -> Option<char> {
    s.get(i).copied()
}

fn slice(s: &[char], a: usize, b: usize) -> String {
    let b = b.min(s.len());
    if a >= b { String::new() } else { s[a..b].iter().collect() }
}

fn find_from(s: &[char], ch: char, from: usize) -> Option<usize> {
    (from..s.len()).find(|&k| s[k] == ch)
}

/// A regex class at s[i] == '['; (glob class, next index).
fn class(s: &[char], i: usize) -> R<(String, usize)> {
    let mut j = i + 1;
    if at(s, j) == Some('^') {
        return Err(No);
    }
    let mut out = String::from("[");
    while j < s.len() && s[j] != ']' {
        let c = s[j];
        if c == '\\' {
            if j + 1 >= s.len() {
                return Err(No);
            }
            let e = s[j + 1];
            if e == 'd' {
                out.push_str("0-9");
            } else if "wsWSDbB".contains(e) || e.is_alphanumeric() {
                return Err(No);
            } else {
                out.push('\\');
                out.push(e);
            }
            j += 2;
            continue;
        }
        if c == '[' {
            return Err(No);
        }
        if (c == '!' || c == '\\') && j == i + 1 {
            out.push('\\');
        }
        out.push(c);
        j += 1;
    }
    if j >= s.len() || out == "[" {
        return Err(No);
    }
    Ok((out + "]", j + 1))
}

fn split_alts(s: &[char]) -> Vec<Vec<char>> {
    let (mut out, mut d, mut cur, mut j) = (Vec::new(), 0i32, Vec::new(), 0usize);
    while j < s.len() {
        let c = s[j];
        if c == '\\' {
            cur.extend_from_slice(&s[j..(j + 2).min(s.len())]);
            j += 2;
            continue;
        }
        if c == '[' {
            let k = find_from(s, ']', j + 1).unwrap_or(s.len());
            cur.extend_from_slice(&s[j..(k + 1).min(s.len())]);
            j = k + 1;
            continue;
        }
        if c == '(' {
            d += 1;
        } else if c == ')' {
            d -= 1;
        }
        if c == '|' && d == 0 {
            out.push(std::mem::take(&mut cur));
        } else {
            cur.push(c);
        }
        j += 1;
    }
    out.push(cur);
    out
}

fn group_end(s: &[char], i: usize) -> R<usize> {
    let (mut d, mut j) = (0i32, i);
    while j < s.len() {
        let c = s[j];
        if c == '\\' {
            j += 2;
            continue;
        }
        if c == '[' {
            j = find_from(s, ']', j + 1).map_or(s.len(), |k| k + 1);
            continue;
        }
        if c == '(' {
            d += 1;
        } else if c == ')' {
            d -= 1;
            if d == 0 {
                return Ok(j);
            }
        }
        j += 1;
    }
    Err(No)
}

/// A regex without top-level `|` to a glob fragment.
fn seq(s: &[char]) -> R<String> {
    let (mut out, mut i) = (String::new(), 0usize);
    while i < s.len() {
        let c = s[i];
        let nxt = at(s, i + 1);
        if c == '.' && nxt == Some('*') {
            if matches!(at(s, i + 2), Some('?' | '+' | '*')) {
                return Err(No);
            }
            out.push_str("**");
            i += 2;
            continue;
        }
        if ".^$+*?|)".contains(c) {
            return Err(No);
        }
        if c == '(' {
            let e = group_end(s, i)?;
            let mut inner: &[char] = &s[i + 1..e];
            if inner.starts_with(&['?', ':']) {
                inner = &inner[2..];
            } else if inner.first() == Some(&'?') {
                return Err(No);
            }
            let mut frag = alts(inner)?;
            i = e + 1;
            if matches!(at(s, i), Some('?' | '*' | '+' | '{')) {
                if at(s, i) == Some('?') && !matches!(at(s, i + 1), Some('?' | '+')) {
                    frag = if flat_alts(&frag) {
                        let mut f: Vec<char> = frag.chars().collect();
                        f.pop();
                        f.into_iter().collect::<String>() + ",}"
                    } else {
                        format!("{{{frag},}}")
                    };
                    i += 1;
                } else {
                    return Err(No);
                }
            }
            out.push_str(&frag);
            continue;
        }
        let mut item;
        if c == '[' {
            let (it, next) = class(s, i)?;
            item = it;
            i = next;
        } else if c == '\\' {
            let Some(n) = nxt else { return Err(No) };
            if n == 'd' {
                item = "[0-9]".into();
            } else if n.is_alphanumeric() {
                return Err(No);
            } else {
                item = lit(n);
            }
            i += 2;
        } else {
            item = lit(c);
            i += 1;
        }
        // `\{(\d+)\}` at s[i:]
        let rest = &s[i.min(s.len())..];
        let mut counted = false;
        if rest.first() == Some(&'{') {
            let digits: Vec<char> = rest[1..].iter().take_while(|c| c.is_ascii_digit()).copied().collect();
            if !digits.is_empty() && rest.get(1 + digits.len()) == Some(&'}') {
                let n: usize = digits.iter().collect::<String>().parse().map_err(|_| No)?;
                item = item.repeat(n);
                i += digits.len() + 2;
                counted = true;
            }
        }
        if !counted && matches!(at(s, i), Some('?' | '*' | '+' | '{')) {
            return Err(No);
        }
        out.push_str(&item);
    }
    Ok(out)
}

/// `{a,b}` with no nested braces.
fn flat_alts(frag: &str) -> bool {
    frag.starts_with('{') && frag.ends_with('}') && frag.len() >= 2 && !frag[1..frag.len() - 1].replace("\\{", "").contains('{')
}

fn alts(s: &[char]) -> R<String> {
    let mut parts = Vec::new();
    for p in split_alts(s) {
        parts.push(seq(&p)?);
    }
    if parts.len() == 1 {
        return Ok(parts.remove(0));
    }
    if parts.iter().any(String::is_empty) {
        let mut kept: Vec<String> = parts.into_iter().filter(|p| !p.is_empty()).collect();
        kept.push(String::new());
        parts = kept;
    }
    Ok(format!("{{{}}}", parts.join(",")))
}

/// The literal text of a glob fragment with no wildcards, or None. A trailing `\` is Python's IndexError.
fn plain(g: &[char]) -> Result<Option<String>, String> {
    let (mut out, mut i) = (String::new(), 0usize);
    while i < g.len() {
        if g[i] == '\\' {
            let Some(c) = g.get(i + 1) else { return Err("string index out of range".into()) };
            out.push(*c);
            i += 2;
        } else if "*?[]{},".contains(g[i]) {
            return Ok(None);
        } else {
            out.push(g[i]);
            i += 1;
        }
    }
    Ok(Some(out))
}

/// (glob, case_insensitive) for a whole-string regex, or None.
fn glob_of(rx: &str) -> Option<(String, bool)> {
    let mut ci = false;
    let mut rx = rx;
    if let Some(c) = super::pyre::find(&super::pyre::re(r"^\(\?([a-zA-Z]+)\)"), rx) {
        if c.get(1).expect("flags").as_str().chars().any(|f| f != 'i') {
            return None;
        }
        ci = true;
        rx = &rx[c.get(0).expect("group 0").end()..];
    }
    if rx.contains("(?<") || rx.contains("(?P<") {
        return None;
    }
    let chars: Vec<char> = rx.chars().collect();
    alts(&chars).ok().map(|g| (g, ci))
}

fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// The glob-family test for a whole-string regex, or None (the caller keeps `match`).
pub fn text_test(rx: &str, subject: &str) -> Result<Option<String>, String> {
    let Some((g, ci)) = glob_of(rx) else { return Ok(None) };
    let i = if ci { "i" } else { "" };
    let gc: Vec<char> = g.chars().collect();
    let n = gc.len();
    if g.starts_with("**") && g.ends_with("**") && n > 4 {
        if let Some(l) = plain(&gc[2..n - 2])?
            && !l.is_empty()
        {
            return Ok(Some(format!("{i}contains({}, {subject})", quote(&l))));
        }
    }
    if g.ends_with("**") && n > 2 {
        if let Some(l) = plain(&gc[..n - 2])?
            && !l.is_empty()
        {
            return Ok(Some(format!("{i}prefix({}, {subject})", quote(&l))));
        }
    }
    if g.starts_with("**") && n > 2 {
        if let Some(l) = plain(&gc[2..])?
            && !l.is_empty()
        {
            return Ok(Some(format!("{i}suffix({}, {subject})", quote(&l))));
        }
    }
    Ok(Some(format!("{i}glob({}, {subject})", quote(&g))))
}

