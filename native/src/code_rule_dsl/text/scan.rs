//! The scan engine: a plan over text with no vocabulary (a module, a function body, a test on
//! nodes of any kind), with the verdict wanted per node: does the node's own text match?
//!
//! A test reads the file once, never each node's text. For "contains a match" (`Anchor::Within`):
//! - every position where a match ends is found by one forward pass (overlapping, so a match
//!   hidden inside another counts: `ab|b` on `xab` ends at 3 for `b` as well);
//! - for each end, the latest start of a match ending there, by a short backward pass (a reverse
//!   automaton stopping at its first match) or, for literals, by the literal's length;
//! - node `[s, e)` holds a match iff some end in `(s, e]` has its latest start at `s` or after: a
//!   range-max query per node (`Placement`).
//!
//! Tests anchored at a node's start or end (`Whole`, `Start`, `End`) read only the node's edge:
//! an anchored search stops at its first match or dead state. A core with a position assertion
//! (`\b`, a multi-line `^`) can read differently in the file and in the node's text, so it is tested
//! on each candidate node's text, after a file-wide pass for its prefix literals rules most out.

use super::Lowered;
use super::plan::{Anchor, Compiled, Plan, literal_searcher};
use regex_syntax::hir::literal::{ExtractKind, Extractor};
use std::ops::Range;

/// The nodes a test is asked about, and how to read them.
pub(in crate::code_rule_dsl) struct Nodes<'a> {
    /// The file's text.
    pub(in crate::code_rule_dsl) text: &'a [u8],
    /// Nodes in the file (bits are over `0..len`).
    pub(in crate::code_rule_dsl) len: usize,
    /// The nodes to decide, ascending.
    pub(in crate::code_rule_dsl) candidates: &'a [u32],
    pub(in crate::code_rule_dsl) span: &'a (dyn Fn(u32) -> Range<usize> + Sync),
}

/// Bit `n` set where candidate `n`'s text matches; `None` where the plan cannot decide a node (a
/// backtracking pattern past its limits, an automaton that did not build).
pub(in crate::code_rule_dsl) fn bits(lowered: &Lowered, nodes: &Nodes<'_>) -> Option<Vec<u64>> {
    let mut bits = vec![0_u64; nodes.len.div_ceil(64)];
    let mut set = |node: u32| bits[node as usize / 64] |= 1 << (node % 64);
    match &lowered.plan {
        Plan::Always => nodes.candidates.iter().for_each(|&n| set(n)),
        Plan::Never => {}
        Plan::Literals { anchor: Anchor::Within, literals } => {
            crate::code_rule_dsl::prof::count("text.scan.placed", 1);
            let searcher = literal_searcher(literals)?;
            let mut ends = Vec::new();
            for hit in searcher.try_find_overlapping_iter(nodes.text).ok()? {
                ends.push((hit.end() as u32, hit.start() as u32));
            }
            Placement::of(ends).mark(nodes, &mut set);
        }
        Plan::Literals { anchor, literals } => {
            crate::code_rule_dsl::prof::count("text.scan.edges", 1);
            let lengths = literals.iter().map(Vec::len).collect::<std::collections::BTreeSet<_>>();
            let longest = lengths.last().copied().unwrap_or(0);
            let compiled = lowered.compiled();
            for &node in nodes.candidates {
                let span = (nodes.span)(node);
                let edge = match anchor {
                    // The whole text: only a node of a literal's length can be one.
                    Anchor::Whole if !lengths.contains(&span.len()) => continue,
                    Anchor::Whole => span,
                    Anchor::Start => span.start..span.end.min(span.start + longest),
                    _ => span.end.saturating_sub(longest).max(span.start)..span.end,
                };
                if compiled.is_match(&nodes.text[edge])? {
                    set(node);
                }
            }
        }
        Plan::Automaton { anchor: Anchor::Within, core } if core.properties().look_set().is_empty() => {
            crate::code_rule_dsl::prof::count("text.scan.placed", 1);
            match ends(core, nodes.text) {
                Some(ends) => Placement::of(ends).mark(nodes, &mut set),
                // The lazy automaton gave up on this file: each candidate on its text.
                None => each(lowered, nodes, None, &mut set)?,
            }
        }
        Plan::Automaton { anchor: Anchor::Within, core } => {
            crate::code_rule_dsl::prof::count("text.scan.nodes", 1);
            let starts = prefix_starts(core, nodes.text);
            each(lowered, nodes, starts.as_deref(), &mut set)?;
        }
        Plan::Automaton { .. } | Plan::Backtracking { .. } => {
            crate::code_rule_dsl::prof::count("text.scan.edges", 1);
            each(lowered, nodes, None, &mut set)?;
        }
    }
    Some(bits)
}

