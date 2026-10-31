//! A label program's walk without the interpreter's machinery (O28).
//!
//! Most of a lowered matcher's label programs are short walks: tests of the node, a move along a
//! tree edge, a cursor saved and restored around it, a fork between tests (`matcher::lower`). The
//! interpreter runs each (program, node) as a task with a scheduler entry, quotas and arena blocks,
//! sized for unbounded backtracking those walks never need. A micro walk is the same program run
//! from one start with a few frames on a stack: a frame is a forked path, or a move's remaining
//! candidates (children, descendants, following siblings), so the stack grows with the program's
//! nesting, not with a node's fan-out. On Metal it is one thread per (pass, start), with no task
//! list built on the host (`label_micro` in `metal_label_passes.metal`, the same walk).
//!
//! The walk's outputs are the interpreter's, as a label reads them: a roots label is the starts that
//! emit (the walk stops at the first), a targets label every node captured on a path that emits. A
//! program with an operation outside the walk's (size, assertions, repeats, greedy forks,
//! directives) is not a micro walk. A text test is a fixed predicate of its node, so it is read
//! from bits computed once per (value, node) on the host (`Micro::texts`), not run as a regex. A walk that needs more frames, captures or steps than it has
//! fails, and its pass runs on the interpreter instead. Its steps scale with the file (`steps`).

use super::*;
use std::collections::BTreeSet;
use crate::code_rule_dsl::tape::DERIVED_CLASS_BASE;
use crate::code_rule_dsl::transducer::packed::value::ValueProgram;

/// Cursor slots, captures and frames a walk carries, and the steps it takes.
pub(in crate::code_rule_dsl) const CURSORS: usize = 2;
pub(in crate::code_rule_dsl) const CAPTURES: usize = 2;
pub(in crate::code_rule_dsl) const FRAMES: usize = 128;
pub(in crate::code_rule_dsl) const STEPS: u32 = 1 << 16;

/// The steps a walk of `words` words may take on a file of `nodes` nodes: `STEPS`, or twice a pass
/// over every node per word where that is more. A move's candidates are a file's nodes at most, so
/// a walk with one move from the root takes about that; one with nested moves may take more and
/// then fails (06 S7: a descent from a 25 K-node module took more than `STEPS`).
pub(in crate::code_rule_dsl) fn steps(nodes: usize, words: usize) -> u32 {
    u32::try_from(nodes.saturating_mul(words).saturating_mul(2)).unwrap_or(u32::MAX).max(STEPS)
}

const NONE: u32 = u32::MAX;

