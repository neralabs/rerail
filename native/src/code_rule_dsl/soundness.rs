use super::lines::LineTable;
use super::matcher::{AstMatcher, GrammarMatcher, UtilRef, UtilitySummary, stop_rule};
use super::origin::Origin;
use super::tape::TapeDoc;
#[cfg(test)]
use super::test_support::{Walk, record_walk};
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
        AstMatcher::Kind(_) | AstMatcher::Regex(_) | AstMatcher::Compare(_) | AstMatcher::Never => {
        }
        AstMatcher::Semantic(semantic) => {
            semantic::optional_outward_captures(semantic, under_any, outward, memo, names)
        }
        #[cfg(test)]
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
        AstMatcher::Pattern(_) | AstMatcher::Kind(_) | AstMatcher::Regex(_) | AstMatcher::Never => {
            true
        }
        AstMatcher::Semantic(semantic) => {
            semantic::sound(semantic, negated, outside, optional, memo)
        }
        #[cfg(test)]
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
        | AstMatcher::Kind(_)
        | AstMatcher::Regex(_)
        | AstMatcher::Compare(_)
        | AstMatcher::Never => false,
        AstMatcher::Semantic(semantic) => semantic::leaves_witness(semantic, memo),
        #[cfg(test)]
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
    let mut ranges = doc
        .syntax_error_nodes()
        .filter_map(|(index, range)| Origin::of_node(doc, index, range))
        .collect::<Vec<_>>();
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
mod tests {
    use super::*;
    use crate::code_rule_dsl::language::GrammarDialect;
    use crate::code_rule_dsl::model::{ClauseKind, Query};
    use crate::code_rule_dsl::test_support::*;
    use ast_grep_language::LanguageExt;
    use serde_json::{Value, json};

    fn compiled_sound(ast: Value) -> bool {
        grammar_matchers(&compiled(ast))
            .iter()
            .all(|grammar| grammar.sound_under_errors)
    }

    fn syntax_errors(language: GrammarDialect, source: &str) -> SyntaxErrors {
        let root = language.ast_grep(source);
        SyntaxErrors::collect(root.root().get_doc().tree.root_node())
    }

    #[test]
    fn syntax_errors_cover_error_and_missing_nodes_only() {
        assert!(syntax_errors(GrammarDialect::TypeScript, "debugger;\n").is_empty());
        let source = "debugger;\nfunction f( {\n  debugger;\n";
        let errors = syntax_errors(GrammarDialect::TypeScript, source);
        assert!(!errors.is_empty());
        assert!(!errors.overlaps(&(0..9)), "the leading statement is clean");
        assert!(
            errors.overlaps(&(0..source.len())),
            "the program contains the error"
        );
        let lines = LineTable::new(source, crate::code_rule_dsl::lines::Terminators::Eslint);
        assert!(errors.partial_message(&lines).contains("first at line 2"));
    }

    fn reading(errors: &SyntaxErrors, source: &str) -> Result<String, String> {
        let lines = LineTable::new(source, crate::code_rule_dsl::lines::Terminators::Eslint);
        match errors.reading(&lines, source.len()) {
            SyntaxReading::Partial(message) => Ok(message),
            SyntaxReading::Unread(message) => Err(message),
        }
    }

    #[test]
    fn a_file_whose_regions_cover_half_of_it_or_more_is_unread() {
        let source = "0123456789\n0123456789\n";
        let region =
            |range: Range<usize>| SyntaxErrors::from_ranges(std::iter::once(range).collect());
        // 10 of 22 bytes, then 11 of 22: the share is of bytes, the boundary is unread.
        assert!(reading(&region(12..22), source).is_ok());
        assert_eq!(
            reading(&region(11..22), source),
            Err(
                "file did not parse: error regions cover 50% of it, first at line 2, column 1"
                    .into()
            )
        );
        // Overlapping regions count once; zero-width MISSING points count nothing.
        let errors = SyntaxErrors::from_ranges(vec![0..6, 3..10, 15..15, 20..20]);
        assert!(reading(&errors, source).is_ok());
        assert_eq!(
            reading(&region(0..22), source),
            Err(
                "file did not parse: error regions cover 100% of it, first at line 1, column 1"
                    .into()
            )
        );
        // An empty file's MISSING point covers nothing of it.
        assert!(reading(&region(0..0), "").is_ok());
    }

