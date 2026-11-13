//! `engine_dialect.engine_text(text, base)`: the mechanical post-pass from translate's display dialect to the one
//! the engine's reader takes (base: the policy id). Each pass is the Python one of the same name, in its order.
//! Offsets are byte offsets; every delimiter the passes look for is ASCII.

use super::lower::R;
use super::patterns::Patterns;
use super::pyre::{captures, find, findall, g, has, is_full, re, search, sub};
use super::souffle::check_rule;
use super::text::{Set, alternation, head_span_ok, split_top_text};
use super::value::{Value, dumps, parse};
use fancy_regex::Regex;
use indexmap::IndexMap;
use std::collections::BTreeSet;

/// `engine_dialect.split_args`: top-level commas, quotes and brackets kept whole.
fn split_args(s: &str) -> Vec<String> {
    let (mut out, mut depth, mut cur, mut q) = (Vec::new(), 0i32, String::new(), None::<char>);
    for ch in s.chars() {
        if let Some(qc) = q {
            cur.push(ch);
            if ch == qc {
                q = None;
            }
            continue;
        }
        if ch == '"' || ch == '`' {
            q = Some(ch);
        } else if "([{".contains(ch) {
            depth += 1;
        } else if ")]}".contains(ch) {
            depth -= 1;
        }
        if ch == ',' && depth == 0 {
            out.push(cur.trim().to_owned());
            cur.clear();
        } else {
            cur.push(ch);
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim().to_owned());
    }
    out
}

/// Index after the head's closing paren, from the '(' at start.
fn head_span(text: &str, start: usize) -> Option<usize> {
    let b = text.as_bytes();
    let (mut depth, mut q) = (0i32, None::<u8>);
    for (i, &ch) in b.iter().enumerate().skip(start) {
        if let Some(qc) = q {
            if ch == qc {
                q = None;
            }
            continue;
        }
        if ch == b'"' || ch == b'`' {
            q = Some(ch);
        } else if ch == b'(' {
            depth += 1;
        } else if ch == b')' {
            depth -= 1;
            if depth == 0 {
                return Some(i + 1);
            }
        }
    }
    None
}

/// Index of the `.` that ends the rule whose body starts at start (outside strings, ticks and braces).
fn rule_end(text: &str, start: usize) -> usize {
    let b = text.as_bytes();
    let (mut q, mut depth, mut esc) = (None::<u8>, 0i32, false);
    for i in start..b.len() {
        let ch = b[i];
        if let Some(qc) = q {
            if esc {
                esc = false;
            } else if ch == b'\\' && qc == b'"' {
                esc = true;
            } else if ch == qc {
                q = None;
            }
            continue;
        }
        if ch == b'"' || ch == b'`' {
            q = Some(ch);
        } else if b"({[".contains(&ch) {
            depth += 1;
        } else if b")}]".contains(&ch) {
            depth -= 1;
        } else if ch == b'.' && depth == 0 && (i + 1 == b.len() || b[i + 1] == b'\n' || b[i + 1] == b' ') {
            return i;
        }
    }
    b.len()
}

fn slice(text: &str, a: usize, b: usize) -> &str {
    let b = b.min(text.len());
    if a >= b { "" } else { &text[a..b] }
}


