//! Text a file, or a node, must contain for a rule to match there, read from the rule's terms.
//!
//! Every node a rule tests is a node of the file, and its text is a slice of the file's source: a
//! `regex` matches only where the node's text holds one of the literals each of its matches begins
//! with, and a pattern's named leaf only where the node's text holds that leaf's text. A rule needs
//! what its conjunctions and relations need (a related node is a node of the file too; a node's
//! descendants are slices of its own text); an `any` needs one of what its branches need; a
//! negation, a count, a comparison and Rerail's predicates (whose sub-rules may read parsed
//! literals rather than the file) need nothing. A file or node without the text cannot match the
//! rule, so it need not be tested at all.

use super::super::{AstMatcher, UtilRef};
use ast_grep_core::MatchStrictness;
use ast_grep_core::matcher::PatternNode;
use regex_syntax::hir::literal::Extractor;
use std::collections::{BTreeMap, BTreeSet};

/// Clauses a file's text must all satisfy, each by holding at least one of its literals.
#[derive(Debug, PartialEq)]
pub(in crate::code_rule_dsl) struct RequiredText {
    clauses: Vec<Vec<Box<str>>>,
}

/// Literals of an `any`'s clause, beyond which the clause is dropped: a long disjunction rarely
/// rules a file out and costs a search per literal.
const MAX_ALTERNATIVES: usize = 16;

/// Clauses one requirement keeps, the most selective first.
const MAX_CLAUSES: usize = 8;

impl RequiredText {
    /// What a file must contain for `matcher` to match in it, or `None` where nothing is required.
    pub(in crate::code_rule_dsl) fn of(matcher: &AstMatcher) -> Option<Self> {
        Self::from_clauses(needs(matcher, &mut Utilities::default(), Reach::File))
    }

    /// What a node's own text must contain for `matcher` to match at it: only what the node and
    /// its descendants are tested for counts, not what relations read elsewhere in the file.
    pub(in crate::code_rule_dsl::matcher) fn of_node(matcher: &AstMatcher) -> Option<Self> {
        Self::from_clauses(needs(matcher, &mut Utilities::default(), Reach::Node))
    }

    fn from_clauses(mut clauses: Vec<Clause>) -> Option<Self> {
        clauses.sort_by_key(|clause| std::cmp::Reverse(selectivity(clause)));
        clauses.dedup();
        clauses.truncate(MAX_CLAUSES);
        (!clauses.is_empty()).then_some(Self {
            clauses: clauses
                .into_iter()
                .map(|clause| clause.into_iter().map(String::into_boxed_str).collect())
                .collect(),
        })
    }

    /// Whether `text` satisfies every clause.
    pub(in crate::code_rule_dsl::matcher) fn holds(&self, text: &str) -> bool {
        self.holds_with(text, &mut |text, literal| {
            memchr::memmem::find(text.as_bytes(), literal.as_bytes()).is_some()
        })
    }

    /// `holds`, with `found` saying whether `text` holds a literal (a file's clauses share many,
    /// so a caller can remember each search).
    pub(in crate::code_rule_dsl) fn holds_with<'a>(
        &'a self,
        text: &str,
        found: &mut impl FnMut(&str, &'a str) -> bool,
    ) -> bool {
        self.clauses
            .iter()
            .all(|clause| clause.iter().any(|literal| found(text, literal)))
    }
}

type Clause = BTreeSet<String>;

/// How much a clause narrows the files that satisfy it: its shortest literal's length.
fn selectivity(clause: &Clause) -> usize {
    clause.iter().map(String::len).min().unwrap_or(0)
}

/// Where a requirement's text must be: anywhere in the file, or in the tested node's own text.
#[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
enum Reach {
    File,
    Node,
}

/// The utilities being read, whose references back need nothing, and those read already.
#[derive(Default)]
struct Utilities {
    visiting: Vec<(usize, usize)>,
    read: BTreeMap<(usize, usize), Vec<Clause>>,
}

/// The clauses `matcher` needs where it matches, within `reach`.
fn needs(matcher: &AstMatcher, utilities: &mut Utilities, reach: Reach) -> Vec<Clause> {
    match matcher {
        AstMatcher::Regex(regex) => regex_clause(regex.as_str()).into_iter().collect(),
        AstMatcher::Pattern(pattern) => {
            let mut clauses = Vec::new();
            if matches!(pattern.strictness, MatchStrictness::Smart) {
                pattern_leaves(&pattern.node, &mut clauses);
            }
            clauses
        }
        AstMatcher::All(items) => items
            .iter()
            .flat_map(|item| needs(item, utilities, reach))
            .collect(),
        AstMatcher::Any(items) => {
            let mut alternatives = Clause::new();
            for item in items {
                // A branch is answered by its most selective clause.
                let Some(best) = needs(item, utilities, reach)
                    .into_iter()
                    .max_by_key(selectivity)
                else {
                    return Vec::new();
                };
                alternatives.extend(best);
                if alternatives.len() > MAX_ALTERNATIVES {
                    return Vec::new();
                }
            }
            if alternatives.is_empty() {
                Vec::new()
            } else {
                vec![alternatives]
            }
        }
        // A descendant's text is a slice of the node's.
        AstMatcher::Has(item, _) => needs(item, utilities, reach),
        AstMatcher::Field(relation) if !relation.inside => needs(&relation.rule, utilities, reach),
        // Ancestors and siblings are elsewhere in the file.
        AstMatcher::Inside(item, _)
        | AstMatcher::Precedes(item, _)
        | AstMatcher::Follows(item, _)
            if reach == Reach::File =>
        {
            needs(item, utilities, reach)
        }
        AstMatcher::Field(relation) if reach == Reach::File => {
            needs(&relation.rule, utilities, reach)
        }
        AstMatcher::Constrained(core) => needs(&core.rule, utilities, reach),
        // A double negation matches where its rule does (requirements read two-valued rules).
        AstMatcher::Not(inner) => match &**inner {
            AstMatcher::Not(rule) => needs(rule, utilities, reach),
            _ => Vec::new(),
        },
        AstMatcher::Util(util) => util_needs(util, utilities, reach),
        AstMatcher::Call(call) => util_needs(&call.util, utilities, reach),
        _ => Vec::new(),
    }
}