/// Each candidate tested on its own text, where `starts` (ascending positions, where given) holds
/// one inside its span.
fn each(lowered: &Lowered, nodes: &Nodes<'_>, starts: Option<&[usize]>, set: &mut impl FnMut(u32)) -> Option<()> {
    let compiled: std::sync::Arc<Compiled> = lowered.compiled();
    for &node in nodes.candidates {
        let span = (nodes.span)(node);
        if let Some(starts) = starts {
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

/// Where a match of `core` may start in `text`: the occurrences of its prefix literals, where it has
/// a finite set of them (none empty), ascending. `None` where it has none.
fn prefix_starts(core: &regex_syntax::hir::Hir, text: &[u8]) -> Option<Vec<usize>> {
    let mut extractor = Extractor::new();
    extractor.kind(ExtractKind::Prefix).limit_total(64);
    let sequence = extractor.extract(core);
    let literals = sequence.literals()?;
    if literals.is_empty() || literals.iter().any(|literal| literal.as_bytes().is_empty()) {
        return None;
    }
    let searcher = literal_searcher(&literals.iter().map(|l| l.as_bytes().to_vec()).collect::<Vec<_>>())?;
    let mut starts = searcher.try_find_overlapping_iter(text).ok()?.map(|hit| hit.start()).collect::<Vec<_>>();
    starts.sort_unstable();
    starts.dedup();
    Some(starts)
}

/// Every (end, latest start) of a match of `core` in `text`; `None` where the lazy automaton gives
/// up (its cache thrashes) or does not build.
fn ends(core: &regex_syntax::hir::Hir, text: &[u8]) -> Option<Vec<(u32, u32)>> {
    use regex_automata::hybrid::dfa::{DFA, OverlappingState};
    use regex_automata::nfa::thompson;
    use regex_automata::{Anchored, Input, MatchKind};
    let forward = DFA::builder()
        .configure(DFA::config().match_kind(MatchKind::All))
        .build_from_nfa(thompson::Compiler::new().build_from_hir(core).ok()?)
        .ok()?;
    let reverse = DFA::builder()
        .configure(DFA::config().match_kind(MatchKind::All))
        .build_from_nfa(thompson::Compiler::new().configure(thompson::Config::new().reverse(true)).build_from_hir(core).ok()?)
        .ok()?;
    let (mut fc, mut rc) = (forward.create_cache(), reverse.create_cache());
    let input = Input::new(text);
    let mut state = OverlappingState::start();
    let mut out = Vec::new();
    loop {
        forward.try_search_overlapping_fwd(&mut fc, &input, &mut state).ok()?;
        let Some(end) = state.get_match() else { break };
        let end = end.offset();
        // The latest start: the shortest match ending here, found going back.
        let back = Input::new(text).range(..end).anchored(Anchored::Yes).earliest(true);
        let start = reverse.try_search_rev(&mut rc, &back).ok()??.offset();
        out.push((end as u32, start as u32));
    }
    Some(out)
}

/// Match ends with their latest starts, for range-max queries by node span.
struct Placement {
    /// Ascending, distinct.
    ends: Vec<u32>,
    /// A max segment tree over each end's latest start: leaves at `ends.len()..`.
    tree: Vec<u32>,
}

impl Placement {
    fn of(mut pairs: Vec<(u32, u32)>) -> Self {
        pairs.sort_unstable();
        // The latest start per end.
        let mut merged: Vec<(u32, u32)> = Vec::with_capacity(pairs.len());
        for (end, start) in pairs {
            match merged.last_mut() {
                Some(last) if last.0 == end => last.1 = last.1.max(start),
                _ => merged.push((end, start)),
            }
        }
        let n = merged.len();
        let mut tree = vec![0_u32; 2 * n];
        for (i, &(_, start)) in merged.iter().enumerate() {
            tree[n + i] = start;
        }
        for i in (1..n).rev() {
            tree[i] = tree[2 * i].max(tree[2 * i + 1]);
        }
        Self { ends: merged.into_iter().map(|(end, _)| end).collect(), tree }
    }

    /// The latest start over ends `lo..hi`, if any.
    fn max(&self, lo: usize, hi: usize) -> Option<u32> {
        let n = self.ends.len();
        let (mut lo, mut hi) = (lo + n, hi + n);
        let mut best = None::<u32>;
        while lo < hi {
            if lo & 1 == 1 {
                best = Some(best.map_or(self.tree[lo], |b| b.max(self.tree[lo])));
                lo += 1;
            }
            if hi & 1 == 1 {
                hi -= 1;
                best = Some(best.map_or(self.tree[hi], |b| b.max(self.tree[hi])));
            }
            lo /= 2;
            hi /= 2;
        }
        best
    }

    fn mark(&self, nodes: &Nodes<'_>, set: &mut impl FnMut(u32)) {
        if self.ends.is_empty() {
            return;
        }
        for &node in nodes.candidates {
            let span = (nodes.span)(node);
            let (s, e) = (span.start as u32, span.end as u32);
            let lo = self.ends.partition_point(|&end| end <= s);
            let hi = self.ends.partition_point(|&end| end <= e);
            if self.max(lo, hi).is_some_and(|start| start >= s) {
                set(node);
            }
        }
    }
}
