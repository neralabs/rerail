//! The Soufflé-text helpers the translator uses: `whole` (a search regex as a whole-string one), the text's
//! segments, and `check_rule` (the variables a rule leaves unbound).

use super::patterns::Patterns;
use super::pyre::{findall, re};
use std::collections::BTreeSet;

// ---- regexes: search semantics -> Soufflé's whole-string `match` ------------------------------------------------

/// `r` split at top-level `|` (outside groups and classes).
fn top_alts(r: &str) -> Vec<String> {
    let b = r.as_bytes();
    let (mut out, mut depth, mut cls, mut i, mut start) = (Vec::new(), 0i32, false, 0usize, 0usize);
    while i < b.len() {
        let ch = b[i];
        if ch == b'\\' {
            i += 2;
            continue;
        }
        if cls {
            if ch == b']' {
                cls = false;
            }
        } else if ch == b'[' {
            cls = true;
            if b.get(i + 1) == Some(&b']') {
                i += 1;
            }
        } else if ch == b'(' {
            depth += 1;
        } else if ch == b')' {
            depth -= 1;
        } else if ch == b'|' && depth == 0 {
            out.push(r[start..i].to_owned());
            start = i + 1;
        }
        i += 1;
    }
    out.push(r.get(start..).unwrap_or("").to_owned());
    out
}

fn trailing_backslashes(s: &str) -> usize {
    s.len() - s.trim_end_matches('\\').len()
}

fn ends_anchor(a: &str) -> bool {
    if !a.ends_with('$') {
        return false;
    }
    trailing_backslashes(&a[..a.len() - 1]) % 2 == 0
}


/// A regex for whole-string matching equivalent to searching for r.
pub fn whole(px: &Patterns, r: &str) -> String {
    let (flags, r) = match px.souffle_flags.find(r).ok().flatten() {
        Some(m) => (m.as_str().to_owned(), &r[m.end()..]),
        None => (String::new(), r),
    };
    let multiline = flags.contains('m');
    let any = if multiline || r.contains("\\n") || r.contains("\\r") { "(?s:.*)" } else { ".*" };
    let starts = |a: &str| (a.starts_with('^') && !multiline) || a.starts_with("\\A");
    let strip_start = |a: &str| -> String { if a.starts_with("\\A") { a[2..].to_owned() } else { a.get(1..).unwrap_or("").to_owned() } };
    let ends = |a: &str| {
        if a.ends_with("\\z") || a.ends_with("\\Z") {
            return trailing_backslashes(&a[..a.len() - 2]) % 2 == 0;
        }
        !multiline && ends_anchor(a)
    };
    let strip_end = |a: &str| -> String { if a.ends_with("\\z") || a.ends_with("\\Z") { a[..a.len() - 2].to_owned() } else { a[..a.len().saturating_sub(1)].to_owned() } };
    let alts = top_alts(r);
    if alts.len() == 1 {
        let mut a = alts[0].clone();
        let (mut left, mut right) = (starts(&a), ends(&a));
        if left {
            a = strip_start(&a);
        }
        if right {
            a = strip_end(&a);
        }
        if !left && (a.starts_with(".*") || a.starts_with("(?s:.*)")) {
            left = true;
        }
        if !right && a.ends_with(".*") && !a.ends_with("\\.*") {
            right = true;
        }
        return format!("{flags}{}{a}{}", if left { "" } else { any }, if right { "" } else { any });
    }
    let left = alts.iter().all(|a| starts(a));
    let right = alts.iter().all(|a| ends(a));
    let mut alts = alts;
    if left {
        alts = alts.iter().map(|a| strip_start(a)).collect();
    }
    if right {
        alts = alts.iter().map(|a| strip_end(a)).collect();
    }
    format!("{flags}{}(?:{}){}", if left { "" } else { any }, alts.join("|"), if right { "" } else { any })
}

// ---- scanning -----------------------------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Seg {
    Code,
    Str,
    Tick,
    Comment,
}

