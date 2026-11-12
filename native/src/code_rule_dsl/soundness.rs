use super::embed::segments::FileRanges;
use super::lines::LineTable;
use super::matcher::{AstMatcher, GrammarMatcher, UtilRef, UtilitySummary, stop_rule};
use super::origin::Origin;
use super::tape::TapeDoc;
#[cfg(test)]
use crate::code_rule_dsl::walks::{Walk, record_walk};
use std::collections::BTreeSet;
use std::ops::Range;

mod semantic;

type SoundMemo<T> = UtilitySummary<(usize, bool, bool), T>;

// Error-tolerant evaluation drops every witness whose range meets a syntax-error region, so
// claims about nodes inside the witness (pattern, kind, regex, `has`, and negations of those)
// only ever read cleanly parsed source. Relations that leave the witness range (`inside`,
// `precedes`, `follows`) can reach nodes in or around an error region. A positive claim there
// is still a fact of the recovered tree: the related node is the witness's parent, an ancestor,
// or a sibling under the same parent, all of which the parser produced; if that node is an ERROR
// ancestor, the witness lies inside it and is dropped anyway. `stopBy: end` only widens the walk
// along the same axis (every ancestor, every sibling of the one parent), so it is sound wherever
// `neighbor` is. Negating a relation that leaves the witness, or anything beneath one, turns it
// into an absence claim over siblings or ancestors the parser may not have read; such rules are
// not evaluated in files with syntax errors. Checking per file whether the parent's range is
// clean could admit some of them; that refinement is not attempted.
//
// A capture that an outward relation binds under `any` is unbound exactly when that relation
// failed, which an error region can cause: reading it where unbound means "pass" (a negated
// comparison, a constraint) turns the relation's failure into a violation, so such a read is unsound.
// Positive comparisons fail on an unbound side, a false negative at worst.
pub(super) fn sound_under_errors(matcher: &AstMatcher) -> bool {
    let mut optional = BTreeSet::new();
    optional_outward_captures(matcher, false, false, &mut SoundMemo::new(), &mut optional);
    sound(matcher, false, false, &optional, &mut SoundMemo::new())
}

/// Names a pattern binds through a relation that leaves the witness and sits under `any`, where
/// its failure does not fail the match. Negations, counts and stop rules are walked too, since a
/// comparison inside them reads their own bindings.
fn optional_outward_captures(
    matcher: &AstMatcher,
    under_any: bool,
    outward: bool,
    memo: &mut SoundMemo<BTreeSet<String>>,
    names: &mut BTreeSet<String>,
) {
    let mut walk = |item: &AstMatcher, under_any: bool, outward: bool| {
        optional_outward_captures(item, under_any, outward, memo, names);
    };
    match matcher {
        AstMatcher::Pattern(pattern) => {
            if under_any && outward {
                names.extend(pattern.defined_vars().into_iter().map(str::to_owned));
            }
        }
        AstMatcher::Kind(_) | AstMatcher::Supertype(_) | AstMatcher::Regex(_) | AstMatcher::Compare(_) | AstMatcher::Never => {
        }
        AstMatcher::Semantic(semantic) => {
            semantic::optional_outward_captures(semantic, under_any, outward, memo, names)
        }
        #[cfg(any(test, feature = "test-hooks"))]
        AstMatcher::TestUnknown(_) => {}
        AstMatcher::All(items) => items.iter().for_each(|item| walk(item, under_any, outward)),
        AstMatcher::Any(items) => items.iter().for_each(|item| walk(item, true, outward)),
        AstMatcher::Not(item) => walk(item, under_any, outward),
        AstMatcher::Count(count) => walk(&count.relation, under_any, outward),
        AstMatcher::Has(item, stop) => {
            walk(item, under_any, outward);
            if let Some(stop) = stop_rule(stop) {
                walk(stop, under_any, outward);
            }
        }
        AstMatcher::Inside(item, stop)
        | AstMatcher::Precedes(item, stop)
        | AstMatcher::Follows(item, stop) => {
            walk(item, under_any, true);
            if let Some(stop) = stop_rule(stop) {
                walk(stop, under_any, true);
            }
        }
        AstMatcher::NthChild(nth) => {
            if let Some(rule) = &nth.of_rule {
                walk(rule, under_any, outward);
            }
        }
        AstMatcher::Field(relation) => {
            let outward = outward || relation.inside;
            walk(&relation.rule, under_any, outward);
            if let Some(stop) = stop_rule(&relation.stop) {
                walk(stop, under_any, outward);
            }
        }
        // A utility's names, and a frame's own (its comparisons read them) together with the
        // caller's names of the parameters among them.
        AstMatcher::Util(util) => {
            names.extend(optional_utility_captures(util, under_any, outward, memo));
        }
        AstMatcher::Call(call) => {
            let found = optional_utility_captures(&call.util, under_any, outward, memo);
            names.extend(call.outward(&found).into_iter().map(str::to_owned));
            names.extend(found);
        }
        AstMatcher::Constrained(core) => {
            walk(&core.rule, under_any, outward);
            for (_, constraint) in &core.constraints {
                walk(constraint, under_any, outward);
            }
        }
    }
}

