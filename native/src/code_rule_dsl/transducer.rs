#[cfg(any(test, feature = "test-hooks"))]
use super::tape::TapeDoc;
#[cfg(any(test, feature = "test-hooks"))]
use ast_grep_core::MatchStrictness;
#[cfg(any(test, feature = "test-hooks"))]
use ast_grep_core::matcher::{Pattern, PatternNode};
#[cfg(any(test, feature = "test-hooks"))]
use regex::Regex;
#[cfg(any(test, feature = "test-hooks"))]
use std::collections::{BTreeMap, BTreeSet};
#[cfg(any(test, feature = "test-hooks"))]
use std::ops::Range;

// The packed runtime (05 Phase 4, from ner-1562-scale-test-the-re-railer): every backend over one
// program. Nothing outside its tests calls it until the demand vtab (Phase 5) drives it.
#[allow(dead_code)]
pub mod packed;
// N's semantic fact rows, which the matcher's families emit for N's catalog backend (tests).
pub use rerail_w::code_rule_dsl::transducer::semantic_facts;

#[cfg(any(test, feature = "test-hooks"))]
pub const STOP: u8 = 0x00;
#[cfg(any(test, feature = "test-hooks"))]
pub const KIND: u8 = 0x01;
#[cfg(any(test, feature = "test-hooks"))]
pub const FIELD: u8 = 0x02;
#[cfg(any(test, feature = "test-hooks"))]
pub const FLAGS: u8 = 0x03;
#[cfg(any(test, feature = "test-hooks"))]
pub const TEXT: u8 = 0x04;
#[cfg(any(test, feature = "test-hooks"))]
pub const SIZE: u8 = 0x05;
#[cfg(any(test, feature = "test-hooks"))]
pub const CHILD: u8 = 0x10;
#[cfg(any(test, feature = "test-hooks"))]
pub const NEXT: u8 = 0x11;
#[cfg(any(test, feature = "test-hooks"))]
pub const PARENT: u8 = 0x12;
#[cfg(any(test, feature = "test-hooks"))]
pub const DESCEND: u8 = 0x13;
#[cfg(any(test, feature = "test-hooks"))]
pub const CAPTURE: u8 = 0x20;
#[cfg(any(test, feature = "test-hooks"))]
pub const FORK: u8 = 0x30;
#[cfg(any(test, feature = "test-hooks"))]
pub const GOTO: u8 = 0x31;
#[cfg(any(test, feature = "test-hooks"))]
pub const EMIT: u8 = 0x40;

#[cfg(any(test, feature = "test-hooks"))]
pub const NAMED: u8 = 1;
#[cfg(any(test, feature = "test-hooks"))]
const SIZE_LONG: u8 = 0x80;
#[cfg(any(test, feature = "test-hooks"))]
const MAX_STEPS_PER_PATH: usize = 1 << 20;

/// A `TEXT` test.
#[cfg(any(test, feature = "test-hooks"))]
#[derive(Clone)]
pub enum TextPredicate {
    Equal(u16),
    Prefix(u16),
    Regex { source: u16, compiled: Regex },
}

#[cfg(any(test, feature = "test-hooks"))]
impl TextPredicate {
    fn string_index(&self) -> u16 {
        match self {
            Self::Equal(index) | Self::Prefix(index) => *index,
            Self::Regex { source, .. } => *source,
        }
    }
}

#[cfg(any(test, feature = "test-hooks"))]
#[derive(Clone)]
pub struct Program {
    pub words: Vec<u32>,
    strings: Vec<Vec<u8>>,
    predicates: Vec<TextPredicate>,
    labels: Vec<u16>,
    longs: Vec<u64>,
}

#[cfg(any(test, feature = "test-hooks"))]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Capture {
    pub(super) label: u16,
    pub(super) node: u32,
    pub span: Range<usize>,
}

#[cfg(any(test, feature = "test-hooks"))]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Output {
    pub(super) start: u32,
    pub(super) label: u16,
    pub captures: Vec<Capture>,
}

