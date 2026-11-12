mod semantic;

use crate::code_rule_dsl::tree::Tree;
use super::collate::natural_compare;
use super::exported::exported_text;
use super::transform::Transform;
use super::value::Reading;
use ast_grep_core::meta_var::MetaVarEnv;
use ast_grep_core::{Doc, Node};
pub(crate) use semantic::SemanticText;
use std::cmp::Ordering;
use std::sync::Arc;

/// `compare` (a Rerail extension): `left op right` over two single-node captures under a text mode
/// and a collation. It reads the bindings made so far, so it runs last in its rule object; both
/// captures must be bound or it does not match. It binds nothing. Inside a relation it is part
/// of the related node's rule, so the walk continues to the first node whose captures compare.
/// Over values or property names it is three-valued: where either side is not known, so is the
/// comparison.
#[derive(Clone)]
pub(crate) struct Comparison {
    pub(crate) left: String,
    pub(crate) op: CompareOp,
    pub(crate) right: String,
    pub(crate) collation: Collation,
    pub(crate) text: CompareText,
    /// How the semantic modes read the captures (`SemanticText`); set per grammar at compile
    /// time.
    pub(crate) reading: Option<Reading>,
    /// The query's `transform`, where a side names one of its variables: that side is its text.
    pub(crate) transform: Option<Arc<Transform>>,
    /// `tokens` only: binary operators whose operands compare in either order (ESLint
    /// no-dupe-else-if's `equal`); empty when none.
    pub(crate) commute: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CompareOp {
    Lt,
    Le,
    Eq,
    Ne,
    Gt,
    Ge,
}

/// How texts are ordered. Every collation compares UTF-16 code units after its fold, as
/// JavaScript's `<` does, so JavaScript linters' orderings port exactly.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Collation {
    Ordinal,
    /// `toLowerCase()` first.
    FoldLower,
    /// `toUpperCase()` first.
    FoldUpper,
    /// ASCII letters swap case first, so lowercase sorts before uppercase.
    LowercaseFirst,
    /// The natural-compare package's order (ESLint sort-keys `natural`): digit runs by number.
    Natural,
    /// `toLowerCase()` first, then natural (sort-keys `natural` without `caseSensitive`).
    NaturalFoldLower,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CompareText {
    /// The captured node's source text.
    Source,
    /// The source text without one pair of enclosing matching quotes (`'`, `"` or `` ` ``);
    /// escapes are not decoded.
    Unquoted,
    /// The leaf tokens, comments and white space aside (ESLint's `equalTokens`); `eq`/`ne` only.
    Tokens,
    /// Node identity: the two captures are the same node (range and kind); `eq`/`ne` only.
    Node,
    /// Each capture's index among the elements of the list that holds it (ESTree's element index):
    /// the commas before it in its parent, so holes count and comments do not. A node outside a list
    /// has none, and compares false.
    Index,
    /// What the captures mean rather than their text: values, names, keys, references and
    /// structures (`SemanticText`).
    Semantic(SemanticText),
}

/// What one side of an `eq` comparison reads as, independently of the other side
/// (`Comparison::eq_class`): two bound nodes compare unknown where either class is `Unknown`,
/// true where both are the same `Text`, and false otherwise.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum EqClass {
    /// Equal exactly to the nodes of the same text.
    Text(Vec<u16>),
    /// Equal to no node: a key or name it does not have.
    Never,
    /// Not known: the comparison with any node is unknown.
    Unknown,
}

impl CompareOp {
    fn test(self, order: Ordering) -> bool {
        match self {
            Self::Lt => order.is_lt(),
            Self::Le => order.is_le(),
            Self::Eq => order.is_eq(),
            Self::Ne => order.is_ne(),
            Self::Gt => order.is_gt(),
            Self::Ge => order.is_ge(),
        }
    }
}

impl Comparison {
    /// `Some(true)`, `Some(false)`, or `None` when a side's value or name is not known.
    pub(super) fn holds<D: Doc>(&self, env: &MetaVarEnv<'_, D>) -> Option<bool> {
        let (Some(left), Some(right)) = (env.get_match(&self.left), env.get_match(&self.right))
        else {
            if let CompareText::Semantic(text) = self.text
                && let Some(verdict) = semantic::unbound(self, text, env)
            {
                return verdict;
            }
            // A capture a parsed tree exported, or a transform variable, is text: compared as
            // text only. A transform of a value that is not known leaves the verdict unknown.
            let text = |name: &str| -> Option<Option<String>> {
                if let Some(text) = env
                    .get_match(name)
                    .map(|node| node.text().into_owned())
                    .or_else(|| exported_text(env, name))
                    .or_else(|| {
                        env.get_transformed(name).map(|bytes| {
                            <D::Source as ast_grep_core::source::Content>::encode_bytes(bytes)
                                .into_owned()
                        })
                    })
                {
                    return Some(Some(text));
                }
                match &self.transform {
                    Some(transform) => transform.text(name, env).ok(),
                    None => Some(None),
                }
            };
            let (left, right) = (text(&self.left)?, text(&self.right)?);
            return match (self.text, left, right) {
                (CompareText::Source | CompareText::Unquoted, Some(left), Some(right)) => {
                    let (left, right) = (self.key(&left), self.key(&right));
                    Some(self.op.test(self.folded_order(&left, &right)))
                }
                _ => Some(false),
            };
        };
        match self.text {
            CompareText::Source | CompareText::Unquoted => {
                let (left, right) = (self.key(&left.text()), self.key(&right.text()));
                Some(self.op.test(self.folded_order(&left, &right)))
            }
            CompareText::Tokens => {
                let equal = if self.commute.is_empty() {
                    tokens(left) == tokens(right)
                } else {
                    commuted_equal(left, right, &self.commute)
                };
                Some(if self.op == CompareOp::Ne {
                    !equal
                } else {
                    equal
                })
            }
            CompareText::Semantic(text) => semantic::holds(self, text, left, right),
            CompareText::Node => {
                let same = left.range() == right.range() && left.kind_id() == right.kind_id();
                Some(same != (self.op == CompareOp::Ne))
            }
            CompareText::Index => match (element_index(left), element_index(right)) {
                (Some(left), Some(right)) => Some(self.op.test(left.cmp(&right))),
                _ => Some(false),
            },
        }
    }

