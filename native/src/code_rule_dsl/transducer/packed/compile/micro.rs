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
//! from bits: the term index's row (`prefilter::terms`) or one pass over the file by the text layer
//! (`text::scan`, decision 0042), never a regex run per node. A walk that needs more frames, captures or steps than it has
//! fails, and its pass runs on the interpreter instead. Its steps scale with the file (`steps`).

use super::*;
use std::collections::BTreeSet;
use crate::code_rule_dsl::tape::DERIVED_CLASS_BASE;
use crate::code_rule_dsl::transducer::packed::value::ValueProgram;

/// Cursor slots, captures and frames a walk carries, and the steps it takes.
pub(in crate::code_rule_dsl) const CURSORS: usize = 8;
pub(in crate::code_rule_dsl) const CAPTURES: usize = 2;
/// Captures a root walk's path carries (`Micro::records`): a root program's metavariables.
pub(in crate::code_rule_dsl) const ROOT_CAPTURES: usize = 8;
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
    /// Set mode's reach (`set_mode::Reach`): a walk emits each label once and drops a frame that
    /// can emit no label not yet fired; None for an exact program.
    pub(in crate::code_rule_dsl) reach: Option<super::set_mode::Reach>,
}

#[derive(Clone, Copy)]
struct State {
    pc: u16,
    node: u32,
    cursors: [u32; CURSORS],
    captures: [u32; ROOT_CAPTURES],
    /// Each capture's label (`CAPTURE`'s operand).
    labels: [u16; ROOT_CAPTURES],
    captured: u8,
}

/// A root walk's path that emits (decision 0024 §3): its start, the emit's output label, and its
/// captures as (capture label, node), in capture order. Integers only: spans and text are the
/// tape's, read where a row needs them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::code_rule_dsl) struct RootRecord {
    pub(in crate::code_rule_dsl) start: u32,
    pub(in crate::code_rule_dsl) label: u16,
    pub(in crate::code_rule_dsl) captured: u8,
    pub(in crate::code_rule_dsl) captures: [(u16, u32); ROOT_CAPTURES],
}

impl RootRecord {
    pub(in crate::code_rule_dsl) fn captures(&self) -> &[(u16, u32)] {
        &self.captures[..usize::from(self.captured)]
    }
}

/// What a frame resumes: a forked path, or a move's next candidate.
#[derive(Clone, Copy)]
enum Resume {
    Path,
    Children { next: u32, named: bool },
    /// `next..end`, then `then` (a walk's tokens below the node, `TapeDoc::walking_tokens`).
    Descend { next: u32, end: u32, then: [u32; 2], named: bool },
    Following { next: u32, named: bool },
}

/// What a walk marks at each path that emits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::code_rule_dsl) enum Emit {
    /// The start, once: a roots label (the walk stops at the first).
    Start,
    /// The start, at every path: a count by root.
    EveryStart,
    /// Each node the path captured: a targets label, or a count by captured node.
    Captures,
}

