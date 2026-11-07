//! Reusable indexes from checked equality predicates; no parsing of rendered or authored SQL.
use super::super::logic::sql::select::{Join, Query, Relation, Select};
use super::super::logic::sql::sql_ast::{SqlBinaryOperator, SqlExpression};
use super::*;

pub(super) fn wanted(emitted: &plan::emit::Emitted) -> BTreeSet<(String, Vec<u32>)> {
    let tables: BTreeSet<_> = emitted
        .program
        .relations
        .iter()
        .map(|(name, _)| name.as_str())
        .collect();
    let mut indexes = BTreeSet::new();
    fn select(block: &Select, tables: &BTreeSet<&str>, indexes: &mut BTreeSet<(String, Vec<u32>)>) {
        let mut columns = BTreeMap::<String, BTreeSet<u32>>::new();
        let ons = block.from.iter().flat_map(|item| match &item.join {
            Join::On(on) => on.iter().collect::<Vec<_>>(),
            _ => vec![],
        });
        for e in block.predicates.iter().chain(ons) {
            e.visit(&mut |e| {
                if let SqlExpression::Binary {
                    operator: SqlBinaryOperator::Equal,
                    left,
                    right,
                } = e
                {
                    for (side,other) in [(left.as_ref(), right.as_ref()),(right.as_ref(), left.as_ref())] {
                        if let SqlExpression::Qualified { alias, column } = side
                            && let Some(item) = block.from.iter().find(|item| item.alias == *alias)
                            && let Relation::Derived { rel, delta } = &item.relation
                            && tables.contains(rel.as_str())
                            && let Some(col) = column.strip_prefix('c').and_then(|s| s.parse().ok())
                        {
                            if *delta && block.from.first().is_some_and(|i| i.alias==*alias)
                                && block.from.iter().skip(1).all(|i|i.join==Join::Cross)
                                && matches!(other,SqlExpression::Qualified {alias:other,..} if other!=alias)
                            {continue;}
                            let table = if *delta {
                                format!("delta_{rel}")
                            } else {
                                rel.clone()
                            };
                            columns.entry(table).or_default().insert(col);
                        }
                    }
                }
            });
        }
        for (table, cols) in columns {
            if !cols.contains(&0) {
                indexes.insert((table, cols.into_iter().collect()));
            }
        }
        for e in block
            .columns
            .iter()
            .map(|(e, _)| e)
            .chain(&block.predicates)
        {
            e.visit(&mut |e| {
                if let SqlExpression::Exists { query, .. } | SqlExpression::Scalar(query) = e {
                    select(query, tables, indexes);
                }
            });
        }
    }
    let mut query = |q: &Query| {
        for block in q.ctes.iter().flat_map(|c| &c.arms).chain([&q.body]) {
            select(block, &tables, &mut indexes);
        }
    };
    for q in emitted
        .queries
        .iter()
        .flatten()
        .chain(emitted.views.iter().map(|(_, q)| q))
    {
        query(q);
    }
    indexes
}