    /// Two strings' order under the collation, over UTF-16 code units after the fold.
    fn units_order(&self, left: &[u16], right: &[u16]) -> Ordering {
        match self.collation {
            Collation::Ordinal => left.cmp(right),
            Collation::Natural => natural_compare(left, right),
            _ => {
                let left = self.fold(&String::from_utf16_lossy(left));
                let right = self.fold(&String::from_utf16_lossy(right));
                self.folded_order(&left, &right)
            }
        }
    }

    /// The order of two texts already folded: natural, or by UTF-16 code unit.
    fn folded_order(&self, left: &str, right: &str) -> Ordering {
        match self.collation {
            Collation::Natural | Collation::NaturalFoldLower => natural_compare(
                &left.encode_utf16().collect::<Vec<_>>(),
                &right.encode_utf16().collect::<Vec<_>>(),
            ),
            _ => left.encode_utf16().cmp(right.encode_utf16()),
        }
    }

    fn key(&self, text: &str) -> String {
        let text = match self.text {
            CompareText::Unquoted => unquoted(text),
            _ => text,
        };
        self.fold(text)
    }

    fn fold(&self, text: &str) -> String {
        match self.collation {
            Collation::Ordinal | Collation::Natural => text.to_owned(),
            Collation::FoldLower | Collation::NaturalFoldLower => text.to_lowercase(),
            Collation::FoldUpper => text.to_uppercase(),
            Collation::LowercaseFirst => text
                .chars()
                .map(|c| {
                    if c.is_ascii_lowercase() {
                        c.to_ascii_uppercase()
                    } else {
                        c.to_ascii_lowercase()
                    }
                })
                .collect(),
        }
    }

    /// Whether `eq_class` decides this comparison between two bound nodes: `eq` under a text mode
    /// that reads each side alone, in a collation whose equality is that of folded texts (natural
    /// collation equates texts that differ, such as `01` and `1`).
    pub(crate) fn classes_decide(&self) -> bool {
        self.op == CompareOp::Eq
            && !matches!(
                self.collation,
                Collation::Natural | Collation::NaturalFoldLower
            )
            && matches!(
                self.text,
                CompareText::Source
                    | CompareText::Unquoted
                    | CompareText::Semantic(SemanticText::Key | SemanticText::Name)
            )
    }

    /// The class of a bound node under a comparison `classes_decide` holds for: `holds` of two
    /// bound nodes is what their classes say (`EqClass`).
    pub(crate) fn eq_class<D: Doc>(&self, node: &Node<'_, D>) -> EqClass {
        match self.text {
            CompareText::Source | CompareText::Unquoted => {
                EqClass::Text(self.key(&node.text()).encode_utf16().collect())
            }
            CompareText::Semantic(text) => match semantic::name(self, text, node) {
                None => EqClass::Unknown,
                Some(None) => EqClass::Never,
                Some(Some(units)) => EqClass::Text(self.folded_units(units)),
            },
            CompareText::Tokens | CompareText::Node | CompareText::Index => {
                unreachable!("`eq_class` of a comparison `classes_decide` refuses")
            }
        }
    }

    /// UTF-16 code units as `units_order` compares them for equality: as they are, or folded.
    fn folded_units(&self, units: Vec<u16>) -> Vec<u16> {
        match self.collation {
            Collation::Ordinal | Collation::Natural => units,
            _ => self
                .fold(&String::from_utf16_lossy(&units))
                .encode_utf16()
                .collect(),
        }
    }
}

fn unquoted(text: &str) -> &str {
    let bytes = text.as_bytes();
    match bytes {
        [first @ (b'\'' | b'"' | b'`'), .., last] if first == last => &text[1..text.len() - 1],
        _ => text,
    }
}

