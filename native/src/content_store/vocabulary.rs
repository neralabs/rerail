//! The prefilter's vocabulary (`code_rule_dsl/prefilter`, Phase 3 row 3): a regex's terms
//! (`ContentStore::regex_terms`, kept facts: `prefilter::vocabulary`), and `nera_term_matches`, a regex
//! tested on a term in SQL, installed on every store connection. Adapted from PIAT's
//! `matcher/anchors/sql/functions.rs`; its node decoders (`nera_decode`, `nera_span`, `nera_slice`,
//! `nera_parent`, ...) read P's private tape copies and went with them (03 §7).
use crate::probes::TextProbe;
use rusqlite::{
    Connection, Result,
    functions::FunctionFlags,
};

impl super::ContentStore {
    /// Each of `expressions`' terms of language `lang` (the store's atoms, ascending): the terms they match, kept
    /// (`prefilter::vocabulary`, kind `REGEX`) and brought up to date. A finite set of exact literals is its literals
    /// (`literal_set`); any other is tried, its probe's byte tests first, on each term.
    pub fn regex_terms(&self, lang: i64, expressions: &[String]) -> super::StoreResult<Vec<Vec<i64>>> {
        let compiled = expressions
            .iter()
            .map(|expression| {
                let regex = regex::Regex::new(expression).map_err(|e| super::StoreError::Invalid(format!("regex {expression:?}: {e}")))?;
                Ok((TextProbe::compile(expression), regex))
            })
            .collect::<super::StoreResult<Vec<_>>>()?;
        let test = |keys: &[usize], terms: &[&str]| -> Vec<Vec<u32>> {
            keys.iter()
                .map(|&key| {
                    let (probe, regex) = &compiled[key];
                    (0..terms.len() as u32).filter(|&i| probe.allows(terms[i as usize]) && regex.is_match(terms[i as usize])).collect()
                })
                .collect()
        };
        self.vocabulary_terms(super::prefilter::vocabulary::REGEX, lang, expressions, &test, &literal_set)
    }
}

/// Registers the term matcher and `generate_series` on `db`.
pub fn install(db: &Connection) -> Result<()> {
    rusqlite::vtab::series::load_module(db)?;
    install_term_matches(db, FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_INNOCUOUS)
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