/// `optional_outward_captures` of a utility's rule in a context, in the utility's own names.
fn optional_utility_captures(
    util: &UtilRef,
    under_any: bool,
    outward: bool,
    memo: &mut SoundMemo<BTreeSet<String>>,
) -> BTreeSet<String> {
    memo.summarize((util.index, under_any, outward), BTreeSet::new(), |memo| {
        #[cfg(test)]
        record_walk(Walk::OptionalCaptures);
        let mut found = BTreeSet::new();
        util.with_target(|rule| {
            optional_outward_captures(rule, under_any, outward, memo, &mut found)
        });
        found
    })
}

// A `matches` reference is as sound as its target in the same context: a verdict is the
// conjunction of everything the rule reaches, so a recursive reference adds nothing and starts at
// sound. Constraints read captured nodes, which lie outside the witness whenever the rule relates
// it to nodes outside it.
fn sound(
    matcher: &AstMatcher,
    negated: bool,
    outside: bool,
    optional: &BTreeSet<String>,
    memo: &mut SoundMemo<bool>,
) -> bool {
    match matcher {
        // Never has no relations and matches nothing, so an error region cannot change its verdict.
        // A comparison reads captured nodes' text, as a constraint's regex does.
        // A value reads the node's own text, like a regex; a builtin name is a value only in a
        // file without syntax errors, so an error region cannot hide the binding that shadows it.
        // Arguments read the call's own argument list, inside the witness, as its pattern does.
        AstMatcher::Pattern(_) | AstMatcher::Kind(_) | AstMatcher::Supertype(_) | AstMatcher::Regex(_) | AstMatcher::Never => {
            true
        }
        AstMatcher::Semantic(semantic) => {
            semantic::sound(semantic, negated, outside, optional, memo)
        }
        #[cfg(any(test, feature = "test-hooks"))]
        AstMatcher::TestUnknown(_) => true,
        // A position counts the siblings before the node, and an error region among them can hide
        // or add one.
        AstMatcher::NthChild(_) => false,
        // An unbound side makes a comparison fail, so negated it holds vacuously.
        // A transform variable reads the captures it is computed from.
        AstMatcher::Compare(comparison) => {
            let optional_read = |name: &str| {
                optional.contains(name)
                    || comparison.transform.as_ref().is_some_and(|transform| {
                        transform
                            .roots(name)
                            .into_iter()
                            .any(|root| optional.contains(root))
                    })
            };
            !negated || !(optional_read(&comparison.left) || optional_read(&comparison.right))
        }
        AstMatcher::All(items) | AstMatcher::Any(items) => items
            .iter()
            .all(|item| sound(item, negated, outside, optional, memo)),
        AstMatcher::Not(item) => !outside && sound(item, true, outside, optional, memo),
        // A stop rule decides where the walk ends, so its non-matches count as much as its match:
        // it is an absence claim over the nodes walked past, classified as `not` over those nodes.
        AstMatcher::Has(item, stop) => {
            sound(item, negated, outside, optional, memo)
                && stop_rule(stop)
                    .is_none_or(|stop| !outside && sound(stop, true, outside, optional, memo))
        }
        // Walking ancestors or siblings past nodes that did not stop it claims they are absent,
        // like a `not` outside the witness: a relation with a stop rule is unsound under errors.
        AstMatcher::Inside(item, stop)
        | AstMatcher::Precedes(item, stop)
        | AstMatcher::Follows(item, stop) => {
            !negated && stop_rule(stop).is_none() && sound(item, negated, true, optional, memo)
        }
        // A field narrows which nodes the walk tests; it is classified as the relation it narrows.
        AstMatcher::Field(relation) if relation.inside => {
            !negated
                && stop_rule(&relation.stop).is_none()
                && sound(&relation.rule, negated, true, optional, memo)
        }
        AstMatcher::Field(relation) => {
            sound(&relation.rule, negated, outside, optional, memo)
                && stop_rule(&relation.stop)
                    .is_none_or(|stop| !outside && sound(stop, true, outside, optional, memo))
        }
        AstMatcher::Util(util) => utility_sound(util, negated, outside, optional, memo),
        AstMatcher::Call(call) => utility_sound(&call.util, negated, outside, optional, memo),
        // A count is a presence and an absence claim over the relation's nodes, so it is as sound
        // as the negated relation.
        AstMatcher::Count(count) => {
            !outside && sound(&count.relation, true, outside, optional, memo)
        }
        AstMatcher::Constrained(core) => {
            let captures_outside =
                outside || leaves_witness(&core.rule, &mut UtilitySummary::new());
            // A constraint on an unbound capture is skipped: it passes vacuously.
            let skippable = core
                .constraints
                .iter()
                .any(|(name, _)| optional.contains(name.as_str()));
            !skippable
                && sound(&core.rule, negated, outside, optional, memo)
                && core.constraints.iter().all(|(_, constraint)| {
                    sound(constraint, negated, captures_outside, optional, memo)
                })
        }
    }
}

