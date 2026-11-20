//! A lowered matcher's label passes on Metal in one command buffer (06-first-slice S2, O23).
//!
//! The labels live in a prefix of the batch's supertype buffer, which `has_supertype` reads as
//! derived classes: element 0 is {label count, words per label}, then one bitset per label over the
//! batch's global nodes. Passes run a dependency level at a time, so the launches grow with the
//! depth of the label graph, not its size (the evaluation's launch budget, `metal_launch_budget`):
//! one interpreter `run_batch` with every program of the level on one shared workspace (its
//! scheduler image copied in by a blit), `label_sink` turning their results into scratch bits or
//! counts, the clause's pattern kernel (`patterns`), and `label_settle` writing each target:
//! from scratch, or as a complement or count bound computed per word. A recursive group's rounds
//! are encoded too and skipped on the GPU once settled (`label_round`), so a clause is one
//! command buffer. A group of programs alone runs as sweeps instead (`label_sweep`): one launch
//! per sweep, each thread a file in node order, each result marking its label at once, so the
//! sweeps follow the recursion's direction rather than one tree edge per round.

#[path = "metal_label_patterns.rs"]
pub mod patterns;

use super::super::PackedPattern;
use super::super::metal_batch_abi::MetalSupertype;
use super::super::native_ir::ArenaKind;
use super::*;
use super::super::metal_batch_abi::MetalEvaluation;
use metal::{Buffer, CommandBuffer};
use std::collections::{BTreeMap, HashMap};
use crate::code_rule_dsl::residency::slabs::ResidentParts as _;
use crate::code_rule_dsl::tape::DERIVED_CLASS_BASE;
pub use crate::code_rule_dsl::transducer::packed::compile_sets::DeviceFormula;
use crate::code_rule_dsl::transducer::packed::compile_sets;
pub use crate::code_rule_dsl::transducer::packed::device::{Outcome, RootJobs, RootsFound};

/// A pass writes the label `target` (an index, not a class); labels it reads are its classes.
pub(in crate::code_rule_dsl) enum Pass<'a> {
    /// A program's roots or captured nodes. Its roots are shared with the passes of the same
    /// domain (`matcher::labels_metal`), so a walk's items are one range for them all.
    Program {
        target: u32,
        program: &'a Program,
        roots: std::sync::Arc<Roots>,
        captured: bool,
    },
    /// The nodes with between `min` and `max` of a program's outputs, by root or captured node,
    /// tallied in count slot `slot`.
    Count {
        target: u32,
        slot: u32,
        program: &'a Program,
        roots: std::sync::Arc<Roots>,
        captured: bool,
        min: u32,
        max: u32,
    },
    Complement { target: u32, of: u32 },
    /// The `roots` where `plan.patterns[pattern]` matches and each named constraint's
    /// single-node capture has its label (an unbound or multi-node capture is not checked).
    Pattern {
        target: u32,
        pattern: usize,
        roots: std::sync::Arc<Roots>,
        constraints: Vec<(String, u32)>,
    },
    /// The roots a semantic predicate's producer reported (O24, decision 0030) whose slots' targets each have their
    /// label (`labels`, a slot each). The findings are where the host wrote them, in the batch's fact block (Q317):
    /// per file, its rows (node, then a target per slot, file-local) or its nodes as bits. The pattern kernel's fact
    /// case (`patterns::FACT`) reads them there and tests them, so a fact settles from scratch as a pattern does.
    Fact {
        target: u32,
        block: std::sync::Arc<rerail_blocks::block::Block>,
        found: Vec<(u32, crate::code_rule_dsl::matcher::FactFound)>,
        labels: Vec<u32>,
    },
    Empty { target: u32 },
    /// A pass whose program is a node-set formula (decision 0036 on Metal, 0365 §9, §10): `formula` evaluated on the
    /// device (`label_phase`) into the pass's scratch row, which settles as a program's does. No walk. A roots
    /// pass's is within `roots`; a targets pass's (`image`) is the image of its starts, which the formula holds.
    Formula { target: u32, roots: std::sync::Arc<Roots>, formula: std::sync::Arc<DeviceFormula>, image: bool },
}

/// A formula `label_phase` evaluates (decision 0365): where its set goes (`row`, `mode`, `rule`, as `FormulaTarget`),
/// and each of its text slots' row in the bound text rows (`None`: every node, for an anchor).
#[derive(Clone)]
struct FormulaJob {
    formula: std::sync::Arc<DeviceFormula>,
    row: u32,
    mode: u32,
    rule: u32,
    texts: Vec<Option<u32>>,
}

/// A pass's roots, as a rule over each document's codes (decision 0033): every root test a pass's
/// domain makes (its kinds, its program's leading kind and supertype tests, `matcher::Entry`) is a
/// property of a node's code. The device is handed the rule, not its roots (decision 0293 §10): a
/// thread takes a pass and a group of 64 of a document's nodes and finds the group's roots from
/// the tape's code bytes (`root_mask`). Listed roots (a clause's prefilter candidates, few and
/// scattered) are a row of bits over the batch's nodes.
///
/// Either may have an anchor (decision 0365): a formula `label_phase` evaluates on the device before the walks
/// of its level, into a row of bits the rule's roots are also in (a rule over codes: its row; listed: the list's
/// row, the nodes outside the anchor cleared). The host's view of the roots (`nodes`) is the rule's, a superset.
#[derive(Debug)]
pub(in crate::code_rule_dsl) enum Roots {
    /// The nodes whose kind is one of `kinds` (any where `None`) and whose code passes `entry`
    /// (`(KIND, kind)` or `(SUPERTYPE, class)`), in the files `active` marks.
    Codes { kinds: Option<Vec<u16>>, entry: Vec<(u8, u16)>, active: Vec<bool>, anchor: Option<std::sync::Arc<DeviceFormula>> },
    /// Per file, its listed nodes (document-local, ascending).
    Listed(Vec<Vec<u32>>, Option<std::sync::Arc<DeviceFormula>>),
}

impl Roots {
    fn admits(kinds: &Option<Vec<u16>>, entry: &[(u8, u16)], kind: u16, classes: &[u16]) -> bool {
        kinds.as_ref().is_none_or(|kinds| kinds.binary_search(&kind).is_ok())
            && entry.iter().all(|&(op, arg)| if op == super::super::KIND { kind == arg } else { classes.contains(&arg) })
    }

    /// Whether block `b` of `tape` may hold a root of a rule over codes, by the tape's presence (decision 0426 §4,
    /// step 6): one of its kinds, and each kind its tests name. The kernels' `rule_may_in`, the same read
    /// (`tape_has_kind`); a supertype test the presence does not tell.
    fn may_in(kinds: &Option<Vec<u16>>, entry: &[(u8, u16)], tape: &TapeDoc, b: usize) -> bool {
        kinds.as_ref().is_none_or(|kinds| kinds.iter().any(|&kind| tape.block_may_have_kind(b, kind)))
            && entry.iter().all(|&(op, arg)| op != super::super::KIND || tape.block_may_have_kind(b, arg))
    }

    /// This rule as the sweep tests it at a node's code (`sweep_admits`): {kind count or NONE,
    /// kinds, test count, tests (op << 16 | argument)}. A sweep's passes have rules over codes.
    fn device_rule(&self) -> Result<Vec<u32>, Error> {
        let Self::Codes { kinds, entry, .. } = self else { return Err(Error::BackendFailure) };
        let mut rule = match kinds {
            None => vec![u32::MAX],
            Some(kinds) => std::iter::once(kinds.len() as u32).chain(kinds.iter().map(|&kind| u32::from(kind))).collect(),
        };
        rule.push(entry.len() as u32);
        rule.extend(entry.iter().map(|&(op, arg)| u32::from(op) << 16 | u32::from(arg)));
        Ok(rule)
    }

    /// The roots in file `file` on the host, ascending (the CPU's readers and the counters).
    pub(in crate::code_rule_dsl) fn nodes<'a>(&'a self, file: usize, tape: &'a TapeDoc) -> Box<dyn Iterator<Item = u32> + 'a> {
        match self {
            Self::Listed(lists, _) => Box::new(lists.get(file).into_iter().flatten().copied()),
            Self::Codes { kinds, entry, active, .. } => {
                if !active.get(file).copied().unwrap_or(false) {
                    return Box::new(std::iter::empty());
                }
                let mut nodes = Vec::new();
                tape.transducer_nodes_where_in(|kind, classes| Self::admits(kinds, entry, kind, classes), &|b| Self::may_in(kinds, entry, tape, b as usize), &mut nodes);
                Box::new(nodes.into_iter())
            }
        }
    }

    /// Whether this rule may have roots in file `file` (a rule over codes runs in the files
    /// `active` marks; a list, where it has some).
    fn runs_in(&self, file: usize) -> bool {
        match self {
            Self::Listed(lists, _) => lists.get(file).is_some_and(|list| !list.is_empty()),
            Self::Codes { active, .. } => active.get(file).copied().unwrap_or(false),
        }
    }

    /// The rule's anchor, if it has one.
    pub(in crate::code_rule_dsl) fn anchor(&self) -> Option<&std::sync::Arc<DeviceFormula>> {
        match self {
            Self::Codes { anchor, .. } | Self::Listed(_, anchor) => anchor.as_ref(),
        }
    }

    /// This rule as the device reads it (`root_mask`), appended to `words`: its kind (0 over codes,
    /// 1 listed, 2 over codes with an anchor); over codes, a bit per file it runs in, then its test
    /// (`device_rule`); listed, its nodes as a row of bits over the batch's nodes. With an anchor,
    /// over codes: the offset of the rule without it, the anchor's row (zero: the formula sets it),
    /// then that rule. A row has two words past the last, which a 64-bit window at the end reads.
    /// With an anchor: the row's word, and whether the formula sets it (0, codes) or clears it (1,
    /// listed).
    fn table(&self, files: &[TapeDoc], words: &mut Vec<u32>) -> Result<Option<(u32, u32)>, Error> {
        let nodes = files.iter().map(TapeDoc::transducer_len).sum::<usize>();
        match self {
            Self::Codes { active, anchor, .. } => {
                // With an anchor: 2, the codes rule's offset from here, the row, then the rule as one without.
                let row = anchor.as_ref().map(|_| {
                    let at = words.len();
                    words.extend([2, (2 + nodes.div_ceil(32) + 2) as u32]);
                    words.resize(at + 2 + nodes.div_ceil(32) + 2, 0);
                    (at + 2) as u32
                });
                words.push(0);
                let mut bits = vec![0_u32; files.len().div_ceil(32)];
                for file in (0..files.len()).filter(|&file| active.get(file).copied().unwrap_or(false)) {
                    bits[file / 32] |= 1 << (file % 32);
                }
                words.extend(bits);
                words.extend(self.device_rule()?);
                if let Some(row) = row {
                    return Ok(Some((row, 0)));
                }
            }
            Self::Listed(lists, anchor) => {
                words.push(1);
                let row = words.len();
                words.resize(row + nodes.div_ceil(32) + 2, 0);
                let mut base = 0;
                for (file, tape) in files.iter().enumerate() {
                    for &local in lists.get(file).into_iter().flatten() {
                        let node = base + local as usize;
                        words[row + node / 32] |= 1 << (node % 32);
                    }
                    base += tape.transducer_len();
                }
                if anchor.is_some() {
                    return Ok(Some((row as u32, 1)));
                }
            }
        }
        Ok(None)
    }
}

/// A batch's groups of 64 nodes, each document's from its first node (`ROOT_GROUP`).
const ROOT_GROUP: usize = 64;

/// A run's pass roots as the device reads them (`root_mask`): the batch's file count and each
/// file's first group (and the total), then one rule per distinct `Roots` (by its `Arc`), so passes
/// of one domain share it. Nothing a root: a rule is a few words, a list a bit a node.
#[derive(Default)]
struct RootTable {
    words: Vec<u32>,
    by_rule: HashMap<usize, (u32, bool)>,
    /// By rule, its anchor's row and mode (`Roots::table`), for `label_phase`.
    anchors: HashMap<usize, (u32, u32)>,
}

impl RootTable {
    fn prefix(&mut self, files: &[TapeDoc]) {
        if self.words.is_empty() {
            self.words.push(files.len() as u32);
            let mut group = 0_u32;
            for tape in files {
                self.words.push(group);
                group += tape.transducer_len().div_ceil(ROOT_GROUP) as u32;
            }
            self.words.push(group);
        }
    }

    /// The batch's groups.
    fn groups(&self) -> u32 {
        if self.words.is_empty() { 0 } else { self.words[1 + self.words[0] as usize] }
    }

    /// A file's groups.
    fn file_groups(files: &[TapeDoc], file: usize) -> u32 {
        files[file].transducer_len().div_ceil(ROOT_GROUP) as u32
    }

    /// The rule's word in the table, and whether it may have roots anywhere in the batch.
    fn of(&mut self, roots: &std::sync::Arc<Roots>, files: &[TapeDoc]) -> Result<(u32, bool), Error> {
        self.prefix(files);
        if let Some(found) = self.by_rule.get(&(std::sync::Arc::as_ptr(roots) as usize)) {
            return Ok(*found);
        }
        let at = self.words.len() as u32;
        if let Some(row) = roots.table(files, &mut self.words)? {
            self.anchors.insert(std::sync::Arc::as_ptr(roots) as usize, row);
        }
        let found = (at, (0..files.len()).any(|file| roots.runs_in(file)));
        self.by_rule.insert(std::sync::Arc::as_ptr(roots) as usize, found);
        Ok(found)
    }

    /// A registered rule's word.
    fn at(&self, roots: &std::sync::Arc<Roots>) -> Result<(u32, bool), Error> {
        self.by_rule.get(&(std::sync::Arc::as_ptr(roots) as usize)).copied().ok_or(Error::BackendFailure)
    }
}

/// Whether a pass runs in the pattern kernel: a pattern's or a fact's.
fn kernel_pass(pass: &Pass<'_>) -> bool {
    matches!(pass, Pass::Pattern { .. } | Pass::Fact { .. })
}

pub(in crate::code_rule_dsl) struct Plan<'a> {
    pub(in crate::code_rule_dsl) labels: usize,
    /// Count slots (`Pass::Count::slot`).
    pub(in crate::code_rule_dsl) counts: usize,
    /// The clause's ast-grep patterns, matched on the GPU (`patterns`).
    pub(in crate::code_rule_dsl) patterns: Vec<&'a PackedPattern>,
    /// Groups in order, each as its dependency levels: a level's passes read only earlier
    /// levels' labels. A recursive group repeats until no label of it changes.
    pub(in crate::code_rule_dsl) steps: Vec<(bool, Vec<Vec<Pass<'a>>>)>,
    /// Per step, for a recursive group: the kinds its recursive labels' nodes have (`None`:
    /// any), and whether its programs step to siblings (`NEXT`, `FOLLOWING`). Its rounds follow
    /// chains of such nodes, nested or (stepping to siblings) side by side.
    pub(in crate::code_rule_dsl) chains: Vec<(Option<Vec<u16>>, bool)>,
    /// The labels' words before the passes (as `Outcome::labels`), where steps before them ran
    /// elsewhere; empty otherwise.
    /// The labels so far, none where they start empty (the prefix starts zeroed). A successful
    /// run's labels come back in the same allocation (`Outcome::labels`), so the host never holds
    /// two copies.
    pub(in crate::code_rule_dsl) initial: std::cell::Cell<Option<Vec<u32>>>,
    /// Text rows evaluated by earlier runs over the same files (`TextRows`), shared by them.
    pub(in crate::code_rule_dsl) rows: Option<&'a TextRows>,
    /// Buffers that earlier runs over the same files left for the next (`RunBuffers`).
    pub(in crate::code_rule_dsl) buffers: Option<&'a RunBuffers>,
    /// Root programs walked after the passes, in the same command buffer, as match records
    /// (`root_records_*`, decision 0024 §3): `Outcome::records`.
    pub(in crate::code_rule_dsl) roots: Option<&'a RootJobs<'a>>,
    /// W's matcher's predicted steps for what goes to it where the pass of label `l` on file `f` is unfinished (decision
    /// 0334: its clause, and every clause that reads it, on the file): the run weighs it against the run again with
    /// twice the budget. `None` where the caller has no W: an overrun runs again, a stop at the ceiling fails.
    pub(in crate::code_rule_dsl) w_cost: Option<&'a (dyn Fn(u32, usize) -> u64 + Sync)>,
}

/// The root walks' kernel in `KERNELS`, for 8 and 32 frames (`ROOT_FRAMES`, its shape's `frames`), and its tiers'
/// names, which their dispatches and the profile keep.
const ROOT_KERNEL: usize = 5;
const ROOT_FRAMES: [u32; 2] = [8, 32];
const ROOT_NAMES: [&str; 2] = ["root_records_8", "root_records_32"];
/// Words of a root record (`root_records_*`).
const ROOT_RECORD: usize = 20;
/// The 32-frame root walks' pool: threads striding over the items, each a device stack of 32
/// `RootFrame`s of 31 words (thread memory for every item in flight held ~140 MB of footprint).
const ROOT_POOL: usize = 4096;
const ROOT_STACK_WORDS: usize = 32 * 31;
// The 32-frame root pool's stacks are part of the pooled stacks.
const _: () = assert!(ROOT_POOL * ROOT_STACK_WORDS <= STACK_WORDS);
/// A deep group walk's device stack: 32 `MicroFrame`s of 17 words, per thread launched.
const GROUP_STACK_WORDS: usize = 32 * 17;

/// A formula's word that is a leaf (decision 0365): a test of the node alone, read for every formula by `label_leaves`.
/// A formula's phases (`phase_program`) at the first chain levels, its leaf tests as the rows the device reads them
/// by: a plan's launches.
fn formula_phase_count(formula: &DeviceFormula) -> usize {
    let words = formula.words.iter().map(|&word| if formula_leaf(word) { compile_sets::device_ops::LEAF } else { word }).collect::<Vec<_>>();
    phase_program(&words, PHASE_CHAIN_LEVELS).map_or(1, |program| program.phases as usize)
}

fn formula_leaf(word: u32) -> bool {
    use compile_sets::device_ops;
    let (op, arg) = (word & 255, word >> 8);
    matches!(op, device_ops::KIND | device_ops::FIELD | device_ops::FLAGS | device_ops::HAS_NEXT | device_ops::HAS_BELOW | device_ops::HAS_NAMED_CHILD | device_ops::HAS_NEXT_NAMED)
        || (op == device_ops::CLASS && arg < u32::from(DERIVED_CLASS_BASE))
}

/// A walk's anchor as the device evaluates it (decision 0365): `Set::anchor` of its micro walk, its text tests kept by
/// slot key (`DeviceFormula::texts`; the device reads the walk's row for each, every node where there is none). `None`
/// where the walk is not a micro walk, or the anchor tests no more than a rule over codes does (`narrowing`).
///
/// A label `unknown` names (a recursive group's own, Q133) is read as every node, as on the CPU (`anchor_bits`).
///
/// A pass that starts within a demand's set (`Domain::Within`, decision 0425) has it in its anchor, as on the CPU.
pub(in crate::code_rule_dsl) fn device_anchor(program: &Program, demand: Option<&compile_sets::Set>, unknown: &dyn Fn(u16) -> bool) -> Option<std::sync::Arc<DeviceFormula>> {
    let walk = Micro::of(program)?;
    let anchor = compile_sets::Set::anchor(&walk.words, &|_| true);
    let anchor = match demand {
        Some(demand) => compile_sets::Set::all_of(vec![anchor, demand.clone()]),
        None => anchor,
    };
    let mut formula = narrowing(&anchor.labels_unknown(unknown))?;
    formula.texts = (0..walk.texts.len()).map(|slot| walk.slot_key(program, slot)).collect();
    Some(std::sync::Arc::new(formula))
}

/// A fused pack's anchor: the union of its members' anchors, `None` where one has none. Each member's text tests are
/// kept, its slots past the members' before it, by their keys (`DeviceFormula::texts`): the device reads the row the
/// pack's walk (or another walk) reads for the key, every node where none does.
fn pack_anchor(programs: &[&Program]) -> Option<std::sync::Arc<DeviceFormula>> {
    let (mut members, mut texts) = (Vec::new(), Vec::new());
    for program in programs {
        let walk = Micro::of(program)?;
        let set = compile_sets::Set::anchor(&walk.words, &|_| true);
        narrowing(&set)?;
        members.push(set.shift_texts(u16::try_from(texts.len()).ok()?));
        texts.extend((0..walk.texts.len()).map(|slot| walk.slot_key(program, slot)));
    }
    let mut union = narrowing(&compile_sets::Set::Or(members))?;
    union.texts = texts;
    Some(std::sync::Arc::new(union))
}

/// A roots pass's program as a device formula (decision 0036 on Metal, 0365 §9): its micro walk's formula (`Set::of`,
/// exact), where the device evaluates it exactly: no text test (a formula pass reads no text row) and no targets image.
pub(in crate::code_rule_dsl) fn device_formula(program: &Program) -> Option<std::sync::Arc<DeviceFormula>> {
    use compile_sets::device_ops;
    let walk = Micro::of(program)?;
    let formula = compile_sets::Set::of(&walk.words)?.device()?;
    (!formula.words.iter().any(|&word| word & 255 == device_ops::TEXT)).then(|| std::sync::Arc::new(formula))
}

/// A targets pass's program as a device formula (decision 0036 on Metal, 0365 §10): the image of its starts under its
/// micro walk (`Set::targets`, exact where the capture is the walk's last act), the starts its rule over codes (kinds
/// and entry tests) as tests the formula reads. No text test, as `device_formula`; listed roots are not a formula's.
pub(in crate::code_rule_dsl) fn device_targets(program: &Program, roots: &Roots) -> Option<std::sync::Arc<DeviceFormula>> {
    use compile_sets::{Set, device_ops};
    let Roots::Codes { kinds, entry, anchor: None, .. } = roots else { return None };
    let walk = Micro::of(program)?;
    let mut starts = entry.iter().map(|&(op, arg)| if op == super::super::KIND { Set::Kind(arg) } else { Set::Class(arg) }).collect::<Vec<_>>();
    if let Some(kinds) = kinds {
        starts.push(Set::Or(kinds.iter().map(|&kind| Set::Kind(kind)).collect()));
    }
    let formula = Set::targets(&walk.words, Set::And(starts))?.device()?;
    (!formula.words.iter().any(|&word| word & 255 == device_ops::TEXT)).then(|| std::sync::Arc::new(formula))
}

/// `set` as the device evaluates it, where it tests more than kinds and grammar supertypes (which a rule over codes and
/// a walk's entry test already): an axis step, a label, a field, flags or text.
fn narrowing(set: &compile_sets::Set) -> Option<DeviceFormula> {
    use compile_sets::device_ops;
    let formula = set.device()?;
    let narrows = formula.words.iter().any(|&word| {
        let (op, arg) = (word & 255, word >> 8);
        !matches!(op, device_ops::ALL | device_ops::KIND | device_ops::AND | device_ops::OR) && !(op == device_ops::CLASS && arg < u32::from(DERIVED_CLASS_BASE))
    });
    narrows.then_some(formula)
}

/// The flushed roots as `RootsFound`: each record to its item's (program, file, task), its
/// nodes back to the file's, sorted by task and the walk's order.
/// Root programs walked as one (`Runner::records`).
struct RootsPack {
    /// The member programs, by index in the jobs.
    members: Vec<usize>,
    /// By the fused emit label: the member and its own label (`fusion::Fused::emits`).
    emits: Vec<(usize, u16)>,
    /// Per member, where its capture labels start.
    captures: Vec<u16>,
    /// Per member, per file, its task by (file-local) start.
    tasks: Vec<Vec<std::collections::HashMap<u32, u32>>>,
}

fn decode_roots(roots: RootsEncoded, files: &[TapeDoc]) -> RootsFound {
    use super::super::RootRecord;
    let read = |buffer: &MappedBuffer, count: usize| unsafe { std::slice::from_raw_parts(buffer.buffer.contents().cast::<u32>(), count) };
    let cursor = read(&roots.cursor, 2);
    let (written, overflow) = (cursor[0] as usize, cursor[1] != 0);
    let programs = roots.walked.len();
    let failures = read(&roots.failures, (programs + roots.packs.len()).max(1));
    if overflow || written > roots.capacity {
        crate::code_rule_dsl::prof::count("gpu.root_overflow", 1);
        return vec![None; roots.walked.len()];
    }
    let bases = files
        .iter()
        .scan(0_u32, |base, tape| {
            let at = *base;
            *base += tape.transducer_len() as u32;
            Some(at)
        })
        .collect::<Vec<_>>();
    type Staged = Vec<Option<Vec<Vec<((u32, u32), RootRecord)>>>>;
    let mut found: Staged = roots
        .walked
        .iter()
        .enumerate()
        .map(|(program, &walked)| (walked && failures[program] == 0).then(|| vec![Vec::new(); files.len()]))
        .collect();
    // A pack that failed anywhere: its members run on the CPU.
    for (index, pack) in roots.packs.iter().enumerate() {
        if failures[programs + index] != 0 {
            pack.members.iter().for_each(|&member| found[member] = None);
        }
    }
    let mut sorted = Vec::<()>::with_capacity(written);
    let words = read(&roots.records, written * ROOT_RECORD);
    for record in words.chunks_exact(ROOT_RECORD) {
        let [pass, file, task, start] = roots.items[record[0] as usize];
        let base = bases[file as usize];
        // A pack's record: its member's program, task, label and capture labels.
        let (program, task, label, capture_base) = match roots.packs.get((pass as usize).wrapping_sub(programs)) {
            Some(pack) if pass as usize >= programs => {
                let (member, label) = pack.emits[record[2] as usize];
                let Some(&task) = pack.tasks[member][file as usize].get(&(start - base)) else { continue };
                (pack.members[member], task, label, pack.captures[member])
            }
            _ => (pass as usize, task, record[2] as u16, 0),
        };
        let Some(per_file) = found[program].as_mut() else { continue };
        let captured = record[3].min(8) as u8;
        let mut captures = [(0_u16, u32::MAX); 8];
        for at in 0..usize::from(captured) {
            captures[at] = (record[12 + at] as u16 - capture_base, record[4 + at] - base);
        }
        sorted.push(());
        per_file[file as usize].push(((task, record[1]), RootRecord { start: start - base, label, captured, captures }));
    }
    crate::code_rule_dsl::prof::count("gpu.root_records", sorted.len());
    found
        .into_iter()
        .map(|per_file| {
            per_file.map(|per_file| {
                per_file
                    .into_iter()
                    .map(|mut records: Vec<((u32, u32), RootRecord)>| {
                        records.sort_unstable_by_key(|(key, _)| *key);
                        records.into_iter().map(|((task, _), record)| (task, record)).collect()
                    })
                    .collect()
            })
        })
        .collect()
}

