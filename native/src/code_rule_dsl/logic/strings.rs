//! Text tests (decision 0037): a rule tests a string from a typed text relation (`name`, `str`, `strprefix`,
//! `text`, `doc`'s path and module) against literals or globs, never a regex.
//!
//! ```text
//! private($n)  :- name($n, $s), @prefix($s, "_").
//! cached($n)   :- name($n, $s), @glob($s, {"_*branch_tip*", "_*revision_cache*"}).
//! in_impl($d)  :- doc($d, $p, _), @glob($p, "impl/**/*.py").
//! ddl($n)      :- str($n, $v), @iword($v, {"create table", "drop table"}).
//! public($n)   :- name($n, $s), !@prefix($s, "_").
//! backend($n)  :- str($n, $v), $v in {"postgres", "sqlite"}.
//! ```
//!
//! - **Tests.** `equals`, `prefix`, `suffix`, `contains`, `glob` and `word`, and each ignoring case (`i…`). A test
//!   takes one pattern or a set (`{a, b}`): any of them. `$s in {…}` is `@equals($s, {…})`.
//! - **Globs** are path globs: `*`, `?` and `[…]` (`[!…]` negated) stay within one path segment, `**` spans
//!   segments, `{a,b}` is alternatives. Names hold no `/`, so on a name `*` is any run.
//! - **Words** are runs of ASCII letters, digits and `_`. `@word($s, "on delete")` holds when the words `on`, `delete`
//!   appear one after the other in `$s`, whatever separates them.
//!
//! **How they run.** A test is a relation over strings. The planner turns each test whose string comes from a text
//! relation into an input relation (`as_relations`): the strings of the batch's vocabulary that pass it, each
//! tested once (`Vocabulary`), by a `Matcher` that groups its patterns by the cheapest exact way to match them
//! (a hash set, one multi-string automaton, a glob set, word sets). Rules join on symbols. A test that stays in a
//! rule (its string is computed in the rule, as `@cat`'s) runs per row in SQL as `str_test(op, s, patterns)`, the
//! patterns joined by `SET_SEP`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, LazyLock, Mutex};

use aho_corasick::AhoCorasick;
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use rusqlite::Connection;

use super::front::{Atom, Literal, Program, Term};

/// The tests, by name. `i…`: the same, ignoring case (by lowercasing both sides).
pub(in crate::code_rule_dsl) const TESTS: [&str; 12] =
    ["equals", "prefix", "suffix", "contains", "glob", "word", "iequals", "iprefix", "isuffix", "icontains", "iglob", "iword"];

pub(in crate::code_rule_dsl) fn is_test(f: &str) -> bool {
    TESTS.contains(&f)
}

/// A pattern set's term: `{a, b}` parses as `@set(a, b)`.
pub(in crate::code_rule_dsl) const SET: &str = "set";

/// Between a set's patterns in SQL's `str_test`.
pub(in crate::code_rule_dsl) const SET_SEP: char = '\u{1f}';

/// The prefix of a test's input relation, `strtest_<k>(s)`.
pub(in crate::code_rule_dsl) const RELATION: &str = "strtest_";

/// The relations a text test's string can come from, and their text columns: the batch interns these strings, so a
/// test over them is answered from the batch's vocabulary (`facts::vtab::syntax`).
pub(in crate::code_rule_dsl) const TEXT_COLUMNS: [(&str, &[usize]); 7] =
    [("text", &[1]), ("name", &[1]), ("str", &[1]), ("strprefix", &[1]), ("op", &[1]), ("doc", &[1, 2]), ("pkg", &[1])];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Anchor {
    Prefix,
    Suffix,
    Contains,
}

