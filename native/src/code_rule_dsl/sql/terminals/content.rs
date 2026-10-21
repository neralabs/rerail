//! Terminals by predicate content: the canonical form a terminal is interned by
//! (`lower::Identity`), so that equal predicates anywhere in a program (other clauses, other
//! policies, other compilations of one utility) are one terminal, and the per-batch caches keyed
//! by terminal (`Batch::fresh`, the fact terms' `FactMemo`) answer each (predicate, input) once
//! per batch.
//!
//! A canonical form names the predicate exactly: two predicates get the same form only where
//! their native evaluation at every node, with every environment, is provably identical. Where
//! that cannot be shown from the compiled predicate, there is no form (`None`) and the terminal
//! stays keyed by its matcher's address, as before.
//!
//! - **Pattern** (`pattern`): the dialect, the strictness, the root kinds and the whole pattern
//!   tree, every node with its kind id, namedness and text, every metavariable as it is matched.
//!   That is everything `Pattern::match_node_with_env` reads but its private `root_kind`, which
//!   only a contextual pattern sets: under `Smart` strictness (the only one a rule compiles to)
//!   a structural root's kind is tested exactly by the tree match itself, unless it is an error
//!   kind (`has_error`, no form), and a metavariable root's is `potential_kinds`. Other
//!   strictnesses have no form. A pattern term's key adds the captures an env-input one reads
//!   (`PatternTerm::reads`), whose values its keys carry.
//! - **Regex**: its source (`Regex::as_str`): rules build every regex with `Regex::new`, whose
//!   matching is a function of the source alone. Node text does not depend on the dialect.
//! - **Fact leaf** (`leaf`, `Verdicts::content`): the leaf's own form; a fact term's key adds the
//!   captures its keys carry, by name and kind, in key order (`FactTerm::reads`). Only leaves
//!   that bind nothing are shared.
//! - **Program**: pushdown keys each program by its rule's canonical form already
//!   (`pushdown::term`).

use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::matcher::{
    AstMatcher, Bound, Collation, CompareOp, CompareText, Comparison, Reading, Semantic,
    SemanticText, ValueTest,
};
use crate::code_rule_dsl::value::text_regex::TextRegex;
use ast_grep_core::MatchStrictness;
use ast_grep_core::matcher::{Matcher, Pattern, PatternNode};
use std::fmt::Write;

/// `pattern`'s canonical form in `dialect`, where it has one (the module doc).
pub(in crate::code_rule_dsl::sql) fn pattern(
    pattern: &Pattern,
    dialect: GrammarDialect,
) -> Option<String> {
    // Only `Smart`: under another strictness a contextual pattern's private root kind may
    // decide a match the tree does not show.
    if !matches!(pattern.strictness, MatchStrictness::Smart) || pattern.has_error() {
        return None;
    }
    let mut form = format!("{dialect}:smart:");
    match pattern.potential_kinds() {
        Some(kinds) => {
            for kind in &kinds {
                let _ = write!(form, "{kind},");
            }
        }
        None => form.push('*'),
    }
    form.push(':');
    tree(&pattern.node, &mut form);
    Some(form)
}

/// A pattern node, unambiguously: texts are length-prefixed and children bracketed.
fn tree(node: &PatternNode, form: &mut String) {
    match node {
        PatternNode::MetaVar { meta_var } => {
            let _ = write!(form, "${meta_var:?};");
        }
        PatternNode::Terminal {
            text,
            is_named,
            kind_id,
        } => {
            let _ = write!(
                form,
                "t{kind_id}{}{}:{text}",
                u8::from(*is_named),
                text.len()
            );
        }
        PatternNode::Internal { kind_id, children } => {
            let _ = write!(form, "i{kind_id}(");
            for child in children {
                tree(child, form);
            }
            form.push(')');
        }
    }
}