    #[test]
    fn a_grammar_that_gave_up_leaves_the_file_unread() {
        // tree-sitter-javascript recovers the leading statement, then reads the prose as one
        // ERROR to the end: 95% of the bytes.
        let source = format!(
            "debugger;\n{}",
            "Install it: run the script, then open the page.\n".repeat(5)
        );
        let errors = syntax_errors(GrammarDialect::JavaScript, &source);
        assert_eq!(
            reading(&errors, &source),
            Err(
                "file did not parse: error regions cover 95% of it, first at line 2, column 1"
                    .into()
            )
        );
        let partial = "debugger; // clean text keeps the error region under half the file\nfunction f( {\n  debugger;\n";
        assert!(reading(&syntax_errors(GrammarDialect::TypeScript, partial), partial).is_ok());
    }

    #[test]
    fn missing_tokens_suppress_only_the_witnesses_that_contain_them() {
        // An unclosed body parses with a zero-width MISSING `}` after its last statement and no
        // ERROR node. A MISSING child sits at its parent's end, so a witness that merely abuts
        // the point is suppressed too (`x = 1;` here); the earlier `debugger;` is clean.
        let source = "function f() {\n  debugger;\n  x = 1;\n";
        let errors = syntax_errors(GrammarDialect::TypeScript, source);
        let point = source.len() - 1;
        assert_eq!(errors.regions.len(), 1);
        assert_eq!(errors.regions[0], point..point);
        let debugger = compile_typescript_rule(json!({ "kind": "debugger_statement" }))
            .expect("rule compiles");
        let result = evaluate_file(&debugger, "typescript", "open.ts", source);
        assert_eq!(result["violations"][0]["span"]["startLine"], 1, "{result}");
        assert_eq!(result["failures"][0]["code"], "source-partial", "{result}");
        let function = compile_typescript_rule(json!({ "kind": "function_declaration" }))
            .expect("rule compiles");
        let result = evaluate_file(&function, "typescript", "open.ts", source);
        assert_eq!(
            result["violations"],
            json!([]),
            "the declaration spans the MISSING token"
        );
        assert_eq!(result["failures"][0]["code"], "source-partial", "{result}");
    }

    /// Each document of a file has its own syntax errors (`embed`): a program apart from its
    /// host's tree (tests' HTML scripts) has none of the host's, and hides no binding of its own;
    /// a program with errors of its own has them as ranges of the file; a program read through the
    /// host's errors (Blade's compiled view) has the template's too.
    #[test]
    fn each_document_has_its_own_errors_as_ranges_of_the_file() {
        use crate::code_rule_dsl::embed::test_script::html;
        use std::sync::Arc;
        let of = |source: &str| {
            let doc = html(source);
            let host = doc.root().get_doc().clone();
            let guest = host.guests().expect("owned")[0].doc.clone();
            (SyntaxErrors::of_doc(&host), SyntaxErrors::of_doc(&guest))
        };

        let (host, clean) = of("<script>let x = 1;</script>\n<div <p\n");
        assert!(!host.is_empty(), "the host has a syntax error");
        assert!(clean.is_empty());
        assert!(!clean.hide_bindings());

        let source = "<script>let = ;</script>\n<div <p\n";
        let (_, broken) = of(source);
        assert!(broken.hide_bindings());
        let script = source.find("let").unwrap()..source.find("</script>").unwrap();
        assert!(broken.overlaps(&script));
        assert!(!broken.overlaps(&(source.len() - 3..source.len())));

        let grammar = crate::code_rule_dsl::tape::Grammar::new(GrammarDialect::Blade).unwrap();
        let source = "{{ $a + }}\n{{-- open\n";
        let image =
            crate::code_rule_dsl::tape::TapeImage::encode_template(&grammar, source, &|| false)
                .unwrap();
        let blade =
            TapeDoc::restore(Arc::new(source.to_owned()), grammar, &image, &|| false).unwrap();
        let template = SyntaxErrors::of_doc(&blade);
        let view = SyntaxErrors::of_doc(&blade.guests().unwrap()[0].doc);
        let comment = source.find("{{--").unwrap();
        assert!(template.overlaps(&(comment..source.len())));
        assert!(!template.overlaps(&(0..10)), "the echo's PHP is the view's");
        assert!(view.overlaps(&(0..10)), "the view's own error");
        assert!(view.overlaps(&(comment..source.len())), "the template's");
        assert!(view.hide_bindings());
    }

