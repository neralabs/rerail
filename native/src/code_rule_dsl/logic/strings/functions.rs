//! The `core.rr` functors that compute text from text (`@head`, `@join`, `@replace_all`, …), each a native SQL
//! function of the same name. A rule's call `@f(a, b)` is lowered to `f(a, b)` in SQL, its symbol arguments
//! decoded to text and a symbol result encoded back (`plan`'s `text_of` / `symbol_of`). Each function's meaning
//! is `core.rr`'s comment on it. A NULL argument, or a regex that does not compile, gives NULL (no value).

use std::cell::RefCell;
use std::collections::HashMap;

use rusqlite::Connection;
use rusqlite::functions::{Context, FunctionFlags};
use rusqlite::types::Value;

/// An argument or result: a symbol (text) or a number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::code_rule_dsl) enum Kind {
    Text,
    Number,
}

use Kind::{Number, Text};

/// Each function: its name, its arguments and its result.
const FUNCTIONS: &[(&str, &[Kind], Kind)] = &[
    ("qname", &[Text, Text], Text),
    ("pkg_up", &[Text, Number], Text),
    ("head", &[Text], Text),
    ("dir", &[Text], Text),
    ("join", &[Text, Text], Text),
    ("glob_sub", &[Text, Text, Text], Text),
    ("package_of", &[Text], Text),
    ("replace_all", &[Text, Text, Text], Text),
    ("convert", &[Text, Text, Text], Text),
    ("slice", &[Text, Number, Number], Text),
    ("count_matches", &[Text, Text], Number),
    ("sha256", &[Text], Text),
    ("json_str", &[Text], Text),
    ("nfkc", &[Text], Text),
    // -1, 0 or 1 as `a` sorts before, with or after `b` (by bytes): the order of two symbols (`plan::lower`).
    ("str_cmp", &[Text, Text], Number),
];

/// A function's name (as SQL names it), arguments and result, if `name` is one of these.
pub(in crate::code_rule_dsl) fn signature(name: &str) -> Option<(&'static str, &'static [Kind], Kind)> {
    FUNCTIONS.iter().find(|(n, _, _)| *n == name).copied()
}

/// Registers every function on `db`.
pub(in crate::code_rule_dsl) fn register(db: &Connection) -> rusqlite::Result<()> {
    for &(name, args, _) in FUNCTIONS {
        let arity = i32::try_from(args.len()).expect("few arguments");
        db.create_scalar_function(name, arity, FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC, move |ctx| Ok(call(name, args, ctx)))?;
    }
    Ok(())
}

fn call(name: &str, kinds: &[Kind], ctx: &Context) -> Value {
    let mut text = Vec::new();
    let mut numbers = Vec::new();
    for (i, kind) in kinds.iter().enumerate() {
        match kind {
            Text => match ctx.get::<Option<String>>(i) {
                Ok(Some(s)) => text.push(s),
                _ => return Value::Null,
            },
            Number => match ctx.get::<Option<i64>>(i) {
                Ok(Some(n)) => numbers.push(n),
                _ => return Value::Null,
            },
        }
    }
    let t = |i: usize| text[i].as_str();
    let out = match name {
        "qname" => Some(format!("{}#{}", t(0), t(1))),
        "pkg_up" => Some(pkg_up(t(0), numbers[0])),
        "head" => Some(t(0).split('.').next().unwrap_or("").to_owned()),
        "dir" => Some(t(0).rsplit_once('/').map_or("", |(d, _)| d).to_owned()),
        "join" => Some(join(t(0), t(1))),
        "glob_sub" => Some(glob_sub(t(0), t(1), t(2))),
        "package_of" => Some(t(0).to_owned()),
        "replace_all" => with_regex(t(1), |re| re.replace_all(t(0), t(2)).into_owned()),
        "convert" => Some(convert(t(0), t(1), t(2))),
        "slice" => Some(slice(t(0), numbers[0], numbers[1])),
        "str_cmp" => return Value::Integer(t(0).cmp(t(1)) as i64),
        "count_matches" => return with_regex(t(0), |re| Value::Integer(re.find_iter(t(1)).count() as i64)).unwrap_or(Value::Null),
        "sha256" => {
            use sha2::Digest;
            Some(sha2::Sha256::digest(t(0).as_bytes()).iter().map(|b| format!("{b:02x}")).collect())
        }
        "json_str" => Some(json_str(t(0))),
        "nfkc" => {
            use unicode_normalization::UnicodeNormalization;
            Some(t(0).nfkc().collect())
        }
        _ => None,
    };
    out.map_or(Value::Null, Value::Text)
}

/// `f(re)` with `re` compiled once per thread; `None` if it does not compile.
fn with_regex<T>(re: &str, f: impl FnOnce(&regex::Regex) -> T) -> Option<T> {
    thread_local! {
        static CACHE: RefCell<HashMap<String, Option<regex::Regex>>> = RefCell::new(HashMap::new());
    }
    CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        let compiled = cache.entry(re.to_owned()).or_insert_with(|| regex::Regex::new(re).ok());
        compiled.as_ref().map(f)
    })
}