/// Lists whose elements are separated by commas, where the commas before an element count its
/// index: TypeScript's array literals and patterns, arguments, parameters, object literals and
/// patterns, type arguments and parameters, tuple types, and import and export lists; Python's
/// displays, argument lists, parameters, patterns and bare tuples.
const LISTS: &[&str] = &[
    "array",
    "array_pattern",
    "arguments",
    "formal_parameters",
    "object",
    "object_pattern",
    "type_arguments",
    "type_parameters",
    "tuple_type",
    "named_imports",
    "export_clause",
    "list",
    "tuple",
    "set",
    "dictionary",
    "argument_list",
    "parameters",
    "lambda_parameters",
    "list_pattern",
    "tuple_pattern",
    "pattern_list",
    "expression_list",
];

/// A list element's 0-based index: the `,` tokens before it among its parent's children.
/// Parentheses are transparent, as in ESTree and Python's AST: `(a)` in `[(a)]` is element 0.
fn element_index<D: Doc>(node: &Node<'_, D>) -> Option<usize> {
    if !node.is_named() || super::util::is_comment_kind(&node.kind()) {
        return None;
    }
    let (element, list) = outside_parentheses(node)?;
    if !LISTS.contains(&list.kind().as_ref()) {
        return None;
    }
    commas_before(&list, &element)
}

/// The outermost parentheses around `node` (or `node` itself, where none are), with its parent.
fn outside_parentheses<'t, D: Doc>(node: &Node<'t, D>) -> Option<(Node<'t, D>, Node<'t, D>)> {
    let mut node = node.clone();
    let mut parent = node.parent()?;
    while super::util::is_paren_kind(&parent.kind()) {
        node = parent;
        parent = node.parent()?;
    }
    Some((node, parent))
}

/// The `,` tokens among `list`'s children before its child `element`.
/// The commas before `element` in `list`: those a tape stores (a hole's, a leading one's), and the
/// one between two elements that none separates, which the grammar implies (RER-131).
fn commas_before<'r, D: Doc>(list: &Node<'r, D>, element: &Node<'r, D>) -> Option<usize> {
    let start = element.range().start;
    let (mut commas, mut after_element) = (0, false);
    for child in list.children().take_while(|child| child.range().end <= start) {
        if child.kind() == "," {
            commas += 1;
            after_element = false;
        } else if child.is_named() && !super::util::is_comment_kind(&child.kind()) {
            commas += usize::from(after_element);
            after_element = true;
        }
    }
    Some(commas + usize::from(after_element))
}

/// The node inside any parentheses: ESTree has none.
fn unparenthesized<'t, D: Doc>(node: &Node<'t, D>) -> Node<'t, D> {
    let mut node = node.clone();
    while super::util::is_paren_kind(&node.kind()) {
        let Some(inner) = node.first_code() else {
            break;
        };
        node = inner;
    }
    node
}

/// A binary expression's operands and operator, where the operator is listed.
fn listed_operands<'t, D: Doc>(
    node: &Node<'t, D>,
    commute: &[String],
) -> Option<(Node<'t, D>, String, Node<'t, D>)> {
    let operator = node.operator_token()?.text().into_owned();
    if !commute.contains(&operator) {
        return None;
    }
    Some((node.field("left")?, operator, node.field("right")?))
}

/// ESLint no-dupe-else-if's `equal`: two binary expressions with the same listed operator are
/// equal where their operands are, in order or swapped; anything else by its tokens. Each step
/// spends one of the file's utility steps, so a pathological pair fails the clause, not the host.
fn commuted_equal<D: Doc>(left: &Node<'_, D>, right: &Node<'_, D>, commute: &[String]) -> bool {
    if !super::spend_util_step() {
        return false;
    }
    let (left, right) = (unparenthesized(left), unparenthesized(right));
    match (
        listed_operands(&left, commute),
        listed_operands(&right, commute),
    ) {
        (Some((a, operator, b)), Some((c, other, d))) if operator == other => {
            (commuted_equal(&a, &c, commute) && commuted_equal(&b, &d, commute))
                || (commuted_equal(&a, &d, commute) && commuted_equal(&b, &c, commute))
        }
        _ => tokens(&left) == tokens(&right),
    }
}

/// A node's leaf tokens: the text of every leaf that is not a comment, and any other text between
/// leaves that is not white space. Comments and white space are not tokens.
pub(crate) fn tokens<D: Doc>(node: &Node<'_, D>) -> Vec<String> {
    let text = node.text();
    let start = node.range().start;
    let mut out = Vec::new();
    let mut at = 0;
    let gap = |from: usize, to: usize, out: &mut Vec<String>| {
        if let Some(between) = text.get(from..to) {
            out.extend(between.split_whitespace().map(str::to_owned));
        }
    };
    for leaf in node.dfs() {
        let comment = super::util::is_comment_kind(&leaf.kind());
        if !comment && leaf.children().next().is_some() {
            continue;
        }
        let range = leaf.range();
        let (from, to) = (range.start - start, range.end - start);
        if from < at {
            continue;
        }
        gap(at, from, &mut out);
        if !comment && to > from {
            out.push(text[from..to].to_owned());
        }
        at = to;
    }
    gap(at, text.len(), &mut out);
    out
}

#[cfg(test)]
mod tests;