    #[test]
    fn syntax_error_regions_merge_and_treat_missing_as_a_point() {
        let errors = SyntaxErrors {
            regions: vec![4..4, 10..20],
        };
        assert!(
            errors.overlaps(&(0..4)),
            "a range ending at a MISSING point touches it"
        );
        assert!(
            errors.overlaps(&(4..8)),
            "a range starting at a MISSING point touches it"
        );
        assert!(
            !errors.overlaps(&(5..10)),
            "ranges are half-open for real regions"
        );
        assert!(
            errors.overlaps(&(12..12)),
            "an empty witness inside a region overlaps"
        );
        assert!(!errors.overlaps(&(20..20)));
        assert!(!errors.overlaps(&(20..30)));
        assert!(errors.overlaps(&(0..100)));
    }

    #[test]
    fn negated_relations_leaving_the_witness_are_unsound_under_errors() {
        let sound = |rule: Value| {
            let ClauseKind::Forbid(Query::Ast { matchers, .. }) = &compile_typescript_rule(rule)
                .expect("rule compiles")
                .inner
                .clauses[0]
                .kind
            else {
                unreachable!()
            };
            matchers.iter().all(|grammar| grammar.sound_under_errors)
        };
        assert!(sound(json!({ "kind": "debugger_statement" })));
        assert!(sound(
            json!({ "kind": "program", "not": { "has": { "kind": "class_declaration", "stopBy": "end" } } })
        ));
        assert!(sound(
            json!({ "kind": "debugger_statement", "inside": { "kind": "function_declaration", "stopBy": "end" } })
        ));
        assert!(sound(
            json!({ "kind": "debugger_statement", "follows": { "kind": "import_statement" } })
        ));
        assert!(!sound(
            json!({ "kind": "debugger_statement", "not": { "inside": { "kind": "function_declaration" } } })
        ));
        assert!(!sound(
            json!({ "kind": "debugger_statement", "not": { "any": [{ "precedes": { "kind": "return_statement" } }] } })
        ));
        // `stopBy: end` walks the same parent's siblings, so a positive claim stays sound.
        assert!(sound(
            json!({ "kind": "debugger_statement", "follows": { "kind": "import_statement", "stopBy": "end" } })
        ));
        assert!(sound(
            json!({ "kind": "debugger_statement", "precedes": { "kind": "return_statement", "stopBy": "end" } })
        ));
        assert!(!sound(
            json!({ "kind": "debugger_statement", "not": { "follows": { "kind": "import_statement", "stopBy": "end" } } })
        ));
        assert!(!sound(
            json!({ "kind": "debugger_statement", "not": { "precedes": { "kind": "return_statement" } } })
        ));
        assert!(!sound(
            json!({ "kind": "debugger_statement", "inside": { "kind": "function_declaration", "not": { "has": { "kind": "return_statement" } } } })
        ));
    }

