//! A Metal source split into units of their own, one a `UKL_ONCE` function (decision 0571).
//!
//! Metal's back end inlines a function with a threadgroup barrier at every call site, whatever its `noinline`, and
//! inside functions that are themselves inlined: a kernel of many barriered phase functions compiles as one function
//! many times its size (`pp_setup`: 166 s on the M3). A dynamic library is the one boundary it does not inline
//! across. So each such function is a unit of its own, compiled once as a dynamic library, and the kernel links them.
//!
//! - **A unit** is the source with its function defined and exported, every other unit's function an `extern`
//!   declaration, and no kernel. It links the units it calls (`calls`), so the units are compiled by `level`: a
//!   level's callees are all in lower levels.
//! - **Kept local:** a `UKL_ONCE` function with no threadgroup barrier in its reach (itself, the helpers and the
//!   functions it calls) and under `LOCAL_MOST` bytes with its helpers stays in every unit that calls it. A call into
//!   a dynamic library costs more than one within a unit, and a small read called a thread an element (the tape's
//!   reads) pays it every element; without a barrier the unit does not inline it at every site.
//! - **The kernel** is the source with every unit's function an `extern` declaration.
//! - **`depth`** is the deepest chain of `UKL_ONCE` calls from a kernel: the pipeline's `maxCallStackDepth` (Metal's
//!   default of 1 page-faults once a dynamic library calls another).
//!
//! The text is the front end's (`metal_source`, macros expanded). The scan knows C's comments, literals and line
//! markers, and the source's top level: a name, its parameters, then a body (a definition) or `;` (a declaration).

use std::collections::{HashMap, HashSet};

/// `UKL_ONCE` as Metal's front end reads it (`ukl/backend.h`).
pub const ONCE: &str = "static __attribute__((noinline)) ";

/// The bytes, with its helpers, under which a barrier-free `UKL_ONCE` function stays in its callers' units.
pub const LOCAL_MOST: usize = 8192;

/// The calls that make a function barriered: Metal inlines it at every call site.
const BARRIERS: [&str; 3] = ["threadgroup_barrier", "ukl_tg_barrier", "ukl_tg_barrier_device"];

/// A unit of its own: one function's dynamic library.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unit {
    pub name: String,
    pub source: String,
    /// The units it calls, which it links.
    pub calls: Vec<String>,
    /// Its level: 0 calls no unit; a level's callees are in lower levels.
    pub level: usize,
}

/// A source split: its units, the kernel's source that links them, and the pipeline's call depth.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Split {
    pub units: Vec<Unit>,
    pub kernel: String,
    pub depth: u64,
    /// The `UKL_ONCE` functions kept in their callers' units.
    pub local: Vec<String>,
}

impl Split {
    /// The levels, lowest first: each one's units compile at once.
    pub fn levels(&self) -> Vec<Vec<&Unit>> {
        let top = self.units.iter().map(|u| u.level).max().map_or(0, |l| l + 1);
        (0..top).map(|l| self.units.iter().filter(|u| u.level == l).collect()).collect()
    }
}

/// A `UKL_ONCE` function's definition: where its `ONCE` begins, its signature after it, the parameters' end, and its
/// body.
struct Once {
    name: String,
    start: usize,
    sig: usize,
    close: usize,
    body: (usize, usize),
}

