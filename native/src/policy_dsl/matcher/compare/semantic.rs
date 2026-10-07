use super::super::exported::exported_text;
use super::{Collation, CompareOp, Comparison};
use crate::policy_dsl::language::Family;
use crate::policy_dsl::value::ecma::ops;
use crate::policy_dsl::value::structure::Equality;
use crate::policy_dsl::value::{Lang, Value, key, php_compare};
use ast_grep_core::meta_var::MetaVarEnv;
use ast_grep_core::{Doc, Node};

/// The text modes that read what the captures mean rather than their text.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum SemanticText {
    /// The captured expressions' values, in the language's sense: Python `==` and order, or
    /// JavaScript `Object.is` and its relational operators' order (IsLessThan, converting
    /// primitives: `null <= 0`, `'1' < 2`). Strings compare under the collation.
    Value,
    /// The static property name of a key or of what holds one: TypeScript's as ESLint's
    /// `getStaticPropertyName` reads it; PHP's property, method, class constant, array key or
    /// named argument, a constant computed name folded (`value/php_compare.rs`). A key without
    /// one equals no key and has no order.
    Key,
    /// Python or PHP expressions or statements compared structurally: Ruff's equalities
    /// (`structure`, `structure-hash`, `structure-parts`), PHP-Parser's trees and PHP's array key
    /// identity (`structure`, `structure-hash`); `eq`/`ne` only.
    Structure(Equality),
    /// The name of a key or of what holds one, following bindings: as typescript-eslint's
    /// `getStaticMemberAccessValue` reads it (TypeScript): a non-computed identifier's or private
    /// name's (without `#`), or `String(v)` of what the key folds to; PHP's key with a computed
    /// name's variables followed. Unknown where that is not known.
    Member,
    /// What a node names: ESTree's name (TypeScript), an identifier's name, escapes decoded (a
    /// private name's without `#`), or a string literal's value; the name PHP looks a symbol up
    /// by, or a string literal's value. A node without one equals no name and has no order, as
    /// `key`'s nameless keys.
    Name,
    /// The same reference: ESLint's `isSameReference` (TypeScript), parentheses and optional
    /// chains aside, the same `this`, name, literal value or member access; with `static_keys`,
    /// `a.b` and `a['b']` are the same. PHP's variables, constants and access chains likewise
    /// (`value/php_compare.rs`). `eq`/`ne` only.
    Reference { static_keys: bool },
}

/// The verdict where a side is not a bound node: `Some` for the modes that read such a side
/// themselves, `None` to compare it as text.
pub(super) fn unbound<D: Doc>(
    comparison: &Comparison,
    text: SemanticText,
    env: &MetaVarEnv<'_, D>,
) -> Option<Option<bool>> {
    // `name` of a capture a parsed tree exported (a directive's item) is its text, as
    // written; of a node, the name it holds.
    if text != SemanticText::Name {
        return None;
    }
    let family = Family::of(comparison.reading.as_ref()?.language);
    // `None` for a node Blade wrote tokens into (`blade_spelled`): its name is not known.
    let name = |side: &str| -> Option<Option<Option<Vec<u16>>>> {
        match env.get_match(side) {
            Some(node) => Some(blade_spelled(family, node, node).then(|| node_name(family, node))),
            None => exported_text(env, side).map(|text| Some(Some(text.encode_utf16().collect()))),
        }
    };
    Some(match (name(&comparison.left), name(&comparison.right)) {
        (Some(None), Some(_)) | (Some(_), Some(None)) => None,
        (Some(Some(Some(left))), Some(Some(Some(right)))) => {
            Some(comparison.op.test(comparison.units_order(&left, &right)))
        }
        (Some(_), Some(_)) => match comparison.op {
            CompareOp::Eq => Some(false),
            CompareOp::Ne => Some(true),
            _ => None,
        },
        // An unbound side (or a transform variable, which is refused) compares false.
        _ => Some(false),
    })
}

/// What a node names, as its family reads names.
fn node_name<D: Doc>(family: Family, node: &Node<'_, D>) -> Option<Vec<u16>> {
    match family {
        Family::Ecma => key::node_name(node),
        // Blade names what the PHP its template embeds names (`blade::view`).
        Family::Php | Family::Blade => php_compare::node_name(node),
        Family::Python => {
            unreachable!("`name` is refused for this language when the policy compiles")
        }
    }
}

/// Whether both nodes are PHP their template spells; Blade's `view` writes tokens (`@unless`'s
/// `!(`) a comparison of tokens must not read as the template's (`blade::rewritten`).
fn blade_spelled<D: Doc>(family: Family, left: &Node<'_, D>, right: &Node<'_, D>) -> bool {
    match family {
        Family::Blade => {
            !crate::policy_dsl::blade::rewritten(left)
                && !crate::policy_dsl::blade::rewritten(right)
        }
        Family::Python | Family::Ecma | Family::Php => true,
    }
}

/// What `holds` reads of one bound node under `key` or `name`: `None` where its verdict is
/// unknown whatever the other side (no reading, or a node Blade rewrote), `Some(None)` where the
/// node has no key or name, which equals none.
pub(super) fn name<D: Doc>(
    comparison: &Comparison,
    text: SemanticText,
    node: &Node<'_, D>,
) -> Option<Option<Vec<u16>>> {
    let reading = comparison.reading.as_ref()?;
    let family = Family::of(reading.language);
    if !blade_spelled(family, node, node) {
        return None;
    }
    Some(match (text, family) {
        (SemanticText::Key, Family::Ecma) => key::static_property_name(node),
        (SemanticText::Key, Family::Php | Family::Blade) => {
            let mut read = |node: &Node<'_, D>| reading.value(node).as_ref().clone();
            php_compare::key_name(node, &mut read).flatten()
        }
        (SemanticText::Name, _) => node_name(family, node),
        (SemanticText::Key, Family::Python) => {
            unreachable!("`key` is refused for this language when the policy compiles")
        }
        _ => unreachable!("`name` reads `key` and `name` only"),
    })
}