    #[test]
    fn never_matching_sub_rules_stay_sound_under_errors() {
        // In the plain dialect the JSX sub-rule compiles to `Never`; it has no relations, so the
        // rule stays sound and witnesses outside the error region are still reported.
        let compiled = compile_typescript_rule(json!({
            "kind": "debugger_statement",
            "not": { "has": { "kind": "jsx_expression", "stopBy": "end" } },
        }))
        .unwrap_or_else(|error| panic!("{}", error.reason));
        let ClauseKind::Forbid(Query::Ast { matchers, .. }) = &compiled.inner.clauses[0].kind
        else {
            unreachable!()
        };
        assert!(matchers.iter().all(|grammar| grammar.sound_under_errors));
        let result = evaluate_file(
            &compiled,
            "typescript",
            "open.ts",
            "debugger; // clean text keeps the error region under half the file\nfunction f( {\n",
        );
        assert_eq!(result["violations"][0]["span"]["startLine"], 0, "{result}");
        assert_eq!(result["failures"][0]["code"], "source-partial", "{result}");

        // A rule that is `Never` as a whole in the plain dialect has no matcher there at all.
        let compiled = compile_typescript_rule(json!({ "kind": "jsx_attribute" }))
            .unwrap_or_else(|error| panic!("{}", error.reason));
        let result = evaluate_file(
            &compiled,
            "typescript",
            "open.ts",
            "debugger; // clean text keeps the error region under half the file\nfunction f( {\n",
        );
        assert_eq!(result["violations"], json!([]), "{result}");
    }

    #[test]
    fn stateless_evaluation_reports_witnesses_outside_error_regions() {
        let compiled = compile_typescript_rule(json!({ "kind": "debugger_statement" }))
            .expect("rule compiles");
        let result = evaluate_file(
            &compiled,
            "typescript",
            "api.test.ts",
            // A missing comma, which TypeScript rejects too: the ERROR stays whatever the grammar fixes.
            "const m = importOriginal(1 load(\"./api\"));\ndebugger;\n",
        );
        assert_eq!(result["status"], "failed", "{result}");
        assert_eq!(
            result["violations"].as_array().map(Vec::len),
            Some(1),
            "{result}"
        );
        assert_eq!(result["violations"][0]["span"]["startLine"], 1, "{result}");
        assert_eq!(
            result["failures"].as_array().map(Vec::len),
            Some(1),
            "{result}"
        );
        assert_eq!(result["failures"][0]["code"], "source-partial", "{result}");
        assert_eq!(result["failures"][0]["path"], "api.test.ts", "{result}");

        let unsound = compile_typescript_rule(json!({
            "kind": "debugger_statement",
            "not": { "follows": { "kind": "import_statement", "stopBy": "end" } },
        }))
        .expect("rule compiles");
        let result = evaluate_file(
            &unsound,
            "typescript",
            "api.test.ts",
            "const m = importOriginal(1 load(\"./api\"));\ndebugger;\n",
        );
        assert_eq!(result["violations"], json!([]), "{result}");
        assert_eq!(
            result["failures"][0]["code"], "source-incomplete",
            "{result}"
        );
    }

    #[test]
    fn stop_rules_are_absence_claims_under_errors() {
        let sound = |rule: Value| {
            grammar_matchers(&compile_typescript_rule(rule).expect("rule compiles"))
                .iter()
                .all(|grammar| grammar.sound_under_errors)
        };
        // Inside the witness a stop rule reads cleanly parsed nodes only.
        assert!(sound(json!({
            "kind": "function_declaration",
            "has": { "kind": "return_statement", "stopBy": { "kind": "function_declaration" } },
        })));
        // Walking ancestors or siblings past nodes that did not stop claims they are absent.
        for relation in ["inside", "precedes", "follows"] {
            assert!(!sound(json!({
                "kind": "identifier",
                relation: { "kind": "class_declaration", "stopBy": { "kind": "function_declaration" } },
            })));
        }
        // A stop rule under `has` that itself leaves the node it tests is unsound too.
        assert!(!sound(json!({
            "kind": "function_declaration",
            "has": { "kind": "return_statement", "stopBy": { "inside": { "kind": "class_body" } } },
        })));
    }

