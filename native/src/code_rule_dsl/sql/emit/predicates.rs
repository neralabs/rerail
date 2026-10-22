//! Total capture-free predicates compile as Boolean SQL. Regex is a leaf intrinsic, like
//! SQLite regexp in ast-index; it never evaluates rule composition or walks the AST.
use super::{Emitter, Live, Op, Reads, Stop, Walk, list};

fn condition(op: &Op, budget: &mut usize) -> Option<String> {
    condition_at(op, budget, "n", "f.file", "f.node")
}

pub(super) fn root_condition(op: &Op) -> Option<String> {
    // An unanchored regex benefits from text_match's amortized whole-file scanner.
    // A Boolean region instead uses a leaf intrinsic to preserve its probe domain.
    if matches!(op, Op::Regex(_)) {
        return None;
    }
    condition_at(op, &mut 64, "n", "n.file", "n.id")
}

fn condition_at(op: &Op, budget: &mut usize, node: &str, file: &str, id: &str) -> Option<String> {
    *budget = budget.checked_sub(1)?;
    let sequence = |items: Vec<Option<String>>, conjunction: bool| {
        let items = items.into_iter().collect::<Option<Vec<_>>>()?;
        let mut result = if conjunction { "1" } else { "0" }.to_owned();
        for item in items.into_iter().rev() {
            result = if conjunction {
                format!("CASE WHEN ({item}) THEN ({result}) ELSE 0 END")
            } else {
                format!("CASE WHEN ({item}) THEN 1 ELSE ({result}) END")
            };
        }
        Some(result)
    };
    match op {
        Op::Kinds(kinds) => Some(format!("COALESCE({node}.kind IN ({}),0)", list(kinds))),
        Op::Never => Some("0".into()),
        Op::Regex(term) => Some(format!("node_regex({term},{file},{id})")),
        Op::Not(op) => Some(format!(
            "NOT ({})",
            condition_at(op, budget, node, file, id)?
        )),
        Op::All(items) | Op::Seq(items) => sequence(
            items
                .iter()
                .map(|op| condition_at(op, budget, node, file, id))
                .collect(),
            true,
        ),
        Op::Any(items) => sequence(
            items
                .iter()
                .map(|op| condition_at(op, budget, node, file, id))
                .collect(),
            false,
        ),
        Op::AllOpen(items) => sequence(
            items
                .iter()
                .map(|t| condition_at(&t.op, budget, node, file, id))
                .collect(),
            true,
        ),
        Op::AnyOpen(items) => sequence(
            items
                .iter()
                .map(|(op, _)| condition_at(op, budget, node, file, id))
                .collect(),
            false,
        ),
        Op::Relation {
            walk,
            sub,
            binds: false,
            need,
        }
        | Op::Search {
            walk,
            sub,
            binds: false,
            need,
            ..
        } => Some(format!(
            "EXISTS ({})",
            walk_query(*walk, sub, need, budget, node, file, id)?
        )),
        Op::Count {
            walk,
            sub,
            min,
            max,
            bind: None,
            need,
        } => {
            let query = walk_query(*walk, sub, need, budget, node, file, id)?;
            if *min == 0 && max.is_none() {
                return Some("1".into());
            }
            if *min == 1 && max.is_none() {
                return Some(format!("EXISTS ({query})"));
            }
            if *max == Some(0) && *min == 0 {
                return Some(format!("NOT EXISTS ({query})"));
            }
            if max.is_some_and(|max| *min > max) || *min > i64::MAX as usize {
                return Some("0".into());
            }
            let lower = if *min == 0 {
                "1".into()
            } else {
                format!("EXISTS ({query} LIMIT 1 OFFSET {})", min - 1)
            };
            let upper = max
                .filter(|&max| max < i64::MAX as usize)
                .map_or("1".into(), |max| {
                    format!("NOT EXISTS ({query} LIMIT 1 OFFSET {max})")
                });
            Some(format!("CASE WHEN ({lower}) THEN ({upper}) ELSE 0 END"))
        }
        Op::Call {
            entry,
            body,
            returns,
        } if entry.is_empty() && returns.is_empty() => condition_at(body, budget, node, file, id),
        _ => None,
    }
}

