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
//! - **Regexes** (`@rx`, `@irx`, Rust `regex` syntax) are allowed and cost what their best rewrite costs: every test
//!   is lowered by the text layer (`text`, decision 0042) to one canonical plan, so a regex that is a literal set or
//!   a glob in disguise runs as one.
//!
//! **How they run.** A test is a relation over strings. The planner turns each test whose string comes from a text
//! relation into an input relation (`as_relations`): the strings of the batch's vocabulary that pass it, each
//! tested once (`Vocabulary`), all tests' plans together by the text layer's vocabulary engine (one multi-literal
//! searcher per anchor, automata in small groups). Rules join on symbols. A test that stays in a rule (its string is
//! computed in the rule, as `@cat`'s) runs per row in SQL as `str_test(op, s, patterns)`, the patterns joined by
//! `SET_SEP`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, LazyLock, Mutex};

use crate::code_rule_dsl::text::{Domain, Lowered, Source, Spelling, lower};
use rusqlite::Connection;

use super::front::{Atom, Literal, Program, Term};

/// The tests, by name. `i…`: the same, ignoring case (the text layer's case folding). `rx`: a regex in Rust `regex`
/// syntax (with backreferences beyond it, as `value.regex`).
pub(in crate::code_rule_dsl) const TESTS: [&str; 14] =
    ["equals", "prefix", "suffix", "contains", "glob", "word", "rx", "iequals", "iprefix", "isuffix", "icontains", "iglob", "iword", "irx"];

pub(in crate::code_rule_dsl) fn is_test(f: &str) -> bool {
    TESTS.contains(&f)
}

/// A pattern set's term: `{a, b}` parses as `@set(a, b)`.
pub(in crate::code_rule_dsl) const SET: &str = "set";

/// Between a set's patterns in SQL's `str_test`.
pub(in crate::code_rule_dsl) const SET_SEP: char = '\u{1f}';

/// The prefix of a test's input relation, `strtest_<k>(s)`.
pub(in crate::code_rule_dsl) const RELATION: &str = "strtest_";

/// The prefix of a `match_all` built-in's input relation, `matchall_<k>(sp, m)` (`as_relations`).
pub(in crate::code_rule_dsl) const MATCHES: &str = "matchall_";

/// The prefix of a `match_group` built-in's input relation, `matchgroup_<k>(sp, m, g, gsp)` (`as_relations`).
pub(in crate::code_rule_dsl) const GROUPS: &str = "matchgroup_";

/// The relations a text test's string can come from, and their text columns: the batch interns these strings, so a
/// test over them is answered from the batch's vocabulary (`facts::vtab::syntax`).
pub(in crate::code_rule_dsl) const TEXT_COLUMNS: [(&str, &[usize]); 28] = [
    ("text", &[1]),
    ("name", &[1]),
    ("str", &[1]),
    ("strprefix", &[1]),
    ("op", &[1]),
    ("doc", &[1, 2]),
    ("pkg", &[1]),
    ("node_text", &[1]),
    ("op_at", &[2]),
    ("prev_token", &[1]),
    ("token", &[2]),
    ("path_part", &[1]),
    ("file_name", &[1]),
    ("value", &[1]),
    ("static_value", &[1]),
    ("value_type_of", &[1]),
    ("value_type", &[0, 1]),
    ("comment", &[1]),
    ("fstring_piece", &[2, 3]),
    ("fstring_template", &[1, 2]),
    ("cpython_syntax_error", &[2]),
    ("parsed", &[1]),
    ("literal_json", &[1]),
    ("cpython_unparse", &[1]),
    ("cpython_ast_dump", &[1]),
    ("dump_sha256", &[1]),
    ("body_sha256", &[1]),
    ("signature_sha256", &[1]),
];

/// One test over a set of patterns, compiled once: each pattern lowered by the text layer to its canonical plan (an
/// exact set, an affix set, an automaton), equal patterns' plans shared by key.
pub(in crate::code_rule_dsl) struct Matcher {
    pub(in crate::code_rule_dsl) plans: Vec<Arc<Lowered>>,
}