    #[test]
    fn utils_and_constraints_are_as_sound_under_errors_as_what_they_reach() {
        let sound = |ast: Value| {
            grammar_matchers(&compiled(ast))
                .iter()
                .all(|grammar| grammar.sound_under_errors)
        };
        // Recursion that stays inside the witness, and positive relations, stay sound.
        assert!(sound(json!({
            "rule": { "kind": "return_statement", "has": { "matches": "tail" } },
            "utils": { "tail": { "any": [{ "kind": "await_expression" }, { "kind": "parenthesized_expression", "has": { "matches": "tail" } }] } },
        })));
        assert!(sound(json!({
            "rule": { "kind": "identifier", "matches": "in-loop" },
            "utils": { "in-loop": { "inside": { "stopBy": "end", "kind": "for_statement" } } },
        })));
        // A negated relation leaving the witness is unsound wherever it hides, including behind a
        // reference, a recursive one, or a reference reached under `not`.
        assert!(!sound(json!({
            "rule": { "kind": "identifier", "matches": "top-level" },
            "utils": { "top-level": { "not": { "inside": { "stopBy": "end", "kind": "function_declaration" } } } },
        })));
        assert!(!sound(json!({
            "rule": { "kind": "identifier", "not": { "matches": "in-function" } },
            "utils": { "in-function": { "inside": { "stopBy": "end", "kind": "function_declaration" } } },
        })));
        assert!(!sound(json!({
            "rule": { "kind": "identifier", "matches": "walk" },
            "utils": { "walk": { "any": [
                { "kind": "program" },
                { "inside": { "matches": "walk" }, "not": { "follows": { "kind": "comment" } } },
            ] } },
        })));
        // A constraint's negation reads the captured node: fine inside the witness, not outside it.
        assert!(sound(json!({
            "rule": { "pattern": "$F($$$)" },
            "constraints": { "F": { "not": { "has": { "kind": "this" } } } },
        })));
        assert!(!sound(json!({
            "rule": { "kind": "return_statement", "follows": { "pattern": "$PREV" } },
            "constraints": { "PREV": { "not": { "has": { "stopBy": "end", "kind": "call_expression" } } } },
        })));
    }

    #[test]
    fn reading_a_capture_an_outward_relation_may_leave_unbound_is_unsound_under_errors() {
        // Review finding 3 on #2561. `any` tolerates the outward relation failing, and it fails
        // when an ERROR sibling swallowed the earlier node: `$PREV` is then unbound, so the negated
        // comparison holds vacuously and so does a constraint on `$PREV` (unbound captures are
        // skipped). The equivalent `not: { follows }` is already unsound.
        let optional_prev = json!([
            { "follows": { "kind": "expression_statement", "pattern": "$PREV" } },
            { "regex": "." },
        ]);
        let differs = json!({
            "kind": "expression_statement",
            "pattern": "$CUR",
            "any": optional_prev,
            "not": { "compare": { "left": "PREV", "op": "eq", "right": "CUR" } },
        });
        let compiled = compile_typescript_rule(differs.clone()).expect("rule compiles");
        assert!(
            !grammar_matchers(&compiled)
                .iter()
                .any(|grammar| grammar.sound_under_errors),
            "a negated comparison of a possibly unbound outward capture"
        );
        let source = "f();\nf(;\nf();\n";
        let result = evaluate_file(&compiled, "typescript", "a.ts", source);
        assert_eq!(result["violations"], json!([]), "{result}");
        assert_eq!(
            result["failures"][0]["code"], "source-incomplete",
            "{result}"
        );
        let constrained = compiled_sound(json!({
            "rule": { "kind": "expression_statement", "any": optional_prev },
            "constraints": { "PREV": { "regex": "^g" } },
        }));
        assert!(
            !constrained,
            "a constraint skipped when the outward capture is unbound"
        );
        // Without `any` the relation must hold for the witness to exist, so the capture is bound.
        assert!(compiled_sound(json!({
            "rule": { "kind": "expression_statement", "pattern": "$CUR", "all": [
                { "follows": { "kind": "expression_statement", "pattern": "$PREV" } },
                { "not": { "compare": { "left": "PREV", "op": "eq", "right": "CUR" } } },
            ] },
        })));
        assert!(compiled_sound(json!({
            "rule": { "kind": "expression_statement", "follows": { "kind": "expression_statement", "pattern": "$PREV" } },
            "constraints": { "PREV": { "regex": "^g" } },
        })));
        // A capture made inside the witness is bound or not by clean source alone.
        assert!(compiled_sound(json!({
            "rule": { "kind": "call_expression", "pattern": "$F($$$)", "any": [{ "has": { "kind": "arguments", "pattern": "$ARGS" } }, { "regex": "." }],
                "not": { "compare": { "left": "F", "op": "eq", "right": "ARGS" } } },
            "constraints": { "ARGS": { "regex": "^\\(" } },
        })));
        // Positive comparisons stay sound: an unbound side makes them fail, a false negative.
        assert!(compiled_sound(json!({
            "rule": { "kind": "expression_statement", "pattern": "$CUR", "any": optional_prev,
                "compare": { "left": "PREV", "op": "eq", "right": "CUR" } },
        })));
    }