/// The roots' device state between encoding and the flush (`Runner::records`).
struct RootsEncoded {
    /// Per program whether it runs on the device.
    walked: Vec<bool>,
    /// The packs, whose passes follow the programs'.
    packs: Vec<RootsPack>,
    /// Per item (program, file, task, global start).
    items: Vec<[u32; 4]>,
    records: MappedBuffer,
    cursor: MappedBuffer,
    failures: MappedBuffer,
    capacity: usize,
}

/// What one run over a labelling's files leaves for the next, since only the labels and the
/// steps change between them (06 S7: 9 runs each rebuilt and uploaded the same tapes, and
/// allocated the same scratch, counts and stacks).
#[derive(Default)]
pub(in crate::code_rule_dsl) struct RunBuffers(Mutex<Option<Kept>>);

struct Kept {
    /// (nodes, labels, counts) the buffers are sized for.
    key: (usize, usize, usize),
    stacks: Option<MappedBuffer>,
}

/// Each distinct text test's (`Micro::slot_key`) row of bits over a run's files, `None` where its
/// evaluation failed: kept across the runs of one labelling (06 S7: 58 evaluations in 9 runs).
pub(in crate::code_rule_dsl) type TextRows = Mutex<HashMap<String, Option<std::sync::Arc<Vec<u32>>>>>;

/// Slot `slot`'s bits over every file's nodes (files from `bases`, `words` words), in parallel;
/// `None` where an evaluation failed.
pub(in crate::code_rule_dsl) fn text_row(
    walk: &Micro,
    program: &Program,
    slot: usize,
    files: &[TapeDoc],
    bases: &[u32],
    words: usize,
) -> Option<std::sync::Arc<Vec<u32>>> {
    use rayon::prelude::*;
    let _texts = crate::code_rule_dsl::prof::Stage::start("size.texts");
    // Each job in its caller's run (decision 0442): a slot's text test reads the run's lowered texts.
    let caller = rerail_prof::code_rule_dsl::logic::profile::caller();
    let per_file = files
        .par_iter()
        .map(|tape| {
            let _in = caller.enter();
            walk.slot(program, slot, tape)
        })
        .collect::<Option<Vec<_>>>()?;
    let mut bits = vec![0_u32; words];
    for (file, base) in per_file.iter().zip(bases) {
        for (index, &word) in file.iter().enumerate() {
            let mut word = word;
            while word != 0 {
                let node = *base + (index * 64) as u32 + word.trailing_zeros();
                bits[node as usize / 32] |= 1 << (node % 32);
                word &= word - 1;
            }
        }
    }
    Some(std::sync::Arc::new(bits))
}

/// Each of `slots`' rows (`text_row`), in one parallel pass over every (slot, file): a
/// labelling's slots take one pass, not one each (06 S7: 54 passes, ~6 ms each, before its
/// first run), and a large file's slots spread over the threads.
pub(in crate::code_rule_dsl) fn text_rows(
    slots: &[(&Micro, &Program, usize)],
    files: &[TapeDoc],
    bases: &[u32],
    words: usize,
) -> Vec<Option<std::sync::Arc<Vec<u32>>>> {
    use rayon::prelude::*;
    let _texts = crate::code_rule_dsl::prof::Stage::start("size.texts");
    crate::code_rule_dsl::prof::count("size.text_slots", slots.len());
    // Each (slot, file)'s bits, `None` where an evaluation failed: one task each, so a large
    // file's slots spread over the threads.
    // Each job in its caller's run (decision 0442): a slot's text test reads the run's lowered texts.
    let caller = rerail_prof::code_rule_dsl::logic::profile::caller();
    let per_pair = (0..slots.len() * files.len())
        .into_par_iter()
        .map(|pair| {
            let _in = caller.enter();
            let (walk, program, slot) = slots[pair / files.len()];
            walk.slot(program, slot, &files[pair % files.len()])
        })
        .collect::<Vec<_>>();
    (0..slots.len())
        .into_par_iter()
        .map(|at| {
            let mut bits = vec![0_u32; words];
            for (file, base) in per_pair[at * files.len()..(at + 1) * files.len()].iter().zip(bases) {
                for (index, &word) in file.as_ref()?.iter().enumerate() {
                    let mut word = word;
                    while word != 0 {
                        let node = *base + (index * 64) as u32 + word.trailing_zeros();
                        bits[node as usize / 32] |= 1 << (node % 32);
                        word &= word - 1;
                    }
                }
            }
            Some(std::sync::Arc::new(bits))
        })
        .collect()
}

/// Slot `slot` of `walk` on the file whose nodes start at global `base`, from `rows` where a run
/// evaluated it (`TextRows`: bits over every file's nodes), else evaluated on `tape` alone.
pub(in crate::code_rule_dsl) fn text_slot(
    rows: Option<(&TextRows, usize)>,
    walk: &Micro,
    program: &Program,
    slot: usize,
    tape: &TapeDoc,
) -> Option<Vec<u64>> {
    if let Some((rows, base)) = rows
        && let Some(Some(bits)) = rows.lock().ok()?.get(&walk.slot_key(program, slot)).cloned()
    {
        let len = tape.transducer_len();
        let at = |bit: usize| -> u64 {
            let word = |i: usize| u128::from(bits.get(i).copied().unwrap_or(0));
            let i = bit / 32;
            ((word(i) | word(i + 1) << 32 | word(i + 2) << 64) >> (bit % 32)) as u64
        };
        crate::code_rule_dsl::prof::count("sweep.text_hits", 1);
        return Some(
            (0..len.div_ceil(64))
                .map(|k| {
                    let value = at(base + 64 * k);
                    let left = len - 64 * k;
                    if left < 64 { value & ((1 << left) - 1) } else { value }
                })
                .collect(),
        );
    }
    crate::code_rule_dsl::prof::count("sweep.text_misses", 1);
    walk.slot(program, slot, tape)
}

/// Decision 0024 §3: a recursive group runs on the device, to its fixpoint in one dispatch where
/// its passes are micro walks (`Runner::group`), else as rounds of levels. The host's sweeps
/// between the GPU's segments (O26) stay as the reference, `RERAIL_HOST_SWEEPS`.
pub(in crate::code_rule_dsl) fn host_sweeps() -> bool {
    rerail_prof::code_rule_dsl::logic::profile::setting(|| std::env::var_os("RERAIL_HOST_SWEEPS").is_some())
}

