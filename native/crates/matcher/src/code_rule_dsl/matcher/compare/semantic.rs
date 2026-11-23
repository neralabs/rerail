use super::super::Reading;
use super::super::exported::exported_text;
use super::{Collation, CompareOp, Comparison};
use crate::code_rule_dsl::language::adapter::pack as language_pack;
use crate::code_rule_dsl::value::Value;
use crate::code_rule_dsl::value::structure::Equality;
use ast_grep_core::meta_var::MetaVarEnv;
use ast_grep_core::{Doc, Node};

pub use crate::code_rule_dsl::language::adapter::value::SemanticText;

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
    let reading = comparison.reading.as_ref()?;
    Some({
        let pack = language_pack::pack(reading.family);
        unbound_names::<D>(pack, comparison, env)
    })
}

/// `name` where a side is not a bound node, `A` reading the bound ones.
fn unbound_names<D: Doc>(
    pack: &dyn language_pack::LanguagePack,
    comparison: &Comparison,
    env: &MetaVarEnv<'_, D>,
) -> Option<bool> {
    // `None` for a node Blade wrote tokens into (`ValueRules::spelled`): its name is not known.
    let name = |side: &str| -> Option<Option<Option<Vec<u16>>>> {
        match env.get_match(side) {
            Some(node) => Some(
                language_pack::value_spelled(pack, node)
                    .then(|| language_pack::value_node_name(pack, node)),
            ),
            None => exported_text(env, side).map(|text| Some(Some(text.encode_utf16().collect()))),
        }
    };
    match (name(&comparison.left), name(&comparison.right)) {
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
    {
        let pack = language_pack::pack(reading.family);
        name_in::<D>(pack, reading, text, node)
    }
}

/// `name` read with `reading`, whose family's adapter is `A`.
fn name_in<D: Doc>(
    pack: &dyn language_pack::LanguagePack,
    reading: &Reading,
    text: SemanticText,
    node: &Node<'_, D>,
) -> Option<Option<Vec<u16>>> {
    if !language_pack::value_spelled(pack, node) {
        return None;
    }
    Some(match text {
        SemanticText::Key => {
            let mut read = |node: &Node<'_, D>| reading.value(node).as_ref().clone();
            language_pack::value_key_name(pack, node, &mut read)
        }
        SemanticText::Name => language_pack::value_node_name(pack, node),
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
    {
        let pack = language_pack::pack(reading.family);
        holds_in::<D>(pack, comparison, reading, text, left, right)
    }
}

/// `holds` read with `reading`, whose family's adapter is `A`.
fn holds_in<D: Doc>(
    pack: &dyn language_pack::LanguagePack,
    comparison: &Comparison,
    reading: &Reading,
    text: SemanticText,
    left: &Node<'_, D>,
    right: &Node<'_, D>,
) -> Option<bool> {
    // Blade's values are decoded with the view's tokens; every other mode reads tokens.
    if text != SemanticText::Value
        && !(language_pack::value_spelled(pack, left) && language_pack::value_spelled(pack, right))
    {
        return None;
    }
    match text {
        SemanticText::Key | SemanticText::Name => {
            match (
                name_in::<D>(pack, reading, text, left)?,
                name_in::<D>(pack, reading, text, right)?,
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
            let same = language_pack::value_same_reference(pack, left, right, static_keys)?;
            Some(same != (comparison.op == CompareOp::Ne))
        }
        SemanticText::Member => {
            let mut read = |node: &Node<'_, D>| reading.value(node).as_ref().clone();
            let (left, right) = (
                language_pack::value_member_name(pack, left, &mut read)?,
                language_pack::value_member_name(pack, right, &mut read)?,
            );
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
        op => {
            let pack = language_pack::pack(lang);
            pack.value_relation(
                match op {
                    CompareOp::Lt => "<",
                    CompareOp::Le => "<=",
                    CompareOp::Gt => ">",
                    _ => ">=",
                },
                &left,
                &right,
            )
        }
    }
}
