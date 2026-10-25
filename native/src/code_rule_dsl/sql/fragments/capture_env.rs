//! Typed capture reconstruction at an atomic AST boundary.
use super::{
    capture_cell::{Cell, decode_list},
    span_capture,
};
use crate::code_rule_dsl::{
    matcher::export_text,
    sql::{nodes::NodeFacts, plan::SlotKind},
};
use ast_grep_core::{Doc, Node, meta_var::MetaVarEnv};
use rusqlite::{Error, Result};

pub(in crate::code_rule_dsl::sql) fn environment<'t, D: Doc>(
    file: u32,
    facts: &NodeFacts,
    at: impl Fn(u32) -> Node<'t, D>,
    bindings: &[(String, SlotKind)],
    values: &[Cell],
) -> Result<MetaVarEnv<'t, D>> {
    let mut env = MetaVarEnv::new();
    for ((name, kind), value) in bindings.iter().zip(values) {
        match (kind, value) {
            (_, Cell::Null) => {}
            (SlotKind::Node, Cell::Integer(id)) => {
                let id = u32::try_from(*id).map_err(|_| invalid("invalid node"))?;
                if id >= facts.len() {
                    return Err(invalid("node outside document"));
                }
                env.insert(name, at(id))
                    .ok_or_else(|| invalid("conflicting node capture"))?;
            }
            (SlotKind::Node | SlotKind::Text, Cell::Span(bytes)) => {
                let span = span_capture::decode(bytes)?;
                if span.file != file {
                    return Err(invalid("span belongs to another document"));
                }
                if let Some(node) = span.node {
                    if node >= facts.len() {
                        return Err(invalid("shadow node outside document"));
                    }
                    env.insert(name, at(node))
                        .ok_or_else(|| invalid("conflicting shadow capture"))?;
                }
                if let Some(text) = span.text {
                    export_text(&mut env, name, &text, span.range);
                } else {
                    use ast_grep_core::{meta_var::MetaVariable, source::Content};
                    let reserved = format!("\0{name}");
                    let range = format!("{}:{}", span.range.start, span.range.end);
                    let bytes = <D::Source as Content>::decode_str(&range).into_owned();
                    env.insert_transformation(
                        &MetaVariable::Capture(reserved.clone(), false),
                        &reserved,
                        bytes,
                    );
                }
            }
            (SlotKind::List, Cell::List(bytes)) => {
                let nodes = decode_list(bytes, file, facts.len())?
                    .into_iter()
                    .map(&at)
                    .collect();
                env.insert_multi(name, nodes)
                    .ok_or_else(|| invalid("conflicting list capture"))?;
            }
            (SlotKind::Count, Cell::Integer(value)) => {
                use ast_grep_core::{meta_var::MetaVariable, source::Content};
                env.insert_transformation(
                    &MetaVariable::Multiple,
                    name,
                    <D::Source as Content>::decode_str(&value.to_string()).into_owned(),
                );
            }
            _ => return Err(invalid("capture type differs from binding contract")),
        }
    }
    Ok(env)
}
fn invalid(message: &str) -> Error {
    Error::ModuleError(format!("capture environment: {message}"))
}