#[cfg(any(test, feature = "test-hooks"))]
#[derive(Debug, Eq, PartialEq)]
pub enum Error {
    InvalidOpcode(usize),
    InvalidMode(usize),
    InvalidOperand(usize),
    InvalidTarget(usize),
    NonProgressingCycle(usize),
    WorkLimit,
    UnsupportedPattern,
    InvalidAnchor,
}

#[cfg(any(test, feature = "test-hooks"))]
#[derive(Clone)]
struct Path {
    pc: u16,
    node: u32,
    start: u32,
    captures: Vec<Capture>,
    steps: usize,
}

#[cfg(any(test, feature = "test-hooks"))]
pub fn split(word: u32) -> (u8, u8, u16) {
    (word as u8, (word >> 8) as u8, (word >> 16) as u16)
}

#[cfg(any(test, feature = "test-hooks"))]
pub const fn word(opcode: u8, mode: u8, operand: u16) -> u32 {
    opcode as u32 | ((mode as u32) << 8) | ((operand as u32) << 16)
}

#[cfg(any(test, feature = "test-hooks"))]
impl Program {
    /// Compiles the exact root-terminal subset of ast-grep patterns. Structural patterns and
    /// metavariables are rejected rather than delegated to ast-grep inside the transducer.
    pub fn compile_ast_grep_literal(pattern: &Pattern) -> Result<Self, Error> {
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

    pub fn new(
        words: Vec<u32>,
        strings: Vec<Vec<u8>>,
        predicates: Vec<TextPredicate>,
        labels: Vec<u16>,
        longs: Vec<u64>,
    ) -> Result<Self, Error> {
        let program = Self {
            words,
            strings,
            predicates,
            labels,
            longs,
        };
        program.validate()?;
        Ok(program)
    }

    fn validate(&self) -> Result<(), Error> {
        if self.words.is_empty() || self.words.len() > usize::from(u16::MAX) + 1 {
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
        }) {
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
                CHILD | NEXT | DESCEND => {
                    if mode > NAMED || operand != 0 {
                        return Err(Error::InvalidMode(pc));
                    }
                }
                CAPTURE | EMIT => {
                    if mode != 0 || usize::from(operand) >= self.labels.len() {
                        return Err(Error::InvalidOperand(pc));
                    }
                }
                FORK | GOTO => {
                    let target = usize::from(operand);
                    if mode != 0 || target >= self.words.len() {
                        return Err(Error::InvalidTarget(pc));
                    }
                    if target <= pc
                        && self.words[target..=pc]
                            .iter()
                            .any(|word| matches!(split(*word), (PARENT | CHILD, _, _)))
                    {
                        return Err(Error::NonProgressingCycle(pc));
                    }
                }
                _ => return Err(Error::InvalidOpcode(pc)),
            }
        }
        // Remove advancing instructions from the control-flow graph. Any remaining directed cycle
        // is reachable without changing nodes, so it would consume the per-path work limit.
        let mut state = vec![0_u8; self.words.len()];
        fn visit(words: &[u32], state: &mut [u8], pc: usize) -> Option<usize> {
            if matches!(split(words[pc]), (NEXT | DESCEND, _, _)) {
                return None;
            }
            match state[pc] {
                1 => return Some(pc),
                2 => return None,
                _ => state[pc] = 1,
            }
            let (opcode, _, operand) = split(words[pc]);
            let next = pc.checked_add(1).filter(|next| *next < words.len());
            let mut follow = |target| visit(words, state, target);
            let cycle = match opcode {
                STOP | EMIT => None,
                GOTO => follow(usize::from(operand)),
                FORK => {
                    let fallthrough = next.and_then(&mut follow);
                    fallthrough.or_else(|| follow(usize::from(operand)))
                }
                _ => next.and_then(follow),
            };
            state[pc] = 2;
            cycle
        }
        for pc in 0..self.words.len() {
            if let Some(pc) = visit(&self.words, &mut state, pc) {
                return Err(Error::NonProgressingCycle(pc));
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

    /// Whether `text` passes the program's text predicate `predicate`.
    fn text_matches(&self, predicate: u16, text: &str) -> bool {
        match &self.predicates[usize::from(predicate)] {
            TextPredicate::Equal(string) => {
                text.as_bytes() == self.strings[usize::from(*string)].as_slice()
            }
            TextPredicate::Prefix(string) => text
                .as_bytes()
                .starts_with(&self.strings[usize::from(*string)]),
            TextPredicate::Regex { compiled, .. } => compiled.is_match(text),
        }
    }

    pub fn execute(&self, tape: &TapeDoc) -> Result<Vec<Output>, Error> {
        self.execute_roots(tape, 0..tape.transducer_len() as u32)
    }

    pub fn execute_roots(
        &self,
        tape: &TapeDoc,
        roots: impl IntoIterator<Item = u32>,
    ) -> Result<Vec<Output>, Error> {
        self.execute_roots_from(tape, roots, 0)
    }

    fn execute_roots_from(
        &self,
        tape: &TapeDoc,
        roots: impl IntoIterator<Item = u32>,
        initial_pc: u16,
    ) -> Result<Vec<Output>, Error> {
        let mut output = Vec::new();
        let mut pending = Vec::new();
        for root in roots {
            debug_assert!(pending.is_empty());
            pending.push(Path {
                pc: initial_pc,
                node: root,
                start: root,
                captures: Vec::new(),
                steps: 0,
            });
            while let Some(mut path) = pending.pop() {
                loop {
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
                            retain =
                                self.text_matches(operand, tape.transducer_text_str(path.node));
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
                                    let mut alternative = path.clone();
                                    alternative.node = node;
                                    alternative.pc =
                                        next.ok_or(Error::InvalidTarget(path.pc as usize))?;
                                    pending.push(alternative);
                                }
                                path.node = first;
                            } else {
                                retain = false;
                            }
                        }
                        NEXT => {
                            if let Some(node) =
                                tape.transducer_next_sibling(path.node, mode == NAMED)
                            {
                                path.node = node;
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
                        CAPTURE => path.captures.push(Capture {
                            label: operand,
                            node: path.node,
                            span: tape.transducer_span(path.node),
                        }),
                        FORK => {
                            let mut alternative = path.clone();
                            alternative.pc = operand;
                            pending.push(alternative);
                        }
                        GOTO => {
                            path.pc = operand;
                            continue;
                        }
                        EMIT => {
                            output.push(Output {
                                start: path.start,
                                label: operand,
                                captures: std::mem::take(&mut path.captures),
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
        Ok(output)
    }
}

/// Executes several programs after one tape scan. A leading kind test becomes dispatch data;
/// every other instruction retains the ordinary transducer semantics.
#[cfg(any(test, feature = "test-hooks"))]
pub fn execute_batch(
    programs: &[Program],
    files: &[TapeDoc],
    anchors: Option<&[Vec<Vec<u32>>]>,
) -> Result<Vec<Vec<Vec<Output>>>, Error> {
    if let Some(anchors) = anchors
        && (anchors.len() != files.len() || anchors.iter().any(|file| file.len() != programs.len()))
    {
        return Err(Error::InvalidAnchor);
    }
    files
        .iter()
        .enumerate()
        .map(|(file_index, tape)| {
            if let Some(anchors) = anchors {
                programs
                    .iter()
                    .enumerate()
                    .map(|(program_index, program)| {
                        let roots = &anchors[file_index][program_index];
                        if roots
                            .iter()
                            .any(|&root| root as usize >= tape.transducer_len())
                            || roots.windows(2).any(|pair| pair[0] >= pair[1])
                        {
                            return Err(Error::InvalidAnchor);
                        }
                        program.execute_roots(tape, roots.iter().copied())
                    })
                    .collect()
            } else {
                execute_many(programs, tape)
            }
        })
        .collect()
}

#[cfg(any(test, feature = "test-hooks"))]
use super::search_index::persist_search_index_batch;

#[cfg(any(test, feature = "test-hooks"))]
pub fn execute_many(
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
            ),
            _ => program.execute(tape),
        })
        .collect()
}

// tests: a test the engine hosts (it reads above this crate; tools/cratify/test-hosts.tsv)
