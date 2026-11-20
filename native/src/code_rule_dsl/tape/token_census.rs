//! A dialect's token automata as the token tests' census asks them: the tokens a grammar may leave out under each
//! kind, and each kind's parents (`compile::token_tests`, `logic::rr::token_reads`).

use super::token_automata::{CensusIn, Words, decode_census};
use crate::code_rule_dsl::language::GrammarDialect;
use std::collections::BTreeSet;

/// One dialect's automata, as the census asks them, read where the build wrote them (`tokens::census`): the census is
/// read as a dialect's first rule compiles, and decoding its thousands of names into sets was most of what that rule's
/// compile cost a process. Its names are checked once; a kind's implied tokens or parents are found by a binary search
/// when asked.
pub struct Lang {
    pub dialect: GrammarDialect,
    census: CensusIn<'static>,
    /// Every (token, field) pair some kind may leave out, once (`tests_implied`): kinds share most of their implied
    /// tokens (`(`, `,`, `;`), and a regex is tested against each pair once, not once a kind.
    distinct: Vec<(&'static str, &'static str)>,
}

impl Lang {
    /// Read once per dialect, as the build wrote it (`tokens::census`): no automaton is decoded for it.
    pub fn of(lang: GrammarDialect) -> Option<&'static Self> {
        static BY_DIALECT: [std::sync::OnceLock<Option<Lang>>; 256] = [const { std::sync::OnceLock::new() }; 256];
        BY_DIALECT
            .get(lang as usize)?
            .get_or_init(|| {
                let census = decode_census(super::tokens::census(lang)?)?;
                let distinct = (0..census.distinct.len())
                    .map(|at| (census.names[census.distinct.get(at, 0) as usize], census.names[census.distinct.get(at, 1) as usize]))
                    .collect();
                Some(Self { dialect: lang, census, distinct })
            })
            .as_ref()
    }

    fn name(&self, at: u32) -> &'static str {
        self.census.names[at as usize]
    }

    /// `name`'s index among the census's names, which are sorted.
    fn index(&self, name: &str) -> Option<u32> {
        self.census.names.binary_search(&name).ok().map(|at| at as u32)
    }

    /// Whether `regex` tests a token the grammar implies somewhere, in `field` where one is given: it matches one. A
    /// regex that also matches names (`(?s).`, `^`) counts too: `not follows (?s).` asks for a first child, and a
    /// node after an implied `(` was never one. A `has` in a field whose implied words hold no such token asks only
    /// for stored children.
    pub fn tests_implied(&self, regex: &regex::Regex, field: Option<&str>) -> bool {
        self.distinct.iter().any(|&(token, token_field)| field.is_none_or(|field| field == token_field) && regex.is_match(token))
    }

    /// Every (token, field) pair some kind may leave out, once.
    pub fn distinct(&self) -> &[(&'static str, &'static str)] {
        &self.distinct
    }

    /// Every token text some automaton moves on, in order.
    pub fn tokens(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.census.tokens.firsts().map(|at| self.name(at))
    }

    /// The (token, field) pairs a tape of `kind` may leave out, in order (none for a kind without an automaton).
    pub fn implied_of(&self, kind: &str) -> impl Iterator<Item = (&'static str, &'static str)> + '_ {
        let pairs = self.index(kind).and_then(|kind| self.census.implied.find(kind)).map(|at| self.range(self.census.implied, self.census.implied_pairs, at));
        pairs.into_iter().flat_map(move |pairs| (0..pairs.len()).map(move |at| (self.name(pairs.get(at, 0)), self.name(pairs.get(at, 1)))))
    }

    /// Every kind with an automaton and the (token, field) pairs a tape of it may leave out, in the kinds' order.
    pub fn implied(&self) -> impl Iterator<Item = (&'static str, impl Iterator<Item = (&'static str, &'static str)> + '_)> + '_ {
        (0..self.census.implied.len()).map(move |at| {
            let pairs = self.range(self.census.implied, self.census.implied_pairs, at);
            (self.name(self.census.implied.get(at, 0)), (0..pairs.len()).map(move |at| (self.name(pairs.get(at, 0)), self.name(pairs.get(at, 1)))))
        })
    }

    /// Every kind some automaton has as a named child, and the kinds whose automata do, in the kinds' order.
    pub fn parents(&self) -> impl Iterator<Item = (&'static str, impl Iterator<Item = &'static str> + '_)> + '_ {
        (0..self.census.parents.len()).map(move |at| {
            (self.name(self.census.parents.get(at, 0)), self.range(self.census.parents, self.census.parent_kinds, at).firsts().map(|parent| self.name(parent)))
        })
    }

    /// Entry `at` of `ranges` (kind, start, length): its range of `items`.
    fn range(&self, ranges: Words<'static>, items: Words<'static>, at: usize) -> Words<'static> {
        items.slice(ranges.get(at, 1) as usize, ranges.get(at, 2) as usize)
    }

    /// The kinds whose automata have `kind` as a named child, in order.
    fn parents_iter(&self, kind: &str) -> impl Iterator<Item = &'static str> + '_ {
        let parents = self.index(kind).and_then(|kind| self.census.parents.find(kind)).map(|at| self.range(self.census.parents, self.census.parent_kinds, at));
        parents.into_iter().flat_map(|parents| parents.firsts()).map(|parent| self.name(parent))
    }

    /// The kinds that have `kind` as a named child.
    pub fn parents_of(&self, kind: &str) -> Option<BTreeSet<String>> {
        let at = self.index(kind).and_then(|kind| self.census.parents.find(kind))?;
        Some(self.range(self.census.parents, self.census.parent_kinds, at).firsts().map(|parent| self.name(parent).to_owned()).collect())
    }

    /// Whether `kind` is one of the grammar's extras (a comment and the like).
    pub fn is_extra(&self, kind: &str) -> bool {
        self.index(kind).is_some_and(|kind| self.census.extras.find(kind).is_some())
    }

    /// The parents among `parents` (all kinds where `None`) where a token `regex` matches is
    /// implied, under `field` where one is given: (parent, token) pairs.
    pub fn implied_in(&self, parents: Option<&BTreeSet<String>>, regex: &regex::Regex, field: Option<&str>) -> BTreeSet<(String, String)> {
        self.implied_where(parents, &|token| regex.is_match(token), field)
    }

    /// `implied_in`, of the tokens `matches` holds.
    pub fn implied_where(&self, parents: Option<&BTreeSet<String>>, matches: &dyn Fn(&str) -> bool, field: Option<&str>) -> BTreeSet<(String, String)> {
        let mut out = BTreeSet::new();
        let mut add = |kind: &str, pairs: &mut dyn Iterator<Item = (&'static str, &'static str)>| {
            for (token, token_field) in pairs {
                if matches(token) && field.is_none_or(|field| field == token_field) {
                    out.insert((kind.to_owned(), token.to_owned()));
                }
            }
        };
        match parents {
            Some(parents) => parents.iter().for_each(|kind| add(kind, &mut self.implied_of(kind))),
            None => self.implied().for_each(|(kind, mut pairs)| add(kind, &mut pairs)),
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
    Lang::of(dialect).map(|lang| kinds.iter().flat_map(|kind| lang.parents_iter(kind)).map(str::to_owned).collect()).unwrap_or_default()
}