/// (kind, text) pieces: code, strings, backtick patterns and comments.
pub fn segments(text: &str) -> Vec<(Seg, String)> {
    let b = text.as_bytes();
    let n = b.len();
    let (mut out, mut i, mut buf_start) = (Vec::new(), 0usize, None::<usize>);
    let flush = |out: &mut Vec<(Seg, String)>, buf_start: &mut Option<usize>, i: usize| {
        if let Some(s) = buf_start.take() {
            out.push((Seg::Code, text[s..i].to_owned()));
        }
    };
    while i < n {
        let ch = b[i];
        if ch == b'"' {
            let mut j = i + 1;
            while j < n && b[j] != b'"' {
                j += if b[j] == b'\\' { 2 } else { 1 };
            }
            flush(&mut out, &mut buf_start, i);
            let end = (j + 1).min(n);
            out.push((Seg::Str, text[i..end].to_owned()));
            i = j + 1;
        } else if ch == b'`' {
            let mut k = i;
            while k < n && b[k] == b'`' {
                k += 1;
            }
            let fence = &text[i..k];
            let j = text[k..].find(fence).map_or(n, |x| x + k);
            flush(&mut out, &mut buf_start, i);
            let end = (j + fence.len()).min(n);
            out.push((Seg::Tick, text[i..end].to_owned()));
            i = j + fence.len();
        } else if text[i..].starts_with("//") {
            let j = text[i..].find('\n').map_or(n, |x| x + i);
            flush(&mut out, &mut buf_start, i);
            out.push((Seg::Comment, text[i..j].to_owned()));
            i = j;
        } else {
            if buf_start.is_none() {
                buf_start = Some(i);
            }
            i += 1;
        }
    }
    if let Some(s) = buf_start {
        out.push((Seg::Code, text[s..n.max(s)].to_owned()));
    }
    out
}

const BUILTIN_TEXT_TESTS: [&str; 10] = ["glob", "iglob", "prefix", "iprefix", "suffix", "isuffix", "contains", "icontains", "word", "iword"];
const BITWISE: [&str; 11] = ["band", "bor", "bxor", "bnot", "bshl", "bshr", "bshru", "land", "lor", "lxor", "lnot"];


type Toks = Vec<String>;

fn lex(px: &Patterns, s: &str) -> Toks {
    let mut out = Vec::new();
    for (kind, seg) in segments(s) {
        match kind {
            Seg::Comment => {}
            Seg::Str | Seg::Tick => out.push(seg),
            Seg::Code => out.extend(findall(&px.souffle_stmt_tok, &seg)),
        }
    }
    drop_json(out)
}

/// The JSON object argument of ast(…) and text_rule(…) is data, not variables.
fn drop_json(toks: Toks) -> Toks {
    let (mut out, mut i) = (Vec::new(), 0);
    while i < toks.len() {
        if (toks[i] == "ast" || toks[i] == "text_rule") && i + 1 < toks.len() && toks[i + 1] == "(" {
            let mut j = i + 2;
            while j < toks.len() && toks[j] != "{" && toks[j] != ")" {
                j += 1;
            }
            if j < toks.len() && toks[j] == "{" {
                let (mut d, mut k) = (0i32, j);
                while k < toks.len() {
                    d += (toks[k] == "{") as i32;
                    d -= (toks[k] == "}") as i32;
                    if d == 0 {
                        break;
                    }
                    k += 1;
                }
                out.extend(toks[i..j].iter().cloned());
                out.push("\"json\"".into());
                i = k + 1;
                continue;
            }
        }
        out.push(toks[i].clone());
        i += 1;
    }
    out
}

fn split_top(toks: &[String], sep: &str) -> Vec<Toks> {
    let (mut out, mut cur, mut d) = (Vec::new(), Vec::new(), 0i32);
    for t in toks {
        if matches!(t.as_str(), "(" | "{" | "[") {
            d += 1;
        } else if matches!(t.as_str(), ")" | "}" | "]") {
            d -= 1;
        }
        if t == sep && d == 0 {
            out.push(std::mem::take(&mut cur));
        } else {
            cur.push(t.clone());
        }
    }
    out.push(cur);
    out
}

