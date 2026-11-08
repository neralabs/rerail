//! Reusable indexes from checked equality predicates; no parsing of rendered or authored SQL.
use super::super::logic::sql::select::{Join, Query, Relation, Select};
use super::super::logic::sql::sql_ast::{SqlBinaryOperator, SqlExpression};
use super::*;

/// Native tables already serve contiguous leading prefixes from their complete-row tree.
/// Keep requirements separate for each alias: two uses of one relation can probe different fields.
pub(super) fn native<'a>(
    queries: impl IntoIterator<Item = &'a Query>,
) -> BTreeMap<String, Vec<usize>> {
    fn block(select: &Select, wanted: &mut BTreeMap<String, BTreeSet<usize>>) {
        let mut aliases = BTreeMap::<String, BTreeSet<usize>>::new();
        let ons = select.from.iter().flat_map(|item| match &item.join {
            Join::On(on) => on.iter().collect::<Vec<_>>(),
            _ => vec![],
        });
        let expressions = select
            .columns
            .iter()
            .map(|(e, _)| e)
            .chain(&select.predicates)
            .chain(ons)
            .collect::<Vec<_>>();
        for e in expressions {
            e.visit(&mut |e| {
                if let SqlExpression::Binary {
                    operator: SqlBinaryOperator::Equal,
                    left,
                    right,
                } = e
                {
                    for side in [left.as_ref(), right.as_ref()] {
                        if let SqlExpression::Qualified { alias, column } = side
                            && let Some(c) = column
                                .strip_prefix('c')
                                .and_then(|s| s.parse::<usize>().ok())
                        {
                            aliases.entry(alias.clone()).or_default().insert(c);
                        }
                    }
                }
                if let SqlExpression::Exists { query, .. } | SqlExpression::Scalar(query) = e {
                    block(query, wanted);
                }
            });
        }
        for (alias, columns) in aliases {
            if columns.iter().copied().eq(0..columns.len()) {
                continue;
            }
            if let Some(item) = select.from.iter().find(|item| item.alias == alias) {
                let name = match &item.relation {
                    Relation::Derived { rel, delta: true } => Some(format!("delta_{rel}")),
                    r => r.program_relation().map(str::to_owned),
                };
                if let Some(name) = name {
                    wanted
                        .entry(name)
                        .or_default()
                        .extend(columns.into_iter().filter(|&c| c > 0));
                }
            }
        }
    }
    let mut wanted = BTreeMap::new();
    for q in queries {
        for select in q.ctes.iter().flat_map(|c| &c.arms).chain([&q.body]) {
            block(select, &mut wanted);
        }
    }
    wanted
        .into_iter()
        .map(|(r, c)| (r, c.into_iter().collect()))
        .collect()
}

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
            let prefix = cols.iter().copied().eq(0..cols.len() as u32);
            if !prefix {
                indexes.insert((table.clone(), cols.iter().copied().collect()));
            }
            // Delta variants can bind a later field before the composite index's
            // leading field. They need that access path without an automatic index.
            for &c in &cols {
                if c > 0 && (prefix || Some(&c) != cols.first()) {
                    indexes.insert((table.clone(), vec![c]));
                }
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