/// One test over a set of patterns, compiled once: the patterns grouped by the cheapest exact way to match them.
pub(in crate::code_rule_dsl) struct Matcher {
    /// Ignoring case: the string is lowercased once, and the patterns were.
    fold: bool,
    exact: HashSet<String>,
    /// Literal prefix, suffix and substring patterns: one automaton for all, each pattern's anchor.
    literals: Option<(AhoCorasick, Vec<Anchor>)>,
    /// Literal shortcuts of globs (`a*`, `*a`, `*a*`) hold only where the string has no `/` (a glob's `*` stays in
    /// a segment), so globs keep every pattern here too.
    globs: Option<GlobSet>,
    glob_literals: bool,
    /// Some glob is not a literal shortcut: the glob set decides what the shortcuts do not.
    glob_rest: bool,
    /// Word tests: one-word needles by hash, longer ones as sequences.
    words: HashSet<String>,
    phrases: Vec<Vec<String>>,
}

fn has_meta(p: &str) -> bool {
    p.contains(['*', '?', '[', ']', '{', '}', '\\'])
}

/// The words of `s`: runs of ASCII letters, digits and `_`.
fn words(s: &str) -> impl Iterator<Item = &str> {
    s.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).filter(|w| !w.is_empty())
}

impl Matcher {
    /// `op` (a test's name) over `patterns`. An invalid glob is an error.
    pub(in crate::code_rule_dsl) fn new(op: &str, patterns: &[String]) -> Result<Self, String> {
        let fold = op.starts_with('i');
        let base = if fold { &op[1..] } else { op };
        let norm = |p: &String| if fold { p.to_lowercase() } else { p.clone() };
        let mut m = Matcher { fold, exact: HashSet::new(), literals: None, globs: None, glob_literals: false, glob_rest: false, words: HashSet::new(), phrases: Vec::new() };
        let mut lits: Vec<(String, Anchor)> = Vec::new();
        match base {
            "equals" => m.exact.extend(patterns.iter().map(norm)),
            "prefix" => lits.extend(patterns.iter().map(|p| (norm(p), Anchor::Prefix))),
            "suffix" => lits.extend(patterns.iter().map(|p| (norm(p), Anchor::Suffix))),
            "contains" => lits.extend(patterns.iter().map(|p| (norm(p), Anchor::Contains))),
            "word" => {
                for p in patterns {
                    let w: Vec<String> = words(&norm(p)).map(str::to_owned).collect();
                    match w.len() {
                        0 => {}
                        1 => {
                            m.words.insert(w.into_iter().next().expect("one word"));
                        }
                        _ => m.phrases.push(w),
                    }
                }
            }
            "glob" => {
                let mut set = GlobSetBuilder::new();
                for p in patterns {
                    let p = norm(p);
                    // `a`, `a*`, `*a`, `*a*` with a literal `a`: exact where the string has no `/`.
                    let head = p.starts_with('*') && !p.starts_with("**");
                    let tail = p.len() > 1 && p.ends_with('*') && !p.ends_with("**");
                    let core = &p[usize::from(head)..p.len() - usize::from(tail)];
                    if !has_meta(core) && !core.is_empty() {
                        m.glob_literals = true;
                        match (head, tail) {
                            (false, false) => {
                                m.exact.insert(core.to_owned());
                            }
                            (false, true) => lits.push((core.to_owned(), Anchor::Prefix)),
                            (true, false) => lits.push((core.to_owned(), Anchor::Suffix)),
                            (true, true) => lits.push((core.to_owned(), Anchor::Contains)),
                        }
                    } else {
                        m.glob_rest = true;
                    }
                    let glob = GlobBuilder::new(&p).literal_separator(true).backslash_escape(true).build().map_err(|e| format!("glob {p}: {e}"))?;
                    set.add(glob);
                }
                m.globs = Some(set.build().map_err(|e| e.to_string())?);
            }
            _ => return Err(format!("no text test {op}")),
        }
        if !lits.is_empty() {
            let ac = AhoCorasick::new(lits.iter().map(|(p, _)| p.as_str())).map_err(|e| e.to_string())?;
            m.literals = Some((ac, lits.into_iter().map(|(_, a)| a).collect()));
        }
        Ok(m)
    }