fn utility_sound(
    util: &UtilRef,
    negated: bool,
    outside: bool,
    optional: &BTreeSet<String>,
    memo: &mut SoundMemo<bool>,
) -> bool {
    memo.summarize((util.index, negated, outside), true, |memo| {
        #[cfg(test)]
        record_walk(Walk::SoundVerdict);
        util.with_target(|rule| sound(rule, negated, outside, optional, memo))
            .unwrap_or(true)
    })
}

/// Whether a rule relates its node to nodes outside that node's subtree (and so may bind them).
/// A stop rule binds nothing, so it never adds a capture.
fn leaves_witness(matcher: &AstMatcher, memo: &mut UtilitySummary<usize, bool>) -> bool {
    match matcher {
        AstMatcher::Pattern(_)
        | AstMatcher::Kind(_) | AstMatcher::Supertype(_)
        | AstMatcher::Regex(_)
        | AstMatcher::Compare(_)
        | AstMatcher::Never => false,
        AstMatcher::Semantic(semantic) => semantic::leaves_witness(semantic, memo),
        #[cfg(any(test, feature = "test-hooks"))]
        AstMatcher::TestUnknown(_) => false,
        AstMatcher::All(items) | AstMatcher::Any(items) => {
            items.iter().any(|item| leaves_witness(item, memo))
        }
        // A negation binds nothing and a count only its value's text (`as`), so nothing either
        // reaches can become a capture.
        AstMatcher::Not(_) | AstMatcher::Count(_) => false,
        AstMatcher::Has(item, _) => leaves_witness(item, memo),
        AstMatcher::Inside(..) | AstMatcher::Precedes(..) | AstMatcher::Follows(..) => true,
        AstMatcher::Field(relation) => relation.inside || leaves_witness(&relation.rule, memo),
        AstMatcher::NthChild(nth) => nth
            .of_rule
            .as_ref()
            .is_some_and(|rule| leaves_witness(rule, memo)),
        AstMatcher::Util(util) => utility_leaves_witness(util, memo),
        AstMatcher::Call(call) => utility_leaves_witness(&call.util, memo),
        AstMatcher::Constrained(core) => {
            leaves_witness(&core.rule, memo)
                || core
                    .constraints
                    .iter()
                    .any(|(_, constraint)| leaves_witness(constraint, memo))
        }
    }
}

