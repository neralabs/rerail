//! The scan engine: a plan over text with no vocabulary (a module, a function body, a test on
//! nodes of any kind), with the verdict wanted per node: does the node's own text match?
//!
//! A set of plans (`Batch`) reads a file once for all of them where it can, and never reads a
//! node's text where the file's reading decides it:
//! - **Literals.** Every literal of every plan that has literals to share is one searcher: one
//!   overlapping pass over the file finds every occurrence (a literal hidden inside another
//!   included), and each goes to the plans holding it. An occurrence `[s, e)` marks the nodes it is
//!   anchored to through an index of the candidates by start and by end (`Whole`: nodes `[s, e)`;
//!   `Start`: nodes from `s` reaching `e`; `End`: nodes ending at `e` from `s` or before), so the
//!   work follows the occurrences, not the nodes.
//! - **Literal plans** (`How::Exact`) are decided by their occurrences.
//! - **Anchored automata** (`How::Gate`): every match begins with one of the core's prefix literals
//!   (ends with a suffix literal), so only the nodes such an occurrence reaches at their start (end)
//!   are tested, by an anchored search on the node's edge that stops at its first match or dead
//!   state. A `Whole` automaton gates on whichever side's literals are rarer.
//! - **"Contains" automata** (`How::Contains`): the match ends come from one forward pass over the
//!   file (overlapping, so `ab|b` on `xab` ends at 3 for `b` as well), each with the latest start of
//!   a match ending there, from a short backward pass. Node `[s, e)` holds a match iff the earliest
//!   end among matches starting at `s` or after is at most `e` (`Placement`): one binary search per
//!   node.
//! - **Position assertions** (`\b`, a multi-line `^`, `\A` in a branch) read one byte either side
//!   of their position. Only an assertion at a match's own edge (before its first byte or after its
//!   last, `looks_at`) reads outside the match. So a match touching the node's start reads
//!   differently only where the core can assert at its start, and likewise at its end. On such a
//!   side the file's matches count only strictly inside the node, and the node-local `Start` or
//!   `End` plan of the same core (gated as above) decides the matches touching it; a side with no
//!   such assertion needs neither. The node matches iff any of them does.
//! - Anything else (`How::Each`: a backtracking pattern, an automaton with no finite edge
//!   literals) is tested on each candidate's text.

use rerail_blocks::vector::Vector;
use super::Lowered;
use super::plan::{Anchor, Plan, literal_searcher};
use regex_syntax::hir::Hir;
use regex_syntax::hir::literal::{ExtractKind, Extractor};
use std::collections::HashMap;
use std::ops::Range;
use std::sync::{Arc, Mutex};

/// The nodes a test is asked about, and how to read them.
pub struct Nodes<'a> {
    /// The file's text.
    pub text: &'a [u8],
    /// Nodes in the file (bits are over `0..len`).
    pub len: usize,
    /// The nodes to decide, ascending: a tape's preorder, so their starts never decrease (a
    /// match's nodes at a start are found by a search over them, with no index).
    pub candidates: &'a [u32],
    pub span: &'a (dyn Fn(u32) -> Range<usize> + Sync),
}

/// Bit `n` set where candidate `n`'s text matches; `None` where the plan cannot decide a node (a
/// backtracking pattern past its limits, an automaton that did not build). A batch of one plan,
/// built once per plan.
pub fn bits(lowered: &Arc<Lowered>, nodes: &Nodes<'_>) -> Option<Vector<u64>> {
    #[derive(Default)]
    struct AloneBatches(Mutex<HashMap<super::Key, Arc<Batch>>>);
    let batch = {
        let held_ = crate::code_rule_dsl::logic::profile::shared::<AloneBatches>();
        let mut alone = held_.0.lock().expect("scan batches");
        Arc::clone(alone.entry(lowered.key.clone()).or_insert_with(|| Arc::new(Batch::of(vec![Arc::clone(lowered)]))))
    };
    batch.bits(nodes).pop().flatten()
}

/// A set of plans scanned together: one literal pass over each file serves every plan.
pub struct Batch {
    /// The plans asked for, then the parts they are made of (`How::Contains`'s edge plans).
    units: Vec<Unit>,
    /// How many of `units` are plans asked for.
    asked: usize,
    /// Every unit's shared literals as one searcher, and each pattern's units.
    literals: Option<(aho_corasick::AhoCorasick, Vec<Vector<u32>>)>,
}

