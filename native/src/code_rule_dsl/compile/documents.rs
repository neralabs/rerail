//! Which document of a file each part of an ast query reads (`embed`): the file's own tree (the
//! host) or the program it embeds (the guest). A clause reads the guest where it names one
//! (`ast.embedded`); `embedded` crosses from a host node to the guest, `host` from a guest node to
//! the host. A `host` inside `parse` is the parse's (the literal it holds), on the same document.

use super::ast_rule::call_arguments;
use super::compare::parse_comparison;
use super::utils::{UtilDef, UtilIds};
use crate::code_rule_dsl::json::invalid;
use crate::code_rule_dsl::language::{GrammarDialect, Via};
use crate::code_rule_dsl::matcher::{CompareText, UtilTable};
use napi::Result;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Weak;

/// The document a rule reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Side {
    Host,
    Guest,
}

/// The documents of the files an embedding language's query reads, each with the utility table
/// its rules compile with: the host's dialect, and each dialect the guest may be in.
pub(crate) struct DocumentScopes {
    pub(crate) via: Via,
    pub(crate) host: (GrammarDialect, Weak<UtilTable>),
    pub(crate) guests: Vec<(GrammarDialect, Weak<UtilTable>)>,
}

/// Keys that read a document's semantics, which a host whose language embeds a program does not
/// have: the program does.
const SEMANTIC_KEYS: &[&str] = &[
    "resolves",
    "binds",
    "value",
    "arguments",
    "calls",
    "formatArgs",
];

/// Checks where `embedded` and `host` stand in a query of `language` that starts on `start`
/// (`embedded`: the guest of the via the language embeds, if any), and that no semantic key reads
/// the host of a language that embeds a program; returns the sides each utility is reached from.
pub(super) fn check_sides(
    roots: &[(&Value, String)],
    utils: &BTreeMap<String, UtilDef>,
    ids: &UtilIds,
    start: Side,
    language: &str,
    embeds: bool,
) -> Result<BTreeMap<String, BTreeSet<Side>>> {
    let mut walk = Walk {
        utils,
        ids,
        language,
        embeds,
        reached: BTreeMap::new(),
    };
    for (rule, path) in roots {
        walk.rule(rule, path, start, false)?;
    }
    Ok(walk.reached)
}

struct Walk<'a, 'u> {
    utils: &'a BTreeMap<String, UtilDef<'u>>,
    ids: &'a UtilIds<'a>,
    language: &'a str,
    embeds: bool,
    reached: BTreeMap<String, BTreeSet<Side>>,
}

impl Walk<'_, '_> {
    fn rule(&mut self, value: &Value, path: &str, side: Side, in_parse: bool) -> Result<()> {
        match value {
            Value::Array(items) => {
                for (index, item) in items.iter().enumerate() {
                    self.rule(item, &format!("{path}/{index}"), side, in_parse)?;
                }
                Ok(())
            }
            Value::Object(object) => {
                for (key, child) in object {
                    let at = format!("{path}/{key}");
                    match key.as_str() {
                        "embedded" => {
                            if in_parse {
                                return Err(invalid(format!(
                                    "{at}: embedded is not supported inside parse"
                                )));
                            }
                            if !self.embeds {
                                return Err(invalid(format!(
                                    "{at}: {} embeds no program",
                                    self.language
                                )));
                            }
                            if side == Side::Guest {
                                return Err(invalid(format!(
                                    "{at}: this rule already reads the embedded program; reach the host's node with host first"
                                )));
                            }
                            self.rule(child, &at, Side::Guest, false)?;
                        }
                        "host" if in_parse => self.rule(child, &at, side, false)?,
                        "host" => {
                            if side == Side::Host {
                                return Err(invalid(if self.embeds {
                                    format!(
                                        "{at}: host reads the file's own tree from its embedded program; use it in a clause with ast.embedded or under embedded"
                                    )
                                } else {
                                    format!(
                                        "{at}: host tests the literal a parse holds; use it inside parse"
                                    )
                                }));
                            }
                            self.rule(child, &at, Side::Host, false)?;
                        }
                        "parse" => self.rule(child, &at, side, true)?,
                        "matches" => {
                            let (id, _) = call_arguments(child, &at, self.ids)?;
                            if self.reached.entry(id.to_owned()).or_default().insert(side)
                                && let Some(util) = self.utils.get(id)
                            {
                                let rule_path = util.rule_path.clone();
                                self.rule(util.rule, &rule_path, side, false)?;
                            }
                        }
                        key if side == Side::Host
                            && self.embeds
                            && !in_parse
                            && (SEMANTIC_KEYS.contains(&key)
                                || (key == "compare"
                                    && parse_comparison(child, &at).is_ok_and(|comparison| {
                                        matches!(comparison.text, CompareText::Semantic(_))
                                    }))) =>
                        {
                            return Err(invalid(format!(
                                "{at}: `{key}` reads {}'s embedded program, not the file's own tree: write it under embedded, or in a clause with ast.embedded",
                                self.language
                            )));
                        }
                        _ => self.rule(child, &at, side, in_parse)?,
                    }
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
}
