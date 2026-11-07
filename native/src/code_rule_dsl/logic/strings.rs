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

/// The relations a text test's string can come from, and their text columns: the batch interns these strings, so a
/// test over them is answered from the batch's vocabulary (`facts::vtab::syntax`).
pub(in crate::code_rule_dsl) const TEXT_COLUMNS: [(&str, &[usize]); 17] = [
    ("value", &[1]),
    ("text", &[1]),
    ("name", &[1]),
    ("str", &[1]),
    ("strprefix", &[1]),
    ("op", &[1]),
    ("op_at", &[2]),
    ("doc", &[1, 2]),
    ("pkg", &[1]),
    ("path_part", &[1]),
    ("file_name", &[1]),
    ("comment", &[1]),
    ("static_value", &[1]),
    ("str_part", &[2]),
    ("value_type", &[0, 1]),
    ("split_line", &[2]),
    ("lf_line", &[2]),
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

static GROUPS: LazyLock<Mutex<HashMap<String, Option<Arc<regex::Regex>>>>> = LazyLock::new(Default::default);

/// The text of named group `name` of regex `re`'s match of `s` (`.rr`'s `match("(?<x>…)", s)`); none when `re`
/// does not match, the group took no part, or `re` is invalid.
pub(in crate::code_rule_dsl) fn group(s: &str, re: &str, name: &str) -> Option<String> {
    let rx = GROUPS.lock().unwrap().entry(re.to_owned()).or_insert_with(|| regex::Regex::new(re).ok().map(Arc::new)).clone()?;
    rx.captures(s)?.name(name).map(|m| m.as_str().to_owned())
}

/// `str_test(op, s, patterns)`: 1 or 0; and `core.rr`'s text functors (`functions`, `rx_group` among them). NULL
/// gives NULL.
pub(in crate::code_rule_dsl) fn register(db: &Connection) -> rusqlite::Result<()> {
    use rusqlite::functions::FunctionFlags;
    db.create_scalar_function(c"str_test", 3, FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC, |ctx| {
        let (Some(op), Some(s), Some(p)) = (ctx.get::<Option<String>>(0)?, ctx.get::<Option<String>>(1)?, ctx.get::<Option<String>>(2)?) else {
            return Ok(None);
        };
        Ok(Some(i64::from(test(&op, &s, &p))))
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
fn vocabulary_columns(prog: &Program) -> HashMap<String, HashSet<usize>> {
    let mut columns: HashMap<String, HashSet<usize>> = TEXT_COLUMNS.iter()
        .map(|(name, columns)| ((*name).to_owned(), columns.iter().copied().collect()))
        .collect();
    loop {
        let mut found = Vec::new();
        for (relation, declaration) in &prog.decls {
            // Facts and unknown external inputs can introduce symbols not in this batch's vocabulary.
            if prog.facts.get(relation).is_some_and(|rows| !rows.is_empty())
                || (prog.inputs.contains(relation) && !columns.contains_key(relation)) {
                continue;
            }
            let writers: Vec<_> = prog.rules.iter().filter(|r| &r.head.rel == relation).collect();
            if writers.is_empty() {
                continue;
            }
            for (column, (_, ty)) in declaration.iter().enumerate() {
                if ty != "sym" || columns.get(relation).is_some_and(|cols| cols.contains(&column)) {
                    continue;
                }
                // Every producer must copy a prepared symbol. One computed or constant arm is enough
                // to make the rewrite unsafe: that arm's new values do not have strtest rows yet.
                let copied = writers.iter().all(|rule| {
                    let Some(Term::Var(value)) = rule.head.args.get(column) else { return false };
                    rule.body.iter().filter_map(Literal::atom).filter(|a| !a.neg).any(|atom| {
                        columns.get(&atom.rel).is_some_and(|cols| cols.iter().any(|&col| {
                            matches!(atom.args.get(col), Some(Term::Var(other)) if other == value)
                        }))
                    })
                });
                if copied {
                    found.push((relation.clone(), column));
                }
            }
        }
        if found.is_empty() {
            return columns;
        }
        for (relation, column) in found {
            columns.entry(relation).or_default().insert(column);
        }
    }
}

pub(in crate::code_rule_dsl) fn as_relations(prog: &mut Program) -> Result<Vocabulary, String> {
    let vocabulary_columns = vocabulary_columns(prog);
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
                vocabulary_columns.get(&a.rel).into_iter().flatten()
                    .filter_map(|&c| if let Some(Term::Var(v)) = a.args.get(c) { Some(v.clone()) } else { None })
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
    for (rel, _) in &tests {
        prog.decls.insert(rel.clone(), vec![("s".to_owned(), "sym".to_owned())]);
        if !prog.inputs.contains(rel) {
            prog.inputs.push(rel.clone());
        }
    }
    Ok(Vocabulary { tests, seen: Mutex::default(), tested: Mutex::default() })
}

pub(in crate::code_rule_dsl) mod functions;
mod signatures;

#[cfg(test)]
mod tests;