struct Unit {
    lowered: Arc<Lowered>,
    how: How,
}

/// How a unit is decided.
#[derive(Clone, Debug)]
enum How {
    /// Every candidate, or none.
    Constant(bool),
    /// Its literals' occurrences at the anchor are its matches.
    Exact(Anchor),
    /// Its literals' occurrences at the anchor pick the candidates its automaton tests.
    Gate(Anchor),
    /// Matches placed from the file's forward and backward passes; `strict` per side (start, end),
    /// a match touching the node's edge on that side does not count, and the units `edges` (its core
    /// at those edges) decide the rest.
    Contains { strict: [bool; 2], edges: Vector<usize> },
    /// A literal set with an empty literal: each candidate's edge.
    Edges,
    /// Each candidate's text.
    Each,
}

impl Batch {
    pub fn of(plans: Vec<Arc<Lowered>>) -> Self {
        let asked = plans.len();
        let mut units = Vec::new();
        let mut shared = Vec::<Option<Vec<Vec<u8>>>>::new();
        // The plans asked for first, at their own positions; their parts after them all.
        for lowered in &plans {
            let (how, literals) = how(&lowered.plan);
            units.push(Unit { lowered: Arc::clone(lowered), how });
            shared.push(literals);
        }
        for (at, lowered) in plans.iter().enumerate() {
            // A "contains" core that asserts at a match's edge: its node-local plan at that edge of
            // the node, as a part.
            if let Plan::Automaton { anchor: Anchor::Within, core } = &lowered.plan
                && (looks_at(core, false) || looks_at(core, true))
            {
                let strict = [looks_at(core, false), looks_at(core, true)];
                let mut edges = rerail_blocks::vector::Vector::new();
                for (anchor, _) in [Anchor::Start, Anchor::End].into_iter().zip(strict).filter(|(_, on)| *on) {
                    let part = super::Lowered::of_plan(Plan::Automaton { anchor, core: core.clone() }, "edge");
                    let (how, literals) = how(&part.plan);
                    edges.push(units.len());
                    units.push(Unit { lowered: part, how });
                    shared.push(literals);
                }
                units[at].how = How::Contains { strict, edges };
            }
        }
        // Every shared literal once, with the units holding it.
        let mut patterns = Vec::<Vec<u8>>::new();
        let mut at = HashMap::<Vec<u8>, usize>::new();
        let mut owners = Vec::<Vector<u32>>::new();
        for (u, literals) in shared.iter().enumerate() {
            for literal in literals.iter().flatten() {
                let i = *at.entry(literal.clone()).or_insert_with(|| {
                    patterns.push(literal.clone());
                    owners.push(rerail_blocks::vector::Vector::new());
                    patterns.len() - 1
                });
                owners[i].push(u as u32);
            }
        }
        let literals = if patterns.is_empty() { None } else { literal_searcher(&patterns).map(|searcher| (searcher, owners)) };
        if literals.is_none() {
            // No searcher: the units that needed one test each candidate.
            for (unit, literals) in units.iter_mut().zip(&shared) {
                if literals.is_some() {
                    unit.how = How::Each;
                }
            }
        }
        Self { units, asked, literals }
    }