fn is_ident(t: &str) -> bool {
    let mut cs = t.chars();
    cs.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') && cs.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn is_var(t: &str) -> bool {
    is_ident(t) && t != "_" && t != "must" && t != "may" && !BITWISE.contains(&t)
}


/// Variables in a term/argument list: identifiers not followed by `(`.
fn vars(px: &Patterns, toks: &[String]) -> BTreeSet<String> {
    let mut vs = BTreeSet::new();
    for (i, t) in toks.iter().enumerate() {
        let nxt = toks.get(i + 1).map_or("", String::as_str);
        if t.starts_with('`') {
            vs.extend(findall(&px.souffle_tick_var, t));
        } else if is_var(t) && nxt != "(" && !t.starts_with('@') {
            vs.insert(t.clone());
        }
    }
    vs
}

fn atom_args(toks: &[String]) -> (String, Vec<Toks>) {
    let name = toks[0].clone();
    let inner = &toks[2..toks.len() - 1];
    (name, if inner.is_empty() { Vec::new() } else { split_top(inner, ",") })
}

fn relation_arg(name: &str) -> Option<&'static [usize]> {
    match name {
        "within_stop" | "nearest" | "precedes_stop" | "inside" | "has" => Some(&[2]),
        "dfs_first" => Some(&[3, 4]),
        "dfs_first_memo" => Some(&[3, 4, 5]),
        _ => None,
    }
}

/// A rule's statement that the checker cannot read (Python raised).
#[derive(Debug)]
pub struct Unreadable(pub String);

type Set = BTreeSet<String>;

/// Unbound variables of one rule, sorted (empty when grounded).
pub fn check_rule(px: &Patterns, stmt: &str, bound_heads: &BTreeSet<String>, bound_vars: &[String]) -> Result<Vec<String>, Unreadable> {
    let mut toks = lex(px, stmt);
    if toks.last().is_some_and(|t| t == ".") {
        toks.pop();
    }
    if toks.first().is_some_and(|t| t == "must" || t == "may") {
        toks.remove(0);
    }
    let (head, mut rest): (Toks, Toks) = if let Some(k) = toks.iter().position(|t| t == ":-") {
        (toks[..k].to_vec(), toks[k + 1..].to_vec())
    } else if let Some(k) = toks.iter().position(|t| t == "=>") {
        let mut r = vec!["=>".to_owned()];
        r.extend(toks[k + 1..].iter().cloned());
        (toks[..k].to_vec(), r)
    } else {
        (toks.clone(), Vec::new())
    };
    let mut edits = Vec::new();
    if let Some(k) = rest.iter().position(|t| t == "=>") {
        edits = rest[k + 1..].to_vec();
        rest.truncate(k);
    }
    let name = head.first().cloned().unwrap_or_default();
    let mut bound: Set = Set::new();
    if bound_heads.contains(&name) {
        bound.extend(vars(px, &head[1.min(head.len())..]));
    }
    bound.extend(bound_vars.iter().cloned());
    let branches = split_top(&rest, ";");
    if branches.len() > 1 {
        let mut tail = Vec::new();
        if !edits.is_empty() {
            tail.push("=>".to_owned());
            tail.extend(edits.iter().cloned());
        }
        let mut all = Set::new();
        for b in branches {
            let mut parts = head.clone();
            parts.push(":-".into());
            parts.extend(b);
            parts.extend(tail.iter().cloned());
            all.extend(check_rule(px, &parts.join(" "), bound_heads, bound_vars)?);
        }
        return Ok(all.into_iter().collect());
    }
    let lits: Vec<Toks> = split_top(&rest, ",").into_iter().filter(|l| !l.is_empty()).collect();
    let mut need = Set::new();
    let mut pending = lits;
    let mut changed = true;
    while changed {
        changed = false;
        // `for l in list(pending)`: each literal of the round, in order; a done one leaves the list
        let round = pending.clone();
        for l in round {
            let (b, n, done) = literal(px, &l, &bound, false)?;
            if done {
                if let Some(p) = pending.iter().position(|x| *x == l) {
                    pending.remove(p);
                }
                bound.extend(b);
                need.extend(n);
                changed = true;
            }
        }
    }
    for l in &pending {
        let (_, n, _) = literal(px, l, &bound, true)?;
        need.extend(n);
    }
    need.extend(vars(px, &drop_aggregates(&head[1.min(head.len())..])));
    need.extend(vars(px, &edits));
    Ok(need.into_iter().filter(|v| !bound.contains(v)).collect())
}

