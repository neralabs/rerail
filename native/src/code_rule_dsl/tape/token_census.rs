//! A dialect's token automata as the token tests' census asks them: the tokens a grammar may leave out under each
//! kind, and each kind's parents (`compile::token_tests`, `logic::rr::token_reads`).

use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::tape::tokens::TokenGrammar;
use std::collections::{BTreeMap, BTreeSet};

/// One dialect's automata, as the census asks them.
pub struct Lang {
    pub(crate) dialect: GrammarDialect,
    pub(crate) implied: BTreeMap<String, BTreeSet<(String, String)>>,
    pub(crate) parents: BTreeMap<String, BTreeSet<String>>,
    pub(crate) tokens: BTreeSet<String>,
}

impl Lang {
    /// Built once per dialect, like its `TokenGrammar` (every rule compiled reads it).
    pub(crate) fn of(lang: GrammarDialect) -> Option<&'static Self> {
        static BY_DIALECT: [std::sync::OnceLock<Option<Lang>>; 256] = [const { std::sync::OnceLock::new() }; 256];
        BY_DIALECT
            .get(lang as usize)?
            .get_or_init(|| {
                let grammar = TokenGrammar::of_dialect(lang)?;
                Some(Self { dialect: lang, implied: grammar.implied_tokens(), parents: grammar.parents(), tokens: grammar.token_texts() })
            })
            .as_ref()
    }

    /// The parents among `parents` (all kinds where `None`) where a token `regex` matches is
    /// implied, under `field` where one is given: (parent, token) pairs.
    pub(crate) fn implied_in(&self, parents: Option<&BTreeSet<String>>, regex: &regex::Regex, field: Option<&str>) -> BTreeSet<(String, String)> {
        let mut out = BTreeSet::new();
        let kinds: Box<dyn Iterator<Item = &String>> = match parents {
            Some(parents) => Box::new(parents.iter()),
            None => Box::new(self.implied.keys()),
        };
        for kind in kinds {
            for (token, token_field) in self.implied.get(kind).into_iter().flatten() {
                if regex.is_match(token) && field.is_none_or(|field| field == token_field) {
                    out.insert((kind.clone(), token.clone()));
                }
            }
        }
        out
    }
}

/// The (parent kind, token) pairs where `dialect`'s grammar may leave out a token `regex` matches: under `kinds`, or
/// under any kind where `None`. A `.rr` program's token reads are refused by this (`logic::rr::token_reads`).
pub fn implied(dialect: GrammarDialect, kinds: Option<&BTreeSet<String>>, regex: &regex::Regex) -> BTreeSet<(String, String)> {
    Lang::of(dialect).map(|lang| lang.implied_in(kinds, regex, None)).unwrap_or_default()
}

/// The kinds that have one of `kinds` as a named child, in `dialect`'s grammar.
pub fn parents_of(dialect: GrammarDialect, kinds: &BTreeSet<String>) -> BTreeSet<String> {
    Lang::of(dialect).map(|lang| kinds.iter().flat_map(|kind| lang.parents.get(kind).into_iter().flatten().cloned()).collect()).unwrap_or_default()
}