    /// Each plan's bits (`bits`), in the order asked.
    pub fn bits(&self, nodes: &Nodes<'_>) -> Vec<Option<Vector<u64>>> {
        let words = nodes.len.div_ceil(64);
        if nodes.candidates.is_empty() {
            return vec![Some(rerail_blocks::vector![0; words]); self.asked];
        }
        // Each unit's occurrences of its literals, from one pass.
        let mut hits = vec![Vec::new(); self.units.len()];
        if let Some((searcher, owners)) = &self.literals {
            let _stage = crate::code_rule_dsl::prof::Stage::start("text.scan.literals");
            match searcher.try_find_overlapping_iter(nodes.text) {
                Ok(found) => {
                    for hit in found {
                        for &u in &owners[hit.pattern().as_usize()] {
                            hits[u as usize].push((hit.start() as u32, hit.end() as u32));
                        }
                    }
                }
                Err(error) => panic!("an overlapping literal search: {error}"),
            }
        }
        let mut index = None;
        let mut out: Vec<Option<Vector<u64>>> = vec![None; self.units.len()];
        // Parts first: they follow the plans they make up.
        for u in (0..self.units.len()).rev() {
            let unit = &self.units[u];
            let mut bits = rerail_blocks::vector![0_u64; words];
            let mut set = |node: u32| bits[node as usize / 64] |= 1 << (node % 64);
            let decided = match &unit.how {
                How::Constant(all) => {
                    if *all {
                        nodes.candidates.iter().for_each(|&node| set(node));
                    }
                    Some(())
                }
                How::Exact(anchor) => {
                    mark(*anchor, &hits[u], nodes, &mut index, &mut set);
                    Some(())
                }
                How::Gate(anchor) => {
                    let _stage = crate::code_rule_dsl::prof::Stage::start("text.scan.gated");
                    let mut gated = vec![0_u64; words];
                    mark(*anchor, &hits[u], nodes, &mut index, &mut |node: u32| gated[node as usize / 64] |= 1 << (node % 64));
                    let compiled = unit.lowered.compiled();
                    ones(&gated).try_for_each(|node| {
                        if compiled.is_match(&nodes.text[(nodes.span)(node)])? {
                            set(node);
                        }
                        Some(())
                    })
                }
                How::Contains { strict, edges } => {
                    let _stage = crate::code_rule_dsl::prof::Stage::start("text.scan.contains");
                    match unit.lowered.prepared().automata.as_ref().and_then(|automata| automata.ends(nodes.text)) {
                        Some(ends) => {
                            Placement::of(ends).mark(nodes, *strict, &mut set);
                            edges.iter().try_for_each(|&edge| {
                                ones(out[edge].as_ref()?).for_each(&mut set);
                                Some(())
                            })
                        }
                        // The lazy automaton gave up on this file: each candidate on its text.
                        None => each(&unit.lowered, nodes, &mut set),
                    }
                }
                How::Edges => edges(&unit.lowered, nodes, &mut set),
                How::Each => {
                    let _stage = crate::code_rule_dsl::prof::Stage::start("text.scan.each");
                    each(&unit.lowered, nodes, &mut set)
                }
            };
            out[u] = decided.map(|()| bits);
        }
        out.truncate(self.asked);
        // `RERAIL_SCAN_CHECK`: every verdict against the plan's test of the node's own text.
        if rerail_prof::code_rule_dsl::logic::profile::setting(|| std::env::var_os("RERAIL_SCAN_CHECK").is_some()) {
            for (unit, bits) in self.units.iter().zip(&out) {
                let Some(bits) = bits else { continue };
                let compiled = unit.lowered.compiled();
                for &node in nodes.candidates {
                    let span = (nodes.span)(node);
                    let want = compiled.is_match(&nodes.text[span.clone()]);
                    let got = bits[node as usize / 64] >> (node % 64) & 1 != 0;
                    if want.is_some_and(|want| want != got) {
                        eprintln!("SCAN_CHECK differs: {:?} gave {got} on {:?} in {:?}", unit.how, String::from_utf8_lossy(&nodes.text[span.clone()]).chars().take(80).collect::<String>(), unit.lowered.plan.canonical().chars().take(120).collect::<String>());
                    }
                }
            }
        }
        out
    }
}

