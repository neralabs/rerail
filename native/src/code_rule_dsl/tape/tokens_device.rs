//! The token automata as the device reads them (RER-131): Metal's `token_parts` kernel recognizes
//! a named-only document's tokens from these tables, as [`TokenGrammar::sequence`] does on the
//! host. Hidden text is matched by [`HiddenText`], one table both sides run, so the two cannot
//! read a gap differently.
//!
//! The tables are `u32` words, offsets in words from the start:
//!
//! - **Header** (`HEADER` words): the symbol count, where `automaton_of` starts, the field count,
//!   where `field_sym` starts, where the token texts start and their length in bytes.
//! - **`automaton_of`**: per symbol, where its automaton starts, or `NONE`.
//! - **`field_sym`**: per tree-sitter field id, its symbol.
//! - **An automaton**: its state count, then per state `STATE` words (accepting, where its moves
//!   start, how many, where its token moves start, how many).
//! - **A move** (`MOVE` words, sorted by label): the label's key (`token << 31 | name`), its field,
//!   the state it goes to.
//! - **A token move** (`TOKEN_MOVE` words, in recognition order): where its hidden text starts
//!   (`NONE` for a token), where its text starts (bytes, in the text area) and how long it is, the
//!   state it goes to, and its tree-sitter kind and field ids (`kind | field << 16`; kind
//!   `NO_KIND` where the language has none).
//! - **Hidden text**: its start state, class count and state count, the 256 byte classes (one a
//!   byte), then per state and class the next state, per state its end-of-input state, per state
//!   its flags (`HIDDEN_MATCH`, `HIDDEN_DEAD`).

use super::{Automaton, Label, Sym, TokenGrammar};
use rustc_hash::FxHashMap;
use std::sync::Arc;

pub(in crate::code_rule_dsl) const NONE: u32 = u32::MAX;
/// A record with no name in the grammar (`DeviceCode::reserved`).
pub(in crate::code_rule_dsl) const NO_SYM: u16 = u16::MAX;
pub(in crate::code_rule_dsl) const NO_KIND: u32 = 0xffff;
const HEADER: usize = 8;
const STATE: usize = 5;
const MOVE: usize = 3;
const TOKEN_MOVE: usize = 5;
const HIDDEN_MATCH: u8 = 1;
const HIDDEN_DEAD: u8 = 2;
/// Hidden text a DFA of more states than this cannot read (no edge is made for it).
const HIDDEN_STATES: usize = 4096;

/// Hidden text's matcher: an anchored, leftmost-first DFA as a table of byte classes, run the same
/// way on the host (`recognize`) and the device (`token_parts`).
pub(in crate::code_rule_dsl) struct HiddenText {
    classes: [u8; 256],
    class_count: usize,
    start: u32,
    /// Per state and class: the next state.
    next: Vec<u32>,
    /// Per state: the state past the end of input.
    eoi: Vec<u32>,
    flags: Vec<u8>,
}