/// The words of a phrase: runs of ASCII letters, digits and `_`.
fn words(s: &str) -> impl Iterator<Item = &str> {
    s.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).filter(|w| !w.is_empty())
}

/// `@word`'s pattern as a regex: the words one after the other, whole, whatever separates them.
fn word_regex(phrase: &str) -> Option<String> {
    let ws: Vec<String> = words(phrase).map(regex_syntax::escape).collect();
    (!ws.is_empty()).then(|| format!("(?:^|[^A-Za-z0-9_]){}(?:[^A-Za-z0-9_]|$)", ws.join("[^A-Za-z0-9_]+")))
}

impl Matcher {
    /// `op` (a test's name) over `patterns`. A pattern that does not parse is an error.
    pub(in crate::code_rule_dsl) fn new(op: &str, patterns: &[String]) -> Result<Self, String> {
        let fold = op.starts_with('i') && TESTS.contains(&&op[1..]);
        let base = if fold { &op[1..] } else { op };
        let sources: Vec<Source> = match base {
            "equals" => vec![Source::of(Spelling::OneOf(patterns.to_vec()))],
            "prefix" => patterns.iter().map(|p| Source::of(Spelling::Prefix(p.clone()))).collect(),
            "suffix" => patterns.iter().map(|p| Source::of(Spelling::Suffix(p.clone()))).collect(),
            "contains" => patterns.iter().map(|p| Source::of(Spelling::Contains(p.clone()))).collect(),
            "glob" => patterns.iter().map(|p| Source::of(Spelling::PathGlob(p.clone()))).collect(),
            "rx" => patterns.iter().map(|p| Source::of(Spelling::Regex(p.clone()))).collect(),
            "word" => patterns.iter().filter_map(|p| word_regex(p)).map(|r| Source::of(Spelling::Regex(r))).collect(),
            _ => return Err(format!("no text test {op}")),
        };
        let mut plans: Vec<Arc<Lowered>> = Vec::new();
        for source in sources {
            let source = if fold { source.folded() } else { source };
            let lowered = lower(&source, Domain::ANY).map_err(|e| format!("@{op}: {}", e.0))?;
            if !plans.iter().any(|p| p.key == lowered.key) {
                plans.push(lowered);
            }
        }
        Ok(Matcher { plans })
    }

    /// How the patterns are matched (EXPLAIN): each plan's production, with the rewrites that led to it.
    pub(in crate::code_rule_dsl) fn describe(&self) -> String {
        let mut by: BTreeMap<String, usize> = BTreeMap::new();
        for p in &self.plans {
            let how = if p.rewrites.is_empty() { p.production.to_owned() } else { format!("{} after {}", p.production, p.rewrites.join(", ")) };
            *by.entry(how).or_default() += 1;
        }
        let parts: Vec<String> = by.into_iter().map(|(how, n)| if n == 1 { how } else { format!("{n} × {how}") }).collect();
        format!("text layer: {}", parts.join(" + "))
    }

    /// A pattern undecided (a backtracking one past its limits) does not match.
    pub(in crate::code_rule_dsl) fn is_match(&self, s: &str) -> bool {
        self.plans.iter().any(|p| p.compiled().is_match(s.as_bytes()) == Some(true))
    }
}

/// A test's patterns: one string, or a set `{a, b}`.
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

static GROUP_REGEXES: LazyLock<Mutex<HashMap<String, Option<Arc<regex::Regex>>>>> = LazyLock::new(Default::default);

/// The text of named group `name` of regex `re`'s match of `s` (`.rr`'s `match("(?<x>…)", s)`); none when `re`
/// does not match, the group took no part, or `re` is invalid.
pub(in crate::code_rule_dsl) fn group(s: &str, re: &str, name: &str) -> Option<String> {
    let rx = GROUP_REGEXES.lock().unwrap().entry(re.to_owned()).or_insert_with(|| regex::Regex::new(re).ok().map(Arc::new)).clone()?;
    rx.captures(s)?.name(name).map(|m| m.as_str().to_owned())
}