/// How a plan is decided, and the literals it adds to the shared pass.
fn how(plan: &Plan) -> (How, Option<Vec<Vec<u8>>>) {
    match plan {
        Plan::Always => (How::Constant(true), None),
        Plan::Never => (How::Constant(false), None),
        Plan::Literals { literals, .. } if literals.iter().any(Vec::is_empty) => (How::Edges, None),
        Plan::Literals { anchor, literals } => (How::Exact(*anchor), Some(literals.clone())),
        Plan::Automaton { anchor: Anchor::Within, .. } => (How::Contains { strict: [false, false], edges: rerail_blocks::vector::Vector::new() }, None),
        Plan::Automaton { anchor, core } => {
            let prefix = matches!(anchor, Anchor::Whole | Anchor::Start).then(|| edge_literals(core, ExtractKind::Prefix)).flatten();
            let suffix = matches!(anchor, Anchor::Whole | Anchor::End).then(|| edge_literals(core, ExtractKind::Suffix)).flatten();
            // The rarer side: the longer shortest literal, then the fewer literals.
            let rarity = |literals: &Vec<Vec<u8>>| (literals.iter().map(Vec::len).min().unwrap_or(0), usize::MAX - literals.len());
            match (prefix, suffix) {
                (Some(prefix), Some(suffix)) if rarity(&suffix) > rarity(&prefix) => (How::Gate(Anchor::End), Some(suffix)),
                (Some(prefix), _) => (How::Gate(Anchor::Start), Some(prefix)),
                (None, Some(suffix)) => (How::Gate(Anchor::End), Some(suffix)),
                (None, None) => (How::Each, None),
            }
        }
        Plan::Backtracking { .. } => (How::Each, None),
    }
}

/// Whether `hir` can assert (`\b`, `^`, ...) at a match's start (`end` false) or end: before it
/// reads its first byte, or after its last. Conservative: a repetition with any assertion does.
fn looks_at(hir: &Hir, end: bool) -> bool {
    use regex_syntax::hir::HirKind;
    if hir.properties().look_set().is_empty() {
        return false;
    }
    match hir.kind() {
        HirKind::Concat(parts) => {
            // Its parts from that edge, up to the first that must read a byte.
            let ordered: Vec<&Hir> = if end { parts.iter().rev().collect() } else { parts.iter().collect() };
            for part in ordered {
                if looks_at(part, end) {
                    return true;
                }
                if part.properties().minimum_len() != Some(0) {
                    return false;
                }
            }
            false
        }
        HirKind::Capture(capture) => looks_at(&capture.sub, end),
        HirKind::Alternation(branches) => branches.iter().any(|branch| looks_at(branch, end)),
        // A look itself, a repetition holding one, or anything else that has one.
        _ => true,
    }
}


/// `core`'s prefix or suffix literals, where it has a finite set of them, none empty: every match
/// begins (ends) with one.
fn edge_literals(core: &Hir, kind: ExtractKind) -> Option<Vec<Vec<u8>>> {
    let mut extractor = Extractor::new();
    extractor.kind(kind).limit_total(256);
    let sequence = extractor.extract(core);
    let literals = sequence.literals()?;
    if literals.is_empty() || literals.iter().any(|literal| literal.as_bytes().is_empty()) {
        return None;
    }
    Some(literals.iter().map(|literal| literal.as_bytes().to_vec()).collect())
}

/// The set bits of `words`, as positions.
fn ones(words: &[u64]) -> impl Iterator<Item = u32> + '_ {
    words.iter().enumerate().flat_map(|(word, &bits)| {
        let mut bits = bits;
        std::iter::from_fn(move || {
            (bits != 0).then(|| {
                let bit = bits.trailing_zeros();
                bits &= bits - 1;
                word as u32 * 64 + bit
            })
        })
    })
}

/// Marks the candidates literal occurrences `hits` (`[start, end)`) match at `anchor`.
fn mark(anchor: Anchor, hits: &[(u32, u32)], nodes: &Nodes<'_>, index: &mut Option<Index>, set: &mut impl FnMut(u32)) {
    if hits.is_empty() {
        return;
    }
    if anchor == Anchor::Within {
        Placement::of(hits.iter().map(|&(start, end)| (end, start)).collect()).mark(nodes, [false, false], set);
        return;
    }
    for &(s, e) in hits {
        match anchor {
            // The candidates starting at `s`: a run of them, found by searching their starts read
            // in place (they never decrease).
            Anchor::Whole | Anchor::Start => {
                let from = nodes.candidates.partition_point(|&node| ((nodes.span)(node).start as u32) < s);
                for &node in &nodes.candidates[from..] {
                    let span = (nodes.span)(node);
                    if span.start as u32 != s {
                        break;
                    }
                    let end = span.end as u32;
                    if end == e || (anchor == Anchor::Start && end > e) {
                        set(node);
                    }
                }
            }
            // Ends do not follow the order: the candidates by end, made once per file where an
            // end-anchored plan has a hit.
            Anchor::End => {
                let index = index.get_or_insert_with(|| {
                    let _stage = crate::code_rule_dsl::prof::Stage::start("text.scan.index");
                    Index::of(nodes)
                });
                let from = index.by_end.partition_point(|&(end, _, _)| end < e);
                for &(_, start, node) in index.by_end[from..].iter().take_while(|&&(end, _, _)| end == e) {
                    if start <= s {
                        set(node);
                    }
                }
            }
            Anchor::Within => unreachable!("placed above"),
        }
    }
}

