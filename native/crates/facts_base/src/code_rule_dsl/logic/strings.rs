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
//! relation into an input relation (`preparation::text::TextRelations`): the strings of the batch's vocabulary that pass it, each
//! tested once (`Vocabulary`), all tests' plans together by the text layer's vocabulary engine (one multi-literal
//! searcher per anchor, automata in small groups). Rules join on symbols. A test that stays in a rule (its string is
//! computed in the rule, as `@cat`'s) runs per row in SQL as `str_test(op, s, patterns)`, the patterns joined by
//! `SET_SEP`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex};

use crate::code_rule_dsl::text::{Domain, Lowered, Source, Spelling, lower};
use rusqlite::Connection;

/// The tests, by name. `i…`: the same, ignoring case (the text layer's case folding). `rx`: a regex in Rust `regex`
/// syntax (with backreferences beyond it, as `value.regex`).
pub const TESTS: [&str; 14] =
    ["equals", "prefix", "suffix", "contains", "glob", "word", "rx", "iequals", "iprefix", "isuffix", "icontains", "iglob", "iword", "irx"];

pub fn is_test(f: &str) -> bool {
    TESTS.contains(&f)
}

/// A pattern set's term: `{a, b}` parses as `@set(a, b)`.
pub const SET: &str = "set";

/// Between a set's patterns in SQL's `str_test`.
pub const SET_SEP: char = '\u{1f}';

/// The prefix of a test's input relation, `strtest_<k>(s)`.
pub const RELATION: &str = "strtest_";

/// The prefix of a test's relation over strings answered during the run, `rr_texttest_<k>_<n>`: the rows of a
/// relation whose string passes test `k` (as in `strtest_<k>`), made natively from the run's vocabulary
/// (`front::Native::TextTest`).
pub const DURING: &str = "rr_texttest_";

/// The prefix of a `match_all` built-in's input relation, `matchall_<k>(sp, m)` (`preparation::text::TextRelations`).
pub const MATCHES: &str = "matchall_";

/// The prefix of a `match_group` built-in's input relation, `matchgroup_<k>(sp, m, g, gsp)` (`preparation::text::TextRelations`).
pub const GROUPS: &str = "matchgroup_";

