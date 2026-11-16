//! A dialect's token automata as the token tests' census asks them: the tokens a grammar may leave out under each
//! kind, and each kind's parents (`compile::token_tests`, `logic::rr::token_reads`).

use crate::code_rule_dsl::language::GrammarDialect;
use std::collections::{BTreeMap, BTreeSet};

/// One dialect's automata, as the census asks them.
pub struct Lang {
    pub dialect: GrammarDialect,
    pub implied: BTreeMap<String, BTreeSet<(String, String)>>,
    pub parents: BTreeMap<String, BTreeSet<String>>,
    pub tokens: BTreeSet<String>,
    pub extras: BTreeSet<String>,
}

impl Lang {
    /// Read once per dialect, as the build wrote it (`tokens::census`): no automaton is decoded for it.
    pub fn of(lang: GrammarDialect) -> Option<&'static Self> {
        static BY_DIALECT: [std::sync::OnceLock<Option<Lang>>; 256] = [const { std::sync::OnceLock::new() }; 256];
        BY_DIALECT
            .get(lang as usize)?
            .get_or_init(|| {
                let census = super::token_automata::decode_census(super::tokens::census(lang)?)?;
                Some(Self { dialect: lang, implied: census.implied, parents: census.parents, tokens: census.tokens, extras: census.extras })
            })
            .as_ref()
    }

    /// Whether `regex` tests a token the grammar implies somewhere, in `field` where one is given: it matches one. A
    /// regex that also matches names (`(?s).`, `^`) counts too: `not follows (?s).` asks for a first child, and a
    /// node after an implied `(` was never one. A `has` in a field whose implied words hold no such token asks only
    /// for stored children.
    pub fn tests_implied(&self, regex: &regex::Regex, field: Option<&str>) -> bool {
        self.implied.values().flatten().any(|(token, token_field)| field.is_none_or(|field| field == token_field) && regex.is_match(token))
    }

    /// Whether `kind` is one of the grammar's extras (a comment and the like).
    pub fn is_extra(&self, kind: &str) -> bool {
        self.extras.contains(kind)
    }

    /// The parents among `parents` (all kinds where `None`) where a token `regex` matches is
    /// implied, under `field` where one is given: (parent, token) pairs.
    pub fn implied_in(&self, parents: Option<&BTreeSet<String>>, regex: &regex::Regex, field: Option<&str>) -> BTreeSet<(String, String)> {
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
