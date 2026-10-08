use super::resolve::is_identifier;
use crate::code_rule_dsl::json::{invalid, object, reject_unknown};
use crate::code_rule_dsl::language::{Family, GrammarDialect, kind_ids};
use crate::code_rule_dsl::matcher::Semantic;
use crate::code_rule_dsl::matcher::{Arguments, AstMatcher, SlotKind};
use napi::Result;
use serde_json::Value;

/// Slots one signature may name.
const MAX_SLOTS: usize = 256;

/// A validated signature: each slot's name and how it may be passed.
fn signature(value: &Value, path: &str) -> Result<Vec<(String, SlotKind)>> {
    let path = format!("{path}/signature");
    let items = value
        .get("signature")
        .ok_or_else(|| invalid(format!("{path} is required")))?
        .as_array()
        .ok_or_else(|| invalid(format!("{path} must be an array of slot names")))?;
    if items.len() > MAX_SLOTS {
        return Err(invalid(format!(
            "{path} lists {} entries; at most {MAX_SLOTS} are allowed",
            items.len()
        )));
    }
    let mut slots: Vec<(String, SlotKind)> = Vec::new();
    let mut kind = SlotKind::Either;
    let (mut slash, mut star) = (false, false);
    for (index, item) in items.iter().enumerate() {
        let name = item
            .as_str()
            .ok_or_else(|| invalid(format!("{path}/{index} must be a string")))?;
        match name {
            "/" => {
                if slash || star {
                    return Err(invalid(format!(
                        "{path}/{index}: `/` may appear once, before `*`"
                    )));
                }
                slash = true;
                for slot in &mut slots {
                    slot.1 = SlotKind::PositionalOnly;
                }
            }
            "*" => {
                if star {
                    return Err(invalid(format!("{path}/{index}: `*` may appear once")));
                }
                star = true;
                kind = SlotKind::KeywordOnly;
            }
            _ if !is_identifier(name) || name.len() > 256 => {
                return Err(invalid(format!(
                    "{path}/{index}: `{name}` is not a slot name (an identifier), `/` or `*`"
                )));
            }
            _ if slots.iter().any(|(slot, _)| slot == name) => {
                return Err(invalid(format!("{path}/{index}: `{name}` is listed twice")));
            }
            _ => slots.push((name.to_owned(), kind)),
        }
    }
    if slots.is_empty() {
        return Err(invalid(format!("{path} must name a slot")));
    }
    Ok(slots)
}

fn slot_index(slots: &[(String, SlotKind)], name: &Value, path: &str) -> Result<usize> {
    let name = name
        .as_str()
        .ok_or_else(|| invalid(format!("{path} must be a slot name")))?;
    slots
        .iter()
        .position(|(slot, _)| slot == name)
        .ok_or_else(|| invalid(format!("{path}: `{name}` is not a slot of the signature")))
}

/// The grammar-independent checks of an `arguments` object.
pub(super) fn validate_arguments(value: &Value, path: &str) -> Result<()> {
    parse(value, path).map(|_| ())
}

type Parsed = (Vec<(String, SlotKind)>, Vec<(String, usize)>, Vec<usize>);

