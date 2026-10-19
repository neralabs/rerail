#![allow(dead_code)]

use super::tape::TapeDoc;
use ast_grep_core::MatchStrictness;
use ast_grep_core::matcher::{Pattern, PatternNode};
use ast_grep_core::meta_var::{MetaVarEnv, MetaVariable};
use ast_grep_core::{Doc, Node};
use regex::Regex;
use sha2::{Digest as _, Sha256};
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::ops::Range;
use std::sync::Arc;
#[cfg(target_os = "macos")]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(target_os = "macos")]
use std::sync::{LazyLock, mpsc};
#[cfg(target_os = "macos")]
use std::time::Duration;

#[cfg(target_os = "macos")]
mod metal_backend;
mod metal_batch_abi;
#[cfg(target_os = "macos")]
mod metal_program_backend;
mod native_cpu;
mod native_cpu_capture;
mod native_cpu_capture_pair;
mod native_cpu_lua;
mod native_cpu_output;
mod native_cpu_output_sort;
mod native_cpu_output_transform;
mod native_cpu_reduction;
mod native_cpu_regex;
mod native_cpu_scheduler;
mod native_cpu_value;
mod native_ir;
mod output;
mod packed_arena;
mod packed_batch;
#[cfg(target_os = "macos")]
mod packed_workspace;
mod scheduler;
mod value;
mod value_packing;

const STOP: u8 = 0x00;
const KIND: u8 = 0x01;
const FIELD: u8 = 0x02;
const FLAGS: u8 = 0x03;
const TEXT: u8 = 0x04;
const SIZE: u8 = 0x05;
const ASSERT: u8 = 0x06;
const FIRST: u8 = 0x07;
const LAST: u8 = 0x08;
const ABSENT_FIELD: u8 = 0x09;
const SUPERTYPE: u8 = 0x0a;
const CHILD: u8 = 0x10;
const NEXT: u8 = 0x11;
const PARENT: u8 = 0x12;
const DESCEND: u8 = 0x13;
const FOLLOWING: u8 = 0x14;
const SAVE_CURSOR: u8 = 0x15;
const RESTORE_CURSOR: u8 = 0x16;
const CAPTURE: u8 = 0x20;
const REPEAT_BEGIN: u8 = 0x21;
const REPEAT_BUMP: u8 = 0x22;
const REPEAT_END: u8 = 0x23;
const FORK: u8 = 0x30;
const GOTO: u8 = 0x31;
const EMIT: u8 = 0x40;

pub(super) const NAMED: u8 = 1;
const GREEDY: u8 = 1;
const EXTRA: u8 = 2;
const MISSING: u8 = 8;
const SIZE_LONG: u8 = 0x80;
const MAX_STEPS_PER_PATH: usize = 1 << 20;
const MAX_WORK_PER_TASK: usize = 1 << 26;

#[derive(Clone)]
pub(super) enum TextPredicate {
    Equal(u16),
    Prefix(u16),
    Regex { source: u16, compiled: Regex },
    LuaPattern { source: u16, compiled: LuaPattern },
}

#[derive(Clone)]
pub(super) struct LuaPattern {
    anchored: bool,
    items: Vec<LuaItem>,
    end_anchored: bool,
    capture_count: usize,
}

#[derive(Clone)]
enum LuaItem {
    Atom {
        matcher: LuaMatcher,
        repetition: LuaRepetition,
    },
    Frontier(LuaMatcher),
    Balanced {
        open: u8,
        close: u8,
    },
    CaptureStart(usize),
    CaptureEnd(usize),
    PositionCapture(usize),
    Backreference(usize),
}

#[derive(Clone)]
enum LuaMatcher {
    Any,
    Byte(u8),
    Class {
        class: u8,
        negated: bool,
    },
    Set {
        members: Vec<LuaSetMember>,
        negated: bool,
    },
}

#[derive(Clone)]
enum LuaSetMember {
    Byte(u8),
    Range(u8, u8),
    Class { class: u8, negated: bool },
}

#[derive(Clone, Copy)]
enum LuaRepetition {
    One,
    Optional,
    ZeroOrMore { greedy: bool },
    OneOrMore,
}

#[derive(Clone)]
enum LuaCapture {
    Open(usize),
    Range(Range<usize>),
    Position,
}

impl LuaMatcher {
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
                    LuaSetMember::Byte(expected) => byte == *expected,
                    LuaSetMember::Range(start, end) => (*start..=*end).contains(&byte),
                    LuaSetMember::Class { class, negated } => {
                        Self::class_matches(*class, byte) != *negated
                    }
                });
                found != *negated
            }
        }
    }
}

impl LuaPattern {
    fn set(source: &[u8], at: &mut usize) -> Result<LuaMatcher, ()> {
        if source.get(*at) != Some(&b'[') {
            return Err(());
        }
        *at += 1;
        let negated = source.get(*at) == Some(&b'^');
        *at += usize::from(negated);
        let mut members = Vec::new();
        while source.get(*at).is_some_and(|byte| *byte != b']') {
            if source[*at] == b'%' {
                let class = *source.get(*at + 1).ok_or(())?;
                if !b"acdglpsuwxz".contains(&class.to_ascii_lowercase()) {
                    return Err(());
                }
                members.push(LuaSetMember::Class {
                    class,
                    negated: class.is_ascii_uppercase(),
                });
                *at += 2;
            } else if source.get(*at + 1) == Some(&b'-')
                && source.get(*at + 2).is_some_and(|byte| *byte != b']')
            {
                members.push(LuaSetMember::Range(source[*at], source[*at + 2]));
                *at += 3;
            } else {
                members.push(LuaSetMember::Byte(source[*at]));
                *at += 1;
            }
        }
        if members.is_empty() || source.get(*at) != Some(&b']') {
            return Err(());
        }
        *at += 1;
        Ok(LuaMatcher::Set { members, negated })
    }

    fn parse(source: &[u8]) -> Result<Self, ()> {
        let mut at = usize::from(source.first() == Some(&b'^'));
        let anchored = at == 1;
        let mut items = Vec::new();
        let mut end_anchored = false;
        let mut capture_stack = Vec::new();
        let mut capture_count = 0_usize;
        let mut position_captures = Vec::new();
        let mut closed_captures = Vec::new();
        while at < source.len() {
            if source[at] == b'(' {
                if capture_count >= 32 {
                    return Err(());
                }
                let capture = capture_count;
                capture_count += 1;
                at += 1;
                if source.get(at) == Some(&b')') {
                    at += 1;
                    position_captures.push(true);
                    closed_captures.push(true);
                    items.push(LuaItem::PositionCapture(capture));
                } else {
                    position_captures.push(false);
                    closed_captures.push(false);
                    capture_stack.push(capture);
                    items.push(LuaItem::CaptureStart(capture));
                }
                continue;
            }
            if source[at] == b')' {
                let capture = capture_stack.pop().ok_or(())?;
                closed_captures[capture] = true;
                items.push(LuaItem::CaptureEnd(capture));
                at += 1;
                continue;
            }
            if source[at] == b'$' && at + 1 == source.len() {
                end_anchored = true;
                break;
            }
            if source[at] == b'%' && source.get(at + 1) == Some(&b'b') {
                let open = *source.get(at + 2).ok_or(())?;
                let close = *source.get(at + 3).ok_or(())?;
                items.push(LuaItem::Balanced { open, close });
                at += 4;
                continue;
            }
            if source[at] == b'%' && source.get(at + 1) == Some(&b'f') {
                at += 2;
                items.push(LuaItem::Frontier(Self::set(source, &mut at)?));
                continue;
            }
            if source[at] == b'%'
                && source
                    .get(at + 1)
                    .is_some_and(|byte| (b'1'..=b'9').contains(byte))
            {
                let capture = usize::from(source[at + 1] - b'1');
                if capture >= capture_count
                    || position_captures[capture]
                    || !closed_captures[capture]
                {
                    return Err(());
                }
                items.push(LuaItem::Backreference(capture));
                at += 2;
                continue;
            }
            let matcher = match source[at] {
                b'.' => {
                    at += 1;
                    LuaMatcher::Any
                }
                b'%' => {
                    let escaped = *source.get(at + 1).ok_or(())?;
                    at += 2;
                    if b"acdglpsuwxzACDGLPSUWXZ".contains(&escaped) {
                        LuaMatcher::Class {
                            class: escaped,
                            negated: escaped.is_ascii_uppercase(),
                        }
                    } else if escaped.is_ascii_punctuation() {
                        LuaMatcher::Byte(escaped)
                    } else {
                        return Err(());
                    }
                }
                b'[' => Self::set(source, &mut at)?,
                byte => {
                    at += 1;
                    LuaMatcher::Byte(byte)
                }
            };
            let repetition = match source.get(at) {
                Some(b'?') => {
                    at += 1;
                    LuaRepetition::Optional
                }
                Some(b'*') => {
                    at += 1;
                    LuaRepetition::ZeroOrMore { greedy: true }
                }
                Some(b'-') => {
                    at += 1;
                    LuaRepetition::ZeroOrMore { greedy: false }
                }
                Some(b'+') => {
                    at += 1;
                    LuaRepetition::OneOrMore
                }
                _ => LuaRepetition::One,
            };
            items.push(LuaItem::Atom {
                matcher,
                repetition,
            });
        }
        if !capture_stack.is_empty() {
            return Err(());
        }
        Ok(Self {
            anchored,
            items,
            end_anchored,
            capture_count,
        })
    }