/// Head tokens without aggregates.
fn drop_aggregates(toks: &[String]) -> Toks {
    let (mut out, mut i) = (Vec::new(), 0);
    while i < toks.len() {
        if matches!(toks[i].as_str(), "count" | "min" | "max" | "sum" | "join" | "mean")
            && let Some(off) = toks[i..].iter().position(|t| t == "{")
        {
            let mut j = i + off;
            if toks[i..j].iter().any(|t| t == ":") {
                let mut depth = 0i32;
                while j < toks.len() {
                    depth += (toks[j] == "{") as i32;
                    depth -= (toks[j] == "}") as i32;
                    j += 1;
                    if depth == 0 {
                        break;
                    }
                }
                i = j;
                continue;
            }
        }
        out.push(toks[i].clone());
        i += 1;
    }
    out
}

fn has(l: &[String], t: &str) -> bool {
    l.iter().any(|x| x == t)
}

/// (binds, needs, ready) for one body literal.
fn literal(px: &Patterns, l: &[String], bound: &Set, force: bool) -> Result<(Set, Set, bool), Unreadable> {
    let sub = |a: &Set, b: &Set| a.is_subset(b);
    if l[0] == "(" && l[l.len() - 1] == ")" && has(l, ";") {
        let branches = split_top(&l[1..l.len() - 1], ";");
        let mut sets: Vec<Set> = Vec::new();
        let mut needs = Set::new();
        for br in branches {
            let mut inner = bound.clone();
            for lit in split_top(&br, ",").into_iter().filter(|x| !x.is_empty()) {
                let (b, n, _) = literal(px, &lit, &inner, true)?;
                inner.extend(b);
                needs.extend(n.into_iter().filter(|v| !inner.contains(v)));
            }
            sets.push(inner.difference(bound).cloned().collect());
        }
        let binds = sets.iter().skip(1).fold(sets.first().cloned().unwrap_or_default(), |acc, s| acc.intersection(s).cloned().collect());
        return Ok((binds, needs, true));
    }
    if l[0] == "!" {
        let rest = &l[1..];
        let mut vs = vars(px, &strip_relation_args(rest)?);
        if !rest.is_empty() && (rest[0] == "match" || rest[0] == "match_all") {
            vs = vars(px, rest.get(3..).unwrap_or(&[]));
        }
        let ready = force || sub(&vs, bound);
        return Ok((Set::new(), vs, ready));
    }
    if has(l, "~") && l.iter().any(|t| t.starts_with('`')) {
        let k = l.iter().position(|t| t == "~").expect("a ~");
        let mut b = vars(px, &l[..k]);
        b.extend(vars(px, &l[k + 1..]));
        return Ok((b, Set::new(), true));
    }
    if BUILTIN_TEXT_TESTS.contains(&l[0].as_str()) && l.len() > 2 && l[1] == "(" {
        let vs = vars(px, l);
        let ready = force || sub(&vs, bound);
        return Ok((Set::new(), vs, ready));
    }
    if (l[0] == "match" || l[0] == "match_all") && l.len() > 2 {
        let args = split_top(&l[2..l.len() - 1], ",");
        let groups: Set = findall(&px.souffle_named_group, &l[2]).into_iter().collect();
        let subj = if args.len() > 1 { vars(px, &args[1]) } else { Set::new() };
        let ready = force || sub(&subj, bound);
        return Ok((groups, subj, ready));
    }
    if ["count", "min", "max", "sum", "join"].iter().any(|t| has(l, t)) && has(l, ":") && has(l, "{") {
        let a = l.iter().position(|t| t == "{").expect("a {");
        let e = l.iter().rposition(|t| t == "}").ok_or_else(|| Unreadable("'}' is not in list".into()))?;
        let inner: Toks = if e > a { l[a + 1..e].to_vec() } else { Vec::new() };
        let mut outside: Toks = l[..a].to_vec();
        outside.extend(l[e + 1..].iter().cloned());
        let inner_vars = inner_needs(px, &inner, bound)?;
        if outside.len() >= 2 && outside[1] == "=" && is_var(&outside[0]) {
            let ready = force || sub(&inner_vars, bound);
            return Ok((Set::from([outside[0].clone()]), inner_vars, ready));
        }
        let kept: Toks = outside.iter().filter(|t| !matches!(t.as_str(), "count" | "min" | "max" | "sum" | "join" | ":")).cloned().collect();
        let mut vs = vars(px, &kept);
        vs.extend(inner_vars);
        if !outside.is_empty() && matches!(outside[0].as_str(), "min" | "max" | "sum") && outside.len() > 1 {
            vs.remove(&outside[1]);
        }
        let ready = force || sub(&vs, bound);
        return Ok((Set::new(), vs, ready));
    }
    for op in ["!=", "<=", ">=", "<", ">"] {
        if has(l, op) {
            let vs = vars(px, l);
            let ready = force || sub(&vs, bound);
            return Ok((Set::new(), vs, ready));
        }
    }
    if let Some(k) = l.iter().position(|t| t == "=") {
        let (left, right) = (&l[..k], &l[k + 1..]);
        let (lv, rv) = (vars(px, left), vars(px, right));
        if left.len() == 1 && is_var(&left[0]) && sub(&rv, bound) {
            return Ok((Set::from([left[0].clone()]), rv, true));
        }
        if right.len() == 1 && is_var(&right[0]) && sub(&lv, bound) {
            return Ok((Set::from([right[0].clone()]), lv, true));
        }
        let all: Set = lv.union(&rv).cloned().collect();
        let ready = force || sub(&all, bound);
        return Ok((Set::new(), all, ready));
    }
    if l.len() >= 3 && l[1] == "(" {
        return Ok((vars(px, &strip_relation_args(l)?), Set::new(), true));
    }
    Ok((Set::new(), vars(px, l), true))
}