/// `anchor x` (a capture): the finding is at x, so x takes the `at` column and the witness n takes x's column.
fn anchor_captures(px: &Patterns, mut text: String) -> String {
    let found: Vec<(String, String, String, String, String)> =
        captures(&px.engine_dialect_anchor, &text).iter().map(|m| (g(m, 0).to_owned(), g(m, 1).to_owned(), g(m, 2).to_owned(), g(m, 3).to_owned(), g(m, 4).to_owned())).collect();
    for (whole, rel, cols, rest, anchor) in found {
        let raw_cols: Vec<&str> = cols.split(',').collect();
        let names: Vec<String> = raw_cols.iter().map(|c| c.split(':').next().unwrap_or("").trim().to_owned()).collect();
        let Some(k) = names.iter().position(|n| *n == anchor) else { continue };
        let new_cols = names.iter().zip(&raw_cols).enumerate().map(|(i, (n, c))| if i == 0 || i == k { format!("{n}: node") } else { c.trim().to_owned() }).collect::<Vec<_>>().join(", ");
        text = text.replacen(&whole, &format!(".violation {rel}({new_cols}){rest}\n"), 1);
        let (mut out, mut pos) = (String::new(), 0usize);
        let heads = captures(&re(&format!(r"(?m)^{rel}\(")), &text).iter().map(|h| h.get(0).expect("a match").end()).collect::<Vec<_>>();
        for hend in heads {
            let Some(end) = head_span(&text, hend - 1) else { panic!("unbalanced head") };
            let mut args = split_args(&text[hend..end - 1]);
            let mut tail = None;
            if args.len() == names.len() {
                let stop = rule_end(&text, end);
                if search(&re(&format!(r"\bunset\({}\)", fancy_regex::escape(&args[k]))), slice(&text, end, stop)) {
                    let (x, at) = (args[k].clone(), args[0].clone());
                    args[0] = "rr_at".into();
                    args[k] = x.clone();
                    tail = Some((stop, format!("{},\n    (unset({x}), rr_at = {at} ; !unset({x}), rr_at = {x})", slice(&text, end, stop))));
                } else {
                    args.swap(0, k);
                }
            }
            out.push_str(slice(&text, pos, hend));
            out.push_str(&args.join(", "));
            out.push(')');
            match tail {
                Some((stop, t)) => {
                    out.push_str(&t);
                    pos = stop;
                }
                None => pos = end,
            }
        }
        out.push_str(slice(&text, pos, text.len()));
        text = out;
    }
    text
}

const KEYWORDS: [&str; 19] = ["suppress", "fix", "code", "tag", "options", "anchor", "each", "at", "in", "over", "count", "sum", "min", "max", "mean", "safe", "noqa", "nosec", "ruff"];

fn rename_outside_literals(px: &Patterns, text: &str, names: &Set, tag: &str) -> String {
    let ns: Vec<String> = names.iter().cloned().collect();
    let pat = re(&format!(r"(?<![\w$])(?<!: )({})(?!\w)", alternation(&ns)));
    let ren = |s: &str| sub(&pat, s, |x| format!("{tag}_{}", g(x, 1)));
    let (mut out, mut pos) = (String::new(), 0usize);
    for m in captures(&px.engine_dialect_lit, text) {
        let m0 = m.get(0).expect("a match");
        out.push_str(&ren(&text[pos..m0.start()]));
        out.push_str(m0.as_str());
        pos = m0.end();
    }
    out.push_str(&ren(&text[pos..]));
    out
}

/// Helper relations a rule declares are its own: prefixed with the rule's code.
fn own_helpers(px: &Patterns, mut text: String, base: &str) -> String {
    // Python takes `base.split("-")[1]`; a base with no `-` (not one of main's ids) is its own tag.
    let part = base.split('-').nth(1).unwrap_or(base);
    let tag = sub(&px.engine_dialect_not_lower_digit, part, |_| "_".into());
    let local: Set = findall(&px.engine_dialect_decl, &text).into_iter().collect();
    let plain: Set = local.iter().filter(|x| !KEYWORDS.contains(&x.as_str())).cloned().collect();
    let keyword: Vec<&String> = local.iter().filter(|x| KEYWORDS.contains(&x.as_str())).collect();
    if !plain.is_empty() {
        text = rename_outside_literals(px, &text, &plain, &tag);
    }
    for k in keyword {
        text = sub(&re(&format!(r"(?<![\w$]){k}(?=\()")), &text, |_| format!("{tag}_{k}"));
    }
    text
}


/// A message's `{x}` names a column (or `{x.line}`); any other brace is the text's own: `{{`, `}}`.
fn message_braces(px: &Patterns, text: String) -> String {
    sub(&px.engine_dialect_message, &text, |m| {
        let cols: Vec<String> = g(m, 2).split(',').map(|c| c.split(':').next().unwrap_or("").trim().to_owned()).collect();
        let msg = sub(&px.engine_dialect_brace, g(m, 4), |b| {
            if has(b, 2) {
                return g(b, 2).repeat(2);
            }
            let key = g(b, 1).split('.').next().unwrap_or("");
            if cols.iter().any(|c| c == key) { g(b, 0).to_owned() } else { format!("{{{{{}}}}}", g(b, 1)) }
        });
        format!(".violation {}({}){}{msg}", g(m, 1), g(m, 2), g(m, 3))
    })
}

const DEFAULT: [i64; 2] = [3, 10];

fn ver(v: &Value) -> Vec<i64> {
    let s = match v {
        Value::Str(s) => s.clone(),
        other => super::value::py_str(other),
    };
    s.split('.').map(|x| x.trim().parse().unwrap_or(0)).collect()
}