impl HiddenText {
    /// The matcher of `pattern` (Rust dialect, matched from the start), or `None` where it has no
    /// such DFA (a Unicode word boundary, too many states).
    pub(in crate::code_rule_dsl) fn new(pattern: &str) -> Option<Self> {
        use regex_automata::dfa::{Automaton as _, StartKind, dense};
        use regex_automata::nfa::thompson;
        use regex_automata::util::syntax;
        use regex_automata::{Anchored, Input, MatchKind};
        let dfa = dense::Builder::new()
            .configure(dense::DFA::config().match_kind(MatchKind::LeftmostFirst).start_kind(StartKind::Anchored))
            .syntax(syntax::Config::new().utf8(false))
            .thompson(thompson::Config::new().utf8(false))
            .build(pattern)
            .ok()?;
        let start = dfa.start_state_forward(&Input::new(&[][..]).anchored(Anchored::Yes)).ok()?;
        let byte_classes = dfa.byte_classes();
        let mut classes = [0_u8; 256];
        let mut representative = Vec::<u8>::new();
        for byte in 0..=255_u8 {
            let class = byte_classes.get(byte);
            classes[usize::from(byte)] = class;
            if usize::from(class) >= representative.len() {
                representative.resize(usize::from(class) + 1, byte);
            }
        }
        let class_count = representative.len();
        let mut ids = vec![start];
        let mut index = FxHashMap::default();
        index.insert(start, 0_u32);
        let (mut next, mut eoi, mut flags) = (Vec::new(), Vec::new(), Vec::new());
        let mut at = 0;
        while at < ids.len() {
            let id = ids[at];
            if dfa.is_quit_state(id) {
                return None;
            }
            flags.push(u8::from(dfa.is_match_state(id)) * HIDDEN_MATCH | u8::from(dfa.is_dead_state(id)) * HIDDEN_DEAD);
            let mut number = |state| {
                let fresh = index.len() as u32;
                *index.entry(state).or_insert_with(|| {
                    ids.push(state);
                    fresh
                })
            };
            for &byte in &representative {
                next.push(number(dfa.next_state(id, byte)));
            }
            eoi.push(number(dfa.next_eoi_state(id)));
            if ids.len() > HIDDEN_STATES {
                return None;
            }
            at += 1;
        }
        Some(Self { classes, class_count, start: 0, next, eoi, flags })
    }

    /// Where the match at the start of `text` ends, as `regex`'s `find` would end it.
    pub(in crate::code_rule_dsl) fn find(&self, text: &[u8]) -> Option<usize> {
        let mut state = self.start as usize;
        let mut last = None;
        for (at, &byte) in text.iter().enumerate() {
            state = self.next[state * self.class_count + usize::from(self.classes[usize::from(byte)])] as usize;
            if self.flags[state] & HIDDEN_MATCH != 0 {
                last = Some(at);
            } else if self.flags[state] & HIDDEN_DEAD != 0 {
                return last;
            }
        }
        state = self.eoi[state] as usize;
        if self.flags[state] & HIDDEN_MATCH != 0 {
            last = Some(text.len());
        }
        last
    }

    fn words(&self, out: &mut Vec<u32>) {
        let states = self.flags.len();
        out.extend([self.start, self.class_count as u32, states as u32]);
        out.extend(self.classes.chunks(4).map(|four| u32::from_le_bytes(four.try_into().expect("four bytes"))));
        out.extend(&self.next);
        out.extend(&self.eoi);
        out.extend(self.flags.iter().map(|&flag| u32::from(flag)));
    }
}

/// A grammar's tables for the device; see the module documentation.
pub(in crate::code_rule_dsl) struct DeviceTokens {
    pub(in crate::code_rule_dsl) words: Vec<u32>,
}

fn key(label: &Label) -> (u32, u32) {
    (u32::from(label.token) << 31 | label.name, label.field)
}

impl TokenGrammar {
    /// A record's name as the device's tables index it: `child_name`'s symbol, [`NO_SYM`] where
    /// it has none (an ERROR, a name the grammar does not know).
    pub(in crate::code_rule_dsl) fn device_sym(&self, kind: u16, alias: Option<&str>, error: bool) -> u16 {
        if error {
            return NO_SYM;
        }
        let sym = match alias {
            Some(alias) => self.names.get(alias).copied(),
            None => self.kind_sym.get(usize::from(kind)).copied().flatten(),
        };
        sym.and_then(|sym| u16::try_from(sym).ok()).filter(|&sym| sym != NO_SYM).unwrap_or(NO_SYM)
    }

    /// The grammar's device tables, built once.
    pub(in crate::code_rule_dsl) fn device(&self) -> Arc<DeviceTokens> {
        Arc::clone(self.device.get_or_init(|| Arc::new(self.build_device())))
    }

