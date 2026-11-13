//! rrlab's regex functors, on the regex crate: Rust-regex semantics, as Ruff's and core.rr's DFA dialect (Unicode
//! classes `\p{L}`, scoped flags `(?is:…)`, `(?P<x>…)` and `(?<x>…)`; no look-around, no back-references).
//! Linked as a static library into functors.cpp's shared library and loaded by souffle -l rrlabfn.
//!
//! match(re, s) is Soufflé's: re must match the whole of s. Python's `\Z` is read as `\z`.
use regex::Regex;
use std::collections::HashMap;
use std::ffi::{CStr, CString, c_char};
use std::sync::{LazyLock, Mutex};

static CACHE: LazyLock<Mutex<HashMap<String, Option<Regex>>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
static POOL: LazyLock<Mutex<HashMap<String, &'static CStr>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

fn arg<'a>(p: *const c_char) -> std::borrow::Cow<'a, str> {
    if p.is_null() {
        return "".into();
    }
    unsafe { CStr::from_ptr(p) }.to_string_lossy()
}

fn intern(s: &str) -> *const c_char {
    let mut pool = POOL.lock().unwrap();
    if let Some(c) = pool.get(s) {
        return c.as_ptr();
    }
    let c: &'static CStr = Box::leak(CString::new(s.replace('\0', "")).unwrap().into_boxed_c_str());
    pool.insert(s.to_string(), c);
    c.as_ptr()
}

