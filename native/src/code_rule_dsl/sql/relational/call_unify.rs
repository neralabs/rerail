//! Atomic single-capture unification for parameter frames, with native channel semantics.
//! SQL owns the sequence, branch verdict, and transactional commit.
use super::{
    capture_cell::Cell,
    span_capture::{self, Span},
};
use crate::code_rule_dsl::{matcher::node_at, sql::exec::Batch};
use rusqlite::{Connection, Error, Result, functions::FunctionFlags};
use std::sync::Arc;

fn invalid(message: &str) -> Error {
    Error::ModuleError(format!("call capture: {message}"))
}
fn span(cell: &Cell, file: u32) -> Result<Option<Span>> {
    let Cell::Span(bytes) = cell else {
        return Ok(None);
    };
    let span = span_capture::decode(bytes)?;
    if span.file != file {
        return Err(invalid("span belongs to another document"));
    }
    Ok(Some(span))
}
fn node(batch: &Batch, file: u32, node: u32) -> Result<(String, (usize, usize))> {
    if let Some(file) = batch.files.get(file as usize) {
        if node >= file.facts.len() {
            return Err(invalid("node outside document"));
        }
        let node = node_at(&file.root, node);
        let range = node.range();
        Ok((node.text().into_owned(), (range.start, range.end)))
    } else if let Some(sub) = batch.subfile(file as usize) {
        if node >= sub.facts.len() {
            return Err(invalid("node outside parsed document"));
        }
        let node = sub.root.adopt(sub.root.root().get_doc().node(node));
        let range = node.range();
        Ok((node.text().into_owned(), (range.start, range.end)))
    } else {
        Err(invalid("missing document"))
    }
}

/// None is a conflicting pair, distinct from a successful absent Cell::Null.
fn unify(batch: &Batch, file: u32, left: &Cell, right: &Cell, held: bool) -> Result<Option<Cell>> {
    if matches!(left, Cell::List(_)) || matches!(right, Cell::List(_)) {
        return Err(invalid("list parameters have no single-capture contract"));
    }
    let left_span = span(left, file)?;
    let right_span = span(right, file)?;
    if let (Some(a), Some(b)) = (&left_span, &right_span)
        && ((a.text.is_some() && b.text.is_some() && a.text != b.text) || a.range != b.range)
    {
        return Ok(None);
    }
    let left_node = left.single_node()?;
    let right_node = right.single_node()?;
    let chosen = match (left_node, right_node) {
        (Some(a), Some(b)) => {
            let av = node(batch, file, a)?;
            let bv = node(batch, file, b)?;
            if av.0 != bv.0 {
                return Ok(None);
            }
            Some(if held || av.1 <= bv.1 { a } else { b })
        }
        (Some(a), None) => {
            node(batch, file, a)?;
            Some(a)
        }
        (None, Some(b)) => {
            node(batch, file, b)?;
            Some(b)
        }
        (None, None) => None,
    };
    let merged_text = left_span
        .as_ref()
        .and_then(|span| span.text.clone())
        .or_else(|| right_span.as_ref().and_then(|span| span.text.clone()));
    if let Some(mut span) = left_span.or(right_span) {
        span.text = merged_text;
        span.node = chosen;
        return Ok(Some(Cell::Span(span_capture::encode(&span)?)));
    }
    Ok(Some(
        chosen.map_or(Cell::Null, |node| Cell::Integer(i64::from(node))),
    ))
}

fn returned(file: u32, before: &Cell, frame: &Cell) -> Result<bool> {
    let before_span = span(before, file)?;
    let frame_span = span(frame, file)?;
    Ok(
        (frame.single_node()?.is_some() && before.single_node()?.is_none())
            || (frame_span.as_ref().is_some_and(|span| span.text.is_some())
                && before_span.as_ref().is_none_or(|span| span.text.is_none())),
    )
}

