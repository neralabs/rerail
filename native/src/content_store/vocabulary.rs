//! The prefilter's vocabulary (`code_rule_dsl/prefilter`, Phase 3 row 3): SQL functions that
//! match regex predicates against distinct terms, installed on every store connection so the
//! vocabulary tables (schema: `vocabulary_*`) fill in SQL. Adapted from Pulling-it-all-together's
//! `matcher/anchors/sql/functions.rs`; its node decoders (`nera_decode`, `nera_span`, `nera_slice`,
//! `nera_parent`, ...) read P's private tape copies and went with them (03 §7).
use rusqlite::{
    Connection, Result,
    functions::FunctionFlags,
};

fn invalid(message: impl Into<String>) -> rusqlite::Error {
    rusqlite::Error::UserFunctionError(message.into().into())
}

/// Registers the term matchers and `generate_series` on `db`.
pub(super) fn install(db: &Connection) -> Result<()> {
    rusqlite::vtab::series::load_module(db)?;
    let flags = FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_INNOCUOUS;
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
        move |ctx| {
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
        },
    )?;
    // One row per term instead of one per pattern and term: the patterns of a set are compiled
    // once and tried in turn (a combined RegexSet was slower, regex.md §2.2).
    let sets = parking_lot::Mutex::new(Vec::<(String, Vec<(String, regex::Regex)>)>::new());
    db.create_scalar_function(
        c"nera_terms_matching",
        2,
        flags | FunctionFlags::SQLITE_DETERMINISTIC,
        move |ctx| {
            let set = ctx.get_raw(0).as_str()?;
            let term = ctx.get_raw(1).as_str()?;
            let mut sets = sets.lock();
            let at = match sets.iter().position(|(key, _)| key == set) {
                Some(at) => at,
                None => {
                    let expressions: Vec<String> =
                        serde_json::from_str(set).map_err(|e| invalid(e.to_string()))?;
                    let compiled = expressions
                        .into_iter()
                        .map(|expression| {
                            let regex = regex::Regex::new(&expression)
                                .map_err(|e| invalid(e.to_string()))?;
                            Ok((expression, regex))
                        })
                        .collect::<Result<Vec<_>>>()?;
                    if sets.len() == 4 {
                        sets.remove(0);
                    }
                    sets.push((set.to_owned(), compiled));
                    sets.len() - 1
                }
            };
            let matched: Vec<&str> = sets[at]
                .1
                .iter()
                .filter(|(_, regex)| regex.is_match(term))
                .map(|(expression, _)| expression.as_str())
                .collect();
            Ok((!matched.is_empty())
                .then(|| serde_json::to_string(&matched).expect("expression JSON")))
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
