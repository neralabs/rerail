//! Atomic coordinate conversion for a parsed single capture; no matcher runs here.
use super::{
    capture_cell::Cell,
    span_capture::{self, Span},
};
use crate::code_rule_dsl::sql::exec::Batch;
use rusqlite::{Connection, Error, Result, functions::FunctionFlags};
use std::sync::Arc;

fn invalid(message: &str) -> Error {
    Error::ModuleError(format!("parse export: {message}"))
}

pub(super) fn map(batch: &Batch, parsed_file: u32, host_file: u32, capture: Cell) -> Result<Cell> {
    if capture == Cell::Null {
        return Ok(Cell::Null);
    }
    let sub = batch
        .subfile(parsed_file as usize)
        .ok_or_else(|| invalid("missing parsed document"))?;
    let doc = sub.root.root().get_doc();
    let tree = crate::code_rule_dsl::literal::sub_tree(&sub.root.root())
        .cloned()
        .ok_or_else(|| invalid("missing parsed tree"))?;
    let (text, range) = match capture {
        Cell::Integer(node) => {
            let node = u32::try_from(node).map_err(|_| invalid("invalid capture node"))?;
            if node >= sub.facts.len() {
                return Err(invalid("capture node is outside parsed document"));
            }
            let node = sub.root.adopt(doc.node(node));
            (node.text().into_owned(), tree.host_range(node.node_id()))
        }
        Cell::Span(bytes) => {
            let span = span_capture::decode(&bytes)?;
            if span.file != parsed_file {
                return Err(invalid("capture belongs to another parsed document"));
            }
            if let Some(node) = span.node {
                if node >= sub.facts.len() {
                    return Err(invalid("shadow node outside parsed document"));
                }
                let node = sub.root.adopt(doc.node(node));
                (node.text().into_owned(), tree.host_range(node.node_id()))
            } else {
                match span.text {
                    Some(text) => (text, tree.host_span(span.range)),
                    None => return Ok(Cell::Null),
                }
            }
        }
        Cell::Null | Cell::List(_) => return Ok(Cell::Null),
    };
    let Some(range) = range else {
        return Ok(Cell::Null);
    };
    Ok(Cell::Span(span_capture::encode(&Span {
        file: host_file,
        node: None,
        text: Some(text),
        range,
    })?))
}

/// Native memo recall patches only remembered channels, not an entire environment cell.
pub(super) fn replay(file: u32, current: Cell, stored: Cell) -> Result<Cell> {
    let current_span = match &current {
        Cell::Span(bytes) => Some(span_capture::decode(bytes)?),
        _ => None,
    };
    let stored_span = match &stored {
        Cell::Span(bytes) => Some(span_capture::decode(bytes)?),
        _ => None,
    };
    if current_span
        .iter()
        .chain(stored_span.iter())
        .any(|span| span.file != file)
    {
        return Err(invalid("memo replay span belongs to another document"));
    }
    if let Some(mut span) = stored_span {
        if span.node.is_some() {
            // A node hides NAME's transformed text from native memo bindings().
            span.text = current_span.as_ref().and_then(|span| span.text.clone());
        } else {
            span.node = current.single_node()?;
            if span.text.is_none() {
                span.text = current_span.as_ref().and_then(|span| span.text.clone());
            }
        }
        return Ok(Cell::Span(span_capture::encode(&span)?));
    }
    if let Some(node) = stored.single_node()? {
        if let Some(mut span) = current_span {
            span.node = Some(node);
            return Ok(Cell::Span(span_capture::encode(&span)?));
        }
        return Ok(Cell::Integer(i64::from(node)));
    }
    Ok(current)
}