/// `msl` split, or `None` where no `UKL_ONCE` function is a unit of its own.
pub fn split(msl: &str) -> Option<Split> {
    let s = msl.as_bytes();
    let every = definitions(msl);
    let mut once: Vec<Once> = Vec::new();
    let mut declared: Vec<(usize, usize, String)> = Vec::new();
    let mut at = 0;
    let mut depth = Depth::new(s);
    while let Some(found) = msl[at..].find(ONCE) {
        let start = at + found;
        let sig = start + ONCE.len();
        at = sig;
        if depth.at(start) != 0 {
            continue;
        }
        let paren = sig + msl[sig..].find('(')?;
        let name = trailing_ident(&msl[sig..paren])?.to_owned();
        let close = matching(s, paren)?;
        let after = skip_trivia(s, close);
        match s.get(after) {
            Some(b';') => declared.push((start, sig, name)),
            Some(b'{') => once.push(Once { name, start, sig, close, body: (after, matching(s, after)?) }),
            _ => return None,
        }
    }
    if once.is_empty() {
        return None;
    }
    let mut bodies: HashMap<&str, &str> = every.iter().map(|(&n, &(a, b))| (n, &msl[a..b])).collect();
    bodies.extend(once.iter().map(|o| (o.name.as_str(), &msl[o.body.0..o.body.1])));
    let body = |n: &str| -> &str { bodies.get(n).copied().unwrap_or("") };
    let is_once: HashSet<&str> = once.iter().map(|o| o.name.as_str()).collect();
    // Each function's calls of defined functions, and whether it calls a barrier itself: found once.
    let mut calls: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut barriers: HashSet<&str> = HashSet::new();
    for (&f, &b) in &bodies {
        let all = called(b);
        if all.iter().any(|c| BARRIERS.contains(c)) {
            barriers.insert(f);
        }
        calls.insert(f, all.into_iter().filter(|c| bodies.contains_key(c)).collect());
    }
    let callees = |f: &str| calls.get(f).map_or(&[][..], Vec::as_slice);
    // The `UKL_ONCE` functions `n` reaches through helpers, stopping at each.
    let reached_once = |n: &str| -> Vec<String> {
        let (mut seen, mut todo, mut out) = (HashSet::from([n]), vec![n], HashSet::new());
        while let Some(f) = todo.pop() {
            for &c in callees(f) {
                if !seen.insert(c) {
                    continue;
                }
                if is_once.contains(c) {
                    out.insert(c);
                } else {
                    todo.push(c);
                }
            }
        }
        once.iter().map(|o| o.name.clone()).filter(|o| out.contains(o.as_str())).collect()
    };
    // Local: no barrier in its whole reach, and its own and its helpers' bytes under `LOCAL_MOST`.
    let mut local: HashSet<String> = HashSet::new();
    for o in &once {
        let (mut seen, mut todo) = (HashSet::from([o.name.as_str()]), vec![o.name.as_str()]);
        let (mut size, mut bar) = (o.body.1 - o.body.0, barriers.contains(o.name.as_str()));
        while let Some(f) = todo.pop() {
            for &c in callees(f) {
                if seen.insert(c) {
                    if !is_once.contains(c) {
                        size += body(c).len();
                    }
                    bar |= barriers.contains(c);
                    todo.push(c);
                }
            }
        }
        if !bar && size < LOCAL_MOST {
            local.insert(o.name.clone());
        }
    }
    let deps: HashMap<String, Vec<String>> = once.iter().map(|o| (o.name.clone(), reached_once(&o.name))).collect();
    // A local function that calls a unit's is no longer local: the unit's callers link it.
    while let Some(n) = local.iter().find(|n| deps[*n].iter().any(|c| !local.contains(c))).cloned() {
        local.remove(&n);
    }
    let units: Vec<&Once> = once.iter().filter(|o| !local.contains(&o.name)).collect();
    if units.is_empty() {
        return None;
    }
    let unit_calls = |n: &str| -> Vec<String> { deps[n].iter().filter(|c| !local.contains(*c)).cloned().collect() };
    let mut level: HashMap<String, usize> = HashMap::new();
    while level.len() < units.len() {
        let before = level.len();
        for u in &units {
            if !level.contains_key(&u.name) && unit_calls(&u.name).iter().all(|c| level.contains_key(c)) {
                let l = unit_calls(&u.name).iter().map(|c| level[c] + 1).max().unwrap_or(0);
                level.insert(u.name.clone(), l);
            }
        }
        if level.len() == before {
            return None; // A cycle: Metal has no recursion, and the source would not compile anyway.
        }
    }
    // The source with `keep` defined and exported, every other unit's function an `extern` declaration.
    let edited = |keep: Option<&str>| -> String {
        let mut edits: Vec<(usize, usize, String)> = Vec::new();
        for u in &units {
            if Some(u.name.as_str()) == keep {
                edits.push((u.start, u.sig, String::new()));
            } else {
                edits.push((u.start, u.body.1, format!("{};", &msl[u.sig..u.close])));
            }
        }
        edits.extend(declared.iter().filter(|(_, _, n)| !local.contains(n)).map(|&(a, b, _)| (a, b, String::new())));
        edits.sort();
        let mut out = String::with_capacity(msl.len());
        let mut at = 0;
        for (a, b, text) in edits {
            out.push_str(&msl[at..a]);
            out.push_str(&text);
            at = b;
        }
        out.push_str(&msl[at..]);
        out
    };
    // The deepest chain of `UKL_ONCE` calls from a kernel (each call a frame).
    let mut chain: HashMap<String, u64> = HashMap::new();
    fn longest(n: &str, deps: &HashMap<String, Vec<String>>, chain: &mut HashMap<String, u64>) -> u64 {
        if let Some(&d) = chain.get(n) {
            return d;
        }
        let d = 1 + deps[n].iter().map(|c| longest(c, deps, chain)).max().unwrap_or(0);
        chain.insert(n.to_owned(), d);
        d
    }
    let depth = kernels(msl).iter().flat_map(|k| reached_once(k)).map(|f| longest(&f, &deps, &mut chain)).max().unwrap_or(1);
    Some(Split {
        units: units.iter().map(|u| Unit { name: u.name.clone(), source: without_kernels(&edited(Some(&u.name))), calls: unit_calls(&u.name), level: level[&u.name] }).collect(),
        kernel: edited(None),
        depth,
        local: once.iter().filter(|o| local.contains(&o.name)).map(|o| o.name.clone()).collect(),
    })
}