    pub(in crate::code_rule_dsl) fn is_match(&self, s: &str) -> bool {
        let folded;
        let s = if self.fold {
            folded = s.to_lowercase();
            folded.as_str()
        } else {
            s
        };
        if let Some(globs) = &self.globs
            && (!self.glob_literals || s.contains('/'))
        {
            return globs.is_match(s);
        }
        if self.exact.contains(s) {
            return true;
        }
        if let Some((ac, anchors)) = &self.literals {
            for found in ac.find_overlapping_iter(s) {
                let ok = match anchors[found.pattern().as_usize()] {
                    Anchor::Contains => true,
                    Anchor::Prefix => found.start() == 0,
                    Anchor::Suffix => found.end() == s.len(),
                };
                if ok {
                    return true;
                }
            }
        }
        if !self.words.is_empty() || !self.phrases.is_empty() {
            let ws: Vec<&str> = words(s).collect();
            if ws.iter().any(|w| self.words.contains(*w)) {
                return true;
            }
            if self.phrases.iter().any(|p| ws.windows(p.len()).any(|w| w.iter().zip(p).all(|(a, b)| a == b))) {
                return true;
            }
        }
        // A glob set with literal shortcuts: the rest of its patterns.
        if let Some(globs) = &self.globs
            && self.glob_rest
        {
            return globs.is_match(s);
        }
        false
    }
}

/// The patterns of a test's second argument: one string, or a set of them.
pub(in crate::code_rule_dsl) fn patterns(t: &Term) -> Option<Vec<String>> {
    match t {
        Term::Str(s) => Some(vec![s.clone()]),
        Term::Call(f, args) if f == SET => args.iter().map(|a| if let Term::Str(s) = a { Some(s.clone()) } else { None }).collect(),
        _ => None,
    }
}

static MATCHERS: LazyLock<Mutex<HashMap<(String, String), Option<Arc<Matcher>>>>> = LazyLock::new(Default::default);

/// Test `op` of `s` against `pattern`, a set's patterns joined by `SET_SEP`. An invalid glob matches nothing.
pub(in crate::code_rule_dsl) fn test(op: &str, s: &str, pattern: &str) -> bool {
    let m = MATCHERS
        .lock()
        .unwrap()
        .entry((op.to_owned(), pattern.to_owned()))
        .or_insert_with(|| Matcher::new(op, &pattern.split(SET_SEP).map(str::to_owned).collect::<Vec<_>>()).ok().map(Arc::new))
        .clone();
    m.is_some_and(|m| m.is_match(s))
}

/// `str_test(op, s, patterns)`: 1 or 0; NULL gives NULL.
pub(in crate::code_rule_dsl) fn register(db: &Connection) -> rusqlite::Result<()> {
    use rusqlite::functions::FunctionFlags;
    db.create_scalar_function(c"str_test", 3, FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC, |ctx| {
        let (Some(op), Some(s), Some(p)) = (ctx.get::<Option<String>>(0)?, ctx.get::<Option<String>>(1)?, ctx.get::<Option<String>>(2)?) else {
            return Ok(None);
        };
        Ok(Some(i64::from(test(&op, &s, &p))))
    })
}

/// A text test in a rule body: `@op($s, patterns) = 1` (holds) or `= 0` (fails).
fn as_test(l: &Literal) -> Option<(&str, &str, Vec<String>, bool)> {
    let Literal::Cmp { l: Term::Call(op, args), op: "=", r: Term::Int(v @ (0 | 1)) } = l else { return None };
    let [Term::Var(s), p] = args.as_slice() else { return None };
    is_test(op).then_some(())?;
    Some((op.as_str(), s.as_str(), patterns(p)?, *v == 1))
}