/// `@pkg_up("a.b.c", 1)` = `"a.b"`: `q` without its last `k` dotted parts; `""` past the root.
pub(in crate::code_rule_dsl) fn pkg_up(q: &str, k: i64) -> String {
    let parts: Vec<&str> = if q.is_empty() { Vec::new() } else { q.split('.').collect() };
    let keep = parts.len().saturating_sub(usize::try_from(k.max(0)).unwrap_or(usize::MAX));
    parts[..keep].join(".")
}

/// `@join("src/a", "../c")` = `"src/c"`: the path `rel` names from `dir`, with `.` and `..` resolved.
pub(in crate::code_rule_dsl) fn join(dir: &str, rel: &str) -> String {
    let mut parts: Vec<&str> = if rel.starts_with('/') { Vec::new() } else { dir.split('/').filter(|p| !p.is_empty()).collect() };
    for p in rel.split('/') {
        match p {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    parts.join("/")
}

/// `@glob_sub("@/*", "src/*", "@/lib/x")` = `"src/lib/x"`: `s` matched against `pat` (one `*`, or none), and what
/// the `*` matched put in `to`'s `*`; `""` if it does not match.
pub(in crate::code_rule_dsl) fn glob_sub(pat: &str, to: &str, s: &str) -> String {
    match pat.split_once('*') {
        None if s == pat => to.to_owned(),
        None => String::new(),
        Some((before, after)) if s.len() >= before.len() + after.len() && s.starts_with(before) && s.ends_with(after) => {
            to.replacen('*', &s[before.len()..s.len() - after.len()], 1)
        }
        Some(_) => String::new(),
    }
}

/// `@slice("_name", 1, 5)` = `"name"`: characters `start` to `end - 1`, a negative position counting from the end.
pub(in crate::code_rule_dsl) fn slice(s: &str, start: i64, end: i64) -> String {
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len() as i64;
    let at = |i: i64| (if i < 0 { n + i } else { i }).clamp(0, n) as usize;
    let (a, b) = (at(start), at(end));
    if a >= b { String::new() } else { chars[a..b].iter().collect() }
}

/// `@convert("max-age", "camelCase", "dash")` = `"maxAge"`. `seps` names the separators (`dash`, `underscore`,
/// `dot`, `space`, `slash`, `caseChange`), comma-separated; `""` is all of them.
pub(in crate::code_rule_dsl) fn convert(s: &str, to: &str, seps: &str) -> String {
    let all = seps.trim().is_empty();
    let named: Vec<&str> = seps.split(',').map(str::trim).collect();
    let on = |name: &str| all || named.contains(&name);
    let sep = |c: char| match c {
        '-' => on("dash"),
        '_' => on("underscore"),
        '.' => on("dot"),
        ' ' => on("space"),
        '/' => on("slash"),
        _ => false,
    };
    let mut words: Vec<String> = Vec::new();
    let mut word = String::new();
    let mut prev_lower = false;
    for c in s.chars() {
        if sep(c) {
            words.push(std::mem::take(&mut word));
            prev_lower = false;
            continue;
        }
        if on("caseChange") && c.is_uppercase() && prev_lower {
            words.push(std::mem::take(&mut word));
        }
        prev_lower = c.is_lowercase() || c.is_ascii_digit();
        word.push(c);
    }
    words.push(word);
    let words: Vec<String> = words.into_iter().filter(|w| !w.is_empty()).collect();
    let cap = |w: &str| {
        let mut c = w.chars();
        c.next().map_or_else(String::new, |f| f.to_uppercase().chain(c.flat_map(char::to_lowercase)).collect())
    };
    match to {
        "lowerCase" => s.to_lowercase(),
        "upperCase" => s.to_uppercase(),
        "capitalize" => {
            let mut c = s.chars();
            c.next().map_or_else(String::new, |f| f.to_uppercase().chain(c).collect())
        }
        "camelCase" => words.iter().enumerate().map(|(i, w)| if i == 0 { w.to_lowercase() } else { cap(w) }).collect(),
        "pascalCase" => words.iter().map(|w| cap(w)).collect(),
        "snakeCase" => words.iter().map(|w| w.to_lowercase()).collect::<Vec<_>>().join("_"),
        "kebabCase" => words.iter().map(|w| w.to_lowercase()).collect::<Vec<_>>().join("-"),
        _ => s.to_owned(),
    }
}

/// `s` as a JSON string literal, quotes included, as Python's `json.dumps` writes it by default: non-ASCII as
/// `\uXXXX` (surrogate pairs above U+FFFF).
pub(in crate::code_rule_dsl) fn json_str(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 || (c as u32) > 0x7e => {
                let mut units = [0u16; 2];
                for u in c.encode_utf16(&mut units) {
                    out.push_str(&format!("\\u{u:04x}"));
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