/// Python spellings the regex crate lacks: `\Z` (end of text) is `\z`.
fn dialect(re: &str) -> String {
    let mut out = String::with_capacity(re.len());
    let mut it = re.chars().peekable();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.next() {
                Some('Z') => out.push_str("\\z"),
                Some(n) => {
                    out.push('\\');
                    out.push(n)
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// The compiled regex for `key`, built by `build` once; None (and a message on stderr) if it does not compile.
fn with<T>(key: String, build: impl FnOnce() -> String, f: impl FnOnce(&Regex) -> T, none: T) -> T {
    let mut cache = CACHE.lock().unwrap();
    let re = cache.entry(key.clone()).or_insert_with(|| {
        let src = build();
        Regex::new(&src).map_err(|e| eprintln!("rrlab: bad regex {key:?}: {e}")).ok()
    });
    match re {
        Some(r) => f(r),
        None => none,
    }
}

fn whole(re: &str) -> (String, String) {
    (format!("whole\0{re}"), format!("^(?:{})$", dialect(re)))
}

fn matches(re: &str, s: &str) -> bool {
    let (k, src) = whole(re);
    with(k, || src, |r| r.is_match(s), false)
}

#[unsafe(no_mangle)]
pub extern "C" fn rr_match(re: *const c_char, s: *const c_char) -> i32 {
    matches(&arg(re), &arg(s)) as i32
}

#[unsafe(no_mangle)]
pub extern "C" fn rr_search(re: *const c_char, s: *const c_char) -> i32 {
    let re = arg(re);
    with(format!("find\0{re}"), || dialect(&re), |r| r.is_match(&arg(s)), false) as i32
}

fn group(re: &str, s: &str, i: i32) -> Option<String> {
    let (k, src) = whole(re);
    with(k, || src, |r| r.captures(s).and_then(|c| c.get(i as usize)).map(|m| m.as_str().to_string()), None)
}

/// rr_cap(re, s, i): group i of re's whole match of s ("" when there is none); rr_capok: 1 when it took part.
#[unsafe(no_mangle)]
pub extern "C" fn rr_cap(re: *const c_char, s: *const c_char, i: i32) -> *const c_char {
    intern(&group(&arg(re), &arg(s), i).unwrap_or_default())
}

#[unsafe(no_mangle)]
pub extern "C" fn rr_capok(re: *const c_char, s: *const c_char, i: i32) -> i32 {
    group(&arg(re), &arg(s), i).is_some() as i32
}

/// @replace_all(s, re, by): every match of re becomes by; `${1}` and `$1` are groups, as in the regex crate.
#[unsafe(no_mangle)]
pub extern "C" fn replace_all(s: *const c_char, re: *const c_char, by: *const c_char) -> *const c_char {
    let (s, re, by) = (arg(s), arg(re), arg(by));
    let out = with(format!("find\0{re}"), || dialect(&re), |r| Some(r.replace_all(&s, by.as_ref()).into_owned()), None);
    intern(&out.unwrap_or_else(|| s.into_owned()))
}

#[unsafe(no_mangle)]
pub extern "C" fn count_matches(re: *const c_char, s: *const c_char) -> i32 {
    let re = arg(re);
    with(format!("find\0{re}"), || dialect(&re), |r| r.find_iter(&arg(s)).count() as i32, 0)
}

// ---- globs (core.rr's text tests) ---------------------------------------------------------------------------------

/// `*` any run without `/`, `**` any run, `?` one character but `/`, `[…]` a class, `{a,b}` alternatives, `\` escapes.
fn glob_regex(p: &str) -> String {
    let c: Vec<char> = p.chars().collect();
    let mut o = String::from("^(?s:");
    let mut brace = 0;
    let mut i = 0;
    while i < c.len() {
        match c[i] {
            '\\' if i + 1 < c.len() => {
                o.push_str(&regex::escape(&c[i + 1].to_string()));
                i += 1;
            }
            '*' if c.get(i + 1) == Some(&'*') => {
                o.push_str(".*");
                i += 1;
            }
            '*' => o.push_str("[^/]*"),
            '?' => o.push_str("[^/]"),
            '[' => match c[i + 1..].iter().position(|&x| x == ']') {
                Some(j) => {
                    let body: String = c[i + 1..i + 1 + j].iter().collect();
                    let body = body.strip_prefix('!').map(|b| format!("^{b}")).unwrap_or(body);
                    o.push('[');
                    o.push_str(&body.replace('[', "\\["));
                    o.push(']');
                    i += j + 1;
                }
                None => o.push_str("\\["),
            },
            '{' => {
                o.push_str("(?:");
                brace += 1
            }
            '}' if brace > 0 => {
                o.push(')');
                brace -= 1
            }
            ',' if brace > 0 => o.push('|'),
            ch => o.push_str(&regex::escape(&ch.to_string())),
        }
        i += 1;
    }
    o.push_str(")$");
    o
}

fn glob(p: &str, s: &str) -> bool {
    with(format!("glob\0{p}"), || glob_regex(p), |r| r.is_match(s), false)
}

#[unsafe(no_mangle)]
pub extern "C" fn rr_glob(p: *const c_char, s: *const c_char) -> i32 {
    glob(&arg(p), &arg(s)) as i32
}

#[unsafe(no_mangle)]
pub extern "C" fn rr_iglob(p: *const c_char, s: *const c_char) -> i32 {
    glob(&arg(p).to_lowercase(), &arg(s).to_lowercase()) as i32
}

/// word(w, s): w's words occur in s as whole words, any non-word run between them.
fn word(w: &str, s: &str) -> bool {
    let words: Vec<String> = w.split_whitespace().map(regex::escape).collect();
    if words.is_empty() {
        return false;
    }
    let src = format!(r"(?:^|[^\w])(?:{})(?:$|[^\w])", words.join(r"[^\w]+"));
    with(format!("word\0{w}"), || src, |r| r.is_match(s), false)
}

#[unsafe(no_mangle)]
pub extern "C" fn rr_word(w: *const c_char, s: *const c_char) -> i32 {
    word(&arg(w), &arg(s)) as i32
}

#[unsafe(no_mangle)]
pub extern "C" fn rr_iword(w: *const c_char, s: *const c_char) -> i32 {
    word(&arg(w).to_lowercase(), &arg(s).to_lowercase()) as i32
}

/// @lower: Unicode default lowercase (JavaScript's toLowerCase on everything but final sigma).
#[unsafe(no_mangle)]
pub extern "C" fn lower(s: *const c_char) -> *const c_char {
    intern(&arg(s).to_lowercase())
}

/// rr_find_all(re, s, whole): every match as byte offsets, for rrlab.py's span builder (match_all, and captures that
/// are spans). One match per line, `start end g1start g1end …`, -1 -1 for a group that took no part. whole = 1: the
/// one whole-string match of `match`; 0: the non-overlapping matches of `match_all`.
#[unsafe(no_mangle)]
pub extern "C" fn rr_find_all(re: *const c_char, s: *const c_char, whole_only: i32) -> *const c_char {
    let (re, s) = (arg(re), arg(s));
    let (key, src) = if whole_only != 0 { whole(&re) } else { (format!("find\0{re}"), dialect(&re)) };
    let out = with(key, || src, |r| {
        let mut o = String::new();
        for c in r.captures_iter(&s) {
            for g in c.iter() {
                match g {
                    Some(m) => o.push_str(&format!("{} {} ", m.start(), m.end())),
                    None => o.push_str("-1 -1 "),
                }
            }
            o.push('\n');
        }
        o
    }, String::new());
    // not interned: the caller copies it before the next call
    thread_local!(static LAST: std::cell::RefCell<CString> = std::cell::RefCell::new(CString::default()));
    LAST.with(|l| {
        *l.borrow_mut() = CString::new(out).unwrap_or_default();
        l.borrow().as_ptr()
    })
}