/// The candidates by start and by end, built once per file for the units that need it.
struct Index {
    /// (end, start, node), ascending.
    by_end: Vec<(u32, u32, u32)>,
}

impl Index {
    fn of(nodes: &Nodes<'_>) -> Self {
        let mut by_end = nodes
            .candidates
            .iter()
            .map(|&node| {
                let span = (nodes.span)(node);
                (span.end as u32, span.start as u32, node)
            })
            .collect::<Vec<_>>();
        by_end.sort_unstable();
        Self { by_end }
    }
}

/// A literal plan with an empty literal, on each candidate's edge.
fn edges(lowered: &Lowered, nodes: &Nodes<'_>, set: &mut impl FnMut(u32)) -> Option<()> {
    let Plan::Literals { anchor, literals } = &lowered.plan else { return None };
    let longest = literals.iter().map(Vec::len).max().unwrap_or(0);
    let compiled = lowered.compiled();
    for &node in nodes.candidates {
        let span = (nodes.span)(node);
        let edge = match anchor {
            Anchor::Whole if span.len() > longest => continue,
            Anchor::Whole | Anchor::Within => span,
            Anchor::Start => span.start..span.end.min(span.start + longest),
            Anchor::End => span.end.saturating_sub(longest).max(span.start)..span.end,
        };
        if compiled.is_match(&nodes.text[edge])? {
            set(node);
        }
    }
    Some(())
}

/// Each candidate tested on its own text; for a "contains" core, only where one of its prefix
/// literals occurs inside the node.
fn each(lowered: &Lowered, nodes: &Nodes<'_>, set: &mut impl FnMut(u32)) -> Option<()> {
    let compiled = lowered.compiled();
    let starts = lowered.prepared().prefixes.as_ref().and_then(|prefixes| {
        let mut starts = prefixes.try_find_overlapping_iter(nodes.text).ok()?.map(|hit| hit.start()).collect::<Vec<_>>();
        starts.sort_unstable();
        starts.dedup();
        Some(starts)
    });
    for &node in nodes.candidates {
        let span = (nodes.span)(node);
        if let Some(starts) = &starts {
            let first = starts.partition_point(|&at| at < span.start);
            if starts.get(first).is_none_or(|&at| at >= span.end) {
                continue;
            }
        }
        if compiled.is_match(&nodes.text[span])? {
            set(node);
        }
    }
    Some(())
}

/// A plan's scan machinery, built once per plan (`Lowered::prepared`) and shared by every file.
pub(in crate::code_rule_dsl) struct Prepared {
    /// A "contains" automaton's forward and reverse automata.
    automata: Option<Automata>,
    /// A "contains" core's prefix literals' searcher, which picks the candidates `each` tests.
    prefixes: Option<aho_corasick::AhoCorasick>,
}

impl Prepared {
    pub(in crate::code_rule_dsl) fn of(plan: &Plan) -> Self {
        match plan {
            Plan::Automaton { anchor: Anchor::Within, core } => {
                Self { automata: Automata::of(core), prefixes: edge_literals(core, ExtractKind::Prefix).and_then(|literals| literal_searcher(&literals)) }
            }
            _ => Self { automata: None, prefixes: None },
        }
    }
}

/// A "contains" automaton's lazy DFAs: forward (every match end, overlapping) and reverse (the
/// latest start of a match ending at a given end), with a pool of their caches. Assertions read
/// the file around a position (a node's text is read by its edge plans, `How::Contains`).
struct Automata {
    forward: regex_automata::hybrid::dfa::DFA,
    reverse: regex_automata::hybrid::dfa::DFA,
    caches: Mutex<Vec<(regex_automata::hybrid::dfa::Cache, regex_automata::hybrid::dfa::Cache)>>,
}