fn get<'a>(v: Option<&'a Value>, k: &str) -> Option<&'a Value> {
    v.filter(|x| x.truthy()).and_then(|x| x.get(k))
}

fn dumps_ascii(v: &Value) -> String {
    dumps(v, true, ", ", ": ", false)
}

/// `options` (main's clause conditions), which a `logic` run does not read, as rules.
fn lower_options(px: &Patterns, mut text: String) -> String {
    let found: Vec<(String, String, String)> = captures(&px.engine_dialect_options, &text).iter().map(|m| (g(m, 1).to_owned(), g(m, 2).to_owned(), g(m, 4).to_owned())).collect();
    for (rel, cols, raw_opts) in found {
        let Ok(opts) = parse(&raw_opts) else { continue };
        text = text.replacen(&format!("    options {raw_opts}\n"), "", 1);
        let empty = Value::Obj(super::value::Obj(Vec::new()));
        let v = get(Some(&opts), "when").and_then(|w| get(Some(w), "pythonVersion")).filter(|x| x.truthy()).unwrap_or(&empty);
        let off = v.get("min").is_some_and(|m| ver(m) > DEFAULT.to_vec()) || v.get("max").is_some_and(|m| ver(m) < DEFAULT.to_vec());
        let fp = get(Some(&opts), "file").and_then(|f| f.get("path")).filter(|x| x.truthy()).unwrap_or(&empty);
        if !off && !fp.truthy() {
            continue;
        }
        let raw = format!("{rel}_raw");
        text = sub(&re(&format!(r"(?m)^{rel}\(")), &text, |_| format!("{raw}("));
        let n = cols.split(',').count();
        let args = (0..n).map(|i| format!("v{i}")).collect::<Vec<_>>().join(", ");
        let decl = format!(".decl {raw}({cols})\n");
        if off {
            text.push_str(&format!("\n// pythonVersion {} excludes Python 3.10, main's default: no findings.\n{decl}", dumps_ascii(v)));
        } else {
            let mut conds = Vec::new();
            if let Some(r) = fp.get("regex") {
                conds.push(format!("match({}, rr_p)", dumps_ascii(r)));
            }
            if let Some(r) = fp.get("notRegex") {
                conds.push(format!("!match({}, rr_p)", dumps_ascii(r)));
            }
            text.push_str(&format!(
                "\n// main's file condition {}\n{decl}{rel}({args}) :- {raw}({args}), rr_d = @doc(v0), doc(rr_d, rr_p, _), {}.\n",
                dumps_ascii(fp),
                conds.join(", ")
            ));
        }
    }
    text
}


/// A pattern that is one metavariable, ``n ~ `$X` ``, matches every node and names it: `X = n`.
fn lone_metavariables(px: &Patterns, text: String) -> String {
    let text = sub(&px.engine_dialect_lone_any, &text, |c| format!("node(_, {}, _, _, _)", g(c, 1)));
    sub(&px.engine_dialect_lone_mv, &text, |c| format!("node(_, {}, _, _, _), {} = {}", g(c, 1), g(c, 2), g(c, 1)))
}


/// Scopes a variable can be local to: each `{ … }` aggregate body, and each branch of a `( a ; b )` disjunction.
fn aggregate_spans(body: &str) -> Vec<(usize, usize)> {
    let b = body.as_bytes();
    let (mut spans, mut stack, mut q) = (Vec::new(), Vec::<(u8, usize, usize, bool)>::new(), None::<u8>);
    for (i, &ch) in b.iter().enumerate() {
        if let Some(qc) = q {
            if ch == qc && b[i - 1] != b'\\' {
                q = None;
            }
            continue;
        }
        if ch == b'"' || ch == b'`' {
            q = Some(ch);
        } else if ch == b'{' || ch == b'(' {
            stack.push((ch, i, i, false));
        } else if ch == b';' && stack.last().is_some_and(|s| s.0 == b'(') {
            let top = stack.last_mut().expect("a scope");
            spans.push((top.2, i));
            top.2 = i;
            top.3 = true;
        } else if ch == b'}' && !stack.is_empty() {
            spans.push((stack.pop().expect("a scope").1, i));
        } else if ch == b')' && !stack.is_empty() {
            let (_, _, branch, disj) = stack.pop().expect("a scope");
            if disj {
                spans.push((branch, i));
            }
        }
    }
    spans
}

