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
mod patterns;

use super::super::PackedPattern;
use super::super::metal_batch_abi::MetalSupertype;
use super::super::native_ir::ArenaKind;
use super::*;
use super::super::metal_batch_abi::MetalEvaluation;
use metal::{Buffer, CommandBuffer};
use std::collections::HashMap;
use crate::code_rule_dsl::residency::slabs::ResidentParts as _;
use crate::code_rule_dsl::tape::DERIVED_CLASS_BASE;

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
    /// The global roots a semantic predicate's producer reported (O24, decision 0030) whose slots'
    /// targets each have their label: per root, its (target node, label) pairs. The pattern kernel's
    /// fact case (`patterns::FACT`) tests them, so a fact settles from scratch as a pattern does.
    Fact { target: u32, rows: Vec<(u32, Vec<(u32, u32)>)> },
    Empty { target: u32 },
}

/// A pass's roots, as a rule over each document's codes (decision 0033): no node list is built.
/// Every root test a pass's domain makes (its kinds, its program's leading kind and supertype
/// tests, `matcher::Entry`) is a property of a node's code, so a pass's roots in a document are
/// the nodes of the codes its rule admits, which the residency indexes (`DocView::by_code`). The
/// run hands the device each pass's (document, code) runs (`RootSeg`); the device reads its root
/// in place. Listed roots (a clause's prefilter candidates, few and scattered) are a list.
#[derive(Debug)]
pub(in crate::code_rule_dsl) enum Roots {
    /// The nodes whose kind is one of `kinds` (any where `None`) and whose code passes `entry`
    /// (`(KIND, kind)` or `(SUPERTYPE, class)`), in the files `active` marks.
    Codes { kinds: Option<Vec<u16>>, entry: Vec<(u8, u16)>, active: Vec<bool> },
    /// Per file, its listed nodes (document-local, ascending).
    Listed(Vec<Vec<u32>>),
}

impl Roots {
    fn admits(kinds: &Option<Vec<u16>>, entry: &[(u8, u16)], code: &crate::code_rule_dsl::tape::resident::DeviceCode) -> bool {
        let class = |class: u16| code.classes[..usize::from(code.class_count)].contains(&class);
        kinds.as_ref().is_none_or(|kinds| kinds.binary_search(&code.kind).is_ok())
            && entry.iter().all(|&(op, arg)| if op == super::super::KIND { code.kind == arg } else { class(arg) })
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
            Self::Listed(lists) => Box::new(lists.get(file).into_iter().flatten().copied()),
            Self::Codes { kinds, entry, active } => {
                if !active.get(file).copied().unwrap_or(false) {
                    return Box::new(std::iter::empty());
                }
                Box::new((0..tape.transducer_len() as u32).filter(move |&node| {
                    kinds.as_ref().is_none_or(|kinds| kinds.binary_search(&tape.transducer_kind(node)).is_ok())
                        && entry.iter().all(|&(op, arg)| if op == super::super::KIND { tape.transducer_kind(node) == arg } else { tape.transducer_has_supertype(node, arg) })
                }))
            }
        }
    }

    /// This rule's segments over a batch's documents, appended to `segs` (in u32s, as `RootSeg`s
    /// at 16-byte records): the header's index (in records) and the roots' count. Listed nodes go
    /// after every segment, in `listed`, which the caller appends and offsets (`Segments`).
    fn segments(&self, documents: &[std::sync::Arc<crate::code_rule_dsl::residency::slabs::Resident>], segs: &mut Segments) -> (u32, u32) {
        let header = segs.records.len();
        segs.records.push([0; 4]);
        let mut total = 0_u32;
        for (doc, resident) in documents.iter().enumerate() {
            match self {
                Self::Listed(lists) => {
                    let Some(list) = lists.get(doc).filter(|list| !list.is_empty()) else { continue };
                    segs.records.push([doc as u32, ROOT_LISTED | segs.listed.len() as u32, list.len() as u32, total]);
                    segs.listed.extend_from_slice(list);
                    total += list.len() as u32;
                }
                Self::Codes { kinds, entry, active } => {
                    if !active.get(doc).copied().unwrap_or(false) {
                        continue;
                    }
                    let starts = resident.code_starts();
                    for (code, device) in resident.codes().iter().enumerate() {
                        let count = starts[code + 1] - starts[code];
                        if count > 0 && Self::admits(kinds, entry, device) {
                            segs.records.push([doc as u32, starts[code], count, total]);
                            total += count;
                        }
                    }
                }
            }
        }
        segs.records[header] = [(segs.records.len() - header - 1) as u32, total, 0, 0];
        (header as u32, total)
    }
}

/// Bit 31 of a segment's start: its nodes are listed (`Roots::Listed`), not a code's.
const ROOT_LISTED: u32 = 1 << 31;

/// A run's root segments (`Roots::segments`): 16-byte records, then the listed nodes. One per
/// distinct rule (by its `Arc`), so passes of one domain share them.
#[derive(Default)]
struct Segments {
    records: Vec<[u32; 4]>,
    listed: Vec<u32>,
    by_rule: HashMap<usize, (u32, u32)>,
}