/// Past `i`'s whitespace, comments and line markers (a `#` line).
fn skip_trivia(s: &[u8], mut i: usize) -> usize {
    loop {
        match s.get(i) {
            Some(c) if c.is_ascii_whitespace() => i += 1,
            Some(b'#') if i == 0 || s[i - 1] == b'\n' => i = line_end(s, i),
            Some(b'/') if s.get(i + 1) == Some(&b'/') => i = line_end(s, i),
            Some(b'/') if s.get(i + 1) == Some(&b'*') => i = comment_end(s, i),
            _ => return i,
        }
    }
}

fn line_end(s: &[u8], i: usize) -> usize {
    s[i..].iter().position(|&c| c == b'\n').map_or(s.len(), |p| i + p + 1)
}

fn comment_end(s: &[u8], i: usize) -> usize {
    s[i + 2..].windows(2).position(|w| w == b"*/").map_or(s.len(), |p| i + 2 + p + 2)
}

/// Past the literal that begins at `i` (`"` or `'`).
fn literal_end(s: &[u8], i: usize) -> usize {
    let q = s[i];
    let mut j = i + 1;
    while j < s.len() && s[j] != q {
        j += if s[j] == b'\\' { 2 } else { 1 };
    }
    (j + 1).min(s.len())
}

/// Past what is not code at `i` (a comment, a literal, a line marker), or `None` where code is.
fn not_code(s: &[u8], i: usize) -> Option<usize> {
    match s[i] {
        b'/' if s.get(i + 1) == Some(&b'/') => Some(line_end(s, i)),
        b'/' if s.get(i + 1) == Some(&b'*') => Some(comment_end(s, i)),
        b'"' | b'\'' => Some(literal_end(s, i)),
        b'#' if i == 0 || s[i - 1] == b'\n' => Some(line_end(s, i)),
        _ => None,
    }
}

/// Past the bracket that closes the one at `open` (`(` or `{`).
fn matching(s: &[u8], open: usize) -> Option<usize> {
    let (o, c) = (s[open], if s[open] == b'(' { b')' } else { b'}' });
    let (mut d, mut i) = (0usize, open);
    while i < s.len() {
        if let Some(j) = not_code(s, i) {
            i = j;
            continue;
        }
        if s[i] == o {
            d += 1;
        } else if s[i] == c {
            d -= 1;
            if d == 0 {
                return Some(i + 1);
            }
        }
        i += 1;
    }
    None
}

/// The brace depth at positions asked in order: one pass over the text however many are asked.
struct Depth<'a> {
    s: &'a [u8],
    i: usize,
    d: usize,
}

impl<'a> Depth<'a> {
    fn new(s: &'a [u8]) -> Self {
        Depth { s, i: 0, d: 0 }
    }

    /// The depth at `at` (at least the last position asked; an earlier one is the depth where the scan is, as a
    /// position inside a comment or literal it skipped is not code).
    fn at(&mut self, at: usize) -> usize {
        while self.i < at {
            if let Some(j) = not_code(self.s, self.i) {
                self.i = j;
                continue;
            }
            match self.s[self.i] {
                b'{' => self.d += 1,
                b'}' => self.d = self.d.saturating_sub(1),
                _ => {}
            }
            self.i += 1;
        }
        // A position the scan passed inside a comment or literal: not code, never at the top level.
        if self.i > at { usize::MAX } else { self.d }
    }
}