fn util_needs(util: &UtilRef, utilities: &mut Utilities, reach: Reach) -> Vec<Clause> {
    let key = (util.key(), 0);
    if let Some(clauses) = utilities.read.get(&key) {
        return clauses.clone();
    }
    if utilities.visiting.contains(&key) {
        return Vec::new();
    }
    utilities.visiting.push(key);
    let clauses = util
        .with_target(|rule| needs(rule, utilities, reach))
        .unwrap_or_default();
    utilities.visiting.pop();
    // A reference back counted as needing nothing only weakens what a utility on a cycle needs,
    // which stays necessary: every read is remembered.
    utilities.read.insert(key, clauses.clone());
    clauses
}

/// The literals every match of `pattern` begins with, where there are finitely many and none is
/// empty.
fn regex_clause(pattern: &str) -> Option<Clause> {
    let hir = regex_syntax::parse(pattern).ok()?;
    let literals = Extractor::new().extract(&hir).literals()?.to_vec();
    if literals.is_empty() || literals.len() > MAX_ALTERNATIVES {
        return None;
    }
    literals
        .iter()
        .map(|literal| {
            let text = std::str::from_utf8(literal.as_bytes()).ok()?;
            (!text.is_empty()).then(|| text.to_owned())
        })
        .collect()
}

/// A clause of one literal for every named leaf of a pattern: a match holds each leaf's text.
fn pattern_leaves(node: &PatternNode, clauses: &mut Vec<Clause>) {
    match node {
        PatternNode::Terminal {
            text,
            is_named: true,
            ..
        } if !text.is_empty() => clauses.push(Clause::from([text.clone()])),
        PatternNode::Internal { children, .. } => {
            for child in children {
                pattern_leaves(child, clauses);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::RequiredText;
    use crate::code_rule_dsl::test_support::{compile_typescript_ast, grammar_matchers};
    use serde_json::{Value, json};

    fn file_text(ast: Value) -> Option<RequiredText> {
        let compiled =
            compile_typescript_ast(ast).unwrap_or_else(|error| panic!("{}", error.reason));
        RequiredText::of(&grammar_matchers(&compiled)[0].matcher)
    }

    fn node_text(ast: Value) -> Option<RequiredText> {
        let compiled =
            compile_typescript_ast(ast).unwrap_or_else(|error| panic!("{}", error.reason));
        RequiredText::of_node(&grammar_matchers(&compiled)[0].matcher)
    }

    #[test]
    fn a_file_needs_what_every_match_needs_through_relations_and_utilities() {
        let text = file_text(json!({
            "rule": {
                "kind": "call_expression",
                "inside": { "stopBy": "end", "regex": "^(alpha|beta)Handler$" },
                "has": { "stopBy": "end", "matches": "named" }
            },
            "utils": { "named": { "pattern": "gamma($A)" } }
        }))
        .expect("a requirement");
        assert!(text.holds("gamma(x); betaHandler"));
        assert!(
            !text.holds("gamma(x); deltaHandler"),
            "an alternative must hold"
        );
        assert!(!text.holds("alphaHandler"), "the utility's leaf must hold");
    }

    #[test]
    fn branches_negations_and_values_need_only_what_every_match_must_hold() {
        // A branch without a literal can match anywhere.
        assert_eq!(
            file_text(
                json!({ "rule": { "any": [{ "regex": "alpha" }, { "kind": "identifier" }] } })
            ),
            None
        );
        // A negation holds where its rule's text is missing.
        assert_eq!(
            file_text(json!({ "rule": { "not": { "regex": "alpha" } } })),
            None
        );
        // Two negations match where their rule does.
        let text = file_text(json!({ "rule": { "not": { "not": { "regex": "alpha" } } } }))
            .expect("a requirement");
        assert!(!text.holds("beta"));
        // Every match of a pattern of unbounded text begins with nothing in particular.
        assert_eq!(file_text(json!({ "rule": { "regex": "^.*$" } })), None);
    }

    #[test]
    fn a_node_needs_only_what_it_and_its_descendants_are_tested_for() {
        let text = node_text(json!({
            "rule": {
                "kind": "call_expression",
                "has": { "stopBy": "end", "regex": "^alpha$" },
                "inside": { "stopBy": "end", "regex": "beta" },
                "follows": { "regex": "gamma" }
            }
        }))
        .expect("a requirement");
        assert!(
            text.holds("alpha()"),
            "ancestors and siblings are elsewhere in the file"
        );
        assert!(!text.holds("delta()"));
    }

    #[test]
    fn recursive_utilities_need_what_their_cycle_reaches() {
        let text = file_text(json!({
            "rule": { "matches": "walk" },
            "utils": {
                "walk": { "any": [
                    { "regex": "^alpha$" },
                    { "kind": "parenthesized_expression", "has": { "matches": "walk" } }
                ] }
            }
        }));
        // The recursive branch reads its own reference back as needing nothing.
        assert_eq!(text, None);
    }
}
