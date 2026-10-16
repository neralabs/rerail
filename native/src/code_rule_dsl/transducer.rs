use super::tape::TapeDoc;
use ast_grep_core::MatchStrictness;
use ast_grep_core::matcher::{Pattern, PatternNode};
use regex::Regex;
#[cfg(test)]
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

pub(super) mod sql_backend;

// Benchmark-only backends are deliberately excluded from the production addon.
#[cfg(all(
    test,
    feature = "bench-backends",
    target_os = "macos",
    target_arch = "aarch64"
))]
mod jit;
#[cfg(all(test, feature = "bench-backends", target_os = "macos"))]
mod metal_backend;

const STOP: u8 = 0x00;
const KIND: u8 = 0x01;
const FIELD: u8 = 0x02;
const FLAGS: u8 = 0x03;
const TEXT: u8 = 0x04;
const SIZE: u8 = 0x05;
const CHILD: u8 = 0x10;
const NEXT: u8 = 0x11;
const PARENT: u8 = 0x12;
const DESCEND: u8 = 0x13;
const CAPTURE: u8 = 0x20;
const FORK: u8 = 0x30;
const GOTO: u8 = 0x31;
const EMIT: u8 = 0x40;

const NAMED: u8 = 1;
const SIZE_LONG: u8 = 0x80;
const MAX_STEPS_PER_PATH: usize = 1 << 20;

/// A `TEXT` test.
#[derive(Clone)]
#[allow(
    dead_code,
    reason = "literal and prefix predicates support restored and benchmark backends"
)]
pub(super) enum TextPredicate {
    Equal(u16),
    Prefix(u16),
    Regex { source: u16, compiled: Regex },
}

impl TextPredicate {
    fn string_index(&self) -> u16 {
        match self {
            Self::Equal(index) | Self::Prefix(index) => *index,
            Self::Regex { source, .. } => *source,
        }
    }
}

#[derive(Clone)]
pub(super) struct Program {
    words: Vec<u32>,
    strings: Vec<Vec<u8>>,
    predicates: Vec<TextPredicate>,
    labels: Vec<u16>,
    longs: Vec<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Capture {
    pub(super) label: u16,
    pub(super) node: u32,
    pub(super) span: Range<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Output {
    pub(super) start: u32,
    pub(super) label: u16,
    pub(super) captures: Vec<Capture>,
}

#[derive(Debug, Eq, PartialEq)]
#[allow(
    dead_code,
    reason = "restored and benchmark backends share this error contract"
)]
pub(super) enum Error {
    InvalidOpcode(usize),
    InvalidMode(usize),
    InvalidOperand(usize),
    InvalidTarget(usize),
    NonProgressingCycle(usize),
    WorkLimit,
    UnsupportedPattern,
    InvalidAnchor,
    #[cfg(all(feature = "bench-backends", target_os = "macos"))]
    UnsupportedBackend,
    #[cfg(all(feature = "bench-backends", target_os = "macos"))]
    BackendFailure,
}

#[derive(Clone)]
struct Path {
    pc: u16,
    node: u32,
    start: u32,
    captures: Vec<Capture>,
    steps: usize,
}

fn split(word: u32) -> (u8, u8, u16) {
    (word as u8, (word >> 8) as u8, (word >> 16) as u16)
}

pub(super) const fn word(opcode: u8, mode: u8, operand: u16) -> u32 {
    opcode as u32 | ((mode as u32) << 8) | ((operand as u32) << 16)
}

impl Program {
    /// Compiles the exact root-terminal subset of ast-grep patterns. Structural patterns and
    /// metavariables are rejected rather than delegated to ast-grep inside the transducer.
    #[allow(dead_code, reason = "retained for restored and benchmark backends")]
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

    pub(super) fn node_test(kinds: &[u16], regexes: &[Regex]) -> Result<Self, Error> {
        let mut words = kinds
            .iter()
            .map(|&kind| word(KIND, 0, kind))
            .collect::<Vec<_>>();
        let mut strings = Vec::with_capacity(regexes.len() + 1);
        let mut predicates = Vec::with_capacity(regexes.len());
        for regex in regexes {
            let index = u16::try_from(strings.len()).map_err(|_| Error::InvalidOperand(0))?;
            words.push(word(TEXT, 0, index));
            strings.push(regex.as_str().as_bytes().to_vec());
            predicates.push(TextPredicate::Regex {
                source: index,
                compiled: regex.clone(),
            });
        }
        let label = u16::try_from(strings.len()).map_err(|_| Error::InvalidOperand(0))?;
        strings.push(b"node-test".to_vec());
        words.push(word(EMIT, 0, 0));
        Self::new(words, strings, predicates, vec![label], Vec::new())
    }

