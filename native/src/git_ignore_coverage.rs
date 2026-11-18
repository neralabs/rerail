//! Descendant reachability for one ordered list of repository-root ignore patterns.

use gix::bstr::ByteSlice as _;
use gix::ignore::glob::{Pattern, pattern::Mode, wildmatch, wildmatch::Mode as Match};
use regex_automata::{
    Anchored, Input, MatchKind,
    dfa::{Automaton, StartKind, dense},
    util::{primitives::StateID, syntax},
};
use std::collections::HashSet;
use std::fmt::Write as _;

pub(super) enum IgnoreCoverage {
    NoNegations,
    Unknown,
    Compiled(Box<Coverage>),
}

pub(super) struct Coverage {
    dfa: dense::DFA<Vec<u32>>,
    start: StateID,
    outcomes: Vec<(bool, bool)>,
    name_bytes: Vec<u8>,
}

impl IgnoreCoverage {
    pub(super) fn new<'a>(patterns: impl IntoIterator<Item = &'a Pattern>) -> Self {
        let mut expressions = Vec::new();
        let mut outcomes = Vec::new();
        for pattern in patterns {
            let include = pattern.is_negative();
            let Some(regex) = pattern_regex(pattern) else {
                if include {
                    return Self::Unknown;
                }
                // A rule we cannot prove excludes a path cannot justify pruning.
                continue;
            };
            expressions.push(regex);
            outcomes.push((include, pattern.mode.contains(Mode::MUST_BE_DIR)));
        }
        if !outcomes.iter().any(|&(include, _)| include) {
            return Self::NoNegations;
        }
        let limit = Some(4 * 1024 * 1024);
        let Ok(dfa) = dense::Builder::new()
            .configure(dense::Config::new().start_kind(StartKind::Anchored).match_kind(MatchKind::All)
                .dfa_size_limit(limit).determinize_size_limit(limit))
            .syntax(syntax::Config::new().unicode(false).utf8(false).dot_matches_new_line(true))
            .build_many(&expressions)
        else {
            return Self::Unknown;
        };
        let Ok(start) = dfa.start_state_forward(&Input::new(b"").anchored(Anchored::Yes)) else {
            return Self::Unknown;
        };
        let name_bytes = dfa.byte_classes().representatives(1..b'/')
            .chain(dfa.byte_classes().representatives(b'/' + 1..=u8::MAX))
            .filter_map(|unit| unit.as_u8())
            .collect();
        Self::Compiled(Box::new(Coverage { dfa, start, outcomes, name_bytes }))
    }

    pub(super) fn may_reinclude(&self, directory: &str) -> bool {
        match self {
            Self::NoNegations => false,
            Self::Unknown => true,
            Self::Compiled(coverage) => coverage.may_reinclude(directory),
        }
    }
}

impl Coverage {
    fn may_reinclude(&self, directory: &str) -> bool {
        let mut state = self.start;
        for byte in directory.bytes().chain([b'/']) {
            state = self.dfa.next_state(state, byte);
        }
        let mut visited = HashSet::new();
        let mut frontier = vec![(state, false)];
        while let Some((state, in_name)) = frontier.pop() {
            if self.dfa.is_dead_state(state) || !visited.insert((state, in_name)) {
                continue;
            }
            // Keep the tree if analysis of an unusually large set of globs is incomplete.
            if visited.len() > 8192 || in_name && self.includes(state) {
                return true;
            }
            if in_name {
                frontier.push((self.dfa.next_state(state, b'/'), false));
            }
            for &byte in &self.name_bytes {
                frontier.push((self.dfa.next_state(state, byte), true));
            }
        }
        false
    }

    fn includes(&self, state: StateID) -> bool {
        let end = self.dfa.next_eoi_state(state);
        if !self.dfa.is_match_state(end) {
            return false;
        }
        let matches = (0..self.dfa.match_len(end)).map(|index| self.dfa.match_pattern(end, index).as_usize());
        let directory = matches.clone().max();
        let file = matches.filter(|&index| !self.outcomes[index].1).max();
        directory.into_iter().chain(file).any(|index| self.outcomes[index].0)
    }
}

fn pattern_regex(pattern: &Pattern) -> Option<String> {
    let mut regex = String::from(r"\A");
    if pattern.mode.contains(Mode::NO_SUB_DIR) && !pattern.mode.contains(Mode::ABSOLUTE) {
        regex.push_str("(?:[^/]+/)*");
    }
    let bytes = pattern.text.as_slice();
    // A positive match must also succeed within gix's wildcard recursion limit.
    if !pattern.is_negative() && bytes.iter().filter(|&&byte| byte == b'*').count() >= 63 {
        return None;
    }
    let mut pos = 0;
    while let Some(&byte) = bytes.get(pos) {
        match byte {
            b'*' => {
                let end = pos + bytes[pos..].iter().take_while(|&&byte| byte == b'*').count();
                let component = end - pos > 1
                    && (pos == 0 || bytes[pos - 1] == b'/')
                    && matches!(bytes.get(end), None | Some(b'/'));
                match (component, bytes.get(end)) {
                    (true, Some(b'/')) => {
                        regex.push_str("(?:[^/]+/)*");
                        pos = end + 1;
                    }
                    (true, None) => {
                        regex.push_str(".*");
                        pos = end;
                    }
                    _ => {
                        regex.push_str("[^/]*");
                        pos = end;
                    }
                }
                continue;
            }
            b'?' => regex.push_str("[^/]"),
            b'[' => {
                let len = class_len(&bytes[pos..])?;
                let class = bytes[pos..pos + len].as_bstr();
                regex.push('[');
                let first = regex.len();
                for candidate in (1..=u8::MAX).filter(|&candidate| candidate != b'/') {
                    if wildmatch(class, [candidate].as_bstr(), Match::NO_MATCH_SLASH_LITERAL) {
                        write!(regex, r"\x{candidate:02x}").ok()?;
                    }
                }
                if regex.len() == first {
                    return None;
                }
                regex.push(']');
                pos += len;
                continue;
            }
            b'\\' => {
                pos += 1;
                let literal = *bytes.get(pos)?;
                // Escaped separators affect gix's recursive-star shortcuts.
                if literal == b'/' {
                    return None;
                }
                write!(regex, r"\x{literal:02x}").ok()?;
            }
            _ => write!(regex, r"\x{byte:02x}").ok()?,
        }
        pos += 1;
    }
    regex.push_str(r"\z");
    Some(regex)
}

fn class_len(bytes: &[u8]) -> Option<usize> {
    let mut pos = 1;
    if matches!(bytes.get(pos), Some(b'!' | b'^')) {
        pos += 1;
    }
    if bytes.get(pos) == Some(&b']') {
        pos += 1;
    }
    while let Some(byte) = bytes.get(pos) {
        match byte {
            b'\\' => pos += 2,
            b']' => return Some(pos + 1),
            b'[' if bytes.get(pos + 1) == Some(&b':') => {
                let end = bytes[pos + 2..].find_byte(b']')? + pos + 2;
                if bytes.get(end - 1) != Some(&b':') {
                    return None;
                }
                pos = end + 1;
            }
            _ => pos += 1,
        }
    }
    None
}