/// Whether a recursive group runs as sweeps: its passes are programs (or empty), whose results only
/// add to their labels, or complements as well, where its programs mark their roots (a node's
/// verdicts are then all its own, computed at it in a reverse sweep).
fn sweeps(levels: &[Vec<Pass<'_>>]) -> bool {
    if !host_sweeps() {
        return false;
    }
    let passes = || levels.iter().flatten();
    passes().all(|pass| matches!(pass, Pass::Program { .. } | Pass::Empty { .. } | Pass::Complement { .. }))
        && passes().any(|pass| matches!(pass, Pass::Program { .. }))
        && (!complements(levels) || passes().all(|pass| !matches!(pass, Pass::Program { captured: true, .. })))
}

/// A group with complements sweeps in reverse preorder only: its walks move forward in preorder.
fn complements(levels: &[Vec<Pass<'_>>]) -> bool {
    levels.iter().flatten().any(|pass| matches!(pass, Pass::Complement { .. }))
}

/// Sweeps encoded per group: a recursion that runs one way settles in the first or second (the
/// directions alternate), one that turns once in the third, and one more sees nothing change.
const SWEEPS: usize = 4;

/// A label level's interpreter lanes, and the states of each transient kind a task may hold
/// (O28). A lane takes tasks from the level's cursor, one after another, and reserves its states
/// up front, so lanes times states is the arena: 64 lanes of 65,536 states left a level of 10^5
/// tasks on two SIMD groups. 1,024 lanes of 4,096 states hold the same records, sixteen times as
/// wide. A task that needs more fails its batch, which runs again as before (`WIDE`).
const NARROW: (usize, Option<u32>) = (1024, Some(4096));
/// The program backend's lanes and states: every task's worst case.
const WIDE: (usize, Option<u32>) = (EXECUTION_LANES, None);
/// The fewest states per lane a run over the buffer budget takes (`Error::BackendBudget`).
const MIN_STATES: u32 = 512;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Shape {
    node_count: u32,
    words: u32,
    result_arena: u32,
    task_count: u32,
    item_count: u32,
    target_count: u32,
    round: u32,
    finish: u32,
    recursive: u32,
    last_level: u32,
    file_count: u32,
    /// A micro walk's steps (`compile_micro::steps` for the largest file and program).
    steps: u32,
    /// Profiling (`RERAIL_GPU_WALK_STEPS`): 1 + the dispatch's first pass's place in its stats
    /// buffer (bound at `WALK_STATS_SLOT`); 0 off.
    profile: u32,
    /// The spill queue's capacity in frames (`MicroQ`, decision 0125); 0 no spill.
    queue: u32,
    /// A walk's steps between spills (`SPILL_BUDGET`).
    budget: u32,
    /// The lanes a micro walk's item takes (`Runner::walk`, `ROOT_LANES`): a power of two up to 32; 0 is 1.
    lanes: u32,
    /// A walk's frames, its stack in `stacks` (`label_micro`, `label_group`, `root_records`: the tier's).
    frames: u32,
}

/// A walk's steps before it spills a frame, and between spills: well past a short walk's (most take
/// under 20), well under a long one's (a module's descendants, 10^5).
const SPILL_BUDGET: u32 = 512;

#[cfg(any(test, feature = "test-hooks"))]
thread_local! {
    /// Tests: the spill budget (`SPILL_BUDGET`), low so every walk spills.
    pub static SPILL_BUDGET_TEST: std::cell::Cell<Option<u32>> = const { std::cell::Cell::new(None) };
}

fn spill_budget() -> u32 {
    #[cfg(any(test, feature = "test-hooks"))]
    if let Some(budget) = SPILL_BUDGET_TEST.get() {
        return budget;
    }
    SPILL_BUDGET
}

/// Where a micro kernel binds its spill queue's slots, control and items (`MicroQ`).
const SPILL_SLOTS: [u64; 3] = [23, 24, 25];
/// The spill queue's frames: a long walk's frames past its step budget (`MICRO_BUDGET`).
const SPILL_FRAMES: usize = 1 << 15;
/// A spilled frame's words (`MICRO_ITEM`).
const SPILL_ITEM: usize = 20;

/// Whether long micro walks spill their frames to other threads (`RERAIL_MICRO_SPILL=off`: not).
fn spill_on() -> bool {
    rerail_prof::code_rule_dsl::logic::profile::setting(|| std::env::var("RERAIL_MICRO_SPILL").map_or(true, |v| v != "off"))
}

/// Where a micro kernel binds its walk statistics (`Shape::profile`): below the slabs.
const WALK_STATS_SLOT: u64 = 26;

/// Profiling (`RERAIL_GPU_WALK_STEPS`): each micro pass's walks, steps and longest walk, by its
/// words, in the profile's stages (`walk_steps.`, `walk_items.`, `walk_max.`).
fn walk_steps_on() -> bool {
    rerail_prof::code_rule_dsl::logic::profile::setting(|| std::env::var_os("RERAIL_GPU_WALK_STEPS").is_some())
}

/// A micro walk's words as text: what a profile names the walk by.
fn describe_walk(tier: usize, words: &[u32]) -> String {
    format!("t{tier}: {}", super::super::compile_micro::describe(words))
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Op {
    kind: u32,
    target: u32,
    of: u32,
    slot: u32,
    minimum: u32,
    maximum: u32,
    /// The target's scratch row: its pass's index in the level (`scratch_rows`).
    row: u32,
}

/// Scratch rows: a level's passes mark row `i` for its pass `i`, and `label_settle` clears them,
/// so scratch is the widest level's, not a row per label (O30: every bound buffer is charged
/// twice; S7's 833-label scratch was 33 MB). A group that runs as sweeps publishes its labels
/// directly (`label_sweep`) and marks no scratch.
/// (Where a plan has such a group, scratch keeps a row per label, as its routes name labels.)
/// The labels a run reads or writes, in the GPU's label prefix by row: (row -> label, label ->
/// row, `u32::MAX` where absent). The others stay on the host: a run's passes read and write a
/// few of a labelling's labels, and every bound buffer is charged twice (O30: S7's prefix was
/// all 833, ~34 MB a run). `None` (every label, by its index) where a pass may read labels by
/// index otherwise: an interpreter task, or a group that runs as sweeps.
fn label_rows(plan: &Plan<'_>, micro: bool) -> Option<(Vec<u32>, Vec<u32>)> {
    if !micro || plan.steps.iter().any(|(recursive, levels)| *recursive && sweeps(levels)) {
        return None;
    }
    let mut used = vec![false; plan.labels];
    let mut mark = |label: u32| -> Option<()> {
        *used.get_mut(label as usize)? = true;
        Some(())
    };
    for pass in plan.steps.iter().flat_map(|(_, levels)| levels).flatten() {
        match pass {
            Pass::Program { target, program, roots, .. } | Pass::Count { target, program, roots, .. } => {
                mark(*target)?;
                // The labels its anchor reads (decision 0365).
                for &label in roots.anchor().iter().flat_map(|anchor| &anchor.labels) {
                    mark(label)?;
                }
                for &word in &Micro::of(program)?.words {
                    let (op, _, arg) = super::super::split(word);
                    if op == super::super::SUPERTYPE && arg >= DERIVED_CLASS_BASE {
                        mark(u32::from(arg - DERIVED_CLASS_BASE))?;
                    }
                }
            }
            Pass::Complement { target, of } => {
                mark(*target)?;
                mark(*of)?;
            }
            Pass::Pattern { target, constraints, .. } => {
                mark(*target)?;
                for (_, label) in constraints {
                    mark(*label)?;
                }
            }
            Pass::Fact { target, labels, .. } => {
                mark(*target)?;
                for label in labels {
                    mark(*label)?;
                }
            }
            Pass::Empty { target } => mark(*target)?,
            Pass::Formula { target, formula, .. } => {
                mark(*target)?;
                for &label in &formula.labels {
                    mark(label)?;
                }
            }
        }
    }
    // The root programs read their labels after the passes (those walked on the host read the
    // host's copy).
    for program in plan.roots.iter().flat_map(|roots| &roots.programs) {
        let Some(walk) = Micro::of(program) else { continue };
        for &word in &walk.words {
            let (op, _, arg) = super::super::split(word);
            if op == super::super::SUPERTYPE && arg >= DERIVED_CLASS_BASE {
                mark(u32::from(arg - DERIVED_CLASS_BASE))?;
            }
        }
    }
    let present = used.iter().enumerate().filter(|(_, used)| **used).map(|(label, _)| label as u32).collect::<Vec<_>>();
    if present.len() == plan.labels {
        return None;
    }
    let mut row_of = vec![u32::MAX; plan.labels];
    for (row, &label) in present.iter().enumerate() {
        row_of[label as usize] = row as u32;
    }
    Some((present, row_of))
}

/// Each of a level's passes' count row: its index among the level's counts. A level's settle
/// consumes and clears its counts, so `counts` is the level with the most counts' (O30: a row
/// per count of the plan was 24 MB on S7).
/// How a pass's target settles (`label_settle`'s `Op`): `None` for a pattern's, whose level
/// matches its pattern first. `place`: a label's row in the prefix.
fn settle(pass: &Pass<'_>, row: u32, tally: u32, place: impl Fn(&u32) -> u32) -> Option<Op> {
    Some(match pass {
                Pass::Program { target, .. } | Pass::Empty { target } | Pass::Formula { target, .. } => Op {
                    target: place(target),
                    row,
                    ..Op::default()
                },
                Pass::Count {
                    target,
                    min,
                    max,
                    ..
                } => Op {
                    kind: 2,
                    target: place(target),
                    slot: tally,
                    minimum: *min,
                    maximum: *max,
                    of: 0,
                    row,
                },
                Pass::Complement { target, of } => Op {
                    kind: 1,
                    target: place(target),
                    of: place(of),
                    row,
                    ..Op::default()
                },
        Pass::Pattern { .. } | Pass::Fact { .. } => return None,
    })
}

fn count_rows(passes: &[Pass<'_>]) -> Vec<u32> {
    passes
        .iter()
        .scan(0_u32, |next, pass| {
            let row = *next;
            *next += u32::from(matches!(pass, Pass::Count { .. }));
            Some(row)
        })
        .collect()
}

/// `count_rows`' widest level (a row per count of the plan where a group runs as sweeps).
fn tally_rows(plan: &Plan<'_>) -> usize {
    if plan.steps.iter().any(|(recursive, levels)| *recursive && sweeps(levels)) {
        return plan.counts.max(1);
    }
    let counts = |level: &Vec<Pass<'_>>| level.iter().filter(|pass| matches!(pass, Pass::Count { .. })).count();
    plan.steps.iter().flat_map(|(_, levels)| levels).map(counts).max().unwrap_or(0).max(1)
}

fn scratch_rows(plan: &Plan<'_>) -> usize {
    if plan.steps.iter().any(|(recursive, levels)| *recursive && sweeps(levels)) {
        return plan.labels.max(1);
    }
    plan.steps.iter().flat_map(|(_, levels)| levels).map(Vec::len).max().unwrap_or(0).max(1)
}

/// The device's tape view (decision 0032), which the label and pattern kernels read: the
/// document table's entry as generated (`slabs::msl`, decisions 0289 §4 and 0310), the reads written once in CLIF,
/// then the readers. The reads are format 15's and 16's (`<ukl/reads/tape.h>`), or 16z's too, through the
/// batch's cache (`<ukl/reads/tape_z.h>`, decision 0353 step 4b) for a batch of 16z or of mixed formats
/// ([`tape_view`]): a pipeline's 16z bodies are compiled though its format makes them dead, and doubled a format
/// 15 label kernel's compile.
///
/// The batch's tape format is the program's specialization constant (`tape_format`, `UKL_CONSTANT_U32`: Metal's
/// function constant 0, a define elsewhere; 0 for a mixed batch): each view load of the format field reads it, so a
/// batch of one format runs reads without the other format's branches (t41: label_micro 388 to about 300 ms at
/// s1000). Every kernel of these sources is made with a value ([`pipeline_of`], [`pipelines`]).
static VIEW_PIECES: std::sync::LazyLock<[String; 2]> = std::sync::LazyLock::new(|| {
    ["#include <ukl/reads/tape.h>\n", "#include <ukl/reads/tape_z.h>\n"].map(|reads| {
        format!(
            "{}UKL_CONSTANT_U32(tape_format)\n#define clif_view(v,a) ((tape_format!=0u&&(a)=={}u)?tape_format:(v)[(a)/4u])\n{}{}",
            crate::code_rule_dsl::residency::slabs::msl(),
            std::mem::offset_of!(crate::code_rule_dsl::residency::slabs::DocView, format),
            reads,
            include_str!("device_tape.h")
        )
    })
});
/// The view as the pattern kernels read it: Metal's, until step D writes them in UKL (decision 0440).
static TAPE_VIEWS: std::sync::LazyLock<[String; 2]> =
    std::sync::LazyLock::new(|| VIEW_PIECES.clone().map(|pieces| format!("#include <metal_stdlib>\nusing namespace metal;\n{pieces}")));
/// Whether a batch of tapes in `format` (0: mixed) reads 16z ([`TAPE_VIEWS`]).
fn reads_z(format: u32) -> bool {
    format != 15 && format != 16
}
/// Which tape view a batch of `format` reads (`TAPE_VIEWS`' index): the kernels' sources differ by it alone.
pub(in crate::code_rule_dsl) fn pattern_variant(format: u32) -> usize {
    usize::from(reads_z(format))
}
/// The tape view for a batch of tapes in `format` (0: mixed).
fn tape_view(format: u32) -> &'static str {
    &TAPE_VIEWS[usize::from(reads_z(format))]
}
/// The label kernels (UKL, decision 0440 step C) over the tape view, for a batch of either kind ([`tape_view`]).
static SOURCES: std::sync::LazyLock<[String; 2]> = std::sync::LazyLock::new(|| {
    [15, 0].map(|format| format!("{}{}", VIEW_PIECES[usize::from(reads_z(format))], include_str!("label_passes.uglykl")))
});
/// Where a pattern kernel binds the batch's chunk block (`DocView`'s `at`): past every other buffer. A label kernel
/// binds it by its row's name (`LabelKernel`).
const SLAB_SLOT: u64 = 27;

/// A label kernel as the runner encodes it: its pipeline, and the arguments its positional buffers do not reach
/// (the batch's block, the walk statistics, the spill queue), where its rows put them (`label_passes.uglykl`); none
/// where the kernel has no such row (`label_round` reads no document).
#[derive(Clone)]
struct LabelKernel {
    state: ComputePipelineState,
    chunk: Option<u64>,
    stats: Option<u64>,
    spill: [Option<u64>; 3],
}

impl LabelKernel {
    fn of(kernel: &rerail_metal::lane::Kernel) -> LabelKernel {
        let at = |name: &str| kernel.get_argument_info().iter().find(|a| a.name == name).map(|a| a.compute_function_arg_index);
        LabelKernel {
            state: kernel.metal().expect("a label kernel on Metal").compute_pipeline_state.clone(),
            chunk: at("chunk"),
            stats: at("wstats"),
            spill: [at("qslots"), at("qcontrol"), at("qitems")],
        }
    }

    /// A pattern kernel (Metal's until step D): its fixed slots.
    fn pattern(state: ComputePipelineState) -> LabelKernel {
        LabelKernel { state, chunk: Some(SLAB_SLOT), stats: Some(WALK_STATS_SLOT), spill: SPILL_SLOTS.map(Some) }
    }
}
/// A walk's stack tiers are one kernel each (`label_micro`, `label_group`, `root_records`), their frames the dispatch's
/// shape's (`Shape::frames`), not a template's: an Apple GPU's compiler compiled each tier's copy of the walk on its own,
/// 8–10 s each for format 16 (work queue Q181).
const KERNELS: [&str; 9] = ["label_sink", "label_settle", "label_round", "label_micro", "label_group", "root_records", "label_leaves", "label_phase", "label_parents"];
/// `label_leaves` in `KERNELS` (decision 0365).
const LEAVES_KERNEL: usize = 6;
/// `label_phase` (decisions 0461, 0463): a level's formulas, a dispatch a phase, a thread a (word, formula).
const PHASE_KERNEL: usize = 7;
/// `label_parents` (0463): the batch's parents once, for `label_phase`.
const PARENTS_KERNEL: usize = 8;
/// `RERAIL_GPU_ENCODER_TIMES=1` (a measurement switch): each compute encoder's own time on the GPU, from timestamps the
/// GPU takes at its start and end inside its command buffer (stage-boundary counters), and the gaps between one
/// encoder's end and the next one's start. `RERAIL_GPU_KERNEL_TIMES` times a command buffer a kernel, committed and
/// waited for alone, so its numbers hold the command buffer's own cost too; this one holds only the encoder's. As
/// `encoder_us.<label>` and `encoder_n.<label>`, and `encoder_gap_us`.
struct EncoderTimes {
    device: metal::Device,
    samples: metal::CounterSampleBuffer,
    /// The labels of the encoders sampled in the current command buffer, in order: encoder `k` at samples `2k`, `2k+1`.
    labels: Vec<String>,
    /// GPU ticks against host nanoseconds, from two `sampleTimestamps` (at the start, and at each read).
    calibration: (u64, std::time::Instant),
}

/// Samples a command buffer holds: two an encoder, for at most this many encoders (a submission holds
/// `KERNEL_LAUNCH_LIMIT` launches).
const ENCODER_SAMPLES: usize = 2048;

impl EncoderTimes {
    fn new(device: &metal::DeviceRef) -> Option<Self> {
        if !rerail_prof::code_rule_dsl::logic::profile::setting(|| std::env::var_os("RERAIL_GPU_ENCODER_TIMES").is_some()) {
            return None;
        }
        let Some(set) = device.counter_sets().into_iter().find(|set| set.name() == "timestamp") else {
            eprintln!("RERAIL_GPU_ENCODER_TIMES: this GPU has no timestamp counter set");
            return None;
        };
        if !device.supports_counter_sampling(metal::MTLCounterSamplingPoint::AtStageBoundary) {
            eprintln!("RERAIL_GPU_ENCODER_TIMES: this GPU samples no counters at stage boundaries");
            return None;
        }
        let descriptor = metal::CounterSampleBufferDescriptor::new();
        descriptor.set_counter_set(&set);
        descriptor.set_sample_count(ENCODER_SAMPLES as u64);
        descriptor.set_storage_mode(metal::MTLStorageMode::Shared);
        let samples = device.new_counter_sample_buffer_with_descriptor(&descriptor).map_err(|error| eprintln!("RERAIL_GPU_ENCODER_TIMES: {error}")).ok()?;
        let (mut cpu, mut gpu) = (0, 0);
        device.sample_timestamps(&mut cpu, &mut gpu);
        Some(Self { device: device.to_owned(), samples, labels: Vec::new(), calibration: (gpu, std::time::Instant::now()) })
    }

    /// The pass descriptor of the next encoder, sampled at its start and end; `None` where the buffer is full.
    fn next(&mut self, label: &str) -> Option<&metal::ComputePassDescriptorRef> {
        let at = 2 * self.labels.len();
        if at + 2 > ENCODER_SAMPLES {
            return None;
        }
        self.labels.push(label.to_owned());
        let descriptor = metal::ComputePassDescriptor::new();
        let attachment = descriptor.sample_buffer_attachments().object_at(0)?;
        attachment.set_sample_buffer(&self.samples);
        attachment.set_start_of_encoder_sample_index(at as u64);
        attachment.set_end_of_encoder_sample_index(at as u64 + 1);
        Some(descriptor)
    }

    /// The finished command buffer's encoders' times, into the profile's counters; the buffer is free again.
    fn read(&mut self) {
        if self.labels.is_empty() {
            return;
        }
        // `resolveCounterRange:` gives an NSData of u64 timestamps (metal-rs's own copies none of its bytes).
        unsafe extern "C" {
            fn sel_registerName(name: *const std::ffi::c_char) -> *mut std::ffi::c_void;
            fn objc_msgSend();
        }
        #[repr(C)]
        struct Range {
            location: u64,
            length: u64,
        }
        let count = 2 * self.labels.len();
        let resolve: unsafe extern "C" fn(*const std::ffi::c_void, *mut std::ffi::c_void, Range) -> *const std::ffi::c_void =
            unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
        let bytes: unsafe extern "C" fn(*const std::ffi::c_void, *mut std::ffi::c_void) -> *const u64 =
            unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
        let length: unsafe extern "C" fn(*const std::ffi::c_void, *mut std::ffi::c_void) -> u64 =
            unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
        let object = std::ptr::from_ref::<metal::CounterSampleBufferRef>(&self.samples).cast::<std::ffi::c_void>();
        let data = unsafe { resolve(object, sel_registerName(c"resolveCounterRange:".as_ptr()), Range { location: 0, length: count as u64 }) };
        let labels = std::mem::take(&mut self.labels);
        if data.is_null() || unsafe { length(data, sel_registerName(c"length".as_ptr())) } < 8 * count as u64 {
            crate::code_rule_dsl::prof::count("encoder_unresolved", labels.len());
            return;
        }
        let stamps = unsafe { std::slice::from_raw_parts(bytes(data, sel_registerName(c"bytes".as_ptr())), count) };
        let (mut cpu, mut gpu) = (0, 0);
        self.device.sample_timestamps(&mut cpu, &mut gpu);
        let (gpu0, at0) = self.calibration;
        let ns_a_tick = at0.elapsed().as_nanos() as f64 / gpu.saturating_sub(gpu0).max(1) as f64;
        crate::code_rule_dsl::prof::count("encoder_ps_a_tick", (ns_a_tick * 1000.0) as usize);
        let mut last_end = None;
        for (k, label) in labels.iter().enumerate() {
            let (start, end) = (stamps[2 * k], stamps[2 * k + 1]);
            // A sample the GPU did not take reads as an error value (`MTLCounterErrorValue`, all ones) or zero.
            if start == 0 || end == 0 || start == u64::MAX || end == u64::MAX || end < start {
                crate::code_rule_dsl::prof::count("encoder_unsampled", 1);
                continue;
            }
            crate::code_rule_dsl::prof::count_named(format!("encoder_us.{label}"), ((end - start) as f64 * ns_a_tick / 1000.0) as usize);
            crate::code_rule_dsl::prof::count_named(format!("encoder_n.{label}"), 1);
            if let Some(previous) = last_end
                && start >= previous
            {
                crate::code_rule_dsl::prof::count("encoder_gap_us", ((start - previous) as f64 * ns_a_tick / 1000.0) as usize);
            }
            last_end = Some(end);
        }
    }
}

/// A compute encoder on `command`, sampled for `RERAIL_GPU_ENCODER_TIMES` where it is on.
fn encoder_of<'c>(command: &'c metal::CommandBufferRef, times: &mut Option<EncoderTimes>, label: &str) -> &'c metal::ComputeCommandEncoderRef {
    match times.as_mut().and_then(|times| times.next(label)) {
        Some(descriptor) => command.compute_command_encoder_with_descriptor(descriptor),
        None => command.new_compute_command_encoder(),
    }
}

/// A formula compiled for `label_phase` (decision 0461): its instructions (`[code, dst, src, extra]`, the kernel's
/// `PHASE_*`), its phases, the slot of its set, and its slots.
#[derive(Debug, PartialEq)]
struct PhaseProgram {
    code: Vec<[u32; 4]>,
    phases: u32,
    result: u32,
    slots: u32,
}

/// The kernel's instructions (`label_passes.uglykl`'s `PHASE_*`).
const PHASE_END: u32 = 0;
const PHASE_ROW: u32 = 1;
const PHASE_AND: u32 = 2;
const PHASE_OR: u32 = 3;
const PHASE_ZERO: u32 = 4;
const PHASE_UP: u32 = 5;
const PHASE_DOWN: u32 = 6;
const PHASE_ZERO32: u32 = 8;
const PHASE_LAST: u32 = 9;
const PHASE_BEFORE: u32 = 10;
const PHASE_SPAN: u32 = 11;
const PHASE_COVER: u32 = 12;
const PHASE_BLOCKERS: u32 = 13;
const PHASE_ANDNOT: u32 = 14;
const PHASE_COPY: u32 = 15;
const PHASE_CHECK: u32 = 16;
const PHASE_CLIMB: u32 = 17;
/// `label_phase`'s threadgroup: its words (`PHASE_WORDS`), a thread each.
const PHASE_WORDS: u64 = 64;
/// The levels of a chain's reach (`REACH_CHAIN`) a run's programs hold at first, each two descendant scans: a member
/// below a blocker below a member is a level. A chain with more fails its run (flags bit 512), which runs again with
/// twice the levels (`Error::ChainLevels`, 0463). One held every chain of s1000.
const PHASE_CHAIN_LEVELS: usize = 1;
/// Past this many levels a chain's run is the interpreter's (as a walk past its frames).
const PHASE_CHAIN_MOST: usize = 64;

/// `words` (a lowered formula, `compile_sets::DeviceFormula`'s postfix) as `label_phase`'s program (decision 0463), where its steps
/// are rows, AND, OR, the one-hop steps (`CHILD`, `PARENT`, `CHILD_IN`), the siblings (`FOLLOWING`, `FOLLOWING_IN`,
/// `REACH_FOLLOWING`), descendants (`REACH_DESCENDANT`), a chain's reach (`REACH_CHAIN`, `levels` of it) and the
/// climbs (`DESCENDANT`, `DESCENDANT_IN`, `CHAIN`); else `None` (a word no device formula has).
///
/// Every value has a slot. A step that reads only its thread's word needs its operands settled there; a pull (the
/// parent's bit, a sibling array's entry, the blocks' ends) reads other words, so it needs them whole: written in an
/// earlier phase. A push sets bits in other words, into a slot cleared in phase 0, so its value is whole only in the
/// next phase. A slot is used again only after the phase that freed it. A node's parent is read where
/// `label_parents` wrote it, once a batch, before the first level (`label_parents`).
fn phase_program(words: &[u32], levels: usize) -> Option<PhaseProgram> {
    use compile_sets::device_ops as op;
    #[derive(Clone, Copy)]
    struct Value {
        slot: u32,
        // Pushed in this phase (others' threads set its bits or entries), the sibling array a pull is still to read
        // (and whether it holds the first, not the last), and the candidates it is still to be cut to.
        pushed: Option<u32>,
        pull: Option<(u32, u32)>,
        cand: Option<u32>,
        // The phase that last wrote its own words.
        written: u32,
    }
    struct Build {
        code: Vec<[u32; 4]>,
        zeros: Vec<[u32; 4]>,
        free: Vec<u32>,
        pending: Vec<u32>,
        slots: u32,
        phase: u32,
    }
    impl Build {
        fn take(&mut self) -> u32 {
            self.free.pop().unwrap_or_else(|| self.fresh(1))
        }
        /// `n` slots in a row, never freed (a node array is 32).
        fn fresh(&mut self, n: u32) -> u32 {
            self.slots += n;
            self.slots - n
        }
        fn end_phase(&mut self) {
            self.code.push([PHASE_END, 0, 0, 0]);
            self.phase += 1;
            self.free.append(&mut self.pending);
        }
        /// A push writes into slots phase 0 clears, so it comes after it.
        fn push_phase(&mut self) {
            if self.phase == 0 {
                self.end_phase();
            }
        }
        /// `value` readable in this thread's word: its pushes done, its pull read, cut to its candidates.
        fn settle(&mut self, value: &mut Value) {
            if value.pushed == Some(self.phase) {
                self.end_phase();
            }
            value.pushed = None;
            if let Some((array, first)) = value.pull.take() {
                self.code.push([PHASE_BEFORE, value.slot, array, first << 1]);
                value.written = self.phase;
            }
            if let Some(cand) = value.cand.take() {
                self.code.push([PHASE_AND, value.slot, cand, 0]);
                self.pending.push(cand);
                value.written = self.phase;
            }
        }
        /// `value` readable in every word: settled, and written before this phase.
        fn whole(&mut self, value: &mut Value) {
            self.settle(value);
            if value.written == self.phase {
                self.end_phase();
            }
        }
        /// The descendants of `level`'s members into a fresh slot: their ends this phase (into `pair`), the cover
        /// the next.
        fn descend(&mut self, level: u32, pair: (u32, u32)) -> u32 {
            self.code.push([PHASE_SPAN, pair.0, level, pair.1]);
            self.end_phase();
            let out = self.take();
            self.code.push([PHASE_COVER, out, level, pair.0 | pair.1 << 16]);
            out
        }
        fn value(&self, slot: u32) -> Value {
            Value { slot, pushed: None, pull: None, cand: None, written: self.phase }
        }
    }
    let mut b = Build { code: Vec::new(), zeros: Vec::new(), free: Vec::new(), pending: Vec::new(), slots: 0, phase: 0 };
    let mut stack = Vec::<Value>::new();
    for &word in words {
        let (kind, arg) = (word & 255, word >> 8);
        match kind {
            op::ALL | op::CLASS | op::NONE | op::LEAF | op::TEXT => {
                let slot = b.take();
                b.code.push([PHASE_ROW, slot, word, 0]);
                stack.push(b.value(slot));
            }
            op::AND | op::OR => {
                let n = arg as usize;
                if n == 0 || stack.len() < n {
                    return None;
                }
                let mut operands = stack.split_off(stack.len() - n);
                for value in &mut operands {
                    b.settle(value);
                }
                let dst = operands[0].slot;
                for other in &operands[1..] {
                    b.code.push([if kind == op::AND { PHASE_AND } else { PHASE_OR }, dst, other.slot, 0]);
                    b.pending.push(other.slot);
                }
                stack.push(b.value(dst));
            }
            op::CHILD | op::CHILD_IN | op::FOLLOWING | op::FOLLOWING_IN | op::REACH_FOLLOWING => {
                let mut inner = stack.pop()?;
                let cand = if kind == op::CHILD_IN || kind == op::FOLLOWING_IN {
                    let mut cand = stack.pop()?;
                    b.settle(&mut cand);
                    Some(cand.slot)
                } else {
                    None
                };
                b.settle(&mut inner);
                b.push_phase();
                b.pending.push(inner.slot);
                // `REACH_FOLLOWING` takes every member; the others their named ones where `arg` says.
                let named = if kind == op::REACH_FOLLOWING { 0 } else { arg & 1 };
                if kind == op::CHILD || kind == op::CHILD_IN {
                    let dst = b.fresh(1);
                    b.zeros.push([PHASE_ZERO, dst, 0, 0]);
                    b.code.push([PHASE_UP, dst, inner.slot, named]);
                    stack.push(Value { pushed: Some(b.phase), cand, ..b.value(dst) });
                } else {
                    // The members' parents' last (first) member child, then each node before (after) it.
                    let first = u32::from(kind == op::REACH_FOLLOWING);
                    let array = b.fresh(32);
                    b.zeros.push([PHASE_ZERO32, array, 0, 0]);
                    b.code.push([PHASE_LAST, array, inner.slot, named | first << 1]);
                    let dst = b.fresh(1);
                    stack.push(Value { pushed: Some(b.phase), pull: Some((array, first)), cand, ..b.value(dst) });
                }
            }
            op::DESCENDANT | op::DESCENDANT_IN | op::CHAIN => {
                // A climb from the members (named where `arg` says), each stopping at an ancestor another set; a
                // chain's goes on only through `through` (the set below `to`, read at the ancestors' words).
                let mut inner = stack.pop()?;
                let cand = if kind == op::DESCENDANT_IN {
                    let mut cand = stack.pop()?;
                    b.settle(&mut cand);
                    Some(cand.slot)
                } else {
                    None
                };
                let through = if kind == op::CHAIN {
                    let mut through = stack.pop()?;
                    b.whole(&mut through);
                    b.pending.push(through.slot);
                    Some(through.slot)
                } else {
                    None
                };
                b.settle(&mut inner);
                b.push_phase();
                b.pending.push(inner.slot);
                let dst = b.fresh(1);
                b.zeros.push([PHASE_ZERO, dst, 0, 0]);
                b.code.push([PHASE_CLIMB, dst, inner.slot, (arg & 1) | u32::from(through.is_some()) << 1 | through.unwrap_or(0) << 8]);
                stack.push(Value { pushed: Some(b.phase), cand, ..b.value(dst) });
            }
            op::PARENT => {
                let mut inner = stack.pop()?;
                b.whole(&mut inner);
                let dst = b.take();
                b.code.push([PHASE_DOWN, dst, inner.slot, 0]);
                b.pending.push(inner.slot);
                stack.push(b.value(dst));
            }
            op::REACH_DESCENDANT => {
                let mut inner = stack.pop()?;
                b.settle(&mut inner);
                let pair = (b.fresh(1), b.fresh(1));
                let out = b.descend(inner.slot, pair);
                b.pending.push(inner.slot);
                stack.push(b.value(out));
            }
            op::REACH_CHAIN => {
                // As the CPU's `Reader` reads it: the union over levels of desc(F_k) \ desc(X_k), F_0 the members (`from`),
                // X_k the blockers (in neither `through` nor `from`) within desc(F_k), F_k+1 the members within
                // desc(X_k).
                let mut through = stack.pop()?;
                let mut from = stack.pop()?;
                b.settle(&mut through);
                b.settle(&mut from);
                let (blockers, out, mut level) = (b.take(), b.take(), b.take());
                b.code.push([PHASE_BLOCKERS, blockers, through.slot, from.slot]);
                b.code.push([PHASE_ZERO, out, 0, 0]);
                b.code.push([PHASE_COPY, level, from.slot, 0]);
                b.pending.push(through.slot);
                // Two pairs for the scans' ends, in turn: a cover reads one while the next span writes the other.
                let pairs = [(b.fresh(1), b.fresh(1)), (b.fresh(1), b.fresh(1))];
                for step in 0..2 * levels {
                    let (keep, cut) = if step % 2 == 0 { (blockers, false) } else { (from.slot, true) };
                    let reached = b.descend(level, pairs[step % 2]);
                    b.code.push([if cut { PHASE_ANDNOT } else { PHASE_OR }, out, reached, 0]);
                    b.code.push([PHASE_AND, reached, keep, 0]);
                    b.pending.push(level);
                    level = reached;
                }
                b.code.push([PHASE_CHECK, 0, level, 0]);
                b.pending.extend([level, blockers, from.slot]);
                stack.push(b.value(out));
            }
            _ => return None,
        }
    }
    let [mut result] = stack[..] else { return None };
    b.settle(&mut result);
    let mut code = b.zeros;
    code.extend(b.code);
    Some(PhaseProgram { code, phases: b.phase + 1, result: result.slot, slots: b.slots })
}
/// `label_phase`'s scratch: the formulas of a dispatch each have their own slots, so a level's formulas take as many
/// dispatches as this holds (and a leaf group's rows, `Runner::formulas`).
const FORMULA_SCRATCH_BYTES: usize = 32 << 20;
/// `label_leaves`' threads a group.
const LEAVES_WIDTH: u64 = 128;
/// The fused group kernel in `KERNELS` (`Runner::group`), for 8, 32, 64, 128 and 256 frames a walk (`GROUP_FRAMES`),
/// and its tiers' names, which their dispatches and the profile keep.
const GROUP_KERNEL: usize = 4;
const GROUP_NAMES: [&str; 5] = ["label_group_8", "label_group_32", "label_group_64", "label_group_128", "label_group_256"];
/// The frames of each fused group kernel (`GROUP_KERNELS`). A walk takes the fewest that hold its
/// static depth, at most the loop tier's 64, which a loop walk (no static depth) takes, as
/// `label_micro` does. A run whose walk outgrows them runs again deep (`Runner::deep`), where the
/// cap is 256: past every static depth seen (159 on the S7 slice), so only a loop walk can still
/// outgrow it, and only that runs on the interpreter.
const GROUP_FRAMES: [usize; 5] = [8, 32, 64, 128, 256];
/// A group dispatch's threads per file.
const GROUP_WIDTH: u64 = 64;

/// A walk's stack tier (`label_micro_8`, `label_micro_32`, `label_micro`): the fewest frames its
/// static depth (`Micro::depth`) fits, 128 where it loops. A walk with a closure (`compile_micro::CLOSURE`) is the
/// loop tier's too: its length follows its start's subtree as a loop's does. In the short tier a SIMD group walks its
/// lanes' roots in rounds as long as their longest walk, and the closures' walks of hundreds of steps among walks of a
/// dozen took `label_micro_8` from 91–101 ms to 281–305 at s1000 (work queue Q56).
fn tier(walk: &Micro) -> usize {
    if walk.closes() {
        return 2;
    }
    let depth = walk.depth();
    // Every tier's stacks are in device memory (`Runner::stacks`): thread memory is the driver's
    // scratch, sized for every thread in flight at the kernel's whole stack and kept by the process
    // for its life (O39: 8 root frames, ~100 MB). The shallow walks (97% of S7's starts) take a
    // wide pool of 8-frame stacks; the rest the loop pool's. (`label_micro_32` stays for a tier
    // between them.)
    match depth {
        Some(depth) if depth <= 8 => 0,
        _ => 2,
    }
}

/// The micro walks' kernel in `KERNELS`, every tier's, and each tier's name, which its dispatches and the profile keep.
const MICRO_KERNEL: usize = 3;
const TIER_NAMES: [&str; 3] = ["label_micro_8", "label_micro_32", "label_micro"];

/// The loop tier's threads (`MICRO_POOL`), each striding over the items with a 64-frame stack
/// of 17 words a frame (`MicroFrame`) in `Runner::stacks`. The pool's threads are the kernel's
/// occupancy: at 1024 threads of 128 frames the loop tier's 3% of S7's starts took ~280 ms of
/// its ~350 ms on the GPU, 2048 of 64 ~150 ms. Timed per kernel (`RERAIL_GPU_KERNEL_TIMES`),
/// 1.73 M loop items took 193 ms at 2048 threads, 121 ms at 4096 and 85 ms at 8192, which put
/// the footprint over budget (511–515 MB). A run whose walk outgrows 64 frames runs again with
/// the same stacks as 2048 of 128 (`label_micro_deep`, `MICRO_DEEP_POOL`).
const MICRO_POOL: usize = 4096;
/// A SIMD group's lanes on every Apple GPU: the kernels that spread a group's roots over its lanes are written for it
/// (`simd_roots`), and every pool and stack count here is a multiple of it.
const SIMD_WIDTH: usize = 32;
const _: () = assert!(MICRO_POOL % SIMD_WIDTH == 0 && MICRO_DEEP_POOL % SIMD_WIDTH == 0);
/// The lanes a micro walk's item (a group of 64 nodes) takes in each tier (`Shape::lanes`): each lane finds the roots
/// among its share of the group's nodes, and the SIMD group walks all its lanes' roots. The short walks' tiers
/// take a SIMD group an item: a thread an item gave their dispatches a 32nd of the threads (at s1000, `label_micro_8`
/// 179 ms against 94; work queue Q62). The loop tier's pool is 4,096 threads whatever its items, and its groups hold
/// few roots (about 6 at s1000): 1, 4 and 32 lanes an item measured the same there (at load 70–95), so it keeps a
/// thread an item.
const TIER_LANES: [usize; 3] = [32, 32, 1];
const _: () = assert!(TIER_LANES[0] <= SIMD_WIDTH && TIER_LANES[1] <= SIMD_WIDTH && TIER_LANES[2] <= SIMD_WIDTH);
const MICRO_STACK_WORDS: usize = 64 * 17;
/// The pooled stacks' words (`Runner::stacks`), shared by every tier and the root walks: a tier
/// of `frames` frames of `words` words each runs this many lanes (32,768 of 8 micro frames).
const STACK_WORDS: usize = MICRO_POOL * MICRO_STACK_WORDS;
fn stack_lanes(frames: usize, words: usize) -> usize {
    STACK_WORDS / (frames * words)
}
/// Frames a micro tier's walks hold (`TIER_KERNELS`).
const TIER_FRAMES: [usize; 3] = [8, 32, 64];
const MICRO_DEEP_POOL: usize = 2048;
/// The deep pool's frames (`label_micro_deep`).
const MICRO_DEEP_FRAMES: usize = 128;
/// The launches an evaluation's encoding stays within (`metal_launch_budget`, list step 13): a
/// recursive group's rounds are encoded from what the plan's fixed launches leave, and a plan
/// that cannot fit (its fixed launches and two rounds a group, or a rerun) is refused before it
/// encodes, as `BackendLaunchLimit`.
const LAUNCHES: usize = super::super::metal_backend::launch::KERNEL_LAUNCH_LIMIT;
/// At most this many rounds of a recursive group are encoded in one go.
const MOST_ROUNDS: usize = 64;
/// Bytes of per-level uploads one command buffer carries before the host waits for it.
const SEGMENT_BYTES: usize = 64 << 20;

/// Compiles the wave's pipelines (the set steps, the interpreter, the sweep and the label kernels) as
/// a job on the pool, once a process: each is compiled from source, 0.6 s together, and a pass that
/// needs one before it is ready waits for the rest of its compile (they are cached behind locks).
pub(in crate::code_rule_dsl) fn warm() {
    // Once a machine (decision 0442): its pipelines are the machine's.
    #[derive(Default)]
    struct Warmed(std::sync::OnceLock<()>);
    crate::code_rule_dsl::logic::profile::machine::<Warmed>().0.get_or_init(|| {
        // The job compiles on this thread's machine: a pool thread holds no run.
        let shared = crate::code_rule_dsl::logic::profile::shared_here();
        // A panic leaves the kernels to be compiled where they are needed (a job's panic would end
        // the process).
        rayon::spawn(move || {
            // Held before the job: `as_job` reads the job walls, the machine's.
            let _in_run = crate::code_rule_dsl::logic::profile::within(shared);
            rerail_accel::exec::as_job("the label kernels' warm-up", || {
                let _ = std::panic::catch_unwind(|| {
                    // The set program's kernels first: it runs first in a wave (its candidates). The shared
                    // device is not opened here: on S7 a wave waiting on this job's open waited longer (to
                    // 0.8 s under load) than opening it itself (50-80 ms).
                    let _ = super::set_steps::kernel();
                    let _ = compile_resident_interpreter_cached();
                    // The run's format, as its tapes are placed (a 16z tape as 16): its batches' kernels.
                    let _ = pipelines(match crate::code_rule_dsl::tape::formats::Format::of_run() {
                        crate::code_rule_dsl::tape::formats::Format::F15 => 15,
                        _ => 16,
                    });
                    let _ = compile_sweep_cached();
                });
            })
        });
    });
}

/// The label kernels, compiled once per tape format (until the residency's drop).
#[derive(Default)]
struct LabelPipelines(Mutex<HashMap<u32, Vec<LabelKernel>>>);
/// The clauses' pattern kernels, by source and tape format (until the residency's drop).
#[derive(Default)]
struct PatternPipelines(Mutex<HashMap<(String, u32), ComputePipelineState>>);

/// A pipeline of a source over [`tape_view`] for a batch of tapes in `format` (0: mixed).
fn pipeline_of(source: &str, name: &str, format: u32) -> Result<ComputePipelineState, Error> {
    rerail_metal::runtime::pipeline_named_fixed(source, name, Some(format))
}

/// Releases the compiled label and pattern kernels (the residency's drop).
pub(in crate::code_rule_dsl) fn drop_pipelines() {
    let held_ = crate::code_rule_dsl::logic::profile::machine::<LabelPipelines>();
    if let Ok(mut pipelines) = held_.0.lock() {
        pipelines.clear();
    }
    let held_ = crate::code_rule_dsl::logic::profile::shared::<PatternPipelines>();
    if let Ok(mut pipelines) = held_.0.lock() {
        pipelines.clear();
    }
}

fn pipelines(format: u32) -> Result<Vec<LabelKernel>, Error> {
    let held_ = crate::code_rule_dsl::logic::profile::machine::<LabelPipelines>();
    let mut cached = held_.0.lock().map_err(|_| Error::BackendFailure)?;
    if let Some(pipelines) = cached.get(&format) {
        return Ok(pipelines.clone());
    }
    // The library once (its front end, with the first kernel), then the others side by side: each pipeline is a
    // back-end compile of its own, and Metal's compiler service runs them at once (work queue Q181: format 16's took 67
    // s one after another, 23 s together, on a loaded M3 Max). A kernel that fails to compile panics on its thread,
    // and the panic goes on here (decision 0382). Each compile waits on the device's compiler, so its thread is a
    // device's worker, scoped as the executor's are (`exec::scoped`, decision 0100 §12). A batch of 16z tapes compiles
    // the reads with 16z's bodies (`reads_z`); the others without them. Both sources are fixed, so their kernels are
    // the ones the engine ships (`kernel_fixed`, decision 0395).
    let source = &SOURCES[usize::from(reads_z(format))];
    // From the engine's kernel store, as the device layer's kernels (UKL, decision 0440 step C).
    let kernel = |name: &str| LabelKernel::of(&rerail_metal::lane::kernel_fixed(source, name, Some(format)));
    let first = kernel(KERNELS[0]);
    let rest = rerail_accel::exec::scoped(|scope| {
        let compiles = KERNELS[1..].iter().map(|name| scope.spawn(move || kernel(name))).collect::<Vec<_>>();
        compiles.into_iter().map(|compile| compile.join().unwrap_or_else(|panic| std::panic::resume_unwind(panic))).collect::<Vec<_>>()
    });
    let pipelines = std::iter::once(first).chain(rest).collect::<Vec<_>>();
    cached.insert(format, pipelines.clone());
    Ok(pipelines)
}

/// The label kernels' source and names (`SOURCES`' without 16z's reads, `KERNELS`), for the compile probe (work queue Q181).
#[cfg(any(test, feature = "test-hooks"))]
pub fn label_source() -> (&'static str, &'static [&'static str]) {
    (&SOURCES[0], &KERNELS)
}

/// Compiles every label kernel and the fact-only pattern kernel for a batch of `format` (0: mixed): how many of each.
#[cfg(any(test, feature = "test-hooks"))]
pub fn compile_label_kernels(format: u32) -> Result<(usize, usize), Error> {
    let kernels = pipelines(format)?.len();
    let patterns = pattern_pipelines(&[], true, format)?.map_or(0, |(pipelines, _)| pipelines.len());
    Ok((kernels, patterns))
}

/// The plan's pattern kernels, one per `patterns::GROUP` of its patterns (a fact-only plan's
/// one), with every pattern's metavariable names in the plan's numbering.
fn pattern_pipelines(patterns: &[&PackedPattern], facts: bool, format: u32) -> Result<Option<(Vec<ComputePipelineState>, Vec<Vec<String>>)>, Error> {
    if patterns.is_empty() && !facts {
        return Ok(None);
    }
    let (mut pipelines, mut names) = (Vec::new(), Vec::new());
    let groups = patterns.chunks(patterns::GROUP).collect::<Vec<_>>();
    for (group, members) in if groups.is_empty() { vec![&[][..]] } else { groups }.into_iter().enumerate() {
        let (pipeline, group_names) = pattern_pipeline(members, group * patterns::GROUP, format)?;
        pipelines.push(pipeline);
        names.extend(group_names);
    }
    Ok(Some((pipelines, names)))
}

/// The sources of the pattern kernels a plan of `patterns` compiles for a batch of `format` (`pattern_pipelines`),
/// in its order: each reads the format's tapes (`tape_view`).
pub(in crate::code_rule_dsl) fn pattern_sources(patterns: &[&PackedPattern], facts: bool, format: u32) -> Vec<String> {
    if patterns.is_empty() && !facts {
        return Vec::new();
    }
    let groups = patterns.chunks(patterns::GROUP).collect::<Vec<_>>();
    let groups = if groups.is_empty() { vec![&[][..]] } else { groups };
    groups.into_iter().enumerate().map(|(group, members)| patterns::source(members, group * patterns::GROUP, format).0).collect()
}

/// Whether the pattern kernels of `sources` (`pattern_sources`) are at hand without a compile (decision 0395):
/// otherwise the wave runs on the CPU, and the kernels are compiled as the mode says
/// (`rerail_accel::gpu::kernels::Mode`). The label kernels are the interpreter's, fixed (shipped): never pending.
/// `format` is the batch's tape format, the kernels' function constant (`pattern_pipeline`).
pub(in crate::code_rule_dsl) fn patterns_ready(sources: &[String], format: u32) -> bool {
    // Each asked, so every one missing is recorded, not only the first.
    sources.iter().map(|source| rerail_accel::gpu::kernels::ready(source, patterns::KERNEL, Some(format)).is_some()).fold(true, |all, ready| all && ready)
}

/// One group's pattern kernel, compiled once per distinct source.
fn pattern_pipeline(patterns: &[&PackedPattern], first: usize, format: u32) -> Result<(ComputePipelineState, Vec<Vec<String>>), Error> {
    let (source, names) = patterns::source(patterns, first, format);
    // `RERAIL_DIAG_PATTERNS=<dir>`: each pattern kernel's source, for measuring it alone.
    if let Some(dir) = std::env::var_os("RERAIL_DIAG_PATTERNS") {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        source.hash(&mut hasher);
        let _ = std::fs::write(std::path::Path::new(&dir).join(format!("{:016x}.metal", hasher.finish())), &source);
    }
    let held_ = crate::code_rule_dsl::logic::profile::shared::<PatternPipelines>();
    let mut cached = held_.0.lock().map_err(|_| Error::BackendFailure)?;
    let key = (source, format);
    if let Some(pipeline) = cached.get(&key) {
        return Ok((pipeline.clone(), names));
    }
    crate::code_rule_dsl::prof::count("gpu.pattern_pipelines", 1);
    let pipeline = pipeline_of(&key.0, patterns::KERNEL, format)?;
    cached.insert(key, pipeline.clone());
    Ok((pipeline, names))
}

fn bytes_of<T: Copy>(values: &[T]) -> &[u8] {
    unsafe { std::slice::from_raw_parts(values.as_ptr().cast::<u8>(), std::mem::size_of_val(values)) }
}

/// The batch's tape buffers, shared by every level.
struct Tapes {
    /// The batch's documents, resident (decision 0032): the label kernels read their tapes in
    /// place, `docs` holding each one's `DocView`.
    table: crate::code_rule_dsl::residency::slabs::DocTable,
    docs: MappedBuffer,
    /// The label state: the prefix (head, labels, the sweeps' words), the scratch rows and the
    /// tallies (at `scratch_at`, `counts_at`, bytes), then the grammar's (node, class) pairs. One
    /// buffer, so a marking program (`packed_batch::MARKS`) reaches its rows from `run_batch`'s
    /// `supertypes`.
    supertypes: MappedBuffer,
    scratch_at: u64,
    counts_at: u64,
    /// Elements in `supertypes`: a label batch's header says so, which `run_batch` reads as
    /// "the prefix head is there".
    supertype_count: u32,
}

/// A level's programs as an interpreter batch, built once: every round encodes the same tasks
/// with a fresh scheduler image.
struct Prepared {
    tasks: MappedBuffer,
    header: MappedBuffer,
    routes: MappedBuffer,
    program: Arc<ProgramBuffers>,
    arenas: MetalGlobalArenas,
    evaluations: Vec<MetalEvaluation>,
    lanes: Vec<MetalEvaluation>,
    task_count: usize,
    result_capacity: usize,
    /// Per program of the batch: its pass's label, its predicted steps per node (`evaluation_costs::per_node`) and its
    /// digest, to name and price an unfinished evaluation (`Runner::unfinished`).
    passes: Vec<(u32, u64, String)>,
}

/// A level's micro walks (`label_micro`): per pass {word start, word count, scratch label or
/// count slot, mode (as `label_sink`'s routes), item start, item count, first text slot, first
/// thread}, the passes' words, and their starts
/// (global nodes).
struct Walks {
    /// Per stack tier (`tier`) with passes: the tier, its passes (threads from 0), their count
    /// and their threads.
    tiers: Vec<(usize, MappedBuffer, usize, usize)>,
    /// Each text slot's bits (`Micro::texts`), one label's words a slot.
    texts: MappedBuffer,
    words: MappedBuffer,
    steps: u32,
    /// Every walk's record and static depth, for a recursive group fused into one dispatch
    /// (`Runner::group`).
    fused: Vec<([u32; 8], Option<usize>)>,
    /// Each text slot's row in `texts`, by its key (`Micro::slot_key`): an anchor's text tests read them (decision 0365).
    slots: HashMap<String, u32>,
    /// How many of the level's passes these walks run (a pack runs several).
    walked: usize,
}

/// A workspace lease with what it was sized for.
struct Workspace {
    lease: MetalWorkspaceLease,
    totals: WorkspaceTotals,
    scheduler_bytes: u32,
    output_bytes: u32,
    tasks: usize,
}

struct Runner<'a> {
    device: &'a Device,
    queue: metal::CommandQueue,
    engine: Engine,
    kernels: Vec<LabelKernel>,
    /// `label_sweep`, where a group runs as sweeps.
    sweep: Option<ComputePipelineState>,
    /// By a sweeping group's address: per file the start of its tasks, then their indices by
    /// (root, program). Its task buffer carries them after the tasks.
    orders: HashMap<usize, Vec<u32>>,
    /// The clause's pattern kernel and each pattern's metavariables in index order.
    patterns: Option<(Vec<ComputePipelineState>, Vec<Vec<String>>)>,
    files: &'a [TapeDoc],
    tapes: Tapes,
    /// Labels in the prefix.
    plan_labels: usize,
    /// Each label's prefix row (`label_rows`), where the prefix holds some of them.
    label_rows: Option<Vec<u32>>,
    words: usize,
    nodes: usize,
    flags: MappedBuffer,
    workspace: Option<Workspace>,
    command: CommandBuffer,
    retained: Vec<MappedBuffer>,
    programs: Vec<Arc<ProgramBuffers>>,
    /// By the level's address in the plan; `None` for a level with no tasks.
    prepared: HashMap<usize, Option<Prepared>>,
    segment_bytes: usize,
    /// A level's lanes and each task's states; a sweeping group's are always `WIDE`.
    lanes: (usize, Option<u32>),
    /// The run's step budget (decision 0334), doubled for each rerun after an overrun.
    budget: super::WorkBudget,
    /// Whether a level's micro walks run as such, and each level's, by its address.
    micro: bool,
    /// Whether the loop tier runs with 128-frame stacks (`label_micro_deep`).
    deep: bool,
    /// The levels of a chain's reach the formulas' phases hold (`PHASE_CHAIN_LEVELS`, doubled when a run's chain
    /// outgrows them).
    chain_levels: usize,
    walks: HashMap<usize, Walks>,
    /// Every pass's roots as rules (`RootTable`), one buffer for the run (`root_words`).
    root_table: RootTable,
    root_words: Option<MappedBuffer>,
    /// The batch's fact block (`Pass::Fact`, Q317), bound to the pattern kernels where the facts are read: its
    /// address, and its device buffer.
    facts: Option<(usize, Buffer)>,
    /// `RERAIL_GPU_ENCODER_TIMES`: each encoder's own GPU time, from timestamps inside its command buffer.
    encoder_times: Option<EncoderTimes>,
    /// The rules whose anchors were evaluated this run (`anchors`), by address.
    anchored: std::collections::HashSet<usize>,
    /// The anchors' leaf tests for the whole run (`size`), and their rows once read (`formulas`).
    run_leaves: Vec<u32>,
    run_leaf_rows: Option<MappedBuffer>,
    /// `label_phase`'s slots (0461).
    phase_scratch: Option<MappedBuffer>,
    /// The batch's parents (`label_parents`, 0463), made at its first phases.
    phase_parents: Option<MappedBuffer>,
    /// The loop tier's stacks (`MICRO_POOL` of them), committed only as walks use them.
    stacks: Option<MappedBuffer>,
    /// The deep fused groups' stacks, one buffer for all of them: the groups run one after
    /// another in the command buffer (five of 33 MB each on S7 before, list step 13).
    group_stacks: [Option<MappedBuffer>; 5],
    encoded: bool,
    outcome: Outcome,
    /// The plan's root walks, encoded after its passes (`records`).
    roots: Option<RootsEncoded>,
    /// Profiling (`RERAIL_GPU_WALK_STEPS`): the stats buffer of each micro dispatch encoded since
    /// the last flush, with its passes' names; the next dispatch's binding.
    walk_stats: Vec<(MappedBuffer, Vec<String>)>,
    walk_bind: Option<Buffer>,
    /// The micro walks' spill queue (`MicroQ`): slots, control, items; made on first use.
    spill: Option<[MappedBuffer; 3]>,
    /// Each program dispatch's evaluations as they ended (copied out before the next level's image replaces them),
    /// with their count and the batch's key: which (pass, file) a task left unfinished (decision 0334).
    reports: Vec<(MappedBuffer, usize, usize)>,
}

#[cfg(any(test, feature = "test-hooks"))]
thread_local! {
    /// Tests: the passes run on the interpreter, as a run whose walks outgrew their frames does.
    pub static INTERPRET_ONLY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn interpret_only() -> bool {
    #[cfg(any(test, feature = "test-hooks"))]
    return INTERPRET_ONLY.get();
    #[cfg(not(any(test, feature = "test-hooks")))]
    false
}

/// `Error::BackendCapacity`, counted by where it was raised (`capacity.<line>` in the profile's stages): which bound a
/// run that fell back to the CPU met.
#[track_caller]
fn capacity() -> Error {
    if crate::code_rule_dsl::prof::on() {
        crate::code_rule_dsl::prof::count_named(format!("capacity.label_passes:{}", std::panic::Location::caller().line()), 1);
    }
    Error::BackendCapacity
}

pub(in crate::code_rule_dsl) fn run(plan: &Plan<'_>, files: &[TapeDoc]) -> Result<Outcome, Error> {
    // The run's command buffers and encoders are autoreleased, and they retain its buffers: drained
    // here, its buffers go with it rather than living until the thread's pool drains (06 S7: the
    // 9 runs of a wave held ~1 GB at once).
    metal::objc::rc::autoreleasepool(|| run_in_pool(plan, files))
}

fn run_in_pool(plan: &Plan<'_>, files: &[TapeDoc]) -> Result<Outcome, Error> {
    let _evaluation = super::super::metal_backend::evaluation_scope();
    let device = METAL_DEVICE.as_ref().ok_or(Error::UnsupportedBackend)?;
    // A level's launches: the programs' dispatch and sink, the derived labels, the settle.
    let launches = |level: &Vec<Pass<'_>>| {
        let programs = level.iter().any(|pass| matches!(pass, Pass::Program { .. } | Pass::Count { .. }));
        let patterns = level.iter().any(kernel_pass);
        // Decision 0365: a level's formulas (its anchors, its formula passes): their leaves, and a dispatch a phase of
        // the one with the most (0463).
        let phases = level
            .iter()
            .filter_map(|pass| match pass {
                Pass::Program { roots, .. } | Pass::Count { roots, .. } => roots.anchor().map(|anchor| &**anchor),
                Pass::Formula { formula, .. } => Some(&**formula),
                _ => None,
            })
            .map(formula_phase_count)
            .max();
        2 * usize::from(programs) + usize::from(patterns) + 1 + phases.map_or(0, |phases| 1 + phases)
    };
    // The batch's parents, once where a level has formulas (`label_parents`, 0463).
    let parents = plan.steps.iter().flat_map(|(_, levels)| levels.iter().flatten()).any(|pass| match pass {
        Pass::Program { roots, .. } | Pass::Count { roots, .. } => roots.anchor().is_some(),
        Pass::Formula { .. } => true,
        _ => false,
    });
    let fixed = plan
        .steps
        .iter()
        .filter(|(recursive, _)| !recursive)
        .flat_map(|(_, levels)| levels)
        .map(launches)
        .sum::<usize>()
        + usize::from(parents);
    // A group of micro walks runs to its fixpoint in one dispatch (`Runner::group`), which needs
    // no pattern pass; whether its walks fuse is known only once sized, so a plan counts it as
    // fused, and the evaluation's ledger refuses the submission of one that did not.
    let fused = |levels: &Vec<Vec<Pass<'_>>>| !levels.iter().flatten().any(kernel_pass);
    // A group of rounds ends with `label_round`; a group of sweeps is checked by the host; a fused
    // group is its dispatch.
    let groups = plan.steps.iter().filter(|(recursive, levels)| *recursive && !sweeps(levels)).count();
    // A recursive group's label grows (or settles) one node of a chain per round: a node joins
    // in the round after the node its walk reaches. Its nodes have the label's kinds, so the
    // longest chain of such nodes, nested along a path or (where the group steps to siblings)
    // side by side, plus a round to see nothing change, is the expected count. It is not a
    // proof: a group still changing in its last round reruns the clause with more.
    let expected = plan
        .chains
        .iter()
        .zip(&plan.steps)
        .map(|((kinds, siblings), (_, levels))| {
            if sweeps(levels) {
                return SWEEPS;
            }
            let has = |tape: &TapeDoc, node: u32| {
                kinds.as_ref().is_none_or(|kinds| kinds.binary_search(&tape.transducer_kind(node)).is_ok())
            };
            files
                .iter()
                .map(|tape| {
                    let mut nested = vec![0_usize; tape.transducer_len()];
                    let mut longest = 0;
                    for (node, parent) in (0..tape.transducer_len() as u32).zip(tape.transducer_parents()) {
                        let above = parent.map_or(0, |parent| nested[parent as usize]);
                        nested[node as usize] = above + usize::from(has(tape, node));
                        longest = longest.max(nested[node as usize]);
                        if *siblings {
                            let mut run = 0;
                            for child in tape.transducer_children(node) {
                                run = if has(tape, child) { run + 1 } else { 0 };
                                longest = longest.max(run);
                            }
                        }
                    }
                    longest
                })
                .max()
                .unwrap_or(0)
                + 2
        })
        .collect::<Vec<_>>();
    // Each group's rounds: as expected, all capped alike where the budget cannot afford that.
    let per_group = plan
        .steps
        .iter()
        .map(|(recursive, levels)| match (*recursive, sweeps(levels)) {
            (false, _) => 0,
            (true, true) => 1,
            (true, false) if fused(levels) => 0,
            (true, false) => levels.iter().map(launches).sum::<usize>(),
        })
        .collect::<Vec<_>>();
    let plan_rounds = |used: usize, wanted: &[usize]| {
        let budget = LAUNCHES.saturating_sub(used + fixed + groups);
        let rounds = |cap: usize| {
            plan.steps
                .iter()
                .zip(wanted)
                .map(|((recursive, _), wanted)| if *recursive { (*wanted).clamp(2, cap.max(2)) } else { 0 })
                .collect::<Vec<_>>()
        };
        let cost = |rounds: &[usize]| rounds.iter().zip(&per_group).map(|(rounds, launches)| rounds * launches).sum::<usize>();
        let mut cap = MOST_ROUNDS;
        while cap > 2 && cost(&rounds(cap)) > budget {
            cap -= 1;
        }
        let rounds = rounds(cap);
        // The plan-time bound: what this encoding launches, with the runs before it.
        if used + fixed + groups + cost(&rounds) > LAUNCHES {
            crate::code_rule_dsl::prof::count("run.launch_limit", 1);
            if std::env::var_os("RERAIL_LAUNCHES").is_some() {
                eprintln!("launch limit: used {used} fixed {fixed} groups {groups} rounds {rounds:?} per group {per_group:?} files {}", files.len());
            }
            return Err(Error::BackendLaunchLimit);
        }
        Ok(rounds)
    };
    let mut wanted = expected.clone();
    if std::env::var_os("RERAIL_LAUNCHES").is_some() {
        let levels = plan.steps.iter().map(|(_, levels)| levels.len()).sum::<usize>();
        eprintln!("PLAN steps {} levels {levels} fixed {fixed} groups {groups} per group {per_group:?} expected {expected:?} files {}", plan.steps.len(), files.len());
    }
    let mut rounds = plan_rounds(0, &wanted)?;
    let nodes = files.iter().map(TapeDoc::transducer_len).sum::<usize>();
    let (mut retries, mut used, mut buffers) = (0, 0, 0);
    let (mut lanes, mut micro, mut deep, mut released) = (NARROW, !interpret_only(), false, false);
    let (mut budget, mut replanned) = (super::WorkBudget::current(), 0);
    let mut chain_levels = PHASE_CHAIN_LEVELS;
    loop {
        let mut outcome = match encode(device, plan, files, &rounds, lanes, micro, deep, chain_levels, budget) {
            // A chain's reach outgrew the levels its phases hold (0463): the run again with twice them, past the most
            // the interpreter's. The interpreter's passes run the same phases, so its runs double them too: refused
            // at the first level, a chain one level deep failed the run.
            Err(Error::ChainLevels) if chain_levels < PHASE_CHAIN_MOST => {
                crate::code_rule_dsl::prof::count("run.chain_levels", 1);
                chain_levels *= 2;
                continue;
            }
            Err(Error::ChainLevels) if micro => {
                crate::code_rule_dsl::prof::count("run.interpreted", 1);
                micro = false;
                continue;
            }
            Err(Error::ChainLevels) => return Err(Error::BackendFailure),
            // A task over its step budget, below the ceiling (decision 0334): the run again with twice the
            // budget, until its tasks' budgets are the ceiling (then a stop is `WorkLimit`'s failure).
            Err(Error::OverBudget) => {
                crate::code_rule_dsl::prof::count("run.replanned", 1);
                crate::code_rule_dsl::logic::profile::decision("label sweep over its budget, run again with twice the budget (0334)", || format!("{budget:?}"));
                budget = budget.doubled();
                replanned += 1;
                continue;
            }
            // Workspaces kept from earlier runs hold the budget: freed once, the same run again.
            Err(Error::BackendCapacity) if !released && super::release_workspaces() => {
                released = true;
                crate::code_rule_dsl::prof::count("run.released", 1);
                continue;
            }
            // A task's states outgrew the narrow lanes' (or another task failed, which fails again).
            Err(Error::BackendFailure) if lanes != WIDE => {
                crate::code_rule_dsl::prof::count("run.widened", 1);
                lanes = WIDE;
                continue;
            }
            // The workspace is over the buffer budget: each lane holds half the states (a task that
            // needs more fails its batch, which then runs `WIDE`).
            Err(Error::BackendBudget) if lanes.1.is_some_and(|states| states > MIN_STATES) => {
                crate::code_rule_dsl::prof::count("run.fewer_states", 1);
                lanes.1 = lanes.1.map(|states| states / 2);
                continue;
            }
            Err(Error::BackendBudget) => return Err(capacity()),
            // A loop walk outgrew the pool's 64 frames (or a walk its captures or steps): the
            // pool's 128-frame stacks first. Only a walk's overflow reaches here; a batch over a
            // budget (`BackendCapacity`) goes back to the wave to be split, before anything of a
            // bigger route is built for it.
            Err(Error::WalkFrames) if micro && !deep => {
                crate::code_rule_dsl::prof::count("run.deep", 1);
                deep = true;
                continue;
            }
            // A micro walk outgrew its frames, captures or steps.
            Err(Error::WalkFrames) if micro => {
                crate::code_rule_dsl::prof::count("run.interpreted", 1);
                micro = false;
                continue;
            }
            Err(Error::WalkFrames) => return Err(Error::BackendFailure),
            outcome => outcome?,
        };
        outcome.interpreted = !micro;
        outcome.widened = lanes == WIDE;
        crate::code_rule_dsl::prof::count("run.launches", outcome.dispatches);
        if std::env::var_os("RERAIL_LAUNCHES").is_some() {
            eprintln!("launches: {} (planned {}) files {}", outcome.dispatches, fixed + groups + rounds.iter().zip(&per_group).map(|(r, l)| r * l).sum::<usize>(), files.len());
        }
        used += outcome.dispatches;
        buffers += outcome.command_buffers;
        outcome.replanned = replanned;
        if outcome.waits[1] == 0 {
            outcome.waits[1] = retries;
            outcome.dispatches = used;
            outcome.command_buffers = buffers;
            return Ok(outcome);
        }
        if rounds.iter().all(|rounds| *rounds > nodes + 1 || *rounds == 0) {
            return Err(Error::WorkLimit);
        }
        retries += 1;
        wanted = rounds.iter().map(|rounds| rounds * 2).collect();
        rounds = plan_rounds(used, &wanted)?;
    }
}

/// The clause's passes in one command buffer, each recursive group as `rounds` rounds.
/// `lanes`: a level's lanes and each task's states (`NARROW`, `WIDE`); `micro`: whether a pass
/// whose program is a micro walk runs as one (`label_micro`).
/// `RERAIL_FOOTPRINT`: the footprint now and its high-water mark at a point in a run.
fn mark(at: &str) {
    if std::env::var_os("RERAIL_FOOTPRINT").is_some() {
        let _ = tikv_jemalloc_ctl::epoch::advance();
        eprintln!(
            "  FOOTPRINT {at}: {} MB (high-water {} MB); Metal buffers {} MB; heap live {} MB",
            crate::code_rule_dsl::prof::footprint_mb(),
            crate::code_rule_dsl::prof::max_footprint_mb(),
            super::super::metal_backend::reserved_mb(),
            tikv_jemalloc_ctl::stats::allocated::read().unwrap_or(0) >> 20
        );
        #[derive(Default)]
        struct HoldSeen(std::sync::atomic::AtomicUsize);
        let wanted = std::env::var("RERAIL_HOLD_AT").ok();
        let nth = wanted.as_deref().and_then(|wanted| wanted.split_once('#')).map_or(Some(1), |(_, n)| n.parse().ok());
        let here = wanted.as_deref().is_some_and(|wanted| wanted.split('#').next() == Some(at))
            && Some(rerail_prof::code_rule_dsl::logic::profile::local::<HoldSeen>().0.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1) == nth;
        if let Some(path) = std::env::var_os("RERAIL_HOLD").filter(|_| here) {
            eprintln!("FLUSH holding, pid {}", std::process::id());
            while !std::path::Path::new(&path).exists() {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
    }
}

/// The unfinished (label, file) a run gives to W's matcher (decision 0334), or the error that runs it again
/// (`OverBudget`: twice the budget) or fails it (`BackendFailure`). An overrun goes to W where W is predicted cheaper than
/// the evaluation again with twice the budget (its steps per node times the file's nodes, doubled); a stop at the
/// ceiling goes to W. Without a W (`Plan::w_cost`), or an unfinished task no evaluation names, as before.
fn replanned(
    plan: &Plan<'_>,
    files: &[TapeDoc],
    budget: super::WorkBudget,
    unfinished: &BTreeMap<(u32, usize), (bool, bool, u64, String)>,
    flags: u32,
) -> Result<Vec<(u32, usize)>, Error> {
    let (over, ceiling) = (unfinished.values().any(|u| u.0), unfinished.values().any(|u| u.1));
    if flags & 4 != 0 && !over {
        return Err(Error::OverBudget);
    }
    if flags & 8 != 0 && !ceiling {
        return Err(Error::BackendFailure);
    }
    let Some(w_cost) = plan.w_cost else {
        return Err(if over { Error::OverBudget } else { Error::BackendFailure });
    };
    let mut given = Vec::new();
    let mut observations = Vec::new();
    for (&(label, file), (overran, stopped, per_node, digest)) in unfinished {
        let nodes = files.get(file).map_or(0, |tape| tape.transducer_len() as u64);
        if *overran && !stopped {
            let spent = u64::from(budget.task_limit(*per_node)).saturating_mul(nodes);
            observations.push(super::evaluation_costs::Observation { program: digest.clone(), steps: spent, size: nodes, predicted: per_node.saturating_mul(nodes), over_budget: true });
            let (w, again) = (w_cost(label, file), spent.saturating_mul(2));
            crate::code_rule_dsl::logic::profile::decision("over its budget, replanned by cost (0334)", || {
                format!("ast label pass on Metal: W {w} steps against the run again {again}: {}", if w < again { "W" } else { "again" })
            });
            if w >= again {
                super::evaluation_costs::observe(observations);
                return Err(Error::OverBudget);
            }
        }
        given.push((label, file));
    }
    super::evaluation_costs::observe(observations);
    Ok(given)
}

fn encode(
    device: &Device,
    plan: &Plan<'_>,
    files: &[TapeDoc],
    rounds: &[usize],
    lanes: (usize, Option<u32>),
    micro: bool,
    deep: bool,
    chain_levels: usize,
    budget: super::WorkBudget,
) -> Result<Outcome, Error> {
    let nodes = files.iter().map(TapeDoc::transducer_len).sum::<usize>();
    let words = nodes.div_ceil(32).max(1);
    let rows = label_rows(plan, micro);
    let prefix_labels = rows.as_ref().map_or(plan.labels, |(present, _)| present.len());
    crate::code_rule_dsl::prof::count("gpu.prefix_labels", prefix_labels);
    // An earlier run's buffers, where sized alike; scratch and counts zeroed again (a run that
    // fails may leave them dirty, and a kept set is taken out while a run holds it).
    let key = (nodes, plan.labels, plan.counts);
    let mut kept = plan
        .buffers
        .and_then(|buffers| buffers.0.lock().ok()?.take())
        .filter(|kept| kept.key == key);
    if kept.is_some() {
        crate::code_rule_dsl::prof::count("gpu.kept", 1);
    }
    mark("before pipelines");
    let queue = rerail_metal::queue()?;
    let command = queue.new_command_buffer().to_owned();
    command.set_label("Rerail label passes");
    let tapes = crate::code_rule_dsl::prof::time("gpu.tapes", || tapes(device, files, plan, words, rows.as_ref(), [scratch_rows(plan) * words, tally_rows(plan) * nodes]))?;
    // The batch's one format, or 0 for a mixed batch: the kernels are made for it (`tape_view`).
    let format = tapes.table.format().unwrap_or(0);
    if crate::code_rule_dsl::prof::on() {
        crate::code_rule_dsl::prof::count_named(format!("gpu.tape_format={format}"), 1);
    }
    let mut runner = Runner {
        device,
        engine: compile_resident_interpreter_cached()?,
        kernels: pipelines(format)?,
        sweep: if plan.steps.iter().any(|(recursive, levels)| *recursive && sweeps(levels)) {
            Some(compile_sweep_cached()?)
        } else {
            None
        },
        orders: HashMap::new(),
        patterns: pattern_pipelines(&plan.patterns, plan.steps.iter().flat_map(|(_, levels)| levels).flatten().any(|pass| matches!(pass, Pass::Fact { .. })), format)?,
        command,
        queue,
        files,
        tapes,
        plan_labels: prefix_labels,
        label_rows: rows.as_ref().map(|(_, row_of)| row_of.clone()),
        words,
        nodes,
        flags: MappedBuffer::zeroed::<u32>(device, 6)?,
        workspace: None,
        retained: Vec::new(),
        programs: Vec::new(),
        prepared: HashMap::new(),
        segment_bytes: 0,
        lanes,
        budget,
        micro,
        deep,
        chain_levels,
        walks: HashMap::new(),
        root_table: RootTable::default(),
        root_words: None,
        facts: None,
        encoder_times: EncoderTimes::new(device),
        anchored: Default::default(),
        run_leaves: Vec::new(),
        run_leaf_rows: None,
        phase_scratch: None,
        phase_parents: None,
        stacks: kept.as_mut().and_then(|kept| kept.stacks.as_ref().map(|stacks| MappedBuffer::wrap(stacks.buffer.clone()))),
        group_stacks: [None, None, None, None, None],
        encoded: false,
        outcome: Outcome {
            words,
            rounds: rounds.iter().copied().max().unwrap_or(0),
            ..Outcome::default()
        },
        roots: None,
        walk_stats: Vec::new(),
        walk_bind: None,
        spill: None,
        reports: Vec::new(),
    };
    mark("buffers");
    if crate::code_rule_dsl::prof::on() {
        use crate::code_rule_dsl::prof::count;
        count("mem.nodes", runner.nodes);
        count("mem.labels", plan.labels);
        count("mem.counts", plan.counts);
        count("mem.scratch_mb", scratch_rows(plan) * words * 4 >> 20);
        count("mem.counts_mb", tally_rows(plan) * runner.nodes * 4 >> 20);
    }
    crate::code_rule_dsl::prof::time("gpu.size", || runner.size(plan))?;
    crate::code_rule_dsl::prof::release();
    mark("sized");
    let _encode = crate::code_rule_dsl::prof::Stage::start("gpu.encode");
    for (group, ((recursive, levels), &rounds)) in plan.steps.iter().zip(rounds).enumerate() {
        if !recursive {
            for level in levels {
                runner.level(level, None)?;
            }
            continue;
        }
        // The group's anchors, its own labels read as every node (Q133): once, before its sweeps or rounds.
        for level in levels {
            runner.anchors(level)?;
        }
        if sweeps(levels) {
            let reverse = complements(levels);
            for round in 0..rounds {
                runner.sweep(levels.as_ptr() as usize, group, round, reverse)?;
            }
            continue;
        }
        // A group of micro walks: its rounds to the fixpoint in one dispatch, a file a threadgroup.
        if runner.group(levels)? {
            continue;
        }
        // Rounds as the CPU's, on the GPU: every member once per round; once a round changes
        // nothing the rest are skipped (`label_settle`), and the group's end records whether it
        // settled (`label_round`).
        for round in 0..rounds {
            for (at, level) in levels.iter().enumerate() {
                runner.level(level, Some((round, at + 1 == levels.len())))?;
            }
        }
        runner.boundary(rounds);
    }
    // The roots after every pass, reading the labels in place (decision 0024 §3).
    if let Some(jobs) = plan.roots {
        runner.records(jobs)?;
    }
    drop(_encode);
    // Encoding's host vectors are freed: their pages go back before the GPU's memory is taken.
    crate::code_rule_dsl::prof::release();
    mark("encoded");
    crate::code_rule_dsl::prof::time("gpu.flush", || runner.flush())?;
    mark("flushed");
    // A node past the batch's, which a pass would have marked or counted (decision 0427): not written, and the
    // passes again on the interpreter. Said once a run: it is a tape's walk or a capture that left its tape.
    if runner.flag(0) & 256 != 0 {
        crate::code_rule_dsl::prof::count("gpu.node_past_batch", 1);
        /// Whether the run said so already (the run's own, decision 0442).
        #[derive(Default)]
        struct Said(std::sync::atomic::AtomicBool);
        if !rerail_prof::code_rule_dsl::logic::profile::with_local::<Said, _>(|said| said.0.swap(true, std::sync::atomic::Ordering::Relaxed)) {
            eprintln!("rerail: a label pass took a node past its batch's {} (decision 0427, Q265): not written; the passes on the interpreter", runner.nodes);
        }
    }
    // A chain's reach with more levels than `label_phase`'s programs hold (0463): the run again with twice them.
    if runner.flag(0) & 512 != 0 {
        crate::code_rule_dsl::prof::count("gpu.phase_levels_overflow", 1);
        return Err(Error::ChainLevels);
    }
    // A micro walk that outgrew its frames: the passes again on the interpreter.
    if runner.flag(0) & 2 != 0 {
        crate::code_rule_dsl::prof::count("gpu.frames_overflow", 1);
        return Err(Error::WalkFrames);
    }
    if runner.flag(0) & 1 != 0 {
        return Err(Error::BackendFailure);
    }
    // Decision 0334: tasks over their budget, or stopped at the ceiling. Each unfinished (pass, file) goes to W's
    // matcher where the caller has one and it is cheaper (at the ceiling, always); else the run again with twice the
    // budget (`OverBudget`), and a stop at the ceiling fails as before.
    let unfinished = runner.unfinished();
    if runner.flag(0) & (4 | 8) != 0 || !unfinished.is_empty() {
        runner.outcome.unfinished = replanned(plan, files, budget, &unfinished, runner.flag(0))?;
    }
    // The sweeps' words: a failure, and per group whether its last sweep still changed a label.
    let sweep_words = unsafe {
        std::slice::from_raw_parts(
            runner.tapes.supertypes.buffer.contents().cast::<u32>().add(2 + prefix_labels * words),
            2 + 2 * plan.steps.len(),
        )
        .to_vec()
    };
    if sweep_words[1] != 0 {
        return Err(Error::BackendFailure);
    }
    let unsettled = plan
        .steps
        .iter()
        .zip(rounds)
        .enumerate()
        .filter(|(group, ((recursive, levels), rounds))| {
            *recursive && sweeps(levels) && **rounds > 0 && sweep_words[2 + 2 * group + (**rounds - 1) % 2] != 0
        })
        .count();
    runner.outcome.waits[1] = runner.flag(3) as usize + unsettled;
    if let Some(buffers) = plan.buffers {
        let kept = Kept {
            key,
            stacks: runner.stacks.as_ref().map(|stacks| MappedBuffer::wrap(stacks.buffer.clone())),
        };
        if let Ok(mut slot) = buffers.0.lock() {
            *slot = Some(kept);
        }
    }
    // The run's queue, command buffer and other buffers go before the host's copy of the labels
    // is made: only the prefix stays (the driver's memory for the run is the footprint's peak).
    let prefix = runner.tapes.supertypes.buffer.clone();
    let records = runner.roots.take().map(|roots| decode_roots(roots, files)).unwrap_or_default();
    let mut outcome = std::mem::take(&mut runner.outcome);
    outcome.records = records;
    drop(runner);
    let result = unsafe {
        std::slice::from_raw_parts(prefix.contents().cast::<u32>().add(2), prefix_labels * words)
    };
    // Into the plan's initial labels where it can take them (no second host copy of the labels),
    // each prefix row to its label's place.
    outcome.labels = match (plan.initial.take(), &rows) {
        (Some(mut labels), None) => {
            labels.clear();
            labels.extend_from_slice(result);
            labels
        }
        (None, None) => result.to_vec(),
        (initial, Some((present, _))) => {
            let mut labels = initial.unwrap_or_default();
            labels.resize(plan.labels * words, 0);
            for (row, &label) in present.iter().enumerate() {
                let at = label as usize * words;
                labels[at..at + words].copy_from_slice(&result[row * words..(row + 1) * words]);
            }
            labels
        }
    };
    Ok(outcome)
}

/// The tape buffers, from a batch of the files with no tasks; the label prefix starts empty.
fn tapes(
    device: &Device,
    files: &[TapeDoc],
    plan: &Plan<'_>,
    words: usize,
    rows: Option<&(Vec<u32>, Vec<u32>)>,
    [scratch_words, count_words]: [usize; 2],
) -> Result<Tapes, Error> {
    let prefix_labels = rows.map_or(plan.labels, |(present, _)| present.len());
    // The labels, then the sweeps' round, failure and per group (by step) changes (`label_sweep`),
    // written in place with no host copy of the labels; then the scratch rows and tallies, each at
    // 256 bytes (32 pairs), as a buffer offset is bound. (Grammar classes are the tapes' codes.)
    let at = |pairs: usize| pairs.div_ceil(32) * 32;
    let scratch_pair = at(1 + (prefix_labels * words + 2 + 2 * plan.steps.len()).div_ceil(2));
    let counts_pair = at(scratch_pair + scratch_words.div_ceil(2));
    let prefix = at(counts_pair + count_words.max(1).div_ceil(2));
    let count = prefix;
    let supertypes = MappedBuffer::zeroed::<MetalSupertype>(device, count)?;
    unsafe {
        let head = supertypes.buffer.contents().cast::<MetalSupertype>();
        head.write(MetalSupertype { node: prefix_labels as u32, kind: words as u32 });
        let initial = plan.initial.take();
        if let Some(initial) = initial.as_ref() {
            match rows {
                None => {
                    let length = initial.len().min(plan.labels * words);
                    std::ptr::copy_nonoverlapping(initial.as_ptr(), head.add(1).cast::<u32>(), length);
                }
                Some((present, _)) => {
                    for (row, &label) in present.iter().enumerate() {
                        let from = (label as usize * words).min(initial.len());
                        let length = initial.len().saturating_sub(from).min(words);
                        std::ptr::copy_nonoverlapping(initial.as_ptr().add(from), head.add(1).cast::<u32>().add(row * words), length);
                    }
                }
            }
        }
        // Back for a retry, which builds the prefix again.
        plan.initial.set(initial);
    }
    // The documents resident, gathered into one block (decision 0289 §4), one table over them.
    let table = crate::code_rule_dsl::prof::time("residency.docs", || crate::code_rule_dsl::residency::slabs::admit_docs(files))?;
    let table = crate::code_rule_dsl::residency::slabs::DocTable::one_block(table).ok_or_else(capacity)?;
    if table.nodes != files.iter().map(TapeDoc::transducer_len).sum::<usize>() {
        return Err(Error::BackendFailure);
    }
    let docs = if table.views.is_empty() {
        MappedBuffer::zeroed::<crate::code_rule_dsl::residency::slabs::DocView>(device, 1)?
    } else {
        MappedBuffer::from_slice(device, &table.views)?
    };
    Ok(Tapes {
        scratch_at: 8 * scratch_pair as u64,
        counts_at: 8 * counts_pair as u64,
        table,
        docs,
        supertype_count: count as u32,
        supertypes,
    })
}

impl Runner<'_> {
    /// The pooled stacks every walk tier and root walk runs on (`STACK_WORDS`), made at first use
    /// and kept for the next run (`Kept`). Kernels that share them run in order (tracked).
    fn stacks(&mut self) -> Result<Buffer, Error> {
        if self.stacks.is_none() {
            self.stacks = Some(MappedBuffer::untouched::<u32>(self.device, STACK_WORDS)?);
        }
        Ok(self.stacks.as_ref().expect("stacks").buffer.clone())
    }

    /// The batch's chunk block, as kernels read documents (`SLAB_SLOT`; a stand-in for none).
    fn block(&self) -> &Buffer {
        self.tapes.table.block.as_ref().map_or(&self.flags.buffer, |block| block.buffer())
    }

    /// The block at the kernel's `chunk` row, where it has one (`LabelKernel`).
    fn bind_slabs(&self, encoder: &metal::ComputeCommandEncoderRef, slot: Option<u64>) {
        let Some(slot) = slot else { return };
        encoder.set_buffer(slot, Some(self.block()), 0);
        // The 16z documents' cache, which their reads reach from the block by the views' offsets, and write.
        if let Some(cache) = &self.tapes.table.cache {
            encoder.use_resource(cache, metal::MTLResourceUsage::Read | metal::MTLResourceUsage::Write);
        }
    }

    fn flag(&self, index: usize) -> u32 {
        unsafe { *self.flags.buffer.contents().cast::<u32>().add(index) }
    }

    /// Commits what is encoded and waits for it.
    fn flush(&mut self) -> Result<(), Error> {
        if !self.encoded {
            return Ok(());
        }
        commit_checked(&self.command)?;
        wait_checked(&self.command)?;
        crate::code_rule_dsl::prof::count("gpu.commits", 1);
        if let Some(times) = &mut self.encoder_times {
            times.read();
        }
        // A 16z block that found no room in the batch's cache read its dead entry: the batch goes back to the wave,
        // to be split, before any of its results is read.
        if let Some(cache) = &self.tapes.table.cache
            && rerail_metal::slabs::zcache_failed(cache) > 0
        {
            return Err(capacity());
        }
        {
            // The command buffer's time on the GPU (`GPUStartTime` to `GPUEndTime`).
            unsafe extern "C" {
                fn sel_registerName(name: *const std::ffi::c_char) -> *mut std::ffi::c_void;
                fn objc_msgSend();
            }
            let send: unsafe extern "C" fn(*const std::ffi::c_void, *mut std::ffi::c_void) -> f64 =
                unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
            let object = std::ptr::from_ref::<metal::CommandBufferRef>(&self.command).cast::<std::ffi::c_void>();
            let (start, end) = unsafe {
                (send(object, sel_registerName(c"GPUStartTime".as_ptr())), send(object, sel_registerName(c"GPUEndTime".as_ptr())))
            };
            crate::code_rule_dsl::prof::count("gpu.device_us", ((end - start) * 1e6) as usize);
            self.outcome.device += (end - start).max(0.0);
        }
        self.outcome.command_buffers += 1;
        for (stats, names) in self.walk_stats.drain(..) {
            let values = unsafe { std::slice::from_raw_parts(stats.buffer.contents().cast::<u32>(), 3 * names.len()) };
            let mut stages = rerail_prof::stages();
            for (name, v) in names.iter().zip(values.chunks(3)) {
                stages.entry(format!("walk_items.{name}")).or_default().1 += v[0] as usize;
                stages.entry(format!("walk_steps.{name}")).or_default().1 += v[1] as usize;
                let max = stages.entry(format!("walk_max.{name}")).or_default();
                max.1 = max.1.max(v[2] as usize);
            }
        }
        self.retained.clear();
        self.programs.clear();
        self.segment_bytes = 0;
        self.encoded = false;
        self.command = self.queue.new_command_buffer().to_owned();
        self.command.set_label("Rerail label passes");
        Ok(())
    }

    /// Profiling (`RERAIL_GPU_KERNEL_TIMES`): the command buffer so far committed and waited on,
    /// its GPU time added to `kernel_us.<label>` and `kernel_n.<label>`, and a new one begun.
    /// Kernels then run one buffer each, in the same order on the same queue.
    /// The dispatch's GPU time in µs where it is timed.
    fn split(&mut self, label: &str) -> Option<f64> {
        if !rerail_prof::code_rule_dsl::logic::profile::setting(|| std::env::var_os("RERAIL_GPU_KERNEL_TIMES").is_some()) {
            return None;
        }
        // Refused (the launch ledger is spent): the flush reports it.
        if commit_checked(&self.command).is_err() || wait_checked(&self.command).is_err() {
            return None;
        }
        if let Some(times) = &mut self.encoder_times {
            times.read();
        }
        unsafe extern "C" {
            fn sel_registerName(name: *const std::ffi::c_char) -> *mut std::ffi::c_void;
            fn objc_msgSend();
        }
        let send: unsafe extern "C" fn(*const std::ffi::c_void, *mut std::ffi::c_void) -> f64 =
            unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
        let object = std::ptr::from_ref::<metal::CommandBufferRef>(&self.command).cast::<std::ffi::c_void>();
        let (start, end) = unsafe {
            (send(object, sel_registerName(c"GPUStartTime".as_ptr())), send(object, sel_registerName(c"GPUEndTime".as_ptr())))
        };
        crate::code_rule_dsl::prof::count_named(format!("kernel_us.{label}"), ((end - start) * 1e6) as usize);
        crate::code_rule_dsl::prof::count_named(format!("kernel_n.{label}"), 1);
        // The run's GPU time too: split off here, its buffer never reaches the flush that counts it.
        crate::code_rule_dsl::prof::count("gpu.device_us", ((end - start) * 1e6) as usize);
        self.outcome.device += (end - start).max(0.0);
        self.outcome.command_buffers += 1;
        self.command = self.queue.new_command_buffer().to_owned();
        self.command.set_label("Rerail label passes");
        Some((end - start) * 1e6)
    }

    fn upload<T: Copy>(&mut self, values: &[T]) -> Result<Buffer, Error> {
        let buffer = if values.is_empty() {
            MappedBuffer::zeroed::<T>(self.device, 1)?
        } else {
            MappedBuffer::from_slice(self.device, values)?
        };
        self.segment_bytes += buffer.buffer.length() as usize;
        let handle = buffer.buffer.clone();
        self.retained.push(buffer);
        Ok(handle)
    }

    /// One label kernel over `threads`, its buffers bound in order and `shape` after them.
    fn kernel(&mut self, pipeline: usize, label: &'static str, buffers: &[(&Buffer, u64)], shape: Shape, threads: usize) {
        let kernel = self.kernels[pipeline].clone();
        self.dispatch(&kernel, label, buffers, shape, threads);
    }

    /// The clause's pattern kernel over the level's (pattern, root) items.
    fn pattern_kernel(&mut self, group: usize, buffers: &[(&Buffer, u64)], shape: Shape) -> Result<(), Error> {
        let state = self.patterns.as_ref().and_then(|(pipelines, _)| pipelines.get(group)).ok_or(Error::BackendFailure)?.clone();
        self.dispatch(&LabelKernel::pattern(state), patterns::KERNEL, buffers, shape, shape.item_count as usize);
        Ok(())
    }

    fn dispatch(&mut self, kernel: &LabelKernel, label: &str, buffers: &[(&Buffer, u64)], shape: Shape, threads: usize) {
        let state = &kernel.state;
        // The kernels that spread a SIMD group's roots over its lanes (`simd_roots`) run whole SIMD groups of 32: a lane
        // past the items holds no roots. Every tier's pool is a multiple of 32, so this stays inside its stacks.
        let threads = if label.starts_with("label_micro") || label == patterns::KERNEL {
            assert_eq!(state.thread_execution_width(), SIMD_WIDTH as u64, "simd_roots is written for SIMD groups of 32");
            threads.div_ceil(SIMD_WIDTH) * SIMD_WIDTH
        } else {
            threads
        };
        if crate::code_rule_dsl::prof::on() {
            crate::code_rule_dsl::prof::count_named(format!("threads.{label}"), threads);
            // The pipeline's thread limit: under 1,024 means the kernel's registers cap its occupancy (t41).
            crate::code_rule_dsl::prof::count_named(format!("pipeline.{label}.max_threads={}", state.max_total_threads_per_threadgroup()), 1);
        }
        if threads == 0 {
            return;
        }
        crate::code_rule_dsl::prof::count("gpu.encoders", 1);
        let encoder = encoder_of(&self.command, &mut self.encoder_times, label);
        encoder.set_label(label);
        encoder.set_compute_pipeline_state(state);
        for (index, (buffer, offset)) in buffers.iter().enumerate() {
            encoder.set_buffer(index as u64, Some(buffer), *offset);
        }
        self.bind_slabs(encoder, kernel.chunk);
        if label.starts_with("label_micro") {
            if let Some(slot) = kernel.stats {
                encoder.set_buffer(slot, Some(self.walk_bind.as_ref().unwrap_or(&self.flags.buffer)), 0);
            }
            for (slot, buffer) in kernel.spill.iter().zip(self.spill.iter().flatten().map(|b| &b.buffer).chain(std::iter::repeat(&self.flags.buffer))) {
                if let Some(slot) = slot {
                    encoder.set_buffer(*slot, Some(buffer), 0);
                }
            }
        }
        encoder.set_bytes(
            buffers.len() as u64,
            std::mem::size_of::<Shape>() as u64,
            std::ptr::from_ref(&shape).cast(),
        );
        let width = state.thread_execution_width().max(1);
        encoder.checked_dispatch_threads(
            &self.command,
            MTLSize::new(threads as u64, 1, 1),
            MTLSize::new(width.min(threads as u64), 1, 1),
        );
        encoder.end_encoding();
        self.encoded = true;
        self.outcome.dispatches += 1;
        self.split(label);
    }

    /// A recursive group's end after `rounds` rounds (`label_round`).
    fn boundary(&mut self, rounds: usize) {
        let (flags, head) = (self.flags.buffer.clone(), self.tapes.supertypes.buffer.clone());
        let shape = Shape {
            round: rounds as u32,
            finish: 1,
            ..self.shape()
        };
        self.kernel(2, KERNELS[2], &[(&flags, 0), (&head, 0)], shape, 1);
    }

    /// Every level's batch, built once, and one workspace for all of them, sized before anything
    /// is encoded so the host never waits to grow it.
    fn size(&mut self, plan: &Plan<'_>) -> Result<(), Error> {
        let mut widest_seen: Option<(WorkspaceTotals, u32, u32, usize)> = None;
        // Every pass's roots as rules, in one buffer for the run, before anything binds it.
        let _roots = crate::code_rule_dsl::prof::Stage::start("size.root_table");
        for pass in plan.steps.iter().flat_map(|(_, levels)| levels).flatten() {
            if let Pass::Program { roots, .. } | Pass::Count { roots, .. } | Pass::Pattern { roots, .. } | Pass::Formula { roots, .. } = pass {
                self.root_table.of(roots, self.files)?;
            }
        }
        self.root_table.prefix(self.files);
        // The anchors' leaves for the run (decision 0365), where their rows fit: no leaf reads a label.
        let mut leaves = Vec::new();
        for pass in plan.steps.iter().flat_map(|(_, levels)| levels).flatten() {
            let formula = match pass {
                Pass::Program { roots, .. } | Pass::Count { roots, .. } => roots.anchor(),
                Pass::Formula { formula, .. } => Some(formula),
                _ => None,
            };
            if let Some(formula) = formula {
                for &word in formula.words.iter().filter(|&&word| formula_leaf(word)) {
                    if !leaves.contains(&word) {
                        leaves.push(word);
                    }
                }
            }
        }
        if leaves.len() * self.region() * 4 <= FORMULA_SCRATCH_BYTES {
            self.run_leaves = leaves;
        }
        drop(_roots);
        crate::code_rule_dsl::prof::count("size.root_words", self.root_table.words.len());
        self.root_words = Some(MappedBuffer::from_slice(self.device, &self.root_table.words)?);
        // A sweeping group is one batch of all its programs, by its address; a level is one by
        // its own.
        let batches = plan.steps.iter().flat_map(|(recursive, levels)| {
            if *recursive && sweeps(levels) {
                vec![(levels.as_ptr() as usize, levels.iter().flat_map(|level| programs_of(level, false)).collect::<Vec<_>>(), Some(levels), levels.iter().flatten().collect::<Vec<_>>())]
            } else {
                levels.iter().map(|level| (level.as_ptr() as usize, programs_of(level, true), None, level.iter().collect::<Vec<_>>())).collect()
            }
        });
        let bases = self.files.iter().scan(0_u32, |base, tape| {
            let at = *base;
            *base += tape.transducer_len() as u32;
            Some(at)
        }).collect::<Vec<_>>();
        // Each distinct (value, kind)'s row of bits over every file's nodes, evaluated once for
        // the run's levels (06 S7: per level, 58 evaluations); `None` where one failed.
        let own = TextRows::default();
        let mut evaluated = plan.rows.unwrap_or(&own).lock().map_err(|_| Error::BackendFailure)?;
        for (key, mut passes, sweep, all) in batches {
            // A level's micro walks run as such (`label_micro`); its other programs are tasks.
            if self.micro && sweep.is_none() {
                let mut words = Vec::new();
                // Per tier, its passes; their threads counted from 0 in each.
                let mut records: [Vec<[u32; 8]>; 3] = Default::default();
                let (largest, mut longest) = (self.files.iter().map(TapeDoc::transducer_len).max().unwrap_or(0), 0);
                // Each text slot's bits, `self.words` words a slot over every file's nodes.
                let (mut texts, mut slots) = (Vec::<u32>::new(), 0_u32);
                let label_words = self.words;
                let files = self.files;
                // Each distinct (value, kind)'s slot row, computed once for the run's levels.
                let mut rows = HashMap::<String, Option<u32>>::new();
                let root_table = &self.root_table;
                // Threads: a pass's from its own start, over its (shared) item range.
                let mut threads = [0_u32; 3];
                let mut fused = Vec::new();
                // Passes walked from one shared root list, marking roots or captures, as one
                // walk (multi-target packing, t1 spec-fusion §2.6, §4.3, §4.4; the host's
                // `matcher::Packs`): an emit marks its member's row (by emit label, `rows`).
                let packs = {
                    let _packs = crate::code_rule_dsl::prof::Stage::start("size.fuse");
                    let mut by = std::collections::BTreeMap::<(usize, u32), Vec<usize>>::new();
                    for (at, &(program, roots, mode, _)) in passes.iter().enumerate() {
                        if mode <= 1 && Micro::of(program).is_some() {
                            by.entry((std::sync::Arc::as_ptr(roots) as usize, mode)).or_default().push(at);
                        }
                    }
                    let mut packs = Vec::new();
                    for members in by.into_values() {
                        for chunk in members.chunks(super::fusion::MEMBERS).filter(|chunk| chunk.len() > 1) {
                            let programs = chunk.iter().map(|&at| passes[at].0).collect::<Vec<_>>();
                            // In its members' deepest tier: the launch plan counts a level's
                            // tiers from its passes' walks (`plan_rounds`).
                            let deepest = programs.iter().filter_map(|program| Micro::of(program)).map(|walk| tier(&walk)).max();
                            if let Some(packed) = super::fusion::Fused::of(&programs)
                                && Micro::of(&packed.program).map(|walk| tier(&walk)) <= deepest
                            {
                                let rows = packed.emits.iter().map(|&(member, _)| passes[chunk[member]].3).collect::<Vec<_>>();
                                packs.push((packed.program, chunk.to_vec(), rows));
                            }
                        }
                    }
                    packs
                };
                let mut place = |program: &Program, roots: &std::sync::Arc<Roots>, mode: u32, target: u32, table: Option<Vec<u32>>| -> bool {
                    let Some(walk) = Micro::of(program) else { return true };
                    // The rule, shared by passes of its domain; a thread a group of the batch.
                    let Ok((start, any)) = root_table.at(roots) else { return true };
                    let count = if any { root_table.groups() } else { 0 };
                    // The profile counts the groups a walk launches, which the rule gives; the roots it visits are
                    // the device's own count (`walk_items.*`). Counting them here walked every root node of every
                    // file for every pass: 173 of `gpu.size`'s 217 ms on s1000 (2026-11-17).
                    if crate::code_rule_dsl::prof::on() {
                        let depth = walk.depth().map_or("loop".to_owned(), |depth| format!("{:02}", depth.min(64)));
                        if walk.depth().is_none() && std::env::var_os("RERAIL_DUMP_LOOP").is_some() {
                            eprintln!("LOOP {:?}", walk.ops());
                        }
                        crate::code_rule_dsl::prof::count_named(format!("depth_groups.{depth}"), count as usize);
                    }
                    // The walk's slots as rows of `texts`, contiguous from `text_start`; a row
                    // already computed for another pass is copied.
                    let mut own = Vec::new();
                    for slot in 0..walk.texts.len() {
                        let key = walk.slot_key(program, slot);
                        let row = match rows.get(&key) {
                            Some(row) => *row,
                            None => {
                                let bits = evaluated
                                    .entry(key.clone())
                                    .or_insert_with(|| text_row(&walk, program, slot, files, &bases, label_words))
                                    .clone();
                                let row = bits.map(|bits| {
                                    let row = slots;
                                    slots += 1;
                                    texts.extend_from_slice(&bits);
                                    row
                                });
                                rows.insert(key.clone(), row);
                                row
                            }
                        };
                        let Some(row) = row else {
                            if crate::code_rule_dsl::prof::on() {
                                crate::code_rule_dsl::prof::count_named(format!("unslotted_groups.{key}"), count as usize);
                            }
                            return true;
                        };
                        own.push(row);
                    }
                    // A pass reads slot `s` at `text_start + s`: its rows, copied where shared.
                    let text_start = slots;
                    for &row in &own {
                        let from = row as usize * label_words;
                        texts.extend_from_within(from..from + label_words);
                    }
                    slots += own.len() as u32;
                    let tier = tier(&walk);
                    // A pack's route has bit 2, its emit count and rows after its words.
                    let mode = if table.is_some() { mode | 4 } else { mode };
                    fused.push(([words.len() as u32, walk.words.len() as u32, target, mode, start, count, text_start, 0], walk.depth()));
                    records[tier].push([words.len() as u32, walk.words.len() as u32, target, mode, start, count, text_start, threads[tier]]);
                    threads[tier] += count;
                    longest = longest.max(walk.words.len());
                    // A label class test reads its label's prefix row (`label_rows`).
                    match &self.label_rows {
                        None => words.extend_from_slice(&walk.words),
                        Some(row_of) => words.extend(walk.words.iter().map(|&word| {
                            let (op, mode, arg) = super::super::split(word);
                            if op == super::super::SUPERTYPE && arg >= DERIVED_CLASS_BASE {
                                let row = row_of[usize::from(arg - DERIVED_CLASS_BASE)];
                                super::super::word(op, mode, DERIVED_CLASS_BASE + row as u16)
                            } else {
                                word
                            }
                        })),
                    }
                    if let Some(table) = table {
                        words.push(table.len() as u32);
                        words.extend(table);
                    }
                    false
                };
                let _placing = crate::code_rule_dsl::prof::Stage::start("size.place");
                let mut unpacked = vec![false; passes.len()];
                let mut placed = 0_usize;
                for (program, members, rows) in &packs {
                    let (_, roots, mode, target) = passes[members[0]];
                    if place(program, roots, mode, target, Some(rows.clone())) {
                        // Its walk did not place (a text slot failed): its members run alone.
                        continue;
                    }
                    crate::code_rule_dsl::prof::count("size.packed", members.len());
                    crate::code_rule_dsl::prof::count("size.packs", 1);
                    for &at in members {
                        unpacked[at] = true;
                    }
                    placed += members.len();
                }
                let mut at = 0;
                passes.retain(|&(program, roots, mode, target)| {
                    at += 1;
                    if unpacked[at - 1] {
                        return false;
                    }
                    let alone = place(program, roots, mode, target, None);
                    placed += usize::from(!alone);
                    alone
                });
                drop(_placing);
                if !words.is_empty() {
                    if texts.is_empty() {
                        texts.push(0);
                    }
                    let mut tiers = Vec::new();
                    for (tier, records) in records.iter().enumerate() {
                        if !records.is_empty() {
                            tiers.push((tier, MappedBuffer::from_slice(self.device, records)?, records.len(), threads[tier] as usize));
                        }
                    }
                    let walks = Walks {
                        slots: rows.iter().filter_map(|(key, row)| row.map(|row| (key.clone(), row))).collect(),
                        texts: MappedBuffer::from_slice(self.device, &texts)?,
                        steps: super::super::micro_steps(largest, longest),
                        tiers,
                        fused,
                        walked: placed,
                        words: MappedBuffer::from_slice(self.device, &words)?,
                    };
                    self.walks.insert(key, walks);
                }
            }
            if passes.is_empty() {
                continue;
            }
            if crate::code_rule_dsl::prof::on() {
                crate::code_rule_dsl::prof::count(if sweep.is_some() { "size.sweep_batches" } else { "size.level_batches" }, 1);
                for &(program, _, _, _) in &passes {
                    let reason = if sweep.is_some() && Micro::of(program).is_some() { "sweep" } else { Micro::refusal(program) };
                    crate::code_rule_dsl::prof::count_named(format!("refused.{reason}"), 1);
                }
            }
            let programs = passes.iter().map(|(program, ..)| (*program).clone()).collect::<Vec<_>>();
            // The level's programs over the resident documents: nothing of the tapes is packed,
            // and no task is a record (decision 0033). An evaluation per (file, program) with
            // roots; its tasks are its document's groups of 64 nodes, whose roots the device finds
            // from the rule (`root_mask`).
            let no_anchors = vec![vec![Vec::<u32>::new(); programs.len()]; self.files.len()];
            let mut batch = crate::code_rule_dsl::prof::time("size.batch", || PackedBatch::new_resident(&programs, self.files, &no_anchors))?;
            let mut specs = Vec::new();
            {
                let rules = passes.iter().map(|(_, roots, ..)| self.root_table.at(roots)).collect::<Result<Vec<_>, Error>>()?;
                for file in 0..self.files.len() {
                    for (program, ((_, roots, ..), (at, _))) in passes.iter().zip(&rules).enumerate() {
                        if roots.runs_in(file) {
                            specs.push([program as u32, file as u32, *at, 0, RootTable::file_groups(self.files, file)]);
                        }
                    }
                }
            }
            let task_count = specs.iter().map(|spec| spec[4] as usize).sum::<usize>();
            crate::code_rule_dsl::prof::count("size.tasks", task_count);
            if task_count == 0 {
                self.prepared.insert(key, None);
                continue;
            }
            // A label pass's output is bits, not records: each program marks its route as it
            // publishes (`packed_batch::MARKS`, `label_mark` in `run_task_in`): its root or its
            // captured nodes, into its scratch row or its tally slot (a sweep's: its label), so it
            // keeps no results or final captures. (Kept as records, both were reserved per task
            // at their worst case and summed: 4.2 M results and 8.4 M captures on eight files,
            // over the buffer budget.) A program that marks its roots keeps its first (O35).
            use super::super::packed_batch::{MARKS, MARKS_CAPTURED, ONE_RESULT, TALLY};
            for (program, (_, _, mode, slot)) in batch.programs.iter_mut().zip(&passes) {
                if *slot >= 1 << 22 {
                    return Err(capacity());
                }
                program.flags |= MARKS | (slot << 8);
                if matches!(*mode, 1 | 3) {
                    program.flags |= MARKS_CAPTURED;
                }
                if matches!(*mode, 2 | 3) {
                    program.flags |= TALLY;
                }
                if *mode == 0 {
                    program.flags |= ONE_RESULT;
                }
            }
            // A sweep's rows are its labels (after the prefix's head); a level's, its scratch rows.
            (batch.header.mark_scratch, batch.header.mark_counts) =
                if sweep.is_some() { (2, 0) } else { ((self.tapes.scratch_at / 4) as u32, (self.tapes.counts_at / 4) as u32) };
            // A sweep's work at a node: the group's programs and complements in order, each
            // program with its rule (`label_sweep`), in place of a list of every node's work.
            let sweep_table = sweep.map(|levels| {
                let (mut entries, mut rules) = (Vec::new(), Vec::new());
                let mut program = 0_u32;
                for pass in levels.iter().flatten() {
                    match pass {
                        Pass::Program { roots, .. } | Pass::Count { roots, .. } => {
                            entries.push([0, program, 0, rules.len() as u32]);
                            rules.extend(roots.device_rule()?);
                            program += 1;
                        }
                        Pass::Complement { target, of } => entries.push([1, *target, *of, 0]),
                        _ => {}
                    }
                }
                // Rules follow the entries: their words are offset past them.
                let offset = 1 + 4 * entries.len() as u32;
                let mut table = vec![entries.len() as u32];
                table.extend(entries.iter().flat_map(|entry| if entry[0] == 0 { [entry[0], entry[1], entry[2], entry[3] + offset] } else { *entry }));
                table.extend(rules);
                Ok::<_, Error>(table)
            }).transpose()?;
            // The label batch's rows are the labels' (`tapes`): W's matchers' programs have no capture predicates, so
            // no contains test's row (`PackedBatch::contains_rows`) is left out.
            assert_eq!(batch.contains_row_count(), 0, "a label pass's program with a contains test");
            batch.header.supertype_count = self.tapes.supertype_count;
            let (lanes, states) = if sweep.is_some() { WIDE } else { self.lanes };
            batch.state_cap = states;
            batch.resident_budget = Some(self.budget);
            // As many lanes as the free budget holds (O38), each with the states a task takes:
            // a workspace is a fixed part and a part per lane, measured at one lane and at two.
            let lanes = crate::code_rule_dsl::prof::time("size.lanes", || fit_lanes(&mut batch, &programs, &specs, lanes))?;
            crate::code_rule_dsl::prof::count("size.lanes", lanes);
            let totals = crate::code_rule_dsl::prof::time("size.assign", || batch.assign_resident(&programs, &specs, lanes))?;
            crate::code_rule_dsl::prof::count("size.evaluations", batch.evaluations.len());
            // A status a lane (`run_batch`'s resident form).
            let seen = (totals, batch.header.scheduler_byte_count, batch.header.workspace_byte_count, batch.evaluation_lanes.len().max(1));
            widest_seen = Some(match widest_seen {
                None => seen,
                Some(old) => (old.0.widest(seen.0), old.1.max(seen.1), old.2.max(seen.2), old.3.max(seen.3)),
            });
            // Marking programs publish no results, so `label_sink` routes none.
            let routes = [[0_u32; 2]];
            // In the tasks' place: the run's root rules, a sweep's table.
            let tasks = match &sweep_table {
                Some(table) => MappedBuffer::from_slice(self.device, table)?,
                None => MappedBuffer::wrap(self.root_words.as_ref().ok_or(Error::BackendFailure)?.buffer.clone()),
            };
            // Each program's pass, by the program it runs (a pass's own: distinct labels, distinct programs).
            let reported = passes
                .iter()
                .map(|&(program, ..)| {
                    let target = all
                        .iter()
                        .find_map(|pass| match pass {
                            Pass::Program { target, program: own, .. } | Pass::Count { target, program: own, .. } if std::ptr::eq(*own, program) => Some(*target),
                            _ => None,
                        })
                        .unwrap_or(u32::MAX);
                    (target, super::super::evaluation_costs::per_node(program), program.plain_digest().to_owned())
                })
                .collect();
            let prepared = Prepared {
                passes: reported,
                tasks,
                header: MappedBuffer::from_slice(self.device, &[batch.header])?,
                routes: MappedBuffer::from_slice(self.device, &routes)?,
                program: crate::code_rule_dsl::prof::time("size.program_buffers", || self.engine.program_buffers(self.device, &batch))?,
                arenas: batch.global_arenas,
                task_count,
                result_capacity: batch.global_arenas.arenas[ArenaKind::Result as usize].capacity as usize,
                evaluations: std::mem::take(&mut batch.evaluations),
                lanes: std::mem::take(&mut batch.evaluation_lanes),
            };
            self.prepared.insert(key, Some(prepared));
        }
        if let Some((totals, scheduler_bytes, output_bytes, tasks)) = widest_seen {
            // Lanes times states is the arena: one over the buffer budget runs again with fewer states.
            let acquire = || crate::code_rule_dsl::prof::time("size.lease", || MetalWorkspaceLease::acquire(self.device, totals, scheduler_bytes, output_bytes, tasks));
            let mut lease = match acquire() {
                // Not room now: admitted as a stratum's tables are (Q203, Q203), its need in bytes, waiting for what
                // threads that go on hold to come back while waiting is worth it; its buffers are then drawn from the
                // reservation. Refused, the run goes on with fewer states (`run_batch`), then the wave splits.
                Err(Error::BackendCapacity) => {
                    let bytes = MetalWorkspace::bytes_for(totals, scheduler_bytes, output_bytes, tasks);
                    match rerail_accel::gpu::shared::admit(bytes, crate::code_rule_dsl::logic::plan::placement::admission_patience(self.nodes as f64)) {
                        Ok(admitted) => {
                            crate::code_rule_dsl::prof::count("wave.admitted", 1);
                            if admitted.waited > 0.0 {
                                crate::code_rule_dsl::prof::count("wave.admission_waits", 1);
                            }
                            let lease = acquire();
                            drop(admitted);
                            lease
                        }
                        Err(why) => {
                            crate::code_rule_dsl::prof::count("wave.admission_refused", 1);
                            crate::code_rule_dsl::logic::profile::decision("a matcher wave's workspace not admitted (Q203)", || format!("{} MB: {why}", bytes >> 20));
                            Err(Error::BackendCapacity)
                        }
                    }
                }
                got => got,
            }
            .map_err(|error| if error == Error::BackendCapacity { Error::BackendBudget } else { error })?;
            crate::code_rule_dsl::prof::count("size.lease_mb", lease.bytes() >> 20);
            // Each encoded level advances the generation itself.
            lease.generation_span = 0;
            self.workspace = Some(Workspace {
                lease,
                totals,
                scheduler_bytes,
                output_bytes,
                tasks,
            });
        }
        Ok(())
    }

    /// A level's formulas (decision 0365), by `label_phase`, before its walks: the anchors of its passes not yet
    /// evaluated, each into its rule's row the walks read, and its formula passes (`Pass::Formula`), each into its
    /// scratch row within its rule. The labels a formula reads are the levels' before, settled in the command buffer by
    /// now; a label test reads the label's row in the prefix (`label_rows`), a text test the row the level's walks read.
    fn anchors(&mut self, passes: &[Pass<'_>]) -> Result<(), Error> {
        let key = passes.as_ptr() as usize;
        let slots = self.walks.get(&key).map(|walks| &walks.slots);
        let texts = |formula: &DeviceFormula| formula.texts.iter().map(|key| slots.and_then(|slots| slots.get(key).copied())).collect::<Vec<_>>();
        let mut list = Vec::new();
        for (at, pass) in passes.iter().enumerate() {
            match pass {
                Pass::Program { roots, .. } | Pass::Count { roots, .. } => {
                    let Some(anchor) = roots.anchor() else { continue };
                    let address = std::sync::Arc::as_ptr(roots) as usize;
                    let Some(&(row, mode)) = self.root_table.anchors.get(&address) else { return Err(Error::BackendFailure) };
                    if !self.anchored.insert(address) {
                        continue;
                    }
                    let (rule, _) = self.root_table.at(roots)?;
                    list.push(FormulaJob { formula: std::sync::Arc::clone(anchor), row, mode, rule, texts: texts(anchor) });
                }
                Pass::Formula { roots, formula, image, .. } => {
                    crate::code_rule_dsl::prof::count(if *image { "gpu.formula_images" } else { "gpu.formula_passes" }, 1);
                    let (rule, _) = self.root_table.at(roots)?;
                    let row = at as u32 * self.words as u32;
                    let mode = if *image { 3 } else { 2 };
                    list.push(FormulaJob { formula: std::sync::Arc::clone(formula), row, mode, rule, texts: texts(formula) });
                }
                _ => {}
            }
        }
        let rows = self.root_words.as_ref().ok_or(Error::BackendFailure)?.buffer.clone();
        let text_rows = self.walks.get(&key).map(|walks| walks.texts.buffer.clone());
        self.formulas(&list, &rows, text_rows.as_ref())
    }

    /// `label_phase` over `list` (each formula's row, in `rows` for modes 0 and 1 and in scratch for 2; its rule, `u32::MAX`
    /// none; its text rows in `texts`), after `label_leaves` (decision 0365): the formulas in groups whose distinct leaf
    /// tests' rows fit `FORMULA_SCRATCH_BYTES`, each group's leaves read in one pass over the nodes.
    fn formulas(&mut self, list: &[FormulaJob], rows: &Buffer, texts: Option<&Buffer>) -> Result<(), Error> {
        if list.is_empty() {
            return Ok(());
        }
        let region = self.region();
        let most = (FORMULA_SCRATCH_BYTES / 4 / region).max(1);
        // The run's leaves (`size`), read once at the first level that needs them, where they hold every leaf here.
        if !self.run_leaves.is_empty() && list.iter().all(|job| job.formula.words.iter().all(|&word| !formula_leaf(word) || self.run_leaves.contains(&word))) {
            if self.run_leaf_rows.is_none() {
                let rows = MappedBuffer::untouched::<u32>(self.device, region * self.run_leaves.len())?;
                let tests = self.run_leaves.clone();
                self.leaves_pass(&tests, &rows.buffer)?;
                self.segment_bytes += rows.buffer.length() as usize;
                self.run_leaf_rows = Some(rows);
            }
            let (tests, leaves) = (self.run_leaves.clone(), self.run_leaf_rows.as_ref().expect("the run's leaves").buffer.clone());
            return self.formula_group(list, &tests, Some(&leaves), region, rows, texts);
        }
        let mut at = 0;
        while at < list.len() {
            // The group: formulas while their leaves fit (one at least).
            let mut tests = Vec::<u32>::new();
            let mut end = at;
            while end < list.len() {
                let mut with = tests.clone();
                for &word in list[end].formula.words.iter().filter(|&&word| formula_leaf(word)) {
                    if !with.contains(&word) {
                        with.push(word);
                    }
                }
                if end > at && with.len() > most {
                    break;
                }
                tests = with;
                end += 1;
            }
            self.formula_group(&list[at..end], &tests, None, region, rows, texts)?;
            at = end;
        }
        Ok(())
    }

    /// A leaf's row, and a formula's scratch a slot: each file's words at its region (`FORMULA_REGION`).
    fn region(&self) -> usize {
        self.words + 2 * self.files.len() + 2
    }

    /// `label_leaves`: `tests`' rows into `leaves`.
    fn leaves_pass(&mut self, tests: &[u32], leaves: &Buffer) -> Result<(), Error> {
        if tests.is_empty() {
            return Ok(());
        }
        let tests_buffer = self.upload(tests)?;
        let docs = self.tapes.docs.buffer.clone();
        let shape = Shape { target_count: tests.len() as u32, recursive: self.region() as u32, ..self.shape() };
        let encoder = encoder_of(&self.command, &mut self.encoder_times, KERNELS[LEAVES_KERNEL]);
        encoder.set_label(KERNELS[LEAVES_KERNEL]);
        crate::code_rule_dsl::prof::count_named(format!("kernel.{}", KERNELS[LEAVES_KERNEL]), 1);
        crate::code_rule_dsl::prof::count("gpu.anchor_leaves", tests.len());
        if crate::code_rule_dsl::prof::on() {
            for test in tests {
                crate::code_rule_dsl::prof::count_named(format!("gpu.anchor_leaves.op{}", test & 255), 1);
            }
        }
        encoder.set_compute_pipeline_state(&self.kernels[LEAVES_KERNEL].state);
        for (index, buffer) in [&docs, &tests_buffer, leaves].into_iter().enumerate() {
            encoder.set_buffer(index as u64, Some(buffer), 0);
        }
        encoder.set_bytes(3, std::mem::size_of::<Shape>() as u64, std::ptr::from_ref(&shape).cast());
        self.bind_slabs(encoder, self.kernels[LEAVES_KERNEL].chunk);
        // A thread a word of the rows' region (`region`: the files' words, two spare a file, two at the end).
        let threads = self.region() as u64;
        let width = LEAVES_WIDTH.min(self.kernels[LEAVES_KERNEL].state.max_total_threads_per_threadgroup());
        encoder.checked_dispatch_threads(&self.command, MTLSize::new(threads, 1, 1), MTLSize::new(width.min(threads), 1, 1));
        encoder.end_encoding();
        self.outcome.dispatches += 1;
        self.encoded = true;
        self.split(KERNELS[LEAVES_KERNEL]);
        Ok(())
    }

    /// One group of `formulas`: its leaves (`tests`; read here unless `leaves` holds them), then its formulas in
    /// dispatches of `FORMULA_SCRATCH_BYTES` of scratch.
    fn formula_group(&mut self, list: &[FormulaJob], tests: &[u32], leaves: Option<&Buffer>, region: usize, rows: &Buffer, texts: Option<&Buffer>) -> Result<(), Error> {
        use crate::code_rule_dsl::transducer::packed::compile_sets::device_ops;
        // Each formula's words as the device reads them: a leaf by its row, a label by its row in the prefix
        // (`label_rows`), a text test by its row in `texts`. A label or text without a row is every node, which only
        // an anchor has (a formula pass's labels are in the prefix, and it has no text test).
        let lowered = |job: &FormulaJob| -> Vec<u32> {
            job.formula
                .words
                .iter()
                .map(|&word| {
                    let (op, arg) = (word & 255, word >> 8);
                    if let Some(index) = tests.iter().position(|&test| test == word) {
                        device_ops::LEAF | (index as u32) << 8
                    } else if op == device_ops::CLASS {
                        let label = arg - u32::from(DERIVED_CLASS_BASE);
                        let row = self.label_rows.as_ref().map_or(Some(label), |rows| rows.get(label as usize).copied().filter(|&row| row != u32::MAX));
                        row.map_or(device_ops::ALL, |row| op | (u32::from(DERIVED_CLASS_BASE) + row) << 8)
                    } else if op == device_ops::TEXT {
                        let row = job.texts.get(arg as usize).copied().flatten().filter(|_| texts.is_some());
                        crate::code_rule_dsl::prof::count(if row.is_some() { "gpu.anchor_texts" } else { "gpu.anchor_texts_unread" }, 1);
                        row.map_or(device_ops::ALL, |row| op | row << 8)
                    } else {
                        word
                    }
                })
                .collect()
        };
        // Identical formulas once, each with its targets.
        let mut unique = Vec::<(Vec<u32>, Vec<[u32; 3]>)>::new();
        let mut at = HashMap::<Vec<u32>, usize>::new();
        for job in list {
            let words = lowered(job);
            match at.get(&words) {
                Some(&index) => unique[index].1.push([job.row, job.mode, job.rule]),
                None => {
                    at.insert(words.clone(), unique.len());
                    unique.push((words, vec![[job.row, job.mode, job.rule]]));
                }
            }
        }
        // Each formula as phases (0463). Every step a device formula has compiles; one that did not would have no
        // kernel to run it.
        let mut programs = Vec::with_capacity(unique.len());
        for (words, _) in &unique {
            let Some(program) = phase_program(words, self.chain_levels) else {
                eprintln!("rerail: a device formula with a step label_phase does not hold: {words:?}");
                return Err(Error::BackendFailure);
            };
            programs.push(program);
        }
        let mut headers = Vec::<u32>::new();
        let mut targets = Vec::<[u32; 3]>::new();
        let mut words = Vec::<u32>::new();
        for (own, to) in &unique {
            headers.extend([targets.len() as u32, to.len() as u32, words.len() as u32, own.len() as u32, 0]);
            targets.extend(to);
            words.extend(own);
        }
        crate::code_rule_dsl::prof::count("gpu.anchors", list.len());
        crate::code_rule_dsl::prof::count("gpu.anchors_distinct", unique.len());
        let (headers, words, targets) = (self.upload(&headers)?, self.upload(&words)?, self.upload(&targets)?);
        let own = match leaves {
            Some(_) => None,
            None => {
                let own = MappedBuffer::untouched::<u32>(self.device, region * tests.len().max(1))?;
                self.leaves_pass(tests, &own.buffer)?;
                Some(own)
            }
        };
        let leaves = leaves.cloned().unwrap_or_else(|| own.as_ref().expect("the group's leaves").buffer.clone());
        let docs = self.tapes.docs.buffer.clone();
        let head = self.tapes.supertypes.buffer.clone();
        let rules = self.root_words.as_ref().ok_or(Error::BackendFailure)?.buffer.clone();
        self.phase_dispatches(&programs, region, &[&docs, &head, &headers, &words, rows], &leaves, &targets, &rules, texts)?;
        if let Some(own) = own {
            self.segment_bytes += own.buffer.length() as usize;
            self.retained.push(own);
        }
        self.encoded = true;
        Ok(())
    }

    /// `label_phase` (0461): `programs`' formulas (the first of the group's headers), a dispatch a phase, a thread a
    /// (word of the region, formula). Their slots in a scratch of their own, as many formulas at once as it holds.
    #[allow(clippy::too_many_arguments)]
    fn phase_dispatches(&mut self, programs: &[PhaseProgram], region: usize, inputs: &[&Buffer; 5], leaves: &Buffer, targets: &Buffer, rules: &Buffer, texts: Option<&Buffer>) -> Result<(), Error> {
        let slots = programs.iter().map(|program| program.slots).max().unwrap_or(1).max(1) as usize;
        let at_once = (FORMULA_SCRATCH_BYTES / 4 / (slots * region)).clamp(1, programs.len());
        if self.phase_scratch.as_ref().is_none_or(|scratch| (scratch.buffer.length() as usize) < 4 * slots * region * at_once) {
            // Not an upload: one buffer for the run, at most `FORMULA_SCRATCH_BYTES`, as the formulas' is.
            let grown = MappedBuffer::untouched::<u32>(self.device, slots * region * at_once)?;
            if let Some(old) = self.phase_scratch.replace(grown) {
                self.retained.push(old);
            }
        }
        let scratch = self.phase_scratch.as_ref().expect("the phases' scratch").buffer.clone();
        let grid = (region as u64).div_ceil(PHASE_WORDS) * PHASE_WORDS;
        if self.phase_parents.is_none() {
            // Not an upload: a derived answer (each node's parent), written once where it is computed (0353).
            let parents = MappedBuffer::untouched::<u32>(self.device, 32 * region)?;
            let LabelKernel { state, chunk, .. } = self.kernels[PARENTS_KERNEL].clone();
            assert!(state.max_total_threads_per_threadgroup() >= PHASE_WORDS, "label_parents takes {PHASE_WORDS} threads a group");
            let encoder = encoder_of(&self.command, &mut self.encoder_times, KERNELS[PARENTS_KERNEL]);
            encoder.set_label(KERNELS[PARENTS_KERNEL]);
            encoder.set_compute_pipeline_state(&state);
            encoder.set_buffer(0, Some(inputs[0]), 0);
            encoder.set_buffer(1, Some(&parents.buffer), 0);
            let shape = Shape { recursive: region as u32, ..self.shape() };
            encoder.set_bytes(2, std::mem::size_of::<Shape>() as u64, std::ptr::from_ref(&shape).cast());
            self.bind_slabs(encoder, chunk);
            crate::code_rule_dsl::prof::count_named(format!("kernel.{}", KERNELS[PARENTS_KERNEL]), 1);
            encoder.checked_dispatch_threads(&self.command, MTLSize::new(grid, 1, 1), MTLSize::new(PHASE_WORDS, 1, 1));
            self.outcome.dispatches += 1;
            encoder.end_encoding();
            let _ = self.split(KERNELS[PARENTS_KERNEL]);
            self.phase_parents = Some(parents);
        }
        let parents = self.phase_parents.as_ref().expect("the batch's parents").buffer.clone();
        let LabelKernel { state, chunk, .. } = self.kernels[PHASE_KERNEL].clone();
        crate::code_rule_dsl::prof::count("gpu.formula_phased", programs.len());
        for first in (0..programs.len()).step_by(at_once) {
            let own = &programs[first..(first + at_once).min(programs.len())];
            // Each formula's header (its instructions' first word, their count, its result, its phases), then the
            // instructions, then each phase's first instruction.
            let mut program = vec![0_u32; 4 * own.len()];
            for (index, compiled) in own.iter().enumerate() {
                let at = program.len() as u32;
                program.extend(compiled.code.iter().flatten());
                program.push(0);
                program.extend(compiled.code.iter().enumerate().filter(|(_, code)| code[0] == PHASE_END).map(|(i, _)| i as u32 + 1));
                program[4 * index..4 * index + 4].copy_from_slice(&[at, compiled.code.len() as u32, compiled.result, compiled.phases]);
            }
            let phases = own.iter().map(|program| program.phases).max().unwrap_or(1);
            crate::code_rule_dsl::prof::count("gpu.formula_phases", phases as usize);
            let program = self.upload(&program)?;
            let encoder = encoder_of(&self.command, &mut self.encoder_times, KERNELS[PHASE_KERNEL]);
            encoder.set_label(KERNELS[PHASE_KERNEL]);
            encoder.set_compute_pipeline_state(&state);
            for (index, buffer) in inputs.iter().enumerate() {
                encoder.set_buffer(index as u64, Some(buffer), 0);
            }
            encoder.set_buffer(5, Some(&scratch), 0);
            encoder.set_buffer(7, Some(leaves), 0);
            encoder.set_buffer(8, Some(&program), 0);
            encoder.set_buffer(9, Some(targets), 0);
            encoder.set_buffer(10, Some(&self.tapes.supertypes.buffer), self.tapes.scratch_at);
            encoder.set_buffer(11, Some(rules), 0);
            encoder.set_buffer(12, Some(texts.unwrap_or(rules)), 0);
            encoder.set_buffer(13, Some(&self.flags.buffer), 0);
            encoder.set_buffer(14, Some(&parents), 0);
            self.bind_slabs(encoder, chunk);
            // A threadgroup a block of `PHASE_WORDS` words, whole: its barriers are every thread's.
            assert!(state.max_total_threads_per_threadgroup() >= PHASE_WORDS, "label_phase takes {PHASE_WORDS} threads a group");
            for round in 0..phases {
                let shape = Shape { item_count: first as u32, target_count: own.len() as u32, round, recursive: region as u32, queue: (slots * region) as u32, ..self.shape() };
                encoder.set_bytes(6, std::mem::size_of::<Shape>() as u64, std::ptr::from_ref(&shape).cast());
                crate::code_rule_dsl::prof::count_named(format!("kernel.{}", KERNELS[PHASE_KERNEL]), 1);
                encoder.checked_dispatch_threads(&self.command, MTLSize::new(grid, own.len() as u64, 1), MTLSize::new(PHASE_WORDS, 1, 1));
                self.outcome.dispatches += 1;
            }
            encoder.end_encoding();
            let _ = self.split(KERNELS[PHASE_KERNEL]);
        }
        Ok(())
    }

    fn shape(&self) -> Shape {
        Shape {
            node_count: self.nodes as u32,
            words: self.words as u32,
            file_count: self.files.len() as u32,
            ..Shape::default()
        }
    }

    /// One dependency level: its programs in one dispatch, their results sunk, the derived
    /// labels, then every target settled.
    /// `round`: in a recursive group, the round and whether this is its last level.
    fn level(&mut self, passes: &[Pass<'_>], round: Option<(usize, bool)>) -> Result<(), Error> {
        let programs = programs_of(passes, true);
        // How each target settles: from scratch, or computed by `label_settle` per word.
        let mut settles = Vec::new();
        let mut constraints = Vec::new();
        // Per pattern kernel (`patterns::GROUP`), its items and their threads; facts in the first.
        let groups = self.patterns.as_ref().map_or(0, |(pipelines, _)| pipelines.len());
        let mut matches = vec![Vec::<u32>::new(); groups];
        let mut pattern_threads = vec![0_u32; groups];
        let tallies = count_rows(passes);
        // A label's place in the prefix (`label_rows`).
        let rows = self.label_rows.clone();
        let place = |label: &u32| rows.as_ref().map_or(*label, |rows| rows[*label as usize]);
        for (row, pass) in passes.iter().enumerate() {
            let tally = tallies[row];
            let row = row as u32;
            settles.push(match pass {
                pass @ (Pass::Program { .. } | Pass::Empty { .. } | Pass::Count { .. } | Pass::Complement { .. } | Pass::Formula { .. }) => {
                    settle(pass, row, tally, place).expect("a settle")
                }
                Pass::Pattern {
                    target,
                    pattern,
                    roots,
                    constraints: named,
                } => {
                    let vars = &self.patterns.as_ref().ok_or(Error::BackendFailure)?.1[*pattern];
                    let start = (constraints.len() / 2) as u32;
                    for (name, label) in named {
                        if let Some(var) = vars.iter().position(|known| known == name) {
                            constraints.extend([var as u32, place(label)]);
                        }
                    }
                    let count = (constraints.len() / 2) as u32 - start;
                    // One item for the pass: `LANES` threads a group of the batch's nodes, which find
                    // its roots from the rule.
                    let (rule, any) = self.root_table.at(roots)?;
                    if any {
                        let group = *pattern / patterns::GROUP;
                        matches[group].extend([*pattern as u32, row, rule, start, count, pattern_threads[group]]);
                        pattern_threads[group] += self.root_table.groups() * patterns::LANES as u32;
                    }
                    Op {
                        target: place(target),
                        row,
                        ..Op::default()
                    }
                }
                Pass::Fact { target, block, found, labels } => {
                    // One item a file: a thread a row, or a word of the bits. Its constraints are the file, whether
                    // its findings are bits, then each slot's label; its root the findings' u32 in the block.
                    // The block bound whole, as a no-copy wrap of its written extent (made once, and kept by it).
                    let fact_block = std::sync::Arc::as_ptr(block) as usize;
                    if self.facts.as_ref().is_none_or(|(at, _)| *at != fact_block) {
                        self.facts = Some((fact_block, block.gpu_buffer(self.device)));
                    }
                    for (file, found) in found {
                        let (at, threads, bits) = match *found {
                            crate::code_rule_dsl::matcher::FactFound::Rows { at, n, .. } => (at, n, 0),
                            crate::code_rule_dsl::matcher::FactFound::Nodes { at, words } => (at, words, 1),
                        };
                        if threads == 0 {
                            continue;
                        }
                        let start = (constraints.len() / 2) as u32;
                        constraints.extend([*file, bits]);
                        if bits == 0 {
                            for label in labels {
                                constraints.extend([0, place(label)]);
                            }
                        }
                        let root = u32::try_from(at / 4).map_err(|_| capacity())?;
                        let slots = if bits == 0 { labels.len() as u32 } else { 0 };
                        matches[0].extend([patterns::FACT, row, root, start, slots, pattern_threads[0]]);
                        pattern_threads[0] += u32::try_from(threads).map_err(|_| capacity())?;
                    }
                    Op {
                        target: place(target),
                        row,
                        ..Op::default()
                    }
                }
            });
        }
        // A level past the recursive groups: its walks start within their anchors, and its formula passes are
        // evaluated (decision 0365).
        if round.is_none() && (!programs.is_empty() || passes.iter().any(|pass| matches!(pass, Pass::Formula { .. }))) {
            self.anchors(passes)?;
        }
        if !programs.is_empty() {
            self.programs(passes.as_ptr() as usize)?;
            self.walk(passes.as_ptr() as usize)?;
        }
        let labels = self.tapes.supertypes.buffer.clone();
        let (state, flags) = (self.tapes.supertypes.buffer.clone(), self.flags.buffer.clone());
        let (scratch, counts) = ((&state, self.tapes.scratch_at), (&state, self.tapes.counts_at));
        // The level's constraints, once for its pattern kernels (each group's binds them): the patterns' (variable,
        // label) pairs and each fact item's file and slots' labels. copy: not block data (the level's task
        // description). The facts' findings are read in their block (`facts`, Q317).
        let constraints = if matches.iter().any(|matches| !matches.is_empty()) { Some(self.upload(&constraints)?) } else { None };
        for (group, (matches, pattern_threads)) in matches.iter().zip(pattern_threads).enumerate() {
            if matches.is_empty() {
                continue;
            }
            let task_count = (matches.len() / 6) as u32;
            // copy: not block data (the group's task list).
            let (items, constraints) = (self.upload(matches)?, constraints.clone().ok_or(Error::BackendFailure)?);
            let docs = self.tapes.docs.buffer.clone();
            let root_words = self.root_words.as_ref().ok_or(Error::BackendFailure)?.buffer.clone();
            let facts = self.facts.as_ref().map_or(&self.flags.buffer, |(_, buffer)| buffer).clone();
            let shape = Shape {
                item_count: pattern_threads,
                task_count,
                ..self.shape()
            };
            self.pattern_kernel(
                group,
                &[
                    (&docs, 0),
                    (&labels, 8),
                    scratch,
                    (&flags, 0),
                    (&items, 0),
                    (&constraints, 0),
                    (&root_words, 0),
                    (&facts, 0),
                ],
                shape,
            )?;
        }
        let target_count = settles.len();
        let settles = self.upload(&settles)?;
        let shape = Shape {
            target_count: target_count as u32,
            round: round.map_or(0, |(round, _)| round as u32),
            recursive: u32::from(round.is_some()),
            last_level: u32::from(round.is_some_and(|(_, last)| last)),
            ..self.shape()
        };
        self.kernel(
            1,
            KERNELS[1],
            &[(&labels, 8), scratch, (&flags, 0), (&settles, 0), counts],
            shape,
            target_count * self.words,
        );
        if self.segment_bytes > SEGMENT_BYTES {
            self.outcome.waits[2] += 1;
            self.flush()?;
        }
        Ok(())
    }

    /// The batch's scheduler image, with a fresh generation, copied into the workspace.
    fn stage(&mut self, key: usize) -> Result<(), Error> {
        let Some(Some(prepared)) = self.prepared.get(&key) else {
            return Ok(());
        };
        let workspace = self.workspace.as_mut().ok_or(Error::BackendFailure)?;
        let lease = &mut workspace.lease;
        let generation = lease.generation;
        let span = u32::try_from(prepared.task_count + 1).map_err(|_| capacity())? * 2;
        lease.generation = lease.generation.checked_add(span).ok_or_else(capacity)?;
        let stamp = |evaluations: &[MetalEvaluation]| {
            let mut evaluations = evaluations.to_vec();
            evaluations.iter_mut().for_each(|evaluation| evaluation.generation = generation);
            evaluations
        };
        let mut image = bytes_of(std::slice::from_ref(&prepared.arenas)).to_vec();
        image.extend_from_slice(bytes_of(&stamp(&prepared.evaluations)));
        image.extend_from_slice(bytes_of(&stamp(&prepared.lanes)));
        let image_length = image.len() as u64;
        crate::code_rule_dsl::prof::count("stage.image_kb", image.len() >> 10);
        let image = self.upload(&image)?;
        let lease = &self.workspace.as_ref().expect("a workspace").lease;
        let blit = self.command.new_blit_command_encoder();
        blit.set_label("Rerail label pass scheduler image");
        blit.copy_from_buffer(&image, 0, &lease.scheduler.buffer, 0, image_length);
        blit.end_encoding();
        Ok(())
    }

    /// The interpreter's buffers for a prepared batch, in `run_batch`'s order: over resident
    /// tapes, the document table at `files` and the chunk block at `nodes`, which the kernel reads
    /// (`RERAIL_RESIDENT`; `children`, `source` and `lineStarts` bind it too, unread).
    fn bound(&self, prepared: &Prepared) -> Vec<Buffer> {
        let lease = &self.workspace.as_ref().expect("a workspace").lease;
        let tapes = &self.tapes;
        let block = self.block();
        let program_buffers = &prepared.program;
        // The block at `nodes`, where the kernel reads it; the other three are unread there.
        let (files, nodes, children, source, lines) = (&tapes.docs.buffer, block, block, block, block);
        [
            &prepared.tasks.buffer,
            &prepared.header.buffer,
            files,
            nodes,
            &program_buffers.programs.buffer,
            &program_buffers.words.buffer,
            &program_buffers.longs.buffer,
            children,
            &tapes.supertypes.buffer,
            &lease.pending.buffer,
            &lease.captures.buffer,
            &lease.cursors.buffer,
            &lease.repeats.buffer,
            &lease.scheduler.buffer,
            &lease.results.buffer,
            &lease.statuses.buffer,
            &program_buffers.values.buffer,
            &program_buffers.strings.buffer,
            &program_buffers.nfa_states.buffer,
            &program_buffers.nfa_data.buffer,
            &program_buffers.nfa_ranges.buffer,
            &program_buffers.lua_instructions.buffer,
            &program_buffers.lua_members.buffer,
            &program_buffers.assertions.buffer,
            &program_buffers.assertion_data.buffer,
            &program_buffers.string_bytes.buffer,
            source,
            &lease.value_states.buffer,
            lines,
            &lease.final_captures.buffer,
            &lease.output_storage.buffer,
        ]
        .into_iter()
        .cloned()
        .collect()
    }

    /// One sweep of the group at step `group` (`label_sweep`), its changed flag cleared first.
    fn sweep(&mut self, key: usize, group: usize, round: usize, reverse: bool) -> Result<(), Error> {
        let Some(Some(prepared)) = self.prepared.get(&key) else {
            return Ok(());
        };
        let lanes = prepared.lanes.len() as u64;
        self.stage(key)?;
        let prepared = self.prepared.get(&key).and_then(Option::as_ref).expect("prepared");
        let buffers = self.bound(prepared);
        // The round, and its changed flag cleared, in the prefix after the labels.
        let at = 4 * (2 + self.plan_labels * self.words) as u64;
        let word = u32::try_from(round).ok().filter(|round| *round < 1 << 16).ok_or(Error::BackendCapacity)?
            | u32::try_from(group).ok().filter(|group| *group < 1 << 15).ok_or(Error::BackendCapacity)? << 16
            | u32::from(reverse) << 31;
        let round_word = self.upload(&[word])?;
        let head = self.tapes.supertypes.buffer.clone();
        let blit = self.command.new_blit_command_encoder();
        blit.set_label("Rerail label sweep round");
        blit.copy_from_buffer(&round_word, 0, &head, at, 4);
        blit.fill_buffer(&head, metal::NSRange::new(at + 4 * (2 + 2 * group as u64 + round as u64 % 2), 4), 0);
        blit.end_encoding();
        let state = self.sweep.clone().ok_or(Error::BackendFailure)?;
        let encoder = encoder_of(&self.command, &mut self.encoder_times, "label_sweep");
        encoder.set_label("label_sweep");
        crate::code_rule_dsl::prof::count(concat!("kernel.", "label_sweep"), 1);
        encoder.set_compute_pipeline_state(&state);
        for (index, buffer) in buffers.iter().enumerate() {
            encoder.set_buffer(index as u64, Some(buffer), 0);
        }
        let width = state.thread_execution_width().max(1);
        encoder.checked_dispatch_threads(&self.command, MTLSize::new(lanes, 1, 1), MTLSize::new(width.min(lanes), 1, 1));
        encoder.end_encoding();
        self.encoded = true;
        self.outcome.dispatches += 1;
        self.report(key)
    }

    /// The batch's evaluations as its dispatch left them, copied out of the scheduler's image (the next level's
    /// replaces it): each records whether a task of it stopped (`Evaluation.outcome`, decision 0334).
    fn report(&mut self, key: usize) -> Result<(), Error> {
        let Some(Some(prepared)) = self.prepared.get(&key) else {
            return Ok(());
        };
        let count = prepared.evaluations.len();
        if count == 0 {
            return Ok(());
        }
        let copy = MappedBuffer::untouched::<MetalEvaluation>(self.device, count)?;
        let scheduler = self.workspace.as_ref().ok_or(Error::BackendFailure)?.lease.scheduler.buffer.clone();
        let blit = self.command.new_blit_command_encoder();
        blit.set_label("Rerail label pass evaluations");
        // The image is the arenas, then the evaluations (`stage`).
        blit.copy_from_buffer(
            &scheduler,
            std::mem::size_of::<super::super::metal_batch_abi::MetalGlobalArenas>() as u64,
            &copy.buffer,
            0,
            (count * std::mem::size_of::<MetalEvaluation>()) as u64,
        );
        blit.end_encoding();
        self.reports.push((copy, count, key));
        Ok(())
    }

    /// The (label, file) a task left unfinished, from the dispatches' evaluations (`report`), once the run is flushed:
    /// whether one ran over its budget, whether one stopped at the ceiling, and its pass's predicted steps per node
    /// and digest.
    fn unfinished(&self) -> BTreeMap<(u32, usize), (bool, bool, u64, String)> {
        let mut found = BTreeMap::new();
        for (copy, count, key) in &self.reports {
            let Some(Some(prepared)) = self.prepared.get(key) else { continue };
            // SAFETY: `count` evaluations were copied in by the flushed command buffer (`report`).
            let evaluations = unsafe { std::slice::from_raw_parts(copy.buffer.contents().cast::<MetalEvaluation>(), *count) };
            for evaluation in evaluations.iter().filter(|evaluation| evaluation.outcome & 3 != 0) {
                let (target, per_node, digest) = prepared.passes.get(evaluation.program as usize).cloned().unwrap_or((u32::MAX, 0, String::new()));
                let entry = found.entry((target, evaluation.file as usize)).or_insert((false, false, per_node, digest));
                entry.0 |= evaluation.outcome & 1 != 0;
                entry.1 |= evaluation.outcome & 2 != 0;
            }
        }
        found
    }

    /// A recursive group's rounds to its fixpoint as one dispatch (`label_group_*`, decision 0024
    /// §3), where every pass is a micro walk (a loop walk with the loop tier's frames, the deep
    /// pool's once a run outgrew them), a complement, a count or empty:
    /// each level's walks, words, starts and text rows concatenated, its settles after them.
    /// `false` where the group is not such (it runs as rounds of levels).
    fn group(&mut self, levels: &[Vec<Pass<'_>>]) -> Result<bool, Error> {
        let unfused = |why: &str| {
            crate::code_rule_dsl::prof::count_named(format!("gpu.unfused.{why}"), 1);
            if std::env::var_os("RERAIL_LAUNCHES").is_some() {
                eprintln!("UNFUSED {why}: {} levels", levels.len());
            }
            Ok(false)
        };
        if !self.micro {
            return unfused("interpreted");
        }
        let loop_frames = if self.deep { GROUP_FRAMES[4] } else { GROUP_FRAMES[2] };
        let place = |rows: &Option<Vec<u32>>, label: &u32| rows.as_ref().map_or(*label, |rows| rows[*label as usize]);
        let (mut records, mut words, mut texts) = (Vec::<[u32; 8]>::new(), Vec::<u32>::new(), Vec::<u32>::new());
        let (mut shape_levels, mut ops, mut steps, mut deepest) = (Vec::<[u32; 4]>::new(), Vec::<Op>::new(), 0_u32, 0_usize);
        let read = |buffer: &MappedBuffer| unsafe {
            std::slice::from_raw_parts(buffer.buffer.contents().cast::<u32>(), buffer.buffer.length() as usize / 4).to_vec()
        };
        for level in levels {
            let key = level.as_ptr() as usize;
            if self.prepared.get(&key).is_some_and(Option::is_some) {
                return unfused("tasks");
            }
            if level.iter().any(kernel_pass) {
                return unfused("patterns");
            }
            let walked = programs_of(level, true).len();
            let walks = self.walks.get(&key);
            if walks.map_or(0, |walks| walks.walked) != walked {
                return unfused("unwalked");
            }
            let (pass_start, op_start) = (records.len() as u32, ops.len() as u32);
            if let Some(walks) = walks {
                // A walk's frames: its static depth, at most the run's tier (64, 256 on the deep run), which a
                // loop walk takes. A static depth bounds the grammar's nesting, not a file's: a walk that does
                // outgrow its frames flags it (`MICRO_FAIL`) and the group runs again deep, then interpreted.
                let frames = |depth: &Option<usize>| depth.unwrap_or(loop_frames).min(loop_frames);
                deepest = deepest.max(walks.fused.iter().map(|(_, depth)| frames(depth)).max().unwrap_or(0));
                steps = steps.max(walks.steps);
                let word_at = words.len() as u32;
                let text_at = (texts.len() / self.words) as u32;
                records.extend(walks.fused.iter().map(|(record, _)| {
                    let mut record = *record;
                    record[0] += word_at;
                    record[6] += text_at;
                    record
                }));
                words.extend(read(&walks.words));
                // Whole text rows only: a level without any holds one placeholder word.
                let own = read(&walks.texts);
                texts.extend_from_slice(&own[..own.len() / self.words * self.words]);
            }
            let tallies = count_rows(level);
            let rows = self.label_rows.clone();
            ops.extend(
                level
                    .iter()
                    .enumerate()
                    .map(|(row, pass)| settle(pass, row as u32, tallies[row], |label| place(&rows, label)).expect("no pattern")),
            );
            shape_levels.push([pass_start, records.len() as u32 - pass_start, op_start, ops.len() as u32 - op_start]);
        }
        crate::code_rule_dsl::prof::count("gpu.fused_groups", 1);
        let (passes, words) = (self.upload(&records)?, self.upload(&words)?);
        let items = self.root_words.as_ref().ok_or(Error::BackendFailure)?.buffer.clone();
        let texts = if texts.is_empty() { self.upload(&[0_u32])? } else { self.upload(&texts)? };
        let (group_levels, ops) = (self.upload(&shape_levels)?, self.upload(&ops)?);
        let mut shape = Shape { target_count: shape_levels.len() as u32, steps, ..self.shape() };
        let tapes = &self.tapes;
        let buffers = [
            tapes.docs.buffer.clone(),
            tapes.supertypes.buffer.clone(),
            passes,
            words,
            items,
            tapes.supertypes.buffer.clone(),
            self.flags.buffer.clone(),
            texts,
            tapes.supertypes.buffer.clone(),
            group_levels,
            ops,
        ];
        // The fewest frames that hold its deepest walk; the kernel's stacks in device memory, one per
        // thread launched.
        let depth = GROUP_FRAMES.iter().position(|&frames| deepest <= frames).expect("checked above");
        let (pipeline, label) = (GROUP_KERNEL, GROUP_NAMES[depth]);
        shape.frames = GROUP_FRAMES[depth] as u32;
        let mut buffers = buffers.to_vec();
        if self.group_stacks[depth].is_none() {
            let words = GROUP_STACK_WORDS / 32 * GROUP_FRAMES[depth];
            self.group_stacks[depth] = Some(MappedBuffer::untouched::<u32>(self.device, self.files.len().max(1) * GROUP_WIDTH as usize * words)?);
        }
        buffers.push(self.group_stacks[depth].as_ref().expect("group stacks").buffer.clone());
        let LabelKernel { state, chunk, .. } = self.kernels[pipeline].clone();
        let encoder = encoder_of(&self.command, &mut self.encoder_times, label);
        encoder.set_label(label);
        crate::code_rule_dsl::prof::count_named(format!("kernel.{label}"), 1);
        encoder.set_compute_pipeline_state(&state);
        // Scratch (5) and the tallies (8) are parts of the label state.
        let (scratch_at, counts_at) = (self.tapes.scratch_at, self.tapes.counts_at);
        for (index, buffer) in buffers.iter().take(11).enumerate() {
            let offset = match index {
                5 => scratch_at,
                8 => counts_at,
                _ => 0,
            };
            encoder.set_buffer(index as u64, Some(buffer), offset);
        }
        encoder.set_bytes(11, std::mem::size_of::<Shape>() as u64, std::ptr::from_ref(&shape).cast());
        if let Some(stacks) = buffers.get(11) {
            encoder.set_buffer(12, Some(stacks), 0);
        }
        self.bind_slabs(encoder, chunk);
        encoder.checked_dispatch_thread_groups(&self.command, MTLSize::new(self.files.len().max(1) as u64, 1, 1), MTLSize::new(GROUP_WIDTH, 1, 1));
        encoder.end_encoding();
        self.encoded = true;
        self.outcome.dispatches += 1;
        self.split(label);
        self.split(label);
        Ok(true)
    }

    /// The root walks (`root_records_*`): a thread per (program, file, task, start) of each
    /// program that is a micro walk of at most 32 frames, by its depth's kernel.
    fn records(&mut self, jobs: &RootJobs<'_>) -> Result<(), Error> {
        let bases = self
            .files
            .iter()
            .scan(0_u32, |base, tape| {
                let at = *base;
                *base += tape.transducer_len() as u32;
                Some(at)
            })
            .collect::<Vec<_>>();
        let (largest, mut longest) = (self.files.iter().map(TapeDoc::transducer_len).max().unwrap_or(0), 0);
        let (mut passes, mut words, mut texts, mut walked) = (Vec::<[u32; 5]>::new(), Vec::<u32>::new(), Vec::<u32>::new(), Vec::new());
        // Decision 0365: each walk's anchor, as a row of `texts` past the text rows (`label_phase` sets it), the
        // walk's starts outside it not walked. A pack's is its members' union.
        // A pass's fifth word: its anchor's index here until the text rows are known, then its row.
        let mut anchors = Vec::<std::sync::Arc<DeviceFormula>>::new();
        let on = crate::code_rule_dsl::transducer::packed::compile_matcher::anchors_on();
        let mut anchor_of = |formula: Option<std::sync::Arc<DeviceFormula>>| -> u32 {
            let Some(formula) = formula.filter(|_| on) else { return u32::MAX };
            anchors.push(formula);
            (anchors.len() - 1) as u32
        };
        // Each walk's text slots' rows, by key, for the anchors' text tests.
        let mut slot_rows = HashMap::<String, u32>::new();
        let mut tiers: [Vec<[u32; 4]>; 2] = Default::default();
        // Root programs of one entry test as one walk (t1 spec-fusion §4.3, §4.4; the host's
        // `PackedDemand::packs`): a pack's passes follow the programs', its items are the union of
        // its members' starts, and `decode_roots` routes each record to its member.
        let walkable = |program: &Program| {
            let walk = Micro::of(program).filter(|walk| walk.depth().is_some_and(|depth| depth <= 32))?;
            let rows = (0..walk.texts.len()).map(|slot| text_row(&walk, program, slot, self.files, &bases, self.words)).collect::<Option<Vec<_>>>()?;
            Some((walk, rows))
        };
        // A pack: its walk, text rows and their keys (`Micro::slot_key`), and set mode.
        let mut packs = Vec::<(RootsPack, Micro, Vec<std::sync::Arc<Vec<u32>>>, Vec<String>, bool)>::new();
        let mut packed = vec![false; jobs.programs.len()];
        {
            let mut by = std::collections::BTreeMap::<u32, Vec<usize>>::new();
            for (at, program) in jobs.programs.iter().enumerate() {
                if let Some(key) = super::fusion::entry_key(program) {
                    by.entry(key).or_default().push(at);
                }
            }
            for members in by.into_values() {
                for chunk in members.chunks(super::fusion::MEMBERS).filter(|chunk| chunk.len() > 1) {
                    let programs = chunk.iter().map(|&at| jobs.programs[at]).collect::<Vec<_>>();
                    let Some(fused) = super::fusion::Fused::of(&programs) else { continue };
                    let Some((walk, rows)) = walkable(&fused.program) else { continue };
                    // In its members' deepest tier, as the launch plan counts them.
                    let root_tier = |walk: &Micro| walk.depth().is_some_and(|depth| depth > 8);
                    if root_tier(&walk) && !programs.iter().filter_map(|program| Micro::of(program)).any(|walk| root_tier(&walk)) {
                        continue;
                    }
                    let set = fused.program.set_mode();
                    for &at in chunk {
                        packed[at] = true;
                    }
                    let tasks = chunk
                        .iter()
                        .map(|&at| jobs.starts[at].iter().map(|starts| starts.iter().enumerate().map(|(task, &start)| (start, task as u32)).collect()).collect())
                        .collect();
                    crate::code_rule_dsl::prof::count("gpu.root_packed", chunk.len());
                    let keys = (0..walk.texts.len()).map(|slot| walk.slot_key(&fused.program, slot)).collect();
                    packs.push((RootsPack { members: chunk.to_vec(), emits: fused.emits, captures: fused.captures, tasks }, walk, rows, keys, set));
                }
            }
        }
        for (at, program) in jobs.programs.iter().enumerate() {
            if packed[at] {
                // Walked in its pack.
                passes.push([0; 5]);
                walked.push(true);
                continue;
            }
            let walk = Micro::of(program).filter(|walk| walk.depth().is_some_and(|depth| depth <= 32));
            let rows = walk.as_ref().and_then(|walk| {
                (0..walk.texts.len()).map(|slot| text_row(walk, program, slot, self.files, &bases, self.words)).collect::<Option<Vec<_>>>()
            });
            let (Some(walk), Some(rows)) = (walk, rows) else {
                passes.push([0; 5]);
                walked.push(false);
                continue;
            };
            let text_start = (texts.len() / self.words) as u32;
            rows.iter().for_each(|row| texts.extend_from_slice(row));
            for slot in 0..walk.texts.len() {
                slot_rows.insert(walk.slot_key(program, slot), text_start + slot as u32);
            }
            // The fourth word: set mode with one label, so the walk ends at its first emit.
            let anchor = anchor_of(device_anchor(program, None, &|_| false));
            passes.push([words.len() as u32, walk.words.len() as u32, text_start, u32::from(walk.key_stop()), anchor]);
            longest = longest.max(walk.words.len());
            match &self.label_rows {
                None => words.extend_from_slice(&walk.words),
                Some(row_of) => words.extend(walk.words.iter().map(|&word| {
                    let (op, mode, arg) = super::super::split(word);
                    if op == super::super::SUPERTYPE && arg >= DERIVED_CLASS_BASE {
                        super::super::word(op, mode, DERIVED_CLASS_BASE + row_of[usize::from(arg - DERIVED_CLASS_BASE)] as u16)
                    } else {
                        word
                    }
                })),
            }
            walked.push(true);
            let tier = usize::from(walk.depth().is_some_and(|depth| depth > 8));
            for (file, starts) in jobs.starts[at].iter().enumerate() {
                tiers[tier].extend(starts.iter().enumerate().map(|(task, &start)| [at as u32, file as u32, task as u32, bases[file] + start]));
            }
        }
        for (index, (pack, walk, rows, keys, set)) in packs.iter().enumerate() {
            let pass = (jobs.programs.len() + index) as u32;
            let text_start = (texts.len() / self.words) as u32;
            rows.iter().for_each(|row| texts.extend_from_slice(row));
            for (slot, key) in keys.iter().enumerate() {
                slot_rows.entry(key.clone()).or_insert(text_start + slot as u32);
            }
            let pad = if *set { 1 + pack.emits.len() as u32 } else { 0 };
            // The union of its members' anchors: none where one has none.
            let members = pack.members.iter().map(|&at| jobs.programs[at]).collect::<Vec<_>>();
            let anchor = anchor_of(pack_anchor(&members));
            passes.push([words.len() as u32, walk.words.len() as u32, text_start, pad, anchor]);
            longest = longest.max(walk.words.len());
            match &self.label_rows {
                None => words.extend_from_slice(&walk.words),
                Some(row_of) => words.extend(walk.words.iter().map(|&word| {
                    let (op, mode, arg) = super::super::split(word);
                    if op == super::super::SUPERTYPE && arg >= DERIVED_CLASS_BASE {
                        super::super::word(op, mode, DERIVED_CLASS_BASE + row_of[usize::from(arg - DERIVED_CLASS_BASE)] as u16)
                    } else {
                        word
                    }
                })),
            }
            let tier = usize::from(walk.depth().is_some_and(|depth| depth > 8));
            for file in 0..self.files.len() {
                let mut starts = pack.tasks.iter().flat_map(|tasks: &Vec<std::collections::HashMap<u32, u32>>| tasks[file].keys().copied()).collect::<Vec<_>>();
                starts.sort_unstable();
                starts.dedup();
                tiers[tier].extend(starts.iter().map(|&start| [pass, file as u32, u32::MAX, bases[file] + start]));
            }
        }
        drop(anchor_of);
        let packs = packs.into_iter().map(|(pack, ..)| pack).collect::<Vec<_>>();
        // The anchors' rows past the text rows, zero (two words more, which a file's last word may reach).
        let first = (texts.len() / self.words) as u32;
        for pass in &mut passes {
            if pass[4] != u32::MAX && pass[1] != 0 {
                pass[4] += first;
            } else {
                pass[4] = u32::MAX;
            }
        }
        if !anchors.is_empty() {
            texts.resize(texts.len() + anchors.len() * self.words + 2, 0);
        }
        let items = tiers.iter().flatten().copied().collect::<Vec<_>>();
        crate::code_rule_dsl::prof::count("gpu.root_items", items.len());
        // Plan-sized (list step 13): a record per 16 roots, where a root's matches are a few per
        // thousand (S7: 5,023 of 1,150,210); 4 a root reserved 351 MB. A batch that writes more
        // overflows, and its roots run on the CPU (`decode_roots`).
        let capacity = items.len() / 16 + 4096;
        let records = MappedBuffer::untouched::<u32>(self.device, capacity * ROOT_RECORD)?;
        let cursor = MappedBuffer::zeroed::<u32>(self.device, 2)?;
        let failures = MappedBuffer::zeroed::<u32>(self.device, (jobs.programs.len() + packs.len()).max(1))?;
        let steps = super::super::micro_steps(largest, longest.max(1));
        let (passes, words) = (self.upload(&passes)?, self.upload(&words)?);
        let texts = if texts.is_empty() { self.upload(&[0_u32])? } else { self.upload(&texts)? };
        if !anchors.is_empty() {
            crate::code_rule_dsl::prof::count("gpu.root_anchors", anchors.len());
            let list = anchors
                .iter()
                .enumerate()
                .map(|(k, formula)| FormulaJob {
                    formula: std::sync::Arc::clone(formula),
                    row: (first as usize + k) as u32 * self.words as u32,
                    mode: 0,
                    rule: u32::MAX,
                    texts: formula.texts.iter().map(|key| slot_rows.get(key).copied()).collect(),
                })
                .collect::<Vec<_>>();
            self.formulas(&list, &texts, Some(&texts))?;
        }
        let mut offset = 0_u64;
        for (tier, tier_items) in tiers.iter().enumerate() {
            if tier_items.is_empty() {
                continue;
            }
            let items_buffer = self.upload(tier_items)?;
            let shape = Shape {
                item_count: tier_items.len() as u32,
                target_count: capacity as u32,
                round: offset as u32,
                steps,
                frames: ROOT_FRAMES[tier.min(1)],
                ..self.shape()
            };
            let tapes = &self.tapes;
            let buffers = [
                (&tapes.docs.buffer.clone(), 0),
                (&tapes.supertypes.buffer.clone(), 0),
                (&passes, 0),
                (&words, 0),
                (&items_buffer, 0),
                (&texts, 0),
                (&records.buffer.clone(), 0),
                (&cursor.buffer.clone(), 0),
                (&failures.buffer.clone(), 0),
            ];
            {
                // The pool's device stacks follow the shape (buffer 10): 8 frames a lane over the
                // pooled stacks, or `ROOT_POOL` lanes of 32.
                let stacks = self.stacks()?;
                let threads = if tier == 0 { tier_items.len().min(stack_lanes(8, 31)) } else { tier_items.len().min(ROOT_POOL) };
                let (kernel, label) = (ROOT_KERNEL, ROOT_NAMES[tier.min(1)]);
                let LabelKernel { state, chunk, .. } = self.kernels[kernel].clone();
                let encoder = encoder_of(&self.command, &mut self.encoder_times, label);
                encoder.set_label(label);
                encoder.set_compute_pipeline_state(&state);
                for (index, (buffer, offset)) in buffers.iter().enumerate() {
                    encoder.set_buffer(index as u64, Some(buffer), *offset);
                }
                encoder.set_bytes(9, std::mem::size_of::<Shape>() as u64, std::ptr::from_ref(&shape).cast());
                encoder.set_buffer(10, Some(&stacks), 0);
                self.bind_slabs(encoder, chunk);
                let width = state.thread_execution_width().max(1);
                encoder.checked_dispatch_threads(&self.command, MTLSize::new(threads as u64, 1, 1), MTLSize::new(width.min(threads as u64), 1, 1));
                encoder.end_encoding();
                self.encoded = true;
                self.outcome.dispatches += 1;
                self.split(label);
            }
            offset += tier_items.len() as u64;
        }
        self.roots = Some(RootsEncoded { walked, packs, items, records, cursor, failures, capacity });
        Ok(())
    }

    /// The level's micro walks (`label_micro`), one thread per (pass, start), marking scratch as
    /// `label_sink` marks the tasks' results.
    fn walk(&mut self, level: usize) -> Result<(), Error> {
        let Some(walks) = self.walks.get(&level) else { return Ok(()) };
        let words = walks.words.buffer.clone();
        let items = self.root_words.as_ref().ok_or(Error::BackendFailure)?.buffer.clone();
        let texts = walks.texts.buffer.clone();
        let steps = walks.steps;
        let tiers = walks.tiers.iter().map(|(tier, passes, count, threads)| (*tier, passes.buffer.clone(), *count, *threads)).collect::<Vec<_>>();
        let stacks = self.stacks()?;
        for (tier, passes, count, threads) in tiers {
        let mut shape = Shape { item_count: threads as u32, target_count: count as u32, steps, ..self.shape() };
        if spill_on() {
            if self.spill.is_none() {
                self.spill = Some([
                    MappedBuffer::zeroed::<u32>(self.device, SPILL_FRAMES)?,
                    MappedBuffer::zeroed::<u32>(self.device, 8)?,
                    MappedBuffer::zeroed::<u32>(self.device, SPILL_FRAMES * SPILL_ITEM)?,
                ]);
            }
            // A fresh queue each dispatch: its slots empty, its head, tail, active, overflow and
            // frames taken zero.
            let [slots, control, _] = self.spill.as_ref().expect("the spill queue");
            let blit = self.command.new_blit_command_encoder();
            blit.set_label("Rerail micro spill queue");
            blit.fill_buffer(&slots.buffer, metal::NSRange::new(0, 4 * SPILL_FRAMES as u64), 0);
            blit.fill_buffer(&control.buffer, metal::NSRange::new(0, 32), 0);
            blit.end_encoding();
            shape.queue = SPILL_FRAMES as u32;
            shape.budget = spill_budget();
        }
        if walk_steps_on() {
            // Each pass's record names it by its words.
            let records = unsafe { std::slice::from_raw_parts(passes.contents().cast::<[u32; 8]>(), count) };
            let all = unsafe { std::slice::from_raw_parts(words.contents().cast::<u32>(), words.length() as usize / 4) };
            let names = records.iter().map(|r| describe_walk(tier, &all[r[0] as usize..(r[0] + r[1]) as usize])).collect();
            let stats = MappedBuffer::zeroed::<u32>(self.device, 3 * count.max(1))?;
            self.walk_bind = Some(stats.buffer.clone());
            self.walk_stats.push((stats, names));
            shape.profile = 1;
        }
        let tapes = &self.tapes;
        let (docs, head) = (tapes.docs.buffer.clone(), tapes.supertypes.buffer.clone());
        let (state, flags) = (self.tapes.supertypes.buffer.clone(), self.flags.buffer.clone());
        let buffers = [
            (&docs, 0),
            (&head, 0),
            (&passes, 0),
            (&words, 0),
            (&items, 0),
            (&state, self.tapes.scratch_at),
            (&flags, 0),
            (&texts, 0),
            (&state, self.tapes.counts_at),
            (&stacks, 0),
        ];
        if crate::code_rule_dsl::prof::on() {
            crate::code_rule_dsl::prof::count_named(format!("items.tier{tier}"), threads);
        }
        let lanes = TIER_LANES[tier];
        shape.lanes = lanes as u32;
        let (label, frames, threads) = match tier {
            2 if self.deep => ("label_micro_deep", MICRO_DEEP_FRAMES, (threads * lanes).min(MICRO_DEEP_POOL)),
            2 => (TIER_NAMES[2], TIER_FRAMES[2], (threads * lanes).min(MICRO_POOL)),
            _ => (TIER_NAMES[tier], TIER_FRAMES[tier], (threads * lanes).min(stack_lanes(TIER_FRAMES[tier], 17))),
        };
        shape.frames = frames as u32;
        self.kernel(MICRO_KERNEL, label, &buffers, shape, threads);
        self.walk_bind = None;
        }
        Ok(())
    }

    /// The level's prepared batch, then `label_sink` routing each result by its task's program.
    fn programs(&mut self, level: usize) -> Result<(), Error> {
        let Some(Some(prepared)) = self.prepared.get(&level) else {
            return Ok(());
        };
        let (lanes_count, capacity) = (prepared.lanes.len(), prepared.result_capacity);
        self.stage(level)?;
        let prepared = self.prepared.get(&level).and_then(Option::as_ref).expect("prepared");
        let routes = prepared.routes.buffer.clone();
        let buffers = self.bound(prepared);
        let encoder = encoder_of(&self.command, &mut self.encoder_times, "Rerail label pass programs");
        encoder.set_label("Rerail label pass programs");
        crate::code_rule_dsl::prof::count(concat!("kernel.", "Rerail label pass programs"), 1);
        encoder.set_compute_pipeline_state(self.engine.pipeline());
        for (index, buffer) in buffers.iter().enumerate() {
            encoder.set_buffer(index as u64, Some(buffer), 0);
        }
        let lanes = lanes_count as u64;
        let width = self.engine.pipeline().thread_execution_width().max(1);
        encoder.checked_dispatch_threads(&self.command, MTLSize::new(lanes, 1, 1), MTLSize::new(width.min(lanes), 1, 1));
        encoder.end_encoding();
        self.encoded = true;
        self.outcome.dispatches += 1;
        self.report(level)?;
        let lease = &self.workspace.as_ref().expect("a workspace").lease;
        let (scheduler, results, captures, statuses) = (
            lease.scheduler.buffer.clone(),
            lease.results.buffer.clone(),
            lease.final_captures.buffer.clone(),
            lease.statuses.buffer.clone(),
        );
        self.split("programs");
        let (state, flags) = (self.tapes.supertypes.buffer.clone(), self.flags.buffer.clone());
        let shape = Shape {
            result_arena: ArenaKind::Result as u32,
            task_count: lanes_count as u32,
            ..self.shape()
        };
        self.kernel(
            0,
            KERNELS[0],
            &[
                (&scheduler, 0),
                (&results, 0),
                (&captures, 0),
                (&statuses, 0),
                (&routes, 0),
                (&state, self.tapes.scratch_at),
                (&state, self.tapes.counts_at),
                (&flags, 0),
            ],
            shape,
            capacity.max(lanes_count),
        );
        Ok(())
    }
}

/// The share of the free buffer budget a level's lanes may take: the rest is the run's other
/// buffers made after sizing (program buffers, walk and root records).
const LANE_SHARE: (usize, usize) = (3, 4);

/// The most lanes, up to `wanted`, whose workspace fits `LANE_SHARE` of the buffer room left (`buffer_room`),
/// each lane with its states (decision 0033, O38): lanes are sized from the space, not states
/// halved until a fixed lane count fits. `BackendBudget` where not even one lane fits.
fn fit_lanes(batch: &mut PackedBatch, programs: &[Program], specs: &[[u32; 5]], wanted: usize) -> Result<usize, Error> {
    let mut bytes = |lanes: usize| -> Result<usize, Error> {
        let totals = batch.assign_resident(programs, specs, lanes)?;
        Ok(MetalWorkspace::bytes_for(totals, batch.header.scheduler_byte_count, batch.header.workspace_byte_count, batch.evaluation_lanes.len().max(1)))
    };
    let (one, two) = (bytes(1)?, bytes(2)?);
    // Of the room a reservation can have now, not of the budget less the ledger alone (Q203).
    lanes_in(one, two, metal_backend::buffer_room(), wanted)
}

/// The lanes, up to `wanted`, whose workspace fits `LANE_SHARE` of `room`, a workspace being `one` bytes at one lane
/// and `two` at two (a fixed part and a part per lane): a level larger than the room runs in fewer lanes, its tasks
/// in turn, rather than off the device. `BackendBudget` where not even one lane fits.
fn lanes_in(one: usize, two: usize, room: usize, wanted: usize) -> Result<usize, Error> {
    let per_lane = two.saturating_sub(one).max(1);
    let fixed = one.saturating_sub(per_lane);
    let share = room / LANE_SHARE.1 * LANE_SHARE.0;
    if fixed + per_lane > share {
        return Err(Error::BackendBudget);
    }
    Ok(((share - fixed) / per_lane).clamp(1, wanted.max(1)))
}

#[cfg(test)]
mod lanes_tests {
    use super::*;

    /// A level whose lanes outgrow the room runs in as many as fit (its tasks in turn), down to one; only a fixed part
    /// past the room is refused.
    #[test]
    fn a_level_larger_than_the_room_runs_in_the_lanes_that_fit() {
        // 10 MB fixed, 1 MB a lane: 4/3 of 40 MB is 30 MB of room, 20 lanes.
        let (one, two) = (11 << 20, 12 << 20);
        assert_eq!(lanes_in(one, two, 40 << 20, 1024).unwrap(), 20);
        assert_eq!(lanes_in(one, two, 40 << 20, 8).unwrap(), 8, "no more than wanted");
        // Room for the fixed part and one lane: one lane.
        assert_eq!(lanes_in(one, two, (11 << 20) * 4 / 3 + 4, 1024).unwrap(), 1);
        assert_eq!(lanes_in(one, two, 12 << 20, 1024), Err(Error::BackendBudget), "not even one lane");
    }
}

/// A level's programs: (program, roots per file, sink mode, scratch label or count slot).
/// A level's programs: (program, roots, mode, slot), the slot a program's scratch row
/// (`scratch_rows`) where `rows`, else its label (a sweep's, `label_sweep`), or a count's slot.
fn programs_of<'p>(passes: &'p [Pass<'_>], rows: bool) -> Vec<(&'p Program, &'p std::sync::Arc<Roots>, u32, u32)> {
    let tallies = count_rows(passes);
    passes
        .iter()
        .enumerate()
        .filter_map(|(row, pass)| match pass {
            Pass::Program {
                target,
                program,
                roots,
                captured,
            } => Some((*program, roots, if *captured { 1 } else { 0 }, if rows { row as u32 } else { *target })),
            Pass::Count {
                slot,
                program,
                roots,
                captured,
                ..
            } => Some((*program, roots, if *captured { 3 } else { 2 }, if rows { tallies[row] } else { *slot })),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod phase_program_tests {
    /// 0461, 0463: a push writes into a slot cleared before phase 0, so it starts in phase 1 and is read in the
    /// next; a pull reads a set written in an earlier phase; `CHILD_IN` takes its candidates when read; a sibling
    /// step is a push into a node array and a pull from it; descendants are a span and a cover a phase apart; a
    /// chain's reach is two scans a level; an ancestor step is a climb, a chain's through a set whole before it.
    #[test]
    fn a_formula_is_compiled_to_phases_of_word_and_push_steps() {
        use super::*;
        use crate::code_rule_dsl::transducer::packed::compile_sets::device_ops as op;
        let word = |kind: u32, arg: u32| arg << 8 | kind;
        let end = [PHASE_END, 0, 0, 0];
        let (a, b) = (word(op::CLASS, 16384), word(op::CLASS, 16385));
        // CHILD(a): the parents of a's named members.
        assert_eq!(
            phase_program(&[a, word(op::CHILD, 1)], 1),
            Some(PhaseProgram {
                code: vec![[PHASE_ZERO, 1, 0, 0], [PHASE_ROW, 0, a, 0], end, [PHASE_UP, 1, 0, 1], end],
                phases: 3,
                result: 1,
                slots: 2,
            })
        );
        // OR(PARENT(a), b): the pull reads `a` from the phase before.
        let program = phase_program(&[a, word(op::PARENT, 0), b, word(op::OR, 2)], 1).expect("phases hold it");
        assert_eq!(program.code, vec![[PHASE_ROW, 0, a, 0], end, [PHASE_DOWN, 1, 0, 0], [PHASE_ROW, 2, b, 0], [PHASE_OR, 1, 2, 0]]);
        assert_eq!(program.phases, 2);
        // CHILD_IN(b, a): a's parents, cut to b when read.
        let program = phase_program(&[b, a, word(op::CHILD_IN, 0)], 1).expect("phases hold it");
        assert_eq!(program.code.last(), Some(&[PHASE_AND, program.result, 0, 0]));
        // FOLLOWING(a): the last member child of each parent (a node array, 32 slots), then the nodes before it.
        let program = phase_program(&[a, word(op::FOLLOWING, 1)], 1).expect("phases hold it");
        assert_eq!(
            program.code,
            vec![[PHASE_ZERO32, 1, 0, 0], [PHASE_ROW, 0, a, 0], end, [PHASE_LAST, 1, 0, 1], end, [PHASE_BEFORE, 33, 1, 0]]
        );
        assert_eq!((program.result, program.slots, program.phases), (33, 34, 3));
        // REACH_DESCENDANT(a): the members' ends and the blocks', then the cover.
        let program = phase_program(&[a, word(op::REACH_DESCENDANT, 0)], 1).expect("phases hold it");
        assert_eq!(program.code, vec![[PHASE_ROW, 0, a, 0], [PHASE_SPAN, 1, 0, 2], end, [PHASE_COVER, 3, 0, 1 | 2 << 16]]);
        // REACH_CHAIN(a, b): two scans a level, and a check that no member is left.
        for levels in [1, 2, 4] {
            let program = phase_program(&[a, b, word(op::REACH_CHAIN, 0)], levels).expect("phases hold it");
            assert_eq!(program.phases as usize, 2 * levels + 1);
            assert_eq!(program.code.last().map(|c| c[0]), Some(PHASE_CHECK));
        }
        // DESCENDANT(a): a climb from a's named members.
        let program = phase_program(&[a, word(op::DESCENDANT, 1)], 1).expect("phases hold it");
        assert_eq!(program.code, vec![[PHASE_ZERO, 1, 0, 0], [PHASE_ROW, 0, a, 0], end, [PHASE_CLIMB, 1, 0, 1], end]);
        // CHAIN(b, a): a climb from a's members through b's, b whole before it.
        let program = phase_program(&[b, a, word(op::CHAIN, 0)], 1).expect("phases hold it");
        assert_eq!(program.code, vec![[PHASE_ZERO, 2, 0, 0], [PHASE_ROW, 0, b, 0], [PHASE_ROW, 1, a, 0], end, [PHASE_CLIMB, 2, 1, 2], end]);
        // A leaf test reaches the device as its row (`LEAF`); its own op is no step.
        assert_eq!(phase_program(&[word(op::KIND, 3)], 1), None);
    }
}