fn walk_query(
    walk: Walk,
    sub: &Op,
    need: &crate::code_rule_dsl::sql::plan::Need,
    budget: &mut usize,
    node: &str,
    file: &str,
    id: &str,
) -> Option<String> {
    if need.kinds.as_ref().is_some_and(Vec::is_empty) {
        return Some("SELECT 1 WHERE 0".into());
    }
    let child = format!("predicate_node_{}", *budget);
    let bounds = match walk {
        Walk::Has(Stop::Neighbor) => format!("{child}.parent={id}"),
        Walk::Has(Stop::End) => format!("{child}.id>{id} AND {child}.id<={node}.\"last\""),
        Walk::Inside(Stop::Neighbor) => format!("{child}.id={node}.parent"),
        Walk::Inside(Stop::End) => format!("{child}.id<{id} AND {child}.\"last\">={id}"),
        _ => return None,
    };
    let condition = condition_at(
        sub,
        budget,
        &child,
        &format!("{child}.file"),
        &format!("{child}.id"),
    )?;
    let kinds = need.kinds.as_ref().map_or(String::new(), |kinds| {
        format!(" AND {child}.kind IN ({})", list(kinds))
    });
    Some(format!(
        "SELECT 1 FROM node {child} WHERE {child}.file={file} AND {bounds}{kinds} AND ({condition})"
    ))
}

impl Emitter<'_> {
    pub(super) fn predicate_region(
        &mut self,
        op: &Op,
        input: &str,
        first: bool,
        live: &Live,
    ) -> Option<String> {
        if matches!(op, Op::Kinds(_) | Op::Regex(_) | Op::Never) {
            return None;
        }
        let condition = condition(op, &mut 64)?;
        Some(self.within(live, |this| {
            let mut selected = this.overridden("f",&[]);
            if first { selected.push_str(this.zeros()); }
            else {
                selected.push_str(", f.unk");
                for word in 0..this.masks { selected.push_str(&format!(", f.wh{word}")); }
            }
            this.frame("predicate",&format!("SELECT {selected} FROM {input} f LEFT JOIN node n ON n.file=f.file AND n.id=f.node WHERE {condition}"),Reads::join(&[input]))
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::{Connection, functions::FunctionFlags};
    use std::sync::{Arc, Mutex};

    #[test]
    fn boolean_sql_keeps_short_circuit_domains_and_unknown_free_truth() {
        assert!(root_condition(&Op::Regex(0)).is_none());
        let rule = Op::All(vec![
            Op::Any(vec![Op::Kinds(vec![7]), Op::Regex(0)]),
            Op::Not(Box::new(Op::Regex(1))),
        ]);
        let expr = condition(&rule, &mut 64).unwrap();
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE data(file,node); CREATE TABLE node(file,id,kind); INSERT INTO data VALUES(0,1),(0,2),(0,3); INSERT INTO node VALUES(0,1,7),(0,2,2),(0,3,2)").unwrap();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&calls);
        db.create_scalar_function("node_regex", 3, FunctionFlags::SQLITE_UTF8, move |ctx| {
            let term: i64 = ctx.get(0)?;
            let node: i64 = ctx.get(2)?;
            recorded.lock().unwrap().push((term, node));
            Ok(if term == 0 { node == 2 } else { node == 1 })
        })
        .unwrap();
        let actual = db.prepare(&format!("SELECT f.node FROM data f LEFT JOIN node n ON n.file=f.file AND n.id=f.node WHERE {expr} ORDER BY f.node")).unwrap()
            .query_map([], |r|r.get::<_,i64>(0)).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
        assert_eq!(actual, vec![2]);
        assert_eq!(*calls.lock().unwrap(), vec![(1, 1), (0, 2), (1, 2), (0, 3)]);
    }
}
