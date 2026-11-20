//! The prefilter's vocabulary (`code_rule_dsl/prefilter`, Phase 3 row 3): SQL functions that
//! match regex predicates against distinct terms, installed on every store connection so the
//! vocabulary tables (schema: `vocabulary_*`) fill in SQL. Adapted from Pulling-it-all-together's
//! `matcher/anchors/sql/functions.rs`; its node decoders (`nera_decode`, `nera_span`, `nera_slice`,
//! `nera_parent`, ...) read P's private tape copies and went with them (03 §7).
use crate::probes::TextProbe;
use rusqlite::{
    Connection, Result,
    functions::{Context, FunctionFlags},
};

fn invalid(message: impl Into<String>) -> rusqlite::Error {
    rusqlite::Error::UserFunctionError(message.into().into())
}

/// Registers the term matchers and `generate_series` on `db`.
pub fn install(db: &Connection) -> Result<()> {
    rusqlite::vtab::series::load_module(db)?;
    let flags = FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_INNOCUOUS;
    install_term_matches(db, flags)?;
    install_terms_matching(db, flags)?;
    install_pattern_analyses(db, flags)
}

fn install_term_matches(db: &Connection, flags: FunctionFlags) -> Result<()> {
    db.create_scalar_function(
        c"nera_term_matches",
        2,
        flags | FunctionFlags::SQLITE_DETERMINISTIC,
        move |ctx| {
            let regex = ctx.get_or_create_aux(0, |value| {
                regex::Regex::new(value.as_str()?).map_err(
                    |error| -> Box<dyn std::error::Error + Send + Sync> { Box::new(error) },
                )
            })?;
            Ok(regex.is_match(ctx.get_raw(1).as_str()?))
        },
    )?;
    // Vocabulary maintenance supplies the pattern from a column. SQLite discards auxdata
    // for that varying argument on every row. A constant third argument owns one bounded
    // pattern cache instead; the SQL visits each predicate's complete term batch together.
    db.create_scalar_function(
        c"nera_term_matches",
        3,
        flags | FunctionFlags::SQLITE_DETERMINISTIC,
        term_matches_cached,
    )?;
    Ok(())
}

fn term_matches_cached(ctx: &Context<'_>) -> Result<bool> {
    type Cache = parking_lot::Mutex<Option<(String, regex::Regex)>>;
    let cache = ctx.get_or_create_aux(2, |_| {
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(Cache::new(None))
    })?;
    let mut cache = cache.lock();
    let expression = ctx.get_raw(0).as_str()?;
    if cache
        .as_ref()
        .is_none_or(|(pattern, _)| pattern != expression)
    {
        let regex = regex::Regex::new(expression).map_err(|e| invalid(e.to_string()))?;
        *cache = Some((expression.to_owned(), regex));
    }
    Ok(cache
        .as_ref()
        .expect("compiled pattern")
        .1
        .is_match(ctx.get_raw(1).as_str()?))
}

type Compiled = Vec<(String, TextProbe, regex::Regex)>;

fn install_terms_matching(db: &Connection, flags: FunctionFlags) -> Result<()> {
    // One row per term instead of one per pattern and term: the patterns of a set are compiled
    // once and tried in turn (a combined RegexSet was slower, regex.md §2.2).
    let sets = parking_lot::Mutex::new(Vec::<(String, Compiled)>::new());
    db.create_scalar_function(
        c"nera_terms_matching",
        2,
        flags | FunctionFlags::SQLITE_DETERMINISTIC,
        move |ctx| {
            let set = ctx.get_raw(0).as_str()?;
            let term = ctx.get_raw(1).as_str()?;
            let mut sets = sets.lock();
            let at = compiled_set(&mut sets, set)?;
            let matched: Vec<&str> = sets[at]
                .1
                .iter()
                // The probe's byte tests reject most terms before the regex runs (SQL index prototype
                // S163/S169: derived probes as top-level conjuncts).
                .filter(|(_, probe, regex)| probe.allows(term) && regex.is_match(term))
                .map(|(expression, _, _)| expression.as_str())
                .collect();
            Ok((!matched.is_empty())
                .then(|| serde_json::to_string(&matched).expect("expression JSON")))
        },
    )?;
    Ok(())
}

/// The index of `set`'s compiled patterns in `sets`; a miss compiles them in, dropping the
/// oldest of four.
#[inline]
fn compiled_set(sets: &mut Vec<(String, Compiled)>, set: &str) -> Result<usize> {
    match sets.iter().position(|(key, _)| key == set) {
        Some(at) => Ok(at),
        None => {
            let expressions: Vec<String> =
                serde_json::from_str(set).map_err(|e| invalid(e.to_string()))?;
            let compiled = expressions
                .into_iter()
                .map(|expression| {
                    let regex =
                        regex::Regex::new(&expression).map_err(|e| invalid(e.to_string()))?;
                    let probe = TextProbe::compile(&expression);
                    Ok((expression, probe, regex))
                })
                .collect::<Result<Vec<_>>>()?;
            if sets.len() == 4 {
                sets.remove(0);
            }
            sets.push((set.to_owned(), compiled));
            Ok(sets.len() - 1)
        }
    }
}

fn install_pattern_analyses(db: &Connection, flags: FunctionFlags) -> Result<()> {
    // An anchored predicate's terms lie in its prefix ranges: the fill seeks them in `term_freq`
    // instead of scanning every term (`vocabulary-fill.sql`).
    db.create_scalar_function(
        c"nera_prefix_ranges",
        1,
        flags | FunctionFlags::SQLITE_DETERMINISTIC,
        |ctx| {
            Ok(TextProbe::compile(ctx.get_raw(0).as_str()?)
                .prefix_ranges()
                .map(|ranges| serde_json::to_string(&ranges).expect("range JSON")))
        },
    )?;
    db.create_scalar_function(
        c"nera_literal_set",
        1,
        flags | FunctionFlags::SQLITE_DETERMINISTIC,
        |ctx| {
            Ok(literal_set(ctx.get_raw(0).as_str()?)
                .map(|literals| serde_json::to_string(&literals).expect("literal JSON")))
        },
    )?;
    Ok(())
}

/// The terms a pattern matches when it is `\A` and `\z` around a finite set of exact literals.
fn literal_set(pattern: &str) -> Option<Vec<String>> {
    use regex_syntax::hir::{Hir, HirKind, Look, literal::Extractor};
    let hir = regex_syntax::parse(pattern).ok()?;
    let HirKind::Concat(parts) = hir.kind() else {
        return None;
    };
    let (first, rest) = parts.split_first()?;
    let (last, middle) = rest.split_last()?;
    if !matches!(first.kind(), HirKind::Look(Look::Start))
        || !matches!(last.kind(), HirKind::Look(Look::End))
    {
        return None;
    }
    let core = Hir::concat(middle.to_vec());
    // The extractor reads an assertion as the empty string.
    if !core.properties().look_set().is_empty() {
        return None;
    }
    let sequence = Extractor::new().extract(&core);
    let literals = sequence.literals()?;
    if literals.is_empty() || !literals.iter().all(|literal| literal.is_exact()) {
        return None;
    }
    literals
        .iter()
        .map(|literal| String::from_utf8(literal.as_bytes().to_vec()).ok())
        .collect()
}
