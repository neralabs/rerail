#![allow(dead_code)]

use super::super::tape::TapeDoc;
use ast_grep_core::MatchStrictness;
use ast_grep_core::matcher::{Pattern, PatternNode};
use ast_grep_core::meta_var::{MetaVarEnv, MetaVariable};
use ast_grep_core::{Doc, Node};
use regex::Regex;
use sha2::{Digest as _, Sha256};
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt::Write as _;
use std::ops::Range;
use std::sync::Arc;
#[cfg(target_os = "macos")]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(target_os = "macos")]
use std::sync::{LazyLock, mpsc};
#[cfg(target_os = "macos")]
use std::time::Duration;

#[cfg(test)]
mod admission_spike;
/// G's literal anchors over its positional search index (O21).
#[cfg(test)]
#[allow(dead_code)]
mod literal_anchors;
#[cfg(test)]
#[allow(unused_imports)]
use literal_anchors::*;
/// G's SQL match vtab (O21).
#[cfg(test)]
#[allow(dead_code)]
mod sql_backend;
mod capture_run;
#[cfg(test)]
mod capture_run_entry_tests;
#[cfg(test)]
mod capture_run_tests;

#[cfg(target_os = "macos")]
pub(in crate::code_rule_dsl) mod metal_backend;
#[cfg(all(test, target_os = "macos"))]
pub(super) use metal_backend::launch::LaunchScope as MetalTestScope;
mod metal_batch_abi;
#[cfg(target_os = "macos")]
mod metal_parallel;
#[cfg(target_os = "macos")]
mod metal_program_backend;

#[cfg(all(test, target_os = "macos"))]
pub(in crate::code_rule_dsl) fn reset_metal_execution_metrics() {
    metal_program_backend::reset_metrics();
}

#[cfg(all(test, target_os = "macos"))]
pub(in crate::code_rule_dsl) fn metal_execution_metrics() -> (usize, usize) {
    metal_program_backend::metrics()
}
// The verifier's proofs (05 Phase 4 row 2, from Tempo's transducer, kept under `packed/verify/`
// and declared here so their `super::` is the program and its opcodes, as in Tempo).
#[cfg(target_os = "macos")]
#[path = "packed/verify/bounded_repeat_proof.rs"]
mod bounded_repeat_proof;
#[path = "packed/verify/closure_proof.rs"]
mod closure_proof;
#[path = "packed/verify/cycle_progress.rs"]
mod cycle_progress;
#[cfg(test)]
#[path = "packed/verify/cycle_progress_tests.rs"]
mod cycle_progress_tests;
#[path = "packed/verify/execution_shape.rs"]
mod execution_shape;
#[path = "packed/verify/greedy_contract.rs"]
mod greedy_contract;
#[cfg(test)]
#[path = "packed/verify/greedy_contract_tests.rs"]
mod greedy_contract_tests;
#[path = "packed/verify/terminal_repeat_proof.rs"]
mod terminal_repeat_proof;
#[cfg(test)]
#[path = "packed/verify/backend_parity_tests.rs"]
mod backend_parity_tests;
// The tree-sitter query front end (05 Phase 4 row 4, from Tempo's `scm.rs`).
#[path = "packed/compile/scm.rs"]
mod scm;
// W's composite matcher as label passes and programs (06-first-slice S1).
#[path = "packed/compile/matcher.rs"]
#[cfg_attr(not(test), allow(dead_code))]
mod compile_matcher;
pub(in crate::code_rule_dsl) use compile_matcher::{LoweredMatcher, LoweredWave};
// A label program's walk without the interpreter's machinery (O28).
#[path = "packed/compile/micro.rs"]
mod compile_micro;
pub(in crate::code_rule_dsl) use compile_micro::{Emit, Frames, Micro, steps as micro_steps};
#[cfg(test)]
#[path = "packed/compile/matcher_tests.rs"]
mod compile_matcher_tests;
mod metal_bulk_closure;
#[cfg(all(test, target_os = "macos", feature = "metal-tests"))]
mod metal_bulk_closure_tests;
#[cfg(target_os = "macos")]
mod metal_candidate_batch;
#[cfg(target_os = "macos")]
mod metal_capture_arguments;
#[cfg(target_os = "macos")]
mod metal_capture_groups;
#[cfg(target_os = "macos")]
mod metal_capture_literals;
#[cfg(target_os = "macos")]
mod metal_capture_predicates;
mod metal_child_index;
#[cfg(target_os = "macos")]
mod metal_closure_continuation;
#[cfg(target_os = "macos")]
mod metal_closure_prepare;
#[cfg(all(test, target_os = "macos", feature = "metal-tests"))]
mod metal_closure_prepare_tests;
#[cfg(target_os = "macos")]
mod metal_constant_output;
#[cfg(all(test, target_os = "macos", feature = "metal-tests"))]
#[path = "packed/metal_cutover_architecture.rs"]
mod metal_cutover_architecture;
#[cfg(target_os = "macos")]
mod metal_dynamic_execution;
#[cfg(target_os = "macos")]
mod metal_failure_records;
#[cfg(target_os = "macos")]
mod metal_frame_identity;
#[cfg(all(test, target_os = "macos", feature = "metal-tests"))]
mod metal_frame_identity_tests;
#[cfg(all(test, target_os = "macos", feature = "metal-tests"))]
mod metal_frontier_conformance;
#[cfg(target_os = "macos")]
mod metal_frontier_reduce;
#[cfg(target_os = "macos")]
mod metal_frontier_seed;
#[cfg(all(test, target_os = "macos", feature = "metal-tests"))]
mod metal_frontier_seed_tests;
#[cfg(target_os = "macos")]
mod metal_frontier_validation;
mod metal_grid;
#[cfg(target_os = "macos")]
mod metal_grid_contract;
#[cfg(target_os = "macos")]
mod metal_grid_executor;
#[cfg(target_os = "macos")]
mod metal_grid_plan;
#[cfg(target_os = "macos")]
mod metal_initial_prefix;
#[cfg(target_os = "macos")]
mod metal_initial_range_codegen;
#[cfg(target_os = "macos")]
mod metal_inline_predicates;
#[cfg(target_os = "macos")]
mod metal_named_order;
#[cfg(target_os = "macos")]
mod metal_output_contract;
#[cfg(target_os = "macos")]
mod metal_output_dispatch;
#[cfg(target_os = "macos")]
mod metal_output_mime;
#[cfg(target_os = "macos")]
mod metal_output_pages;
#[cfg(target_os = "macos")]
mod metal_output_selection;
#[cfg(target_os = "macos")]
mod metal_output_strip;
#[cfg(target_os = "macos")]
mod metal_position_order;
#[cfg(target_os = "macos")]
mod metal_grid_backend;
#[cfg(target_os = "macos")]
mod metal_ready_batch;
#[cfg(all(test, target_os = "macos", feature = "metal-tests"))]
mod metal_ready_batch_tests;
#[cfg(target_os = "macos")]
mod metal_ready_proof;
#[cfg(target_os = "macos")]
mod metal_regex_code;
#[cfg(target_os = "macos")]
mod metal_regex_frontier;
#[cfg(target_os = "macos")]
mod metal_register_codegen;
#[cfg(all(test, target_os = "macos", feature = "metal-tests"))]
mod metal_register_conformance;
#[cfg(target_os = "macos")]
mod metal_register_execution;
#[cfg(target_os = "macos")]
mod metal_semantic_columns;
#[cfg(all(test, target_os = "macos", feature = "metal-tests"))]
mod metal_semantic_conformance;
#[cfg(target_os = "macos")]
mod metal_state_values;
#[cfg(all(test, target_os = "macos", feature = "metal-tests"))]
mod metal_state_values_tests;
#[cfg(target_os = "macos")]
mod metal_successor;
#[cfg(target_os = "macos")]
mod metal_successor_branch_codegen;
#[cfg(target_os = "macos")]
mod metal_successor_codegen;
#[cfg(target_os = "macos")]
mod metal_successor_prepare;
#[cfg(target_os = "macos")]
mod metal_successor_repeat_codegen;
#[cfg(all(test, target_os = "macos", feature = "metal-tests"))]
mod metal_successor_tests;
#[cfg(target_os = "macos")]
mod metal_terminal;
#[cfg(target_os = "macos")]
mod metal_terminal_capture_order;
#[cfg(target_os = "macos")]
mod metal_terminal_repeat;
#[cfg(target_os = "macos")]
mod metal_typed_output;
#[cfg(all(test, target_os = "macos", feature = "metal-tests"))]
mod metal_unique_child_tests;
mod compilation;
mod native_cpu;
mod native_cpu_capture;
mod native_cpu_capture_pair;
mod native_cpu_codegen;
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
pub(super) use packed_batch::PackedExecution;
#[cfg(all(test, target_os = "macos", feature = "metal-tests"))]
mod metal_value_prefix_tests;
mod packed_workspace;
mod scheduler;
mod value;
mod value_packing;
mod value_prefix;

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

pub(in crate::code_rule_dsl) const NAMED: u8 = 1;
const GREEDY: u8 = 1;
const EXTRA: u8 = 2;
const MISSING: u8 = 4;
const ERROR: u8 = 8;
const SIZE_LONG: u8 = 0x80;
const MAX_FINITE_JOB_INSTRUCTIONS: usize = 256;
const MAX_STEPS_PER_PATH: usize = 1 << 20;
const MAX_WORK_PER_TASK: usize = 1 << 26;

#[derive(Clone)]
pub(in crate::code_rule_dsl) enum TextPredicate {
    Equal(u16),
    Prefix(u16),
    Regex { source: u16, compiled: Regex },
    LuaPattern { source: u16, compiled: LuaPattern },
}