pub(super) fn register(db: &Connection, batch: &Arc<Batch>) -> Result<()> {
    let flags = FunctionFlags::SQLITE_UTF8;
    db.create_scalar_function("relation_call_returned", 3, flags, |ctx| {
        returned(ctx.get(0)?, &ctx.get::<Cell>(1)?, &ctx.get::<Cell>(2)?)
    })?;
    let compatible = Arc::clone(batch);
    db.create_scalar_function("relation_call_compatible", 3, flags, move |ctx| {
        Ok(unify(
            &compatible,
            ctx.get(0)?,
            &ctx.get::<Cell>(1)?,
            &ctx.get::<Cell>(2)?,
            false,
        )?
        .is_some())
    })?;
    let merge = Arc::clone(batch);
    db.create_scalar_function("relation_call_merge", 4, flags, move |ctx| {
        Ok(unify(
            &merge,
            ctx.get(0)?,
            &ctx.get::<Cell>(1)?,
            &ctx.get::<Cell>(2)?,
            ctx.get::<i64>(3)? != 0,
        )?
        .unwrap_or(Cell::Null))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_rule_dsl::{
        language::GrammarDialect,
        sql::{exec::Batch, tests::differential::tape},
    };
    #[test]
    fn absent_and_conflict_are_distinct_and_span_channels_unify_independently() {
        let batch = Batch::new(
            vec![(tape(GrammarDialect::Python, "a = a\n"), Default::default())],
            vec![],
        );
        assert_eq!(
            unify(&batch, 0, &Cell::Null, &Cell::Null, false).unwrap(),
            Some(Cell::Null)
        );
        let a = Cell::Span(
            span_capture::encode(&Span {
                file: 0,
                node: None,
                text: Some("decoded".into()),
                range: 0..1,
            })
            .unwrap(),
        );
        let b = Cell::Span(
            span_capture::encode(&Span {
                file: 0,
                node: None,
                text: Some("decoded".into()),
                range: 4..5,
            })
            .unwrap(),
        );
        assert!(unify(&batch, 0, &a, &b, false).unwrap().is_none());
        assert_eq!(
            unify(&batch, 0, &Cell::Null, &a, false).unwrap(),
            Some(a.clone())
        );
        let file = &batch.files[0];
        let ids: Vec<_> = (0..file.facts.len())
            .filter(|id| {
                let node = node_at(&file.root, *id);
                node.kind() == "identifier" && node.text() == "a"
            })
            .collect();
        assert_eq!(ids.len(), 2);
        let early = Cell::Integer(i64::from(ids[0]));
        let late = Cell::Integer(i64::from(ids[1]));
        assert_eq!(
            unify(&batch, 0, &late, &early, false).unwrap(),
            Some(early.clone())
        );
        assert_eq!(unify(&batch, 0, &late, &early, true).unwrap(), Some(late));
        let Some(Cell::Span(shadow)) = unify(&batch, 0, &a, &early, false).unwrap() else {
            panic!("span and node channels must coexist")
        };
        let shadow = span_capture::decode(&shadow).unwrap();
        assert_eq!(shadow.node, Some(ids[0]));
        assert_eq!(shadow.text.as_deref(), Some("decoded"));
        let range_only = Cell::Span(
            span_capture::encode(&Span {
                file: 0,
                node: Some(ids[0]),
                text: None,
                range: 0..1,
            })
            .unwrap(),
        );
        assert!(!returned(0, &early, &range_only).unwrap());
        let new_shadow = Cell::Span(
            span_capture::encode(&Span {
                file: 0,
                node: Some(ids[0]),
                text: Some("new".into()),
                range: 0..1,
            })
            .unwrap(),
        );
        assert!(returned(0, &early, &new_shadow).unwrap());
        assert!(!returned(0, &a, &b).unwrap());
        let foreign = Cell::Span(
            span_capture::encode(&Span {
                file: 1,
                node: None,
                text: Some("decoded".into()),
                range: 0..1,
            })
            .unwrap(),
        );
        assert!(unify(&batch, 0, &a, &foreign, false).is_err());
    }
}