/// Two bound captures compared under a semantic mode.
pub(super) fn holds<D: Doc>(
    comparison: &Comparison,
    text: SemanticText,
    left: &Node<'_, D>,
    right: &Node<'_, D>,
) -> Option<bool> {
    let reading = comparison.reading.as_ref()?;
    let family = Family::of(reading.language);
    // Blade's values are decoded with the view's tokens; every other mode reads tokens.
    if text != SemanticText::Value && !blade_spelled(family, left, right) {
        return None;
    }
    match text {
        SemanticText::Key | SemanticText::Name => {
            match (
                name(comparison, text, left)?,
                name(comparison, text, right)?,
            ) {
                (Some(left), Some(right)) => {
                    Some(comparison.op.test(comparison.units_order(&left, &right)))
                }
                // A key without a static name equals no name, another nameless one included,
                // as the ESLint rules that compare names skip null; it has no order.
                _ => match comparison.op {
                    CompareOp::Eq => Some(false),
                    CompareOp::Ne => Some(true),
                    _ => None,
                },
            }
        }
        SemanticText::Value => values(comparison, left, right),
        SemanticText::Reference { static_keys } => {
            let same = match family {
                Family::Ecma => key::same_reference(left, right, static_keys)?,
                Family::Php | Family::Blade => {
                    php_compare::same_reference(left, right, static_keys)?
                }
                Family::Python => unreachable!(
                    "`reference` is refused for this language when the policy compiles"
                ),
            };
            Some(same != (comparison.op == CompareOp::Ne))
        }
        SemanticText::Member => {
            let mut read = |node: &Node<'_, D>| reading.value(node).as_ref().clone();
            let (left, right) = match family {
                Family::Ecma => (
                    key::member_access_name(left, &mut read)?,
                    key::member_access_name(right, &mut read)?,
                ),
                // A node holding no key has no member name either.
                Family::Php | Family::Blade => (
                    php_compare::key_name(left, &mut read)??,
                    php_compare::key_name(right, &mut read)??,
                ),
                Family::Python => {
                    unreachable!("`member` is refused for this language when the policy compiles")
                }
            };
            Some(comparison.op.test(comparison.units_order(&left, &right)))
        }
        SemanticText::Structure(equality) => {
            let (left, right) = (
                reading.structure(left, equality),
                reading.structure(right, equality),
            );
            let equal = left.as_ref().as_ref()? == right.as_ref().as_ref()?;
            Some(if comparison.op == CompareOp::Ne {
                !equal
            } else {
                equal
            })
        }
    }
}

fn values<D: Doc>(
    comparison: &Comparison,
    left: &Node<'_, D>,
    right: &Node<'_, D>,
) -> Option<bool> {
    let reading = comparison.reading.as_ref()?;
    // Only expressions have values.
    if !reading.has_value(left) || !reading.has_value(right) {
        return Some(false);
    }
    let (left, right) = (reading.value(left), reading.value(right));
    let lang = reading.lang();
    let strings = match (left.as_ref(), right.as_ref()) {
        (Value::Str(left), Value::Str(right)) => Some((
            left.encode_utf16().collect::<Vec<_>>(),
            right.encode_utf16().collect::<Vec<_>>(),
        )),
        (Value::Utf16(left), Value::Utf16(right)) => Some((left.clone(), right.clone())),
        // A PHP string compares as text where both are UTF-8.
        (Value::PhpStr(left), Value::PhpStr(right)) => {
            match (std::str::from_utf8(left), std::str::from_utf8(right)) {
                (Ok(left), Ok(right)) => Some((
                    left.encode_utf16().collect::<Vec<_>>(),
                    right.encode_utf16().collect::<Vec<_>>(),
                )),
                _ => None,
            }
        }
        _ => None,
    };
    if let Some((left_units, right_units)) = strings
        && comparison.collation != Collation::Ordinal
    {
        return Some(
            comparison
                .op
                .test(comparison.units_order(&left_units, &right_units)),
        );
    }
    match comparison.op {
        CompareOp::Eq => left.equals(&right, lang),
        CompareOp::Ne => left.equals(&right, lang).map(|equal| !equal),
        // JavaScript orders as its relational operators do, converting primitives.
        op if lang == Lang::Js => ops::relation(
            match op {
                CompareOp::Lt => "<",
                CompareOp::Le => "<=",
                CompareOp::Gt => ">",
                _ => ">=",
            },
            &left,
            &right,
        ),
        // PHP orders as its relational operators do: `a > b` is `b < a`, which PHP's comparison of
        // arrays does not make the same question.
        op if lang == Lang::Php => crate::policy_dsl::value::php::relation(
            match op {
                CompareOp::Lt => "<",
                CompareOp::Le => "<=",
                CompareOp::Gt => ">",
                _ => ">=",
            },
            &left,
            &right,
        ),
        // Python values that do not order (NaN, different types) fail an ordering.
        op => left
            .order(&right, lang)
            .map(|order| order.is_some_and(|order| op.test(order))),
    }
}