impl Emit {
    /// A pass's mode as `metal_label_passes` routes it: 0 roots, 1 captured, 2 and 3 counts.
    pub(in crate::code_rule_dsl) fn of_mode(mode: u32) -> Self {
        match mode {
            0 => Self::Start,
            2 => Self::EveryStart,
            _ => Self::Captures,
        }
    }
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
                // `of` takes a plain text test as a slot, whatever its value program.
                TEXT if mode == 0 && usize::from(arg) < program.value_programs.len() => continue,
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
                SAVE_CURSOR | RESTORE_CURSOR if usize::from(arg) >= CURSORS => "cursors",
                CHILD | DESCEND | NEXT | FOLLOWING if mode != 0 && mode != NAMED => "axis mode",
                STOP | CAPTURE | EMIT | PARENT | FLAGS | KIND | FIELD | SUPERTYPE | CHILD | DESCEND | NEXT | FOLLOWING
                | SAVE_CURSOR | RESTORE_CURSOR | FORK | GOTO => continue,
                _ => "op",
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
        let reach = if program.set_mode { super::set_mode::Reach::of(&words) } else { None };
        Some(Self { words, texts, reach })
    }

    /// Whether a walk ends at its first emit: set mode with one label (the device's root walks
    /// read this, `metal_label_passes`).
    pub(in crate::code_rule_dsl) fn key_stop(&self) -> bool {
        self.reach.as_ref().is_some_and(|reach| reach.labels().len() == 1)
    }

    /// The walk's words as (op, mode, arg), for diagnostics.
    pub(in crate::code_rule_dsl) fn ops(&self) -> Vec<(u8, u8, u16)> {
        self.words.iter().map(|&word| split(word)).collect()
    }

    /// The most frames a walk of this program holds at once. The frames on the stack were pushed
    /// at increasing words: a path goes back only by a jump whose loop holds no pushing word (an
    /// axis step or a fork), so it never returns before a word that pushed. Each pushing word
    /// then holds at most one frame. `None` where a loop pushes.
    pub(in crate::code_rule_dsl) fn depth(&self) -> Option<usize> {
        let pushes = |word: u32| matches!(split(word).0, FORK | CHILD | DESCEND | FOLLOWING);
        for (pc, &word) in self.words.iter().enumerate() {
            let (op, _, arg) = split(word);
            if matches!(op, FORK | GOTO) && usize::from(arg) <= pc && self.words[usize::from(arg)..=pc].iter().any(|&word| pushes(word)) {
                return None;
            }
        }
        Some(self.words.iter().filter(|&&word| pushes(word)).count())
    }

    /// Each text slot's bits on `tape` (bit `n` of slot `s`: node `n` passes text test `s`), by the
    /// program's own value programs; `None` where one fails (its work limit), so the program runs
    /// on the interpreter.
    pub(in crate::code_rule_dsl) fn texts(&self, program: &Program, tape: &TapeDoc) -> Option<Vec<Vec<u64>>> {
        (0..self.texts.len()).map(|slot| self.slot(program, slot, tape)).collect()
    }

    /// Slot `slot`'s bits on `tape` (`texts`): the term index's row where the tape has it
    /// (`R[tid]`, `prefilter::terms`), else each node's text tested.
    pub(in crate::code_rule_dsl) fn slot(&self, program: &Program, slot: usize, tape: &TapeDoc) -> Option<Vec<u64>> {
        // A walk over tokens (`TapeDoc::walking_tokens`): the index's row covers the records, and
        // only the tokens' texts are tested.
        if tape.walks_tokens()
            && let Some(row) = tape.record_text_row(&self.slot_key(program, slot))
        {
            crate::code_rule_dsl::prof::count("texts.indexed", 1);
            let mut bits = self.scan_from(program, slot, tape, tape.record_count() as u32)?;
            for (word, &records) in bits.iter_mut().zip(row.iter()) {
                *word |= records;
            }
            return Some(bits);
        }
        if let Some(row) = tape.transducer_text_row(&self.slot_key(program, slot)) {
            crate::code_rule_dsl::prof::count("texts.indexed", 1);
            // `RERAIL_TEXTS_CHECK`: the index's row against the text's.
            if std::env::var_os("RERAIL_TEXTS_CHECK").is_some() {
                let scanned = self.scan(program, slot, tape);
                if scanned.as_deref() != Some(&row[..]) {
                    crate::code_rule_dsl::prof::count("texts.check_differ", 1);
                    eprintln!("TEXTS differ: {}", self.slot_key(program, slot));
                } else {
                    crate::code_rule_dsl::prof::count("texts.check_same", 1);
                }
            }
            return Some(row.to_vec());
        }
        crate::code_rule_dsl::prof::count("texts.scanned", 1);
        self.scan(program, slot, tape)
    }

    /// Whether node `node`'s text passes slot `slot`'s test (its kind aside).
    pub(in crate::code_rule_dsl) fn test(&self, program: &Program, slot: usize, tape: &TapeDoc, node: u32) -> Option<bool> {
        if let Some(lowered) = self.lowered(program, slot) {
            return lowered.compiled().is_match(tape.transducer_text(node));
        }
        let (value, _) = self.texts[slot];
        let mut work = 0;
        program.value_programs[usize::from(value)].is_match(tape.transducer_text(node), &program.strings, &mut work).ok()
    }

    /// Slot `slot`'s test in the text layer (decision 0042); `None` for a Lua pattern, which keeps
    /// its value program.
    pub(in crate::code_rule_dsl) fn lowered(&self, program: &Program, slot: usize) -> Option<std::sync::Arc<crate::code_rule_dsl::text::Lowered>> {
        use crate::code_rule_dsl::text::{Domain, Source, Spelling, lower};
        let (value, _) = self.texts[slot];
        let string = |index: u16| String::from_utf8(program.strings.get(usize::from(index))?.clone()).ok();
        let source = match &program.predicates[usize::from(value)] {
            TextPredicate::Equal(index) => Source::of(Spelling::Equal(string(*index)?)),
            TextPredicate::Prefix(index) => Source::of(Spelling::Prefix(string(*index)?)),
            TextPredicate::Regex { source, .. } => Source::regex(string(*source)?),
            TextPredicate::LuaPattern { .. } => return None,
        };
        lower(&source, Domain::ANY).ok()
    }

    /// Slot `slot`'s bits on `tape`, from one pass over the file's text (`text::scan`), not each
    /// node's; a Lua pattern on each node's text. A lowered test's bits cover every node, whatever
    /// the slot's kind: they are keyed by plan alone (`slot_key`).
    fn scan(&self, program: &Program, slot: usize, tape: &TapeDoc) -> Option<Vec<u64>> {
        self.scan_from(program, slot, tape, 0)
    }

    /// [`Self::scan`] of the nodes from `from` on: the others' bits are clear.
    fn scan_from(&self, program: &Program, slot: usize, tape: &TapeDoc, from: u32) -> Option<Vec<u64>> {
        let (value, kind) = self.texts[slot];
        let len = tape.transducer_len();
        if let Some(lowered) = self.lowered(program, slot) {
            let candidates = (from..len as u32).collect::<Vec<_>>();
            let span = |node: u32| tape.transducer_span(node);
            let nodes = crate::code_rule_dsl::text::scan::Nodes { text: tape.transducer_source(), len, candidates: &candidates, span: &span };
            return crate::code_rule_dsl::text::scan::bits(&lowered, &nodes);
        }
        let candidates = (from..len as u32).filter(|&node| kind.is_none_or(|kind| tape.transducer_kind(node) == kind));
        let mut bits = vec![0_u64; len.div_ceil(64)];
        let value_program = &program.value_programs[usize::from(value)];
        for node in candidates {
            let mut work = 0;
            if value_program.is_match(tape.transducer_text(node), &program.strings, &mut work).ok()? {
                bits[node as usize / 64] |= 1 << (node % 64);
            }
        }
        Some(bits)
    }

    /// What slot `slot`'s bits depend on, as a key. Passes whose slots share a key share their bits.
    /// A lowered test's key is its plan's (decision 0042: equal however spelt), with no kind: its
    /// bits cover every node, and a walk reads them only after the slot's kind test, so one row
    /// serves every walk with the test (a fused walk's slots, whose kind is unknown, included).
    /// Another test's key is its value program, the strings it reads and its kind.
    pub(in crate::code_rule_dsl) fn slot_key(&self, program: &Program, slot: usize) -> String {
        let (value, kind) = self.texts[slot];
        if let Some(lowered) = self.lowered(program, slot) {
            return lowered.key.0.clone();
        }
        let value = &program.value_programs[usize::from(value)];
        let strings = match value {
            ValueProgram::Equal { string } | ValueProgram::Prefix { string } => vec![program.strings[usize::from(*string)].clone()],
            ValueProgram::Regex(_) => Vec::new(),
            _ => program.strings.clone(),
        };
        format!("{value:?}|{strings:?}|{kind:?}")
    }

    /// The walk from `start` on `tape`, whose labels (classes from `DERIVED_CLASS_BASE`) are
    /// `labels` and whose text slots are `texts` (`Micro::texts`): `mark` takes what `emit` says of
    /// each output (a count tallies one call a mark).
    pub(in crate::code_rule_dsl) fn run(
        &self,
        tape: &TapeDoc,
        labels: &[Vec<u64>],
        texts: &[Vec<u64>],
        start: u32,
        emit: Emit,
        frames: &mut Frames,
        mark: &mut impl FnMut(u32),
    ) -> Result<(), Overflow> {
        self.walk(tape, labels, texts, start, CAPTURES, frames, &mut |state: &State, _| match emit {
            Emit::Start => {
                mark(start);
                true
            }
            Emit::EveryStart => {
                mark(start);
                false
            }
            Emit::Captures => {
                state.captures[..usize::from(state.captured)].iter().for_each(|&node| mark(node));
                false
            }
        })
    }

    /// A fused program's walk (`fusion::Fused`) from `start`: `mark` called with each emit's label
    /// (its member) and node, as `run` marks them. A start's walk does not end at its first emit,
    /// which answers one member: in set mode each member's later emits are skipped instead.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::code_rule_dsl) fn run_labelled(
        &self,
        tape: &TapeDoc,
        labels: &[Vec<u64>],
        texts: &[Vec<u64>],
        start: u32,
        emit: Emit,
        frames: &mut Frames,
        mark: &mut impl FnMut(u16, u32),
    ) -> Result<(), Overflow> {
        self.walk(tape, labels, texts, start, CAPTURES, frames, &mut |state: &State, label| {
            match emit {
                Emit::Start | Emit::EveryStart => mark(label, start),
                Emit::Captures => state.captures[..usize::from(state.captured)].iter().for_each(|&node| mark(label, node)),
            }
            false
        })
    }

    /// A root program's walk from `start` on `tape` (labelled: `transducer_labels`), every path
    /// that emits appended to `out` as a record, in the walk's order.
    pub(in crate::code_rule_dsl) fn records(
        &self,
        tape: &TapeDoc,
        texts: &[Vec<u64>],
        start: u32,
        frames: &mut Frames,
        out: &mut Vec<RootRecord>,
    ) -> Result<(), Overflow> {
        self.walk(tape, tape.transducer_labels(), texts, start, ROOT_CAPTURES, frames, &mut |state: &State, label| {
            let mut captures = [(0_u16, NONE); ROOT_CAPTURES];
            for at in 0..usize::from(state.captured) {
                captures[at] = (state.labels[at], state.captures[at]);
            }
            out.push(RootRecord { start, label, captured: state.captured, captures });
            false
        })
    }

    /// The walk from `start`, `emitted` called at each path that emits with the emit's label
    /// (`true`: the walk ends there); a path holds at most `most` captures.
    #[allow(clippy::too_many_arguments)]
    fn walk(
        &self,
        tape: &TapeDoc,
        labels: &[Vec<u64>],
        texts: &[Vec<u64>],
        start: u32,
        most: usize,
        frames: &mut Frames,
        emitted: &mut impl FnMut(&State, u16) -> bool,
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
        let mut state = State {
            pc: 0,
            node: start,
            cursors: [NONE; CURSORS],
            captures: [NONE; ROOT_CAPTURES],
            labels: [0; ROOT_CAPTURES],
            captured: 0,
        };
        let mut steps = 0_u32;
        let mut fired = 0_u64;
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
                                let tokens = tape.transducer_descendant_tokens(state.node);
                                Resume::Descend { next: range.start, end: range.end, then: [tokens.start, tokens.end], named }
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
                        if at == most {
                            return Err(Overflow("captures"));
                        }
                        state.captures[at] = state.node;
                        state.labels[at] = arg;
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
                    EMIT => match &self.reach {
                        // Set mode: a label once; the walk ends once every label fired.
                        Some(reach) if fired & reach.bit(arg) != 0 => false,
                        Some(reach) => {
                            fired |= reach.bit(arg);
                            if emitted(&state, arg) || fired == reach.all() {
                                return Ok(());
                            }
                            false
                        }
                        None => {
                            if emitted(&state, arg) {
                                return Ok(());
                            }
                            false
                        }
                    },
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
                if let Some(reach) = &self.reach
                    && !reach.live(frame.pc, fired)
                {
                    continue;
                }
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
        Resume::Descend { next, end, then, named } => loop {
            while *next < *end {
                let node = *next;
                *next += 1;
                if named_ok(node, *named) {
                    return Some(node);
                }
            }
            if then[0] >= then[1] {
                return None;
            }
            [*next, *end] = *then;
            *then = [0, 0];
        },
        Resume::Following { next, named } => {
            let node = *next;
            (node != NONE).then(|| {
                *next = tape.transducer_next_sibling(node, *named).unwrap_or(NONE);
                node
            })
        }
    }
}