impl Segments {
    fn of(&mut self, roots: &std::sync::Arc<Roots>, documents: &[std::sync::Arc<crate::code_rule_dsl::residency::slabs::Resident>]) -> (u32, u32) {
        if let Some(found) = self.by_rule.get(&(std::sync::Arc::as_ptr(roots) as usize)) {
            return *found;
        }
        let found = roots.segments(documents, self);
        self.by_rule.insert(std::sync::Arc::as_ptr(roots) as usize, found);
        found
    }

    /// A registered rule's header and, per document with roots, its first root's index in the
    /// rule and how many.
    fn by_doc(&self, roots: &std::sync::Arc<Roots>) -> Result<(u32, HashMap<u32, (u32, u32)>), Error> {
        let &(header, _) = self.by_rule.get(&(std::sync::Arc::as_ptr(roots) as usize)).ok_or(Error::BackendFailure)?;
        let count = self.records[header as usize][0] as usize;
        let mut docs = HashMap::<u32, (u32, u32)>::new();
        for record in &self.records[header as usize + 1..header as usize + 1 + count] {
            docs.entry(record[0]).and_modify(|(_, total)| *total += record[2]).or_insert((record[3], record[2]));
        }
        Ok((header, docs))
    }

    /// As the device reads them: listed starts offset past the records.
    fn words(&self) -> Vec<u32> {
        let offset = 4 * self.records.len() as u32;
        let mut words = Vec::with_capacity(4 * self.records.len() + self.listed.len());
        for record in &self.records {
            let from = if record[1] & ROOT_LISTED != 0 && record[2] > 0 { ROOT_LISTED | ((record[1] & !ROOT_LISTED) + offset) } else { record[1] };
            words.extend([record[0], from, record[2], record[3]]);
        }
        words.extend_from_slice(&self.listed);
        if words.is_empty() {
            words.push(0);
        }
        words
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
}

/// Root programs and, per program, per file, their starts (an anchor a task).
pub(in crate::code_rule_dsl) struct RootJobs<'a> {
    pub(in crate::code_rule_dsl) programs: Vec<&'a Program>,
    pub(in crate::code_rule_dsl) starts: Vec<Vec<Vec<u32>>>,
}

/// Per root program: per file its (task, record)s in task then walk order, or `None` where the
/// program did not run on the device (not a micro walk of at most 32 frames, a walk that
/// overflowed, or records past the buffer); the host walks those.
pub(in crate::code_rule_dsl) type RootsFound = Vec<Option<Vec<Vec<(u32, super::super::RootRecord)>>>>;