#[derive(Clone)]
pub(in crate::code_rule_dsl) struct LuaPattern {
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
                        LuaRepetition::Optional => (0, available.min(1), true),
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

pub(in crate::code_rule_dsl) struct AstPatternCapture {
    pub(in crate::code_rule_dsl) label: String,
    pub(in crate::code_rule_dsl) occurrence: usize,
    pub(in crate::code_rule_dsl) node: u32,
    pub(in crate::code_rule_dsl) multi: bool,
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
pub(in crate::code_rule_dsl) struct Program {
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
pub(in crate::code_rule_dsl) enum Directive {
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
pub(in crate::code_rule_dsl) enum CapturePredicate {
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
pub(in crate::code_rule_dsl) struct Capture {
    pub(in crate::code_rule_dsl) label: u16,
    pub(in crate::code_rule_dsl) node: u32,
    pub(in crate::code_rule_dsl) span: Range<usize>,
    pub(in crate::code_rule_dsl) text: Option<Vec<u8>>,
    pub(in crate::code_rule_dsl) properties: Vec<(u16, Option<u16>)>,
}

type DynamicProperty = (Vec<u8>, Option<Vec<u8>>);

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::code_rule_dsl) struct Output {
    pub(in crate::code_rule_dsl) task_ordinal: u32,
    pub(in crate::code_rule_dsl) local_ordinal: u32,
    pub(in crate::code_rule_dsl) start: u32,
    pub(in crate::code_rule_dsl) label: u16,
    pub(in crate::code_rule_dsl) captures: Vec<Capture>,
    pub(in crate::code_rule_dsl) properties: Vec<(u16, Option<u16>)>,
    pub(in crate::code_rule_dsl) dynamic_properties: Vec<DynamicProperty>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::code_rule_dsl) enum ExecutionBackend {
    CpuInterpreter,
    CpuJit,
    /// The CPU JIT's programs once compiled in the background, the interpreter until then
    /// (`compilation`).
    CpuBackground,
    MetalInterpreter,
    MetalJit,
    /// Metal's compiled kernel once built in the background, its interpreter until then.
    MetalBackground,
    Metal,
}

#[cfg(test)]
pub(super) fn interpreter_reference(
    programs: &[Program],
    file: &TapeDoc,
    roots: &[Vec<u32>],
    limits: &[u32],
) -> Result<packed_batch::PackedExecution, Error> {
    packed_batch::PackedExecution::interpret_reference(programs, file, roots, limits)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::code_rule_dsl) enum Error {
    InvalidOpcode(usize),
    InvalidMode(usize),
    InvalidOperand(usize),
    InvalidTarget(usize),
    NonProgressingCycle(usize),
    InvalidGreedyRegion(usize),
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
    BackendLaunchLimit,
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
    sibling_edge_visits: usize,
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
// N's (5bf45ff0), which bb4c9605's merge dropped where G's version of this file replaced the
// mechanism; kept as copied (open item O21).
#[allow(dead_code)]
#[derive(Default)]
struct CaptureAntichain {
    states: Vec<Option<Vec<CaptureIdentity>>>,
    by_length: BTreeMap<usize, Vec<usize>>,
}

#[allow(dead_code)]
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
struct CaptureStateSet {
    states: HashSet<Arc<[CaptureIdentity]>>,
}

fn charge_task_work(work: &mut usize) -> Result<(), Error> {
    *work = work.checked_add(1).ok_or(Error::WorkLimit)?;
    #[cfg(test)]
    let limit = capture_run_tests::task_work_limit();
    #[cfg(not(test))]
    let limit = MAX_WORK_PER_TASK;
    if *work > limit {
        return Err(Error::WorkLimit);
    }
    Ok(())
}

impl CaptureStateSet {
    fn insert_exact(
        &mut self,
        captures: Arc<[CaptureIdentity]>,
        work: &mut usize,
    ) -> Result<bool, Error> {
        charge_task_work(work)?;
        Ok(self.states.insert(captures))
    }
}

pub(in crate::code_rule_dsl) const fn word(opcode: u8, mode: u8, operand: u16) -> u32 {
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
        seen_states: &mut HashMap<QueryStateKey, CaptureStateSet>,
        work: &mut usize,
    ) -> Result<bool, Error> {
        let key = (
            path.pc,
            path.node,
            path.cursors,
            path.active_repeats,
            self.sibling_requirement(path.pc),
        );
        seen_states.entry(key).or_default().insert_exact(
            Arc::clone(&arena.ordered_captures[path.captures as usize]),
            work,
        )
    }

    pub(in crate::code_rule_dsl) fn compile_regex(regex: &Regex) -> Result<Self, Error> {
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

    pub(in crate::code_rule_dsl) fn is_navigation_descriptor(&self) -> bool {
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

    pub(in crate::code_rule_dsl) fn compile_navigation() -> Result<Self, Error> {
        Self::new(
            vec![word(STOP, 0, 0)],
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
    }

    pub(in crate::code_rule_dsl) fn compile_universe() -> Result<Self, Error> {
        Self::new(
            vec![word(CAPTURE, 0, 0), word(EMIT, 0, 1)],
            vec![b"match".to_vec(), b"universe".to_vec()],
            Vec::new(),
            vec![0, 1],
            Vec::new(),
        )
    }

    pub(in crate::code_rule_dsl) fn compile_kind(kind: u16) -> Result<Self, Error> {
        Self::compile_kind_operation(KIND, kind)
    }

    pub(in crate::code_rule_dsl) fn compile_supertype(kind: u16) -> Result<Self, Error> {
        Self::compile_kind_operation(SUPERTYPE, kind)
    }

    fn compile_kind_operation(operation: u8, kind: u16) -> Result<Self, Error> {
        Self::new(
            vec![
                word(operation, 0, kind),
                word(CAPTURE, 0, 0),
                word(EMIT, 0, 1),
            ],
            vec![b"match".to_vec(), b"kind".to_vec()],
            Vec::new(),
            vec![0, 1],
            Vec::new(),
        )
    }

    pub(super) fn compile_ast_grep_literal(pattern: &Pattern) -> Result<Self, Error> {
        if !matches!(pattern.strictness, MatchStrictness::Smart) {
            return Err(Error::UnsupportedPattern);
        }
        let PatternNode::Terminal {
            text,
            is_named,
            kind_id,
        } = &pattern.node
        else {
            return Err(Error::UnsupportedPattern);
        };
        let mut words = vec![word(KIND, 0, *kind_id)];
        let predicates = if *is_named {
            words.push(word(TEXT, 0, 0));
            vec![TextPredicate::Equal(0)]
        } else {
            Vec::new()
        };
        words.extend([word(CAPTURE, 0, 0), word(EMIT, 0, 1)]);
        Self::new(
            words,
            vec![
                text.as_bytes().to_vec(),
                b"match".to_vec(),
                b"ast-grep-literal".to_vec(),
            ],
            predicates,
            vec![1, 2],
            Vec::new(),
        )
    }

    pub(in crate::code_rule_dsl) fn compile_ast_grep_pattern(pattern: &Pattern) -> Result<Self, Error> {
        if !matches!(pattern.strictness, MatchStrictness::Smart) {
            return Err(Error::UnsupportedPattern);
        }
        let mut program = Self::compile_navigation()?;
        program.ast_pattern = Some(PackedPattern::lower(&pattern.node));
        program.validate()?;
        Ok(program)
    }

    pub(in crate::code_rule_dsl) fn is_ast_pattern(&self) -> bool {
        self.ast_pattern.is_some()
    }

    pub(in crate::code_rule_dsl) fn execute_ast_pattern(
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

    pub(in crate::code_rule_dsl) fn new(
        words: Vec<u32>,
        strings: Vec<Vec<u8>>,
        predicates: Vec<TextPredicate>,
        labels: Vec<u16>,
        longs: Vec<u64>,
    ) -> Result<Self, Error> {
        Self::new_with_capture_predicates(words, strings, predicates, Vec::new(), labels, longs)
    }

    pub(in crate::code_rule_dsl) fn new_with_capture_predicates(
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

    pub(in crate::code_rule_dsl) fn canonical_digest(&self, language: &str, scm_source: &[u8]) -> String {
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

    pub(in crate::code_rule_dsl) fn leading_kinds(&self) -> Option<BTreeSet<u16>> {
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

    pub(in crate::code_rule_dsl) fn label_name(&self, label: u16) -> Option<&str> {
        let string = *self.labels.get(usize::from(label))?;
        std::str::from_utf8(self.strings.get(usize::from(string))?).ok()
    }

    pub(in crate::code_rule_dsl) fn string_value(&self, string: u16) -> Option<&str> {
        std::str::from_utf8(self.strings.get(usize::from(string))?).ok()
    }

    pub(in crate::code_rule_dsl) fn capture_names(&self) -> impl Iterator<Item = &str> {
        self.words
            .iter()
            .filter(|word| (**word & 0xff) as u8 == CAPTURE)
            .filter_map(|word| {
                let label = usize::from((word >> 16) as u16);
                let string = usize::from(*self.labels.get(label)?);
                std::str::from_utf8(self.strings.get(string)?).ok()
            })
    }

    pub(in crate::code_rule_dsl) fn leaves_start(&self) -> bool {
        self.words
            .iter()
            .any(|word| matches!((word & 0xff) as u8, NEXT | PARENT))
    }

    pub(in crate::code_rule_dsl) fn with_capture_predicates(
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

    pub(in crate::code_rule_dsl) fn tree_sitter_top_level_patterns(source: &str) -> Result<Vec<String>, Error> {
        scm::parse(source).map(|patterns| {
            patterns
                .into_iter()
                .map(|pattern| pattern.source().to_owned())
                .collect()
        })
    }

    /// Parses one complete Tree-sitter query pattern and lowers its typed syntax tree to Program.
    pub(in crate::code_rule_dsl) fn compile_tree_sitter_query_pattern(
        source: &str,
        grammar: &tree_sitter::Language,
    ) -> Result<Self, Error> {
        let mut patterns = scm::parse(source)?;
        if patterns.len() != 1 {
            return Err(Error::UnsupportedPattern);
        }
        scm::compile(&patterns.remove(0), grammar)
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
        cycle_progress::validate(&self.words)?;
        if self.tree_sitter_query {
            greedy_contract::validate(&self.words)?;
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

    pub(in crate::code_rule_dsl) fn match_node_with_env<'tree, D: Doc>(
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
    ) -> Result<Option<Vec<DynamicProperty>>, Error> {
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
        let mut transform_work = 0usize;
        for (index, directive) in self.directives.iter().enumerate() {
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
                        let Some(start) = byte_at(
                            source,
                            start_row + i64::from(offsets[0]),
                            start_column + i64::from(offsets[1]),
                        ) else {
                            return Ok(None);
                        };
                        let Some(end) = byte_at(
                            source,
                            end_row + i64::from(offsets[2]),
                            end_column + i64::from(offsets[3]),
                        ) else {
                            return Ok(None);
                        };
                        if start > end {
                            return Ok(None);
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
                        let text = std::str::from_utf8(text).map_err(|_| Error::BackendFailure)?;
                        let Some(output::OutputInstruction::Strip { regex, .. }) =
                            self.output_program.instructions.get(index)
                        else {
                            return Err(Error::BackendFailure);
                        };
                        let mut search = 0;
                        loop {
                            let remaining = MAX_STEPS_PER_PATH
                                .checked_sub(transform_work)
                                .ok_or(Error::WorkLimit)?;
                            let (result, used) =
                                regex.find_with_budget_from(text.as_bytes(), search, remaining);
                            transform_work =
                                transform_work.checked_add(used).ok_or(Error::WorkLimit)?;
                            let Some([start, end]) = result? else { break };
                            search = end as usize;
                            if start == end {
                                if search == text.len() {
                                    break;
                                }
                                search += text[search..]
                                    .chars()
                                    .next()
                                    .ok_or(Error::BackendFailure)?
                                    .len_utf8();
                            }
                        }
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
                    let Some(found) = captures.iter().find(|found| found.label == *capture) else {
                        return Ok(None);
                    };
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
        Ok(Some(dynamic_properties))
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

    pub(in crate::code_rule_dsl) fn execute(&self, tape: &TapeDoc) -> Result<Vec<Output>, Error> {
        self.execute_roots(tape, 0..tape.transducer_len() as u32)
    }

    #[cfg(test)]
    fn execute_with_stats(&self, tape: &TapeDoc) -> Result<(Vec<Output>, ExecutionStats), Error> {
        let mut stats = ExecutionStats::default();
        let output =
            self.execute_roots_from(tape, 0..tape.transducer_len() as u32, 0, Some(&mut stats))?;
        Ok((output, stats))
    }

    pub(in crate::code_rule_dsl) fn execute_roots(
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
        stats: Option<&mut ExecutionStats>,
    ) -> Result<Vec<Output>, Error> {
        self.execute_roots_from_with_work_limit(tape, roots, initial_pc, stats, MAX_STEPS_PER_PATH)
    }

    #[cfg(all(test, target_os = "macos"))]
    fn execute_terminal_reference_with_work_limit(
        &self,
        tape: &TapeDoc,
        roots: impl IntoIterator<Item = u32>,
        work_limit: usize,
    ) -> Result<Vec<Output>, Error> {
        if work_limit > MAX_STEPS_PER_PATH || terminal_repeat_proof::prove(self).is_none() {
            return Err(Error::InvalidAnchor);
        }
        self.execute_roots_from_with_work_limit(tape, roots, 0, None, work_limit)
    }

    fn execute_roots_from_with_work_limit(
        &self,
        tape: &TapeDoc,
        roots: impl IntoIterator<Item = u32>,
        initial_pc: u16,
        #[cfg_attr(not(test), allow(unused_variables, unused_mut))] mut stats: Option<
            &mut ExecutionStats,
        >,
        work_limit: usize,
    ) -> Result<Vec<Output>, Error> {
        let mut output = Vec::new();
        let mut pending = Vec::new();
        let mut queued_states = HashSet::new();
        let has_repeats =
            self.tree_sitter_query && self.words.iter().any(|word| split(*word).0 == REPEAT_BEGIN);
        // This immutable navigation cache belongs to this one tape. Named and
        // all-child edges have distinct keys; semantic admission stays separate.
        let mut sibling_edges = HashMap::<(u32, bool), [Option<u32>; 2]>::new();
        for root in roots {
            debug_assert!(pending.is_empty());
            let mut seen_states = HashMap::<QueryStateKey, CaptureStateSet>::new();
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
                                    .flat_map(|states| states.states.iter())
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
                    if path.steps > work_limit {
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
                            let edge = tape
                                .transducer_parent(path.node)
                                .map(|parent| {
                                    let edges =
                                        sibling_edges.entry((parent, named)).or_insert_with(|| {
                                            let mut children = tape
                                                .transducer_children(parent)
                                                .inspect(|_| {
                                                    #[cfg(test)]
                                                    if let Some(stats) = stats.as_deref_mut() {
                                                        stats.sibling_edge_visits += 1;
                                                    }
                                                })
                                                .filter(|&node| {
                                                    !named || tape.transducer_flags(node) & 1 != 0
                                                });
                                            let first = children.next();
                                            [first, children.last().or(first)]
                                        });
                                    edges[usize::from(opcode == LAST)]
                                })
                                .unwrap_or(Some(path.node));
                            retain = edge == Some(path.node);
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
                                    if self.tree_sitter_query {
                                        0
                                    } else {
                                        capture.label
                                    },
                                )
                            });
                            let dynamic_properties = if self.tree_sitter_query {
                                Vec::new()
                            } else {
                                let Some(properties) =
                                    self.apply_directives(&mut captures, tape.transducer_source())?
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
                self.apply_directives(&mut found.captures, tape.transducer_source())?
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
pub(in crate::code_rule_dsl) fn execute_batch(
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
pub(in crate::code_rule_dsl) struct BatchOutcome {
    pub(in crate::code_rule_dsl) outputs: Vec<Vec<Vec<Output>>>,
    pub(in crate::code_rule_dsl) errors: Vec<Vec<Option<Error>>>,
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

pub(in crate::code_rule_dsl) fn execute_with_backend(
    backend: ExecutionBackend,
    programs: &[Program],
    files: &[TapeDoc],
    anchors: &[Vec<Vec<u32>>],
) -> Result<Vec<Vec<Vec<Output>>>, Error> {
    execute_with_backend_outcomes(backend, programs, files, anchors)?.into_result()
}

pub(in crate::code_rule_dsl) fn packed_workspace_fits(
    programs: &[Program],
    files: &[TapeDoc],
    anchors: &[Vec<Vec<u32>>],
    result_limits: &[u32],
) -> Result<(), Error> {
    let mut batch = packed_batch::PackedBatch::new(programs, files, anchors)?;
    batch
        .assign_workspace_with_result_limits(programs, Some(result_limits))
        .map(|_| ())
}

/// The bytes a packed workspace for `programs` on `files` takes (`packed_workspace_fits`'s
/// batch, sized): what an execution of them holds while it runs.
pub(in crate::code_rule_dsl) fn packed_workspace_bytes(
    programs: &[Program],
    files: &[TapeDoc],
    anchors: &[Vec<Vec<u32>>],
    result_limits: &[u32],
) -> Result<u64, Error> {
    let mut batch = packed_batch::PackedBatch::new(programs, files, anchors)?;
    let totals = batch.assign_workspace_with_result_limits(programs, Some(result_limits))?;
    batch.workspace_allocation_bytes(totals).ok_or(Error::BackendCapacity)
}

pub(in crate::code_rule_dsl) fn execute_with_backend_packed_result_limits(
    backend: ExecutionBackend,
    programs: &[Program],
    files: &[TapeDoc],
    anchors: &[Vec<Vec<u32>>],
    result_limits: Option<&[u32]>,
) -> Result<packed_batch::PackedExecution, Error> {
    if anchors.len() != files.len() || anchors.iter().any(|file| file.len() != programs.len()) {
        return Err(Error::InvalidAnchor);
    }
    match compilation::resolve(backend, programs)? {
        ExecutionBackend::CpuInterpreter => {
            let limits = result_limits.map_or_else(|| vec![u32::MAX; programs.len()], <[u32]>::to_vec);
            packed_batch::PackedExecution::interpret(programs, files, anchors, &limits)
        }
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
            let engine = crate::code_rule_dsl::prof::time("roots.jit", || metal_program_backend::compile_jit_cached(programs))?;
            let _run = crate::code_rule_dsl::prof::Stage::start("roots.metal");
            metal_program_backend::execute_packed_with_result_limits(
                &engine,
                programs,
                files,
                anchors,
                result_limits,
            )
        }
        ExecutionBackend::Metal | ExecutionBackend::CpuBackground | ExecutionBackend::MetalBackground => {
            Err(Error::UnsupportedBackend)
        }
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
    match compilation::resolve(backend, programs)? {
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
        #[cfg(target_os = "macos")]
        ExecutionBackend::Metal => {
            metal_grid_backend::execute_outcomes(programs, files, anchors)
        }
        #[cfg(not(target_os = "macos"))]
        ExecutionBackend::MetalInterpreter | ExecutionBackend::MetalJit | ExecutionBackend::Metal => {
            Err(Error::UnsupportedBackend)
        }
        ExecutionBackend::CpuBackground | ExecutionBackend::MetalBackground => Err(Error::UnsupportedBackend),
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
pub(in crate::code_rule_dsl) struct ProductionDispatchGroup {
    id: u64,
    expected_files: usize,
}

#[cfg(target_os = "macos")]
static NEXT_PRODUCTION_DISPATCH_GROUP: AtomicU64 = AtomicU64::new(1);

pub(in crate::code_rule_dsl) fn new_production_dispatch_group(expected_files: usize) -> ProductionDispatchGroup {
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

pub(in crate::code_rule_dsl) fn warm_production_programs(programs: &[Program]) -> Result<(), Error> {
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

pub(in crate::code_rule_dsl) fn execute_production_packed_result_limits(
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

pub(super) fn execute_production_corpus(
    programs: &[Program],
    files: &[TapeDoc],
    anchors: &[Vec<Arc<[u32]>>],
    result_limits: &[u32],
) -> Result<packed_batch::PackedExecution, Error> {
    #[cfg(target_os = "macos")]
    {
        metal_grid_backend::execute_shared_corpus(programs, files, anchors, result_limits)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let anchors = anchors
            .iter()
            .map(|file| file.iter().map(|roots| roots.to_vec()).collect())
            .collect::<Vec<_>>();
        execute_with_backend_packed_result_limits(
            ExecutionBackend::CpuJit,
            programs,
            files,
            &anchors,
            Some(result_limits),
        )
    }
}

pub(super) fn execute_production_corpus_pages(
    programs: &[Program],
    files: &[crate::code_rule_dsl::tape::TapeSource],
    anchors: &[Vec<Arc<[u32]>>],
    result_limits: &[u32],
    consumer: &mut dyn FnMut(PackedExecution, usize) -> Result<(), Error>,
) -> Result<(), Error> {
    #[cfg(target_os = "macos")]
    {
        metal_grid_backend::execute_shared_corpus_pages(
            programs,
            files,
            anchors,
            result_limits,
            consumer,
        )
    }
    #[cfg(not(target_os = "macos"))]
    {
        let docs = files
            .iter()
            .map(|file| file.restore(&|| false).map_err(|_| Error::BackendFailure))
            .collect::<Result<Vec<_>, _>>()?;
        consumer(
            execute_production_corpus(programs, &docs, anchors, result_limits)?,
            files.len(),
        )
    }
}

pub(in crate::code_rule_dsl) const POSITIONAL_INDEX_VERSION: i64 = 3;
const VALUE_INDEX_LIMIT: usize = 96;

pub(in crate::code_rule_dsl) fn execute_many(
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

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn metal_backend_rejects_execution_without_platform_support() {
        assert!(matches!(
            execute_with_backend_packed_result_limits(ExecutionBackend::Metal, &[], &[], &[], None),
            Err(Error::UnsupportedBackend)
        ));
    }
    use crate::code_rule_dsl::tape::{Grammar, TapeImage};
    use ast_grep_core::AstGrep;
    use ast_grep_core::matcher::Pattern;
    use ast_grep_core::tree_sitter::StrDoc;
    use crate::code_rule_dsl::language::GrammarDialect as SupportLang;
    use ast_grep_core::tree_sitter::LanguageExt;
    use serde_json::json;
    use sha2::{Digest, Sha256};
    use std::fs;
    use std::hint::black_box;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    fn tape(source: &str) -> TapeDoc {
        let grammar = Grammar::new(SupportLang::Python).unwrap();
        let native = StrDoc::new(source, SupportLang::Python);
        let image =
            TapeImage::encode(native.tree.root_node(), &grammar, source, &|| false).unwrap();
        TapeDoc::restore(Arc::new(source.to_owned()), grammar, &image, &|| false).unwrap()
    }

    fn identifier_program(text: &str) -> Program {
        let pattern = Pattern::try_new(text, SupportLang::Python).unwrap();
        Program::compile_ast_grep_literal(&pattern).unwrap()
    }

    #[test]
    fn sibling_edges_preserve_complete_outputs_with_linear_source_traversal() {
        let source = format!(
            "f({})\n",
            (0..128)
                .map(|i| format!("v{i}"))
                .collect::<Vec<_>>()
                .join(",")
        );
        let file = tape(&source);
        let arguments = SupportLang::Python
            .get_ts_language()
            .id_for_node_kind("argument_list", true);
        let root = (0..file.transducer_len() as u32)
            .find(|&node| file.transducer_kind(node) == arguments)
            .unwrap();
        let children = file.transducer_children(root).count();
        let expected = |span: std::ops::Range<usize>, start| {
            let node = (0..file.transducer_len() as u32)
                .find(|&node| file.transducer_span(node) == span)
                .unwrap();
            vec![Output {
                task_ordinal: 0,
                local_ordinal: 0,
                start,
                label: 0,
                captures: vec![Capture {
                    label: 0,
                    node,
                    span,
                    text: None,
                    properties: vec![],
                }],
                properties: vec![],
                dynamic_properties: vec![],
            }]
        };
        for edge in [FIRST, LAST] {
            for mode in [0, NAMED] {
                let program = Program::new(
                    vec![
                        word(CHILD, 0, 0),
                        word(edge, mode, 0),
                        word(CAPTURE, 0, 0),
                        word(EMIT, 0, 0),
                    ],
                    vec![b"edge".to_vec()],
                    vec![],
                    vec![0],
                    vec![],
                )
                .unwrap();
                let span = match (edge, mode) {
                    (FIRST, 0) => 1..2,
                    (FIRST, NAMED) => 2..4,
                    (LAST, 0) => source.len() - 2..source.len() - 1,
                    (LAST, NAMED) => source.len() - 6..source.len() - 2,
                    _ => unreachable!(),
                };
                let mut stats = ExecutionStats::default();
                let actual = program
                    .execute_roots_from(&file, [root], 0, Some(&mut stats))
                    .unwrap();
                assert_eq!(actual, expected(span, root));
                assert!(
                    stats.sibling_edge_visits <= 2 * children + 2,
                    "edge lookup must stay linear in the immutable child domain: {stats:?}"
                );
            }
        }
        // Named and all-sibling queries share a parent but have different edges.
        let mixed = Program::new(
            vec![
                word(CHILD, 0, 0),
                word(FORK, 0, 6),
                word(FIRST, NAMED, 0),
                word(LAST, 0, 0),
                word(CAPTURE, 0, 0),
                word(EMIT, 0, 0),
                word(FIRST, 0, 0),
                word(CAPTURE, 0, 0),
                word(EMIT, 0, 0),
            ],
            vec![b"edge".to_vec()],
            vec![],
            vec![0],
            vec![],
        )
        .unwrap();
        assert_eq!(
            mixed.execute_roots(&file, [root]).unwrap(),
            expected(1..2, root)
        );
        for edge in [FIRST, LAST] {
            let program = Program::new(
                vec![word(edge, NAMED, 0), word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
                vec![b"edge".to_vec()],
                vec![],
                vec![0],
                vec![],
            )
            .unwrap();
            assert_eq!(
                program.execute_roots(&file, [0]).unwrap(),
                expected(0..source.len(), 0)
            );
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn production_corpus_executes_all_files_in_one_packed_grid() {
        const FILE_COUNT: usize = 129;
        let kind = tape("value = other\n").transducer_kind(1);
        let programs = [Program::new(
            vec![word(KIND, 0, kind), word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
            vec![b"match".to_vec()],
            vec![],
            vec![0],
            vec![],
        )
        .unwrap()];
        let document = tape("value = other\n");
        let roots = (0..document.transducer_len() as u32).collect::<Vec<_>>();
        let files = vec![document; FILE_COUNT];
        let anchors = vec![vec![Arc::<[u32]>::from(roots)]; FILE_COUNT];
        let execution = execute_production_corpus(&programs, &files, &anchors, &[32]).unwrap();
        assert_eq!(execution.physical_file_count(), FILE_COUNT);
        assert_eq!(execution.file_count(), FILE_COUNT);
        let oracle_anchors = anchors
            .iter()
            .map(|file| file.iter().map(|roots| roots.to_vec()).collect())
            .collect::<Vec<_>>();
        let expected = execute_batch(&programs, &files, Some(&oracle_anchors)).unwrap();
        assert_eq!(execution.into_outcome().unwrap().outputs, expected);
    }

    fn assert_jit_parity(programs: &[Program], tapes: &[TapeDoc]) {
        let anchors = tapes
            .iter()
            .map(|tape| {
                programs
                    .iter()
                    .map(|_| (0..tape.transducer_len() as u32).collect::<Vec<_>>())
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let expected = execute_batch(programs, tapes, Some(&anchors)).unwrap();
        for backend in [ExecutionBackend::CpuInterpreter, ExecutionBackend::CpuJit] {
            assert_eq!(
                execute_with_backend(backend, programs, tapes, &anchors).unwrap(),
                expected
            );
        }
        #[cfg(target_os = "macos")]
        {
            let backend = ExecutionBackend::Metal;
            let actual = execute_with_backend(backend, programs, tapes, &anchors).unwrap();
            if actual != expected {
                let mismatch = actual
                    .iter()
                    .zip(&expected)
                    .position(|(actual, expected)| actual != expected)
                    .unwrap_or(actual.len().min(expected.len()));
                let output_mismatch = actual.get(mismatch).zip(expected.get(mismatch)).and_then(
                    |(actual, expected)| {
                        actual
                            .iter()
                            .zip(expected)
                            .position(|(actual, expected)| actual != expected)
                    },
                );
                panic!(
                    "backend {backend:?} diverged at evaluation {mismatch}, output {output_mismatch:?}: actual count {}, expected count {}, actual output {:?}, expected output {:?}",
                    actual.get(mismatch).map_or(0, Vec::len),
                    expected.get(mismatch).map_or(0, Vec::len),
                    output_mismatch.and_then(|index| actual.get(mismatch)?.get(index)),
                    output_mismatch.and_then(|index| expected.get(mismatch)?.get(index)),
                );
            }
        }
    }

    #[cfg(target_os = "macos")]
    fn assert_metal_parity(programs: &[Program], tapes: &[TapeDoc]) {
        let anchors = tapes
            .iter()
            .map(|tape| {
                programs
                    .iter()
                    .map(|_| (0..tape.transducer_len() as u32).collect::<Vec<_>>())
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let expected = execute_batch(programs, tapes, Some(&anchors)).unwrap();
        {
            assert_eq!(
                metal_grid_backend::execute(programs, tapes, &anchors).unwrap(),
                expected
            );
        }
    }

    fn assert_scm_native_parity(query_source: &str, source: &str) {
        use tree_sitter::StreamingIterator;

        let language = SupportLang::Python.get_ts_language();
        let program = Program::compile_tree_sitter_query_pattern(query_source, &language).unwrap();
        let tape = tape(source);
        let outputs = program.execute(&tape).unwrap();
        let actual = outputs
            .iter()
            .map(|output| {
                output
                    .captures
                    .iter()
                    .map(|capture| {
                        let string = program.labels[usize::from(capture.label)];
                        (
                            std::str::from_utf8(&program.strings[usize::from(string)])
                                .unwrap()
                                .to_owned(),
                            capture.span.start,
                            capture.span.end,
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();

        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&language).unwrap();
        let tree = parser.parse(source, None).unwrap();
        let query = tree_sitter::Query::new(&language, query_source).unwrap();
        let mut cursor = tree_sitter::QueryCursor::new();
        let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());
        let mut expected = Vec::new();
        while let Some(found) = matches.next() {
            expected.push(
                found
                    .captures()
                    .iter()
                    .map(|capture| {
                        (
                            query.capture_names()[capture.index as usize].to_owned(),
                            capture.node.start_byte(),
                            capture.node.end_byte(),
                        )
                    })
                    .collect::<Vec<_>>(),
            );
        }
        assert_eq!(actual, expected, "query: {query_source}\nsource:\n{source}");

        // QueryMatch does not expose its structural root. Add one oracle-only capture and compare
        // root spans as an unordered relation, while the original query above remains the ordering
        // oracle. One common root capture does not change capture-containment relationships.
        const MATCH_CAPTURE: &str = "__tempo_match_span";
        let span_query_source = format!("{query_source} @{MATCH_CAPTURE}");
        let span_query = tree_sitter::Query::new(&language, &span_query_source).unwrap();
        let match_capture = span_query
            .capture_names()
            .iter()
            .position(|name| *name == MATCH_CAPTURE)
            .unwrap() as u32;
        let mut span_cursor = tree_sitter::QueryCursor::new();
        let mut span_matches =
            span_cursor.matches(&span_query, tree.root_node(), source.as_bytes());
        let mut expected_spans = Vec::new();
        while let Some(found) = span_matches.next() {
            let root = found
                .captures()
                .iter()
                .find(|capture| capture.index == match_capture)
                .unwrap()
                .node
                .byte_range();
            let captures = found
                .captures()
                .iter()
                .filter(|capture| capture.index != match_capture)
                .map(|capture| {
                    (
                        span_query.capture_names()[capture.index as usize].to_owned(),
                        capture.node.start_byte(),
                        capture.node.end_byte(),
                    )
                })
                .collect::<Vec<_>>();
            expected_spans.push((captures, root.start, root.end));
        }
        let mut actual_spans = outputs
            .iter()
            .map(|output| {
                (
                    output
                        .captures
                        .iter()
                        .map(|capture| {
                            let string = program.labels[usize::from(capture.label)];
                            (
                                std::str::from_utf8(&program.strings[usize::from(string)])
                                    .unwrap()
                                    .to_owned(),
                                capture.span.start,
                                capture.span.end,
                            )
                        })
                        .collect::<Vec<_>>(),
                    tape.transducer_span(output.start).start,
                    tape.transducer_span(output.start).end,
                )
            })
            .collect::<Vec<_>>();
        actual_spans.sort();
        expected_spans.sort();
        assert_eq!(
            actual_spans, expected_spans,
            "match spans differ for query: {query_source}\nsource:\n{source}"
        );
    }

    #[test]
    fn alternative_programs_expose_every_sound_root_kind() {
        let language = SupportLang::Python.get_ts_language();
        let program =
            Program::compile_tree_sitter_query_pattern("[(identifier) (string)] @item", &language)
                .unwrap();
        let expected = [
            language.id_for_node_kind("identifier", true),
            language.id_for_node_kind("string", true),
        ]
        .into_iter()
        .collect();
        assert_eq!(program.leading_kinds(), Some(expected));
    }

    #[test]
    fn complete_scm_file_parser_returns_top_level_patterns_without_a_copied_catalog() {
        let source = r##"
; leading comment
(class_definition) @definition

((identifier) @name
 (#eq? @name "escaped\\\"value"))

["+" "-"] @operator
"##;
        assert_eq!(
            Program::tree_sitter_top_level_patterns(source).unwrap(),
            [
                "(class_definition) @definition",
                "((identifier) @name\n (#eq? @name \"escaped\\\\\\\"value\"))",
                "[\"+\" \"-\"] @operator",
            ]
        );
        assert!(Program::tree_sitter_top_level_patterns("(module").is_err());
        assert!(Program::tree_sitter_top_level_patterns("[\"x").is_err());
    }

    #[test]
    fn scm_compiler_accepts_captureless_patterns_and_rejects_invalid_input() {
        let language = SupportLang::Python.get_ts_language();
        let captureless =
            Program::compile_tree_sitter_query_pattern("(module)", &language).unwrap();
        assert_eq!(captureless.capture_names().count(), 0);
        assert!(
            Program::compile_tree_sitter_query_pattern(
                "(module) @scope (identifier) @extra",
                &language,
            )
            .is_err()
        );
        assert!(
            Program::compile_tree_sitter_query_pattern("(not_a_python_kind) @x", &language)
                .is_err()
        );
        let first =
            Program::compile_tree_sitter_query_pattern("  (module)   @root.scope  ", &language)
                .unwrap();
        let second =
            Program::compile_tree_sitter_query_pattern("(module) @root.scope", &language).unwrap();
        assert_eq!(first.capture_names().collect::<Vec<_>>(), ["root.scope"]);
        assert_eq!(
            first.canonical_digest("python", b"(module) @root.scope"),
            second.canonical_digest("python", b"(module) @root.scope")
        );
    }

    #[test]
    fn scm_parser_lowers_nested_fields_alternatives_and_predicates() {
        assert_scm_native_parity(
            "(function_definition name: (identifier) @name body: (block (return_statement (identifier) @value))) @function",
            "def answer():\n    return result\n",
        );
        assert_scm_native_parity(
            "([(integer) (float)] @number (#match? @number \"^[12]\"))",
            "first = 1\nsecond = 3.0\n",
        );
        assert_scm_native_parity(
            "(module (expression_statement (identifier) @item)+) @root",
            "a\nb\nc\n",
        );
        assert_scm_native_parity(
            "(module (expression_statement (identifier) @item)? (expression_statement (integer) @tail)) @root",
            "a\n1\n",
        );
        assert_scm_native_parity(
            "(module (expression_statement (identifier) @item)* (expression_statement (integer) @tail)) @root",
            "1\n",
        );
        assert_scm_native_parity(
            "(module (expression_statement (identifier) @item)+ (expression_statement (integer) @number)+) @root",
            "a\nb\n1\n2\n",
        );
        assert_scm_native_parity(
            "(module . (expression_statement [(identifier) (integer)] @item)+ .) @root",
            "a\n1\n",
        );
    }

    #[test]
    fn scm_quantifiers_match_native_for_groups_competing_paths_and_recovery() {
        let cases = [
            (
                "(module (expression_statement (identifier) @left)? (expression_statement (identifier) @right)? (expression_statement (integer) @tail)) @root",
                "a\nb\n1\n",
            ),
            (
                "(module ((expression_statement (identifier) @name) (expression_statement (integer) @value))+) @root",
                "a\n1\nb\n2\n",
            ),
            (
                "(module (expression_statement [(identifier) (integer)] @item)+ (expression_statement (string) @tail)) @root",
                "a\n1\nb\n\"done\"\n",
            ),
            (
                "(module (expression_statement (identifier) @item)* (expression_statement (integer) @tail)+) @root",
                "a\nb\n1\n2\n",
            ),
            (
                "(module (ERROR) @error+ ) @root",
                "def broken(:\n    pass\n",
            ),
            (
                "(module (function_definition (MISSING \":\")? @missing) @function) @root",
                "def broken()\n    pass\n",
            ),
            (
                "(module ((comment)+)+ @comments) @root",
                "# one\n# two\n# three\n",
            ),
            (
                "(module (comment)+ (expression_statement (integer)))",
                "# before\n1\n# after\n",
            ),
            (
                "(module ((comment)+) (expression_statement (integer)))",
                "# before\n1\n# after\n",
            ),
        ];
        for (query, source) in cases {
            assert_scm_native_parity(query, source);
        }
    }

    #[test]
    fn tree_sitter_capture_containment_uses_node_capture_and_span_identity() {
        let capture = |label, node, start, end| CaptureIdentity {
            label,
            node,
            start,
            end,
        };
        assert_eq!(capture_identity_containment(&[], &[]), (true, true));
        assert_eq!(
            capture_identity_containment(
                &[capture(0, 1, 0, 1), capture(0, 2, 2, 3)],
                &[capture(0, 1, 0, 1)],
            ),
            (true, false)
        );
        assert_eq!(
            capture_identity_containment(&[capture(0, 1, 0, 1)], &[capture(0, 2, 2, 3)],),
            (false, false)
        );
        assert_eq!(
            capture_identity_containment(&[capture(0, 1, 0, 1)], &[capture(1, 1, 0, 1)],),
            (false, false)
        );
        assert_eq!(
            capture_identity_containment(&[capture(0, 1, 4, 4)], &[capture(0, 2, 4, 4)],),
            (false, false),
            "distinct zero-width nodes at one point are not capture subsets",
        );
    }

    #[test]
    #[ignore = "offline inspection of compiled sibling-repeat instructions"]
    fn inspect_compiled_sibling_repeat_instructions() {
        let language = SupportLang::Python.get_ts_language();
        for query in [
            "(module (comment)+ (comment) @middle (comment)+)",
            "(module ((comment) @item)+)",
            "(module (comment)* (identifier) @id)",
        ] {
            let program = Program::compile_tree_sitter_query_pattern(query, &language).unwrap();
            eprintln!("QUERY {query}");
            for (pc, instruction) in program.words.iter().enumerate() {
                eprintln!("{pc}: {:?}", split(*instruction));
            }
            eprintln!("labels={:?}", program.labels);
        }
    }

    #[test]
    fn scm_quantified_runs_preserve_each_distinct_middle_capture() {
        let middle = "(module (comment)+ (comment) @middle (comment)+)";
        let captured_run = "(module ((comment) @item)+)";
        let captured_alternatives = "(module [(comment) @left (comment) @right]+)";
        let variable_width_alternatives = "(module [(comment) @item ((comment) (comment) @item)]+)";
        for count in 0..=8 {
            let source = (0..count)
                .map(|index| format!("# {index}\n"))
                .collect::<String>();
            assert_scm_native_parity(middle, &source);
            assert_scm_native_parity(captured_run, &source);
            assert_scm_native_parity(captured_alternatives, &source);
            assert_scm_native_parity(variable_width_alternatives, &source);
        }

        let stress = (0..128)
            .map(|index| format!("# {index}\n"))
            .collect::<String>();
        assert_scm_native_parity(middle, &stress);
        assert_scm_native_parity(captured_run, &stress);

        let language = SupportLang::Python.get_ts_language();
        let program = Program::compile_tree_sitter_query_pattern(middle, &language).unwrap();
        let stress_tape = tape(&stress);
        let plain_started = Instant::now();
        let plain_output = program.execute(&stress_tape).unwrap();
        let plain_elapsed = plain_started.elapsed();

        let jit_programs = vec![program.clone()];
        let jit_compile_started = Instant::now();
        let native = native_ir::lower(&program).unwrap();
        let _compiled = native_cpu::compile_straight_line(&native).unwrap();
        let jit_compile_elapsed = jit_compile_started.elapsed();
        let jit_tapes = vec![stress_tape.clone()];
        let jit_roots = (0..stress_tape.transducer_len() as u32).collect::<Vec<_>>();
        let jit_anchors = vec![vec![jit_roots]];
        let jit_started = Instant::now();
        let jit_output = execute_with_backend(
            ExecutionBackend::CpuJit,
            &jit_programs,
            &jit_tapes,
            &jit_anchors,
        )
        .unwrap();
        let jit_elapsed = jit_started.elapsed();
        assert_eq!(jit_output[0][0], plain_output);
        #[cfg(target_os = "macos")]
        {
            assert_eq!(
                metal_grid_backend::execute(&jit_programs, &jit_tapes, &jit_anchors,).unwrap()
                    [0][0],
                plain_output
            );
        }
        if let Some(iterations) = std::env::var("TEMPO_QUANTIFIER_PROFILE_ITERATIONS")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
        {
            for _ in 0..iterations {
                black_box(
                    execute_with_backend(
                        ExecutionBackend::CpuJit,
                        &jit_programs,
                        &jit_tapes,
                        &jit_anchors,
                    )
                    .unwrap(),
                );
            }
        }

        let started = Instant::now();
        let (stress_output, stats) = program.execute_with_stats(&stress_tape).unwrap();
        let elapsed = started.elapsed();
        assert_eq!(stress_output.len(), 126);
        assert!(stats.explored_states < 100_000, "{stats:?}");
        assert!(stats.max_pending_paths < 512, "{stats:?}");
        assert!(stats.peak_retained_bytes < 2 * 1024 * 1024, "{stats:?}");
        assert!(elapsed < Duration::from_secs(5), "{elapsed:?} {stats:?}");
        assert!(plain_elapsed < Duration::from_secs(2), "{plain_elapsed:?}");
        assert!(jit_elapsed < Duration::from_secs(2), "{jit_elapsed:?}");
        eprintln!(
            "128-comment quantifier stress: interpreter={plain_elapsed:?} jitCompile={jit_compile_elapsed:?} jit={jit_elapsed:?} instrumented={elapsed:?} {stats:?}"
        );

        assert_scm_native_parity(
            "((module ((comment) @item)+) (#eq? @item \"# 0\"))",
            "# 0\n# 1\n",
        );
    }

    #[test]
    fn scm_quantifier_zero_paths_preserve_anchor_obligations() {
        let cases = [
            (
                "(module . (comment)* (expression_statement (identifier) @value))",
                "value\n",
            ),
            (
                "(module . (comment)* (expression_statement (identifier) @value))",
                "1\nvalue\n",
            ),
            (
                "(module (expression_statement (identifier) @left) . (comment)* (expression_statement (identifier) @right))",
                "left\n1\nright\n",
            ),
            (
                "(module (expression_statement (identifier) @left) . (comment)* . (expression_statement (identifier) @right))",
                "left\n1\nright\n",
            ),
            (
                "(module (expression_statement (identifier) @left) (comment)* . (expression_statement (identifier) @right))",
                "left\n1\nright\n",
            ),
            (
                "(module (expression_statement (identifier) @value) (comment)* .)",
                "value\n",
            ),
            (
                "(module (expression_statement (identifier) @value) (comment)* .)",
                "value\n1\n",
            ),
            (
                "(module (expression_statement (identifier) @value) (expression_statement (integer))* .)",
                "value\n\"later\"\n",
            ),
        ];
        for (query, source) in cases {
            assert_scm_native_parity(query, source);
        }
    }

    #[test]
    fn scm_quantifiers_match_native_for_generated_sibling_counts() {
        for count in 0..=5 {
            let mut source = String::new();
            for index in 0..count {
                source.push_str(&format!("name_{index}\n"));
            }
            source.push_str("1\n");
            assert_scm_native_parity(
                "(module (expression_statement (identifier) @item)* (expression_statement (integer) @tail)) @root",
                &source,
            );
            assert_scm_native_parity(
                "(module (expression_statement (identifier))* (expression_statement (integer) @tail)) @root",
                &source,
            );
            if count > 0 {
                assert_scm_native_parity(
                    "(module (expression_statement (identifier) @item)+ (expression_statement (integer) @tail)) @root",
                    &source,
                );
            }
        }

        assert_scm_native_parity(
            "(module . (expression_statement (identifier) @left) . (expression_statement (identifier) @right)) @root",
            "left\n# an extra node\nright\n",
        );
        assert_scm_native_parity(
            "((module (expression_statement (identifier) @item)+) @root (#any-eq? @item \"name_1\"))",
            "name_0\nname_1\nname_2\n",
        );
        assert_scm_native_parity(
            "((module (expression_statement (identifier) @item)+) @root (#any-not-eq? @item \"name_1\"))",
            "name_0\nname_1\nname_2\n",
        );
    }

    #[test]
    fn cpu_jit_compiles_general_scm_control_flow() {
        let language = SupportLang::Python.get_ts_language();
        let source = "# 0\n# 1\n# 2\n# 3\nvalue = call(arg)\n";
        let programs = [
            "(module (comment)+ (comment) @middle (comment)+)",
            "(module ((comment) @item)+)",
            "(module (expression_statement (assignment left: (identifier) @left right: (call function: (identifier) @callee arguments: (argument_list (identifier) @argument)))) @root)",
            "((identifier) @name (#eq? @name \"value\"))",
            "((identifier) @name (#set! \"role\" \"definition\") (#downcase! @name))",
        ]
        .into_iter()
        .map(|query| Program::compile_tree_sitter_query_pattern(query, &language).unwrap())
        .collect::<Vec<_>>();
        let tapes = vec![tape(source)];
        let roots = (0..tapes[0].transducer_len() as u32).collect::<Vec<_>>();
        let anchors = vec![programs.iter().map(|_| roots.clone()).collect::<Vec<_>>()];
        let expected = execute_batch(&programs, &tapes, Some(&anchors)).unwrap();
        assert_eq!(
            execute_with_backend(ExecutionBackend::CpuJit, &programs, &tapes, &anchors).unwrap(),
            expected
        );
        #[cfg(target_os = "macos")]
        assert_metal_parity(&programs, &tapes);
    }

    #[test]
    fn all_backends_execute_every_verified_instruction_and_mode() {
        let grammar = SupportLang::Python.get_ts_language();
        let identifier = grammar.id_for_node_kind("identifier", true);
        let function = grammar.field_id_for_name("function").unwrap().get();
        let alternative = grammar.field_id_for_name("alternative").unwrap().get();
        let labels = || vec![0];
        let strings = || vec![b"match".to_vec()];
        let plain =
            |words| Program::new(words, strings(), vec![], labels(), vec![100_000]).unwrap();
        let mut programs = vec![
            plain(vec![word(STOP, 0, 0)]),
            plain(vec![word(KIND, 0, identifier), word(EMIT, 0, 0)]),
            plain(vec![word(FIELD, 0, function), word(EMIT, 0, 0)]),
            plain(vec![word(FLAGS, 1, 0b1110), word(EMIT, 0, 0)]),
            plain(vec![word(FIRST, 0, 0), word(EMIT, 0, 0)]),
            plain(vec![word(FIRST, NAMED, 0), word(EMIT, 0, 0)]),
            plain(vec![word(LAST, 0, 0), word(EMIT, 0, 0)]),
            plain(vec![word(LAST, NAMED, 0), word(EMIT, 0, 0)]),
            plain(vec![word(ABSENT_FIELD, 0, alternative), word(EMIT, 0, 0)]),
            plain(vec![word(CHILD, 0, 0), word(EMIT, 0, 0)]),
            plain(vec![word(CHILD, NAMED, 0), word(EMIT, 0, 0)]),
            plain(vec![word(NEXT, 0, 0), word(EMIT, 0, 0)]),
            plain(vec![word(NEXT, NAMED, 0), word(EMIT, 0, 0)]),
            plain(vec![word(PARENT, 0, 0), word(EMIT, 0, 0)]),
            plain(vec![word(DESCEND, 0, 0), word(EMIT, 0, 0)]),
            plain(vec![word(DESCEND, NAMED, 0), word(EMIT, 0, 0)]),
            plain(vec![word(FOLLOWING, 0, 0), word(EMIT, 0, 0)]),
            plain(vec![word(FOLLOWING, NAMED, 0), word(EMIT, 0, 0)]),
            plain(vec![
                word(SAVE_CURSOR, 0, 0),
                word(CHILD, NAMED, 0),
                word(RESTORE_CURSOR, 0, 0),
                word(EMIT, 0, 0),
            ]),
            plain(vec![word(CAPTURE, 0, 0), word(EMIT, 0, 0)]),
            plain(vec![
                word(REPEAT_BEGIN, 0, 7),
                word(REPEAT_BUMP, 0, 7),
                word(REPEAT_END, 0, 7),
                word(EMIT, 0, 0),
            ]),
            plain(vec![word(FORK, 0, 2), word(EMIT, 0, 0), word(EMIT, 0, 0)]),
            plain(vec![
                word(FORK, GREEDY, 2),
                word(EMIT, 0, 0),
                word(EMIT, 0, 0),
            ]),
            plain(vec![word(GOTO, 0, 1), word(EMIT, 0, 0)]),
        ];
        #[cfg(target_os = "macos")]
        let mut metal_programs = programs.clone();
        for required in 0_u8..=0x0f {
            for forbidden in 0_u16..=0x000f {
                if u16::from(required) & forbidden == 0 {
                    programs.push(plain(vec![
                        word(FLAGS, required, forbidden),
                        word(EMIT, 0, 0),
                    ]));
                }
            }
        }
        for mode in [0, 1, 2] {
            programs.push(plain(vec![word(SIZE, mode, 1), word(EMIT, 0, 0)]));
        }
        for mode in [0x80, 0x81, 0x82] {
            programs.push(plain(vec![word(SIZE, mode, 0), word(EMIT, 0, 0)]));
        }
        for predicate in [
            TextPredicate::Equal(0),
            TextPredicate::Prefix(0),
            TextPredicate::Regex {
                source: 0,
                compiled: Regex::new("match").unwrap(),
            },
            TextPredicate::LuaPattern {
                source: 0,
                compiled: LuaPattern::parse(b"match").unwrap(),
            },
        ] {
            let program = Program::new(
                vec![word(TEXT, 0, 0), word(EMIT, 0, 0)],
                vec![b"match".to_vec(), b"output".to_vec()],
                vec![predicate],
                vec![1],
                vec![],
            )
            .unwrap();
            #[cfg(target_os = "macos")]
            metal_programs.push(program.clone());
            programs.push(program);
        }
        let assertion_program = Program::new_with_capture_predicates(
            vec![word(CAPTURE, 0, 0), word(ASSERT, 0, 0), word(EMIT, 0, 0)],
            vec![b"capture".to_vec(), b"output".to_vec()],
            vec![],
            vec![CapturePredicate::Kind {
                capture: 0,
                kind: identifier,
                negated: false,
                any: false,
            }],
            vec![0, 1],
            vec![],
        )
        .unwrap();
        #[cfg(target_os = "macos")]
        metal_programs.push(assertion_program.clone());
        programs.push(assertion_program);
        let source = "# first\nvalue = call(argument)\nif value:\n  pass\n";
        let python_tape = tape(source);
        assert_jit_parity(&programs, std::slice::from_ref(&python_tape));
        #[cfg(target_os = "macos")]
        {
            let alternatives = |blocks: Vec<Vec<u32>>| {
                let prefix = blocks.len() + 1;
                let mut at = prefix;
                let mut words = Vec::new();
                for block in &blocks {
                    words.push(word(FORK, 0, at as u16));
                    at += block.len();
                }
                words.push(word(STOP, 0, 0));
                words.extend(blocks.into_iter().flatten());
                plain(words)
            };
            let flags = alternatives(
                (0_u8..=0x0f)
                    .flat_map(|required| {
                        (0_u16..=0x000f).filter_map(move |forbidden| {
                            (u16::from(required) & forbidden == 0)
                                .then_some(vec![word(FLAGS, required, forbidden), word(EMIT, 0, 0)])
                        })
                    })
                    .collect(),
            );
            let sizes = alternatives(
                [0, 1, 2, 0x80, 0x81, 0x82]
                    .into_iter()
                    .map(|mode| {
                        vec![
                            word(SIZE, mode, u16::from(mode & SIZE_LONG == 0)),
                            word(EMIT, 0, 0),
                        ]
                    })
                    .collect(),
            );
            metal_programs.extend([flags, sizes]);
            assert_metal_parity(&metal_programs, std::slice::from_ref(&python_tape));
        }
        for words in [
            vec![word(RESTORE_CURSOR, 0, 0), word(EMIT, 0, 0)],
            vec![word(REPEAT_BUMP, 0, 7), word(EMIT, 0, 0)],
            vec![word(REPEAT_END, 0, 7), word(EMIT, 0, 0)],
        ] {
            let program = plain(words);
            let expected = program.execute_roots(&python_tape, [0]).unwrap_err();
            let actual = execute_with_backend(
                ExecutionBackend::CpuJit,
                std::slice::from_ref(&program),
                std::slice::from_ref(&python_tape),
                &[vec![vec![0]]],
            )
            .unwrap_err();
            assert_eq!(actual, expected);
            #[cfg(target_os = "macos")]
            {
                let actual = metal_grid_backend::execute(
                    std::slice::from_ref(&program),
                    std::slice::from_ref(&python_tape),
                    &[vec![vec![0]]],
                )
                .unwrap_err();
                assert_eq!(actual, expected);
            }
        }

        let js = SupportLang::JavaScript;
        let js_language = js.get_ts_language();
        let js_source = "run();\n";
        let supertype = Program::compile_tree_sitter_query_pattern(
            "(primary_expression/call_expression) @reference",
            &js_language,
        )
        .unwrap();
        let js_grammar = Grammar::new(js).unwrap();
        let js_native = StrDoc::new(js_source, js);
        let js_image =
            TapeImage::encode(js_native.tree.root_node(), &js_grammar, js_source, &|| {
                false
            })
            .unwrap();
        let js_tape = TapeDoc::restore(
            Arc::new(js_source.to_owned()),
            js_grammar,
            &js_image,
            &|| false,
        )
        .unwrap();
        assert_jit_parity(
            std::slice::from_ref(&supertype),
            std::slice::from_ref(&js_tape),
        );
        #[cfg(target_os = "macos")]
        assert_metal_parity(std::slice::from_ref(&supertype), &[js_tape]);

        let opcodes = programs
            .iter()
            .chain(std::iter::once(&supertype))
            .flat_map(|program| program.words.iter().map(|word| split(*word).0))
            .collect::<BTreeSet<_>>();
        assert_eq!(
            opcodes,
            BTreeSet::from([
                STOP,
                KIND,
                FIELD,
                FLAGS,
                TEXT,
                SIZE,
                ASSERT,
                FIRST,
                LAST,
                ABSENT_FIELD,
                SUPERTYPE,
                CHILD,
                NEXT,
                PARENT,
                DESCEND,
                FOLLOWING,
                SAVE_CURSOR,
                RESTORE_CURSOR,
                CAPTURE,
                REPEAT_BEGIN,
                REPEAT_BUMP,
                REPEAT_END,
                FORK,
                GOTO,
                EMIT,
            ])
        );
    }

    #[test]
    fn scm_supertype_paths_use_persisted_hidden_ancestors() {
        use tree_sitter::StreamingIterator;

        let support = SupportLang::JavaScript;
        let language = support.get_ts_language();
        let source = "run();\n";
        let query = "(primary_expression/call_expression) @reference";
        let program = Program::compile_tree_sitter_query_pattern(query, &language).unwrap();
        let grammar = Grammar::new(support).unwrap();
        let native_doc = StrDoc::new(source, support);
        let image =
            TapeImage::encode(native_doc.tree.root_node(), &grammar, source, &|| false).unwrap();
        let tape =
            TapeDoc::restore(Arc::new(source.to_owned()), grammar, &image, &|| false).unwrap();
        let actual = program
            .execute(&tape)
            .unwrap()
            .into_iter()
            .flat_map(|output| output.captures)
            .map(|capture| capture.span)
            .collect::<Vec<_>>();

        let query = tree_sitter::Query::new(&language, query).unwrap();
        let mut cursor = tree_sitter::QueryCursor::new();
        let mut matches = cursor.matches(&query, native_doc.tree.root_node(), source.as_bytes());
        let mut expected = Vec::new();
        while let Some(found) = matches.next() {
            expected.extend(
                found
                    .captures()
                    .iter()
                    .map(|capture| capture.node.byte_range()),
            );
        }
        assert_eq!(actual, expected);
        assert!(!actual.is_empty());
    }

    #[test]
    fn scm_host_predicates_filter_text_kinds_lines_and_relations() {
        let language = SupportLang::Python.get_ts_language();
        let source = "insert(foo)\nvalue = 1\na = (\n  b\n)\n";
        let cases = [
            ("((identifier) @x (#contains? @x \"sert\"))", vec!["insert"]),
            (
                "([(identifier) (integer)] @x (#not-kind-eq? @x \"integer\"))",
                vec!["insert", "foo", "value", "a", "b"],
            ),
            (
                "((identifier) @x (#has-parent? @x argument_list))",
                vec!["foo"],
            ),
            (
                "((identifier) @x (#has-ancestor? @x call))",
                vec!["insert", "foo"],
            ),
            (
                "((assignment left: (identifier) @left right: (parenthesized_expression (identifier) @right)) (#same-line? @left @right))",
                Vec::new(),
            ),
            (
                "((assignment left: (identifier) @left right: (parenthesized_expression (identifier) @right)) (#not-same-line? @left @right))",
                vec!["a", "b"],
            ),
        ];
        let tape = tape(source);
        let programs = cases
            .iter()
            .map(|(query, _)| Program::compile_tree_sitter_query_pattern(query, &language).unwrap())
            .collect::<Vec<_>>();
        for ((query, expected), program) in cases.into_iter().zip(&programs) {
            let actual = program
                .execute(&tape)
                .unwrap()
                .into_iter()
                .flat_map(|output| output.captures)
                .filter(|capture| program.label_name(capture.label) != Some("tree-sitter-query"))
                .map(|capture| source[capture.span].to_owned())
                .collect::<Vec<_>>();
            assert_eq!(actual, expected, "query: {query}");
        }
        #[cfg(target_os = "macos")]
        assert_metal_parity(&programs, std::slice::from_ref(&tape));
    }

    #[test]
    fn scm_output_directives_transform_text_adjacency_and_language() {
        let language = SupportLang::Python.get_ts_language();
        let source =
            "# old\n\n# near\n# nearest\ndef run():\n  pass\nRUBY\n'application/ecmascript'\n";
        let adjacency = Program::compile_tree_sitter_query_pattern(
            r#"((module (comment)+ @doc . (function_definition) @definition)
                 (#strip! @doc "^#\\s*")
                 (#select-adjacent! @doc @definition))"#,
            &language,
        )
        .unwrap();
        let label = |name: &[u8]| {
            adjacency
                .labels
                .iter()
                .position(|label| adjacency.strings[usize::from(*label)] == name)
                .unwrap() as u16
        };
        let mut captures = vec![
            Capture {
                label: label(b"doc"),
                node: 0,
                span: 0..5,
                text: None,
                properties: Vec::new(),
            },
            Capture {
                label: label(b"doc"),
                node: 1,
                span: 7..13,
                text: None,
                properties: Vec::new(),
            },
            Capture {
                label: label(b"doc"),
                node: 2,
                span: 14..23,
                text: None,
                properties: Vec::new(),
            },
            Capture {
                label: label(b"definition"),
                node: 3,
                span: 24..42,
                text: None,
                properties: Vec::new(),
            },
        ];
        adjacency
            .apply_directives(&mut captures, source.as_bytes())
            .unwrap()
            .unwrap();
        let transformed = captures
            .iter()
            .filter(|capture| capture.label == label(b"doc"))
            .map(|capture| std::str::from_utf8(capture.text.as_deref().unwrap()).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(transformed, ["near", "nearest"]);

        let query = r#"
          ((identifier) @language (#downcase! @language))
          ((string (string_content) @mime) (#set-lang-from-mimetype! @mime))
        "#;
        let directive_programs = Program::tree_sitter_top_level_patterns(query)
            .unwrap()
            .iter()
            .map(|pattern| Program::compile_tree_sitter_query_pattern(pattern, &language).unwrap())
            .collect::<Vec<_>>();
        let directive_tape = tape(source);
        let outputs = directive_programs
            .iter()
            .flat_map(|program| program.execute(&directive_tape).unwrap())
            .collect::<Vec<_>>();
        #[cfg(target_os = "macos")]
        {
            assert_metal_parity(&directive_programs, std::slice::from_ref(&directive_tape));
            assert_metal_parity(
                std::slice::from_ref(&adjacency),
                std::slice::from_ref(&directive_tape),
            );
        }
        assert!(
            outputs
                .iter()
                .flat_map(|output| &output.captures)
                .any(|capture| capture.text.as_deref() == Some(b"ruby"))
        );
        assert!(outputs.iter().any(|output| {
            output
                .dynamic_properties
                .iter()
                .any(|(key, value)| key == b"language" && value.as_deref() == Some(b"javascript"))
        }));

        let alias = Program::compile_tree_sitter_query_pattern(
            "((module (comment)+ @doc . (function_definition) @definition) (#set-adjacent! @doc @definition))",
            &language,
        );
        assert!(alias.is_ok());
    }

    #[test]
    fn scm_directives_adjust_capture_spans_and_retain_properties() {
        let language = SupportLang::Python.get_ts_language();
        let program = Program::compile_tree_sitter_query_pattern(
            "((identifier) @name (#offset! @name 0 1 0 -1) (#set! role \"trimmed\") (#set! @name bo.commentstring \"// %s\"))",
            &language,
        )
        .unwrap();
        let value_tape = tape("value\n");
        let output = program.execute(&value_tape).unwrap();
        #[cfg(target_os = "macos")]
        assert_metal_parity(
            std::slice::from_ref(&program),
            std::slice::from_ref(&value_tape),
        );
        assert_eq!(output.len(), 1);
        assert_eq!(output[0].captures[0].span, 1..4);
        assert_eq!(output[0].captures[0].properties.len(), 1);
        let (capture_key, capture_value) = output[0].captures[0].properties[0];
        assert_eq!(program.string_value(capture_key), Some("bo.commentstring"));
        assert_eq!(
            capture_value.and_then(|value| program.string_value(value)),
            Some("// %s")
        );
        assert_eq!(output[0].properties.len(), 1);
        let (key, value) = output[0].properties[0];
        assert_eq!(program.strings[usize::from(key)], b"role");
        assert_eq!(
            program.strings[usize::from(value.expect("property value"))],
            b"trimmed"
        );

        let invalid = Program::compile_tree_sitter_query_pattern(
            "((identifier) @name (#offset! @name 0 -1 0 0))",
            &language,
        )
        .unwrap();
        assert!(invalid.execute(&tape("value\n")).unwrap().is_empty());

        let utf8_split = Program::compile_tree_sitter_query_pattern(
            "((identifier) @name (#offset! @name 0 1 0 0))",
            &language,
        )
        .unwrap();
        assert!(utf8_split.execute(&tape("éx\n")).unwrap().is_empty());
    }

    #[test]
    fn lua_patterns_use_lua_byte_classes_and_repetition() {
        let cases = [
            ("^[A-Z].*[a-z]", b"ClassName".as_slice(), true),
            ("^[A-Z].*[a-z]", b"CONSTANT".as_slice(), false),
            ("^[%l_].*$", b"_member".as_slice(), true),
            ("^%u", b"Builder".as_slice(), true),
            ("^%([^\n]", b"(value".as_slice(), true),
            ("[^\n ]%)$", b"value)".as_slice(), true),
            ("^else", b"or_else".as_slice(), false),
        ];
        for (source, value, expected) in cases {
            let pattern = LuaPattern::parse(source.as_bytes()).unwrap();
            assert_eq!(pattern.matches(value), expected, "{source}");
        }
        assert!(LuaPattern::parse(b"(%a+)").unwrap().matches(b"name"));
        assert!(
            LuaPattern::parse(b"^[-][-](%s?)@")
                .unwrap()
                .matches(b"-- @")
        );
        assert!(LuaPattern::parse(b"%f[%a]word").unwrap().matches(b"a word"));
        assert!(!LuaPattern::parse(b"%f[%a]word").unwrap().matches(b"sword"));
        assert!(LuaPattern::parse(b"^%b()$").unwrap().matches(b"(a(b)c)"));
        assert!(!LuaPattern::parse(b"^%b()$").unwrap().matches(b"(a(b)"));
        assert!(
            LuaPattern::parse(b"^(%a+)%s+%1$")
                .unwrap()
                .matches(b"same same")
        );
        assert!(
            !LuaPattern::parse(b"^(%a+)%s+%1$")
                .unwrap()
                .matches(b"same other")
        );
        assert!(LuaPattern::parse(b"()value").unwrap().matches(b"value"));
        assert!(LuaPattern::parse(b"^a.-b$").unwrap().matches(b"axxxbxxb"));
        assert!(LuaPattern::parse(b"^%z$").unwrap().matches(b"\0"));
        assert!(LuaPattern::parse(b"^..$").unwrap().matches("é".as_bytes()));
        assert!(!LuaPattern::parse(b"^%a+$").unwrap().matches("é".as_bytes()));
        assert!(LuaPattern::parse(b"%1").is_err());
        assert!(LuaPattern::parse(b"()%1").is_err());
        assert!(LuaPattern::parse(b"(%a%1)").is_err());
        assert!(LuaPattern::parse(b"(%a+").is_err());
        assert!(LuaPattern::parse(b"%b(").is_err());
        assert!(LuaPattern::parse(b"%f%a").is_err());

        let bounded = LuaPattern::parse(b"a*").unwrap();
        let mut exhausted = MAX_STEPS_PER_PATH;
        assert!(bounded.matches_with_work(b"aaaa", &mut exhausted).is_err());

        let language = SupportLang::Python.get_ts_language();
        let program = Program::compile_tree_sitter_query_pattern(
            "((identifier) @name (#not-lua-match? @name \"^skip$\"))",
            &language,
        )
        .unwrap();
        let source = "keep = skip\n";
        let found = program
            .execute(&tape(source))
            .unwrap()
            .into_iter()
            .flat_map(|output| output.captures)
            .map(|capture| &source[capture.span])
            .collect::<Vec<_>>();
        assert_eq!(found, ["keep"]);
    }

    #[test]
    fn canonical_identity_changes_for_every_program_component() {
        fn changed(base: &Program, mutate: impl FnOnce(&mut Program)) {
            let original = base.canonical_digest("python", b"source");
            let mut changed = base.clone();
            mutate(&mut changed);
            assert_ne!(original, changed.canonical_digest("python", b"source"));
        }

        let language = SupportLang::Python.get_ts_language();
        let base =
            Program::compile_tree_sitter_query_pattern("(identifier) @symbol", &language).unwrap();
        assert_ne!(
            base.canonical_digest("python", b"source"),
            base.canonical_digest("javascript", b"source")
        );
        assert_ne!(
            base.canonical_digest("python", b"source"),
            base.canonical_digest("python", b"other source")
        );
        changed(&base, |program| program.words[0] ^= 1 << 8);
        changed(&base, |program| program.strings[0].push(b'x'));
        changed(&base, |program| {
            program.predicates.push(TextPredicate::Equal(0))
        });
        changed(&base, |program| program.labels[0] = 1);
        changed(&base, |program| program.longs.push(1));
        changed(&base, |program| {
            program.tree_sitter_query = !program.tree_sitter_query
        });
        changed(&base, |program| {
            program.directives.push(Directive::Set {
                key: 0,
                value: None,
            });
        });

        let capture_predicates = [
            CapturePredicate::Equal {
                left: 0,
                right: 0,
                negated: false,
                any: false,
            },
            CapturePredicate::Text {
                capture: 0,
                predicate: 0,
                negated: false,
                any: false,
            },
            CapturePredicate::AnyOf {
                capture: 0,
                strings: vec![0],
                negated: false,
                any: false,
            },
            CapturePredicate::Kind {
                capture: 0,
                kind: 1,
                negated: false,
                any: false,
            },
        ];
        for predicate in capture_predicates {
            changed(&base, |program| program.capture_predicates.push(predicate));
        }
        let with_any_of = {
            let mut program = base.clone();
            program.capture_predicates.push(CapturePredicate::AnyOf {
                capture: 0,
                strings: vec![0],
                negated: false,
                any: false,
            });
            program
        };
        changed(&with_any_of, |program| {
            let CapturePredicate::AnyOf { strings, .. } = &mut program.capture_predicates[0] else {
                unreachable!()
            };
            strings.push(1);
        });
    }

    #[test]
    fn fixed_words_execute_identifier_transducer() {
        let tape = tape("execute(x)\nother()\nexecute(y)\n");
        let program = identifier_program("execute");
        assert!(
            program
                .words
                .iter()
                .all(|word| word.to_le_bytes().len() == 4)
        );
        let output = program.execute(&tape).unwrap();
        assert_eq!(output.len(), 2);
        assert_eq!(output[0].captures[0].span, 0..7);
        assert_eq!(output[1].captures[0].span, 19..26);
    }

    #[test]
    fn navigation_captures_eval_call_and_callee() {
        let grammar = SupportLang::Python.get_ts_language();
        let call = grammar.id_for_node_kind("call", true);
        let identifier = grammar.id_for_node_kind("identifier", true);
        let function = grammar.field_id_for_name("function").unwrap().get();
        let program = Program::new(
            vec![
                word(KIND, 0, call),
                word(CAPTURE, 0, 0),
                word(CHILD, 0, 0),
                word(FIELD, 0, function),
                word(KIND, 0, identifier),
                word(TEXT, 0, 0),
                word(CAPTURE, 0, 1),
                word(EMIT, 0, 2),
            ],
            vec![
                b"eval".to_vec(),
                b"call".to_vec(),
                b"callee".to_vec(),
                b"match".to_vec(),
            ],
            vec![TextPredicate::Equal(0)],
            vec![1, 2, 3],
            vec![],
        )
        .unwrap();
        let output = program
            .execute(&tape("eval(x)\nobj.eval(x)\nother(x)\n"))
            .unwrap();
        assert_eq!(output.len(), 1);
        assert_eq!(output[0].captures[0].span, 0..7);
        assert_eq!(output[0].captures[1].span, 0..4);
    }

    #[test]
    fn alternatives_do_not_share_capture_mutation() {
        let program = Program::new(
            vec![
                word(
                    KIND,
                    0,
                    SupportLang::Python
                        .get_ts_language()
                        .id_for_node_kind("identifier", true),
                ),
                word(FORK, 0, 4),
                word(CAPTURE, 0, 0),
                word(EMIT, 0, 1),
                word(EMIT, 0, 1),
            ],
            vec![b"captured".to_vec(), b"match".to_vec()],
            vec![],
            vec![0, 1],
            vec![],
        )
        .unwrap();
        let output = program.execute(&tape("name\n")).unwrap();
        assert_eq!(output.len(), 2);
        assert_eq!(output[0].captures.len(), 1);
        assert!(output[1].captures.is_empty());
    }

    #[test]
    fn text_predicate_variants_use_declared_semantics() {
        let tape = tape("execute\n");
        for (predicate, source) in [
            (TextPredicate::Prefix(0), b"exec".to_vec()),
            (
                TextPredicate::Regex {
                    source: 0,
                    compiled: Regex::new("^exec(?:ute)?$").unwrap(),
                },
                b"^exec(?:ute)?$".to_vec(),
            ),
        ] {
            let program = Program::new(
                vec![
                    word(
                        KIND,
                        0,
                        SupportLang::Python
                            .get_ts_language()
                            .id_for_node_kind("identifier", true),
                    ),
                    word(TEXT, 0, 0),
                    word(EMIT, 0, 0),
                ],
                vec![source, b"match".to_vec()],
                vec![predicate],
                vec![1],
                vec![],
            )
            .unwrap();
            assert_eq!(program.execute(&tape).unwrap().len(), 1);
        }
    }

    #[test]
    fn capture_predicates_compare_text_sets_regex_kinds_and_negation() {
        let grammar = SupportLang::Python.get_ts_language();
        let identifier = grammar.id_for_node_kind("identifier", true);
        let program = Program::new_with_capture_predicates(
            vec![
                word(KIND, 0, identifier),
                word(CAPTURE, 0, 0),
                word(NEXT, NAMED, 0),
                word(KIND, 0, identifier),
                word(CAPTURE, 0, 1),
                word(ASSERT, 0, 0),
                word(ASSERT, 0, 1),
                word(ASSERT, 0, 2),
                word(ASSERT, 0, 3),
                word(EMIT, 0, 2),
            ],
            vec![
                b"left".to_vec(),
                b"right".to_vec(),
                b"match".to_vec(),
                b"^same$".to_vec(),
                b"same".to_vec(),
                b"other".to_vec(),
            ],
            vec![TextPredicate::Regex {
                source: 3,
                compiled: Regex::new("^same$").unwrap(),
            }],
            vec![
                CapturePredicate::Equal {
                    left: 0,
                    right: 1,
                    negated: false,
                    any: false,
                },
                CapturePredicate::Text {
                    capture: 0,
                    predicate: 0,
                    negated: false,
                    any: false,
                },
                CapturePredicate::AnyOf {
                    capture: 1,
                    strings: vec![4, 5],
                    negated: false,
                    any: false,
                },
                CapturePredicate::Kind {
                    capture: 1,
                    kind: identifier,
                    negated: true,
                    any: false,
                },
            ],
            vec![0, 1, 2],
            vec![],
        )
        .unwrap();
        assert!(program.execute(&tape("same = same\n")).unwrap().is_empty());

        let mut predicates = program.capture_predicates.clone();
        predicates[3] = CapturePredicate::Kind {
            capture: 1,
            kind: identifier,
            negated: false,
            any: false,
        };
        let program = program.with_capture_predicates(predicates).unwrap();
        assert_eq!(program.execute(&tape("same = same\n")).unwrap().len(), 1);
        assert!(program.execute(&tape("same = other\n")).unwrap().is_empty());
    }

    #[test]
    fn quantified_capture_predicates_apply_negation_per_capture() {
        let identifier = SupportLang::Python
            .get_ts_language()
            .id_for_node_kind("identifier", true);
        let program = Program::new_with_capture_predicates(
            vec![
                word(KIND, 0, identifier),
                word(CAPTURE, 0, 0),
                word(NEXT, NAMED, 0),
                word(KIND, 0, identifier),
                word(CAPTURE, 0, 0),
                word(ASSERT, 0, 0),
                word(EMIT, 0, 1),
            ],
            vec![b"item".to_vec(), b"match".to_vec(), b"same".to_vec()],
            vec![],
            vec![CapturePredicate::AnyOf {
                capture: 0,
                strings: vec![2],
                negated: true,
                any: false,
            }],
            vec![0, 1],
            vec![],
        )
        .unwrap();
        assert!(program.execute(&tape("same = other\n")).unwrap().is_empty());
        assert_eq!(program.execute(&tape("left = right\n")).unwrap().len(), 1);
    }

    #[test]
    fn positional_and_negated_field_tests_filter_nodes() {
        let grammar = SupportLang::Python.get_ts_language();
        let identifier = grammar.id_for_node_kind("identifier", true);
        let alternative = grammar.field_id_for_name("alternative").unwrap().get();
        let positions = Program::new(
            vec![
                word(KIND, 0, identifier),
                word(FIRST, NAMED, 0),
                word(LAST, NAMED, 0),
                word(EMIT, 0, 0),
            ],
            vec![b"match".to_vec()],
            vec![],
            vec![0],
            vec![],
        )
        .unwrap();
        assert_eq!(positions.execute(&tape("only\n")).unwrap().len(), 1);
        assert!(
            positions
                .execute(&tape("left + right\n"))
                .unwrap()
                .is_empty()
        );

        let no_else = Program::new(
            vec![
                word(KIND, 0, grammar.id_for_node_kind("if_statement", true)),
                word(ABSENT_FIELD, 0, alternative),
                word(EMIT, 0, 0),
            ],
            vec![b"match".to_vec()],
            vec![],
            vec![0],
            vec![],
        )
        .unwrap();
        assert_eq!(no_else.execute(&tape("if ok:\n  pass\n")).unwrap().len(), 1);
        assert!(
            no_else
                .execute(&tape("if ok:\n  pass\nelse:\n  pass\n"))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn traversal_flags_sizes_and_long_table_execute() {
        let grammar = SupportLang::Python.get_ts_language();
        let identifier = grammar.id_for_node_kind("identifier", true);
        let program = Program::new(
            vec![
                word(DESCEND, NAMED, 0),
                word(KIND, 0, identifier),
                word(FLAGS, 1, 0b1110),
                word(SIZE, 0x82, 0),
                word(CAPTURE, 0, 0),
                word(NEXT, NAMED, 0),
                word(PARENT, 0, 0),
                word(EMIT, 0, 1),
            ],
            vec![b"identifier".to_vec(), b"match".to_vec()],
            vec![],
            vec![0, 1],
            vec![100_000],
        )
        .unwrap();
        // Every identifier is shorter than the long constant. Only identifiers with a later
        // named sibling survive NEXT, and PARENT returns their shared syntax parent.
        let output = program.execute(&tape("pair = (alpha, beta)\n")).unwrap();
        assert!(!output.is_empty());
        assert!(
            output
                .iter()
                .all(|item| item.captures[0].span.len() <= 100_000)
        );
    }

    #[test]
    fn backward_goto_requires_and_uses_monotonic_next() {
        let identifier = SupportLang::Python
            .get_ts_language()
            .id_for_node_kind("identifier", true);
        let program = Program::new(
            vec![
                word(KIND, 0, identifier),
                word(CAPTURE, 0, 0),
                word(FORK, 0, 5),
                word(NEXT, NAMED, 0),
                word(GOTO, 0, 0),
                word(EMIT, 0, 1),
            ],
            vec![b"item".to_vec(), b"sequence".to_vec()],
            vec![],
            vec![0, 1],
            vec![],
        )
        .unwrap();
        let tape = tape("f(a, b, c)\n");
        let root = (0..tape.transducer_len() as u32)
            .find(|&node| tape.transducer_text(node) == b"a")
            .unwrap();
        let output = program.execute_roots(&tape, [root]).unwrap();
        assert_eq!(output.len(), 3);
        assert_eq!(output[0].captures.len(), 3);
        assert_eq!(output[1].captures.len(), 2);
        assert_eq!(output[2].captures.len(), 1);
    }

    #[test]
    fn batch_kind_dispatch_matches_individual_execution() {
        let tape = tape("execute(); commit(); execute()\n");
        let programs = [identifier_program("execute"), identifier_program("commit")];
        let individual = programs
            .iter()
            .map(|program| program.execute(&tape).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(execute_many(&programs, &tape).unwrap(), individual);
    }

    #[test]
    fn bounded_program_batches_preserve_global_task_and_local_ordinals() {
        let files = [tape("execute(); commit()\n"), tape("commit(); execute()\n")];
        let programs = [identifier_program("execute"), identifier_program("commit")];
        let anchors = files
            .iter()
            .map(|file| {
                programs
                    .iter()
                    .map(|program| {
                        let kind = split(program.words[0]).2;
                        (0..file.transducer_len() as u32)
                            .filter(|&node| file.transducer_kind(node) == kind)
                            .collect()
                    })
                    .collect()
            })
            .collect::<Vec<Vec<Vec<u32>>>>();
        let expected =
            execute_with_backend(ExecutionBackend::CpuJit, &programs, &files, &anchors).unwrap();
        assert_eq!(
            execute_with_backend_bounded_limit(
                ExecutionBackend::CpuJit,
                &programs,
                &files,
                &anchors,
                1,
            )
            .unwrap()
            .into_result()
            .unwrap(),
            expected
        );
    }

    #[test]
    fn batch_runs_many_programs_files_and_exact_anchor_sets() {
        let files = [tape("execute(); commit()\n"), tape("commit(); execute()\n")];
        let programs = [identifier_program("execute"), identifier_program("commit")];
        let unanchored = execute_batch(&programs, &files, None).unwrap();
        assert_eq!(unanchored.len(), 2);
        assert!(
            unanchored
                .iter()
                .all(|file| file.iter().all(|matches| matches.len() == 1))
        );
        let anchors = files
            .iter()
            .map(|file| {
                programs
                    .iter()
                    .map(|program| {
                        let (_, _, kind) = split(program.words[0]);
                        (0..file.transducer_len() as u32)
                            .filter(|&node| file.transducer_kind(node) == kind)
                            .collect()
                    })
                    .collect()
            })
            .collect::<Vec<Vec<Vec<u32>>>>();
        let semantic = |mut batch: Vec<Vec<Vec<Output>>>| {
            for output in batch.iter_mut().flatten().flatten() {
                output.task_ordinal = 0;
                output.local_ordinal = 0;
            }
            batch
        };
        assert_eq!(
            semantic(execute_batch(&programs, &files, Some(&anchors)).unwrap()),
            semantic(unanchored)
        );
        let empty = vec![vec![Vec::new(), Vec::new()], vec![Vec::new(), Vec::new()]];
        assert!(
            execute_batch(&programs, &files, Some(&empty))
                .unwrap()
                .iter()
                .flatten()
                .all(Vec::is_empty)
        );
        let invalid = vec![
            vec![vec![u32::MAX], Vec::new()],
            vec![Vec::new(), Vec::new()],
        ];
        assert_eq!(
            execute_batch(&programs, &files, Some(&invalid)),
            Err(Error::InvalidAnchor)
        );
    }

#[test]
    fn query_independent_fts_feeds_batched_execution_and_scopes_ordinals_to_tapes() {
        let database = std::env::temp_dir().join(format!(
            "tempo-transducer-anchor-{}-{}.sqlite",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let store = crate::content_store::ContentStore::open(&database).unwrap();
        let files = [
            tape("execute(); commit(); execute()\n"),
            tape("commit(); execute()\n"),
        ];
        let programs = [identifier_program("execute"), identifier_program("absent")];
        let source = store.put_source(b"source", &[]).unwrap();
        let tape_ids = [
            store.put_tape(source, 1, 1, 1, b"first").unwrap(),
            store.put_tape(source, 2, 1, 1, b"second").unwrap(),
        ];
        persist_search_index_batch(&store, &tape_ids, &files).unwrap();
        let identifier_kind = split(programs[0].words[0]).2;
        let kind_roots = load_kind_anchor_batch(&store, &tape_ids, &[identifier_kind]).unwrap();
        assert!(
            kind_roots
                .iter()
                .zip(&files)
                .all(|(roots, tape)| roots[0].as_ref().is_some_and(|roots| roots
                    .iter()
                    .all(|&root| tape.transducer_kind(root) == identifier_kind)))
        );
        let loaded = load_literal_anchor_batch(&store, &tape_ids, &programs).unwrap();
        assert!(
            loaded
                .iter()
                .all(|file| file[0].is_none() && file[1] == Some(Vec::new()))
        );
        let anchors = loaded
            .into_iter()
            .zip(&files)
            .map(|(file, tape)| {
                file.into_iter()
                    .map(|roots| {
                        roots.unwrap_or_else(|| (0..tape.transducer_len() as u32).collect())
                    })
                    .collect()
            })
            .collect::<Vec<Vec<Vec<u32>>>>();
        let semantic = |mut batch: Vec<Vec<Vec<Output>>>| {
            for output in batch.iter_mut().flatten().flatten() {
                output.task_ordinal = 0;
                output.local_ordinal = 0;
            }
            batch
        };
        assert_eq!(
            semantic(execute_batch(&programs, &files, Some(&anchors)).unwrap()),
            semantic(execute_batch(&programs, &files, None).unwrap())
        );

        let stale = store
            .put_tape(source, 3, 1, 1, b"changed revision")
            .unwrap();
        assert_eq!(
            load_literal_anchor_batch(&store, &[stale], &programs).unwrap(),
            vec![vec![None, None]]
        );
        drop(store);
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", database.display()));
        }
    }

#[test]
    fn sqlite_virtual_table_drives_batched_matching_and_relational_composition() {
        let files = [
            tape("execute(); commit(); execute()\n"),
            tape("commit(); execute()\n"),
        ];
        let programs = [identifier_program("execute"), identifier_program("absent")];
        let database = std::env::temp_dir().join(format!(
            "tempo-sql-indexed-match-{}-{}.sqlite",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let store = crate::content_store::ContentStore::open(&database).unwrap();
        let source = store.put_source(b"source", &[]).unwrap();
        let tape_ids = [
            store.put_tape(source, 1, 1, 1, b"first").unwrap(),
            store.put_tape(source, 2, 1, 1, b"second").unwrap(),
        ];
        persist_search_index_batch(&store, &tape_ids, &files).unwrap();
        let connection = rusqlite::Connection::open_in_memory().unwrap();
        let registry = Arc::new(sql_backend::MatchRegistry::default());
        sql_backend::register_module(&connection, Arc::clone(&registry)).unwrap();
        let batch = registry
            .register_indexed_batch(&store, &tape_ids, &programs, &files)
            .unwrap();
        connection
            .execute_batch(
                "CREATE TABLE expectations(program_index INTEGER PRIMARY KEY,min_matches INTEGER NOT NULL);\
                 INSERT INTO expectations VALUES(0,2),(1,1);",
            )
            .unwrap();
        let selected = connection
            .prepare(
                "WITH terminal AS MATERIALIZED (SELECT * FROM tempo_matches(?1)) \
                 SELECT e.program_index,count(DISTINCT CASE WHEN t.match_index IS NOT NULL \
                   THEN printf('%d:%d',t.file_index,t.match_index) END) matches \
                 FROM expectations e LEFT JOIN terminal t ON t.program_index=e.program_index \
                   AND t.complete=1 AND t.match_index IS NOT NULL \
                 GROUP BY e.program_index,e.min_matches \
                 HAVING count(DISTINCT CASE WHEN t.match_index IS NOT NULL \
                   THEN printf('%d:%d',t.file_index,t.match_index) END)>=e.min_matches \
                 ORDER BY e.program_index",
            )
            .unwrap()
            .query_map([batch], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(selected, vec![(0, 3)]);
        let negatives: i64 = connection
            .query_row(
                "SELECT count(*) FROM tempo_matches(?1) WHERE program_index=1 \
                 AND complete=1 AND match_index IS NULL AND error_code IS NULL",
                [batch],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(negatives, 2);
        assert_eq!(registry.execution_count(batch), Some(1));
        registry.remove_batch(batch);
        #[cfg(target_os = "macos")]
        {
            let anchors = files
                .iter()
                .map(|file| {
                    programs
                        .iter()
                        .map(|_| (0..file.transducer_len() as u32).collect())
                        .collect()
                })
                .collect::<Vec<Vec<Vec<u32>>>>();
            {
                let metal_batch = registry
                    .register_metal_batch(&programs, &files, &anchors)
                    .unwrap();

                let (matches, failures): (i64, i64) = connection
                    .query_row(
                        "SELECT count(DISTINCT CASE WHEN match_index IS NOT NULL THEN printf('%d:%d:%d',file_index,program_index,match_index) END),count(DISTINCT error_code) FROM tempo_matches(?1)",
                        [metal_batch],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .unwrap();
                assert_eq!(matches, 3);
                assert_eq!(failures, 0);
                assert_eq!(registry.execution_count(metal_batch), Some(1));

                registry.remove_batch(metal_batch);
            }
        }
        drop(store);
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", database.display()));
        }
    }

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    #[test]
    fn cpu_jit_and_metal_anchor_scan_match_anchored_interpreter() {
        let files = [
            tape("execute(); commit(); execute()\n"),
            tape("commit(); execute()\n"),
        ];
        let programs = [identifier_program("execute"), identifier_program("commit")];
        let anchors = files
            .iter()
            .map(|file| {
                programs
                    .iter()
                    .map(|program| {
                        let kind = split(program.words[0]).2;
                        (0..file.transducer_len() as u32)
                            .filter(|&node| file.transducer_kind(node) == kind)
                            .collect()
                    })
                    .collect()
            })
            .collect::<Vec<Vec<Vec<u32>>>>();
        let expected = execute_batch(&programs, &files, Some(&anchors)).unwrap();
        assert_eq!(
            execute_with_backend(ExecutionBackend::CpuJit, &programs, &files, &anchors).unwrap(),
            expected
        );
        let connection = rusqlite::Connection::open_in_memory().unwrap();
        let registry = Arc::new(sql_backend::MatchRegistry::default());
        sql_backend::register_module(&connection, Arc::clone(&registry)).unwrap();
        let expected_matches = expected.iter().flatten().map(Vec::len).sum::<usize>() as i64;
        {
            let batch = registry
                .register_metal_batch(&programs, &files, &anchors)
                .unwrap();
            let matches: i64 = connection
                .query_row(
                    "SELECT count(*) FROM tempo_matches(?1) WHERE match_index IS NOT NULL",
                    [batch],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(matches, expected_matches);
            registry.remove_batch(batch);
        }
        let anchor_scan = metal_backend::prepare_anchor_scan(&programs, &files).unwrap();
        let anchor_pipeline_count = metal_backend::pipeline_cache_len();
        let gpu_anchors = anchor_scan.run().unwrap();
        assert_eq!(anchor_scan.run().unwrap(), gpu_anchors);
        let semantic = |mut batch: Vec<Vec<Vec<Output>>>| {
            for output in batch.iter_mut().flatten().flatten() {
                output.task_ordinal = 0;
                output.local_ordinal = 0;
            }
            batch
        };
        assert_eq!(
            semantic(execute_batch(&programs, &files, Some(&gpu_anchors)).unwrap()),
            semantic(expected.clone())
        );
        assert_eq!(
            semantic(materialize_literal_anchors(&programs, &files, &gpu_anchors).unwrap()),
            semantic(expected)
        );
        let filtered_scan = metal_backend::prepare_anchor_scan_with_eligibility(
            &programs,
            &files,
            Some(&[vec![0b01], vec![0b10]]),
        )
        .unwrap();
        assert_eq!(metal_backend::pipeline_cache_len(), anchor_pipeline_count);
        let filtered = filtered_scan.run().unwrap();
        assert_eq!(filtered[0][0], gpu_anchors[0][0]);
        assert!(filtered[0][1].is_empty());
        assert!(filtered[1][0].is_empty());
        assert_eq!(filtered[1][1], gpu_anchors[1][1]);

        let many_programs = (0..40)
            .map(|_| identifier_program("execute"))
            .collect::<Vec<_>>();
        let many_masks = vec![vec![1, 1 << 7], vec![1, 1 << 7]];
        let many = metal_backend::prepare_anchor_scan_with_eligibility(
            &many_programs,
            &files,
            Some(&many_masks),
        )
        .unwrap()
        .run()
        .unwrap();
        for file in 0..files.len() {
            assert_eq!(many[file][0], gpu_anchors[file][0]);
            assert_eq!(many[file][39], gpu_anchors[file][0]);
            assert!(many[file][1..39].iter().all(Vec::is_empty));
        }
    }

    #[test]
    fn verifier_rejects_invalid_and_non_progressing_programs() {
        assert!(matches!(
            Program::new(vec![word(GOTO, 0, 0)], vec![], vec![], vec![], vec![]),
            Err(Error::NonProgressingCycle(0))
        ));
        assert!(matches!(
            Program::new(
                vec![word(TEXT, 0, 0), word(STOP, 0, 0)],
                vec![],
                vec![],
                vec![],
                vec![]
            ),
            Err(Error::InvalidOperand(0))
        ));
        assert!(matches!(
            Program::new(
                vec![word(CHILD, 2, 0), word(STOP, 0, 0)],
                vec![],
                vec![],
                vec![],
                vec![]
            ),
            Err(Error::InvalidMode(0))
        ));
        assert!(
            Program::new(
                vec![word(FORK, GREEDY, 1), word(STOP, 0, 0)],
                vec![],
                vec![],
                vec![],
                vec![]
            )
            .is_ok()
        );
        assert!(matches!(
            Program::new(
                vec![word(FORK, 2, 1), word(STOP, 0, 0)],
                vec![],
                vec![],
                vec![],
                vec![]
            ),
            Err(Error::InvalidMode(0))
        ));
    }

    fn split_tree_sitter_patterns(text: &str) -> Vec<String> {
        let bytes = text.as_bytes();
        let mut patterns = Vec::new();
        let mut depth = 0_i32;
        let mut start = None;
        let mut index = 0;
        while index < bytes.len() {
            match bytes[index] {
                b';' => {
                    index += 1;
                    while index < bytes.len() && bytes[index] != b'\n' {
                        index += 1;
                    }
                    continue;
                }
                b'"' => {
                    if depth == 0
                        && let Some(start) = start.replace(index)
                    {
                        patterns.push(text[start..index].trim().to_owned());
                    }
                    index += 1;
                    while index < bytes.len() && bytes[index] != b'"' {
                        if bytes[index] == b'\\' {
                            index += 1;
                        }
                        index += 1;
                    }
                }
                b'(' | b'[' => {
                    if depth == 0
                        && let Some(start) = start.replace(index)
                    {
                        patterns.push(text[start..index].trim().to_owned());
                    }
                    depth += 1;
                }
                b')' | b']' => depth -= 1,
                _ => {}
            }
            index += 1;
        }
        if let Some(start) = start {
            patterns.push(text[start..].trim().to_owned());
        }
        patterns.retain(|pattern| !pattern.is_empty());
        patterns
    }

    fn collect_extension_files(root: &Path, extensions: &[&str], output: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(root) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                if path.file_name().is_none_or(|name| name != ".git") {
                    collect_extension_files(&path, extensions, output);
                }
            } else if file_type.is_file()
                && path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| extensions.contains(&extension))
                && fs::metadata(&path).is_ok_and(|metadata| metadata.len() <= 4_000_000)
            {
                output.push(path);
            }
        }
    }

    fn collect_python(root: &Path, output: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(root) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if !matches!(
                    path.file_name().and_then(|s| s.to_str()),
                    Some(".git" | ".venv" | "node_modules" | "__pycache__")
                ) {
                    collect_python(&path, output);
                }
            } else if path.extension().and_then(|s| s.to_str()) == Some("py") {
                output.push(path);
            }
        }
    }

    fn digest_ranges<'a>(ranges: impl IntoIterator<Item = &'a Range<usize>>) -> String {
        let mut digest = Sha256::new();
        for range in ranges {
            digest.update((range.start as u64).to_le_bytes());
            digest.update((range.end as u64).to_le_bytes());
        }
        format!("{:x}", digest.finalize())
    }

    fn median(mut values: Vec<Duration>) -> Duration {
        values.sort_unstable();
        values[values.len() / 2]
    }

    #[test]
    #[ignore = "controlled corpus benchmark"]
    fn benchmark_three_ast_grep_paths() {
        let root = PathBuf::from(
            std::env::var("TEMPO_TRANSDUCER_CORPUS")
                .unwrap_or_else(|_| "../../nera-data-layer".into()),
        );
        let limit = std::env::var("TEMPO_TRANSDUCER_FILES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(usize::MAX);
        let iterations = std::env::var("TEMPO_TRANSDUCER_ITERATIONS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(7usize);
        let names = [
            "execute", "commit", "rollback", "flush", "select", "where", "join", "delete",
        ];
        let patterns = names
            .iter()
            .map(|name| Pattern::try_new(name, SupportLang::Python).unwrap())
            .collect::<Vec<_>>();
        let programs = names
            .iter()
            .map(|name| identifier_program(name))
            .collect::<Vec<_>>();
        let mut paths = Vec::new();
        collect_python(&root, &mut paths);
        paths.sort();
        paths.truncate(limit);
        assert!(
            !paths.is_empty(),
            "benchmark corpus is empty: {}",
            root.display()
        );

        let grammar = Grammar::new(SupportLang::Python).unwrap();
        let preparation = Instant::now();
        let mut native = Vec::with_capacity(paths.len());
        let mut tape_roots = Vec::with_capacity(paths.len());
        let mut tapes = Vec::with_capacity(paths.len());
        let mut source_bytes = 0usize;
        let mut parse_time = Duration::ZERO;
        let mut encode_restore_time = Duration::ZERO;
        for path in &paths {
            let source = fs::read_to_string(path).unwrap();
            source_bytes += source.len();
            let started = Instant::now();
            let doc = StrDoc::new(&source, SupportLang::Python);
            parse_time += started.elapsed();
            let started = Instant::now();
            let image =
                TapeImage::encode(doc.tree.root_node(), &grammar, &source, &|| false).unwrap();
            let tape = TapeDoc::restore(Arc::new(source), Arc::clone(&grammar), &image, &|| false)
                .unwrap();
            encode_restore_time += started.elapsed();
            native.push(AstGrep::doc(doc));
            tape_roots.push(AstGrep::doc(tape.clone()));
            tapes.push(tape);
        }
        let preparation_time = preparation.elapsed();

        let native_run = || {
            let mut ranges = Vec::new();
            for root in &native {
                for pattern in &patterns {
                    ranges.extend(root.root().find_all(pattern).map(|m| m.get_node().range()));
                }
            }
            ranges
        };
        let tape_run = || {
            let mut ranges = Vec::new();
            for root in &tape_roots {
                for pattern in &patterns {
                    ranges.extend(root.root().find_all(pattern).map(|m| m.get_node().range()));
                }
            }
            ranges
        };
        let transducer_run = || {
            let mut ranges = Vec::new();
            for tape in &tapes {
                for output in execute_many(&programs, tape).unwrap() {
                    ranges.extend(output.into_iter().map(|m| m.captures[0].span.clone()));
                }
            }
            ranges
        };

        let expected = native_run();
        let tape_result = tape_run();
        let transducer_result = transducer_run();
        assert_eq!(tape_result, expected, "tape ast-grep parity failed");
        assert_eq!(transducer_result, expected, "transducer parity failed");
        let digest = digest_ranges(&expected);

        let mut native_times = Vec::new();
        let mut tape_times = Vec::new();
        let mut transducer_times = Vec::new();
        for iteration in 0..=iterations {
            let order = if iteration % 2 == 0 {
                [0, 1, 2]
            } else {
                [2, 1, 0]
            };
            for engine in order {
                let started = Instant::now();
                let count = match engine {
                    0 => native_run().len(),
                    1 => tape_run().len(),
                    _ => transducer_run().len(),
                };
                black_box(count);
                let elapsed = started.elapsed();
                if iteration > 0 {
                    match engine {
                        0 => native_times.push(elapsed),
                        1 => tape_times.push(elapsed),
                        _ => transducer_times.push(elapsed),
                    }
                }
            }
        }
        let native_median = median(native_times);
        let tape_median = median(tape_times);
        let transducer_median = median(transducer_times);
        let ms = |duration: Duration| duration.as_secs_f64() * 1000.0;
        let report = json!({
            "benchmark": "tempo-fixed-word-transducer-v1",
            "corpus": fs::canonicalize(&root).unwrap(),
            "files": paths.len(), "sourceBytes": source_bytes, "patterns": names,
            "matches": expected.len(), "orderedRangeSha256": digest,
            "iterations": iterations,
            "preparationMs": ms(preparation_time), "parseMs": ms(parse_time),
            "tapeEncodeRestoreMs": ms(encode_restore_time),
            "medianMs": {
                "astGrepTree": ms(native_median),
                "astGrepTape": ms(tape_median),
                "transducerTape": ms(transducer_median),
            },
            "speedupVsTree": ms(native_median) / ms(transducer_median),
            "speedupVsTapeAstGrep": ms(tape_median) / ms(transducer_median),
        });
        println!("TRANSDUCER_BENCHMARK={report}");
    }

#[test]
    #[ignore = "full ast-index-spike source corpus indexing benchmark"]
    fn benchmark_full_fts_source_corpus() {
        let corpus = PathBuf::from(
            std::env::var("TEMPO_SOURCE_CORPUS")
                .expect("TEMPO_SOURCE_CORPUS must name ast-index-spike bench/corpus/repos"),
        );
        let database = std::env::temp_dir().join(format!(
            "tempo-full-fts-{}-{}.sqlite",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut connection = rusqlite::Connection::open(&database).unwrap();
        connection
            .execute_batch(
                "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA cache_size=-16384; \
                 CREATE TABLE search_index_sets(tape_id INTEGER PRIMARY KEY,index_version INTEGER,node_count INTEGER,complete INTEGER); \
                 CREATE VIRTUAL TABLE search_kind_fts USING fts5(token,content='',contentless_delete=1,detail=full,tokenize='ascii'); \
                 CREATE VIRTUAL TABLE search_value_fts USING fts5(token,content='',contentless_delete=1,detail=none,tokenize='ascii'); \
                 CREATE VIRTUAL TABLE search_text_fts USING fts5(token,content='',contentless_delete=1,detail=none,tokenize='unicode61 remove_diacritics 0');",
            )
            .unwrap();
        let languages: &[(&str, SupportLang, &[&str])] = &[
            ("python", SupportLang::Python, &["py"]),
            (
                "javascript",
                SupportLang::JavaScript,
                &["js", "mjs", "cjs", "jsx"],
            ),
            ("typescript", SupportLang::TypeScript, &["ts"]),
            ("tsx", SupportLang::Tsx, &["tsx"]),
            ("java", SupportLang::Java, &["java"]),
            ("csharp", SupportLang::CSharp, &["cs"]),
            (
                "cpp",
                SupportLang::Cpp,
                &["cc", "cpp", "cxx", "hpp", "hh", "hxx"],
            ),
            ("c", SupportLang::C, &["c", "h"]),
            ("php", SupportLang::Php, &["php"]),
            ("bash", SupportLang::Bash, &["sh", "bash", "zsh"]),
            ("ruby", SupportLang::Ruby, &["rb"]),
            ("go", SupportLang::Go, &["go"]),
            ("rust", SupportLang::Rust, &["rs"]),
            ("kotlin", SupportLang::Kotlin, &["kt", "kts"]),
            ("scala", SupportLang::Scala, &["scala", "sc"]),
            ("swift", SupportLang::Swift, &["swift"]),
            ("html", SupportLang::Html, &["html", "htm"]),
            ("css", SupportLang::Css, &["css"]),
            ("lua", SupportLang::Lua, &["lua"]),
            ("json", SupportLang::Json, &["json"]),
        ];
        let selected_languages = std::env::var("TEMPO_BENCH_LANGS")
            .ok()
            .map(|value| value.split(',').map(str::to_owned).collect::<BTreeSet<_>>());
        let started = Instant::now();
        let mut next_id = 1_i64;
        let mut total_files = 0usize;
        let mut total_bytes = 0usize;
        let mut total_nodes = 0usize;
        let mut failures = Vec::new();
        let mut per_language = serde_json::Map::new();
        for &(name, language, extensions) in languages {
            if selected_languages
                .as_ref()
                .is_some_and(|selected| !selected.contains(name))
            {
                continue;
            }
            let root = if name == "tsx" {
                corpus.join("typescript")
            } else {
                corpus.join(name)
            };
            let mut paths = Vec::new();
            collect_extension_files(&root, extensions, &mut paths);
            paths.sort();
            let grammar = match Grammar::new(language) {
                Ok(grammar) => grammar,
                Err(error) => {
                    failures.push(format!("{name}: {error}"));
                    continue;
                }
            };
            let language_started = Instant::now();
            let mut files = 0usize;
            let mut bytes = 0usize;
            let mut nodes = 0usize;
            for chunk in paths.chunks(64) {
                let transaction = connection.transaction().unwrap();
                for path in chunk {
                    let Ok(source) = fs::read_to_string(path) else {
                        continue;
                    };
                    if source.is_empty() {
                        continue;
                    }
                    let native = StrDoc::new(&source, language);
                    let image =
                        match TapeImage::encode(native.tree.root_node(), &grammar, &source, &|| {
                            false
                        }) {
                            Ok(image) => image,
                            Err(error) => {
                                failures.push(format!("{}: {error}", path.display()));
                                continue;
                            }
                        };
                    let tape =
                        TapeDoc::restore(Arc::new(source), Arc::clone(&grammar), &image, &|| false)
                            .unwrap();
                    let document = search_index_document(
                        crate::content_store::TapeId::for_benchmark(next_id),
                        &tape,
                    );
                    transaction
                        .execute(
                            "INSERT INTO search_kind_fts(rowid,token) VALUES(?1,?2)",
                            rusqlite::params![next_id, document.kind_tokens],
                        )
                        .unwrap();
                    transaction
                        .execute(
                            "INSERT INTO search_value_fts(rowid,token) VALUES(?1,?2)",
                            rusqlite::params![next_id, document.value_tokens],
                        )
                        .unwrap();
                    transaction
                        .execute(
                            "INSERT INTO search_text_fts(rowid,token) VALUES(?1,?2)",
                            rusqlite::params![next_id, document.source_text],
                        )
                        .unwrap();
                    transaction
                        .execute(
                            "INSERT INTO search_index_sets VALUES(?1,?2,?3,1)",
                            rusqlite::params![
                                next_id,
                                POSITIONAL_INDEX_VERSION,
                                document.node_count
                            ],
                        )
                        .unwrap();
                    next_id += 1;
                    files += 1;
                    bytes += tape.transducer_source().len();
                    nodes += tape.transducer_len();
                }
                transaction.commit().unwrap();
            }
            total_files += files;
            total_bytes += bytes;
            total_nodes += nodes;
            per_language.insert(
                name.to_owned(),
                json!({"files":files,"sourceBytes":bytes,"nodes":nodes,
                       "seconds":language_started.elapsed().as_secs_f64()}),
            );
        }
        connection
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
            .unwrap();
        let mut sizes = [0_i64; 4];
        let mut statement = connection
            .prepare(
                "SELECT CASE WHEN name LIKE 'search_kind_fts%' THEN 1 \
                             WHEN name LIKE 'search_value_fts%' THEN 2 \
                             WHEN name LIKE 'search_text_fts%' THEN 3 ELSE 0 END family, \
                        sum(pgsize) FROM dbstat WHERE name LIKE 'search_%' GROUP BY family",
            )
            .unwrap();
        for row in statement
            .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)))
            .unwrap()
        {
            let (family, bytes) = row.unwrap();
            sizes[family as usize] = bytes;
        }
        println!(
            "FULL_FTS_SOURCE_BENCHMARK={}",
            json!({"files":total_files,"sourceBytes":total_bytes,"nodes":total_nodes,
                   "seconds":started.elapsed().as_secs_f64(),"storageBytes":sizes.iter().sum::<i64>(),
                   "storageBytesPerNode":sizes.iter().sum::<i64>() as f64/total_nodes as f64,
                   "storageBytesPerSourceByte":sizes.iter().sum::<i64>() as f64/total_bytes as f64,
                   "storageBreakdownBytes":{"metadata":sizes[0],"kindFts":sizes[1],
                     "valueFts":sizes[2],"sourceWordFts":sizes[3]},
                   "languages":per_language,"failures":failures,
                   "unsupported":["sql: no ast-grep-language grammar"]})
        );
        drop(statement);
        drop(connection);
        for suffix in ["", "-wal", "-shm"] {
            let _ = fs::remove_file(format!("{}{suffix}", database.display()));
        }
    }

    #[test]
    #[ignore = "full ast-index-spike tree-sitter query oracle benchmark"]
    fn benchmark_tree_sitter_query_oracle() {
        use tree_sitter::StreamingIterator;

        let query_root = PathBuf::from(
            std::env::var("TEMPO_QUERY_CORPUS")
                .expect("TEMPO_QUERY_CORPUS must name ast-index-spike bench/corpus/queries"),
        );
        let source_root = PathBuf::from(
            std::env::var("TEMPO_SOURCE_CORPUS")
                .expect("TEMPO_SOURCE_CORPUS must name ast-index-spike bench/corpus/repos"),
        );
        let languages: &[(&str, SupportLang, &[&str])] = &[
            ("python", SupportLang::Python, &["py"]),
            (
                "javascript",
                SupportLang::JavaScript,
                &["js", "mjs", "cjs", "jsx"],
            ),
            ("typescript", SupportLang::TypeScript, &["ts"]),
            ("tsx", SupportLang::Tsx, &["tsx"]),
            ("java", SupportLang::Java, &["java"]),
            ("csharp", SupportLang::CSharp, &["cs"]),
            (
                "cpp",
                SupportLang::Cpp,
                &["cc", "cpp", "cxx", "hpp", "hh", "hxx"],
            ),
            ("c", SupportLang::C, &["c", "h"]),
            ("php", SupportLang::Php, &["php"]),
            ("bash", SupportLang::Bash, &["sh", "bash", "zsh"]),
            ("ruby", SupportLang::Ruby, &["rb"]),
            ("go", SupportLang::Go, &["go"]),
            ("rust", SupportLang::Rust, &["rs"]),
            ("kotlin", SupportLang::Kotlin, &["kt", "kts"]),
            ("scala", SupportLang::Scala, &["scala", "sc"]),
            ("swift", SupportLang::Swift, &["swift"]),
            ("html", SupportLang::Html, &["html", "htm"]),
            ("css", SupportLang::Css, &["css"]),
            ("lua", SupportLang::Lua, &["lua"]),
            ("json", SupportLang::Json, &["json"]),
        ];
        let selected_languages = std::env::var("TEMPO_QUERY_LANGS")
            .ok()
            .map(|value| value.split(',').map(str::to_owned).collect::<BTreeSet<_>>());
        let query_filter = std::env::var("TEMPO_QUERY_FILTER").ok();
        let source_limit = std::env::var("TEMPO_QUERY_SOURCE_BYTES")
            .ok()
            .map(|value| value.parse::<usize>().unwrap())
            .unwrap_or(1_000_000);
        let shard_count = std::env::var("TEMPO_QUERY_SHARD_COUNT")
            .ok()
            .map(|value| value.parse::<usize>().unwrap())
            .unwrap_or(1);
        let shard_index = std::env::var("TEMPO_QUERY_SHARD_INDEX")
            .ok()
            .map(|value| value.parse::<usize>().unwrap())
            .unwrap_or(0);
        assert!(shard_count > 0 && shard_index < shard_count);
        let mut all_micros = Vec::new();
        let mut per_language = serde_json::Map::new();
        let mut total_matches = 0usize;
        let mut deadline_violations = 0usize;
        let mut slow_queries = Vec::new();
        for &(name, language, extensions) in languages {
            if selected_languages
                .as_ref()
                .is_some_and(|selected| !selected.contains(name))
            {
                continue;
            }
            let mut patterns = BTreeSet::new();
            let language_query_root = if name == "tsx" {
                query_root.join("typescript")
            } else {
                query_root.join(name)
            };
            for entry in fs::read_dir(language_query_root).unwrap() {
                let path = entry.unwrap().path();
                if path.extension().is_some_and(|extension| extension == "scm") {
                    patterns.extend(split_tree_sitter_patterns(
                        &fs::read_to_string(path).unwrap(),
                    ));
                }
            }
            let grammar = language.get_ts_language();
            let queries = patterns
                .iter()
                .filter(|pattern| {
                    query_filter
                        .as_ref()
                        .is_none_or(|filter| pattern.contains(filter))
                })
                .filter_map(|pattern| {
                    tree_sitter::Query::new(&grammar, pattern)
                        .ok()
                        .map(|query| (pattern, query))
                })
                .collect::<Vec<_>>();
            let mut paths = Vec::new();
            let language_source_root = if name == "tsx" {
                source_root.join("typescript")
            } else {
                source_root.join(name)
            };
            collect_extension_files(&language_source_root, extensions, &mut paths);
            paths.sort();
            let mut parser = tree_sitter::Parser::new();
            parser.set_language(&grammar).unwrap();
            let mut documents = Vec::new();
            let mut source_bytes = 0usize;
            for (ordinal, path) in paths.into_iter().enumerate() {
                if ordinal % shard_count != shard_index {
                    continue;
                }
                let Ok(source) = fs::read(path) else { continue };
                if source.is_empty() || source_bytes + source.len() > source_limit {
                    continue;
                }
                if let Some(tree) = parser.parse(&source, None) {
                    source_bytes += source.len();
                    documents.push((source, tree));
                }
            }
            let language_started = Instant::now();
            let mut language_matches = 0usize;
            for (pattern, query) in &queries {
                let query_started = Instant::now();
                for (source, tree) in &documents {
                    let mut cursor = tree_sitter::QueryCursor::new();
                    let mut matches = cursor.matches(query, tree.root_node(), source.as_slice());
                    while let Some(found) = matches.next() {
                        language_matches += 1;
                        std::hint::black_box(found.captures().len());
                    }
                }
                let micros = query_started.elapsed().as_micros() as u64;
                if micros > 500_000 {
                    deadline_violations += 1;
                    slow_queries.push(json!({"language":name,"milliseconds":micros as f64/1_000.0,
                                            "pattern":pattern}));
                }
                all_micros.push(micros);
            }
            total_matches += language_matches;
            per_language.insert(
                name.to_owned(),
                json!({"queries":queries.len(),"files":documents.len(),"sourceBytes":source_bytes,
                       "matches":language_matches,"seconds":language_started.elapsed().as_secs_f64()}),
            );
        }
        all_micros.sort_unstable();
        let percentile = |numerator: usize, denominator: usize| {
            all_micros[(all_micros.len() - 1) * numerator / denominator] as f64 / 1_000.0
        };
        println!(
            "TREE_SITTER_QUERY_ORACLE_BENCHMARK={}",
            json!({"queries":all_micros.len(),"matches":total_matches,
                   "medianMs":percentile(1,2),"p95Ms":percentile(95,100),
                   "maxMs":all_micros.last().copied().unwrap_or_default() as f64/1_000.0,
                   "deadlineViolations":deadline_violations,"slowQueries":slow_queries,
                   "languages":per_language,
                   "unsupported":["SQL grammar","Tempo transducer compilation of .scm queries"]})
        );
    }

    #[test]
    #[ignore = "pinned Python SCM execution-signature profile"]
    fn profile_pinned_python_execution_signatures() {
        let query_root = PathBuf::from(
            std::env::var("TEMPO_PYTHON_SCM_QUERY_DIR")
                .expect("TEMPO_PYTHON_SCM_QUERY_DIR must name pinned Python SCM queries"),
        );
        let grammar = SupportLang::Python.get_ts_language();
        let mut paths = fs::read_dir(query_root)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|extension| extension == "scm"))
            .collect::<Vec<_>>();
        paths.sort();
        let mut signatures = BTreeMap::<String, usize>::new();
        let mut compiled_programs = Vec::new();
        let mut programs = 0_usize;
        for path in paths {
            let source = fs::read_to_string(path).unwrap();
            for pattern in Program::tree_sitter_top_level_patterns(&source).unwrap() {
                let program =
                    Program::compile_tree_sitter_query_pattern(&pattern, &grammar).unwrap();
                let opcodes = program
                    .words
                    .iter()
                    .map(|word| split(*word).0)
                    .collect::<Vec<_>>();
                let signature = format!(
                    "ops={opcodes:?};values={};captures={};outputs={}",
                    program.value_programs.len(),
                    program.capture_programs.len(),
                    program.output_program.instructions.len(),
                );
                *signatures.entry(signature).or_default() += 1;
                compiled_programs.push(program);
                programs += 1;
            }
        }
        let mut groups = signatures.into_iter().collect::<Vec<_>>();
        groups.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
        println!(
            "TEMPO_PYTHON_EXECUTION_SIGNATURES {}",
            json!({"programs":programs,"signatures":groups.len(),"groups":groups})
        );
        assert_eq!(programs, 335);
        let grid_shapes = metal_grid::compile_program_shapes(&compiled_programs);
        let grid_signatures = grid_shapes
            .iter()
            .map(|shape| shape.signature)
            .collect::<BTreeSet<_>>();
        let grid_routes = grid_shapes
            .iter()
            .map(|shape| shape.route)
            .collect::<BTreeSet<_>>();
        let route_counts = grid_shapes
            .iter()
            .fold(BTreeMap::new(), |mut counts, shape| {
                *counts.entry(shape.route).or_insert(0_usize) += 1;
                counts
            });
        let structural_maxima = compiled_programs.iter().fold(
            execution_shape::ProgramRequirements::default(),
            |mut maxima, program| {
                let found = execution_shape::program_requirements(program);
                maxima.cursor_slots = maxima.cursor_slots.max(found.cursor_slots);
                maxima.capture_operations = maxima.capture_operations.max(found.capture_operations);
                maxima.child_sites = maxima.child_sites.max(found.child_sites);
                maxima.descendant_sites = maxima.descendant_sites.max(found.descendant_sites);
                maxima.following_sites = maxima.following_sites.max(found.following_sites);
                maxima.repeat_sites = maxima.repeat_sites.max(found.repeat_sites);
                maxima.instruction_count = maxima.instruction_count.max(found.instruction_count);
                maxima
            },
        );
        println!("TEMPO_PYTHON_EXECUTION_ROUTES {route_counts:?}");
        println!("TEMPO_PYTHON_STRUCTURAL_MAXIMA {structural_maxima:?}");
        // The spike counted opcode shapes. Resource classes can split those
        // shapes, but must never merge different opcode/ABI contracts.
        assert_eq!(groups.len(), 106);
        let mut contracts = BTreeMap::new();
        for (program, shape) in compiled_programs.iter().zip(&grid_shapes) {
            let contract = (
                program
                    .words
                    .iter()
                    .map(|word| split(*word).0)
                    .collect::<Vec<_>>(),
                program.value_programs.len(),
                program.capture_programs.len(),
                program.output_program.instructions.len(),
                shape.class,
                shape.route,
            );
            if let Some(previous) = contracts.insert(shape.signature, contract.clone()) {
                assert_eq!(previous, contract, "signature={}", shape.signature);
            }
        }
        println!(
            "TEMPO_PYTHON_GRID_SIGNATURES opcodeShapes={} resourceShapes={}",
            groups.len(),
            grid_signatures.len()
        );
        assert!(grid_routes.len() <= 10);
    }

#[test]
    #[ignore = "full ast-index-spike tree-sitter query corpus audit"]
    fn audit_tree_sitter_query_corpus() {
        let query_root = PathBuf::from(
            std::env::var("TEMPO_QUERY_CORPUS")
                .expect("TEMPO_QUERY_CORPUS must name ast-index-spike bench/corpus/queries"),
        );
        let languages = [
            ("python", SupportLang::Python),
            ("javascript", SupportLang::JavaScript),
            ("typescript", SupportLang::TypeScript),
            ("java", SupportLang::Java),
            ("csharp", SupportLang::CSharp),
            ("cpp", SupportLang::Cpp),
            ("c", SupportLang::C),
            ("php", SupportLang::Php),
            ("bash", SupportLang::Bash),
            ("ruby", SupportLang::Ruby),
            ("go", SupportLang::Go),
            ("rust", SupportLang::Rust),
            ("kotlin", SupportLang::Kotlin),
            ("scala", SupportLang::Scala),
            ("swift", SupportLang::Swift),
            ("html", SupportLang::Html),
            ("css", SupportLang::Css),
            ("lua", SupportLang::Lua),
            ("json", SupportLang::Json),
        ];
        let mut report = serde_json::Map::new();
        let mut corpus = Vec::new();
        let mut total = 0usize;
        for (name, language) in languages {
            let mut patterns = BTreeSet::new();
            for entry in fs::read_dir(query_root.join(name)).unwrap() {
                let path = entry.unwrap().path();
                if path.extension().is_some_and(|extension| extension == "scm") {
                    patterns.extend(split_tree_sitter_patterns(
                        &fs::read_to_string(path).unwrap(),
                    ));
                }
            }
            let grammar = language.get_ts_language();
            let mut valid = 0usize;
            let mut invalid = 0usize;
            let mut lowered = 0usize;
            let mut lowering_errors = BTreeMap::<String, usize>::new();
            for pattern in &patterns {
                match tree_sitter::Query::new(&grammar, pattern) {
                    Ok(query) if query.pattern_count() == 1 => {
                        valid += 1;
                        corpus.push(json!({"lang":name,"text":pattern}));
                        match Program::compile_tree_sitter_query_pattern(pattern, &grammar) {
                            Ok(_) => lowered += 1,
                            Err(error) => {
                                if std::env::var_os("TEMPO_QUERY_AUDIT_ERRORS").is_some() {
                                    eprintln!(
                                        "SCM_LOWERING_ERROR lang={name} error={error:?} pattern={pattern:?}"
                                    );
                                }
                                *lowering_errors.entry(format!("{error:?}")).or_default() += 1;
                            }
                        }
                    }
                    _ => invalid += 1,
                }
            }
            total += valid;
            report.insert(
                name.to_owned(),
                json!({"attempted": patterns.len(), "valid": valid, "invalid": invalid,
                       "lowered": lowered, "loweringErrors": lowering_errors,
                       "grammarSymbols": grammar.node_kind_count(),
                       "grammarFields": grammar.field_count()}),
            );
            if name == "typescript" {
                let tsx = SupportLang::Tsx.get_ts_language();
                let mut valid_tsx = 0usize;
                let mut lowered_tsx = 0usize;
                let mut errors = BTreeMap::<String, usize>::new();
                for pattern in &patterns {
                    if tree_sitter::Query::new(&tsx, pattern).is_err() {
                        continue;
                    }
                    valid_tsx += 1;
                    match Program::compile_tree_sitter_query_pattern(pattern, &tsx) {
                        Ok(_) => lowered_tsx += 1,
                        Err(error) => *errors.entry(format!("{error:?}")).or_default() += 1,
                    }
                }
                report.insert(
                    "tsx".to_owned(),
                    json!({"attempted": patterns.len(), "valid": valid_tsx,
                           "invalid": patterns.len() - valid_tsx, "lowered": lowered_tsx,
                           "loweringErrors": errors,
                           "grammarSymbols": tsx.node_kind_count(),
                           "grammarFields": tsx.field_count()}),
                );
            }
        }
        let mut sql_patterns = BTreeSet::new();
        for entry in fs::read_dir(query_root.join("sql")).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|extension| extension == "scm") {
                sql_patterns.extend(split_tree_sitter_patterns(
                    &fs::read_to_string(path).unwrap(),
                ));
            }
        }
        // G's TapeLanguage::Sql is W's SQL dialect.
        let sql = crate::code_rule_dsl::language::GrammarDialect::Sql.get_ts_language();
        let mut valid_sql = 0usize;
        let mut lowered_sql = 0usize;
        let mut sql_errors = BTreeMap::<String, usize>::new();
        for pattern in &sql_patterns {
            if tree_sitter::Query::new(&sql, pattern).is_err() {
                continue;
            }
            valid_sql += 1;
            match Program::compile_tree_sitter_query_pattern(pattern, &sql) {
                Ok(_) => lowered_sql += 1,
                Err(error) => *sql_errors.entry(format!("{error:?}")).or_default() += 1,
            }
        }
        report.insert(
            "sql".to_owned(),
            json!({"attempted": sql_patterns.len(), "valid": valid_sql,
                   "invalid": sql_patterns.len() - valid_sql, "lowered": lowered_sql,
                   "loweringErrors": sql_errors,
                   "grammarSymbols": sql.node_kind_count(), "grammarFields": sql.field_count()}),
        );
        if let Ok(path) = std::env::var("TEMPO_QUERY_CORPUS_OUTPUT") {
            fs::write(path, serde_json::to_vec_pretty(&corpus).unwrap()).unwrap();
        }
        println!(
            "TREE_SITTER_QUERY_AUDIT={}",
            json!({"validWithoutSql": total, "languages": report})
        );
    }

#[test]
    #[ignore = "controlled corpus FTS storage and lookup benchmark"]
    fn benchmark_fts_search_index() {
        let root = PathBuf::from(
            std::env::var("TEMPO_TRANSDUCER_CORPUS")
                .unwrap_or_else(|_| "../../nera-data-layer".into()),
        );
        let limit = std::env::var("TEMPO_TRANSDUCER_FILES")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(usize::MAX);
        let iterations = std::env::var("TEMPO_TRANSDUCER_ITERATIONS")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(7usize);
        let names = [
            "execute", "commit", "rollback", "flush", "select", "where", "join", "delete",
        ];
        let programs = names
            .iter()
            .map(|name| identifier_program(name))
            .collect::<Vec<_>>();
        let mut paths = Vec::new();
        collect_python(&root, &mut paths);
        paths.sort();
        paths.truncate(limit);
        assert!(!paths.is_empty());
        let database = std::env::temp_dir().join(format!(
            "tempo-fts-benchmark-{}-{}.sqlite",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let store = crate::content_store::ContentStore::open(&database).unwrap();
        let grammar = Grammar::new(SupportLang::Python).unwrap();
        let mut tapes = Vec::with_capacity(paths.len());
        let mut tape_ids = Vec::with_capacity(paths.len());
        let mut source_bytes = 0usize;
        for path in &paths {
            let source = fs::read_to_string(path).unwrap();
            source_bytes += source.len();
            let native = StrDoc::new(&source, SupportLang::Python);
            let image =
                TapeImage::encode(native.tree.root_node(), &grammar, &source, &|| false).unwrap();
            // W's encode yields a FileImage; G's tape bytes are its primary's.
            let tape_bytes = image.primary.to_bytes().unwrap();
            let source_id = store.put_source(source.as_bytes(), &[]).unwrap();
            tape_ids.push(store.put_tape(source_id, 1, 1, 2, &tape_bytes).unwrap());
            tapes.push(
                TapeDoc::restore(Arc::new(source), Arc::clone(&grammar), &image, &|| false)
                    .unwrap(),
            );
        }
        let nodes = tapes.iter().map(TapeDoc::transducer_len).sum::<usize>();
        let started = Instant::now();
        persist_search_index_batch(&store, &tape_ids, &tapes).unwrap();
        let build = started.elapsed();
        let (storage_bytes, storage_parts) = store.search_index_storage_bytes().unwrap();
        let expected = execute_batch(&programs, &tapes, None).unwrap();
        let mut lookup_times = Vec::new();
        let mut selection_times = Vec::new();
        let mut execution_times = Vec::new();
        for _ in 0..iterations {
            let started = Instant::now();
            let loaded = load_literal_anchor_batch(&store, &tape_ids, &programs).unwrap();
            lookup_times.push(started.elapsed());
            let anchors = widen_anchor_candidates(&programs, &tapes, loaded);
            selection_times.push(started.elapsed());
            let started = Instant::now();
            assert_eq!(
                execute_batch(&programs, &tapes, Some(&anchors)).unwrap(),
                expected
            );
            execution_times.push(started.elapsed());
        }
        let started = Instant::now();
        let text_candidates =
            load_text_candidate_batch(&store, &tape_ids, &["TODO".to_owned()]).unwrap();
        let text_lookup = started.elapsed();
        let ms = |duration: Duration| duration.as_secs_f64() * 1000.0;
        #[allow(unused_mut)]
        let mut report = json!({
            "benchmark": "tempo-contentless-fts-search-index-v1",
            "corpus": fs::canonicalize(&root).unwrap(),
            "files": paths.len(),
            "sourceBytes": source_bytes,
            "nodes": nodes,
            "patterns": names,
            "iterations": iterations,
            "buildMs": ms(build),
            "storageBytes": storage_bytes,
            "storageBreakdownBytes": {
                "metadata": storage_parts[0],
                "kindFts": storage_parts[1],
                "valueFts": storage_parts[2],
                "sourceWordFts": storage_parts[3],
            },
            "storageBytesPerNode": storage_bytes as f64 / nodes as f64,
            "storageBytesPerSourceByte": storage_bytes as f64 / source_bytes as f64,
            "medianSqlLookupMs": ms(median(lookup_times)),
            "medianSelectionMs": ms(median(selection_times)),
            "medianTerminalMs": ms(median(execution_times)),
            "textWordLookupMs": ms(text_lookup),
            "textWordCandidateFiles": text_candidates.iter().filter(|value| **value == Some(true)).count(),
        });
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        {
            let started = Instant::now();
            let eligibility = load_literal_eligibility_batch(&store, &tape_ids, &programs).unwrap();
            let masks = eligibility_masks(eligibility, programs.len());
            let sql_and_masks = started.elapsed();
            let started = Instant::now();
            let scan = metal_backend::prepare_anchor_scan_with_eligibility(
                &programs,
                &tapes,
                Some(&masks),
            )
            .unwrap();
            let prepare = started.elapsed();
            let mut fused = Vec::new();
            for _ in 0..iterations {
                let started = Instant::now();
                let anchors = scan.run().unwrap();
                assert_eq!(
                    materialize_literal_anchors(&programs, &tapes, &anchors).unwrap(),
                    expected
                );
                fused.push(started.elapsed());
            }
            report["metal"] = json!({
                "sqlAndEligibilityMasksMs": ms(sql_and_masks),
                "prepareMs": ms(prepare),
                "warmFusedTerminalMedianMs": ms(median(fused)),
            });
        }
        println!("FTS_SEARCH_INDEX_BENCHMARK={report}");
        drop(store);
        for suffix in ["", "-wal", "-shm"] {
            let _ = fs::remove_file(format!("{}{suffix}", database.display()));
        }
    }

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    #[test]
    #[ignore = "Apple Silicon backend benchmark"]
    fn benchmark_anchored_cpu_and_gpu_anchor_scan() {
        let root = PathBuf::from(
            std::env::var("TEMPO_TRANSDUCER_CORPUS")
                .unwrap_or_else(|_| "../../nera-data-layer".into()),
        );
        let limit = std::env::var("TEMPO_TRANSDUCER_FILES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(usize::MAX);
        let iterations = std::env::var("TEMPO_TRANSDUCER_ITERATIONS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(7usize);
        let names = [
            "execute", "commit", "rollback", "flush", "select", "where", "join", "delete",
        ];
        let programs = names
            .iter()
            .map(|name| identifier_program(name))
            .collect::<Vec<_>>();
        let mut paths = Vec::new();
        collect_python(&root, &mut paths);
        paths.sort();
        paths.truncate(limit);
        assert!(!paths.is_empty());
        let grammar = Grammar::new(SupportLang::Python).unwrap();
        let mut tapes = Vec::with_capacity(paths.len());
        let mut source_bytes = 0;
        for path in &paths {
            let source = fs::read_to_string(path).unwrap();
            source_bytes += source.len();
            let native = StrDoc::new(&source, SupportLang::Python);
            let image =
                TapeImage::encode(native.tree.root_node(), &grammar, &source, &|| false).unwrap();
            tapes.push(
                TapeDoc::restore(Arc::new(source), Arc::clone(&grammar), &image, &|| false)
                    .unwrap(),
            );
        }
        let anchor_started = Instant::now();
        let anchors = tapes
            .iter()
            .map(|tape| {
                programs
                    .iter()
                    .map(|program| {
                        let kind = split(program.words[0]).2;
                        let predicate = split(program.words[1]).2;
                        let TextPredicate::Equal(string) =
                            program.predicates[usize::from(predicate)]
                        else {
                            unreachable!()
                        };
                        let expected = &program.strings[usize::from(string)];
                        (0..tape.transducer_len() as u32)
                            .filter(|&node| {
                                tape.transducer_kind(node) == kind
                                    && tape.transducer_text(node) == expected
                            })
                            .collect()
                    })
                    .collect()
            })
            .collect::<Vec<Vec<Vec<u32>>>>();
        let anchor_time = anchor_started.elapsed();
        let started = Instant::now();
        let _cpu_jit = programs
            .iter()
            .map(|program| {
                let native = native_ir::lower(program).unwrap();
                native_cpu::compile_straight_line(&native).unwrap()
            })
            .collect::<Vec<_>>();
        let cpu_compile = started.elapsed();
        let started = Instant::now();
        let gpu_anchor_scan = metal_backend::prepare_anchor_scan(&programs, &tapes).unwrap();
        let gpu_anchor_prepare = started.elapsed();
        let gpu_anchors = gpu_anchor_scan.run().unwrap();
        assert_eq!(
            gpu_anchors, anchors,
            "GPU anchor scan changed preorder postings"
        );
        let unanchored_run = || execute_batch(&programs, &tapes, None).unwrap();
        let interpreted_run = || execute_batch(&programs, &tapes, Some(&anchors)).unwrap();
        let cpu_jit_run =
            || execute_with_backend(ExecutionBackend::CpuJit, &programs, &tapes, &anchors).unwrap();
        let ranges = |result: Vec<Vec<Vec<Output>>>| {
            result
                .into_iter()
                .flatten()
                .flatten()
                .map(|m| m.captures[0].span.clone())
                .collect::<Vec<_>>()
        };
        let expected = ranges(interpreted_run());
        assert_eq!(ranges(unanchored_run()), expected);
        assert_eq!(ranges(cpu_jit_run()), expected);
        let mut times: [Vec<Duration>; 4] = std::array::from_fn(|_| Vec::new());
        for iteration in 0..=iterations {
            for offset in 0..4 {
                let engine = (iteration + offset) % 4;
                let started = Instant::now();
                let count = match engine {
                    0 => ranges(unanchored_run()).len(),
                    1 => ranges(interpreted_run()).len(),
                    2 => ranges(cpu_jit_run()).len(),
                    _ => gpu_anchor_scan
                        .run()
                        .unwrap()
                        .iter()
                        .flatten()
                        .map(Vec::len)
                        .sum(),
                };
                black_box(count);
                if iteration > 0 {
                    times[engine].push(started.elapsed());
                }
            }
        }
        let medians = times.map(median);
        let mut gpu_anchor_dispatch = Vec::new();
        let mut gpu_anchor_reconstruction = Vec::new();
        let mut gpu_anchor_hits = 0;
        for _ in 0..iterations {
            let (profiled, profile) = gpu_anchor_scan.run_profiled().unwrap();
            assert_eq!(profiled, anchors);
            gpu_anchor_dispatch.push(profile.dispatch_and_wait);
            gpu_anchor_reconstruction.push(profile.reconstruction);
            gpu_anchor_hits = profile.hit_count;
        }
        let gpu_anchor_dispatch = median(gpu_anchor_dispatch);
        let gpu_anchor_reconstruction = median(gpu_anchor_reconstruction);
        let mut gpu_fused_terminal = Vec::new();
        for _ in 0..iterations {
            let started = Instant::now();
            let selected = gpu_anchor_scan.run().unwrap();
            let output = materialize_literal_anchors(&programs, &tapes, &selected).unwrap();
            assert_eq!(ranges(output), expected);
            gpu_fused_terminal.push(started.elapsed());
        }
        let gpu_fused_terminal = median(gpu_fused_terminal);
        let ms = |d: Duration| d.as_secs_f64() * 1000.0;
        let report = json!({"benchmark":"tempo-anchored-transducer-and-anchor-scan-v5","corpus":fs::canonicalize(&root).unwrap(),"files":paths.len(),"sourceBytes":source_bytes,"patterns":names,"matches":expected.len(),"orderedRangeSha256":digest_ranges(&expected),"iterations":iterations,"cpuAnchorBuildMs":ms(anchor_time),"prepareMs":{"gpuAnchorScan":ms(gpu_anchor_prepare)},"compileMs":{"cpuJit":ms(cpu_compile)},"medianMs":{"cpuUnanchored":ms(medians[0]),"cpuInterpreted":ms(medians[1]),"cpuJit":ms(medians[2]),"gpuAnchorScanWarm":ms(medians[3])},"gpuAnchorProfile":{"dispatchAndWaitMs":ms(gpu_anchor_dispatch),"reconstructionMs":ms(gpu_anchor_reconstruction),"fusedTerminalMs":ms(gpu_fused_terminal),"sparseHitCount":gpu_anchor_hits},"anchorSpeedup":ms(medians[0])/ms(medians[1]),"speedupVsCpuInterpreted":{"cpuJit":ms(medians[1])/ms(medians[2])}});
        println!("TRANSDUCER_BACKEND_BENCHMARK={report}");
    }
}