    fn matches_with_work(&self, bytes: &[u8], work: &mut usize) -> Result<bool, ()> {
        fn suffix(
            pattern: &LuaPattern,
            item: usize,
            bytes: &[u8],
            at: usize,
            captures: Vec<Option<LuaCapture>>,
            work: &mut usize,
        ) -> Result<bool, ()> {
            *work += 1;
            if *work > MAX_STEPS_PER_PATH {
                return Err(());
            }
            let Some(current) = pattern.items.get(item) else {
                return Ok(!pattern.end_anchored || at == bytes.len());
            };
            match current {
                LuaItem::Atom {
                    matcher,
                    repetition,
                } => {
                    let available = bytes[at..]
                        .iter()
                        .take_while(|byte| matcher.matches(**byte))
                        .count();
                    let (minimum, maximum, greedy) = match repetition {
                        LuaRepetition::One => (1, 1, true),
                        LuaRepetition::Optional => (0, 1, true),
                        LuaRepetition::ZeroOrMore { greedy } => (0, available, *greedy),
                        LuaRepetition::OneOrMore => (1, available, true),
                    };
                    if available < minimum {
                        return Ok(false);
                    }
                    let mut counts = (minimum..=maximum).collect::<Vec<_>>();
                    if greedy {
                        counts.reverse();
                    }
                    for count in counts {
                        if suffix(pattern, item + 1, bytes, at + count, captures.clone(), work)? {
                            return Ok(true);
                        }
                    }
                    Ok(false)
                }
                LuaItem::Frontier(set) => {
                    let previous = at.checked_sub(1).map_or(0, |index| bytes[index]);
                    let current = bytes.get(at).copied().unwrap_or(0);
                    if !set.matches(previous) && set.matches(current) {
                        suffix(pattern, item + 1, bytes, at, captures, work)
                    } else {
                        Ok(false)
                    }
                }
                LuaItem::Balanced { open, close } => {
                    if bytes.get(at) != Some(open) {
                        return Ok(false);
                    }
                    let mut depth = 1_usize;
                    let mut end = at + 1;
                    while let Some(byte) = bytes.get(end) {
                        *work += 1;
                        if *work > MAX_STEPS_PER_PATH {
                            return Err(());
                        }
                        if byte == open {
                            depth += 1;
                        } else if byte == close {
                            depth -= 1;
                            if depth == 0 {
                                return suffix(pattern, item + 1, bytes, end + 1, captures, work);
                            }
                        }
                        end += 1;
                    }
                    Ok(false)
                }
                LuaItem::CaptureStart(capture) => {
                    let mut captures = captures;
                    captures[*capture] = Some(LuaCapture::Open(at));
                    suffix(pattern, item + 1, bytes, at, captures, work)
                }
                LuaItem::CaptureEnd(capture) => {
                    let mut captures = captures;
                    let Some(LuaCapture::Open(start)) = captures[*capture] else {
                        return Ok(false);
                    };
                    captures[*capture] = Some(LuaCapture::Range(start..at));
                    suffix(pattern, item + 1, bytes, at, captures, work)
                }
                LuaItem::PositionCapture(capture) => {
                    let mut captures = captures;
                    captures[*capture] = Some(LuaCapture::Position);
                    suffix(pattern, item + 1, bytes, at, captures, work)
                }
                LuaItem::Backreference(capture) => {
                    let Some(Some(LuaCapture::Range(range))) = captures.get(*capture) else {
                        return Ok(false);
                    };
                    let expected = &bytes[range.clone()];
                    let Some(end) = at.checked_add(expected.len()) else {
                        return Ok(false);
                    };
                    if bytes.get(at..end) == Some(expected) {
                        suffix(pattern, item + 1, bytes, end, captures, work)
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
            if suffix(self, 0, bytes, at, vec![None; self.capture_count], work)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn matches(&self, bytes: &[u8]) -> bool {
        self.matches_with_work(bytes, &mut 0).unwrap_or(false)
    }
}

impl TextPredicate {
    fn string_index(&self) -> u16 {
        match self {
            Self::Equal(index)
            | Self::Prefix(index)
            | Self::Regex { source: index, .. }
            | Self::LuaPattern { source: index, .. } => *index,
        }
    }

    fn matches(&self, bytes: &[u8], strings: &[Vec<u8>]) -> bool {
        self.matches_with_work(bytes, strings, &mut 0)
            .unwrap_or(false)
    }

    fn matches_with_work(
        &self,
        bytes: &[u8],
        strings: &[Vec<u8>],
        work: &mut usize,
    ) -> Result<bool, Error> {
        let expected = &strings[usize::from(self.string_index())];
        Ok(match self {
            Self::Equal(_) => bytes == expected,
            Self::Prefix(_) => bytes.starts_with(expected),
            Self::Regex { compiled, .. } => {
                std::str::from_utf8(bytes).is_ok_and(|text| compiled.is_match(text))
            }
            Self::LuaPattern { compiled, .. } => compiled
                .matches_with_work(bytes, work)
                .map_err(|()| Error::WorkLimit)?,
        })
    }
}

#[derive(Clone, Debug)]
enum PackedPattern {
    Terminal {
        text: String,
        named: bool,
        kind: u16,
    },
    Single {
        name: Option<String>,
        named: bool,
    },
    Ellipsis(Option<String>),
    Internal {
        kind: u16,
        children: Vec<PackedPattern>,
    },
}

pub(super) struct AstPatternCapture {
    pub(super) label: String,
    pub(super) occurrence: usize,
    pub(super) node: u32,
    pub(super) multi: bool,
}

#[derive(Clone, Default)]
struct PatternBindings {
    values: BTreeMap<String, Vec<u32>>,
    multi: BTreeSet<String>,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum PatternVerdict {
    Matched,
    SkipGoal,
    SkipBoth,
    SkipCandidate,
    NoMatch,
}

#[derive(Clone)]
pub(super) struct Program {
    words: Vec<u32>,
    strings: Vec<Vec<u8>>,
    predicates: Vec<TextPredicate>,
    value_programs: Vec<value::ValueProgram>,
    capture_predicates: Vec<CapturePredicate>,
    capture_programs: Vec<value::CaptureProgram>,
    labels: Vec<u16>,
    longs: Vec<u64>,
    directives: Vec<Directive>,
    output_program: output::OutputProgram,
    tree_sitter_query: bool,
    ast_pattern: Option<PackedPattern>,
}

#[derive(Clone)]
pub(super) enum Directive {
    Set {
        key: u16,
        value: Option<u16>,
    },
    SetCapture {
        capture: u16,
        key: u16,
        value: Option<u16>,
    },
    Offset {
        capture: u16,
        offsets: [i32; 4],
    },
    Strip {
        capture: u16,
        source: u16,
        compiled: Regex,
    },
    SelectAdjacent {
        capture: u16,
        target: u16,
    },
    Downcase {
        capture: u16,
    },
    SetLanguageFromMimeType {
        capture: u16,
    },
}

#[derive(Clone)]
pub(super) enum CapturePredicate {
    Equal {
        left: u16,
        right: u16,
        negated: bool,
        any: bool,
    },
    Text {
        capture: u16,
        predicate: u16,
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
        kind: u16,
        negated: bool,
        any: bool,
    },
    Kinds {
        capture: u16,
        kinds: Vec<u16>,
        negated: bool,
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Capture {
    pub(super) label: u16,
    pub(super) node: u32,
    pub(super) span: Range<usize>,
    pub(super) text: Option<Vec<u8>>,
    pub(super) properties: Vec<(u16, Option<u16>)>,
}

type DynamicProperty = (Vec<u8>, Option<Vec<u8>>);

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Output {
    pub(super) task_ordinal: u32,
    pub(super) local_ordinal: u32,
    pub(super) start: u32,
    pub(super) label: u16,
    pub(super) captures: Vec<Capture>,
    pub(super) properties: Vec<(u16, Option<u16>)>,
    pub(super) dynamic_properties: Vec<DynamicProperty>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ExecutionBackend {
    CpuInterpreter,
    CpuJit,
    MetalInterpreter,
    MetalJit,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum Error {
    InvalidOpcode(usize),
    InvalidMode(usize),
    InvalidOperand(usize),
    InvalidTarget(usize),
    NonProgressingCycle(usize),
    WorkLimit,
    UnsupportedPattern,
    UnsupportedFeature(&'static str),
    UnknownNodeKind(String),
    UnknownField(String),
    UnknownCapture(String),
    ScmParse { offset: usize, message: String },
    InvalidAnchor,
    UnsupportedBackend,
    BackendCapacity,
    BackendFailure,
}

#[derive(Clone, Copy)]
struct Path {
    pc: u16,
    node: u32,
    start: u32,
    captures: u32,
    cursors: u32,
    active_repeats: u32,
    steps: usize,
}

struct ExecutionArena {
    captures: Vec<Vec<Capture>>,
    ordered_captures: Vec<Arc<[CaptureIdentity]>>,
    capture_transitions: HashMap<(u32, u16, u32), u32>,
    cursors: Vec<Vec<Option<u32>>>,
    cursor_indexes: HashMap<Vec<Option<u32>>, u32>,
    repeats: Vec<Vec<u16>>,
    repeat_indexes: HashMap<Vec<u16>, u32>,
    greedy_paths: Vec<Option<Path>>,
    greedy_indexes: HashMap<(u16, u32, u32, u32, u32), u32>,
}

impl Default for ExecutionArena {
    fn default() -> Self {
        Self {
            captures: vec![Vec::new()],
            ordered_captures: vec![Arc::from([])],
            capture_transitions: HashMap::new(),
            cursors: vec![Vec::new()],
            cursor_indexes: HashMap::from([(Vec::new(), 0)]),
            repeats: vec![Vec::new()],
            repeat_indexes: HashMap::from([(Vec::new(), 0)]),
            greedy_paths: Vec::new(),
            greedy_indexes: HashMap::new(),
        }
    }
}

impl ExecutionArena {
    fn capture(&mut self, state: u32, capture: Capture) -> u32 {
        let key = (state, capture.label, capture.node);
        if let Some(next) = self.capture_transitions.get(&key) {
            return *next;
        }
        let mut captures = self.captures[state as usize].clone();
        captures.push(capture.clone());
        let mut ordered = self.ordered_captures[state as usize].to_vec();
        let identity = CaptureIdentity {
            label: capture.label,
            node: capture.node,
            start: capture.span.start,
            end: capture.span.end,
        };
        let position = ordered
            .iter()
            .position(|existing| {
                compare_capture_nodes(*existing, identity) == std::cmp::Ordering::Greater
            })
            .unwrap_or(ordered.len());
        ordered.insert(position, identity);
        let next = self.captures.len() as u32;
        self.captures.push(captures);
        self.ordered_captures.push(ordered.into());
        self.capture_transitions.insert(key, next);
        next
    }

    fn save_cursor(&mut self, state: u32, slot: u16, node: u32) -> u32 {
        let mut cursors = self.cursors[state as usize].clone();
        cursors.resize(usize::from(slot) + 1, None);
        cursors[usize::from(slot)] = Some(node);
        if let Some(next) = self.cursor_indexes.get(&cursors) {
            return *next;
        }
        let next = self.cursors.len() as u32;
        self.cursors.push(cursors.clone());
        self.cursor_indexes.insert(cursors, next);
        next
    }

    fn push_repeat(&mut self, state: u32, repeat: u16) -> u32 {
        let mut repeats = self.repeats[state as usize].clone();
        repeats.push(repeat);
        if let Some(next) = self.repeat_indexes.get(&repeats) {
            return *next;
        }
        let next = self.repeats.len() as u32;
        self.repeats.push(repeats.clone());
        self.repeat_indexes.insert(repeats, next);
        next
    }

    fn schedule_greedy(&mut self, path: Path) -> Option<u32> {
        let key = (
            path.pc,
            path.start,
            path.captures,
            path.cursors,
            path.active_repeats,
        );
        if let Some(index) = self.greedy_indexes.get(&key).copied() {
            self.greedy_paths[index as usize] = Some(path);
            return None;
        }
        let index = self.greedy_paths.len() as u32;
        self.greedy_paths.push(Some(path));
        self.greedy_indexes.insert(key, index);
        Some(index)
    }

    fn take_greedy(&mut self, index: u32) -> Option<Path> {
        let path = self.greedy_paths[index as usize].take()?;
        let key = (
            path.pc,
            path.start,
            path.captures,
            path.cursors,
            path.active_repeats,
        );
        self.greedy_indexes.remove(&key);
        Some(path)
    }

    fn pop_repeat(&mut self, state: u32, repeat: u16) -> Option<u32> {
        let mut repeats = self.repeats[state as usize].clone();
        if repeats.pop() != Some(repeat) {
            return None;
        }
        if let Some(next) = self.repeat_indexes.get(&repeats) {
            return Some(*next);
        }
        let next = self.repeats.len() as u32;
        self.repeats.push(repeats.clone());
        self.repeat_indexes.insert(repeats, next);
        Some(next)
    }
}

enum WorkItem {
    Path(Path),
    Greedy(u32),
    Following {
        path: Path,
        node: u32,
        next_pc: u16,
        named: bool,
    },
}

impl WorkItem {
    #[cfg(test)]
    fn retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
    }
}

fn split(word: u32) -> (u8, u8, u16) {
    (word as u8, (word >> 8) as u8, (word >> 16) as u16)
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
enum SiblingRequirement {
    None,
    Immediate,
    First,
    Last,
}

#[derive(Clone, Copy, Debug, Default)]
#[cfg_attr(not(test), allow(dead_code))]
struct ExecutionStats {
    explored_states: usize,
    max_pending_paths: usize,
    peak_retained_bytes: usize,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct CaptureIdentity {
    label: u16,
    node: u32,
    start: usize,
    end: usize,
}

/// Mirrors `ts_query_cursor__compare_nodes`. Distinct zero-width nodes at one point compare equal
/// for ordering, but remain different identities in capture containment.
fn compare_capture_nodes(left: CaptureIdentity, right: CaptureIdentity) -> std::cmp::Ordering {
    if left.node == right.node {
        return std::cmp::Ordering::Equal;
    }
    left.start
        .cmp(&right.start)
        .then_with(|| right.end.cmp(&left.end))
}

/// Mirrors `ts_query_cursor__compare_captures`: determine whether either ordered capture list is a
/// superset of the other.
fn capture_identity_containment(
    left: &[CaptureIdentity],
    right: &[CaptureIdentity],
) -> (bool, bool) {
    let mut left_contains_right = true;
    let mut right_contains_left = true;
    let (mut left_at, mut right_at) = (0, 0);
    loop {
        match (left.get(left_at), right.get(right_at)) {
            (Some(left), Some(right)) if left.node == right.node && left.label == right.label => {
                left_at += 1;
                right_at += 1;
            }
            (Some(left), Some(right)) => match compare_capture_nodes(*left, *right) {
                std::cmp::Ordering::Less => {
                    right_contains_left = false;
                    left_at += 1;
                }
                std::cmp::Ordering::Greater => {
                    left_contains_right = false;
                    right_at += 1;
                }
                std::cmp::Ordering::Equal => {
                    left_contains_right = false;
                    right_contains_left = false;
                    left_at += 1;
                    right_at += 1;
                }
            },
            (Some(_), None) => {
                right_contains_left = false;
                break;
            }
            (None, Some(_)) => {
                left_contains_right = false;
                break;
            }
            (None, None) => break,
        }
    }
    (left_contains_right, right_contains_left)
}

fn ordered_capture_identities(captures: &[Capture]) -> Vec<CaptureIdentity> {
    let mut identities = captures
        .iter()
        .map(|capture| CaptureIdentity {
            label: capture.label,
            node: capture.node,
            start: capture.span.start,
            end: capture.span.end,
        })
        .collect::<Vec<_>>();
    identities.sort_by(|left, right| compare_capture_nodes(*left, *right));
    identities
}

fn capture_containment(left: &[Capture], right: &[Capture]) -> (bool, bool) {
    capture_identity_containment(
        &ordered_capture_identities(left),
        &ordered_capture_identities(right),
    )
}

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
struct QueryStateIdentity {
    pc: u16,
    node: u32,
    cursors: u32,
    active_repeats: u32,
    sibling_requirement: SiblingRequirement,
    captures: u32,
}

type QueryStateKey = (u16, u32, u32, u32, SiblingRequirement);

#[derive(Default)]
struct CaptureAntichain {
    states: Vec<Option<Vec<CaptureIdentity>>>,
    by_length: BTreeMap<usize, Vec<usize>>,
}

fn charge_task_work(work: &mut usize) -> Result<(), Error> {
    *work = work.checked_add(1).ok_or(Error::WorkLimit)?;
    if *work > MAX_WORK_PER_TASK {
        return Err(Error::WorkLimit);
    }
    Ok(())
}

impl CaptureAntichain {
    fn insert_unless_contained(
        &mut self,
        captures: Vec<CaptureIdentity>,
        work: &mut usize,
    ) -> Result<bool, Error> {
        for index in self
            .by_length
            .range(captures.len()..)
            .flat_map(|(_, indexes)| indexes)
        {
            let Some(existing) = self.states[*index].as_ref() else {
                continue;
            };
            charge_task_work(work)?;
            if capture_identity_containment(existing, &captures).0 {
                return Ok(false);
            }
        }
        let dominated = self
            .by_length
            .range(..=captures.len())
            .flat_map(|(_, indexes)| indexes)
            .copied()
            .collect::<Vec<_>>();
        for index in dominated {
            let Some(existing) = self.states[index].as_ref() else {
                continue;
            };
            charge_task_work(work)?;
            if capture_identity_containment(&captures, existing).0 {
                self.states[index] = None;
            }
        }
        let length = captures.len();
        let index = self.states.len();
        self.states.push(Some(captures));
        self.by_length.entry(length).or_default().push(index);
        Ok(true)
    }
}

pub(super) const fn word(opcode: u8, mode: u8, operand: u16) -> u32 {
    opcode as u32 | ((mode as u32) << 8) | ((operand as u32) << 16)
}

impl PackedPattern {
    fn lower(node: &PatternNode) -> Self {
        match node {
            PatternNode::Terminal {
                text,
                is_named,
                kind_id,
            } => Self::Terminal {
                text: text.clone(),
                named: *is_named,
                kind: *kind_id,
            },
            PatternNode::MetaVar { meta_var } => match meta_var {
                MetaVariable::Capture(name, named) => Self::Single {
                    name: Some(name.clone()),
                    named: *named,
                },
                MetaVariable::Dropped(named) => Self::Single {
                    name: None,
                    named: *named,
                },
                MetaVariable::Multiple => Self::Ellipsis(None),
                MetaVariable::MultiCapture(name) => Self::Ellipsis(Some(name.clone())),
            },
            PatternNode::Internal { kind_id, children } => Self::Internal {
                kind: *kind_id,
                children: children.iter().map(Self::lower).collect(),
            },
        }
    }

    fn is_trivial(&self) -> bool {
        matches!(self, Self::Terminal { named: false, .. })
    }

    fn exact(tape: &TapeDoc, left: u32, right: u32) -> bool {
        if left == right {
            return true;
        }
        if tape.transducer_is_named_leaf(left) || tape.transducer_is_named_leaf(right) {
            return tape.transducer_text(left) == tape.transducer_text(right);
        }
        if tape.transducer_kind(left) != tape.transducer_kind(right) {
            return false;
        }
        let left = tape.transducer_children(left).collect::<Vec<_>>();
        let right = tape.transducer_children(right).collect::<Vec<_>>();
        left.len() == right.len()
            && left
                .into_iter()
                .zip(right)
                .all(|(left, right)| Self::exact(tape, left, right))
    }

    fn bind_single(bindings: &mut PatternBindings, name: &str, node: u32, tape: &TapeDoc) -> bool {
        match bindings.values.get(name) {
            Some(bound) => bound.len() == 1 && Self::exact(tape, bound[0], node),
            None => {
                bindings.values.insert(name.to_owned(), vec![node]);
                true
            }
        }
    }

    fn bind_multi(
        bindings: &mut PatternBindings,
        name: &str,
        nodes: Vec<u32>,
        tape: &TapeDoc,
    ) -> bool {
        match bindings.values.get(name) {
            Some(bound) => {
                let bound = bound
                    .iter()
                    .copied()
                    .filter(|node| tape.transducer_flags(*node) & NAMED != 0)
                    .collect::<Vec<_>>();
                let candidate = nodes
                    .iter()
                    .copied()
                    .filter(|node| tape.transducer_flags(*node) & NAMED != 0)
                    .collect::<Vec<_>>();
                bound.len() == candidate.len()
                    && bound
                        .into_iter()
                        .zip(candidate)
                        .all(|(left, right)| Self::exact(tape, left, right))
            }
            None => {
                bindings.multi.insert(name.to_owned());
                bindings.values.insert(name.to_owned(), nodes);
                true
            }
        }
    }

    fn one(
        &self,
        node: u32,
        tape: &TapeDoc,
        bindings: &mut PatternBindings,
        root: bool,
    ) -> PatternVerdict {
        match self {
            Self::Terminal { text, named, kind } => {
                if ast_grep_core::matcher::kind_utils::are_kinds_matching(
                    *kind,
                    tape.transducer_kind(node),
                ) && (!named || tape.transducer_text(node) == text.as_bytes())
                {
                    PatternVerdict::Matched
                } else if tape.transducer_flags(node) & EXTRA != 0
                    || tape.transducer_flags(node) & NAMED == 0
                {
                    PatternVerdict::SkipCandidate
                } else {
                    PatternVerdict::NoMatch
                }
            }
            Self::Single { name, named } => {
                if !root && tape.transducer_flags(node) & EXTRA != 0 {
                    return PatternVerdict::SkipCandidate;
                }
                if *named && tape.transducer_flags(node) & NAMED == 0 {
                    return PatternVerdict::NoMatch;
                }
                if name
                    .as_ref()
                    .is_none_or(|name| Self::bind_single(bindings, name, node, tape))
                {
                    PatternVerdict::Matched
                } else {
                    PatternVerdict::NoMatch
                }
            }
            Self::Ellipsis(name) if root => {
                if name
                    .as_ref()
                    .is_none_or(|name| Self::bind_multi(bindings, name, vec![node], tape))
                {
                    PatternVerdict::Matched
                } else {
                    PatternVerdict::NoMatch
                }
            }
            Self::Ellipsis(_) => PatternVerdict::SkipGoal,
            Self::Internal { kind, children } => {
                if !ast_grep_core::matcher::kind_utils::are_kinds_matching(
                    *kind,
                    tape.transducer_kind(node),
                ) {
                    return PatternVerdict::NoMatch;
                }
                let candidates = tape.transducer_children(node).collect::<Vec<_>>();
                if Self::children(children, &candidates, tape, bindings) {
                    PatternVerdict::Matched
                } else {
                    PatternVerdict::NoMatch
                }
            }
        }
    }

    fn children(
        goals: &[Self],
        candidates: &[u32],
        tape: &TapeDoc,
        bindings: &mut PatternBindings,
    ) -> bool {
        if candidates.is_empty() {
            return false;
        }
        let (mut goal, mut candidate) = (0_usize, 0_usize);
        while goal < goals.len() {
            if let Self::Ellipsis(name) = &goals[goal] {
                let name = name.clone();
                goal += 1;
                if goal == goals.len() {
                    return name.as_ref().is_none_or(|name| {
                        Self::bind_multi(bindings, name, candidates[candidate..].to_vec(), tape)
                    });
                }
                let mut skipped_anonymous = 0_usize;
                while goal < goals.len() && goals[goal].is_trivial() {
                    goal += 1;
                    skipped_anonymous += 1;
                }
                if goal == goals.len() {
                    let end = candidates
                        .len()
                        .saturating_sub(skipped_anonymous)
                        .max(candidate);
                    return name.as_ref().is_none_or(|name| {
                        Self::bind_multi(bindings, name, candidates[candidate..end].to_vec(), tape)
                    });
                }
                if matches!(goals[goal], Self::Ellipsis(_)) {
                    if candidate == candidates.len() {
                        return false;
                    }
                    let captured = vec![candidates[candidate]];
                    candidate += 1;
                    if candidate == candidates.len() {
                        return false;
                    }
                    if name
                        .as_ref()
                        .is_some_and(|name| !Self::bind_multi(bindings, name, captured, tape))
                    {
                        return false;
                    }
                    continue;
                }
                let start = candidate;
                loop {
                    let Some(&node) = candidates.get(candidate) else {
                        return false;
                    };
                    let mut probe = bindings.clone();
                    if goals[goal].one(node, tape, &mut probe, false) == PatternVerdict::Matched {
                        let end = candidate.saturating_sub(skipped_anonymous).max(start);
                        if name.as_ref().is_some_and(|name| {
                            !Self::bind_multi(bindings, name, candidates[start..end].to_vec(), tape)
                        }) {
                            return false;
                        }
                        break;
                    }
                    candidate += 1;
                }
            }
            loop {
                let Some(&node) = candidates.get(candidate) else {
                    return goals[goal..]
                        .iter()
                        .all(|goal| matches!(goal, Self::Ellipsis(_)));
                };
                match goals[goal].one(node, tape, bindings, false) {
                    PatternVerdict::Matched => break,
                    PatternVerdict::SkipGoal => {
                        goal += 1;
                        if goal == goals.len() {
                            return true;
                        }
                    }
                    PatternVerdict::SkipBoth => {
                        goal += 1;
                        candidate += 1;
                        if goal == goals.len() {
                            return true;
                        }
                    }
                    PatternVerdict::SkipCandidate => candidate += 1,
                    PatternVerdict::NoMatch => return false,
                }
            }
            goal += 1;
            candidate += 1;
            if goal == goals.len() {
                return true;
            }
        }
        true
    }
}

impl Program {
    fn sibling_requirement(&self, pc: u16) -> SiblingRequirement {
        match self.words.get(usize::from(pc)).map(|word| split(*word).0) {
            Some(NEXT) => SiblingRequirement::Immediate,
            Some(FIRST) => SiblingRequirement::First,
            Some(LAST) => SiblingRequirement::Last,
            _ => SiblingRequirement::None,
        }
    }

    fn query_state_identity(&self, path: &Path) -> QueryStateIdentity {
        QueryStateIdentity {
            pc: path.pc,
            node: path.node,
            cursors: path.cursors,
            active_repeats: path.active_repeats,
            sibling_requirement: self.sibling_requirement(path.pc),
            captures: path.captures,
        }
    }

    fn admit_query_state(
        &self,
        path: &Path,
        arena: &ExecutionArena,
        seen_states: &mut HashMap<QueryStateKey, CaptureAntichain>,
        work: &mut usize,
    ) -> Result<bool, Error> {
        let key = (
            path.pc,
            path.node,
            path.cursors,
            path.active_repeats,
            self.sibling_requirement(path.pc),
        );
        seen_states.entry(key).or_default().insert_unless_contained(
            arena.ordered_captures[path.captures as usize].to_vec(),
            work,
        )
    }

    pub(super) fn compile_regex(regex: &Regex) -> Result<Self, Error> {
        Self::new(
            vec![word(TEXT, 0, 0), word(CAPTURE, 0, 0), word(EMIT, 0, 1)],
            vec![
                regex.as_str().as_bytes().to_vec(),
                b"match".to_vec(),
                b"regex".to_vec(),
            ],
            vec![TextPredicate::Regex {
                source: 0,
                compiled: regex.clone(),
            }],
            vec![1, 2],
            Vec::new(),
        )
    }

    pub(super) fn is_navigation_descriptor(&self) -> bool {
        self.words == [word(STOP, 0, 0)]
            && self.strings.is_empty()
            && self.predicates.is_empty()
            && self.value_programs.is_empty()
            && self.capture_predicates.is_empty()
            && self.capture_programs.is_empty()
            && self.labels.is_empty()
            && self.longs.is_empty()
            && self.directives.is_empty()
            && self.output_program.instructions.is_empty()
            && !self.tree_sitter_query
            && self.ast_pattern.is_none()
    }

    pub(super) fn compile_navigation() -> Result<Self, Error> {
        Self::new(
            vec![word(STOP, 0, 0)],
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
    }

    pub(super) fn compile_universe() -> Result<Self, Error> {
        Self::new(
            vec![word(CAPTURE, 0, 0), word(EMIT, 0, 1)],
            vec![b"match".to_vec(), b"universe".to_vec()],
            Vec::new(),
            vec![0, 1],
            Vec::new(),
        )
    }

    pub(super) fn compile_kind(kind: u16) -> Result<Self, Error> {
        Self::new(
            vec![word(KIND, 0, kind), word(CAPTURE, 0, 0), word(EMIT, 0, 1)],
            vec![b"match".to_vec(), b"kind".to_vec()],
            Vec::new(),
            vec![0, 1],
            Vec::new(),
        )
    }

    pub(super) fn compile_ast_grep_pattern(pattern: &Pattern) -> Result<Self, Error> {
        if !matches!(pattern.strictness, MatchStrictness::Smart) {
            return Err(Error::UnsupportedPattern);
        }
        let mut program = Self::compile_navigation()?;
        program.ast_pattern = Some(PackedPattern::lower(&pattern.node));
        program.validate()?;
        Ok(program)
    }

    pub(super) fn is_ast_pattern(&self) -> bool {
        self.ast_pattern.is_some()
    }

    pub(super) fn execute_ast_pattern(
        &self,
        tape: &TapeDoc,
        root: u32,
    ) -> Result<Option<Vec<AstPatternCapture>>, Error> {
        let pattern = self.ast_pattern.as_ref().ok_or(Error::UnsupportedPattern)?;
        if root as usize >= tape.transducer_len() {
            return Err(Error::InvalidAnchor);
        }
        let mut bindings = PatternBindings::default();
        if pattern.one(root, tape, &mut bindings, true) != PatternVerdict::Matched {
            return Ok(None);
        }
        let PatternBindings { values, multi } = bindings;
        let captures = values
            .into_iter()
            .flat_map(|(name, nodes)| {
                let multi = multi.contains(&name);
                nodes
                    .into_iter()
                    .enumerate()
                    .map(move |(occurrence, node)| AstPatternCapture {
                        label: name.clone(),
                        occurrence,
                        node,
                        multi,
                    })
            })
            .collect();
        Ok(Some(captures))
    }

    pub(super) fn new(
        words: Vec<u32>,
        strings: Vec<Vec<u8>>,
        predicates: Vec<TextPredicate>,
        labels: Vec<u16>,
        longs: Vec<u64>,
    ) -> Result<Self, Error> {
        Self::new_with_capture_predicates(words, strings, predicates, Vec::new(), labels, longs)
    }

    pub(super) fn new_with_capture_predicates(
        words: Vec<u32>,
        strings: Vec<Vec<u8>>,
        predicates: Vec<TextPredicate>,
        capture_predicates: Vec<CapturePredicate>,
        labels: Vec<u16>,
        longs: Vec<u64>,
    ) -> Result<Self, Error> {
        let value_programs = predicates
            .iter()
            .map(|predicate| value::ValueProgram::lower(predicate, &strings))
            .collect::<Result<Vec<_>, _>>()?;
        let capture_programs = capture_predicates
            .iter()
            .map(value::CaptureProgram::lower)
            .collect();
        let program = Self {
            words,
            strings,
            predicates,
            value_programs,
            capture_predicates,
            capture_programs,
            labels,
            longs,
            directives: Vec::new(),
            output_program: output::OutputProgram {
                instructions: Vec::new(),
            },
            tree_sitter_query: false,
            ast_pattern: None,
        };
        program.validate()?;
        Ok(program)
    }

    pub(super) fn canonical_digest(&self, language: &str, scm_source: &[u8]) -> String {
        let mut digest = Sha256::new();
        digest.update(b"tempo-program-v7\0");
        digest.update((language.len() as u64).to_le_bytes());
        digest.update(language.as_bytes());
        digest.update((scm_source.len() as u64).to_le_bytes());
        digest.update(Sha256::digest(scm_source));
        digest.update([u8::from(self.tree_sitter_query)]);
        if let Some(pattern) = &self.ast_pattern {
            let pattern = format!("{pattern:?}");
            digest.update((pattern.len() as u64).to_le_bytes());
            digest.update(pattern.as_bytes());
        } else {
            digest.update(0_u64.to_le_bytes());
        }
        digest.update((self.words.len() as u64).to_le_bytes());
        for word in &self.words {
            digest.update(word.to_le_bytes());
        }
        digest.update((self.strings.len() as u64).to_le_bytes());
        for string in &self.strings {
            digest.update((string.len() as u64).to_le_bytes());
            digest.update(string);
        }
        digest.update((self.predicates.len() as u64).to_le_bytes());
        for predicate in &self.predicates {
            match predicate {
                TextPredicate::Equal(source) => {
                    digest.update([0, (*source & 0xff) as u8, (*source >> 8) as u8])
                }
                TextPredicate::Prefix(source) => {
                    digest.update([1, (*source & 0xff) as u8, (*source >> 8) as u8])
                }
                TextPredicate::Regex { source, .. } => {
                    digest.update([2, (*source & 0xff) as u8, (*source >> 8) as u8])
                }
                TextPredicate::LuaPattern { source, .. } => {
                    digest.update([3, (*source & 0xff) as u8, (*source >> 8) as u8])
                }
            }
        }
        digest.update((self.value_programs.len() as u64).to_le_bytes());
        for program in &self.value_programs {
            let bytes = program.canonical_bytes();
            digest.update((bytes.len() as u64).to_le_bytes());
            digest.update(bytes);
        }
        digest.update((self.capture_predicates.len() as u64).to_le_bytes());
        for predicate in &self.capture_predicates {
            match predicate {
                CapturePredicate::Equal {
                    left,
                    right,
                    negated,
                    any,
                } => {
                    digest.update([0, u8::from(*negated), u8::from(*any)]);
                    digest.update(left.to_le_bytes());
                    digest.update(right.to_le_bytes());
                }
                CapturePredicate::Text {
                    capture,
                    predicate,
                    negated,
                    any,
                } => {
                    digest.update([1, u8::from(*negated), u8::from(*any)]);
                    digest.update(capture.to_le_bytes());
                    digest.update(predicate.to_le_bytes());
                }
                CapturePredicate::AnyOf {
                    capture,
                    strings,
                    negated,
                    any,
                } => {
                    digest.update([2, u8::from(*negated), u8::from(*any)]);
                    digest.update(capture.to_le_bytes());
                    digest.update((strings.len() as u64).to_le_bytes());
                    for string in strings {
                        digest.update(string.to_le_bytes());
                    }
                }
                CapturePredicate::Kind {
                    capture,
                    kind,
                    negated,
                    any,
                } => {
                    digest.update([3, u8::from(*negated), u8::from(*any)]);
                    digest.update(capture.to_le_bytes());
                    digest.update(kind.to_le_bytes());
                }
                CapturePredicate::Kinds {
                    capture,
                    kinds,
                    negated,
                } => {
                    digest.update([4, u8::from(*negated)]);
                    digest.update(capture.to_le_bytes());
                    digest.update((kinds.len() as u64).to_le_bytes());
                    for kind in kinds {
                        digest.update(kind.to_le_bytes());
                    }
                }
                CapturePredicate::Contains {
                    capture,
                    strings,
                    negated,
                    any,
                } => {
                    digest.update([5, u8::from(*negated), u8::from(*any)]);
                    digest.update(capture.to_le_bytes());
                    digest.update((strings.len() as u64).to_le_bytes());
                    for string in strings {
                        digest.update(string.to_le_bytes());
                    }
                }
                CapturePredicate::Relation {
                    capture,
                    kinds,
                    ancestor,
                    negated,
                } => {
                    digest.update([6, u8::from(*ancestor), u8::from(*negated)]);
                    digest.update(capture.to_le_bytes());
                    digest.update((kinds.len() as u64).to_le_bytes());
                    for kind in kinds {
                        digest.update(kind.to_le_bytes());
                    }
                }
                CapturePredicate::SameLine {
                    left,
                    right,
                    negated,
                } => {
                    digest.update([7, u8::from(*negated)]);
                    digest.update(left.to_le_bytes());
                    digest.update(right.to_le_bytes());
                }
            }
        }
        digest.update((self.directives.len() as u64).to_le_bytes());
        for directive in &self.directives {
            match directive {
                Directive::Set { key, value } => {
                    digest.update([0]);
                    digest.update(key.to_le_bytes());
                    digest.update(value.unwrap_or(u16::MAX).to_le_bytes());
                }
                Directive::SetCapture {
                    capture,
                    key,
                    value,
                } => {
                    digest.update([6]);
                    digest.update(capture.to_le_bytes());
                    digest.update(key.to_le_bytes());
                    digest.update(value.unwrap_or(u16::MAX).to_le_bytes());
                }
                Directive::Offset { capture, offsets } => {
                    digest.update([1]);
                    digest.update(capture.to_le_bytes());
                    for offset in offsets {
                        digest.update(offset.to_le_bytes());
                    }
                }
                Directive::Strip {
                    capture, source, ..
                } => {
                    digest.update([2]);
                    digest.update(capture.to_le_bytes());
                    digest.update(source.to_le_bytes());
                }
                Directive::SelectAdjacent { capture, target } => {
                    digest.update([3]);
                    digest.update(capture.to_le_bytes());
                    digest.update(target.to_le_bytes());
                }
                Directive::Downcase { capture } => {
                    digest.update([4]);
                    digest.update(capture.to_le_bytes());
                }
                Directive::SetLanguageFromMimeType { capture } => {
                    digest.update([5]);
                    digest.update(capture.to_le_bytes());
                }
            }
        }
        digest.update((self.labels.len() as u64).to_le_bytes());
        for label in &self.labels {
            digest.update(label.to_le_bytes());
        }
        digest.update((self.longs.len() as u64).to_le_bytes());
        for long in &self.longs {
            digest.update(long.to_le_bytes());
        }
        format!("{:x}", digest.finalize())
    }

    pub(super) fn leading_kinds(&self) -> Option<BTreeSet<u16>> {
        let mut pending = vec![0_usize];
        let mut visited = BTreeSet::new();
        let mut kinds = BTreeSet::new();
        while let Some(pc) = pending.pop() {
            if !visited.insert(pc) {
                continue;
            }
            let &word = self.words.get(pc)?;
            let (opcode, _, operand) = split(word);
            match opcode {
                KIND => {
                    kinds.insert(operand);
                }
                FORK => {
                    pending.push(pc.checked_add(1)?);
                    pending.push(usize::from(operand));
                }
                GOTO => pending.push(usize::from(operand)),
                FIELD | FLAGS | TEXT | SIZE | ASSERT | FIRST | LAST | ABSENT_FIELD | SUPERTYPE
                | SAVE_CURSOR | CAPTURE | REPEAT_BEGIN | REPEAT_BUMP | REPEAT_END => {
                    pending.push(pc.checked_add(1)?)
                }
                CHILD | NEXT | PARENT | DESCEND | FOLLOWING | RESTORE_CURSOR | EMIT => {
                    return None;
                }
                _ => return None,
            }
        }
        (!kinds.is_empty()).then_some(kinds)
    }

    pub(super) fn label_name(&self, label: u16) -> Option<&str> {
        let string = *self.labels.get(usize::from(label))?;
        std::str::from_utf8(self.strings.get(usize::from(string))?).ok()
    }

    pub(super) fn string_value(&self, string: u16) -> Option<&str> {
        std::str::from_utf8(self.strings.get(usize::from(string))?).ok()
    }

    pub(super) fn capture_names(&self) -> impl Iterator<Item = &str> {
        self.words
            .iter()
            .filter(|word| (**word & 0xff) as u8 == CAPTURE)
            .filter_map(|word| {
                let label = usize::from((word >> 16) as u16);
                let string = usize::from(*self.labels.get(label)?);
                std::str::from_utf8(self.strings.get(string)?).ok()
            })
    }

    pub(super) fn leaves_start(&self) -> bool {
        self.words
            .iter()
            .any(|word| matches!((word & 0xff) as u8, NEXT | PARENT))
    }

    pub(super) fn with_capture_predicates(
        mut self,
        capture_predicates: Vec<CapturePredicate>,
    ) -> Result<Self, Error> {
        self.capture_programs = capture_predicates
            .iter()
            .map(value::CaptureProgram::lower)
            .collect();
        self.capture_predicates = capture_predicates;
        self.validate()?;
        Ok(self)
    }

    fn with_directives(mut self, directives: Vec<Directive>) -> Result<Self, Error> {
        self.output_program = output::OutputProgram::lower(&directives, &self.strings)?;
        self.directives = directives;
        self.validate()?;
        Ok(self)
    }

    fn with_tree_sitter_query_semantics(mut self) -> Result<Self, Error> {
        self.tree_sitter_query = true;
        self.validate()?;
        Ok(self)
    }

    fn validate(&self) -> Result<(), Error> {
        if self.words.is_empty()
            || self.words.len() > usize::from(u16::MAX) + 1
            || self.value_programs.len() != self.predicates.len()
            || self.capture_programs.len() != self.capture_predicates.len()
            || self.output_program.instructions.len() != self.directives.len()
        {
            return Err(Error::InvalidTarget(0));
        }
        if self.predicates.iter().any(|predicate| {
            let index = usize::from(predicate.string_index());
            index >= self.strings.len()
                || matches!(predicate, TextPredicate::Regex { compiled, .. }
                    if self.strings[index] != compiled.as_str().as_bytes())
        }) || self.labels.iter().any(|&label| {
            let index = usize::from(label);
            index >= self.strings.len() || std::str::from_utf8(&self.strings[index]).is_err()
        }) || self
            .capture_predicates
            .iter()
            .any(|predicate| match predicate {
                CapturePredicate::Equal { left, right, .. } => {
                    usize::from(*left) >= self.labels.len()
                        || usize::from(*right) >= self.labels.len()
                }
                CapturePredicate::Text {
                    capture, predicate, ..
                } => {
                    usize::from(*capture) >= self.labels.len()
                        || usize::from(*predicate) >= self.predicates.len()
                }
                CapturePredicate::AnyOf {
                    capture, strings, ..
                } => {
                    usize::from(*capture) >= self.labels.len()
                        || strings
                            .iter()
                            .any(|string| usize::from(*string) >= self.strings.len())
                }
                CapturePredicate::Kind { capture, .. }
                | CapturePredicate::Kinds { capture, .. }
                | CapturePredicate::Relation { capture, .. } => {
                    usize::from(*capture) >= self.labels.len()
                }
                CapturePredicate::Contains {
                    capture, strings, ..
                } => {
                    usize::from(*capture) >= self.labels.len()
                        || strings
                            .iter()
                            .any(|string| usize::from(*string) >= self.strings.len())
                }
                CapturePredicate::SameLine { left, right, .. } => {
                    usize::from(*left) >= self.labels.len()
                        || usize::from(*right) >= self.labels.len()
                }
            })
            || self.directives.iter().any(|directive| match directive {
                Directive::Set { key, value } => {
                    usize::from(*key) >= self.strings.len()
                        || value.is_some_and(|value| usize::from(value) >= self.strings.len())
                }
                Directive::SetCapture {
                    capture,
                    key,
                    value,
                } => {
                    usize::from(*capture) >= self.labels.len()
                        || usize::from(*key) >= self.strings.len()
                        || value.is_some_and(|value| usize::from(value) >= self.strings.len())
                }
                Directive::Offset { capture, .. }
                | Directive::Downcase { capture }
                | Directive::SetLanguageFromMimeType { capture } => {
                    usize::from(*capture) >= self.labels.len()
                }
                Directive::Strip {
                    capture,
                    source,
                    compiled,
                } => {
                    usize::from(*capture) >= self.labels.len()
                        || usize::from(*source) >= self.strings.len()
                        || self.strings[usize::from(*source)] != compiled.as_str().as_bytes()
                }
                Directive::SelectAdjacent { capture, target } => {
                    usize::from(*capture) >= self.labels.len()
                        || usize::from(*target) >= self.labels.len()
                }
            })
        {
            return Err(Error::InvalidOperand(0));
        }
        for (pc, &instruction) in self.words.iter().enumerate() {
            let (opcode, mode, operand) = split(instruction);
            match opcode {
                STOP | PARENT => {
                    if mode != 0 || operand != 0 {
                        return Err(Error::InvalidMode(pc));
                    }
                }
                KIND | FIELD => {
                    if mode != 0 {
                        return Err(Error::InvalidMode(pc));
                    }
                }
                FLAGS => {
                    if mode & !0x0f != 0 || operand & !0x000f != 0 || u16::from(mode) & operand != 0
                    {
                        return Err(Error::InvalidMode(pc));
                    }
                }
                TEXT => {
                    if mode != 0 || usize::from(operand) >= self.predicates.len() {
                        return Err(Error::InvalidOperand(pc));
                    }
                }
                SIZE => {
                    if !matches!(mode, 0 | 1 | 2 | 0x80 | 0x81 | 0x82)
                        || (mode & SIZE_LONG != 0 && usize::from(operand) >= self.longs.len())
                    {
                        return Err(Error::InvalidOperand(pc));
                    }
                }
                ASSERT => {
                    if mode != 0 || usize::from(operand) >= self.capture_predicates.len() {
                        return Err(Error::InvalidOperand(pc));
                    }
                }
                FIRST | LAST => {
                    if mode > NAMED || operand != 0 {
                        return Err(Error::InvalidMode(pc));
                    }
                }
                ABSENT_FIELD => {
                    if mode != 0 || operand == 0 {
                        return Err(Error::InvalidOperand(pc));
                    }
                }
                SUPERTYPE => {
                    if mode != 0 || operand == 0 {
                        return Err(Error::InvalidOperand(pc));
                    }
                }
                CHILD | NEXT | DESCEND | FOLLOWING => {
                    if mode > NAMED || operand != 0 {
                        return Err(Error::InvalidMode(pc));
                    }
                }
                SAVE_CURSOR | RESTORE_CURSOR => {
                    if mode != 0 {
                        return Err(Error::InvalidMode(pc));
                    }
                }
                CAPTURE | EMIT => {
                    if mode != 0 || usize::from(operand) >= self.labels.len() {
                        return Err(Error::InvalidOperand(pc));
                    }
                }
                REPEAT_BEGIN | REPEAT_BUMP | REPEAT_END => {
                    if mode != 0 {
                        return Err(Error::InvalidMode(pc));
                    }
                }
                FORK | GOTO => {
                    let target = usize::from(operand);
                    if (opcode == GOTO && mode != 0) || (opcode == FORK && mode > GREEDY) {
                        return Err(Error::InvalidMode(pc));
                    }
                    if target >= self.words.len() {
                        return Err(Error::InvalidTarget(pc));
                    }
                    if target <= pc {
                        let cycle = &self.words[target..=pc];
                        let sibling_advance = cycle
                            .iter()
                            .any(|word| matches!(split(*word), (NEXT | FOLLOWING, _, _)));
                        let descendant_advance = cycle
                            .iter()
                            .any(|word| matches!(split(*word), (DESCEND, _, _)));
                        let reverses = cycle
                            .iter()
                            .any(|word| matches!(split(*word), (PARENT | CHILD, _, _)));
                        if !sibling_advance && (!descendant_advance || reverses) {
                            return Err(Error::NonProgressingCycle(pc));
                        }
                    }
                }
                _ => return Err(Error::InvalidOpcode(pc)),
            }
        }
        if !matches!(
            split(*self.words.last().expect("nonempty")),
            (STOP | EMIT | GOTO, _, _)
        ) {
            return Err(Error::InvalidTarget(self.words.len() - 1));
        }
        Ok(())
    }

    fn capture_predicate_matches(
        &self,
        predicate: &CapturePredicate,
        captures: &[Capture],
        tape: &TapeDoc,
        work: &mut usize,
    ) -> Result<bool, Error> {
        let nodes = |label: u16| {
            captures
                .iter()
                .filter(move |capture| capture.label == label)
                .map(|capture| capture.node)
                .collect::<Vec<_>>()
        };
        let aggregate = |results: Vec<bool>, any: bool| {
            if results.is_empty() {
                return (false, false);
            }
            if any {
                (
                    results.iter().any(|result| *result),
                    results.iter().any(|result| !*result),
                )
            } else {
                (
                    results.iter().all(|result| *result),
                    results.iter().all(|result| !*result),
                )
            }
        };
        let (matched, negated) = match predicate {
            CapturePredicate::Equal {
                left,
                right,
                negated,
                any,
            } => {
                let left = nodes(*left);
                let right = nodes(*right);
                let results = left
                    .iter()
                    .zip(&right)
                    .map(|(&left, &right)| {
                        tape.transducer_text(left) == tape.transducer_text(right)
                    })
                    .collect();
                (aggregate(results, *any), *negated)
            }
            CapturePredicate::Text {
                capture,
                predicate,
                negated,
                any,
            } => {
                let results = nodes(*capture)
                    .into_iter()
                    .map(|node| {
                        self.value_programs[usize::from(*predicate)].is_match(
                            tape.transducer_text(node),
                            &self.strings,
                            work,
                        )
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                (aggregate(results, *any), *negated)
            }
            CapturePredicate::AnyOf {
                capture,
                strings,
                negated,
                any,
            } => {
                let results = nodes(*capture)
                    .into_iter()
                    .map(|node| {
                        let text = tape.transducer_text(node);
                        strings
                            .iter()
                            .any(|string| text == self.strings[usize::from(*string)])
                    })
                    .collect();
                (aggregate(results, *any), *negated)
            }
            CapturePredicate::Kind {
                capture,
                kind,
                negated,
                any,
            } => {
                let results = nodes(*capture)
                    .into_iter()
                    .map(|node| tape.transducer_kind(node) == *kind)
                    .collect();
                (aggregate(results, *any), *negated)
            }
            CapturePredicate::Kinds {
                capture,
                kinds,
                negated,
            } => {
                let captured = nodes(*capture);
                let positive = captured
                    .iter()
                    .all(|node| kinds.contains(&tape.transducer_kind(*node)));
                let negative = captured
                    .iter()
                    .all(|node| !kinds.contains(&tape.transducer_kind(*node)));
                ((positive, negative), *negated)
            }
            CapturePredicate::Contains {
                capture,
                strings,
                negated,
                any,
            } => {
                let captured = nodes(*capture);
                if captured.is_empty() {
                    ((true, false), *negated)
                } else {
                    let results = captured
                        .into_iter()
                        .flat_map(|node| {
                            let text = tape.transducer_text(node);
                            strings.iter().map(move |string| {
                                let needle = &self.strings[usize::from(*string)];
                                needle.is_empty()
                                    || text.windows(needle.len()).any(|window| window == needle)
                            })
                        })
                        .collect();
                    (aggregate(results, *any), *negated)
                }
            }
            CapturePredicate::Relation {
                capture,
                kinds,
                ancestor,
                negated,
            } => {
                let captured = nodes(*capture);
                let found = captured.iter().any(|node| {
                    let mut parent = tape.transducer_parent(*node);
                    while let Some(current) = parent {
                        if kinds.contains(&tape.transducer_kind(current)) {
                            return true;
                        }
                        if !ancestor {
                            break;
                        }
                        parent = tape.transducer_parent(current);
                    }
                    false
                });
                if captured.is_empty() {
                    ((true, false), *negated)
                } else {
                    ((found, !found), *negated)
                }
            }
            CapturePredicate::SameLine {
                left,
                right,
                negated,
            } => {
                let line = |node| {
                    let start = tape.transducer_span(node).start;
                    tape.transducer_source()[..start]
                        .iter()
                        .filter(|byte| **byte == b'\n')
                        .count()
                };
                let results = nodes(*left)
                    .into_iter()
                    .zip(nodes(*right))
                    .map(|(left, right)| line(left) == line(right))
                    .collect();
                (aggregate(results, false), *negated)
            }
        };
        Ok(if negated { matched.1 } else { matched.0 })
    }

    fn generic_capture_predicate_matches<D: Doc>(
        &self,
        predicate: &CapturePredicate,
        captures: &[(u16, Node<'_, D>)],
    ) -> bool {
        let nodes = |label: u16| {
            captures
                .iter()
                .filter(move |(capture, _)| *capture == label)
                .map(|(_, node)| node)
                .collect::<Vec<_>>()
        };
        let aggregate = |results: Vec<bool>, any: bool| {
            if results.is_empty() {
                return (false, false);
            }
            if any {
                (
                    results.iter().any(|result| *result),
                    results.iter().any(|result| !*result),
                )
            } else {
                (
                    results.iter().all(|result| *result),
                    results.iter().all(|result| !*result),
                )
            }
        };
        let (matched, negated) = match predicate {
            CapturePredicate::Equal {
                left,
                right,
                negated,
                any,
            } => {
                let left = nodes(*left);
                let right = nodes(*right);
                let results = left
                    .iter()
                    .zip(&right)
                    .map(|(left, right)| left.text() == right.text())
                    .collect();
                (aggregate(results, *any), *negated)
            }
            CapturePredicate::Text {
                capture,
                predicate,
                negated,
                any,
            } => {
                let results = nodes(*capture)
                    .into_iter()
                    .map(|node| {
                        self.predicates[usize::from(*predicate)]
                            .matches(node.text().as_bytes(), &self.strings)
                    })
                    .collect();
                (aggregate(results, *any), *negated)
            }
            CapturePredicate::AnyOf {
                capture,
                strings,
                negated,
                any,
            } => {
                let results = nodes(*capture)
                    .into_iter()
                    .map(|node| {
                        let text = node.text();
                        strings
                            .iter()
                            .any(|string| text.as_bytes() == self.strings[usize::from(*string)])
                    })
                    .collect();
                (aggregate(results, *any), *negated)
            }
            CapturePredicate::Kind {
                capture,
                kind,
                negated,
                any,
            } => {
                let results = nodes(*capture)
                    .into_iter()
                    .map(|node| node.kind_id() == *kind)
                    .collect();
                (aggregate(results, *any), *negated)
            }
            CapturePredicate::Kinds {
                capture,
                kinds,
                negated,
            } => {
                let captured = nodes(*capture);
                let positive = captured.iter().all(|node| kinds.contains(&node.kind_id()));
                let negative = captured.iter().all(|node| !kinds.contains(&node.kind_id()));
                ((positive, negative), *negated)
            }
            CapturePredicate::Contains {
                capture,
                strings,
                negated,
                any,
            } => {
                let captured = nodes(*capture);
                if captured.is_empty() {
                    ((true, false), *negated)
                } else {
                    let results = captured
                        .into_iter()
                        .flat_map(|node| {
                            let text = node.text();
                            strings.iter().map(move |string| {
                                let needle = &self.strings[usize::from(*string)];
                                needle.is_empty()
                                    || text
                                        .as_bytes()
                                        .windows(needle.len())
                                        .any(|window| window == needle)
                            })
                        })
                        .collect();
                    (aggregate(results, *any), *negated)
                }
            }
            CapturePredicate::Relation {
                capture,
                kinds,
                ancestor,
                negated,
            } => {
                let captured = nodes(*capture);
                let found = captured.iter().any(|node| {
                    let mut parent = node.parent();
                    while let Some(current) = parent {
                        if kinds.contains(&current.kind_id()) {
                            return true;
                        }
                        if !ancestor {
                            break;
                        }
                        parent = current.parent();
                    }
                    false
                });
                if captured.is_empty() {
                    ((true, false), *negated)
                } else {
                    ((found, !found), *negated)
                }
            }
            CapturePredicate::SameLine {
                left,
                right,
                negated,
            } => {
                let results = nodes(*left)
                    .into_iter()
                    .zip(nodes(*right))
                    .map(|(left, right)| left.start_pos().line() == right.start_pos().line())
                    .collect();
                (aggregate(results, false), *negated)
            }
        };
        if negated { matched.1 } else { matched.0 }
    }

    pub(super) fn match_node_with_env<'tree, D: Doc>(
        &self,
        start: Node<'tree, D>,
        env: &mut Cow<MetaVarEnv<'tree, D>>,
    ) -> Option<Node<'tree, D>> {
        #[derive(Clone)]
        struct Path<'tree, D: Doc> {
            pc: usize,
            node: Node<'tree, D>,
            captures: Vec<(u16, Node<'tree, D>)>,
            cursors: Vec<Option<Node<'tree, D>>>,
            active_repeats: Vec<u16>,
            steps: usize,
        }
        let mut paths = vec![Path {
            pc: 0,
            node: start.clone(),
            captures: Vec::new(),
            cursors: Vec::new(),
            active_repeats: Vec::new(),
            steps: 0,
        }];
        while let Some(mut path) = paths.pop() {
            while path.pc < self.words.len() && path.steps < MAX_STEPS_PER_PATH {
                let instruction = self.words[path.pc];
                let opcode = (instruction & 0xff) as u8;
                let mode = ((instruction >> 8) & 0xff) as u8;
                let operand = (instruction >> 16) as u16;
                path.pc += 1;
                path.steps += 1;
                let mut retain = true;
                match opcode {
                    STOP => break,
                    KIND => retain = path.node.kind_id() == operand,
                    FIELD => {
                        retain = path.node.parent().is_some_and(|parent| {
                            parent.child_by_field_id(operand).is_some_and(|field| {
                                field.range() == path.node.range()
                                    && field.kind_id() == path.node.kind_id()
                            })
                        });
                    }
                    FLAGS => {
                        let actual = u16::from(path.node.is_named())
                            | (u16::from(path.node.is_extra()) << 1)
                            | (u16::from(path.node.is_error()) << 2)
                            | (u16::from(path.node.is_missing()) << 3);
                        retain = actual & operand == u16::from(mode) & operand;
                    }
                    TEXT => {
                        retain = self.predicates[usize::from(operand)]
                            .matches(path.node.text().as_bytes(), &self.strings);
                    }
                    SIZE => {
                        let actual = path.node.range().len() as u64;
                        let expected = if mode & SIZE_LONG == 0 {
                            u64::from(operand)
                        } else {
                            self.longs[usize::from(operand)]
                        };
                        retain = match mode & !SIZE_LONG {
                            0 => actual == expected,
                            1 => actual >= expected,
                            2 => actual <= expected,
                            _ => false,
                        };
                    }
                    ASSERT => {
                        retain = self.generic_capture_predicate_matches(
                            &self.capture_predicates[usize::from(operand)],
                            &path.captures,
                        );
                    }
                    FIRST | LAST => {
                        let named = mode == NAMED;
                        let siblings = path
                            .node
                            .parent()
                            .map(|parent| {
                                parent
                                    .children()
                                    .filter(|node| !named || node.is_named())
                                    .collect::<Vec<_>>()
                            })
                            .unwrap_or_else(|| vec![path.node.clone()]);
                        retain = if opcode == FIRST {
                            siblings.first().is_some_and(|node| {
                                node.range() == path.node.range()
                                    && node.kind_id() == path.node.kind_id()
                            })
                        } else {
                            siblings.last().is_some_and(|node| {
                                node.range() == path.node.range()
                                    && node.kind_id() == path.node.kind_id()
                            })
                        };
                    }
                    ABSENT_FIELD => retain = path.node.child_by_field_id(operand).is_none(),
                    // Generic ast-grep nodes do not expose hidden-supertype cursor status. Program
                    // workspaces execute this opcode against the production tape side table.
                    SUPERTYPE => retain = false,
                    CHILD => {
                        let children = path
                            .node
                            .children()
                            .filter(|node| mode != NAMED || node.is_named())
                            .collect::<Vec<_>>();
                        if let Some((first, alternatives)) = children.split_first() {
                            for node in alternatives.iter().rev() {
                                let mut alternative = path.clone();
                                alternative.node = node.clone();
                                paths.push(alternative);
                            }
                            path.node = first.clone();
                        } else {
                            retain = false;
                        }
                    }
                    NEXT | FOLLOWING => {
                        let choices = path
                            .node
                            .next_all()
                            .filter(|node| mode != NAMED || node.is_named())
                            .collect::<Vec<_>>();
                        if let Some((first, rest)) = choices.split_first() {
                            if opcode == FOLLOWING {
                                for node in rest.iter().rev() {
                                    let mut alternative = path.clone();
                                    alternative.node = node.clone();
                                    paths.push(alternative);
                                }
                            }
                            path.node = first.clone();
                        } else {
                            retain = false;
                        }
                    }
                    PARENT => {
                        if let Some(parent) = path.node.parent() {
                            path.node = parent;
                        } else {
                            retain = false;
                        }
                    }
                    DESCEND => {
                        let descendants = path
                            .node
                            .dfs()
                            .skip(1)
                            .filter(|node| mode != NAMED || node.is_named())
                            .collect::<Vec<_>>();
                        if let Some((first, alternatives)) = descendants.split_first() {
                            for node in alternatives.iter().rev() {
                                let mut alternative = path.clone();
                                alternative.node = node.clone();
                                paths.push(alternative);
                            }
                            path.node = first.clone();
                        } else {
                            retain = false;
                        }
                    }
                    SAVE_CURSOR => {
                        let slot = usize::from(operand);
                        path.cursors.resize_with(slot + 1, || None);
                        path.cursors[slot] = Some(path.node.clone());
                    }
                    RESTORE_CURSOR => {
                        path.node = path.cursors.get(usize::from(operand))?.clone()?;
                    }
                    CAPTURE => path.captures.push((operand, path.node.clone())),
                    REPEAT_BEGIN => path.active_repeats.push(operand),
                    REPEAT_BUMP => {
                        if path.active_repeats.last() != Some(&operand) {
                            return None;
                        }
                    }
                    REPEAT_END => {
                        if path.active_repeats.pop() != Some(operand) {
                            return None;
                        }
                    }
                    FORK => {
                        let mut alternative = path.clone();
                        alternative.pc = usize::from(operand);
                        paths.push(alternative);
                    }
                    GOTO => path.pc = usize::from(operand),
                    EMIT => {
                        let mut trial = Cow::Borrowed(env.as_ref());
                        let mut grouped = BTreeMap::<u16, Vec<Node<'tree, D>>>::new();
                        for (label, node) in path.captures {
                            grouped.entry(label).or_default().push(node);
                        }
                        for (label, mut nodes) in grouped {
                            let string = self.labels[usize::from(label)];
                            let name =
                                std::str::from_utf8(&self.strings[usize::from(string)]).ok()?;
                            if nodes.len() == 1 {
                                trial.to_mut().insert(name, nodes.pop()?)?;
                            } else {
                                trial.to_mut().insert_multi(name, nodes)?;
                            }
                        }
                        if let Cow::Owned(bound) = trial {
                            *env = Cow::Owned(bound);
                        }
                        return Some(start);
                    }
                    _ => return None,
                }
                if !retain {
                    break;
                }
            }
        }
        None
    }

    fn apply_directives(
        &self,
        captures: &mut Vec<Capture>,
        source: &[u8],
    ) -> Option<Vec<DynamicProperty>> {
        fn point(source: &[u8], byte: usize) -> (i64, i64) {
            let prefix = &source[..byte];
            let row = prefix.iter().filter(|byte| **byte == b'\n').count() as i64;
            let column = prefix
                .iter()
                .rposition(|byte| *byte == b'\n')
                .map_or(byte, |newline| prefix.len() - newline - 1) as i64;
            (row, column)
        }
        fn byte_at(source: &[u8], row: i64, column: i64) -> Option<usize> {
            if row < 0 || column < 0 {
                return None;
            }
            let mut start = 0_usize;
            for _ in 0..row {
                start = source[start..].iter().position(|byte| *byte == b'\n')? + start + 1;
            }
            let end = source[start..]
                .iter()
                .position(|byte| *byte == b'\n')
                .map_or(source.len(), |newline| start + newline);
            let byte = start.checked_add(usize::try_from(column).ok()?)?;
            (byte <= end
                && std::str::from_utf8(source).is_ok_and(|source| source.is_char_boundary(byte)))
            .then_some(byte)
        }
        let mut dynamic_properties = Vec::new();
        for directive in &self.directives {
            match directive {
                Directive::Set { .. } => {}
                Directive::SetCapture {
                    capture,
                    key,
                    value,
                } => {
                    for found in captures.iter_mut().filter(|found| found.label == *capture) {
                        found.properties.push((*key, *value));
                    }
                }
                Directive::Offset { capture, offsets } => {
                    for found in captures.iter_mut().filter(|found| found.label == *capture) {
                        let (start_row, start_column) = point(source, found.span.start);
                        let (end_row, end_column) = point(source, found.span.end);
                        let start = byte_at(
                            source,
                            start_row + i64::from(offsets[0]),
                            start_column + i64::from(offsets[1]),
                        )?;
                        let end = byte_at(
                            source,
                            end_row + i64::from(offsets[2]),
                            end_column + i64::from(offsets[3]),
                        )?;
                        if start > end {
                            return None;
                        }
                        found.span = start..end;
                    }
                }
                Directive::Strip {
                    capture, compiled, ..
                } => {
                    for found in captures.iter_mut().filter(|found| found.label == *capture) {
                        let text = found
                            .text
                            .as_deref()
                            .unwrap_or_else(|| &source[found.span.clone()]);
                        let text = std::str::from_utf8(text).ok()?;
                        found.text = Some(compiled.replace_all(text, "").as_bytes().to_vec());
                    }
                }
                Directive::SelectAdjacent { capture, target } => {
                    let Some(target_start) = captures
                        .iter()
                        .find(|found| found.label == *target)
                        .map(|found| point(source, found.span.start).0)
                    else {
                        continue;
                    };
                    let selected = captures
                        .iter()
                        .enumerate()
                        .filter(|(_, found)| found.label == *capture)
                        .collect::<Vec<_>>();
                    let mut keep = selected.len();
                    let mut next_start = target_start;
                    while keep > 0 {
                        let found = selected[keep - 1].1;
                        let end_row = point(source, found.span.end).0;
                        if end_row + 1 < next_start {
                            break;
                        }
                        keep -= 1;
                        next_start = point(source, found.span.start).0;
                    }
                    let remove = selected[..keep]
                        .iter()
                        .map(|(index, _)| *index)
                        .collect::<BTreeSet<_>>();
                    let mut index = 0_usize;
                    captures.retain(|_| {
                        let retain = !remove.contains(&index);
                        index += 1;
                        retain
                    });
                }
                Directive::Downcase { capture } => {
                    for found in captures.iter_mut().filter(|found| found.label == *capture) {
                        let mut text = found
                            .text
                            .clone()
                            .unwrap_or_else(|| source[found.span.clone()].to_vec());
                        text.make_ascii_lowercase();
                        found.text = Some(text);
                    }
                }
                Directive::SetLanguageFromMimeType { capture } => {
                    let found = captures.iter().find(|found| found.label == *capture)?;
                    let text = &source[found.span.clone()];
                    let language = match text {
                        b"importmap" => b"json".as_slice(),
                        b"module" | b"application/ecmascript" | b"text/ecmascript" => {
                            b"javascript".as_slice()
                        }
                        _ => text.rsplit(|byte| *byte == b'/').next().unwrap_or(text),
                    };
                    dynamic_properties.push((b"language".to_vec(), Some(language.to_vec())));
                }
            }
        }
        Some(dynamic_properties)
    }

    fn properties(&self) -> Vec<(u16, Option<u16>)> {
        self.directives
            .iter()
            .filter_map(|directive| match directive {
                Directive::Set { key, value } => Some((*key, *value)),
                Directive::SetCapture { .. }
                | Directive::Offset { .. }
                | Directive::Strip { .. }
                | Directive::SelectAdjacent { .. }
                | Directive::Downcase { .. }
                | Directive::SetLanguageFromMimeType { .. } => None,
            })
            .collect()
    }

    pub(super) fn execute(&self, tape: &TapeDoc) -> Result<Vec<Output>, Error> {
        self.execute_roots(tape, 0..tape.transducer_len() as u32)
    }

    #[cfg(test)]
    fn execute_with_stats(&self, tape: &TapeDoc) -> Result<(Vec<Output>, ExecutionStats), Error> {
        let mut stats = ExecutionStats::default();
        let output =
            self.execute_roots_from(tape, 0..tape.transducer_len() as u32, 0, Some(&mut stats))?;
        Ok((output, stats))
    }

    pub(super) fn execute_roots(
        &self,
        tape: &TapeDoc,
        roots: impl IntoIterator<Item = u32>,
    ) -> Result<Vec<Output>, Error> {
        self.execute_roots_from(tape, roots, 0, None)
    }

    fn execute_roots_from(
        &self,
        tape: &TapeDoc,
        roots: impl IntoIterator<Item = u32>,
        initial_pc: u16,
        #[cfg_attr(not(test), allow(unused_variables, unused_mut))] mut stats: Option<
            &mut ExecutionStats,
        >,
    ) -> Result<Vec<Output>, Error> {
        let mut output = Vec::new();
        let mut pending = Vec::new();
        let mut queued_states = HashSet::new();
        let has_repeats =
            self.tree_sitter_query && self.words.iter().any(|word| split(*word).0 == REPEAT_BEGIN);
        for root in roots {
            debug_assert!(pending.is_empty());
            let mut seen_states = HashMap::<QueryStateKey, CaptureAntichain>::new();
            let mut task_work = 0_usize;
            let mut arena = ExecutionArena::default();
            pending.push(WorkItem::Path(Path {
                pc: initial_pc,
                node: root,
                start: root,
                captures: 0,
                cursors: 0,
                active_repeats: 0,
                steps: 0,
            }));
            while let Some(work) = pending.pop() {
                let (mut path, mut admitted) = match work {
                    WorkItem::Path(path) => {
                        if has_repeats {
                            queued_states.remove(&self.query_state_identity(&path));
                        }
                        (path, false)
                    }
                    WorkItem::Greedy(index) => {
                        let Some(path) = arena.take_greedy(index) else {
                            continue;
                        };
                        (path, false)
                    }
                    WorkItem::Following {
                        mut path,
                        node,
                        next_pc,
                        named,
                    } => {
                        path.node = node;
                        path.pc = next_pc;
                        if has_repeats
                            && !self.admit_query_state(
                                &path,
                                &arena,
                                &mut seen_states,
                                &mut task_work,
                            )?
                        {
                            continue;
                        }
                        if let Some(next_node) = tape.transducer_next_sibling(node, named) {
                            pending.push(WorkItem::Following {
                                path,
                                node: next_node,
                                next_pc,
                                named,
                            });
                        }
                        (path, has_repeats)
                    }
                };
                loop {
                    #[cfg(test)]
                    if let Some(stats) = stats.as_deref_mut() {
                        stats.explored_states += 1;
                        stats.max_pending_paths = stats.max_pending_paths.max(pending.len() + 1);
                        if stats.explored_states % 1_024 == 1 {
                            let current = WorkItem::Path(path).retained_bytes();
                            let retained = current
                                + pending.iter().map(WorkItem::retained_bytes).sum::<usize>()
                                + output
                                    .iter()
                                    .map(|found: &Output| {
                                        std::mem::size_of::<Output>()
                                            + found.captures.capacity()
                                                * std::mem::size_of::<Capture>()
                                    })
                                    .sum::<usize>()
                                + arena
                                    .captures
                                    .iter()
                                    .map(|captures| {
                                        captures.capacity() * std::mem::size_of::<Capture>()
                                    })
                                    .sum::<usize>()
                                + arena
                                    .cursors
                                    .iter()
                                    .map(|cursors| {
                                        cursors.capacity() * std::mem::size_of::<Option<u32>>()
                                    })
                                    .sum::<usize>()
                                + arena
                                    .repeats
                                    .iter()
                                    .map(|repeats| repeats.capacity() * std::mem::size_of::<u16>())
                                    .sum::<usize>()
                                + seen_states
                                    .values()
                                    .flat_map(|antichain| antichain.states.iter().flatten())
                                    .map(|captures| {
                                        captures.len() * std::mem::size_of::<CaptureIdentity>()
                                    })
                                    .sum::<usize>();
                            stats.peak_retained_bytes = stats.peak_retained_bytes.max(retained);
                        }
                    }
                    charge_task_work(&mut task_work)?;
                    if has_repeats
                        && !admitted
                        && !self.admit_query_state(
                            &path,
                            &arena,
                            &mut seen_states,
                            &mut task_work,
                        )?
                    {
                        break;
                    }
                    admitted = false;
                    path.steps += 1;
                    if path.steps > MAX_STEPS_PER_PATH {
                        return Err(Error::WorkLimit);
                    }
                    let (opcode, mode, operand) = split(self.words[usize::from(path.pc)]);
                    let next = path.pc.checked_add(1);
                    let mut retain = true;
                    match opcode {
                        STOP => retain = false,
                        KIND => retain = tape.transducer_kind(path.node) == operand,
                        FIELD => retain = tape.transducer_field(path.node) == operand,
                        FLAGS => {
                            let flags = tape.transducer_flags(path.node);
                            retain = flags & mode == mode && flags & operand as u8 == 0;
                        }
                        TEXT => {
                            retain = self.value_programs[usize::from(operand)].is_match(
                                tape.transducer_text(path.node),
                                &self.strings,
                                &mut path.steps,
                            )?;
                        }
                        SIZE => {
                            let actual = tape.transducer_span(path.node).len() as u64;
                            let expected = if mode & SIZE_LONG == 0 {
                                u64::from(operand)
                            } else {
                                self.longs[usize::from(operand)]
                            };
                            retain = match mode & !SIZE_LONG {
                                0 => actual == expected,
                                1 => actual >= expected,
                                2 => actual <= expected,
                                _ => unreachable!("validated size mode"),
                            };
                        }
                        ASSERT => {
                            if !self.tree_sitter_query {
                                retain = self.capture_predicate_matches(
                                    &self.capture_predicates[usize::from(operand)],
                                    &arena.captures[path.captures as usize],
                                    tape,
                                    &mut path.steps,
                                )?;
                            }
                        }
                        FIRST | LAST => {
                            let named = mode == NAMED;
                            let siblings = tape
                                .transducer_parent(path.node)
                                .map(|parent| {
                                    tape.transducer_children(parent)
                                        .filter(|&node| {
                                            !named || tape.transducer_flags(node) & 1 != 0
                                        })
                                        .collect::<Vec<_>>()
                                })
                                .unwrap_or_else(|| vec![path.node]);
                            retain = if opcode == FIRST {
                                siblings.first() == Some(&path.node)
                            } else {
                                siblings.last() == Some(&path.node)
                            };
                        }
                        ABSENT_FIELD => {
                            retain = !tape
                                .transducer_children(path.node)
                                .any(|node| tape.transducer_field(node) == operand);
                        }
                        SUPERTYPE => {
                            retain = tape.transducer_has_supertype(path.node, operand);
                        }
                        CHILD | DESCEND => {
                            let named = mode == NAMED;
                            let choices = if opcode == CHILD {
                                tape.transducer_children(path.node)
                                    .filter(|&node| !named || tape.transducer_flags(node) & 1 != 0)
                                    .collect::<Vec<_>>()
                            } else {
                                tape.transducer_descendants(path.node)
                                    .filter(|&node| !named || tape.transducer_flags(node) & 1 != 0)
                                    .collect::<Vec<_>>()
                            };
                            if let Some((&first, rest)) = choices.split_first() {
                                for &node in rest.iter().rev() {
                                    let mut alternative = path;
                                    alternative.node = node;
                                    alternative.pc =
                                        next.ok_or(Error::InvalidTarget(path.pc as usize))?;
                                    if !has_repeats
                                        || queued_states
                                            .insert(self.query_state_identity(&alternative))
                                    {
                                        pending.push(WorkItem::Path(alternative));
                                    }
                                }
                                path.node = first;
                            } else {
                                retain = false;
                            }
                        }
                        NEXT | FOLLOWING => {
                            let named = mode == NAMED;
                            if let Some(first) = tape.transducer_next_sibling(path.node, named) {
                                if opcode == FOLLOWING
                                    && let Some(second) = tape.transducer_next_sibling(first, named)
                                {
                                    pending.push(WorkItem::Following {
                                        path,
                                        node: second,
                                        next_pc: next
                                            .ok_or(Error::InvalidTarget(path.pc as usize))?,
                                        named,
                                    });
                                }
                                path.node = first;
                            } else {
                                retain = false;
                            }
                        }
                        PARENT => {
                            if let Some(node) = tape.transducer_parent(path.node) {
                                path.node = node;
                            } else {
                                retain = false;
                            }
                        }
                        SAVE_CURSOR => {
                            path.cursors = arena.save_cursor(path.cursors, operand, path.node);
                        }
                        RESTORE_CURSOR => {
                            path.node = arena.cursors[path.cursors as usize]
                                .get(usize::from(operand))
                                .copied()
                                .flatten()
                                .ok_or(Error::InvalidOperand(path.pc as usize))?;
                        }
                        CAPTURE => {
                            path.captures = arena.capture(
                                path.captures,
                                Capture {
                                    label: operand,
                                    node: path.node,
                                    span: tape.transducer_span(path.node),
                                    text: None,
                                    properties: Vec::new(),
                                },
                            );
                        }
                        REPEAT_BEGIN => {
                            path.active_repeats = arena.push_repeat(path.active_repeats, operand);
                        }
                        REPEAT_BUMP => {
                            if arena.repeats[path.active_repeats as usize].last() != Some(&operand)
                            {
                                return Err(Error::InvalidOperand(path.pc as usize));
                            }
                        }
                        REPEAT_END => {
                            path.active_repeats = arena
                                .pop_repeat(path.active_repeats, operand)
                                .ok_or(Error::InvalidOperand(path.pc as usize))?;
                        }
                        FORK => {
                            let mut alternative = path;
                            alternative.pc = operand;
                            if mode == GREEDY {
                                if let Some(index) = arena.schedule_greedy(alternative) {
                                    pending.push(WorkItem::Greedy(index));
                                }
                            } else if !has_repeats
                                || queued_states.insert(self.query_state_identity(&alternative))
                            {
                                pending.push(WorkItem::Path(alternative));
                            }
                        }
                        GOTO => {
                            path.pc = operand;
                            continue;
                        }
                        EMIT => {
                            let mut captures = arena.captures[path.captures as usize].clone();
                            captures.sort_by_key(|capture| {
                                (
                                    capture.span.start,
                                    std::cmp::Reverse(capture.span.end),
                                    capture.label,
                                )
                            });
                            let dynamic_properties = if self.tree_sitter_query {
                                Vec::new()
                            } else {
                                let Some(properties) =
                                    self.apply_directives(&mut captures, tape.transducer_source())
                                else {
                                    break;
                                };
                                properties
                            };
                            output.push(Output {
                                task_ordinal: 0,
                                local_ordinal: output.len() as u32,
                                start: path.start,
                                label: operand,
                                captures,
                                properties: self.properties(),
                                dynamic_properties,
                            });
                            retain = false;
                        }
                        _ => unreachable!("validated opcode"),
                    }
                    if !retain {
                        break;
                    }
                    path.pc = next.ok_or(Error::InvalidTarget(path.pc as usize))?;
                    if usize::from(path.pc) >= self.words.len() {
                        return Err(Error::InvalidTarget(self.words.len()));
                    }
                }
            }
        }
        self.finalize_outputs(tape, output, has_repeats)
    }

    fn finalize_outputs(
        &self,
        tape: &TapeDoc,
        output: Vec<Output>,
        has_repeats: bool,
    ) -> Result<Vec<Output>, Error> {
        if !self.tree_sitter_query {
            return Ok(output);
        }
        let mut dominated = vec![false; output.len()];
        if has_repeats {
            let mut by_start = BTreeMap::<u32, Vec<usize>>::new();
            for (index, found) in output.iter().enumerate() {
                by_start.entry(found.start).or_default().push(index);
            }
            for indices in by_start.values() {
                for (position, &left_index) in indices.iter().enumerate() {
                    if dominated[left_index] {
                        continue;
                    }
                    for &right_index in &indices[position + 1..] {
                        if dominated[right_index] {
                            continue;
                        }
                        let (left_contains_right, right_contains_left) = capture_containment(
                            &output[left_index].captures,
                            &output[right_index].captures,
                        );
                        if left_contains_right {
                            dominated[right_index] = true;
                        } else if right_contains_left {
                            dominated[left_index] = true;
                            break;
                        }
                    }
                }
            }
        }
        let mut selected = Vec::new();
        for (mut found, dominated) in output.into_iter().zip(dominated) {
            if dominated {
                continue;
            }
            let mut work = 0_usize;
            let mut eligible = true;
            for predicate in &self.capture_predicates {
                if !self.capture_predicate_matches(predicate, &found.captures, tape, &mut work)? {
                    eligible = false;
                    break;
                }
            }
            if !eligible {
                continue;
            }
            let Some(dynamic_properties) =
                self.apply_directives(&mut found.captures, tape.transducer_source())
            else {
                continue;
            };
            found.dynamic_properties = dynamic_properties;
            selected.push(found);
        }
        selected.sort_by_key(|found| {
            (
                found.start,
                found.captures.first().map(|capture| capture.node),
            )
        });
        Ok(selected)
    }
}

/// Executes several programs after one tape scan. A leading kind test becomes dispatch data;
/// every other instruction retains the ordinary transducer semantics.
pub(super) fn execute_batch(
    programs: &[Program],
    files: &[TapeDoc],
    anchors: Option<&[Vec<Vec<u32>>]>,
) -> Result<Vec<Vec<Vec<Output>>>, Error> {
    if let Some(anchors) = anchors
        && (anchors.len() != files.len() || anchors.iter().any(|file| file.len() != programs.len()))
    {
        return Err(Error::InvalidAnchor);
    }
    let Some(anchors) = anchors else {
        return files
            .iter()
            .map(|tape| execute_many(programs, tape))
            .collect();
    };
    let mut task_ordinal = 0_u32;
    let mut batch = Vec::with_capacity(files.len());
    for (file_index, tape) in files.iter().enumerate() {
        let mut file_outputs = Vec::with_capacity(programs.len());
        for (program_index, program) in programs.iter().enumerate() {
            let roots = &anchors[file_index][program_index];
            if roots
                .iter()
                .any(|&root| root as usize >= tape.transducer_len())
                || roots.windows(2).any(|pair| pair[0] >= pair[1])
            {
                return Err(Error::InvalidAnchor);
            }
            let mut program_outputs = Vec::new();
            for &root in roots {
                let mut root_outputs = program.execute_roots(tape, [root])?;
                for output in &mut root_outputs {
                    output.task_ordinal = task_ordinal;
                }
                program_outputs.append(&mut root_outputs);
                task_ordinal = task_ordinal.checked_add(1).ok_or(Error::BackendCapacity)?;
            }
            file_outputs.push(program_outputs);
        }
        batch.push(file_outputs);
    }
    Ok(batch)
}

#[derive(Debug, Eq, PartialEq)]
pub(super) struct BatchOutcome {
    pub(super) outputs: Vec<Vec<Vec<Output>>>,
    pub(super) errors: Vec<Vec<Option<Error>>>,
}

impl BatchOutcome {
    fn successful(outputs: Vec<Vec<Vec<Output>>>) -> Self {
        let errors = outputs
            .iter()
            .map(|file| file.iter().map(|_| None).collect())
            .collect();
        Self { outputs, errors }
    }

    fn into_result(self) -> Result<Vec<Vec<Vec<Output>>>, Error> {
        if let Some(error) = self.errors.into_iter().flatten().flatten().next() {
            Err(error)
        } else {
            Ok(self.outputs)
        }
    }
}

fn execute_batch_outcomes(
    programs: &[Program],
    files: &[TapeDoc],
    anchors: &[Vec<Vec<u32>>],
) -> Result<BatchOutcome, Error> {
    if anchors.len() != files.len() || anchors.iter().any(|file| file.len() != programs.len()) {
        return Err(Error::InvalidAnchor);
    }
    let mut outcome = BatchOutcome::successful(
        files
            .iter()
            .map(|_| programs.iter().map(|_| Vec::new()).collect())
            .collect(),
    );
    let mut task_ordinal = 0_u32;
    for (file_index, tape) in files.iter().enumerate() {
        for (program_index, program) in programs.iter().enumerate() {
            let roots = &anchors[file_index][program_index];
            if roots
                .iter()
                .any(|&root| root as usize >= tape.transducer_len())
                || roots.windows(2).any(|pair| pair[0] >= pair[1])
            {
                return Err(Error::InvalidAnchor);
            }
            for (root_index, &root) in roots.iter().enumerate() {
                match program.execute_roots(tape, [root]) {
                    Ok(mut found) => {
                        let ordinal = task_ordinal
                            .checked_add(root_index.try_into().map_err(|_| Error::BackendCapacity)?)
                            .ok_or(Error::BackendCapacity)?;
                        for output in &mut found {
                            output.task_ordinal = ordinal;
                        }
                        outcome.outputs[file_index][program_index].append(&mut found);
                    }
                    Err(error) => {
                        outcome.outputs[file_index][program_index].clear();
                        outcome.errors[file_index][program_index] = Some(error);
                        break;
                    }
                }
            }
            task_ordinal = task_ordinal
                .checked_add(roots.len().try_into().map_err(|_| Error::BackendCapacity)?)
                .ok_or(Error::BackendCapacity)?;
        }
    }
    Ok(outcome)
}

pub(super) fn execute_with_backend(
    backend: ExecutionBackend,
    programs: &[Program],
    files: &[TapeDoc],
    anchors: &[Vec<Vec<u32>>],
) -> Result<Vec<Vec<Vec<Output>>>, Error> {
    execute_with_backend_outcomes(backend, programs, files, anchors)?.into_result()
}

pub(super) fn execute_with_backend_packed_result_limits(
    backend: ExecutionBackend,
    programs: &[Program],
    files: &[TapeDoc],
    anchors: &[Vec<Vec<u32>>],
    result_limits: Option<&[u32]>,
) -> Result<packed_batch::PackedExecution, Error> {
    if anchors.len() != files.len() || anchors.iter().any(|file| file.len() != programs.len()) {
        return Err(Error::InvalidAnchor);
    }
    match backend {
        ExecutionBackend::CpuJit => native_cpu::execute_batch_packed_with_result_limits(
            programs,
            files,
            anchors,
            result_limits,
        ),
        #[cfg(target_os = "macos")]
        ExecutionBackend::MetalInterpreter => {
            let engine = metal_program_backend::compile_interpreter()?;
            metal_program_backend::execute_packed_with_result_limits(
                &engine,
                programs,
                files,
                anchors,
                result_limits,
            )
        }
        #[cfg(target_os = "macos")]
        ExecutionBackend::MetalJit => {
            let engine = metal_program_backend::compile_jit_cached(programs)?;
            metal_program_backend::execute_packed_with_result_limits(
                &engine,
                programs,
                files,
                anchors,
                result_limits,
            )
        }
        ExecutionBackend::CpuInterpreter => Err(Error::UnsupportedBackend),
        #[cfg(not(target_os = "macos"))]
        ExecutionBackend::MetalInterpreter | ExecutionBackend::MetalJit => {
            Err(Error::UnsupportedBackend)
        }
    }
}

fn execute_with_backend_outcomes(
    backend: ExecutionBackend,
    programs: &[Program],
    files: &[TapeDoc],
    anchors: &[Vec<Vec<u32>>],
) -> Result<BatchOutcome, Error> {
    match backend {
        ExecutionBackend::CpuInterpreter => execute_batch_outcomes(programs, files, anchors),
        ExecutionBackend::CpuJit => native_cpu::execute_batch_outcomes(programs, files, anchors),
        #[cfg(target_os = "macos")]
        ExecutionBackend::MetalInterpreter => {
            let engine = metal_program_backend::compile_interpreter()?;
            metal_program_backend::execute_outcomes(&engine, programs, files, anchors)
        }
        #[cfg(target_os = "macos")]
        ExecutionBackend::MetalJit => {
            let engine = metal_program_backend::compile_jit_cached(programs)?;
            metal_program_backend::execute_outcomes(&engine, programs, files, anchors)
        }
        #[cfg(not(target_os = "macos"))]
        ExecutionBackend::MetalInterpreter | ExecutionBackend::MetalJit => {
            Err(Error::UnsupportedBackend)
        }
    }
}

fn execute_with_backend_bounded_limit(
    backend: ExecutionBackend,
    programs: &[Program],
    files: &[TapeDoc],
    anchors: &[Vec<Vec<u32>>],
    maximum_programs: usize,
) -> Result<BatchOutcome, Error> {
    if anchors.len() != files.len() || anchors.iter().any(|file| file.len() != programs.len()) {
        return Err(Error::InvalidAnchor);
    }
    let mut outputs = (0..files.len())
        .map(|_| programs.iter().map(|_| Vec::new()).collect::<Vec<_>>())
        .collect::<Vec<_>>();
    let mut errors = (0..files.len())
        .map(|_| programs.iter().map(|_| None).collect::<Vec<_>>())
        .collect::<Vec<_>>();
    let mut global_bases = vec![vec![0_u32; programs.len()]; files.len()];
    let mut global = 0_u32;
    for (file_index, file) in anchors.iter().enumerate() {
        for (program_index, roots) in file.iter().enumerate() {
            global_bases[file_index][program_index] = global;
            global = global
                .checked_add(roots.len().try_into().map_err(|_| Error::BackendCapacity)?)
                .ok_or(Error::BackendCapacity)?;
        }
    }
    let mut start = 0_usize;
    while start < programs.len() {
        let mut low = start + 1;
        let mut high = programs.len().min(start.saturating_add(maximum_programs));
        let mut accepted = None;
        while low <= high {
            let middle = low + (high - low) / 2;
            let selected_anchors = anchors
                .iter()
                .map(|file| file[start..middle].to_vec())
                .collect::<Vec<_>>();
            match packed_batch::PackedBatch::workspace_fits(
                &programs[start..middle],
                files,
                &selected_anchors,
            ) {
                Ok(()) => {
                    accepted = Some((middle, selected_anchors));
                    low = middle + 1;
                }
                Err(Error::BackendCapacity) if middle > start + 1 => {
                    high = middle - 1;
                }
                Err(error) => return Err(error),
            }
        }
        let Some((end, selected_anchors)) = accepted else {
            let mut offsets = vec![0_usize; files.len()];
            loop {
                let remaining = anchors
                    .iter()
                    .enumerate()
                    .map(|(file_index, file)| file[start].len() - offsets[file_index])
                    .sum::<usize>();
                if remaining == 0 {
                    break;
                }
                let select = |count: usize| {
                    let mut left = count;
                    anchors
                        .iter()
                        .enumerate()
                        .map(|(file_index, file)| {
                            let available = file[start].len() - offsets[file_index];
                            let take = available.min(left);
                            left -= take;
                            vec![
                                file[start][offsets[file_index]..offsets[file_index] + take]
                                    .to_vec(),
                            ]
                        })
                        .collect::<Vec<_>>()
                };
                let mut root_low = 1_usize;
                let mut root_high = remaining;
                let mut root_accepted = None;
                while root_low <= root_high {
                    let middle = root_low + (root_high - root_low) / 2;
                    let selected_anchors = select(middle);
                    match packed_batch::PackedBatch::workspace_fits(
                        &programs[start..start + 1],
                        files,
                        &selected_anchors,
                    ) {
                        Ok(()) => {
                            root_accepted = Some((middle, selected_anchors));
                            root_low = middle + 1;
                        }
                        Err(Error::BackendCapacity) if middle > 1 => root_high = middle - 1,
                        Err(error) => return Err(error),
                    }
                }
                let (selected_count, selected_anchors) =
                    root_accepted.ok_or(Error::BackendCapacity)?;
                let mut selected = execute_with_backend_outcomes(
                    backend,
                    &programs[start..start + 1],
                    files,
                    &selected_anchors,
                )?;
                let mut selected_before = 0_u32;
                for (file_index, file_outputs) in selected.outputs.iter_mut().enumerate() {
                    let selected_roots = selected_anchors[file_index][0].len();
                    if let Some(error) = selected.errors[file_index][0].take() {
                        errors[file_index][start].get_or_insert(error);
                        outputs[file_index][start].clear();
                        offsets[file_index] = anchors[file_index][start].len();
                        selected_before = selected_before
                            .checked_add(
                                selected_roots
                                    .try_into()
                                    .map_err(|_| Error::BackendCapacity)?,
                            )
                            .ok_or(Error::BackendCapacity)?;
                        continue;
                    }
                    for output in &mut file_outputs[0] {
                        let local_root = output
                            .task_ordinal
                            .checked_sub(selected_before)
                            .ok_or(Error::BackendFailure)?;
                        output.task_ordinal = global_bases[file_index][start]
                            .checked_add(
                                u32::try_from(offsets[file_index])
                                    .map_err(|_| Error::BackendCapacity)?,
                            )
                            .and_then(|base| base.checked_add(local_root))
                            .ok_or(Error::BackendCapacity)?;
                    }
                    outputs[file_index][start].append(&mut file_outputs[0]);
                    offsets[file_index] += selected_roots;
                    selected_before = selected_before
                        .checked_add(
                            selected_roots
                                .try_into()
                                .map_err(|_| Error::BackendCapacity)?,
                        )
                        .ok_or(Error::BackendCapacity)?;
                }
                debug_assert_eq!(selected_count, selected_before as usize);
            }
            start += 1;
            continue;
        };
        let mut selected = execute_with_backend_outcomes(
            backend,
            &programs[start..end],
            files,
            &selected_anchors,
        )?;
        let mut local_bases = vec![vec![0_u32; end - start]; files.len()];
        let mut local = 0_u32;
        for (file_index, file) in selected_anchors.iter().enumerate() {
            for (program_index, roots) in file.iter().enumerate() {
                local_bases[file_index][program_index] = local;
                local = local
                    .checked_add(roots.len().try_into().map_err(|_| Error::BackendCapacity)?)
                    .ok_or(Error::BackendCapacity)?;
            }
        }
        for (file_index, file_outputs) in selected.outputs.iter_mut().enumerate() {
            for (local_program, found) in file_outputs.iter_mut().enumerate() {
                if let Some(error) = selected.errors[file_index][local_program].take() {
                    errors[file_index][start + local_program].get_or_insert(error);
                    outputs[file_index][start + local_program].clear();
                    continue;
                }
                for output in found.iter_mut() {
                    let root_offset = output
                        .task_ordinal
                        .checked_sub(local_bases[file_index][local_program])
                        .ok_or(Error::BackendFailure)?;
                    output.task_ordinal = global_bases[file_index][start + local_program]
                        .checked_add(root_offset)
                        .ok_or(Error::BackendCapacity)?;
                }
                outputs[file_index][start + local_program].append(found);
            }
        }
        start = end;
    }
    Ok(BatchOutcome { outputs, errors })
}

#[derive(Clone, Copy)]
pub(super) struct ProductionDispatchGroup {
    id: u64,
    expected_files: usize,
}

#[cfg(target_os = "macos")]
static NEXT_PRODUCTION_DISPATCH_GROUP: AtomicU64 = AtomicU64::new(1);

pub(super) fn new_production_dispatch_group(expected_files: usize) -> ProductionDispatchGroup {
    ProductionDispatchGroup {
        #[cfg(target_os = "macos")]
        id: NEXT_PRODUCTION_DISPATCH_GROUP.fetch_add(1, Ordering::Relaxed),
        #[cfg(not(target_os = "macos"))]
        id: 0,
        expected_files,
    }
}

#[cfg(target_os = "macos")]
struct ProductionDispatchRequest {
    group: ProductionDispatchGroup,
    key: metal_program_backend::ProgramBufferKey,
    programs: Vec<Program>,
    file: TapeDoc,
    anchors: Vec<Vec<u32>>,
    result_limits: Vec<u32>,
    response: mpsc::SyncSender<Result<packed_batch::PackedExecution, Error>>,
}

#[cfg(target_os = "macos")]
fn production_grid_fits(requests: &[ProductionDispatchRequest]) -> bool {
    let mut tasks = 0_u64;
    let mut nodes = 0_u64;
    let mut source_bytes = 0_u64;
    for request in requests {
        tasks = tasks.saturating_add(
            request
                .anchors
                .iter()
                .map(|roots| roots.len() as u64)
                .sum::<u64>(),
        );
        nodes = nodes.saturating_add(request.file.transducer_len() as u64);
        source_bytes = source_bytes.saturating_add(request.file.transducer_source().len() as u64);
    }
    let maximum_tasks = u64::from(u32::MAX / 2 - 1);
    tasks <= maximum_tasks && nodes <= u64::from(u32::MAX) && source_bytes <= u64::from(u32::MAX)
}

#[cfg(target_os = "macos")]
fn production_grid_partitions(requests: &[ProductionDispatchRequest]) -> Vec<Range<usize>> {
    let mut pending = Vec::with_capacity(requests.len());
    pending.push(0..requests.len());
    let mut partitions = Vec::new();
    while let Some(range) = pending.pop() {
        if range.len() <= 1 || production_grid_fits(&requests[range.clone()]) {
            partitions.push(range);
            continue;
        }
        let middle = range.start + range.len() / 2;
        pending.push(middle..range.end);
        pending.push(range.start..middle);
    }
    partitions
}

#[cfg(target_os = "macos")]
fn execute_production_grid(requests: &[ProductionDispatchRequest]) {
    for range in production_grid_partitions(requests) {
        let partition = &requests[range.clone()];
        let programs = &partition[0].programs;
        let program_count = programs.len();
        let files = partition
            .iter()
            .map(|request| request.file.clone())
            .collect::<Vec<_>>();
        let anchors = partition
            .iter()
            .map(|request| request.anchors.clone())
            .collect::<Vec<_>>();
        if std::env::var_os("TEMPO_PACKED_BATCH_PROFILE").is_some() {
            eprintln!(
                "TEMPO_PRODUCTION_GRID files={} sourceBytes={:?} tasks={:?}",
                partition.len(),
                partition
                    .iter()
                    .map(|request| request.file.transducer_source().len())
                    .collect::<Vec<_>>(),
                partition
                    .iter()
                    .map(|request| request.anchors.iter().map(Vec::len).sum::<usize>())
                    .collect::<Vec<_>>()
            );
        }
        let result = execute_with_backend_packed_result_limits(
            ExecutionBackend::MetalJit,
            programs,
            &files,
            &anchors,
            Some(&partition[0].result_limits),
        );
        match result {
            Ok(execution) => {
                for (file, request) in partition.iter().enumerate() {
                    let view = execution.view(file, 1, 0, program_count);
                    let _ = request.response.send(view);
                }
            }
            Err(error) => {
                for request in partition {
                    let _ = request.response.send(Err(error.clone()));
                }
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn production_dispatch_worker(receiver: mpsc::Receiver<ProductionDispatchRequest>) {
    const MAX_FILES_PER_DISPATCH: usize = 64;
    const INCOMPLETE_GROUP_TIMEOUT: Duration = Duration::from_millis(100);
    let mut pending = None;
    loop {
        let first = match pending.take().map(Ok).unwrap_or_else(|| receiver.recv()) {
            Ok(request) => request,
            Err(_) => return,
        };
        let expected_files = first.group.expected_files.clamp(1, MAX_FILES_PER_DISPATCH);
        let mut requests = vec![first];
        while requests.len() < expected_files {
            match receiver.recv_timeout(INCOMPLETE_GROUP_TIMEOUT) {
                Ok(request)
                    if request.group.id == requests[0].group.id
                        && request.key == requests[0].key
                        && request.result_limits == requests[0].result_limits =>
                {
                    requests.push(request);
                }
                Ok(request) => {
                    pending = Some(request);
                    break;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => break,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        execute_production_grid(&requests);
    }
}

#[cfg(target_os = "macos")]
static PRODUCTION_DISPATCH: LazyLock<mpsc::SyncSender<ProductionDispatchRequest>> =
    LazyLock::new(|| {
        let (sender, receiver) = mpsc::sync_channel(64);
        std::thread::Builder::new()
            .name("tempo-metal-dispatch".to_owned())
            .spawn(move || production_dispatch_worker(receiver))
            .expect("Tempo Metal dispatch worker must start");
        sender
    });

pub(super) fn warm_production_programs(programs: &[Program]) -> Result<(), Error> {
    #[cfg(target_os = "macos")]
    {
        metal_program_backend::compile_jit_cached(programs).map(|_| ())
    }
    #[cfg(not(target_os = "macos"))]
    {
        for program in programs {
            native_cpu::warm_jit(program)?;
        }
        Ok(())
    }
}

pub(super) fn execute_production_packed_result_limits(
    programs: &[Program],
    files: &[TapeDoc],
    anchors: &[Vec<Vec<u32>>],
    result_limits: &[u32],
    group: ProductionDispatchGroup,
) -> Result<packed_batch::PackedExecution, Error> {
    #[cfg(target_os = "macos")]
    {
        if files.len() != 1
            || anchors.len() != 1
            || anchors[0].len() != programs.len()
            || result_limits.len() != programs.len()
        {
            return Err(Error::InvalidAnchor);
        }
        let key = metal_program_backend::program_buffer_key(programs)?;
        let (response, receive) = mpsc::sync_channel(1);
        PRODUCTION_DISPATCH
            .send(ProductionDispatchRequest {
                group,
                key,
                programs: programs.to_vec(),
                file: files[0].clone(),
                anchors: anchors[0].clone(),
                result_limits: result_limits.to_vec(),
                response,
            })
            .map_err(|_| Error::BackendFailure)?;
        loop {
            match receive.try_recv() {
                Ok(result) => break result,
                Err(mpsc::TryRecvError::Disconnected) => break Err(Error::BackendFailure),
                Err(mpsc::TryRecvError::Empty) => {
                    if !matches!(rayon::yield_now(), Some(rayon::Yield::Executed)) {
                        std::thread::yield_now();
                    }
                }
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = group;
        execute_with_backend_packed_result_limits(
            ExecutionBackend::CpuJit,
            programs,
            files,
            anchors,
            Some(result_limits),
        )
    }
}

pub(super) const POSITIONAL_INDEX_VERSION: i64 = 3;
const VALUE_INDEX_LIMIT: usize = 96;

pub(super) fn execute_many(
    programs: &[Program],
    tape: &TapeDoc,
) -> Result<Vec<Vec<Output>>, Error> {
    let required = programs
        .iter()
        .filter_map(|program| match split(program.words[0]) {
            (KIND, 0, kind) => Some(kind),
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    let mut roots = required
        .iter()
        .map(|&kind| (kind, Vec::new()))
        .collect::<BTreeMap<_, _>>();
    if !roots.is_empty() {
        for node in 0..tape.transducer_len() as u32 {
            if let Some(postings) = roots.get_mut(&tape.transducer_kind(node)) {
                postings.push(node);
            }
        }
    }
    programs
        .iter()
        .map(|program| match split(program.words[0]) {
            (KIND, 0, kind) => program.execute_roots_from(
                tape,
                roots
                    .get(&kind)
                    .expect("leading kind collected")
                    .iter()
                    .copied(),
                1,
                None,
            ),
            _ => program.execute(tape),
        })
        .collect()
}