/// `str_test(op, s, patterns)`: 1 or 0; `rx_group(s, re, name)`: the named group's text (`group`); and `core.rr`'s
/// text functors (`functions`). NULL gives NULL.
pub(in crate::code_rule_dsl) fn register(db: &Connection) -> rusqlite::Result<()> {
    use rusqlite::functions::FunctionFlags;
    db.create_scalar_function(c"str_test", 3, FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC, |ctx| {
        let (Some(op), Some(s), Some(p)) = (ctx.get::<Option<String>>(0)?, ctx.get::<Option<String>>(1)?, ctx.get::<Option<String>>(2)?) else {
            return Ok(None);
        };
        Ok(Some(i64::from(test(&op, &s, &p))))
    })?;
    db.create_scalar_function(c"rx_group", 3, FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC, |ctx| {
        let (Some(s), Some(re), Some(name)) = (ctx.get::<Option<String>>(0)?, ctx.get::<Option<String>>(1)?, ctx.get::<Option<String>>(2)?) else {
            return Ok(None);
        };
        Ok(group(&s, &re, &name))
    })?;
    functions::register(db)
}

/// A text test in a rule body: `@op($s, patterns) = 1` (holds) or `= 0` (fails).
fn as_test(l: &Literal) -> Option<(&str, &str, Vec<String>, bool)> {
    let Literal::Cmp { l: Term::Call(op, args), op: "=", r: Term::Int(v @ (0 | 1)) } = l else { return None };
    let [Term::Var(s), p] = args.as_slice() else { return None };
    is_test(op).then_some(())?;
    Some((op.as_str(), s.as_str(), patterns(p)?, *v == 1))
}