fn is_ident(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// The identifier `text` ends with (a declarator's name, after its type and any `*` or `&`).
fn trailing_ident(text: &str) -> Option<&str> {
    let t = text.trim_end();
    let from = t.bytes().rposition(|c| !is_ident(c)).map_or(0, |p| p + 1);
    let name = &t[from..];
    (!name.is_empty() && !name.as_bytes()[0].is_ascii_digit()).then_some(name)
}

/// Every function defined at the top level, by name (its first definition): its body's range, braces included.
fn definitions(msl: &str) -> HashMap<&str, (usize, usize)> {
    let s = msl.as_bytes();
    let mut out = HashMap::new();
    let (mut d, mut i) = (0usize, 0);
    while i < s.len() {
        if let Some(j) = not_code(s, i) {
            i = j;
            continue;
        }
        let c = s[i];
        if c == b'{' {
            d += 1;
        } else if c == b'}' {
            d = d.saturating_sub(1);
        } else if d == 0 && (c.is_ascii_alphabetic() || c == b'_') && (i == 0 || !is_ident(s[i - 1])) {
            let end = i + s[i..].iter().position(|&c| !is_ident(c)).unwrap_or(s.len() - i);
            let k = skip_trivia(s, end);
            if s.get(k) == Some(&b'(') {
                if let Some(close) = matching(s, k) {
                    let r = skip_trivia(s, close);
                    if s.get(r) == Some(&b'{') {
                        if let Some(stop) = matching(s, r) {
                            out.entry(&msl[i..end]).or_insert((r, stop));
                            i = stop;
                            continue;
                        }
                    }
                }
            }
            i = end;
            continue;
        }
        i += 1;
    }
    out
}

/// The names called in `text`: each identifier followed by `(`.
fn called(text: &str) -> Vec<&str> {
    let s = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < s.len() {
        if let Some(j) = not_code(s, i) {
            i = j;
            continue;
        }
        if (s[i].is_ascii_alphabetic() || s[i] == b'_') && (i == 0 || !is_ident(s[i - 1])) {
            let end = i + s[i..].iter().position(|&c| !is_ident(c)).unwrap_or(s.len() - i);
            let k = s[end..].iter().position(|c| !c.is_ascii_whitespace()).map_or(s.len(), |p| end + p);
            if s.get(k) == Some(&b'(') {
                out.push(&text[i..end]);
            }
            i = end;
            continue;
        }
        i += 1;
    }
    out
}

/// Where each `kernel void NAME(` begins (any `[[...]]` attributes before it included), its name, and past its body.
fn kernel_spans(msl: &str) -> Vec<(usize, &str, usize)> {
    let s = msl.as_bytes();
    let mut out = Vec::new();
    let mut at = 0;
    let mut depth = Depth::new(s);
    while let Some(found) = msl[at..].find("kernel") {
        let k = at + found;
        at = k + "kernel".len();
        if (k > 0 && is_ident(s[k - 1])) || s.get(at).is_some_and(|&c| is_ident(c)) || depth.at(k) != 0 {
            continue;
        }
        let v = skip_trivia(s, at);
        if !msl[v..].starts_with("void") {
            continue;
        }
        let Some(paren) = msl[v..].find('(').map(|p| v + p) else { continue };
        let Some(name) = trailing_ident(&msl[v + 4..paren]) else { continue };
        let Some(close) = matching(s, paren) else { continue };
        let r = skip_trivia(s, close);
        if s.get(r) != Some(&b'{') {
            continue;
        }
        let Some(end) = matching(s, r) else { continue };
        // Attributes before it (`[[max_total_threads_per_threadgroup(N)]] kernel`) go with it.
        let mut start = k;
        let before = msl[..k].trim_end();
        if before.ends_with("]]") {
            if let Some(open) = before.rfind("[[") {
                start = open;
            }
        }
        out.push((start, name, end));
        at = end;
    }
    out
}

fn kernels(msl: &str) -> Vec<&str> {
    kernel_spans(msl).into_iter().map(|(_, n, _)| n).collect()
}

fn without_kernels(msl: &str) -> String {
    let mut out = String::with_capacity(msl.len());
    let mut at = 0;
    for (start, _, end) in kernel_spans(msl) {
        out.push_str(&msl[at..start]);
        at = end;
    }
    out.push_str(&msl[at..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCE: &str = r#"#include <metal_stdlib>
using namespace metal;
// A comment with a brace { and a call fake_call( that is not code.
static inline uint helper(uint x) { return x + 1u; }
static __attribute__((noinline)) uint low(threadgroup uint* t, uint x);
static __attribute__((noinline)) uint low(threadgroup uint* t, uint x) {
    t[0] = helper(x);
    threadgroup_barrier(mem_flags::mem_threadgroup);
    return t[0];
}
static __attribute__((noinline)) uint read_one(device const uint* d, uint i) { return d[i] + helper(i); }
static __attribute__((noinline)) uint high(threadgroup uint* t, device const uint* d, uint x) {
    const char* s = "a } brace in a string";
    return low(t, read_one(d, x)) + (s[0] == '}' ? 1u : 0u);
}
[[max_total_threads_per_threadgroup(256)]] kernel void k(device uint* out [[buffer(0)]], device const uint* d [[buffer(1)]], uint i [[thread_position_in_grid]]) {
    threadgroup uint t[1];
    out[i] = high(t, d, i) + read_one(d, i);
}
"#;

    /// Barriered functions are units of their own, by level; a small barrier-free one stays in each unit; the
    /// kernel declares the units' functions; the depth is the deepest chain of calls.
    #[test]
    fn a_source_splits_one_unit_a_barriered_function() {
        let s = split(SOURCE).expect("two barriered functions");
        let names: Vec<&str> = s.units.iter().map(|u| u.name.as_str()).collect();
        assert_eq!(names, ["low", "high"]);
        assert_eq!(s.local, ["read_one"]);
        let (low, high) = (&s.units[0], &s.units[1]);
        assert_eq!((low.level, low.calls.as_slice()), (0, &[][..]));
        assert_eq!((high.level, high.calls.as_slice()), (1, &["low".to_owned()][..]));
        assert_eq!(s.levels().iter().map(Vec::len).collect::<Vec<_>>(), [1, 1]);
        // kernel → high → low (read_one's chain is shorter).
        assert_eq!(s.depth, 2);
        // The kernel: both units' functions declared, `extern` (no `static noinline`), the local one defined.
        assert!(s.kernel.contains("uint low(threadgroup uint* t, uint x);\n"), "{}", s.kernel);
        assert!(s.kernel.contains("uint high(threadgroup uint* t, device const uint* d, uint x);"), "{}", s.kernel);
        assert!(!s.kernel.contains("threadgroup_barrier"), "a unit's body left in the kernel");
        assert!(s.kernel.contains(&format!("{ONCE}uint read_one(device const uint* d, uint i) {{")));
        assert!(s.kernel.contains("kernel void k("));
        // A unit: its function exported and defined, the other declared, no kernel (nor its attribute).
        assert!(high.source.contains("\nuint high(threadgroup uint* t, device const uint* d, uint x) {"), "{}", high.source);
        assert!(high.source.contains("uint low(threadgroup uint* t, uint x);"));
        assert!(!high.source.contains("threadgroup_barrier"));
        assert!(!high.source.contains("kernel") && !high.source.contains("max_total_threads"), "{}", high.source);
        assert!(low.source.contains("threadgroup_barrier") && low.source.contains("static inline uint helper"));
    }

    /// A barrier-free function that calls a unit's function is a unit too: its callers link that unit.
    #[test]
    fn a_function_that_calls_a_unit_is_a_unit() {
        let src = SOURCE.replace("{ return d[i] + helper(i); }", "{ threadgroup uint t[1]; return d[i] + low(t, i); }");
        let s = split(&src).unwrap();
        let names: Vec<&str> = s.units.iter().map(|u| u.name.as_str()).collect();
        assert_eq!(names, ["low", "read_one", "high"]);
        assert!(s.local.is_empty());
        assert_eq!(s.units[2].calls, ["low", "read_one"]);
        assert_eq!(s.depth, 3);
    }

    /// No `UKL_ONCE` function past the local rule: no split.
    #[test]
    fn a_source_without_barriered_functions_is_not_split() {
        assert_eq!(split("kernel void k(device uint* o [[buffer(0)]]) { o[0] = 1u; }\n"), None);
        let small = "static __attribute__((noinline)) uint f(uint x) { return x; }\nkernel void k(device uint* o [[buffer(0)]]) { o[0] = f(1u); }\n";
        assert_eq!(split(small), None);
    }
}