    #[test]
    fn parameterized_calls_track_optional_outward_captures_per_mapping() {
        let optional_previous = json!({
            "params": ["P"],
            "rule": { "any": [
                { "follows": { "kind": "expression_statement", "pattern": "$P" } },
                { "regex": "." }
            ] }
        });
        let rule = json!({
            "rule": {
                "pattern": "$C",
                "all": [
                    { "matches": { "previous": { "P": "$A" } } },
                    { "matches": { "previous": { "P": "$B" } } },
                    { "not": { "compare": { "left": "B", "op": "eq", "right": "C" } } }
                ]
            },
            "utils": { "previous": optional_previous }
        });
        let compiled = compile_typescript_ast(rule).expect("rule compiles");
        assert!(
            !grammar_matchers(&compiled)
                .iter()
                .all(|grammar| grammar.sound_under_errors),
            "each caller capture can be unbound when the outward relation is swallowed by syntax errors"
        );
        let result = evaluate_file(&compiled, "typescript", "a.ts", "a();\na(;\na();\n");
        assert_eq!(result["violations"], json!([]), "{result}");
        assert_eq!(
            result["failures"][0]["code"], "source-incomplete",
            "{result}"
        );
    }

    #[test]
    fn utility_capture_analysis_memoizes_completed_diamond_paths() {
        let mut utils = serde_json::Map::new();
        utils.insert("u0".to_owned(), json!({ "kind": "identifier" }));
        for depth in 1..=8 {
            let prior = format!("u{}", depth - 1);
            utils.insert(
                format!("u{depth}"),
                json!({ "all": [{ "matches": prior }, { "matches": prior }] }),
            );
        }
        reset_walks();
        let _ = compiled(json!({
            "rule": { "matches": "u8" },
            "utils": utils,
        }));
        assert!(
            walks(Walk::OptionalCaptures) <= 36,
            "a completed utility/context must be analyzed once, not once per diamond path"
        );
    }

    /// A parameterized call's summary is its utility's, in the utility's own names, whatever the
    /// mapping: each utility is analysed a constant number of times, not once per call path.
    #[test]
    fn parameterized_utility_diamonds_are_analysed_in_linear_visits() {
        let compiled = compiled_parameterized_utility_diamonds(DIAMOND_DEPTH);
        let matcher = &grammar_matchers(&compiled)[0].matcher;
        reset_walks();
        sound_under_errors(matcher);
        let bound = linear_walk(DIAMOND_DEPTH);
        assert!(
            walks(Walk::OptionalCaptures) <= bound,
            "optional capture analysis expanded {} utilities (bound {bound})",
            walks(Walk::OptionalCaptures)
        );
        assert!(
            walks(Walk::SoundVerdict) <= bound,
            "the soundness verdict expanded {} utilities (bound {bound})",
            walks(Walk::SoundVerdict)
        );
    }

