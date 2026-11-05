//! Text tests on names, paths and strings (step 30's primitives): a rule tests a value's text against a
//! literal pattern.
//!
//! ```text
//! private($n)  :- text($n, $s), @prefix($s, "_").
//! is_async($f) :- defines_fn(_, $x, $f, _), @suffix($x, "_async").
//! sqlish($n)   :- text($n, $s), @contains($s, "Sql").
//! in_impl($d)  :- doc($d, $p, _), @glob($p, "impl/**/*.py").
//! cache($n)    :- text($n, $s), @rx($s, "^_\\w*cache\\w*$").
//! ```
//!
//! A test is a body literal on its own. It stands for `@f(a, b) = 1`, and every backend runs it in SQL as
//! `str_test(f, a, b)`. In atoms mode the arguments are the atoms' text (`sym_text`). Globs are path globs: `*`,
//! `?` and `[…]` stay within one path segment, and `**` spans segments.
//!
//! Every test is lowered by the text layer (decision 0039): a regex and its glob or literal rewrite are one plan
//! with one key, compiled once per process.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

use crate::code_rule_dsl::text::{Compiled, Domain, Source, Spelling, lower};
use rusqlite::Connection;

/// The tests, by name. `i…`: the same, ignoring case (the regex engine's simple case folding). `rx`: a
/// regex in Rust `regex` syntax (with backreferences beyond it, as `value.regex`).
pub(in crate::code_rule_dsl) const TESTS: [&str; 10] = ["prefix", "suffix", "contains", "glob", "iprefix", "isuffix", "icontains", "iequals", "rx", "irx"];

pub(in crate::code_rule_dsl) fn is_test(f: &str) -> bool {
    TESTS.contains(&f)
}

/// Test `op` as the text layer's source.
fn source(op: &str, pattern: &str) -> Option<Source> {
    let p = || pattern.to_owned();
    let (spelling, folded) = match op {
        "prefix" => (Spelling::Prefix(p()), false),
        "suffix" => (Spelling::Suffix(p()), false),
        "contains" => (Spelling::Contains(p()), false),
        "glob" => (Spelling::PathGlob(p()), false),
        "iprefix" => (Spelling::Prefix(p()), true),
        "isuffix" => (Spelling::Suffix(p()), true),
        "icontains" => (Spelling::Contains(p()), true),
        "iequals" => (Spelling::Equal(p()), true),
        "rx" => (Spelling::Regex(p()), false),
        "irx" => (Spelling::Regex(p()), true),
        _ => return None,
    };
    let source = Source::of(spelling);
    Some(if folded { source.folded() } else { source })
}

/// Each (test, pattern)'s compiled plan: `None` where the pattern does not parse.
type Cache = HashMap<String, HashMap<String, Option<Arc<Compiled>>>>;
static COMPILED: LazyLock<Mutex<Cache>> = LazyLock::new(Default::default);

fn compiled(op: &str, pattern: &str) -> Option<Arc<Compiled>> {
    let mut cache = COMPILED.lock().expect("string tests");
    if let Some(found) = cache.get(op).and_then(|by| by.get(pattern)) {
        return found.clone();
    }
    let made = source(op, pattern).and_then(|source| lower(&source, Domain::ANY).ok()).map(|lowered| lowered.compiled());
    cache.entry(op.to_owned()).or_default().insert(pattern.to_owned(), made.clone());
    made
}

/// Test `op` of `s` against `pattern`. A pattern that does not parse, or a backtracking one past its limits,
/// matches nothing.
pub(in crate::code_rule_dsl) fn test(op: &str, s: &str, pattern: &str) -> bool {
    compiled(op, pattern).is_some_and(|compiled| compiled.is_match(s.as_bytes()) == Some(true))
}

/// `str_test(op, s, pattern)`: 1 or 0; NULL gives NULL.
pub(in crate::code_rule_dsl) fn register(db: &Connection) -> rusqlite::Result<()> {
    use rusqlite::functions::FunctionFlags;
    db.create_scalar_function(c"str_test", 3, FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC, |ctx| {
        let (Some(op), Some(s), Some(p)) = (ctx.get::<Option<String>>(0)?, ctx.get::<Option<String>>(1)?, ctx.get::<Option<String>>(2)?) else {
            return Ok(None);
        };
        Ok(Some(i64::from(test(&op, &s, &p))))
    })
}

#[cfg(test)]
mod tests;
