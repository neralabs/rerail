//! One generic atomic pattern over typed inputs, including parsed documents.
use super::{
    capture_cell::{Cell, encode_list},
    capture_env,
};
use crate::code_rule_dsl::sql::{
    nodes::{NodeFacts, sub_index},
    plan::SlotKind,
    terminals::PatternTerm,
};
use ast_grep_core::{Doc, Node, matcher::Matcher};
use rusqlite::{Error, Result};
use std::borrow::Cow;

pub(super) fn match_at<'t, D: Doc>(
    term: &PatternTerm,
    file: u32,
    node: u32,
    facts: &NodeFacts,
    at: impl Fn(u32) -> Node<'t, D>,
    inputs: &[Cell],
) -> Result<Option<Vec<Cell>>> {
    let env = capture_env::environment(file, facts, &at, &term.vars, inputs)?;
    let mut env = Cow::Owned(env);
    if term
        .pattern
        .match_node_with_env(at(node), &mut env)
        .is_none()
    {
        return Ok(None);
    }
    let local = |id: usize| {
        if id & crate::code_rule_dsl::literal::ID_FLAG != 0 {
            sub_index(id)
        } else {
            id as u32
        }
    };
    term.vars
        .iter()
        .zip(inputs)
        .map(|((name, kind), input)| {
            Ok(match kind {
                SlotKind::Node => match env.get_match(name) {
                    Some(node) => {
                        let id = local(node.node_id());
                        if let Cell::Span(bytes) = input {
                            let mut span = super::span_capture::decode(bytes)?;
                            span.node = Some(id);
                            Cell::Span(super::span_capture::encode(&span)?)
                        } else {
                            Cell::Integer(i64::from(id))
                        }
                    }
                    None => Cell::Null,
                },
                SlotKind::List => match env.get_labels(name) {
                    Some(nodes) => Cell::List(encode_list(
                        file,
                        &nodes.iter().map(|n| local(n.node_id())).collect::<Vec<_>>(),
                    )?),
                    None => Cell::Null,
                },
                _ => {
                    return Err(Error::ModuleError(
                        "parsed pattern: unsupported capture type".into(),
                    ));
                }
            })
        })
        .collect::<Result<Vec<_>>>()
        .map(Some)
}