/// The text tests a run answers from the vocabulary: each input relation `strtest_<k>(s)` and its matcher; the
/// regexes of `match_all` and `match_group` the batches find in their spans, regex `k`'s matches in
/// `matchall_<k>(sp, m)` and their groups in `matchgroup_<k>(sp, m, g, gsp)`; and the interpolation markers the
/// batches make `fstring_template(n, marker, v)` rows for.
#[derive(Default)]
pub(in crate::code_rule_dsl) struct Vocabulary {
    pub(in crate::code_rule_dsl) tests: Vec<(String, Arc<Matcher>)>,
    pub(in crate::code_rule_dsl) matches: Vec<Arc<regex::Regex>>,
    /// The constant markers of the program's `fstring_template` atoms, distinct, in order.
    pub(in crate::code_rule_dsl) markers: Vec<String>,
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
        // The strings not seen before, tested by every test's plans at once (the text layer's vocabulary engine).
        let mut fresh: Vec<(u32, &str)> = Vec::new();
        let mut queued: HashSet<u32> = HashSet::new();
        for (s, &sym) in strings.iter().zip(symbols) {
            if !seen.contains_key(&sym) && queued.insert(sym) {
                fresh.push((sym, s.as_str()));
            }
        }
        if !fresh.is_empty() {
            tested += fresh.len();
            let (plans, owner): (Vec<&crate::code_rule_dsl::text::Plan>, Vec<usize>) =
                self.tests.iter().enumerate().flat_map(|(k, (_, m))| m.plans.iter().map(move |p| (&p.plan, k))).unzip();
            let terms: Vec<&[u8]> = fresh.iter().map(|(_, s)| s.as_bytes()).collect();
            let (hits, _undecided) = crate::code_rule_dsl::text::vocabulary::evaluate(&plans, &terms);
            let mut bits: Vec<Box<[u64]>> = vec![vec![0u64; words].into_boxed_slice(); fresh.len()];
            for (p, matched) in hits.iter().enumerate() {
                let k = owner[p];
                for &t in matched {
                    bits[t as usize][k / 64] |= 1 << (k % 64);
                }
            }
            for ((sym, _), b) in fresh.iter().zip(bits) {
                seen.insert(*sym, b);
            }
        }
        for &sym in symbols {
            let bits = &seen[&sym];
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
///
/// And each `match_all("re", sp, m)` (core.rr's built-in: `m` is each match of `re` in span `sp`'s text, as Rust's
/// `find_iter` finds them) into an atom over the input relation `matchall_<k>(sp, m)`, and each
/// `match_group("re", sp, m, g, gsp)` (`gsp` the span of named group `g` in match `m`) into one over
/// `matchgroup_<k>(sp, m, g, gsp)`, `k` the regex's (one per distinct regex, so the two agree on `m`), which the
/// batches fill (`facts::vtab::syntax`). The regex is a constant, in Rust `regex` syntax (W's).
///
/// And collects the markers of `fstring_template(n, "marker", v)` (W's `interpolation`): a constant, since the
/// batches parse each host once per marker (`facts::vtab::syntax::fstring`).
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
                    let matcher = Matcher::new(&key.0, &key.1)?;
                    super::profile::decision("text tests (decision 0037: a relation per distinct test, each string tested once)", || {
                        let shown: Vec<String> = key.1.iter().take(6).map(|p| format!("{p:?}")).collect();
                        let more = if key.1.len() > 6 { format!(", … {} in all", key.1.len()) } else { String::new() };
                        format!("{rel} = @{}({{{}{more}}}): {}", key.0, shown.join(", "), matcher.describe())
                    });
                    tests.push((rel.clone(), Arc::new(matcher)));
                    ids.insert(key, rel.clone());
                    rel
                }
            };
            *literal = Literal::Atom(Atom { rel, args: vec![Term::Var(s.to_owned())], neg: !holds, gated: false });
        }
    }
    let mut markers: Vec<String> = Vec::new();
    let mut regexes: BTreeMap<String, usize> = BTreeMap::new();
    let mut matches = Vec::new();
    let mut made = BTreeMap::new();
    for rule in &mut prog.rules {
        for literal in &mut rule.body {
            let (Literal::Atom(a) | Literal::Agg { atom: a, .. }) = literal else { continue };
            if a.rel == "fstring_template" {
                let [_, Term::Str(marker), _] = a.args.as_slice() else {
                    return Err(format!("fstring_template takes a constant marker: fstring_template(n, \"{{...}}\", v), not {a}"));
                };
                if !markers.contains(marker) {
                    markers.push(marker.clone());
                }
                continue;
            }
            let (args, group) = match (a.rel.as_str(), a.args.as_slice()) {
                ("match_all", [Term::Str(_), sp, m]) => (vec![sp.clone(), m.clone()], false),
                ("match_group", [Term::Str(_), sp, m, g, gsp]) => (vec![sp.clone(), m.clone(), g.clone(), gsp.clone()], true),
                ("match_all", _) => return Err(format!("match_all takes a constant regex, a span and its match: match_all(\"re\", sp, m), not {a}")),
                ("match_group", _) => {
                    return Err(format!("match_group takes a constant regex, a span, its match, a group name and its span: match_group(\"re\", sp, m, g, gsp), not {a}"));
                }
                _ => continue,
            };
            let Term::Str(re) = &a.args[0] else { unreachable!("matched above") };
            let k = match regexes.get(re) {
                Some(&k) => k,
                None => {
                    let regex = regex::Regex::new(re).map_err(|e| format!("{}({re:?}, …): {e}", a.rel))?;
                    matches.push(Arc::new(regex));
                    regexes.insert(re.clone(), matches.len() - 1);
                    matches.len() - 1
                }
            };
            let rel = if group { format!("{GROUPS}{k}") } else { format!("{MATCHES}{k}") };
            made.insert(rel.clone(), group);
            a.args = args;
            a.rel = rel;
        }
    }
    for (rel, _) in &tests {
        prog.decls.insert(rel.clone(), vec![("s".to_owned(), "sym".to_owned())]);
        if !prog.inputs.contains(rel) {
            prog.inputs.push(rel.clone());
        }
    }
    for (rel, group) in made {
        let node = |c: &str| (c.to_owned(), "node".to_owned());
        let columns = if group { vec![node("sp"), node("m"), ("g".to_owned(), "sym".to_owned()), node("gsp")] } else { vec![node("sp"), node("m")] };
        prog.decls.insert(rel.clone(), columns);
        if !prog.inputs.contains(&rel) {
            prog.inputs.push(rel);
        }
    }
    Ok(Vocabulary { tests, matches, markers, seen: Mutex::default(), tested: Mutex::default() })
}

pub(in crate::code_rule_dsl) mod functions;

#[cfg(test)]
mod tests;