fn utility_leaves_witness(util: &UtilRef, memo: &mut UtilitySummary<usize, bool>) -> bool {
    memo.summarize(util.index, false, |memo| {
        util.with_target(|rule| leaves_witness(rule, memo))
            .unwrap_or(false)
    })
}

/// Byte ranges of the file a document's parser could not read: ERROR subtrees and zero-width
/// MISSING tokens, and for a guest document (`embed`) the file bytes its own stand for and the
/// host's errors it reads through (`TapeDoc::host_errors`). Sorted by start; overlapping ranges
/// are merged, so range ends never decrease. Each document of a file has its own: a host's errors
/// are not a guest's, unless the guest reads the host through them (a Blade view).
pub(super) struct SyntaxErrors {
    regions: Vec<Range<usize>>,
}

/// The share of a file's bytes, in percent, its error regions must cover for the file to count as
/// not read at all. Below it, sound clauses run around the regions (`source-partial`); at it or
/// above, what the parser recovered is mostly guesswork (a tree-sitter grammar that gave up
/// returns one ERROR over the whole file), so no AST clause runs there.
const UNREAD_PERCENT: usize = 50;

/// The failure message an AST clause gets on a file the parser did not read; persisted results
/// store it, and `SyntaxReading::Unread` carries the detail reported in its place.
pub(super) const UNREAD: &str = "file did not parse";
/// The `source-incomplete` failure of an AST clause a file's syntax errors kept out.
pub(super) const SYNTAX_ERRORS: &str = "file has syntax errors";

/// How much of a file with syntax errors its AST clauses read, with the message to report.
pub(super) enum SyntaxReading {
    /// Sound clauses ran outside the error regions (`source-partial`).
    Partial(String),
    /// The regions cover at least `UNREAD_PERCENT` of the file: no AST clause ran there
    /// (`source-incomplete`, `UNREAD`).
    Unread(String),
}

/// A document's error regions as ranges of the file, unmerged: its ERROR and MISSING nodes', and
/// the file bytes a guest cannot be read through.
fn file_ranges(doc: &TapeDoc) -> Vec<std::ops::Range<usize>> {
    // An error node of a program standing for several file ranges is an error in each.
    let mut ranges = Vec::new();
    for (index, range) in doc.syntax_error_nodes() {
        if let Some(placed) = Origin::of_node(doc, index, range) {
            ranges.extend(placed.iter().cloned());
        }
    }
    ranges.extend(doc.host_errors());
    ranges
}

impl SyntaxErrors {
    /// The native tree's regions, which tapes record (`TapeDoc::syntax_error_ranges`).
    #[cfg(test)]
    pub(super) fn collect(root: tree_sitter::Node<'_>) -> Self {
        let mut found = Vec::new();
        // `has_error` covers the whole subtree, so clean subtrees are never entered.
        let mut stack = if root.has_error() {
            vec![root]
        } else {
            Vec::new()
        };
        while let Some(node) = stack.pop() {
            if node.is_error() || node.is_missing() {
                found.push(node.byte_range());
                continue;
            }
            let mut cursor = node.walk();
            stack.extend(node.children(&mut cursor).filter(|child| child.has_error()));
        }
        Self::from_ranges(found)
    }

