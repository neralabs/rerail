//! Literal tests on names and paths (step 30's primitives): a rule tests a name atom's text against a literal,
//! never a regex.
//!
//! ```text
//! private($n)  :- text($n, $s), @prefix($s, "_").
//! is_async($f) :- defines_fn(_, $x, $f, _), @suffix($x, "_async").
//! sqlish($n)   :- text($n, $s), @contains($s, "Sql").
//! in_impl($d)  :- doc($d, $p, _), @glob($p, "impl/**/*.py").
//! ```
//!
//! A test is a body literal on its own. It stands for `@f(a, b) = 1`, and every backend runs it in SQL as
//! `str_test(f, a, b)`. In atoms mode the arguments are the atoms' text (`sym_text`). Globs are path globs: `*`,
//! `?` and `[…]` stay within one path segment, and `**` spans segments. Compiled globs are kept per pattern.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use rusqlite::Connection;

/// The tests, by name.
/// `i…`: the same, ignoring case (ASCII and Unicode simple case folding by lowercasing both sides).
pub(in crate::code_rule_dsl) const TESTS: [&str; 8] = ["prefix", "suffix", "contains", "glob", "iprefix", "isuffix", "icontains", "iequals"];

pub(in crate::code_rule_dsl) fn is_test(f: &str) -> bool {
    TESTS.contains(&f)
}

static GLOBS: LazyLock<Mutex<HashMap<String, Option<globset::GlobMatcher>>>> = LazyLock::new(Default::default);

/// Test `op` of `s` against the literal `pattern`. An invalid glob matches nothing.
pub(in crate::code_rule_dsl) fn test(op: &str, s: &str, pattern: &str) -> bool {
    match op {
        "prefix" => s.starts_with(pattern),
        "suffix" => s.ends_with(pattern),
        "contains" => s.contains(pattern),
        "glob" => {
            let mut globs = GLOBS.lock().unwrap();
            let m = globs.entry(pattern.to_owned()).or_insert_with(|| {
                globset::GlobBuilder::new(pattern).literal_separator(true).build().ok().map(|g| g.compile_matcher())
            });
            m.as_ref().is_some_and(|m| m.is_match(s))
        }
        "iprefix" | "isuffix" | "icontains" | "iequals" => {
            let (s, p) = (s.to_lowercase(), pattern.to_lowercase());
            match op {
                "iprefix" => s.starts_with(&p),
                "isuffix" => s.ends_with(&p),
                "icontains" => s.contains(&p),
                _ => s == p,
            }
        }
        _ => false,
    }
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