/// A body that joins two nodes by equal text gets `@doc(A) = @doc(B)`.
fn same_document_joins(px: &Patterns, text: String) -> String {
    sub(&px.engine_dialect_rule, &text, |m| {
        let mut body = g(m, 0).to_owned();
        if body.contains("parsed(") {
            return body;
        }
        let spans = aggregate_spans(&body);
        let scope = |pos: usize| -> Option<(usize, usize)> {
            let mut best: Option<(usize, usize)> = None;
            for &(s, e) in &spans {
                if s < pos && pos < e && best.is_none_or(|b| e - s < b.1 - b.0) {
                    best = Some((s, e));
                }
            }
            best
        };
        let mut by_t: IndexMap<String, Vec<(String, Option<(usize, usize)>)>> = IndexMap::new();
        for u in captures(&px.engine_dialect_text_use, &body) {
            let v = g(&u, 1);
            if search(&re(&format!(r#"(?<![\w$]){} = [@"]"#, fancy_regex::escape(v))), &body) {
                continue;
            }
            by_t.entry(g(&u, 2).to_owned()).or_default().push((v.to_owned(), scope(u.get(0).expect("a match").start())));
        }
        let mut adds: IndexMap<Option<(usize, usize)>, Vec<String>> = IndexMap::new();
        for uses in by_t.values() {
            let first = &uses[0].0;
            for (node, sc) in &uses[1..] {
                if node == first {
                    continue;
                }
                let s0 = uses[0].1;
                let deeper = match sc {
                    Some(s) if s0.is_none_or(|z| s.1 - s.0 < z.1 - z.0) => Some(*s),
                    _ => s0,
                };
                let c = format!("@doc({first}) = @doc({node})");
                let list = adds.entry(deeper).or_default();
                if !list.contains(&c) {
                    list.push(c);
                }
            }
        }
        let mut scs: Vec<(usize, usize)> = adds.keys().flatten().copied().collect();
        scs.sort_by_key(|s| std::cmp::Reverse(s.1));
        for sc in scs {
            body = format!("{}, {} {}", body[..sc.1].trim_end(), adds[&Some(sc)].join(", "), &body[sc.1..]);
        }
        let extra = adds.get(&None).cloned().unwrap_or_default();
        format!("{}{}.", &body[..body.len() - 1], if extra.is_empty() { String::new() } else { format!(", {}", extra.join(", ")) })
    })
}

/// `anchor file.start`: the finding is at the witness, the module.
fn file_anchor(text: String) -> String {
    text.replace("    anchor file.start\n", "    // anchor file.start: at the witness, the module, which starts the file\n")
}


fn unbound(px: &Patterns, stmt: &str) -> R<Vec<String>> {
    check_rule(px, stmt, &BTreeSet::new(), &[]).map_err(|e| e.0)
}

/// A branch that does not bind X gets `unset(X)`; a head column the body never names gets `unset(X)` at the end.
fn unset_unbound(px: &Patterns, text: String) -> R<String> {
    let binds = |body: &str, x: &str| -> R<bool> { Ok(!unbound(px, &format!("rr_probe({x}) :- {body}."))?.iter().any(|y| y == x)) };
    let (mut out, mut pos) = (String::new(), 0usize);
    let starts: Vec<(usize, usize)> = captures(&px.engine_dialect_head_start, &text).iter().map(|m| (m.get(0).expect("a match").start(), m.get(0).expect("a match").end())).collect();
    for (mstart, mend) in starts {
        if mstart < pos {
            continue;
        }
        let Some(end) = head_span(&text, mend - 1) else { continue };
        if !text[end..].starts_with(" :- ") {
            continue;
        }
        let stop = rule_end(&text, end);
        let rule = slice(&text, mstart, stop + 1);
        let bad: Vec<String> = match unbound(px, rule) {
            Ok(b) => b.into_iter().filter(|x| is_full(r"[A-Z][A-Z0-9_]*", x)).collect(),
            Err(_) => Vec::new(),
        };
        if bad.is_empty() {
            continue;
        }
        let (head, body) = (&text[mstart..end], slice(&text, end + 4, stop));
        let mut lits = split_top_text(body, ',');
        let mut changed = false;
        for x in &bad {
            let mut done = false;
            for i in 0..lits.len() {
                let t = lits[i].trim().to_owned();
                if !(t.starts_with('(') && t.ends_with(')') && head_span_ok(px, &t)) {
                    continue;
                }
                let branches = split_top_text(&t[1..t.len() - 1], ';');
                if branches.len() < 2 {
                    continue;
                }
                let mut ok = Vec::new();
                for b in &branches {
                    ok.push(binds(b, x)?);
                }
                if ok.iter().any(|k| *k) && !ok.iter().all(|k| *k) {
                    lits[i] = format!(
                        " ({})",
                        branches.iter().zip(&ok).map(|(b, k)| if *k { b.trim().to_owned() } else { format!("{}, unset({x})", b.trim()) }).collect::<Vec<_>>().join(" ; ")
                    );
                    done = true;
                    changed = true;
                    break;
                }
            }
            if !done && !search(&re(&format!(r"(?<![\w$]){x}\b")), &sub(&px.engine_dialect_strings, body, |_| String::new())) {
                lits.push(format!(" unset({x})"));
                changed = true;
            }
        }
        if changed {
            out.push_str(slice(&text, pos, mstart));
            out.push_str(&format!("{head} :- {}.", lits.join(",").trim()));
            pos = stop + 1;
        }
    }
    out.push_str(slice(&text, pos, text.len()));
    Ok(out)
}


/// `t = A, t = B` with t used only there: `A = B`.
fn compare_pairs(px: &Patterns, text: String) -> String {
    sub(&px.engine_dialect_rule, &text, |r| {
        let body = g(r, 0);
        sub(&px.engine_dialect_pair, body, |m| {
            let uses = findall(&re(&format!(r"(?<![\w$]){}(?!\w)", fancy_regex::escape(g(m, 1)))), body).len();
            if uses == 2 { format!("{} = {}", g(m, 2), g(m, 3)) } else { g(m, 0).to_owned() }
        })
    })
}


/// main's test that X is unbound, `count : { text(X, t), text(X, t) } = 0`, is `unset(X)`.
fn unbound_tests(px: &Patterns, text: String) -> String {
    sub(&px.engine_dialect_unbound_test, &text, |c| format!("unset({})", g(c, 1)))
}


/// An axis with a stop set inside an aggregate becomes a relation of its own, read inside the aggregate.
fn stop_axes_in_aggregates(px: &Patterns, text: String) -> String {
    let mut made: IndexMap<String, (String, String)> = IndexMap::new();
    let b = text.as_bytes();
    let (mut out, mut pos) = (String::new(), 0usize);
    for m in captures(&px.engine_dialect_agg_open, &text) {
        let m0 = m.get(0).expect("a match");
        if m0.start() < pos {
            continue;
        }
        let (mut depth, mut i) = (0i64, m0.end() - 1);
        while i < b.len() {
            depth += (b[i] == b'{') as i64;
            depth -= (b[i] == b'}') as i64;
            if depth == 0 {
                break;
            }
            i += 1;
        }
        out.push_str(slice(&text, pos, m0.start()));
        out.push_str(&sub(&px.engine_dialect_stop_axis, slice(&text, m0.start(), i + 1), |a| {
            let (axis, x, y, rel) = (g(a, 1), g(a, 2), g(a, 3), g(a, 4));
            let name = format!("{rel}_{}", axis.split('_').next().unwrap_or(axis));
            made.insert(name.clone(), (axis.to_owned(), rel.to_owned()));
            format!("{name}({x}, {y})")
        }));
        pos = i + 1;
    }
    out.push_str(slice(&text, pos, text.len()));
    for (name, (axis, rel)) in &made {
        let cols = if axis == "within_stop" { "n: node, a: node" } else { "a: node, b: node" };
        out.push_str(&format!("\n// {name}: {axis} over the stop set {rel}, read inside an aggregate\n.decl {name}({cols})\n{name}(x, y) :- {axis}(x, y, {rel}).\n"));
    }
    out
}

/// One translated rule's display text in the engine's dialect.
pub fn engine_text(px: &Patterns, text: String, base: &str) -> R<String> {
    let text = stop_axes_in_aggregates(px, text);
    let text = anchor_captures(px, text);
    let text = file_anchor(text);
    let text = own_helpers(px, text, base);
    let text = message_braces(px, text);
    let text = lower_options(px, text);
    let text = lone_metavariables(px, text);
    let text = same_document_joins(px, text);
    let text = compare_pairs(px, text);
    let text = unbound_tests(px, text);
    unset_unbound(px, text)
}