    fn build_device(&self) -> DeviceTokens {
        let syms = self.texts.len();
        let mut words = vec![0_u32; HEADER];
        words[0] = syms as u32;
        words[1] = words.len() as u32;
        words.resize(words.len() + syms, NONE);
        words[2] = self.field_sym.len() as u32;
        words[3] = words.len() as u32;
        words.extend(&self.field_sym);
        // The texts of tokens, by symbol, as bytes; placed after the rest.
        let mut text = Vec::<u8>::new();
        let mut text_at = FxHashMap::<Sym, (u32, u32)>::default();
        let mut hidden_at = FxHashMap::<Sym, u32>::default();
        let mut automata: Vec<(&Sym, &Automaton)> = self.kinds.iter().collect();
        automata.sort_by_key(|(sym, _)| **sym);
        for (&sym, automaton) in automata {
            let start = words.len();
            words[HEADER + sym as usize] = start as u32;
            let states = automaton.moves.len();
            words.push(states as u32);
            let table = words.len();
            words.resize(table + states * STATE, 0);
            for state in 0..states {
                let mut moves: Vec<((u32, u32), u32)> = automaton.moves[state].iter().map(|(label, &to)| (key(label), to)).collect();
                moves.sort_unstable();
                let moves_at = words.len() as u32;
                for ((k0, k1), to) in &moves {
                    words.extend([*k0, *k1, *to]);
                }
                let tokens_at = words.len() as u32;
                let mut hidden_pending = Vec::new();
                for (label, to) in &automaton.tokens[state] {
                    let kind = self.token_id.get(&label.name).map_or(NO_KIND, |&id| u32::from(id));
                    let field = self.field_id.get(&label.field).map_or(0, |&id| u32::from(id));
                    let (text_start, text_len) = *text_at.entry(label.name).or_insert_with(|| {
                        let bytes = self.texts[label.name as usize].as_bytes();
                        let start = text.len() as u32;
                        text.extend_from_slice(bytes);
                        (start, bytes.len() as u32)
                    });
                    let entry = words.len();
                    words.extend([NONE, text_start, text_len, *to, kind | field << 16]);
                    if self.hidden_text.contains_key(&label.name) {
                        hidden_pending.push((entry, label.name));
                    }
                }
                for (entry, name) in hidden_pending {
                    let at = match hidden_at.get(&name) {
                        Some(&at) => at,
                        None => {
                            let at = words.len() as u32;
                            self.hidden_text[&name].words(&mut words);
                            hidden_at.insert(name, at);
                            at
                        }
                    };
                    words[entry] = at;
                }
                let row = table + state * STATE;
                words[row..row + STATE].copy_from_slice(&[
                    u32::from(automaton.accepting[state]),
                    moves_at,
                    moves.len() as u32,
                    tokens_at,
                    automaton.tokens[state].len() as u32,
                ]);
            }
        }
        words[4] = words.len() as u32;
        words[5] = text.len() as u32;
        text.resize(text.len().div_ceil(4) * 4, 0);
        words.extend(text.chunks(4).map(|four| u32::from_le_bytes(four.try_into().expect("four bytes"))));
        DeviceTokens { words }
    }
}

#[cfg(test)]
mod tests {
    use super::HiddenText;

    /// The table finds what `regex` finds, from the start of the text.
    #[test]
    fn hidden_text_finds_as_regex_does() {
        for pattern in [r"\\\r?\n", r"[^\n]*", r"a|ab", r"(ab)+", r"\s+", r"x?", r"[0-9]+(\.[0-9]+)?"] {
            let table = HiddenText::new(pattern).expect(pattern);
            let regex = regex::bytes::Regex::new(&format!("^(?:{pattern})")).unwrap();
            for text in ["", "\\\n", "\\\r\nx", "abab!", "ab", "a", "  \t x", "12.5e", "12.", "x"] {
                assert_eq!(table.find(text.as_bytes()), regex.find(text.as_bytes()).map(|found| found.end()), "{pattern} on {text:?}");
            }
        }
    }
}