    /// Regions from ERROR and MISSING node ranges, in any order.
    pub(super) fn from_ranges(mut found: Vec<Range<usize>>) -> Self {
        found.sort_by_key(|range| (range.start, range.end));
        let mut regions: Vec<Range<usize>> = Vec::with_capacity(found.len());
        for range in found {
            match regions.last_mut() {
                Some(last) if range.start < last.end || *last == range => {
                    last.end = last.end.max(range.end);
                }
                _ => regions.push(range),
            }
        }
        Self { regions }
    }

    /// The regions of a restored document, as ranges of the file.
    pub(super) fn of_doc(doc: &TapeDoc) -> Self {
        Self::from_ranges(file_ranges(doc))
    }

    /// The regions of every document of the file whose own tree `host` is (`embed`): what a rule
    /// crossing between them (`GrammarMatcher::crosses`) reads around.
    pub(super) fn of_file(host: &TapeDoc) -> Self {
        let mut ranges = file_ranges(host);
        for guest in host.guests().iter().flat_map(|guests| guests.iter()) {
            ranges.extend(file_ranges(&guest.doc));
        }
        Self::from_ranges(ranges)
    }

    /// Whether the errors leave a clause of `grammar` incomplete: one not sound under them
    /// (`sound_under_errors`).
    pub(super) fn incomplete(&self, grammar: &GrammarMatcher) -> bool {
        !self.is_empty() && !grammar.sound_under_errors
    }

    /// Whether a binding may hide in an error region from the document's scope tables.
    pub(super) fn hide_bindings(&self) -> bool {
        !self.is_empty()
    }

    pub(super) fn is_empty(&self) -> bool {
        self.regions.is_empty()
    }

    /// The merged regions, by start.
    pub(super) fn regions(&self) -> &[Range<usize>] {
        &self.regions
    }

    /// Whether any of the file ranges a node stands for meets a region (`overlaps`).
    pub(super) fn meets(&self, ranges: &FileRanges) -> bool {
        ranges.iter().any(|range| self.overlaps(range))
    }

    /// Whether `range` meets any region; a zero-width MISSING point meets a range that touches it.
    pub(super) fn overlaps(&self, range: &Range<usize>) -> bool {
        let first = self
            .regions
            .partition_point(|region| region.end < range.start);
        self.regions[first..]
            .iter()
            .take_while(|region| region.start <= range.end)
            .any(|region| {
                if region.is_empty() {
                    range.start <= region.start && region.start <= range.end
                } else if range.is_empty() {
                    region.start <= range.start && range.start < region.end
                } else {
                    range.start < region.end && region.start < range.end
                }
            })
    }

    pub(super) fn partial_message(&self, lines: &LineTable) -> String {
        let (line, column) = lines.point(self.regions[0].start);
        format!(
            "file has syntax errors; AST clauses ran outside {} error region(s), first at line {}, column {}",
            self.regions.len(),
            line + 1,
            column + 1
        )
    }

    /// Bytes inside some region; merged regions never overlap.
    fn covered_bytes(&self) -> usize {
        self.regions
            .iter()
            .map(|region| region.end - region.start)
            .sum()
    }

    /// How a file of `source_len` bytes with these (non-empty) regions is read. An empty file's
    /// regions (zero-width MISSING tokens) cover none of it.
    pub(super) fn reading(&self, lines: &LineTable, source_len: usize) -> SyntaxReading {
        let covered = self.covered_bytes();
        if source_len == 0 || covered * 100 < source_len * UNREAD_PERCENT {
            return SyntaxReading::Partial(self.partial_message(lines));
        }
        let (line, column) = lines.point(self.regions[0].start);
        SyntaxReading::Unread(format!(
            "{UNREAD}: error regions cover {}% of it, first at line {}, column {}",
            covered * 100 / source_len,
            line + 1,
            column + 1
        ))
    }
}

#[cfg(test)]
mod tests;
