//! A dialect's token automata as the token tests' census asks them: the tokens a grammar may leave out under each
//! kind, and each kind's parents (`compile::token_tests`, `logic::rr::token_reads`).

use crate::code_rule_dsl::language::GrammarDialect;
use std::collections::{BTreeMap, BTreeSet};

/// One dialect's automata, as the census asks them. Its names are the build's bytes (`tokens::census`), read where
/// they lie: the census is read as a dialect's first rule compiles, and copying its thousands of names was most of
/// that.
pub struct Lang {
    pub dialect: GrammarDialect,
    pub implied: BTreeMap<&'static str, BTreeSet<(&'static str, &'static str)>>,
    pub parents: BTreeMap<&'static str, BTreeSet<&'static str>>,
    pub tokens: BTreeSet<&'static str>,
    pub extras: BTreeSet<&'static str>,
    /// Every (token, field) pair `implied` holds, once (`tests_implied`): kinds share most of their implied
    /// tokens (`(`, `,`, `;`), and a regex is tested against each pair once, not once a kind.
    distinct: std::sync::OnceLock<Vec<(&'static str, &'static str)>>,
}

impl Lang {
    /// Read once per dialect, as the build wrote it (`tokens::census`): no automaton is decoded for it.
    pub fn of(lang: GrammarDialect) -> Option<&'static Self> {
        static BY_DIALECT: [std::sync::OnceLock<Option<Lang>>; 256] = [const { std::sync::OnceLock::new() }; 256];
        BY_DIALECT
            .get(lang as usize)?
            .get_or_init(|| {
                let census = super::token_automata::decode_census(super::tokens::census(lang)?)?;
                Some(Self { dialect: lang, implied: census.implied, parents: census.parents, tokens: census.tokens, extras: census.extras, distinct: std::sync::OnceLock::new() })
            })
            .as_ref()
    }

    /// Whether `regex` tests a token the grammar implies somewhere, in `field` where one is given: it matches one. A
    /// regex that also matches names (`(?s).`, `^`) counts too: `not follows (?s).` asks for a first child, and a
    /// node after an implied `(` was never one. A `has` in a field whose implied words hold no such token asks only
    /// for stored children.
    pub fn tests_implied(&self, regex: &regex::Regex, field: Option<&str>) -> bool {
        self.distinct().iter().any(|&(token, token_field)| field.is_none_or(|field| field == token_field) && regex.is_match(token))
    }

    /// Every (token, field) pair some kind may leave out, once.
    pub fn distinct(&self) -> &[(&'static str, &'static str)] {
        self.distinct.get_or_init(|| {
            let pairs: BTreeSet<&(&str, &str)> = self.implied.values().flatten().collect();
            pairs.into_iter().copied().collect()
        })
    }

    /// The kinds that have `kind` as a named child.
    pub fn parents_of(&self, kind: &str) -> Option<BTreeSet<String>> {
        self.parents.get(kind).map(|parents| parents.iter().map(|&parent| parent.to_owned()).collect())
    }

    /// Whether `kind` is one of the grammar's extras (a comment and the like).
    pub fn is_extra(&self, kind: &str) -> bool {
        self.extras.contains(kind)
    }

    /// The parents among `parents` (all kinds where `None`) where a token `regex` matches is
    /// implied, under `field` where one is given: (parent, token) pairs.
    pub fn implied_in(&self, parents: Option<&BTreeSet<String>>, regex: &regex::Regex, field: Option<&str>) -> BTreeSet<(String, String)> {
        self.implied_where(parents, &|token| regex.is_match(token), field)
    }

    /// `implied_in`, of the tokens `matches` holds.
    pub fn implied_where(&self, parents: Option<&BTreeSet<String>>, matches: &dyn Fn(&str) -> bool, field: Option<&str>) -> BTreeSet<(String, String)> {
        let mut out = BTreeSet::new();
        let kinds: Box<dyn Iterator<Item = &str>> = match parents {
            Some(parents) => Box::new(parents.iter().map(String::as_str)),
            None => Box::new(self.implied.keys().copied()),
        };
        for kind in kinds {
            for &(token, token_field) in self.implied.get(kind).into_iter().flatten() {
                if matches(token) && field.is_none_or(|field| field == token_field) {
                    out.insert((kind.to_owned(), token.to_owned()));
                }
            }
        }
        out
    }
}

/// The (parent kind, token) pairs where `dialect`'s grammar may leave out a token `regex` matches: under `kinds`, or
/// under any kind where `None`. A `.rerail` program's token reads are refused by this (`logic::rr::token_reads`).
pub fn implied(dialect: GrammarDialect, kinds: Option<&BTreeSet<String>>, regex: &regex::Regex) -> BTreeSet<(String, String)> {
    Lang::of(dialect).map(|lang| lang.implied_in(kinds, regex, None)).unwrap_or_default()
}

/// The kinds that have one of `kinds` as a named child, in `dialect`'s grammar.
pub fn parents_of(dialect: GrammarDialect, kinds: &BTreeSet<String>) -> BTreeSet<String> {
    Lang::of(dialect).map(|lang| kinds.iter().flat_map(|kind| lang.parents.get(kind.as_str()).into_iter().flatten().map(|&parent| parent.to_owned())).collect()).unwrap_or_default()
}