pub(super) fn register(db: &Connection, batch: &Arc<Batch>) -> Result<()> {
    let flags = FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC;
    db.create_scalar_function("relation_capture_replay", 3, flags, |ctx| {
        replay(ctx.get(0)?, ctx.get(1)?, ctx.get(2)?)
    })?;
    db.create_scalar_function("relation_capture_key", 1, flags, |ctx| {
        ctx.get::<Cell>(0)?.memo_key()
    })?;
    db.create_scalar_function("relation_capture_node", 1, flags, |ctx| {
        ctx.get::<Cell>(0)?.single_node()
    })?;
    db.create_scalar_function("relation_capture_bind", 3, flags, |ctx| {
        let file = ctx.get::<u32>(0)?;
        let cell = ctx.get::<Cell>(1)?;
        let node = ctx.get::<u32>(2)?;
        if let Cell::Span(bytes) = cell {
            let mut span = span_capture::decode(&bytes)?;
            if span.file != file {
                return Err(invalid("capture binding belongs to another document"));
            }
            span.node = Some(node);
            return Ok(Cell::Span(span_capture::encode(&span)?));
        }
        Ok(Cell::Integer(i64::from(node)))
    })?;
    db.create_scalar_function("relation_capture_export", 2, flags, |ctx| {
        let before = ctx.get::<Cell>(0)?;
        let exported = ctx.get::<Cell>(1)?;
        let Cell::Span(bytes) = exported else {
            return Ok(before);
        };
        let mut span = span_capture::decode(&bytes)?;
        span.node = before.single_node()?;
        Ok(Cell::Span(span_capture::encode(&span)?))
    })?;
    let batch = Arc::clone(batch);
    db.create_scalar_function(
        "relation_parse_export",
        3,
        FunctionFlags::SQLITE_UTF8,
        move |ctx| map(&batch, ctx.get(0)?, ctx.get(1)?, ctx.get(2)?),
    )?;
    db.create_scalar_function(
        "relation_span_json",
        2,
        FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC,
        |ctx| {
            let cell = ctx.get::<Cell>(0)?;
            let Cell::Span(bytes) = cell else {
                return Ok(None::<String>);
            };
            let span = span_capture::decode(&bytes)?;
            if span.file != ctx.get::<u32>(1)? {
                return Err(invalid("span publication belongs to another document"));
            }
            Ok(Some(match span.node {
                Some(node) => node.to_string(),
                None => match span.text {
                    Some(text) => {
                        serde_json::json!({"text":text,"span":[span.range.start,span.range.end]})
                            .to_string()
                    }
                    None => "null".into(),
                },
            }))
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn memo_replay_preserves_shadow_text_and_can_publish_only_reserved_range() {
        use crate::code_rule_dsl::{
            language::GrammarDialect,
            matcher::{exported_span, node_at},
            sql::{exec::Batch, plan::SlotKind, tests::differential::tape},
        };
        let stored = Cell::Span(
            span_capture::encode(&Span {
                file: 0,
                node: Some(0),
                text: Some("changed-during-miss".into()),
                range: 0..1,
            })
            .unwrap(),
        );
        let current = Cell::Span(
            span_capture::encode(&Span {
                file: 0,
                node: Some(0),
                text: Some("current-caller-text".into()),
                range: 2..3,
            })
            .unwrap(),
        );
        let Cell::Span(bytes) = replay(0, current, stored.clone()).unwrap() else {
            panic!("shadow cell")
        };
        let result = span_capture::decode(&bytes).unwrap();
        assert_eq!(result.text.as_deref(), Some("current-caller-text"));
        assert_eq!(result.range, 0..1);
        let replayed = replay(0, Cell::Integer(0), stored).unwrap();
        let Cell::Span(bytes) = &replayed else {
            panic!("range-only shadow cell")
        };
        let result = span_capture::decode(bytes).unwrap();
        assert_eq!(result.text, None);
        assert_eq!(result.node, Some(0));
        let batch = Batch::new(
            vec![(tape(GrammarDialect::Python, "x"), Default::default())],
            vec![],
        );
        let file = &batch.files[0];
        let env = super::super::capture_env::environment(
            0,
            &file.facts,
            |id| node_at(&file.root, id),
            &[("C".into(), SlotKind::Node)],
            &[replayed],
        )
        .unwrap();
        assert!(env.get_match("C").is_some());
        assert!(env.get_transformed("C").is_none());
        assert_eq!(exported_span(&env, "C"), Some(0..1));
    }
}