    pub(super) fn accepts(&self, tape: &TapeDoc, node: u32) -> Result<bool, Error> {
        for &instruction in &self.words {
            let (opcode, mode, operand) = split(instruction);
            let retain = match opcode {
                KIND => tape.transducer_kind(node) == operand,
                FIELD => tape.transducer_field(node) == operand,
                FLAGS => {
                    let flags = tape.transducer_flags(node);
                    flags & mode == mode && flags & operand as u8 == 0
                }
                TEXT => self.text_matches(operand, tape.transducer_text_str(node)),
                EMIT => return Ok(true),
                STOP => return Ok(false),
                _ => {
                    return self
                        .execute_roots(tape, std::iter::once(node))
                        .map(|outputs| !outputs.is_empty());
                }
            };
            if !retain {
                return Ok(false);
            }
        }
        unreachable!("a validated program ends in STOP, EMIT or GOTO")
    }

    pub(super) fn new(
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

    #[allow(dead_code, reason = "retained for restored and benchmark backends")]
    pub(super) fn execute(&self, tape: &TapeDoc) -> Result<Vec<Output>, Error> {
        self.execute_roots(tape, 0..tape.transducer_len() as u32)
    }

    pub(super) fn execute_roots(
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
#[cfg(test)]
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

pub(crate) const POSITIONAL_INDEX_VERSION: i64 = 2;

#[cfg(all(test, feature = "bench-backends", target_os = "macos"))]
fn exact_anchor_key(program: &Program) -> Option<(u16, &[u8])> {
    if program.words.len() != 4
        || !matches!(split(program.words[0]), (KIND, 0, _))
        || !matches!(split(program.words[1]), (TEXT, 0, _))
        || !matches!(split(program.words[2]), (CAPTURE, 0, _))
        || !matches!(split(program.words[3]), (EMIT, 0, _))
    {
        return None;
    }
    let kind = split(program.words[0]).2;
    let predicate = split(program.words[1]).2;
    let TextPredicate::Equal(string) = program.predicates[usize::from(predicate)] else {
        return None;
    };
    Some((kind, &program.strings[usize::from(string)]))
}

pub(crate) fn kind_index_token(kind: u16) -> String {
    format!("k{kind:04x}")
}

fn search_index_document(
    tape_id: crate::content_store::TapeId,
    file: &TapeDoc,
) -> crate::content_store::TapeSearchIndex {
    let mut kinds = String::new();
    for node in 0..file.transducer_len() as u32 {
        if node != 0 {
            kinds.push(' ');
        }
        kinds.push_str(&kind_index_token(file.transducer_kind(node)));
    }
    crate::content_store::TapeSearchIndex {
        tape_id,
        index_version: POSITIONAL_INDEX_VERSION,
        node_count: u32::try_from(file.transducer_len())
            .expect("tape node count is limited to u32"),
        has_errors: !file.syntax_error_ranges().is_empty(),
        kind_tokens: kinds,
    }
}

/// Build query-independent kind streams. The stream preserves preorder positions.
pub(crate) fn persist_search_index_batch(
    store: &crate::content_store::ContentStore,
    tape_ids: &[crate::content_store::TapeId],
    files: &[TapeDoc],
) -> crate::content_store::StoreResult<()> {
    if tape_ids.len() != files.len() {
        return Err(crate::content_store::StoreError::Invalid(
            "tape identity count does not match anchor files".to_owned(),
        ));
    }
    for (files, tape_ids) in files.chunks(64).zip(tape_ids.chunks(64)) {
        let indexes = files
            .iter()
            .zip(tape_ids)
            .map(|(file, &tape_id)| search_index_document(tape_id, file))
            .collect::<Vec<_>>();
        store.put_search_indexes_bulk(&indexes)?;
    }
    Ok(())
}

/// Convert complete exact-literal anchors directly into terminal outputs. The selector already
/// proved the full predicate, so a second matcher or GPU dispatch would duplicate work.
#[cfg(all(test, feature = "bench-backends", target_os = "macos"))]
pub(super) fn materialize_literal_anchors(
    programs: &[Program],
    files: &[TapeDoc],
    anchors: &[Vec<Vec<u32>>],
) -> Result<Vec<Vec<Vec<Output>>>, Error> {
    if anchors.len() != files.len() || anchors.iter().any(|file| file.len() != programs.len()) {
        return Err(Error::InvalidAnchor);
    }
    files
        .iter()
        .enumerate()
        .map(|(file_index, tape)| {
            programs
                .iter()
                .enumerate()
                .map(|(program_index, program)| {
                    exact_anchor_key(program).ok_or(Error::UnsupportedBackend)?;
                    let capture = split(program.words[2]).2;
                    let label = split(program.words[3]).2;
                    let roots = &anchors[file_index][program_index];
                    if roots
                        .iter()
                        .any(|&root| root as usize >= tape.transducer_len())
                        || roots.windows(2).any(|pair| pair[0] >= pair[1])
                    {
                        return Err(Error::InvalidAnchor);
                    }
                    Ok(roots
                        .iter()
                        .map(|&node| Output {
                            start: node,
                            label,
                            captures: vec![Capture {
                                label: capture,
                                node,
                                span: tape.transducer_span(node),
                            }],
                        })
                        .collect())
                })
                .collect()
        })
        .collect()
}

#[cfg(test)]
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
            ),
            _ => program.execute(tape),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_rule_dsl::language::GrammarDialect;
    use crate::code_rule_dsl::tape::{Grammar, TapeImage};
    use ast_grep_core::AstGrep;
    use ast_grep_core::matcher::Pattern;
    use ast_grep_core::tree_sitter::StrDoc;
    use ast_grep_language::LanguageExt;
    use serde_json::json;
    use sha2::{Digest, Sha256};
    use std::fs;
    use std::hint::black_box;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    fn tape(source: &str) -> TapeDoc {
        let grammar = Grammar::new(GrammarDialect::Python).unwrap();
        let native = StrDoc::new(source, GrammarDialect::Python);
        let image =
            TapeImage::encode(native.tree.root_node(), &grammar, source, &|| false).unwrap();
        TapeDoc::restore(Arc::new(source.to_owned()), grammar, &image, &|| false).unwrap()
    }

    fn identifier_program(text: &str) -> Program {
        let pattern = Pattern::try_new(text, GrammarDialect::Python).unwrap();
        Program::compile_ast_grep_literal(&pattern).unwrap()
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
        let grammar = GrammarDialect::Python.get_ts_language();
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
                    GrammarDialect::Python
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
                        GrammarDialect::Python
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
    fn traversal_flags_sizes_and_long_table_execute() {
        let grammar = GrammarDialect::Python.get_ts_language();
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
        let identifier = GrammarDialect::Python
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
            .find(|&node| tape.transducer_text_str(node).as_bytes() == b"a")
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
        assert_eq!(
            execute_batch(&programs, &files, Some(&anchors)).unwrap(),
            unanchored
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
    fn selector_index_persists_only_the_kind_family() {
        let database = std::env::temp_dir().join(format!(
            "rerail-kind-index-{}-{}.sqlite",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let store = crate::content_store::ContentStore::open(&database).unwrap();
        let files = [tape("execute();\n"), tape("commit();\n")];
        let source = store.put_source(b"source", &[]).unwrap();
        let tape_ids = [
            store.put_tape(source, 1, 1, 1, b"first").unwrap(),
            store.put_tape(source, 2, 1, 1, b"second").unwrap(),
        ];
        crate::content_store::reset_search_index_family_writes();
        persist_search_index_batch(&store, &tape_ids, &files).unwrap();
        assert_eq!(crate::content_store::search_index_family_writes(), 2);
        drop(store);
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", database.display()));
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
        assert!(matches!(
            Program::new(
                vec![word(GOTO, 0, 2), word(NEXT, 0, 0), word(GOTO, 0, 0)],
                vec![],
                vec![],
                vec![],
                vec![]
            ),
            Err(Error::NonProgressingCycle(_))
        ));
        assert!(matches!(
            Program::new(
                vec![word(DESCEND, 0, 0), word(PARENT, 0, 0), word(GOTO, 0, 0)],
                vec![],
                vec![],
                vec![],
                vec![]
            ),
            Err(Error::NonProgressingCycle(_))
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

    /// The Python tree the transducer benchmarks read: `RERAIL_TRANSDUCER_CORPUS`. Without it they
    /// skip, saying so.
    fn transducer_corpus() -> Option<PathBuf> {
        let corpus = std::env::var_os("RERAIL_TRANSDUCER_CORPUS").map(PathBuf::from);
        if corpus.is_none() {
            eprintln!(
                "skipped: set RERAIL_TRANSDUCER_CORPUS to a directory of Python sources to benchmark"
            );
        }
        corpus
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
        let Some(root) = transducer_corpus() else {
            return;
        };
        let limit = std::env::var("RERAIL_TRANSDUCER_FILES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(usize::MAX);
        let iterations = std::env::var("RERAIL_TRANSDUCER_ITERATIONS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(7usize);
        let names = [
            "execute", "commit", "rollback", "flush", "select", "where", "join", "delete",
        ];
        let patterns = names
            .iter()
            .map(|name| Pattern::try_new(name, GrammarDialect::Python).unwrap())
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

        let grammar = Grammar::new(GrammarDialect::Python).unwrap();
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
            let doc = StrDoc::new(&source, GrammarDialect::Python);
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
            "benchmark": "rerail-fixed-word-transducer-v1",
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
    #[ignore = "full ast-index-spike tree-sitter query oracle benchmark"]
    fn benchmark_tree_sitter_query_oracle() {
        use tree_sitter::StreamingIterator;

        let query_root = PathBuf::from(
            std::env::var("RERAIL_QUERY_CORPUS")
                .expect("RERAIL_QUERY_CORPUS must name ast-index-spike bench/corpus/queries"),
        );
        let source_root = PathBuf::from(
            std::env::var("RERAIL_SOURCE_CORPUS")
                .expect("RERAIL_SOURCE_CORPUS must name ast-index-spike bench/corpus/repos"),
        );
        let languages: &[(&str, GrammarDialect, &[&str])] = &[
            ("python", GrammarDialect::Python, &["py"]),
            (
                "javascript",
                GrammarDialect::JavaScript,
                &["js", "mjs", "cjs", "jsx"],
            ),
            ("typescript", GrammarDialect::TypeScript, &["ts"]),
            ("tsx", GrammarDialect::Tsx, &["tsx"]),
            ("java", GrammarDialect::Java, &["java"]),
            ("csharp", GrammarDialect::CSharp, &["cs"]),
            (
                "cpp",
                GrammarDialect::Cpp,
                &["cc", "cpp", "cxx", "hpp", "hh", "hxx"],
            ),
            ("c", GrammarDialect::C, &["c", "h"]),
            ("php", GrammarDialect::Php, &["php"]),
            ("bash", GrammarDialect::Bash, &["sh", "bash", "zsh"]),
            ("ruby", GrammarDialect::Ruby, &["rb"]),
            ("go", GrammarDialect::Go, &["go"]),
            ("rust", GrammarDialect::Rust, &["rs"]),
            ("kotlin", GrammarDialect::Kotlin, &["kt", "kts"]),
            ("scala", GrammarDialect::Scala, &["scala", "sc"]),
            ("swift", GrammarDialect::Swift, &["swift"]),
            ("html", GrammarDialect::Html, &["html", "htm"]),
            ("css", GrammarDialect::Css, &["css"]),
            ("lua", GrammarDialect::Lua, &["lua"]),
            ("json", GrammarDialect::Json, &["json"]),
        ];
        let selected_languages = std::env::var("RERAIL_QUERY_LANGS")
            .ok()
            .map(|value| value.split(',').map(str::to_owned).collect::<BTreeSet<_>>());
        let query_filter = std::env::var("RERAIL_QUERY_FILTER").ok();
        let source_limit = std::env::var("RERAIL_QUERY_SOURCE_BYTES")
            .ok()
            .map(|value| value.parse::<usize>().unwrap())
            .unwrap_or(1_000_000);
        let shard_count = std::env::var("RERAIL_QUERY_SHARD_COUNT")
            .ok()
            .map(|value| value.parse::<usize>().unwrap())
            .unwrap_or(1);
        let shard_index = std::env::var("RERAIL_QUERY_SHARD_INDEX")
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
                   "unsupported":["SQL grammar","Rerail transducer compilation of .scm queries"]})
        );
    }

    #[test]
    #[ignore = "full ast-index-spike tree-sitter query corpus audit"]
    fn audit_tree_sitter_query_corpus() {
        let query_root = PathBuf::from(
            std::env::var("RERAIL_QUERY_CORPUS")
                .expect("RERAIL_QUERY_CORPUS must name ast-index-spike bench/corpus/queries"),
        );
        let languages = [
            ("python", GrammarDialect::Python),
            ("javascript", GrammarDialect::JavaScript),
            ("typescript", GrammarDialect::TypeScript),
            ("java", GrammarDialect::Java),
            ("csharp", GrammarDialect::CSharp),
            ("cpp", GrammarDialect::Cpp),
            ("c", GrammarDialect::C),
            ("php", GrammarDialect::Php),
            ("bash", GrammarDialect::Bash),
            ("ruby", GrammarDialect::Ruby),
            ("go", GrammarDialect::Go),
            ("rust", GrammarDialect::Rust),
            ("kotlin", GrammarDialect::Kotlin),
            ("scala", GrammarDialect::Scala),
            ("swift", GrammarDialect::Swift),
            ("html", GrammarDialect::Html),
            ("css", GrammarDialect::Css),
            ("lua", GrammarDialect::Lua),
            ("json", GrammarDialect::Json),
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
            for pattern in &patterns {
                match tree_sitter::Query::new(&grammar, pattern) {
                    Ok(query) if query.pattern_count() == 1 => {
                        valid += 1;
                        corpus.push(json!({"lang":name,"text":pattern}));
                    }
                    _ => invalid += 1,
                }
            }
            total += valid;
            report.insert(
                name.to_owned(),
                json!({"attempted": patterns.len(), "valid": valid, "invalid": invalid,
                       "grammarSymbols": grammar.node_kind_count(),
                       "grammarFields": grammar.field_count()}),
            );
            if name == "typescript" {
                let tsx = GrammarDialect::Tsx.get_ts_language();
                let valid_tsx = patterns
                    .iter()
                    .filter(|pattern| tree_sitter::Query::new(&tsx, pattern).is_ok())
                    .count();
                report.insert(
                    "tsx".to_owned(),
                    json!({"attempted": patterns.len(), "valid": valid_tsx,
                           "invalid": patterns.len() - valid_tsx,
                           "grammarSymbols": tsx.node_kind_count(),
                           "grammarFields": tsx.field_count()}),
                );
            }
        }
        report.insert(
            "sql".to_owned(),
            json!({"supported": false, "reason": "ast-grep-language has no SQL grammar"}),
        );
        if let Ok(path) = std::env::var("RERAIL_QUERY_CORPUS_OUTPUT") {
            fs::write(path, serde_json::to_vec_pretty(&corpus).unwrap()).unwrap();
        }
        println!(
            "TREE_SITTER_QUERY_AUDIT={}",
            json!({"validWithoutSql": total, "languages": report})
        );
    }

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    #[test]
    #[cfg(feature = "bench-backends")]
    #[ignore = "Apple Silicon backend benchmark"]
    fn benchmark_anchored_cpu_and_gpu_backends() {
        let Some(root) = transducer_corpus() else {
            return;
        };
        let limit = std::env::var("RERAIL_TRANSDUCER_FILES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(usize::MAX);
        let iterations = std::env::var("RERAIL_TRANSDUCER_ITERATIONS")
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
        let grammar = Grammar::new(GrammarDialect::Python).unwrap();
        let mut tapes = Vec::with_capacity(paths.len());
        let mut source_bytes = 0;
        for path in &paths {
            let source = fs::read_to_string(path).unwrap();
            source_bytes += source.len();
            let native = StrDoc::new(&source, GrammarDialect::Python);
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
                                    && tape.transducer_text_str(node).as_bytes() == expected
                            })
                            .collect()
                    })
                    .collect()
            })
            .collect::<Vec<Vec<Vec<u32>>>>();
        let anchor_time = anchor_started.elapsed();
        let started = Instant::now();
        let cpu_jit = jit::compile(&programs).unwrap();
        let cpu_compile = started.elapsed();
        let started = Instant::now();
        let gpu_interpreter = metal_backend::compile_interpreted(&programs).unwrap();
        let gpu_interpreter_compile = started.elapsed();
        let started = Instant::now();
        let gpu_jit = metal_backend::compile_jit(&programs).unwrap();
        let gpu_jit_compile = started.elapsed();
        let started = Instant::now();
        let gpu_anchor_scan = metal_backend::prepare_anchor_scan(&programs, &tapes).unwrap();
        let gpu_anchor_prepare = started.elapsed();
        let gpu_anchors = gpu_anchor_scan.run().unwrap();
        assert_eq!(
            gpu_anchors, anchors,
            "GPU anchor scan changed preorder postings"
        );
        let started = Instant::now();
        let gpu_interpreted_batch =
            metal_backend::prepare(&gpu_interpreter, &programs, &tapes, &gpu_anchors).unwrap();
        let gpu_interpreted_prepare = started.elapsed();
        let started = Instant::now();
        let gpu_jit_batch =
            metal_backend::prepare(&gpu_jit, &programs, &tapes, &gpu_anchors).unwrap();
        let gpu_jit_prepare = started.elapsed();
        let unanchored_run = || execute_batch(&programs, &tapes, None).unwrap();
        let interpreted_run = || execute_batch(&programs, &tapes, Some(&anchors)).unwrap();
        let cpu_jit_run = || jit::execute(&cpu_jit, &tapes, &anchors).unwrap();
        let gpu_interpreted_cold =
            || metal_backend::execute(&gpu_interpreter, &programs, &tapes, &anchors).unwrap();
        let gpu_jit_cold =
            || metal_backend::execute(&gpu_jit, &programs, &tapes, &anchors).unwrap();
        let gpu_interpreted_warm = || gpu_interpreted_batch.run().unwrap();
        let gpu_jit_warm = || gpu_jit_batch.run().unwrap();
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
        assert_eq!(ranges(gpu_interpreted_cold()), expected);
        assert_eq!(ranges(gpu_jit_cold()), expected);
        assert_eq!(ranges(gpu_interpreted_warm()), expected);
        assert_eq!(ranges(gpu_jit_warm()), expected);
        let mut times: [Vec<Duration>; 8] = std::array::from_fn(|_| Vec::new());
        for iteration in 0..=iterations {
            for offset in 0..8 {
                let engine = (iteration + offset) % 8;
                let started = Instant::now();
                let count = match engine {
                    0 => ranges(unanchored_run()).len(),
                    1 => ranges(interpreted_run()).len(),
                    2 => ranges(cpu_jit_run()).len(),
                    3 => ranges(gpu_interpreted_cold()).len(),
                    4 => ranges(gpu_jit_cold()).len(),
                    5 => ranges(gpu_interpreted_warm()).len(),
                    6 => ranges(gpu_jit_warm()).len(),
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
        let report = json!({"benchmark":"rerail-anchored-transducer-backends-v3","corpus":fs::canonicalize(&root).unwrap(),"files":paths.len(),"sourceBytes":source_bytes,"patterns":names,"matches":expected.len(),"orderedRangeSha256":digest_ranges(&expected),"iterations":iterations,"cpuAnchorBuildMs":ms(anchor_time),"prepareMs":{"gpuAnchorScan":ms(gpu_anchor_prepare),"gpuInterpreterBatch":ms(gpu_interpreted_prepare),"gpuJitBatch":ms(gpu_jit_prepare)},"compileMs":{"cpuJit":ms(cpu_compile),"gpuInterpreter":ms(gpu_interpreter_compile),"gpuJit":ms(gpu_jit_compile)},"medianMs":{"cpuUnanchored":ms(medians[0]),"cpuInterpreted":ms(medians[1]),"cpuJit":ms(medians[2]),"gpuInterpretedCold":ms(medians[3]),"gpuJitCold":ms(medians[4]),"gpuInterpretedWarm":ms(medians[5]),"gpuJitWarm":ms(medians[6]),"gpuAnchorScanWarm":ms(medians[7])},"gpuAnchorProfile":{"dispatchAndWaitMs":ms(gpu_anchor_dispatch),"reconstructionMs":ms(gpu_anchor_reconstruction),"fusedTerminalMs":ms(gpu_fused_terminal),"sparseHitCount":gpu_anchor_hits},"anchorSpeedup":ms(medians[0])/ms(medians[1]),"speedupVsCpuInterpreted":{"cpuJit":ms(medians[1])/ms(medians[2]),"gpuInterpretedWarm":ms(medians[1])/ms(medians[5]),"gpuJitWarm":ms(medians[1])/ms(medians[6])}});
        println!("TRANSDUCER_BACKEND_BENCHMARK={report}");
    }
}