/// The text tests a run answers from the vocabulary: each input relation `strtest_<k>(s)` and its matcher.
#[derive(Default)]
pub(in crate::code_rule_dsl) struct Vocabulary {
    pub(in crate::code_rule_dsl) tests: Vec<(String, Arc<Matcher>)>,
    /// Per symbol, the tests it passes (bit k is test k): each string is tested once per run.
    seen: Mutex<HashMap<u32, Box<[u64]>>>,
    /// The strings tested, and the CPU (user + system seconds) testing them took.
    pub(in crate::code_rule_dsl) tested: Mutex<(usize, f64)>,
}

impl Vocabulary {
    /// For each test, the symbols among `strings` (a batch's distinct strings and their symbols) that pass it.
    pub(in crate::code_rule_dsl) fn rows(&self, strings: &[String], symbols: &[u32]) -> Vec<Vec<i64>> {
        let mut out = vec![Vec::new(); self.tests.len()];
        if self.tests.is_empty() {
            return out;
        }
        let words = self.tests.len().div_ceil(64);
        let mut seen = self.seen.lock().unwrap();
        let (mut tested, t0) = (0, super::plan::corpus::thread_usage());
        for (s, &sym) in strings.iter().zip(symbols) {
            let bits = seen.entry(sym).or_insert_with(|| {
                tested += 1;
                let mut bits = vec![0u64; words].into_boxed_slice();
                for (k, (_, m)) in self.tests.iter().enumerate() {
                    if m.is_match(s) {
                        bits[k / 64] |= 1 << (k % 64);
                    }
                }
                bits
            });
            for (k, rows) in out.iter_mut().enumerate() {
                if bits[k / 64] >> (k % 64) & 1 == 1 {
                    rows.push(i64::from(sym));
                }
            }
        }
        let t1 = super::plan::corpus::thread_usage();
        let mut total = self.tested.lock().unwrap();
        total.0 += tested;
        total.1 += (t1.0 - t0.0) + (t1.1 - t0.1);
        out
    }
}

/// Turns each text test whose string a text relation of the same rule gives (`TEXT_COLUMNS`) into an atom over
/// the input relation `strtest_<k>(s)`, negated where the test is: rules then join on symbols, and the run fills
/// each relation from the vocabulary (decision 0037). Equal tests share one relation. The others stay per row.
pub(in crate::code_rule_dsl) fn as_relations(prog: &mut Program) -> Result<Vocabulary, String> {
    let mut ids: BTreeMap<(String, Vec<String>), String> = BTreeMap::new();
    let mut tests = Vec::new();
    for rule in &mut prog.rules {
        let texts: HashSet<String> = rule
            .body
            .iter()
            .filter_map(|l| match l {
                Literal::Atom(a) if !a.neg => Some(a),
                _ => None,
            })
            .flat_map(|a| {
                let cols = TEXT_COLUMNS.iter().find(|(r, _)| *r == a.rel).map_or(&[][..], |(_, c)| *c);
                cols.iter().filter_map(|&c| if let Some(Term::Var(v)) = a.args.get(c) { Some(v.clone()) } else { None })
            })
            .collect();
        for literal in &mut rule.body {
            let Some((op, s, mut pats, holds)) = as_test(literal) else { continue };
            if !texts.contains(s) {
                continue;
            }
            pats.sort();
            pats.dedup();
            let key = (op.to_owned(), pats);
            let rel = match ids.get(&key) {
                Some(r) => r.clone(),
                None => {
                    let rel = format!("{RELATION}{}", ids.len());
                    tests.push((rel.clone(), Arc::new(Matcher::new(&key.0, &key.1)?)));
                    ids.insert(key, rel.clone());
                    rel
                }
            };
            *literal = Literal::Atom(Atom { rel, args: vec![Term::Var(s.to_owned())], neg: !holds, gated: false });
        }
    }
    for (rel, _) in &tests {
        prog.decls.insert(rel.clone(), vec![("s".to_owned(), "sym".to_owned())]);
        if !prog.inputs.contains(rel) {
            prog.inputs.push(rel.clone());
        }
    }
    Ok(Vocabulary { tests, seen: Mutex::default(), tested: Mutex::default() })
}

#[cfg(test)]
mod tests;