impl Automata {
    fn of(core: &Hir) -> Option<Self> {
        use regex_automata::MatchKind;
        use regex_automata::hybrid::dfa::DFA;
        use regex_automata::nfa::thompson;
        // A Unicode word boundary runs while the text is ASCII; past it the search gives up.
        let config = || DFA::config().match_kind(MatchKind::All).unicode_word_boundary(true);
        let forward = DFA::builder().configure(config()).build_from_nfa(thompson::Compiler::new().build_from_hir(core).ok()?).ok()?;
        let reverse = DFA::builder()
            .configure(config())
            .build_from_nfa(thompson::Compiler::new().configure(thompson::Config::new().reverse(true)).build_from_hir(core).ok()?)
            .ok()?;
        Some(Self { forward, reverse, caches: Default::default() })
    }

    /// Every (end, latest start) of a match in `text`; `None` where the lazy automaton gives up (its
    /// cache thrashes, or a Unicode word boundary meets a non-ASCII byte).
    fn ends(&self, text: &[u8]) -> Option<Vec<(u32, u32)>> {
        use regex_automata::hybrid::dfa::OverlappingState;
        use regex_automata::{Anchored, Input};
        let pooled = self.caches.lock().ok()?.pop();
        let (mut fc, mut rc) = pooled.unwrap_or_else(|| (self.forward.create_cache(), self.reverse.create_cache()));
        let found = (|| {
            let input = Input::new(text);
            let mut state = OverlappingState::start();
            let mut out = Vec::new();
            loop {
                self.forward.try_search_overlapping_fwd(&mut fc, &input, &mut state).ok()?;
                let Some(end) = state.get_match() else { break };
                let end = end.offset();
                // The latest start: the shortest match ending here, found going back.
                let back = Input::new(text).range(..end).anchored(Anchored::Yes).earliest(true);
                let start = self.reverse.try_search_rev(&mut rc, &back).ok()??.offset();
                out.push((end as u32, start as u32));
            }
            Some(out)
        })();
        if let Ok(mut caches) = self.caches.lock() {
            caches.push((fc, rc));
        }
        found
    }
}

/// Matches as (start, end) pairs, for the per-node test: node `[s, e)` holds a match iff the
/// earliest end among matches starting at `s` or after is at most `e` (strictly inside: starting
/// after `s`, ending before `e`).
struct Placement {
    /// Ascending, distinct.
    starts: Vector<u32>,
    /// `earliest[i]`: the least end of a match starting at `starts[i]` or after.
    earliest: Vector<u32>,
}

impl Placement {
    /// From (end, start) pairs, as the scans find them.
    fn of(pairs: Vec<(u32, u32)>) -> Self {
        let mut pairs = pairs.into_iter().map(|(end, start)| (start, end)).collect::<Vec<_>>();
        pairs.sort_unstable();
        let mut starts = rerail_blocks::vector::Vector::with_capacity(pairs.len());
        let mut earliest = rerail_blocks::vector::Vector::with_capacity(pairs.len());
        for (start, end) in pairs {
            // Sorted by start, then end: the first pair of a start has its least end.
            if starts.last() != Some(&start) {
                starts.push(start);
                earliest.push(end);
            }
        }
        for i in (0..earliest.len().saturating_sub(1)).rev() {
            earliest[i] = earliest[i].min(earliest[i + 1]);
        }
        Self { starts, earliest }
    }

    fn mark(&self, nodes: &Nodes<'_>, strict: [bool; 2], set: &mut impl FnMut(u32)) {
        if self.starts.is_empty() {
            return;
        }
        let _stage = crate::code_rule_dsl::prof::Stage::start("text.scan.place");
        // A candidate starting past the last match's start holds none (starts never decrease).
        let last = *self.starts.last().expect("a match");
        let upto = nodes.candidates.partition_point(|&node| (nodes.span)(node).start as u32 <= last);
        for &node in &nodes.candidates[..upto] {
            let span = (nodes.span)(node);
            let (s, e) = (span.start as u32, span.end as u32);
            let first = if strict[0] { self.starts.partition_point(|&start| start <= s) } else { self.starts.partition_point(|&start| start < s) };
            if self.earliest.get(first).is_some_and(|&end| if strict[1] { end < e } else { end <= e }) {
                set(node);
            }
        }
    }
}