/// The root walks' kernels in `KERNELS`: 8 and 32 frames.
const ROOT_KERNELS: [usize; 2] = [9, 10];
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
    let per_file = files.par_iter().map(|tape| walk.slot(program, slot, tape)).collect::<Option<Vec<_>>>()?;
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
    let per_pair = (0..slots.len() * files.len())
        .into_par_iter()
        .map(|pair| {
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
    static HOST: LazyLock<bool> = LazyLock::new(|| std::env::var_os("RERAIL_HOST_SWEEPS").is_some());
    *HOST
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

#[derive(Debug, Default)]
pub(in crate::code_rule_dsl) struct Outcome {
    /// Every label's words after the passes.
    pub(in crate::code_rule_dsl) labels: Vec<u32>,
    pub(in crate::code_rule_dsl) words: usize,
    pub(in crate::code_rule_dsl) command_buffers: usize,
    pub(in crate::code_rule_dsl) dispatches: usize,
    /// Why the host waited before the end: workspace growth, a recursive group that needed more
    /// rounds than were encoded (the whole clause again), the upload budget.
    pub(in crate::code_rule_dsl) waits: [usize; 3],
    /// Rounds encoded per recursive group.
    pub(in crate::code_rule_dsl) rounds: usize,
    /// Whether a task outgrew `NARROW`'s states, so the batch ran again with `WIDE`'s.
    pub(in crate::code_rule_dsl) widened: bool,
    /// Whether a micro walk outgrew its frames, so every program ran on the interpreter.
    pub(in crate::code_rule_dsl) interpreted: bool,
    /// The plan's root records (`Plan::roots`); empty without roots.
    pub(in crate::code_rule_dsl) records: RootsFound,
}

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
            Pass::Program { target, program, .. } | Pass::Count { target, program, .. } => {
                mark(*target)?;
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
            Pass::Fact { target, rows } => {
                mark(*target)?;
                for (_, label) in rows.iter().flat_map(|(_, targets)| targets) {
                    mark(*label)?;
                }
            }
            Pass::Empty { target } => mark(*target)?,
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
                Pass::Program { target, .. } | Pass::Empty { target } => Op {
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

/// The device's tape view (decision 0032), which the label and pattern kernels read.
const TAPE_VIEW: &str = concat!("#include <metal_stdlib>\nusing namespace metal;\n", include_str!("metal_tape.metal"));
/// The label kernels over the tape view.
const SOURCE: &str = concat!("#include <metal_stdlib>\nusing namespace metal;\n", include_str!("metal_tape.metal"), include_str!("metal_label_passes.metal"));
/// Where a label kernel binds the batch's four slabs (`Slabs`): past every other buffer.
const SLAB_SLOT: u64 = 27;
const KERNELS: [&str; 11] = [
    "label_sink",
    "label_settle",
    "label_round",
    "label_micro",
    "label_micro_8",
    "label_micro_32",
    "label_micro_deep",
    "label_group_8",
    "label_group_32",
    "root_records_8",
    "root_records_32",
];
/// The fused group kernels in `KERNELS` (`Runner::group`): 8 and 32 frames a walk.
const GROUP_KERNELS: [usize; 2] = [7, 8];
/// A group dispatch's threads per file.
const GROUP_WIDTH: u64 = 64;

/// A walk's stack tier (`label_micro_8`, `label_micro_32`, `label_micro`): the fewest frames its
/// static depth (`Micro::depth`) fits, 128 where it loops.
fn tier(depth: Option<usize>) -> usize {
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

/// Each tier's kernel in `KERNELS`.
const TIER_KERNELS: [usize; 3] = [4, 5, 3];

/// The loop tier's threads (`MICRO_POOL`), each striding over the items with a 64-frame stack
/// of 17 words a frame (`MicroFrame`) in `Runner::stacks`. The pool's threads are the kernel's
/// occupancy: at 1024 threads of 128 frames the loop tier's 3% of S7's starts took ~280 ms of
/// its ~350 ms on the GPU, 2048 of 64 ~150 ms. Timed per kernel (`RERAIL_GPU_KERNEL_TIMES`),
/// 1.73 M loop items took 193 ms at 2048 threads, 121 ms at 4096 and 85 ms at 8192, which put
/// the footprint over budget (511–515 MB). A run whose walk outgrows 64 frames runs again with
/// the same stacks as 2048 of 128 (`label_micro_deep`, `MICRO_DEEP_POOL`).
const MICRO_POOL: usize = 4096;
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
/// The deep pool's kernel in `KERNELS`.
const DEEP_KERNEL: usize = 6;
/// The launches an evaluation's encoding stays within (`metal_launch_budget`, list step 13): a
/// recursive group's rounds are encoded from what the plan's fixed launches leave, and a plan
/// that cannot fit (its fixed launches and two rounds a group, or a rerun) is refused before it
/// encodes, as `BackendLaunchLimit`.
const LAUNCHES: usize = super::super::metal_backend::launch::KERNEL_LAUNCH_LIMIT;
/// At most this many rounds of a recursive group are encoded in one go.
const MOST_ROUNDS: usize = 64;
/// Bytes of per-level uploads one command buffer carries before the host waits for it.
const SEGMENT_BYTES: usize = 64 << 20;

/// Compiles the wave's pipelines (the set steps, the interpreter, the sweep and the label kernels) on a
/// thread, once a process: each is compiled from source, 0.6 s together, and a pass that needs
/// one before it is ready waits for the rest of its compile (they are cached behind locks).
pub(in crate::code_rule_dsl) fn warm() {
    static STARTED: std::sync::Once = std::sync::Once::new();
    STARTED.call_once(|| {
        std::thread::spawn(|| {
            // The set program's kernels first: it runs first in a wave (its candidates). The shared
            // device is not opened here: on S7 a wave waiting on this thread's open waited longer
            // (to 0.8 s under load) than opening it itself (50-80 ms).
            let _ = super::set_steps::pipelines();
            let _ = compile_resident_interpreter_cached();
            let _ = pipelines();
            let _ = compile_sweep_cached();
        });
    });
}

/// The label kernels, compiled once (until the residency's drop).
static PIPELINES: Mutex<Option<Vec<ComputePipelineState>>> = Mutex::new(None);
/// The clauses' pattern kernels, by source (until the residency's drop).
static PATTERN_PIPELINES: LazyLock<Mutex<HashMap<String, ComputePipelineState>>> = LazyLock::new(Default::default);

/// Releases the compiled label and pattern kernels (the residency's drop).
pub(in crate::code_rule_dsl) fn drop_pipelines() {
    if let Ok(mut pipelines) = PIPELINES.lock() {
        *pipelines = None;
    }
    if let Ok(mut pipelines) = PATTERN_PIPELINES.lock() {
        pipelines.clear();
    }
}

fn pipelines() -> Result<Vec<ComputePipelineState>, Error> {
    let mut cached = PIPELINES.lock().map_err(|_| Error::BackendFailure)?;
    if let Some(pipelines) = &*cached {
        return Ok(pipelines.clone());
    }
    let pipelines = KERNELS
        .iter()
        .map(|name| pipeline_named(SOURCE, name))
        .collect::<Result<Vec<_>, _>>()?;
    *cached = Some(pipelines.clone());
    Ok(pipelines)
}

/// The plan's pattern kernels, one per `patterns::GROUP` of its patterns (a fact-only plan's
/// one), with every pattern's metavariable names in the plan's numbering.
fn pattern_pipelines(patterns: &[&PackedPattern], facts: bool) -> Result<Option<(Vec<ComputePipelineState>, Vec<Vec<String>>)>, Error> {
    if patterns.is_empty() && !facts {
        return Ok(None);
    }
    let (mut pipelines, mut names) = (Vec::new(), Vec::new());
    let groups = patterns.chunks(patterns::GROUP).collect::<Vec<_>>();
    for (group, members) in if groups.is_empty() { vec![&[][..]] } else { groups }.into_iter().enumerate() {
        let (pipeline, group_names) = pattern_pipeline(members, group * patterns::GROUP)?;
        pipelines.push(pipeline);
        names.extend(group_names);
    }
    Ok(Some((pipelines, names)))
}

/// One group's pattern kernel, compiled once per distinct source.
fn pattern_pipeline(patterns: &[&PackedPattern], first: usize) -> Result<(ComputePipelineState, Vec<Vec<String>>), Error> {
    let (source, names) = patterns::source(patterns, first);
    // `RERAIL_DIAG_PATTERNS=<dir>`: each pattern kernel's source, for measuring it alone.
    if let Some(dir) = std::env::var_os("RERAIL_DIAG_PATTERNS") {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        source.hash(&mut hasher);
        let _ = std::fs::write(std::path::Path::new(&dir).join(format!("{:016x}.metal", hasher.finish())), &source);
    }
    let mut cached = PATTERN_PIPELINES.lock().map_err(|_| Error::BackendFailure)?;
    if let Some(pipeline) = cached.get(&source) {
        return Ok((pipeline.clone(), names));
    }
    crate::code_rule_dsl::prof::count("gpu.pattern_pipelines", 1);
    let pipeline = pipeline_named(&source, patterns::KERNEL)?;
    cached.insert(source, pipeline.clone());
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
    kernels: Vec<ComputePipelineState>,
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
    /// Whether a level's micro walks run as such, and each level's, by its address.
    micro: bool,
    /// Whether the loop tier runs with 128-frame stacks (`label_micro_deep`).
    deep: bool,
    walks: HashMap<usize, Walks>,
    /// Every pass's roots as segments (`Roots::segments`), one buffer for the run (`segs`).
    segments: Segments,
    segs: Option<MappedBuffer>,
    /// The loop tier's stacks (`MICRO_POOL` of them), committed only as walks use them.
    stacks: Option<MappedBuffer>,
    /// The deep fused groups' stacks, one buffer for all of them: the groups run one after
    /// another in the command buffer (five of 33 MB each on S7 before, list step 13).
    group_stacks: [Option<MappedBuffer>; 2],
    encoded: bool,
    outcome: Outcome,
    /// The plan's root walks, encoded after its passes (`records`).
    roots: Option<RootsEncoded>,
}

#[cfg(test)]
thread_local! {
    /// Tests: the passes run on the interpreter, as a run whose walks outgrew their frames does.
    pub(in crate::code_rule_dsl) static INTERPRET_ONLY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn interpret_only() -> bool {
    #[cfg(test)]
    return INTERPRET_ONLY.get();
    #[cfg(not(test))]
    false
}

pub(in crate::code_rule_dsl) fn run(plan: &Plan<'_>, files: &[TapeDoc]) -> Result<Outcome, Error> {
    // The run's command buffers and encoders are autoreleased, and they retain its buffers: drained
    // here, its buffers go with it rather than living until the thread's pool drains (06 S7: the
    // 9 runs of a wave held ~1 GB at once).
    metal::objc::rc::autoreleasepool(|| run_in_pool(plan, files))
}

fn run_in_pool(plan: &Plan<'_>, files: &[TapeDoc]) -> Result<Outcome, Error> {
    let _evaluation = super::super::metal_backend::metal_execution_guard()?;
    let device = METAL_DEVICE.as_ref().ok_or(Error::UnsupportedBackend)?;
    // A level's launches: the programs' dispatch and sink, the derived labels, the settle.
    let launches = |level: &Vec<Pass<'_>>| {
        let programs = level.iter().any(|pass| matches!(pass, Pass::Program { .. } | Pass::Count { .. }));
        let patterns = level.iter().any(kernel_pass);
        2 * usize::from(programs) + usize::from(patterns) + 1
    };
    let fixed = plan
        .steps
        .iter()
        .filter(|(recursive, _)| !recursive)
        .flat_map(|(_, levels)| levels)
        .map(launches)
        .sum::<usize>();
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
                    for node in 0..tape.transducer_len() as u32 {
                        let above = tape.transducer_parent(node).map_or(0, |parent| nested[parent as usize]);
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
    let mut rounds = plan_rounds(0, &wanted)?;
    let nodes = files.iter().map(TapeDoc::transducer_len).sum::<usize>();
    let (mut retries, mut used, mut buffers) = (0, 0, 0);
    let (mut lanes, mut micro, mut deep, mut released) = (NARROW, !interpret_only(), false, false);
    loop {
        let mut outcome = match encode(device, plan, files, &rounds, lanes, micro, deep) {
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
            Err(Error::BackendBudget) => return Err(Error::BackendCapacity),
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
        static SEEN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let wanted = std::env::var("RERAIL_HOLD_AT").ok();
        let nth = wanted.as_deref().and_then(|wanted| wanted.split_once('#')).map_or(Some(1), |(_, n)| n.parse().ok());
        let here = wanted.as_deref().is_some_and(|wanted| wanted.split('#').next() == Some(at))
            && Some(SEEN.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1) == nth;
        if let Some(path) = std::env::var_os("RERAIL_HOLD").filter(|_| here) {
            eprintln!("FLUSH holding, pid {}", std::process::id());
            while !std::path::Path::new(&path).exists() {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
    }
}

fn encode(
    device: &Device,
    plan: &Plan<'_>,
    files: &[TapeDoc],
    rounds: &[usize],
    lanes: (usize, Option<u32>),
    micro: bool,
    deep: bool,
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
    let queue = device.new_command_queue();
    queue.set_label("Rerail label passes");
    let command = queue.new_command_buffer().to_owned();
    command.set_label("Rerail label passes");
    let mut runner = Runner {
        device,
        engine: compile_resident_interpreter_cached()?,
        kernels: pipelines()?,
        sweep: if plan.steps.iter().any(|(recursive, levels)| *recursive && sweeps(levels)) {
            Some(compile_sweep_cached()?)
        } else {
            None
        },
        orders: HashMap::new(),
        patterns: pattern_pipelines(&plan.patterns, plan.steps.iter().flat_map(|(_, levels)| levels).flatten().any(|pass| matches!(pass, Pass::Fact { .. })))?,
        command,
        queue,
        files,
        tapes: crate::code_rule_dsl::prof::time("gpu.tapes", || tapes(device, files, plan, words, rows.as_ref(), [scratch_rows(plan) * words, tally_rows(plan) * nodes]))?,
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
        micro,
        deep,
        walks: HashMap::new(),
        segments: Segments::default(),
        segs: None,
        stacks: kept.as_mut().and_then(|kept| kept.stacks.as_ref().map(|stacks| MappedBuffer::wrap(stacks.buffer.clone()))),
        group_stacks: [None, None],
        encoded: false,
        outcome: Outcome {
            words,
            rounds: rounds.iter().copied().max().unwrap_or(0),
            ..Outcome::default()
        },
        roots: None,
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
    // A micro walk that outgrew its frames: the passes again on the interpreter.
    if runner.flag(0) & 2 != 0 {
        crate::code_rule_dsl::prof::count("gpu.frames_overflow", 1);
        return Err(Error::WalkFrames);
    }
    if runner.flag(0) != 0 {
        return Err(Error::BackendFailure);
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
    // The documents in the slabs, one table over them: a batch over `MAX_SLABS` is the wave's to
    // split.
    let table = crate::code_rule_dsl::prof::time("residency.docs", || crate::code_rule_dsl::residency::slabs::admit_docs(files))?;
    let table = crate::code_rule_dsl::residency::slabs::DocTable::new(table).ok_or(Error::BackendCapacity)?;
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
    /// The batch's slabs at `SLAB_SLOT..` (`Slabs`), the last repeated where it has fewer.
    /// The pooled stacks every walk tier and root walk runs on (`STACK_WORDS`), made at first use
    /// and kept for the next run (`Kept`). Kernels that share them run in order (tracked).
    fn stacks(&mut self) -> Result<Buffer, Error> {
        if self.stacks.is_none() {
            self.stacks = Some(MappedBuffer::untouched::<u32>(self.device, STACK_WORDS)?);
        }
        Ok(self.stacks.as_ref().expect("stacks").buffer.clone())
    }

    fn bind_slabs(&self, encoder: &metal::ComputeCommandEncoderRef) {
        let slabs = &self.tapes.table.slabs;
        for slot in 0..crate::code_rule_dsl::residency::slabs::MAX_SLABS {
            let buffer = slabs.get(slot).or(slabs.last()).map_or(&self.flags.buffer, |slab| &slab.buffer.buffer);
            encoder.set_buffer(SLAB_SLOT + slot as u64, Some(buffer), 0);
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
        if crate::code_rule_dsl::prof::on() {
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
        }
        self.outcome.command_buffers += 1;
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
    fn split(&mut self, label: &str) {
        static ON: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| std::env::var_os("RERAIL_GPU_KERNEL_TIMES").is_some());
        if !*ON {
            return;
        }
        // Refused (the launch ledger is spent): the flush reports it.
        if commit_checked(&self.command).is_err() || wait_checked(&self.command).is_err() {
            return;
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
        self.command = self.queue.new_command_buffer().to_owned();
        self.command.set_label("Rerail label passes");
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
    fn kernel(&mut self, pipeline: usize, buffers: &[(&Buffer, u64)], shape: Shape, threads: usize) {
        let state = self.kernels[pipeline].clone();
        self.dispatch(&state, KERNELS[pipeline], buffers, shape, threads);
    }

    /// The clause's pattern kernel over the level's (pattern, root) items.
    fn pattern_kernel(&mut self, group: usize, buffers: &[(&Buffer, u64)], shape: Shape) -> Result<(), Error> {
        let state = self.patterns.as_ref().and_then(|(pipelines, _)| pipelines.get(group)).ok_or(Error::BackendFailure)?.clone();
        self.dispatch(&state, patterns::KERNEL, buffers, shape, shape.item_count as usize);
        Ok(())
    }

    fn dispatch(&mut self, state: &ComputePipelineState, label: &str, buffers: &[(&Buffer, u64)], shape: Shape, threads: usize) {
        if crate::code_rule_dsl::prof::on() {
            crate::code_rule_dsl::prof::count_named(format!("threads.{label}"), threads);
        }
        if threads == 0 {
            return;
        }
        crate::code_rule_dsl::prof::count("gpu.encoders", 1);
        let encoder = self.command.new_compute_command_encoder();
        encoder.set_label(label);
        encoder.set_compute_pipeline_state(state);
        for (index, (buffer, offset)) in buffers.iter().enumerate() {
            encoder.set_buffer(index as u64, Some(buffer), *offset);
        }
        self.bind_slabs(encoder);
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
        self.kernel(2, &[(&flags, 0), (&head, 0)], shape, 1);
    }

    /// Every level's batch, built once, and one workspace for all of them, sized before anything
    /// is encoded so the host never waits to grow it.
    fn size(&mut self, plan: &Plan<'_>) -> Result<(), Error> {
        let mut widest_seen: Option<(WorkspaceTotals, u32, u32, usize)> = None;
        // Every pass's roots as segments, in one buffer for the run, before anything binds it.
        let documents = self.tapes.table.documents.clone();
        for pass in plan.steps.iter().flat_map(|(_, levels)| levels).flatten() {
            if let Pass::Program { roots, .. } | Pass::Count { roots, .. } | Pass::Pattern { roots, .. } = pass {
                self.segments.of(roots, &documents);
            }
        }
        let segs = self.segments.words();
        crate::code_rule_dsl::prof::count("size.segment_words", segs.len());
        self.segs = Some(MappedBuffer::from_slice(self.device, &segs)?);
        // A sweeping group is one batch of all its programs, by its address; a level is one by
        // its own.
        let batches = plan.steps.iter().flat_map(|(recursive, levels)| {
            if *recursive && sweeps(levels) {
                vec![(levels.as_ptr() as usize, levels.iter().flat_map(|level| programs_of(level, false)).collect::<Vec<_>>(), Some(levels))]
            } else {
                levels.iter().map(|level| (level.as_ptr() as usize, programs_of(level, true), None)).collect()
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
        for (key, mut passes, sweep) in batches {
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
                let documents = self.tapes.table.documents.clone();
                let segments = &mut self.segments;
                // Threads: a pass's from its own start, over its (shared) item range.
                let mut threads = [0_u32; 3];
                let mut fused = Vec::new();
                // Passes walked from one shared root list, marking roots or captures, as one
                // walk (multi-target packing, t1 spec-fusion §2.6, §4.3, §4.4; the host's
                // `matcher::Packs`): an emit marks its member's row (by emit label, `rows`).
                let packs = {
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
                            let deepest = programs.iter().filter_map(|program| Micro::of(program)).map(|walk| tier(walk.depth())).max();
                            if let Some(packed) = super::fusion::Fused::of(&programs)
                                && Micro::of(&packed.program).map(|walk| tier(walk.depth())) <= deepest
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
                    // The rule's segments, shared by passes of its domain.
                    let (start, count) = segments.of(roots, &documents);
                    if crate::code_rule_dsl::prof::on() {
                        let depth = walk.depth().map_or("loop".to_owned(), |depth| format!("{:02}", depth.min(64)));
                        if walk.depth().is_none() && std::env::var_os("RERAIL_DUMP_LOOP").is_some() {
                            eprintln!("LOOP {:?}", walk.ops());
                        }
                        crate::code_rule_dsl::prof::count_named(format!("depth.{depth}"), count as usize);
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
                                crate::code_rule_dsl::prof::count_named(format!("unslotted.{key}"), count as usize);
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
                    let tier = tier(walk.depth());
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
            // roots; its tasks are its roots, which the device takes from the rule's segments.
            let no_anchors = vec![vec![Vec::<u32>::new(); programs.len()]; self.files.len()];
            let mut batch = crate::code_rule_dsl::prof::time("size.batch", || PackedBatch::new_resident(&programs, self.files, &no_anchors))?;
            let mut specs = Vec::new();
            {
                let by_doc = passes
                    .iter()
                    .map(|(_, roots, ..)| self.segments.by_doc(roots))
                    .collect::<Result<Vec<_>, Error>>()?;
                for file in 0..self.files.len() {
                    for (program, (header, docs)) in by_doc.iter().enumerate() {
                        if let Some(&(base, count)) = docs.get(&(file as u32)) {
                            specs.push([program as u32, file as u32, *header, base, count]);
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
                    return Err(Error::BackendCapacity);
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
            batch.header.supertype_count = self.tapes.supertype_count;
            let (lanes, states) = if sweep.is_some() { WIDE } else { self.lanes };
            batch.state_cap = states;
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
            // In the tasks' place: a level's roots' segments (the run's), a sweep's table.
            let tasks = match &sweep_table {
                Some(table) => MappedBuffer::from_slice(self.device, table)?,
                None => MappedBuffer::wrap(self.segs.as_ref().ok_or(Error::BackendFailure)?.buffer.clone()),
            };
            let prepared = Prepared {
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
            let mut lease = crate::code_rule_dsl::prof::time("size.lease", || MetalWorkspaceLease::acquire(self.device, totals, scheduler_bytes, output_bytes, tasks))
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
                pass @ (Pass::Program { .. } | Pass::Empty { .. } | Pass::Count { .. } | Pass::Complement { .. }) => {
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
                    // One item for the pass: its threads find their roots in its segments.
                    let (segment, roots) = self.segments.by_rule.get(&(std::sync::Arc::as_ptr(roots) as usize)).copied().ok_or(Error::BackendFailure)?;
                    if roots > 0 {
                        let group = *pattern / patterns::GROUP;
                        matches[group].extend([*pattern as u32, row, segment, start, count, pattern_threads[group]]);
                        pattern_threads[group] += roots;
                    }
                    Op {
                        target: place(target),
                        row,
                        ..Op::default()
                    }
                }
                Pass::Fact { target, rows: reported } => {
                    for (root, targets) in reported {
                        let start = (constraints.len() / 2) as u32;
                        for (node, label) in targets {
                            constraints.extend([*node, place(label)]);
                        }
                        matches[0].extend([patterns::FACT, row, *root, start, targets.len() as u32, pattern_threads[0]]);
                        pattern_threads[0] += 1;
                    }
                    Op {
                        target: place(target),
                        row,
                        ..Op::default()
                    }
                }
            });
        }
        if !programs.is_empty() {
            self.programs(passes.as_ptr() as usize)?;
            self.walk(passes.as_ptr() as usize)?;
        }
        let labels = self.tapes.supertypes.buffer.clone();
        let (state, flags) = (self.tapes.supertypes.buffer.clone(), self.flags.buffer.clone());
        let (scratch, counts) = ((&state, self.tapes.scratch_at), (&state, self.tapes.counts_at));
        for (group, (matches, pattern_threads)) in matches.iter().zip(pattern_threads).enumerate() {
            if matches.is_empty() {
                continue;
            }
            let task_count = (matches.len() / 6) as u32;
            let (items, constraints) = (self.upload(matches)?, self.upload(&constraints)?);
            let docs = self.tapes.docs.buffer.clone();
            let segs = self.segs.as_ref().ok_or(Error::BackendFailure)?.buffer.clone();
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
                    (&segs, 0),
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
        let span = u32::try_from(prepared.task_count + 1).map_err(|_| Error::BackendCapacity)? * 2;
        lease.generation = lease.generation.checked_add(span).ok_or(Error::BackendCapacity)?;
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
    /// tapes, the document table at `files` and the slabs at `nodes`, `children`, `source` and
    /// `lineStarts` (`RERAIL_RESIDENT`).
    fn bound(&self, prepared: &Prepared) -> Vec<Buffer> {
        let lease = &self.workspace.as_ref().expect("a workspace").lease;
        let tapes = &self.tapes;
        let slabs = &tapes.table.slabs;
        let slab = |slot: usize| slabs.get(slot).or(slabs.last()).map_or(&self.flags.buffer, |slab| &slab.buffer.buffer);
        let program_buffers = &prepared.program;
        let (files, nodes, children, source, lines) = (&tapes.docs.buffer, slab(0), slab(1), slab(2), slab(3));
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
        let encoder = self.command.new_compute_command_encoder();
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
        Ok(())
    }

    /// A recursive group's rounds to its fixpoint as one dispatch (`label_group_*`, decision 0024
    /// §3), where every pass is a micro walk of at most 32 frames, a complement, a count or empty:
    /// each level's walks, words, starts and text rows concatenated, its settles after them.
    /// `false` where the group is not such (it runs as rounds of levels).
    fn group(&mut self, levels: &[Vec<Pass<'_>>]) -> Result<bool, Error> {
        let unfused = |why: &str| {
            crate::code_rule_dsl::prof::count_named(format!("gpu.unfused.{why}"), 1);
            Ok(false)
        };
        if !self.micro {
            return unfused("interpreted");
        }
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
                if walks.fused.iter().any(|(_, depth)| depth.is_none_or(|depth| depth > 32)) {
                    return unfused("deep");
                }
                deepest = deepest.max(walks.fused.iter().filter_map(|(_, depth)| *depth).max().unwrap_or(0));
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
        let items = self.segs.as_ref().ok_or(Error::BackendFailure)?.buffer.clone();
        let texts = if texts.is_empty() { self.upload(&[0_u32])? } else { self.upload(&texts)? };
        let (group_levels, ops) = (self.upload(&shape_levels)?, self.upload(&ops)?);
        let shape = Shape { target_count: shape_levels.len() as u32, steps, ..self.shape() };
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
        let pipeline = GROUP_KERNELS[usize::from(deepest > 8)];
        // The kernel's stacks in device memory, one per thread launched (8 or 32 frames).
        let mut buffers = buffers.to_vec();
        let depth = usize::from(deepest > 8);
        if self.group_stacks[depth].is_none() {
            let words = GROUP_STACK_WORDS / if depth == 0 { 4 } else { 1 };
            self.group_stacks[depth] = Some(MappedBuffer::untouched::<u32>(self.device, self.files.len().max(1) * GROUP_WIDTH as usize * words)?);
        }
        buffers.push(self.group_stacks[depth].as_ref().expect("group stacks").buffer.clone());
        let state = self.kernels[pipeline].clone();
        let encoder = self.command.new_compute_command_encoder();
        encoder.set_label(KERNELS[pipeline]);
        crate::code_rule_dsl::prof::count_named(format!("kernel.{}", KERNELS[pipeline]), 1);
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
        self.bind_slabs(encoder);
        encoder.checked_dispatch_thread_groups(&self.command, MTLSize::new(self.files.len().max(1) as u64, 1, 1), MTLSize::new(GROUP_WIDTH, 1, 1));
        encoder.end_encoding();
        self.encoded = true;
        self.outcome.dispatches += 1;
        self.split(KERNELS[pipeline]);
        self.split(KERNELS[pipeline]);
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
        let (mut passes, mut words, mut texts, mut walked) = (Vec::<[u32; 4]>::new(), Vec::<u32>::new(), Vec::<u32>::new(), Vec::new());
        let mut tiers: [Vec<[u32; 4]>; 2] = Default::default();
        // Root programs of one entry test as one walk (t1 spec-fusion §4.3, §4.4; the host's
        // `PackedDemand::packs`): a pack's passes follow the programs', its items are the union of
        // its members' starts, and `decode_roots` routes each record to its member.
        let walkable = |program: &Program| {
            let walk = Micro::of(program).filter(|walk| walk.depth().is_some_and(|depth| depth <= 32))?;
            let rows = (0..walk.texts.len()).map(|slot| text_row(&walk, program, slot, self.files, &bases, self.words)).collect::<Option<Vec<_>>>()?;
            Some((walk, rows))
        };
        let mut packs = Vec::<(RootsPack, Micro, Vec<std::sync::Arc<Vec<u32>>>, bool)>::new();
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
                    packs.push((RootsPack { members: chunk.to_vec(), emits: fused.emits, captures: fused.captures, tasks }, walk, rows, set));
                }
            }
        }
        for (at, program) in jobs.programs.iter().enumerate() {
            if packed[at] {
                // Walked in its pack.
                passes.push([0; 4]);
                walked.push(true);
                continue;
            }
            let walk = Micro::of(program).filter(|walk| walk.depth().is_some_and(|depth| depth <= 32));
            let rows = walk.as_ref().and_then(|walk| {
                (0..walk.texts.len()).map(|slot| text_row(walk, program, slot, self.files, &bases, self.words)).collect::<Option<Vec<_>>>()
            });
            let (Some(walk), Some(rows)) = (walk, rows) else {
                passes.push([0; 4]);
                walked.push(false);
                continue;
            };
            let text_start = (texts.len() / self.words) as u32;
            rows.iter().for_each(|row| texts.extend_from_slice(row));
            // The fourth word: set mode with one label, so the walk ends at its first emit.
            passes.push([words.len() as u32, walk.words.len() as u32, text_start, u32::from(walk.key_stop())]);
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
        for (index, (pack, walk, rows, set)) in packs.iter().enumerate() {
            let pass = (jobs.programs.len() + index) as u32;
            let text_start = (texts.len() / self.words) as u32;
            rows.iter().for_each(|row| texts.extend_from_slice(row));
            let pad = if *set { 1 + pack.emits.len() as u32 } else { 0 };
            passes.push([words.len() as u32, walk.words.len() as u32, text_start, pad]);
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
        let packs = packs.into_iter().map(|(pack, ..)| pack).collect::<Vec<_>>();
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
                let kernel = ROOT_KERNELS[tier.min(1)];
                let state = self.kernels[kernel].clone();
                let encoder = self.command.new_compute_command_encoder();
                encoder.set_label(KERNELS[kernel]);
                encoder.set_compute_pipeline_state(&state);
                for (index, (buffer, offset)) in buffers.iter().enumerate() {
                    encoder.set_buffer(index as u64, Some(buffer), *offset);
                }
                encoder.set_bytes(9, std::mem::size_of::<Shape>() as u64, std::ptr::from_ref(&shape).cast());
                encoder.set_buffer(10, Some(&stacks), 0);
                self.bind_slabs(encoder);
                let width = state.thread_execution_width().max(1);
                encoder.checked_dispatch_threads(&self.command, MTLSize::new(threads as u64, 1, 1), MTLSize::new(width.min(threads as u64), 1, 1));
                encoder.end_encoding();
                self.encoded = true;
                self.outcome.dispatches += 1;
                self.split(KERNELS[kernel]);
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
        let items = self.segs.as_ref().ok_or(Error::BackendFailure)?.buffer.clone();
        let texts = walks.texts.buffer.clone();
        let steps = walks.steps;
        let tiers = walks.tiers.iter().map(|(tier, passes, count, threads)| (*tier, passes.buffer.clone(), *count, *threads)).collect::<Vec<_>>();
        let stacks = self.stacks()?;
        for (tier, passes, count, threads) in tiers {
        let shape = Shape { item_count: threads as u32, target_count: count as u32, steps, ..self.shape() };
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
        let (pipeline, threads) = match tier {
            2 if self.deep => (DEEP_KERNEL, threads.min(MICRO_DEEP_POOL)),
            2 => (TIER_KERNELS[2], threads.min(MICRO_POOL)),
            _ => (TIER_KERNELS[tier], threads.min(stack_lanes(TIER_FRAMES[tier], 17))),
        };
        self.kernel(pipeline, &buffers, shape, threads);
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
        let lease = &self.workspace.as_ref().expect("a workspace").lease;
        let encoder = self.command.new_compute_command_encoder();
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

/// The most lanes, up to `wanted`, whose workspace fits `LANE_SHARE` of the free buffer budget,
/// each lane with its states (decision 0033, O38): lanes are sized from the space, not states
/// halved until a fixed lane count fits. `BackendBudget` where not even one lane fits.
fn fit_lanes(batch: &mut PackedBatch, programs: &[Program], specs: &[[u32; 5]], wanted: usize) -> Result<usize, Error> {
    let mut bytes = |lanes: usize| -> Result<usize, Error> {
        let totals = batch.assign_resident(programs, specs, lanes)?;
        Ok(MetalWorkspace::bytes_for(totals, batch.header.scheduler_byte_count, batch.header.workspace_byte_count, batch.evaluation_lanes.len().max(1)))
    };
    let (one, two) = (bytes(1)?, bytes(2)?);
    let per_lane = two.saturating_sub(one).max(1);
    let fixed = one.saturating_sub(per_lane);
    let share = metal_backend::buffer_free() / LANE_SHARE.1 * LANE_SHARE.0;
    if fixed + per_lane > share {
        return Err(Error::BackendBudget);
    }
    Ok(((share - fixed) / per_lane).clamp(1, wanted.max(1)))
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