/// Variables an aggregate body needs from outside.
fn inner_needs(px: &Patterns, inner: &[String], bound: &Set) -> Result<Set, Unreadable> {
    let lits: Vec<Toks> = split_top(inner, ",").into_iter().filter(|x| !x.is_empty()).collect();
    let mut b = bound.clone();
    let mut needs = Set::new();
    let mut pend = lits;
    let mut changed = true;
    while changed {
        changed = false;
        for lit in pend.clone() {
            let (bb, nn, done) = literal(px, &lit, &b, false)?;
            if done {
                if let Some(p) = pend.iter().position(|x| *x == lit) {
                    pend.remove(p);
                }
                b.extend(bb);
                needs.extend(nn);
                changed = true;
            }
        }
    }
    for lit in &pend {
        let (_, nn, _) = literal(px, lit, &b, true)?;
        needs.extend(nn);
    }
    let local: Set = b.difference(bound).cloned().collect();
    Ok(needs.into_iter().filter(|v| !local.contains(v)).collect())
}

fn strip_relation_args(l: &[String]) -> Result<Toks, Unreadable> {
    if l.len() >= 3
        && l[1] == "("
        && let Some(drop_at) = relation_arg(&l[0])
    {
        let (name, args) = atom_args(l);
        let drop: Vec<usize> = drop_at.iter().copied().filter(|&k| k < args.len() && args[k].len() == 1).collect();
        let args: Vec<Toks> = args.into_iter().enumerate().filter(|(j, _)| !drop.contains(j)).map(|(_, a)| a).collect();
        let mut out = vec![name, "(".to_owned()];
        for (j, a) in args.into_iter().enumerate() {
            if j > 0 {
                out.push(",".into());
            }
            out.extend(a);
        }
        out.push(")".into());
        return Ok(out);
    }
    Ok(l.to_vec())
}

