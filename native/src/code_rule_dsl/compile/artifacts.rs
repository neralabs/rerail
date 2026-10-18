use crate::code_rule_dsl::language::{GrammarDialect, PatternSource, rule_pattern};
use ast_grep_core::matcher::Pattern;
use parking_lot::Mutex;
use regex::Regex;
use rustc_hash::FxHashMap;
use std::sync::{Arc, OnceLock};

#[derive(Clone, Hash, PartialEq, Eq)]
struct PatternKey {
    language: GrammarDialect,
    context: String,
    selector: Option<String>,
}

type RegexCell = Arc<OnceLock<Result<Regex, String>>>;
type PatternValue = (Arc<Pattern>, GrammarDialect);
type PatternCell = Arc<OnceLock<Result<PatternValue, String>>>;

/// Expensive immutable leaves interned for one policy compilation. Final matchers retain their own
/// cheap clones/Arcs; dropping the compiler drops entries no compiled matcher uses.
#[derive(Default)]
pub(crate) struct CompilerArtifacts {
    regexes: Mutex<FxHashMap<String, RegexCell>>,
    patterns: Mutex<FxHashMap<PatternKey, PatternCell>>,
}

impl CompilerArtifacts {
    pub(super) fn regex(&self, source: &str) -> Result<Regex, String> {
        let cell = {
            let mut regexes = self.regexes.lock();
            Arc::clone(
                regexes
                    .entry(source.to_owned())
                    .or_insert_with(|| Arc::new(OnceLock::new())),
            )
        };
        cell.get_or_init(|| Regex::new(source).map_err(|error| error.to_string()))
            .clone()
    }

    pub(super) fn pattern(
        &self,
        source: PatternSource<'_>,
        language: GrammarDialect,
    ) -> Result<PatternValue, String> {
        let key = match source {
            PatternSource::Plain(context) => PatternKey {
                language,
                context: context.to_owned(),
                selector: None,
            },
            PatternSource::Contextual { context, selector } => PatternKey {
                language,
                context: context.to_owned(),
                selector: Some(selector.to_owned()),
            },
        };
        let cell = {
            let mut patterns = self.patterns.lock();
            Arc::clone(
                patterns
                    .entry(key.clone())
                    .or_insert_with(|| Arc::new(OnceLock::new())),
            )
        };
        cell.get_or_init(|| {
            let source = match &key.selector {
                Some(selector) => PatternSource::Contextual {
                    context: &key.context,
                    selector,
                },
                None => PatternSource::Plain(&key.context),
            };
            rule_pattern(source, key.language)
                .map(|(pattern, dialect)| (Arc::new(pattern), dialect))
                .map_err(|error| error.to_string())
        })
        .clone()
    }
}