    #[test]
    fn soundness_verdict_memoizes_completed_diamond_paths() {
        let mut utils = serde_json::Map::new();
        utils.insert("u0".to_owned(), json!({ "kind": "identifier" }));
        for depth in 1..=16 {
            let prior = format!("u{}", depth - 1);
            utils.insert(
                format!("u{depth}"),
                json!({ "all": [{ "matches": prior }, { "matches": prior }] }),
            );
        }
        reset_walks();
        let _ = compiled(json!({
            "rule": { "matches": "u16" },
            "utils": utils,
        }));
        assert!(
            walks(Walk::SoundVerdict) <= 64,
            "a completed utility/context must be checked once, not once per diamond path"
        );
    }

    #[test]
    fn counts_are_as_sound_under_errors_as_the_negated_relation() {
        let sound = |rule: Value| {
            grammar_matchers(&compile_typescript_rule(rule).expect("rule compiles"))
                .iter()
                .all(|grammar| grammar.sound_under_errors)
        };
        assert!(sound(json!({
            "kind": "function_declaration",
            "count": { "has": { "kind": "if_statement", "stopBy": { "kind": "arrow_function" } }, "min": 2 },
        })));
        for relation in ["inside", "precedes", "follows"] {
            assert!(!sound(json!({
                "kind": "if_statement",
                "count": { relation: { "kind": "if_statement", "stopBy": "end" }, "min": 2 },
            })));
        }
    }

    #[test]
    fn utility_diamonds_compile_in_linear_utility_visits() {
        let compilers: [fn(usize) -> crate::code_rule_dsl::CompiledCodeRuleDsl; 2] = [
            compiled_utility_diamonds,
            compiled_parameterized_utility_diamonds,
        ];
        for compile in compilers {
            reset_walks();
            compile(30);
            assert!(
                walks(Walk::OptionalCaptures) <= 128,
                "optional capture analysis visited {} utility contexts",
                walks(Walk::OptionalCaptures)
            );
            assert!(
                walks(Walk::SoundVerdict) <= 128,
                "soundness visited {} utility contexts",
                walks(Walk::SoundVerdict)
            );
        }
    }

    /// A capture bound through a cycle of parameterized calls under an outward relation under
    /// `any` is optional whichever call a walk summarizes first: a summary computed while the other
    /// was still being summarized holds only part of it, and must not stand for the whole.
    #[test]
    fn optional_captures_through_recursive_calls_do_not_depend_on_branch_order() {
        for branches in recursive_call_branch_orders() {
            let mut utils = mutually_recursive_calls();
            utils.insert("bindX".to_owned(), json!({ "pattern": "$X" }));
            let sound = compiled_sound(json!({
                "rule": { "kind": "expression_statement", "any": branches },
                "constraints": { "X": { "regex": "^g" } },
                "utils": utils,
            }));
            assert!(
                !sound,
                "a constraint on `$X`, which `W` may leave unbound, is skipped under errors ({branches})"
            );
        }
    }

    /// A self-recursive call binding its parameter through an outward relation under `any`: the
    /// negated comparison of what the inner call binds reads an optional capture, also when the
    /// root's call sits under `any` itself (the inner call's context).
    #[test]
    fn a_self_recursive_calls_optional_capture_is_unsound_wherever_it_is_called() {
        let utils = json!({
            "W": { "params": ["P"], "rule": { "pattern": "$N", "any": [
                { "follows": { "kind": "expression_statement", "pattern": "$P" } },
                { "has": { "all": [
                    { "matches": { "W": { "P": "$Q" } } },
                    { "not": { "compare": { "left": "Q", "op": "eq", "right": "N" } } }
                ] } }
            ] } },
        });
        for rule in [
            json!({ "kind": "expression_statement", "matches": { "W": { "P": "$X" } } }),
            json!({ "kind": "expression_statement", "any": [
                { "matches": { "W": { "P": "$X" } } }, { "kind": "identifier" } ] }),
        ] {
            assert!(
                !compiled_sound(json!({ "rule": rule, "utils": utils.clone() })),
                "{rule}"
            );
        }
    }
}