/// The relations a text test's string can come from, and their text columns: the batch interns these strings, so a
/// test over them is answered from the batch's vocabulary (`facts::vtab::syntax`).
pub const TEXT_COLUMNS: [(&str, &[usize]); 31] = [
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
    ("rr_token_anon", &[2]),
    ("path_part", &[1]),
    ("file_name", &[1]),
    ("value", &[1]),
    ("static_value", &[1]),
    ("rr_value_lit", &[1]),
    ("value_type_of", &[1]),
    ("value_type", &[0, 1]),
    ("comment", &[1]),
    ("str_part", &[2]),
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
pub struct Matcher {
    pub plans: Vec<Arc<Lowered>>,
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
    pub fn new(op: &str, patterns: &[String]) -> Result<Self, String> {
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
    pub fn describe(&self) -> String {
        let mut by: BTreeMap<String, usize> = BTreeMap::new();
        for p in &self.plans {
            let how = if p.rewrites.is_empty() { p.production.to_owned() } else { format!("{} after {}", p.production, p.rewrites.join(", ")) };
            *by.entry(how).or_default() += 1;
        }
        let parts: Vec<String> = by.into_iter().map(|(how, n)| if n == 1 { how } else { format!("{n} × {how}") }).collect();
        format!("text layer: {}", parts.join(" + "))
    }

    /// A pattern undecided (a backtracking one past its limits) does not match.
    pub fn is_match(&self, s: &str) -> bool {
        self.plans.iter().any(|p| p.compiled().is_match(s.as_bytes()) == Some(true))
    }
}

#[derive(Default)]
struct Matchers(Mutex<HashMap<(String, String), Option<Arc<Matcher>>>>);

/// Test `op` of `s` against `pattern`, a set's patterns joined by `SET_SEP`. An invalid glob matches nothing.
pub fn test(op: &str, s: &str, pattern: &str) -> bool {
    let m = crate::code_rule_dsl::logic::profile::shared::<Matchers>().0
        .lock()
        .unwrap()
        .entry((op.to_owned(), pattern.to_owned()))
        .or_insert_with(|| Matcher::new(op, &pattern.split(SET_SEP).map(str::to_owned).collect::<Vec<_>>()).ok().map(Arc::new))
        .clone();
    m.is_some_and(|m| m.is_match(s))
}

#[derive(Default)]
struct GroupRegexes(Mutex<HashMap<String, Option<Arc<regex::Regex>>>>);

/// The text of named group `name` of regex `re`'s match of `s` (`.rerail`'s `match("(?<x>…)", s)`); none when `re`
/// does not match, the group took no part, or `re` is invalid.
pub(in crate::code_rule_dsl) fn group(s: &str, re: &str, name: &str) -> Option<String> {
    let held_ = crate::code_rule_dsl::logic::profile::shared::<GroupRegexes>();
    let rx = held_.0.lock().unwrap().entry(re.to_owned()).or_insert_with(|| regex::Regex::new(re).ok().map(Arc::new)).clone()?;
    rx.captures(s)?.name(name).map(|m| m.as_str().to_owned())
}

/// `str_test(op, s, patterns)`: 1 or 0; and `core.rerail`'s text functors (`functions`, `rx_group` among them, and
/// `functors::register_functors`'s: `rr_join`, `@rx_replace`, `@nfkc`, …). NULL gives NULL.
pub fn register(db: &Connection) -> rusqlite::Result<()> {
    use rusqlite::functions::FunctionFlags;
    db.create_scalar_function(c"str_test", 3, FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC, |ctx| {
        let (Some(op), Some(s), Some(p)) = (ctx.get::<Option<String>>(0)?, ctx.get::<Option<String>>(1)?, ctx.get::<Option<String>>(2)?) else {
            return Ok(None);
        };
        Ok(Some(i64::from(test(&op, &s, &p))))
    })?;
    functions::register(db)?;
    match FUNCTORS.get() {
        Some(register) => register(db),
        None => Ok(()),
    }
}

/// What registers core.rerail's functors whose meaning is W's or a language's (`functors::register_functors`).
static FUNCTORS: std::sync::OnceLock<fn(&Connection) -> rusqlite::Result<()>> = std::sync::OnceLock::new();

/// Registers what adds those functors to a connection (once, at load).
pub fn install_functors(register: fn(&Connection) -> rusqlite::Result<()>) {
    let _ = FUNCTORS.set(register);
}

/// Every test's plans as the vocabulary engine runs them, and each plan's test.
struct Compiled {
    searchers: crate::code_rule_dsl::text::vocabulary::Searchers,
    owner: Vec<usize>,
}

/// The text tests a run answers from the vocabulary: each input relation `strtest_<k>(s)` and its matcher; the
/// regexes of `match_all` and `match_group` the batches find in their spans, regex `k`'s matches in
/// `matchall_<k>(sp, m)` and their groups in `matchgroup_<k>(sp, m, g, gsp)`; and the interpolation markers the
/// batches make `fstring_template(n, marker, v)` rows for.
#[derive(Default)]
pub struct Vocabulary {
    /// Each test's input relation and matcher (`again` keeps them for another run).
    pub tests: Vec<(String, Arc<Matcher>)>,
    pub matches: Vec<Arc<regex::Regex>>,
    /// The constant markers of the program's `fstring_template` atoms, distinct, in order.
    pub markers: Vec<String>,
    /// Each test's operator and patterns, in `tests`' order: what its matcher is made from (`from_specs`).
    pub specs: Vec<(String, Vec<String>)>,
    /// Every test's plans compiled together, once (decision 0075): shared by `again`, so a program kept for the
    /// process compiles them once, and every batch runs the same searchers.
    compiled: Arc<std::sync::OnceLock<Compiled>>,
    /// Per symbol, the tests it passes (bit k is test k): each string is tested once per run.
    seen: Mutex<HashMap<u32, Box<[u64]>>>,
    /// The strings tested, and the CPU (user + system seconds) testing them took.
    pub tested: Mutex<(usize, f64)>,
}

impl Vocabulary {
    /// For each test, the symbols among `strings` (a batch's distinct strings and their symbols) that pass it.
    pub fn rows(&self, strings: &[String], symbols: &[u32]) -> Vec<Vec<i64>> {
        let mut out = vec![Vec::new(); self.tests.len()];
        if self.tests.is_empty() {
            return out;
        }
        let seen = self.learn(strings, symbols);
        for &sym in symbols {
            let bits = &seen[&sym];
            for (k, rows) in out.iter_mut().enumerate() {
                if bits[k / 64] >> (k % 64) & 1 == 1 {
                    rows.push(i64::from(sym));
                }
            }
        }
        out
    }

    /// The symbols among `strings` that pass test `k`: `rows`' `k`th, the others not read. A batch's text test sized
    /// or scanned one at a time (`TextTest::whole`) read every test's bits of every string and kept one test's: of
    /// N tests, N² bits a string (1.3 s of ep2's cold run, in `sampled_statistics_unanswered`).
    pub fn rows_of(&self, k: usize, strings: &[String], symbols: &[u32]) -> Vec<i64> {
        if k >= self.tests.len() {
            return Vec::new();
        }
        let seen = self.learn(strings, symbols);
        symbols.iter().filter(|sym| seen[*sym][k / 64] >> (k % 64) & 1 == 1).map(|&sym| i64::from(sym)).collect()
    }

    /// `strings` not tested before, tested by every test at once: each string's bits, kept (`seen`), held.
    fn learn(&self, strings: &[String], symbols: &[u32]) -> std::sync::MutexGuard<'_, HashMap<u32, Box<[u64]>>> {
        let words = self.tests.len().div_ceil(64);
        let (mut tested, t0) = (0, crate::code_rule_dsl::prof::thread_usage());
        // The strings not seen before, tested by every test's plans at once (the text layer's vocabulary engine).
        // `seen` is not held while they are tested: the searchers run on the thread pool, and a pool thread waiting
        // there runs other queued jobs, among them another batch's call here. Held, that call waited on the lock
        // its own thread's earlier frame held, and the run hung (every pool thread asleep in a join). Two batches
        // may then test one new string each; the answer is the same.
        let mut fresh: Vec<(u32, &str)> = Vec::new();
        {
            let seen = self.seen.lock().unwrap();
            let mut queued: HashSet<u32> = HashSet::new();
            for (s, &sym) in strings.iter().zip(symbols) {
                if !seen.contains_key(&sym) && queued.insert(sym) {
                    fresh.push((sym, s.as_str()));
                }
            }
        }
        let mut learned: Vec<(u32, Box<[u64]>)> = Vec::new();
        if !fresh.is_empty() {
            tested += fresh.len();
            let compiled = self.compiled.get_or_init(|| self.compile_with(None, true));
            let terms: Vec<&[u8]> = fresh.iter().map(|(_, s)| s.as_bytes()).collect();
            let (hits, _undecided) = compiled.searchers.evaluate(&terms);
            let mut bits: Vec<Box<[u64]>> = vec![vec![0u64; words].into_boxed_slice(); fresh.len()];
            for (p, matched) in hits.iter().enumerate() {
                let k = compiled.owner[p];
                for &t in matched {
                    bits[t as usize][k / 64] |= 1 << (k % 64);
                }
            }
            learned.extend(fresh.iter().map(|(sym, _)| *sym).zip(bits));
        }
        let t1 = crate::code_rule_dsl::prof::thread_usage();
        {
            let mut total = self.tested.lock().unwrap();
            total.0 += tested;
            total.1 += (t1.0 - t0.0) + (t1.1 - t0.1);
        }
        let mut seen = self.seen.lock().unwrap();
        seen.extend(learned);
        seen
    }

    /// Of `strings` (distinct), the positions of those that pass test `k`: a test over strings the run answered
    /// (`front::Native::TextTest`). With their `symbols` they are tested as a batch's are (`rows`): every test at
    /// once, each string once a run. Without (a run whose symbols are text) they are tested, every test at once.
    pub fn passing(&self, k: usize, strings: &[String], symbols: Option<&[u32]>) -> Vec<usize> {
        if let Some(symbols) = symbols.filter(|s| s.len() == strings.len()) {
            let passed: HashSet<i64> = self.rows_of(k, strings, symbols).into_iter().collect();
            return symbols.iter().enumerate().filter(|(_, s)| passed.contains(&i64::from(**s))).map(|(i, _)| i).collect();
        }
        let (tested, t0) = (strings.len(), crate::code_rule_dsl::prof::thread_usage());
        let compiled = self.compiled.get_or_init(|| self.compile_with(None, true));
        let terms: Vec<&[u8]> = strings.iter().map(|s| s.as_bytes()).collect();
        let (hits, _undecided) = compiled.searchers.evaluate(&terms);
        let mut out: Vec<usize> = hits.iter().enumerate().filter(|(p, _)| compiled.owner[*p] == k).flat_map(|(_, m)| m.iter().map(|&t| t as usize)).collect();
        out.sort_unstable();
        out.dedup();
        let t1 = crate::code_rule_dsl::prof::thread_usage();
        let mut total = self.tested.lock().unwrap();
        total.0 += tested;
        total.1 += (t1.0 - t0.0) + (t1.1 - t0.1);
        out
    }
}

impl Vocabulary {
    /// The same tests for another run, with nothing tested yet.
    pub fn again(&self) -> Vocabulary {
        Vocabulary {
            tests: self.tests.clone(),
            specs: self.specs.clone(),
            matches: self.matches.clone(),
            markers: self.markers.clone(),
            compiled: Arc::clone(&self.compiled),
            ..Vocabulary::default()
        }
    }

    fn compile_with(&self, cache: Option<&dyn crate::code_rule_dsl::text::vocabulary::DfaCache>, dense: bool) -> Compiled {
        let (plans, owner): (Vec<&crate::code_rule_dsl::text::Plan>, Vec<usize>) =
            self.tests.iter().enumerate().flat_map(|(k, (_, m))| m.plans.iter().map(move |p| (&p.plan, k))).unzip();
        Compiled { searchers: crate::code_rule_dsl::text::vocabulary::Searchers::new(&plans, cache, dense), owner }
    }

    /// Compiles every test's plans now, its groups of automata read from `cache` or kept in it (decision 0075), and
    /// says what it did; `None` when they were compiled before (in this process). `dense`: a group neither kept nor
    /// cached is determinized; otherwise it runs on the lazy DFA (`Searchers::new`).
    pub fn compile(&self, cache: Option<&dyn crate::code_rule_dsl::text::vocabulary::DfaCache>, dense: bool) -> Option<crate::code_rule_dsl::text::vocabulary::Compiling> {
        let mut made = None;
        let compiled = self.compiled.get_or_init(|| {
            let compiled = self.compile_with(cache, dense);
            made = Some(compiled.searchers.compiling);
            compiled
        });
        // Compiled before (another run of this process): its groups named in this run's cache, as compiling did.
        if made.is_none()
            && let Some(c) = cache
        {
            compiled.searchers.name_in(c);
        }
        made
    }

    /// A vocabulary of these tests (each relation and matcher), their specs, the regexes and the markers, nothing
    /// compiled or tested yet: what `preparation::text::TextRelations` makes.
    pub fn from_parts(tests: Vec<(String, Arc<Matcher>)>, specs: Vec<(String, Vec<String>)>, matches: Vec<Arc<regex::Regex>>, markers: Vec<String>) -> Vocabulary {
        Vocabulary { tests, specs, matches, markers, ..Vocabulary::default() }
    }

    /// The tests `relations` and `specs` name, their matchers made again (a prepared program read from the store,
    /// decision 0073).
    pub fn from_specs(relations: &[String], specs: Vec<(String, Vec<String>)>) -> Result<Vocabulary, String> {
        let tests = relations.iter().zip(&specs).map(|(rel, (op, pats))| Ok((rel.clone(), Arc::new(Matcher::new(op, pats)?)))).collect::<Result<_, String>>()?;
        Ok(Vocabulary { tests, specs, ..Vocabulary::default() })
    }
}

pub mod functions;
pub mod git;

// cratify: `tests` is rerail-logic's (it mounts it)