/// A fact leaf's canonical form (`terminals::Leaf`), where it has one:
///
/// - `layout` and `calls` (without `chain`) by their complete `Debug` forms, which hold every
///   field their evaluation reads. A `calls` term's `key` is the digest of its inner rule and
///   seed with the query's utilities and language options (`calls::CallsCollector`), and the
///   closure it reads is keyed by it.
/// - `compare`: its sides, operator, collation, text mode, reading, commuted operators and its
///   query's `transform` by address (a transform is not compared by content).
/// - `value`: its reading and every predicate's operands.
///
/// A reading (`Reading`) is its dialect, whether it follows bindings or reads a parsed literal,
/// and its query's language options by their `Debug` forms; its expression kinds follow from
/// the dialect and `literal`, and its cache identity names cache entries only. `resolves` has no
/// form.
pub(in crate::code_rule_dsl::sql) fn leaf(leaf: &AstMatcher) -> Option<String> {
    match leaf {
        AstMatcher::Semantic(Semantic::Layout(layout)) => Some(format!("layout:{layout:?}")),
        AstMatcher::Semantic(Semantic::Calls(test)) if test.chain.is_none() => {
            Some(format!("calls:{test:?}"))
        }
        AstMatcher::Compare(comparison) => Some(compare(comparison)),
        AstMatcher::Semantic(Semantic::Value(test)) => Some(value(test)),
        // Unknown where the node's text matches, true elsewhere: its regex alone.
        #[cfg(test)]
        AstMatcher::TestUnknown(regex) => Some(format!("testUnknown:{}", regex.as_str())),
        _ => None,
    }
}

fn compare(comparison: &Comparison) -> String {
    let Comparison {
        left,
        op,
        right,
        collation,
        text,
        reading: read,
        transform,
        commute,
    } = comparison;
    let op = match op {
        CompareOp::Lt => "lt",
        CompareOp::Le => "le",
        CompareOp::Eq => "eq",
        CompareOp::Ne => "ne",
        CompareOp::Gt => "gt",
        CompareOp::Ge => "ge",
    };
    let collation = match collation {
        Collation::Ordinal => "ordinal",
        Collation::FoldLower => "fold-lower",
        Collation::FoldUpper => "fold-upper",
        Collation::LowercaseFirst => "lowercase-first",
        Collation::Natural => "natural",
        Collation::NaturalFoldLower => "natural-fold-lower",
    };
    let text = match text {
        CompareText::Source => "source".to_owned(),
        CompareText::Unquoted => "unquoted".to_owned(),
        CompareText::Tokens => "tokens".to_owned(),
        CompareText::Node => "node".to_owned(),
        CompareText::Index => "index".to_owned(),
        CompareText::Semantic(semantic) => match semantic {
            SemanticText::Value => "value".to_owned(),
            SemanticText::Key => "key".to_owned(),
            SemanticText::Structure(equality) => format!("structure-{equality:?}"),
            SemanticText::Member => "member".to_owned(),
            SemanticText::Name => "name".to_owned(),
            SemanticText::Reference { static_keys } => format!("reference-{static_keys}"),
        },
    };
    let read = read.as_ref().map(reading);
    let transform = transform.as_ref().map(std::sync::Arc::as_ptr);
    format!("compare:{left:?} {op} {right:?}:{collation}:{text}:{read:?}:{transform:?}:{commute:?}")
}

fn value(test: &ValueTest) -> String {
    let ValueTest {
        reading: read,
        eq,
        types,
        constant,
        starts_with,
        regex,
        bits_any,
        bounds,
        truthy,
    } = test;
    let regex = regex.as_ref().map(|regex| match regex {
        TextRegex::Plain(regex) => format!("plain:{}", regex.as_str()),
        TextRegex::Backreferences(regex) => format!("backreferences:{}", regex.as_str()),
    });
    let bounds: Vec<(&str, &crate::code_rule_dsl::value::Value)> = bounds
        .iter()
        .map(|(bound, value)| {
            let bound = match bound {
                Bound::Gt => "gt",
                Bound::Ge => "ge",
                Bound::Lt => "lt",
                Bound::Le => "le",
            };
            (bound, value)
        })
        .collect();
    format!(
        "value:{}:{eq:?}:{types:?}:{constant:?}:{starts_with:?}:{regex:?}:{bits_any:?}:{bounds:?}:{truthy:?}",
        reading(read)
    )
}

fn reading(reading: &Reading) -> String {
    format!(
        "{}:follow={}:literal={}:{:?}:{:?}",
        reading.language, reading.follow, reading.literal, reading.options, reading.python
    )
}
