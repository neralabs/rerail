use super::{
    CapturePredicate, Error, LuaItem, LuaMatcher, LuaPattern, LuaRepetition, LuaSetMember,
    TextPredicate,
};
use regex_automata::{
    Input,
    dfa::{Automaton, dense},
    nfa::thompson::{self, State},
    util::{
        look::{Look, LookMatcher},
        primitives::StateID,
    },
};
use std::collections::{HashMap, VecDeque};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum CaptureProgram {
    Equal {
        left: u16,
        right: u16,
        negated: bool,
        any: bool,
    },
    Text {
        capture: u16,
        value: u16,
        negated: bool,
        any: bool,
    },
    AnyOf {
        capture: u16,
        strings: Vec<u16>,
        negated: bool,
        any: bool,
    },
    Kind {
        capture: u16,
        kinds: Vec<u16>,
        negated: bool,
        any: bool,
        empty_all: bool,
    },
    Contains {
        capture: u16,
        strings: Vec<u16>,
        negated: bool,
        any: bool,
    },
    Relation {
        capture: u16,
        kinds: Vec<u16>,
        ancestor: bool,
        negated: bool,
    },
    SameLine {
        left: u16,
        right: u16,
        negated: bool,
    },
}

impl CaptureProgram {
    pub(super) fn lower(predicate: &CapturePredicate) -> Self {
        match predicate {
            CapturePredicate::Equal {
                left,
                right,
                negated,
                any,
            } => Self::Equal {
                left: *left,
                right: *right,
                negated: *negated,
                any: *any,
            },
            CapturePredicate::Text {
                capture,
                predicate,
                negated,
                any,
            } => Self::Text {
                capture: *capture,
                value: *predicate,
                negated: *negated,
                any: *any,
            },
            CapturePredicate::AnyOf {
                capture,
                strings,
                negated,
                any,
            } => Self::AnyOf {
                capture: *capture,
                strings: strings.clone(),
                negated: *negated,
                any: *any,
            },
            CapturePredicate::Kind {
                capture,
                kind,
                negated,
                any,
            } => Self::Kind {
                capture: *capture,
                kinds: vec![*kind],
                negated: *negated,
                any: *any,
                empty_all: false,
            },
            CapturePredicate::Kinds {
                capture,
                kinds,
                negated,
            } => Self::Kind {
                capture: *capture,
                kinds: kinds.clone(),
                negated: *negated,
                any: false,
                empty_all: true,
            },
            CapturePredicate::Contains {
                capture,
                strings,
                negated,
                any,
            } => Self::Contains {
                capture: *capture,
                strings: strings.clone(),
                negated: *negated,
                any: *any,
            },
            CapturePredicate::Relation {
                capture,
                kinds,
                ancestor,
                negated,
            } => Self::Relation {
                capture: *capture,
                kinds: kinds.clone(),
                ancestor: *ancestor,
                negated: *negated,
            },
            CapturePredicate::SameLine {
                left,
                right,
                negated,
            } => Self::SameLine {
                left: *left,
                right: *right,
                negated: *negated,
            },
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum ValueProgram {
    Equal { string: u16 },
    Prefix { string: u16 },
    Regex(ByteNfa),
    Lua(LuaProgram),
}

impl ValueProgram {
    pub(super) fn maximum_lua_capture_slot(&self) -> u32 {
        match self {
            Self::Lua(program) => program.capture_count,
            _ => 0,
        }
    }

    pub(super) fn lower(predicate: &TextPredicate, strings: &[Vec<u8>]) -> Result<Self, Error> {
        Ok(match predicate {
            TextPredicate::Equal(string) => Self::Equal { string: *string },
            TextPredicate::Prefix(string) => Self::Prefix { string: *string },
            TextPredicate::Regex { source, .. } => Self::Regex(ByteNfa::compile(
                std::str::from_utf8(&strings[usize::from(*source)])
                    .map_err(|_| Error::InvalidOperand(usize::from(*source)))?,
            )?),
            TextPredicate::LuaPattern { compiled, .. } => Self::Lua(LuaProgram::lower(compiled)),
        })
    }

    pub(super) fn canonical_bytes(&self) -> Vec<u8> {
        fn u32_bytes(output: &mut Vec<u8>, value: u32) {
            output.extend_from_slice(&value.to_le_bytes());
        }
        fn matcher(output: &mut Vec<u8>, found: &ValueMatcher) {
            match found {
                ValueMatcher::Any => output.push(0),
                ValueMatcher::Byte(byte) => output.extend([1, *byte]),
                ValueMatcher::Class { class, negated } => {
                    output.extend([2, *class, u8::from(*negated)]);
                }
                ValueMatcher::Set { members, negated } => {
                    output.extend([3, u8::from(*negated)]);
                    u32_bytes(output, members.len() as u32);
                    for member in members {
                        match member {
                            ValueSetMember::Byte(byte) => output.extend([0, *byte]),
                            ValueSetMember::Range(start, end) => {
                                output.extend([1, *start, *end]);
                            }
                            ValueSetMember::Class { class, negated } => {
                                output.extend([2, *class, u8::from(*negated)]);
                            }
                        }
                    }
                }
            }
        }
        let mut output = Vec::new();
        match self {
            Self::Equal { string } => {
                output.push(0);
                output.extend_from_slice(&string.to_le_bytes());
            }
            Self::Prefix { string } => {
                output.push(1);
                output.extend_from_slice(&string.to_le_bytes());
            }
            Self::Regex(program) => {
                output.push(2);
                u32_bytes(&mut output, program.start);
                u32_bytes(&mut output, program.anchored_start);
                u32_bytes(&mut output, program.states.len() as u32);
                for state in &program.states {
                    match state {
                        NfaState::Bytes(ranges) => {
                            output.push(0);
                            u32_bytes(&mut output, ranges.len() as u32);
                            for (start, end, next) in ranges {
                                output.extend([*start, *end]);
                                u32_bytes(&mut output, *next);
                            }
                        }
                        NfaState::Look { kind, next } => {
                            output.push(1);
                            u32_bytes(&mut output, *kind as u32);
                            u32_bytes(&mut output, *next);
                        }
                        NfaState::Split(alternatives) => {
                            output.push(2);
                            u32_bytes(&mut output, alternatives.len() as u32);
                            for alternative in alternatives {
                                u32_bytes(&mut output, *alternative);
                            }
                        }
                        NfaState::Capture { slot, next } => {
                            output.push(3);
                            u32_bytes(&mut output, *slot);
                            u32_bytes(&mut output, *next);
                        }
                        NfaState::Fail => output.push(4),
                        NfaState::Match => output.push(5),
                    }
                }
            }
            Self::Lua(program) => {
                output.extend([
                    3,
                    u8::from(program.anchored),
                    u8::from(program.end_anchored),
                ]);
                u32_bytes(&mut output, program.capture_count);
                u32_bytes(&mut output, program.items.len() as u32);
                for item in &program.items {
                    match item {
                        LuaInstruction::Atom {
                            matcher: found,
                            repetition,
                        } => {
                            output.extend([0, *repetition]);
                            matcher(&mut output, found);
                        }
                        LuaInstruction::Frontier(found) => {
                            output.push(1);
                            matcher(&mut output, found);
                        }
                        LuaInstruction::Balanced { open, close } => {
                            output.extend([2, *open, *close]);
                        }
                        LuaInstruction::CaptureStart(slot) => {
                            output.push(3);
                            u32_bytes(&mut output, *slot);
                        }
                        LuaInstruction::CaptureEnd(slot) => {
                            output.push(4);
                            u32_bytes(&mut output, *slot);
                        }
                        LuaInstruction::PositionCapture(slot) => {
                            output.push(5);
                            u32_bytes(&mut output, *slot);
                        }
                        LuaInstruction::Backreference(slot) => {
                            output.push(6);
                            u32_bytes(&mut output, *slot);
                        }
                    }
                }
            }
        }
        output
    }

    pub(super) fn is_match(
        &self,
        bytes: &[u8],
        strings: &[Vec<u8>],
        work: &mut usize,
    ) -> Result<bool, Error> {
        match self {
            Self::Equal { string } => Ok(bytes == strings[usize::from(*string)]),
            Self::Prefix { string } => Ok(bytes.starts_with(&strings[usize::from(*string)])),
            Self::Regex(program) => program.is_match(bytes),
            Self::Lua(program) => program.is_match(bytes, work),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct LuaProgram {
    pub(super) anchored: bool,
    pub(super) end_anchored: bool,
    pub(super) capture_count: u32,
    pub(super) items: Vec<LuaInstruction>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum LuaInstruction {
    Atom {
        matcher: ValueMatcher,
        repetition: u8,
    },
    Frontier(ValueMatcher),
    Balanced {
        open: u8,
        close: u8,
    },
    CaptureStart(u32),
    CaptureEnd(u32),
    PositionCapture(u32),
    Backreference(u32),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum ValueMatcher {
    Any,
    Byte(u8),
    Class {
        class: u8,
        negated: bool,
    },
    Set {
        members: Vec<ValueSetMember>,
        negated: bool,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum ValueSetMember {
    Byte(u8),
    Range(u8, u8),
    Class { class: u8, negated: bool },
}

impl ValueMatcher {
    fn class_matches(class: u8, byte: u8) -> bool {
        match class.to_ascii_lowercase() {
            b'a' => byte.is_ascii_alphabetic(),
            b'c' => byte.is_ascii_control(),
            b'd' => byte.is_ascii_digit(),
            b'g' => byte.is_ascii_graphic(),
            b'l' => byte.is_ascii_lowercase(),
            b'p' => byte.is_ascii_punctuation(),
            b's' => byte.is_ascii_whitespace(),
            b'u' => byte.is_ascii_uppercase(),
            b'w' => byte.is_ascii_alphanumeric(),
            b'x' => byte.is_ascii_hexdigit(),
            b'z' => byte == 0,
            _ => false,
        }
    }

    fn matches(&self, byte: u8) -> bool {
        match self {
            Self::Any => true,
            Self::Byte(expected) => byte == *expected,
            Self::Class { class, negated } => Self::class_matches(*class, byte) != *negated,
            Self::Set { members, negated } => {
                let found = members.iter().any(|member| match member {
                    ValueSetMember::Byte(expected) => byte == *expected,
                    ValueSetMember::Range(start, end) => (*start..=*end).contains(&byte),
                    ValueSetMember::Class { class, negated } => {
                        Self::class_matches(*class, byte) != *negated
                    }
                });
                found != *negated
            }
        }
    }
}

#[derive(Clone)]
enum ValueCapture {
    Open(usize),
    Range(std::ops::Range<usize>),
    Position,
}

impl LuaProgram {
    fn lower(pattern: &LuaPattern) -> Self {
        let matcher = |matcher: &LuaMatcher| match matcher {
            LuaMatcher::Any => ValueMatcher::Any,
            LuaMatcher::Byte(byte) => ValueMatcher::Byte(*byte),
            LuaMatcher::Class { class, negated } => ValueMatcher::Class {
                class: *class,
                negated: *negated,
            },
            LuaMatcher::Set { members, negated } => ValueMatcher::Set {
                members: members
                    .iter()
                    .map(|member| match member {
                        LuaSetMember::Byte(byte) => ValueSetMember::Byte(*byte),
                        LuaSetMember::Range(start, end) => ValueSetMember::Range(*start, *end),
                        LuaSetMember::Class { class, negated } => ValueSetMember::Class {
                            class: *class,
                            negated: *negated,
                        },
                    })
                    .collect(),
                negated: *negated,
            },
        };
        let items = pattern
            .items
            .iter()
            .map(|item| match item {
                LuaItem::Atom {
                    matcher: found,
                    repetition,
                } => LuaInstruction::Atom {
                    matcher: matcher(found),
                    repetition: match repetition {
                        LuaRepetition::One => 0,
                        LuaRepetition::Optional => 1,
                        LuaRepetition::ZeroOrMore { greedy: true } => 2,
                        LuaRepetition::ZeroOrMore { greedy: false } => 3,
                        LuaRepetition::OneOrMore => 4,
                    },
                },
                LuaItem::Frontier(found) => LuaInstruction::Frontier(matcher(found)),
                LuaItem::Balanced { open, close } => LuaInstruction::Balanced {
                    open: *open,
                    close: *close,
                },
                LuaItem::CaptureStart(slot) => LuaInstruction::CaptureStart(*slot as u32),
                LuaItem::CaptureEnd(slot) => LuaInstruction::CaptureEnd(*slot as u32),
                LuaItem::PositionCapture(slot) => LuaInstruction::PositionCapture(*slot as u32),
                LuaItem::Backreference(slot) => LuaInstruction::Backreference(*slot as u32),
            })
            .collect();
        Self {
            anchored: pattern.anchored,
            end_anchored: pattern.end_anchored,
            capture_count: pattern.capture_count as u32,
            items,
        }
    }

    fn is_match(&self, bytes: &[u8], work: &mut usize) -> Result<bool, Error> {
        fn suffix(
            program: &LuaProgram,
            item: usize,
            bytes: &[u8],
            at: usize,
            captures: Vec<Option<ValueCapture>>,
            work: &mut usize,
        ) -> Result<bool, Error> {
            *work += 1;
            if *work > super::MAX_STEPS_PER_PATH {
                return Err(Error::WorkLimit);
            }
            let Some(current) = program.items.get(item) else {
                return Ok(!program.end_anchored || at == bytes.len());
            };
            match current {
                LuaInstruction::Atom {
                    matcher,
                    repetition,
                } => {
                    let available = bytes[at..]
                        .iter()
                        .take_while(|byte| matcher.matches(**byte))
                        .count();
                    let (minimum, maximum, greedy) = match repetition {
                        0 => (1, 1, true),
                        1 => (0, 1, true),
                        2 => (0, available, true),
                        3 => (0, available, false),
                        4 => (1, available, true),
                        _ => return Err(Error::InvalidOperand(item)),
                    };
                    if available < minimum {
                        return Ok(false);
                    }
                    let counts: Box<dyn Iterator<Item = usize>> = if greedy {
                        Box::new((minimum..=maximum).rev())
                    } else {
                        Box::new(minimum..=maximum)
                    };
                    for count in counts {
                        if suffix(program, item + 1, bytes, at + count, captures.clone(), work)? {
                            return Ok(true);
                        }
                    }
                    Ok(false)
                }
                LuaInstruction::Frontier(set) => {
                    let previous = at.checked_sub(1).map_or(0, |index| bytes[index]);
                    let current = bytes.get(at).copied().unwrap_or(0);
                    if !set.matches(previous) && set.matches(current) {
                        suffix(program, item + 1, bytes, at, captures, work)
                    } else {
                        Ok(false)
                    }
                }
                LuaInstruction::Balanced { open, close } => {
                    if bytes.get(at) != Some(open) {
                        return Ok(false);
                    }
                    let mut depth = 1_usize;
                    let mut end = at + 1;
                    while let Some(byte) = bytes.get(end) {
                        *work += 1;
                        if *work > super::MAX_STEPS_PER_PATH {
                            return Err(Error::WorkLimit);
                        }
                        if byte == open {
                            depth += 1;
                        } else if byte == close {
                            depth -= 1;
                            if depth == 0 {
                                return suffix(program, item + 1, bytes, end + 1, captures, work);
                            }
                        }
                        end += 1;
                    }
                    Ok(false)
                }
                LuaInstruction::CaptureStart(slot) => {
                    let mut captures = captures;
                    captures[*slot as usize] = Some(ValueCapture::Open(at));
                    suffix(program, item + 1, bytes, at, captures, work)
                }
                LuaInstruction::CaptureEnd(slot) => {
                    let mut captures = captures;
                    let Some(ValueCapture::Open(start)) = captures[*slot as usize] else {
                        return Ok(false);
                    };
                    captures[*slot as usize] = Some(ValueCapture::Range(start..at));
                    suffix(program, item + 1, bytes, at, captures, work)
                }
                LuaInstruction::PositionCapture(slot) => {
                    let mut captures = captures;
                    captures[*slot as usize] = Some(ValueCapture::Position);
                    suffix(program, item + 1, bytes, at, captures, work)
                }
                LuaInstruction::Backreference(slot) => {
                    let Some(Some(ValueCapture::Range(range))) = captures.get(*slot as usize)
                    else {
                        return Ok(false);
                    };
                    let expected = &bytes[range.clone()];
                    let Some(end) = at.checked_add(expected.len()) else {
                        return Ok(false);
                    };
                    if bytes.get(at..end) == Some(expected) {
                        suffix(program, item + 1, bytes, end, captures, work)
                    } else {
                        Ok(false)
                    }
                }
            }
        }
        let starts: Box<dyn Iterator<Item = usize>> = if self.anchored {
            Box::new(std::iter::once(0))
        } else {
            Box::new(0..=bytes.len())
        };
        for at in starts {
            if suffix(
                self,
                0,
                bytes,
                at,
                vec![None; self.capture_count as usize],
                work,
            )? {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ByteDfa {
    start: u32,
    transitions: Vec<[u32; 256]>,
    eoi: Vec<u32>,
    matches: Vec<bool>,
    dead: Vec<bool>,
    quit: Vec<bool>,
}

impl ByteDfa {
    pub(super) fn compile(source: &str) -> Result<Self, Error> {
        let dfa = dense::Builder::new()
            .configure(dense::Config::new().unicode_word_boundary(true))
            .build(source)
            .map_err(|_| Error::UnsupportedPattern)?;
        let start = dfa
            .start_state_forward(&Input::new(b""))
            .map_err(|_| Error::UnsupportedPattern)?;
        let mut indexes = HashMap::<StateID, u32>::new();
        let mut queue = VecDeque::new();
        indexes.insert(start, 0);
        queue.push_back(start);
        let mut transitions = Vec::new();
        let mut eoi = Vec::new();
        let mut matches = Vec::new();
        let mut dead = Vec::new();
        let mut quit = Vec::new();
        while let Some(state) = queue.pop_front() {
            let index = indexes[&state] as usize;
            debug_assert_eq!(index, transitions.len());
            let mut row = [0_u32; 256];
            for (byte, target) in row.iter_mut().zip(0_u8..=u8::MAX) {
                let next = dfa.next_state(state, target);
                *byte = Self::intern(next, &mut indexes, &mut queue);
            }
            let next_eoi = dfa.next_eoi_state(state);
            transitions.push(row);
            eoi.push(Self::intern(next_eoi, &mut indexes, &mut queue));
            matches.push(dfa.is_match_state(state));
            dead.push(dfa.is_dead_state(state));
            quit.push(dfa.is_quit_state(state));
        }
        Ok(Self {
            start: 0,
            transitions,
            eoi,
            matches,
            dead,
            quit,
        })
    }

    fn intern(
        state: StateID,
        indexes: &mut HashMap<StateID, u32>,
        queue: &mut VecDeque<StateID>,
    ) -> u32 {
        if let Some(index) = indexes.get(&state) {
            return *index;
        }
        let index = indexes.len() as u32;
        indexes.insert(state, index);
        queue.push_back(state);
        index
    }

    pub(super) fn is_match(&self, bytes: &[u8]) -> Result<bool, Error> {
        let mut state = self.start;
        for &byte in bytes {
            state = self.transitions[state as usize][byte as usize];
            if self.matches[state as usize] {
                return Ok(true);
            }
            if self.dead[state as usize] {
                return Ok(false);
            }
            if self.quit[state as usize] {
                return Err(Error::UnsupportedPattern);
            }
        }
        state = self.eoi[state as usize];
        if self.quit[state as usize] {
            return Err(Error::UnsupportedPattern);
        }
        Ok(self.matches[state as usize])
    }

    pub(super) fn state_count(&self) -> usize {
        self.transitions.len()
    }
}

struct NfaRun<'a> {
    bytes: &'a [u8],
    look: &'a LookMatcher,
    seen: &'a mut [bool],
    consuming: &'a mut Vec<u32>,
    work: &'a mut usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ByteNfa {
    pub(super) start: u32,
    pub(super) anchored_start: u32,
    pub(super) states: Vec<NfaState>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum NfaState {
    Bytes(Vec<(u8, u8, u32)>),
    Look { kind: Look, next: u32 },
    Split(Vec<u32>),
    Capture { slot: u32, next: u32 },
    Fail,
    Match,
}

impl ByteNfa {
    pub(super) fn compile(source: &str) -> Result<Self, Error> {
        let nfa = thompson::NFA::new(source).map_err(|_| Error::UnsupportedPattern)?;
        let states = nfa
            .states()
            .iter()
            .map(|state| match state {
                State::ByteRange { trans } => {
                    NfaState::Bytes(vec![(trans.start, trans.end, trans.next.as_usize() as u32)])
                }
                State::Sparse(sparse) => NfaState::Bytes(
                    sparse
                        .transitions
                        .iter()
                        .map(|trans| (trans.start, trans.end, trans.next.as_usize() as u32))
                        .collect(),
                ),
                State::Dense(dense) => {
                    let mut ranges = Vec::<(u8, u8, u32)>::new();
                    for byte in 0_u8..=u8::MAX {
                        let Some(next) = dense.matches_byte(byte) else {
                            continue;
                        };
                        let next = next.as_usize() as u32;
                        if let Some((_, end, previous)) = ranges.last_mut()
                            && *previous == next
                            && *end != u8::MAX
                            && *end + 1 == byte
                        {
                            *end = byte;
                        } else {
                            ranges.push((byte, byte, next));
                        }
                    }
                    NfaState::Bytes(ranges)
                }
                State::Look { look, next } => NfaState::Look {
                    kind: *look,
                    next: next.as_usize() as u32,
                },
                State::Union { alternates } => NfaState::Split(
                    alternates
                        .iter()
                        .map(|state| state.as_usize() as u32)
                        .collect(),
                ),
                State::BinaryUnion { alt1, alt2 } => {
                    NfaState::Split(vec![alt1.as_usize() as u32, alt2.as_usize() as u32])
                }
                State::Capture { next, slot, .. } => NfaState::Capture {
                    slot: slot.as_usize() as u32,
                    next: next.as_usize() as u32,
                },
                State::Fail => NfaState::Fail,
                State::Match { .. } => NfaState::Match,
            })
            .collect();
        Ok(Self {
            start: nfa.start_unanchored().as_usize() as u32,
            anchored_start: nfa.start_anchored().as_usize() as u32,
            states,
        })
    }

    pub(super) fn is_match(&self, bytes: &[u8]) -> Result<bool, Error> {
        let look = LookMatcher::new();
        let mut current = Vec::new();
        let mut seen = vec![false; self.states.len()];
        let mut work = 0_usize;
        if self.add_state(
            self.start,
            0,
            &mut NfaRun {
                bytes,
                look: &look,
                seen: &mut seen,
                consuming: &mut current,
                work: &mut work,
            },
        )? {
            return Ok(true);
        }
        for (at, &byte) in bytes.iter().enumerate() {
            let mut next = Vec::new();
            seen.fill(false);
            for &state in &current {
                let NfaState::Bytes(ranges) = &self.states[state as usize] else {
                    continue;
                };
                for &(start, end, target) in ranges {
                    work += 1;
                    if work > super::MAX_STEPS_PER_PATH {
                        return Err(Error::WorkLimit);
                    }
                    if (start..=end).contains(&byte)
                        && self.add_state(
                            target,
                            at + 1,
                            &mut NfaRun {
                                bytes,
                                look: &look,
                                seen: &mut seen,
                                consuming: &mut next,
                                work: &mut work,
                            },
                        )?
                    {
                        return Ok(true);
                    }
                }
            }
            current = next;
        }
        Ok(false)
    }

    fn add_state(&self, state: u32, at: usize, run: &mut NfaRun<'_>) -> Result<bool, Error> {
        let mut pending = vec![state];
        while let Some(state) = pending.pop() {
            *run.work += 1;
            if *run.work > super::MAX_STEPS_PER_PATH {
                return Err(Error::WorkLimit);
            }
            let index = state as usize;
            if run.seen[index] {
                continue;
            }
            run.seen[index] = true;
            match &self.states[index] {
                NfaState::Bytes(_) => run.consuming.push(state),
                NfaState::Look { kind, next } => {
                    if run.look.matches(*kind, run.bytes, at) {
                        pending.push(*next);
                    }
                }
                NfaState::Split(alternatives) => {
                    pending.extend(alternatives.iter().rev().copied());
                }
                NfaState::Capture { next, .. } => pending.push(*next),
                NfaState::Fail => {}
                NfaState::Match => return Ok(true),
            }
        }
        Ok(false)
    }
}