fn parse(value: &Value, path: &str) -> Result<Parsed> {
    let entry = object(value, path)?;
    reject_unknown(entry, &["signature", "bind", "absent", "mode"], path)?;
    if let Some(mode) = entry.get("mode")
        && !matches!(mode.as_str(), Some("language" | "syntactic" | "estree"))
    {
        return Err(invalid(format!(
            "{path}/mode must be \"language\", \"syntactic\" or \"estree\""
        )));
    }
    let slots = signature(value, path)?;
    let mut bind = Vec::new();
    if let Some(map) = entry.get("bind") {
        let bind_path = format!("{path}/bind");
        let map = object(map, &bind_path)?;
        if map.is_empty() {
            return Err(invalid(format!("{bind_path} must bind a capture")));
        }
        for (name, slot) in map {
            let valid = name
                .chars()
                .next()
                .is_some_and(|first| first.is_ascii_uppercase())
                && name
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
            if !valid {
                return Err(invalid(format!(
                    "{bind_path}: `{name}` is not a metavariable name (A-Z first, then A-Z, 0-9, _, without `$`)"
                )));
            }
            bind.push((
                name.clone(),
                slot_index(&slots, slot, &format!("{bind_path}/{name}"))?,
            ));
        }
    }
    let mut absent = Vec::new();
    if let Some(items) = entry.get("absent") {
        let absent_path = format!("{path}/absent");
        let items = items
            .as_array()
            .ok_or_else(|| invalid(format!("{absent_path} must be an array of slot names")))?;
        if items.is_empty() {
            return Err(invalid(format!("{absent_path} must not be empty")));
        }
        for (index, item) in items.iter().enumerate() {
            let slot = slot_index(&slots, item, &format!("{absent_path}/{index}"))?;
            if bind.iter().any(|(_, bound)| *bound == slot) {
                return Err(invalid(format!(
                    "{absent_path}/{index}: `{}` is bound, so it is passed",
                    slots[slot].0
                )));
            }
            absent.push(slot);
        }
    }
    if bind.is_empty() && absent.is_empty() {
        return Err(invalid(format!(
            "{path} must `bind` a slot or list `absent` ones"
        )));
    }
    Ok((slots, bind, absent))
}

/// Compiles a validated `arguments` for one grammar. Only Python has positional-only and
/// keyword-only parameters (`/`, `*`); Python, PHP and Blade's embedded PHP have Ruff's syntactic
/// reading; only ECMAScript has ESLint's `node.arguments` (`estree`). JavaScript has no keyword
/// arguments; PHP's named arguments bind any parameter.
pub(super) fn compile_arguments(
    value: &Value,
    language: GrammarDialect,
    path: &str,
) -> Result<AstMatcher> {
    let (slots, bind, absent) = parse(value, path)?;
    let family = Family::of(language);
    let markers = value
        .get("signature")
        .and_then(Value::as_array)
        .is_some_and(|items| {
            items
                .iter()
                .any(|item| matches!(item.as_str(), Some("/" | "*")))
        });
    let marker_refusal = match family {
        Family::Python => None,
        Family::Ecma => Some("JavaScript passes arguments by position only"),
        // Blade's calls are those of the PHP its template embeds.
        Family::Php | Family::Blade => {
            Some("PHP has no positional-only or keyword-only parameters")
        }
    };
    if let Some(detail) = marker_refusal.filter(|_| markers) {
        return Err(invalid(format!(
            "{path}/signature: `/` and `*` mark Python's positional-only and keyword-only slots; {detail}"
        )));
    }
    let mode = value.get("mode").and_then(Value::as_str);
    let syntactic = mode == Some("syntactic");
    let estree = mode == Some("estree");
    let mode_refusal = match (family, mode) {
        (Family::Ecma, Some("syntactic")) => Some(
            "`syntactic` finds arguments by position and keyword as Ruff's find_argument does; TypeScript has no keywords",
        ),
        (Family::Python, Some("estree")) => {
            Some("`estree` reads ESLint's `node.arguments`; use language or syntactic for Python")
        }
        (Family::Php, Some("estree")) => Some(
            "`estree` reads ESLint's `node.arguments` of ESTree, JavaScript's syntax tree; php has none (use language or syntactic)",
        ),
        (Family::Blade, Some("estree")) => Some(
            "`estree` reads ESLint's `node.arguments` of ESTree, JavaScript's syntax tree; blade has none (use language or syntactic)",
        ),
        _ => None,
    };
    if let Some(detail) = mode_refusal {
        return Err(invalid(format!("{path}/mode: {detail}")));
    }
    let call_kinds = match family {
        Family::Python => kind_ids(language, &["call"]),
        Family::Ecma => kind_ids(language, &["call_expression", "new_expression"]),
        Family::Php | Family::Blade => kind_ids(
            language,
            &[
                "function_call_expression",
                "member_call_expression",
                "nullsafe_member_call_expression",
                "scoped_call_expression",
                "object_creation_expression",
            ],
        ),
    };
    Ok(AstMatcher::Semantic(Semantic::Arguments(Box::new(
        Arguments {
            language,
            slots,
            bind,
            absent,
            call_kinds,
            syntactic,
            estree,
        },
    ))))
}

#[cfg(test)]
mod tests;