/// A walk outgrew its frames, captures or steps, or read a cursor it never saved.
#[derive(Debug, PartialEq, Eq)]
pub(in crate::code_rule_dsl) struct Overflow(pub(in crate::code_rule_dsl) &'static str);

#[derive(Clone, Debug)]
pub(in crate::code_rule_dsl) struct Micro {
    /// The program's words, each `TEXT`'s operand its slot in `texts`.
    pub(in crate::code_rule_dsl) words: Vec<u32>,
    /// Each text test's value (an index into the program's values) and the kind its node has, where
    /// the test follows a kind test that no jump skips (its domain).
    pub(in crate::code_rule_dsl) texts: Vec<(u16, Option<u16>)>,
}

#[derive(Clone, Copy)]
struct State {
    pc: u16,
    node: u32,
    cursors: [u32; CURSORS],
    captures: [u32; CAPTURES],
    captured: u8,
}

/// What a frame resumes: a forked path, or a move's next candidate.
#[derive(Clone, Copy)]
enum Resume {
    Path,
    Children { next: u32, named: bool },
    Descend { next: u32, end: u32, named: bool },
    Following { next: u32, named: bool },
}

/// A walk's frames, kept between walks so a sweep allocates them once.
#[derive(Default)]
pub(in crate::code_rule_dsl) struct Frames(Vec<(State, Resume)>);

impl Micro {
    /// Why `program` is not a micro walk: its first operation outside the walk's, by name.
    pub(in crate::code_rule_dsl) fn refusal(program: &Program) -> &'static str {
        if program.tree_sitter_query || !program.directives.is_empty() || !program.output_program.instructions.is_empty() {
            return "query, directives or output";
        }
        for &word in &program.words {
            let (op, mode, arg) = split(word);
            let name = match op {
                TEXT => match program.value_programs.get(usize::from(arg)) {
                    Some(ValueProgram::Equal { .. }) => "text equal",
                    Some(ValueProgram::Prefix { .. }) => "text prefix",
                    Some(ValueProgram::Regex(_)) => "text regex",
                    _ => "text lua",
                },
                SIZE => "size",
                ASSERT => "assert",
                FIRST | LAST => "first or last",
                ABSENT_FIELD => "absent field",
                REPEAT_BEGIN | REPEAT_BUMP | REPEAT_END => "repeat",
                FORK if mode != 0 => "greedy fork",
                KIND | FIELD | SUPERTYPE if mode != 0 => "test mode",
                _ => continue,
            };
            return name;
        }
        "other"
    }

    /// `program` as a micro walk, where every operation is one.
    pub(in crate::code_rule_dsl) fn of(program: &Program) -> Option<Self> {
        if program.tree_sitter_query
            || !program.directives.is_empty()
            || !program.output_program.instructions.is_empty()
            || program.words.is_empty()
            || program.words.len() > usize::from(u16::MAX)
        {
            return None;
        }
        let len = program.words.len();
        let walks = program.words.iter().all(|&word| {
            let (op, mode, arg) = split(word);
            match op {
                STOP | CAPTURE | EMIT | PARENT | FLAGS => true,
                KIND | FIELD | SUPERTYPE => mode == 0,
                CHILD | DESCEND | NEXT | FOLLOWING => mode == 0 || mode == NAMED,
                SAVE_CURSOR | RESTORE_CURSOR => usize::from(arg) < CURSORS,
                TEXT => mode == 0 && usize::from(arg) < program.value_programs.len(),
                FORK => mode == 0 && usize::from(arg) < len,
                GOTO => usize::from(arg) < len,
                _ => false,
            }
        });
        if !walks {
            return None;
        }
        // A text test reads a fixed predicate of its node: a slot of bits (`texts`), not a regex.
        let targets = program
            .words
            .iter()
            .filter_map(|&word| match split(word) {
                (FORK | GOTO, _, arg) => Some(arg),
                _ => None,
            })
            .collect::<BTreeSet<_>>();
        let (mut words, mut texts) = (program.words.clone(), Vec::<(u16, Option<u16>)>::new());
        for pc in 0..words.len() {
            let (op, mode, value) = split(words[pc]);
            if op != TEXT {
                continue;
            }
            let kind = (pc > 0 && !targets.contains(&(pc as u16)))
                .then(|| split(words[pc - 1]))
                .and_then(|(op, mode, arg)| (op == KIND && mode == 0).then_some(arg));
            let slot = texts.iter().position(|&text| text == (value, kind)).unwrap_or_else(|| {
                texts.push((value, kind));
                texts.len() - 1
            });
            words[pc] = u32::from(op) | u32::from(mode) << 8 | (slot as u32) << 16;
        }
        Some(Self { words, texts })
    }

    /// Each text slot's bits on `tape` (bit `n` of slot `s`: node `n` passes text test `s`), by the
    /// program's own value programs; `None` where one fails (its work limit), so the program runs
    /// on the interpreter.
    pub(in crate::code_rule_dsl) fn texts(&self, program: &Program, tape: &TapeDoc) -> Option<Vec<Vec<u64>>> {
        (0..self.texts.len()).map(|slot| self.slot(program, slot, tape)).collect()
    }

    /// Slot `slot`'s bits on `tape` (`texts`).
    pub(in crate::code_rule_dsl) fn slot(&self, program: &Program, slot: usize, tape: &TapeDoc) -> Option<Vec<u64>> {
        let (value, kind) = self.texts[slot];
        let len = tape.transducer_len();
        let mut bits = vec![0_u64; len.div_ceil(64)];
        for node in 0..len as u32 {
            if kind.is_some_and(|kind| tape.transducer_kind(node) != kind) {
                continue;
            }
            let mut work = 0;
            if program.value_programs[usize::from(value)].is_match(tape.transducer_text(node), &program.strings, &mut work).ok()? {
                bits[node as usize / 64] |= 1 << (node % 64);
            }
        }
        Some(bits)
    }

    /// What slot `slot`'s bits depend on, as a key: its value program (with the strings it reads)
    /// and its kind. Passes whose slots share a key share their bits.
    pub(in crate::code_rule_dsl) fn slot_key(&self, program: &Program, slot: usize) -> String {
        let (value, kind) = self.texts[slot];
        let value = &program.value_programs[usize::from(value)];
        let strings = match value {
            ValueProgram::Equal { string } | ValueProgram::Prefix { string } => vec![program.strings[usize::from(*string)].clone()],
            ValueProgram::Regex(_) => Vec::new(),
            _ => program.strings.clone(),
        };
        format!("{value:?}|{strings:?}|{kind:?}")
    }

    /// The walk from `start` on `tape`, whose labels (classes from `DERIVED_CLASS_BASE`) are
    /// `labels` and whose text slots are `texts` (`Micro::texts`): `mark` takes each output's start (`targets` false), or each node an output
    /// captured.
    pub(in crate::code_rule_dsl) fn run(
        &self,
        tape: &TapeDoc,
        labels: &[Vec<u64>],
        texts: &[Vec<u64>],
        start: u32,
        targets: bool,
        frames: &mut Frames,
        mark: &mut impl FnMut(u32),
    ) -> Result<(), Overflow> {
        let stack = &mut frames.0;
        stack.clear();
        let has = |node: u32, class: u16| {
            if class >= DERIVED_CLASS_BASE {
                labels
                    .get(usize::from(class - DERIVED_CLASS_BASE))
                    .and_then(|bits| bits.get(node as usize / 64))
                    .is_some_and(|word| word >> (node % 64) & 1 != 0)
            } else {
                tape.transducer_has_supertype(node, class)
            }
        };
        let mut state = State { pc: 0, node: start, cursors: [NONE; CURSORS], captures: [NONE; CAPTURES], captured: 0 };
        let mut steps = 0_u32;
        let limit = self::steps(tape.transducer_len() as usize, self.words.len());
        loop {
            // The path from `state` until it ends.
            loop {
                steps += 1;
                if steps > limit {
                    return Err(Overflow("steps"));
                }
                let (op, mode, arg) = split(self.words[usize::from(state.pc)]);
                let named = mode == NAMED;
                let alive = match op {
                    STOP => false,
                    KIND => tape.transducer_kind(state.node) == arg,
                    FIELD => tape.transducer_field(state.node) == arg,
                    FLAGS => {
                        let flags = tape.transducer_flags(state.node);
                        flags & mode == mode && flags & arg as u8 == 0
                    }
                    SUPERTYPE => has(state.node, arg),
                    TEXT => texts[usize::from(arg)][state.node as usize / 64] >> (state.node % 64) & 1 != 0,
                    PARENT => tape.transducer_parent(state.node).map(|parent| state.node = parent).is_some(),
                    NEXT => tape.transducer_next_sibling(state.node, named).map(|next| state.node = next).is_some(),
                    CHILD | DESCEND | FOLLOWING => {
                        let mut resume = match op {
                            CHILD => Resume::Children { next: tape.transducer_children(state.node).next().unwrap_or(NONE), named },
                            DESCEND => {
                                let range = tape.transducer_descendants(state.node);
                                Resume::Descend { next: range.start, end: range.end, named }
                            }
                            _ => Resume::Following { next: tape.transducer_next_sibling(state.node, named).unwrap_or(NONE), named },
                        };
                        match candidate(tape, &mut resume) {
                            Some(node) => {
                                if stack.len() == FRAMES {
                                    return Err(Overflow("frames"));
                                }
                                stack.push((State { pc: state.pc + 1, ..state }, resume));
                                state.node = node;
                                true
                            }
                            None => false,
                        }
                    }
                    SAVE_CURSOR => {
                        state.cursors[usize::from(arg)] = state.node;
                        true
                    }
                    RESTORE_CURSOR => match state.cursors[usize::from(arg)] {
                        NONE => return Err(Overflow("cursor")),
                        node => {
                            state.node = node;
                            true
                        }
                    },
                    CAPTURE => {
                        let at = usize::from(state.captured);
                        if at == CAPTURES {
                            return Err(Overflow("captures"));
                        }
                        state.captures[at] = state.node;
                        state.captured += 1;
                        true
                    }
                    FORK => {
                        if stack.len() == FRAMES {
                            return Err(Overflow("frames"));
                        }
                        stack.push((State { pc: arg, ..state }, Resume::Path));
                        true
                    }
                    GOTO => {
                        state.pc = arg;
                        continue;
                    }
                    EMIT => {
                        if !targets {
                            mark(start);
                            return Ok(());
                        }
                        state.captures[..usize::from(state.captured)].iter().for_each(|&node| mark(node));
                        false
                    }
                    _ => return Err(Overflow("operation")),
                };
                if !alive {
                    break;
                }
                state.pc += 1;
                if usize::from(state.pc) >= self.words.len() {
                    return Err(Overflow("end"));
                }
            }
            // The next frame with a path left.
            loop {
                let Some((frame, mut resume)) = stack.pop() else {
                    return Ok(());
                };
                if let Resume::Path = resume {
                    state = frame;
                    break;
                }
                if let Some(node) = candidate(tape, &mut resume) {
                    stack.push((frame, resume));
                    state = State { node, ..frame };
                    break;
                }
            }
        }
    }
}

/// A move's next candidate, advancing past it.
fn candidate(tape: &TapeDoc, resume: &mut Resume) -> Option<u32> {
    let named_ok = |node: u32, named: bool| !named || tape.transducer_flags(node) & 1 != 0;
    match resume {
        Resume::Path => None,
        Resume::Children { next, named } => {
            while *next != NONE {
                let node = *next;
                *next = tape.transducer_next_sibling(node, false).unwrap_or(NONE);
                if named_ok(node, *named) {
                    return Some(node);
                }
            }
            None
        }
        Resume::Descend { next, end, named } => {
            while *next < *end {
                let node = *next;
                *next += 1;
                if named_ok(node, *named) {
                    return Some(node);
                }
            }
            None
        }
        Resume::Following { next, named } => {
            let node = *next;
            (node != NONE).then(|| {
                *next = tape.transducer_next_sibling(node, *named).unwrap_or(NONE);
                node
            })
        }
    }
}
