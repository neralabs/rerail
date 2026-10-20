//! Bounded, single-consumer frame inlining before register allocation. Native input handles and
//! mutable-state barriers remain materialized. Each definition is substituted at most once.

use super::{Emitted, Rule, TEMP, Writes};
use std::collections::HashMap;

const MAX_FRAMES: usize = 4;
const MAX_SQL_BYTES: usize = 64 * 1024;

#[derive(Default)]
struct Declaration {
    columns: Vec<String>,
    steps: Vec<usize>,
    clears: Vec<usize>,
}

/// Include references inside descriptors, so a frame read directly by a native terminal
/// cannot disappear. The trailing identifier boundary distinguishes `x1` from `x10`.
fn references(sql: &str) -> Vec<(usize, &str)> {
    sql.match_indices(TEMP)
        .filter_map(|(at, _)| {
            let end = at
                + TEMP.len()
                + sql[at + TEMP.len()..]
                    .bytes()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == b'_')
                    .count();
            (end > at + TEMP.len()).then_some((at, &sql[at..end]))
        })
        .collect()
}

fn administrative(sql: &str, effect: &super::Effect) -> bool {
    sql.starts_with("CREATE ") || matches!(effect.writes, Writes::Clear(_))
}

fn join_rule(rule: Rule) -> bool {
    matches!(rule, Rule::EmptyIfAnyInputEmpty | Rule::CopyOf)
}

fn native_source(sql: &str) -> bool {
    ["fact", "pattern", "text_match"].iter().any(|table| {
        sql.contains(&format!("FROM {table} ")) || sql.contains(&format!("JOIN {table} "))
    })
}

/// A native terminal may fail. Its evaluation cannot move behind a later filter. These two
/// projection shapes consume all its rows, optionally reattaching their original input's metadata.
fn projection_consumer(sql: &str, producer: &str, input: Option<&String>) -> bool {
    let Some((head, tail)) = sql.split_once(" SELECT ") else {
        return false;
    };
    if !head.starts_with("INSERT INTO ") || tail.contains("SELECT ") {
        return false;
    }
    tail.ends_with(&format!(" FROM {producer} o"))
        || input.is_some_and(|input| {
            tail.ends_with(&format!(" FROM {producer} o JOIN {input} p ON p.k = o.k"))
        })
}

pub(super) fn fuse(mut emitted: Emitted) -> Emitted {
    let mut declarations = HashMap::<String, Declaration>::new();
    let mut uses = HashMap::<String, Vec<usize>>::new();
    let mut effect_uses = HashMap::<String, Vec<usize>>::new();
    for (index, (sql, effect)) in emitted.statements.iter().zip(&emitted.effects).enumerate() {
        if let Some(rest) = sql.strip_prefix("CREATE TEMP TABLE ") {
            let rest = rest.strip_prefix("IF NOT EXISTS ").unwrap_or(rest);
            if let Some((name, columns)) = rest.split_once('(')
                && name.starts_with(TEMP)
                && let Some(columns) = columns.strip_suffix(')')
                && columns.starts_with("k INTEGER PRIMARY KEY, file, node")
                && !columns.contains(" DEFAULT ")
            {
                let decl = declarations.entry(name.to_owned()).or_default();
                decl.columns = columns
                    .split(',')
                    .map(|column| column.split_whitespace().next().unwrap().to_owned())
                    .collect();
                decl.steps.push(index);
            }
        } else if sql.starts_with("CREATE INDEX ") {
            if let Some((_, rest)) = sql.split_once(" ON ") {
                let name = rest.split(['(', ' ']).next().unwrap_or_default();
                if name.starts_with(TEMP) {
                    declarations
                        .entry(name.to_owned())
                        .or_default()
                        .steps
                        .push(index);
                }
            }
        } else if let Writes::Clear(name) = &effect.writes {
            declarations
                .entry(name.clone())
                .or_default()
                .clears
                .push(index);
        }
        if !administrative(sql, effect) {
            for (_, name) in references(sql) {
                uses.entry(name.to_owned()).or_default().push(index);
            }
        }
        for input in &effect.inputs {
            effect_uses.entry(input.clone()).or_default().push(index);
        }
    }
    let mut removed = vec![false; emitted.statements.len()];
    let mut depth = vec![0; emitted.statements.len()];
    for producer in 0..emitted.statements.len() {
        if removed[producer] || !join_rule(emitted.effects[producer].rule) {
            continue;
        }
        let Writes::Insert(name) = &emitted.effects[producer].writes else {
            continue;
        };
        let name = name.clone();
        if emitted.root.as_ref() == Some(&name) {
            continue;
        }
        let Some(decl) = declarations.get(&name) else {
            continue;
        };
        if decl.columns.is_empty() || decl.clears.iter().any(|&clear| clear > producer) {
            continue;
        }
        let Some(sites) = uses.get(&name) else {
            continue;
        };
        if sites.len() != 2 || sites[0] != producer {
            continue;
        }
        let consumer = sites[1];
        if consumer <= producer
            || removed[consumer]
            || !join_rule(emitted.effects[consumer].rule)
            || !emitted.effects[consumer].inputs.contains(&name)
        {
            continue;
        }
        let combined_depth = depth[producer] + depth[consumer] + 1;
        if combined_depth > MAX_FRAMES {
            continue;
        }
        if effect_uses.get(&name).is_some_and(|sites| {
            sites
                .iter()
                .any(|&site| site != consumer && !decl.clears.contains(&site))
        }) {
            continue;
        }
        let consumer_target = match &emitted.effects[consumer].writes {
            Writes::Insert(target)
                if declarations
                    .get(target)
                    .is_some_and(|d| !d.columns.is_empty()) =>
            {
                Some(target)
            }
            Writes::None if emitted.statements[consumer].starts_with("SELECT ") => None,
            _ => continue,
        };
        if (producer + 1..consumer).any(|at| {
            !removed[at] && !emitted.statements[at].starts_with("CREATE ")
                && !matches!(&emitted.effects[at].writes, Writes::Clear(target)
                    if Some(target) == consumer_target && !emitted.statements[producer].contains(target))
        }) { continue; }
        let sql = &emitted.statements[producer];
        // These relations need a materialized index. An outer keyed lookup must not rescan
        // a grouped or compound CTE after the original frame's primary key disappears.
        if [
            " GROUP BY ",
            " DISTINCT ",
            " UNION ",
            " ORDER BY ",
            " LIMIT ",
            " HAVING ",
            " OVER ",
        ]
        .iter()
        .any(|part| sql.contains(part))
        {
            continue;
        }
        let insert = format!("INSERT INTO {name} ");
        let Some((with, select)) = sql.split_once(&insert) else {
            continue;
        };
        let Some(first) = select
            .strip_prefix("SELECT ")
            .and_then(|s| s.split(',').next())
        else {
            continue;
        };
        // NULL keys acquire a rowid during materialization; inlining cannot reproduce that.
        if !first.trim().ends_with(".k") || select.contains("SELECT NULL") {
            continue;
        }
        let projection = projection_consumer(
            &emitted.statements[consumer],
            &name,
            emitted.effects[producer].inputs.first(),
        );
        // Inlining also removes the frame's equality indexes. A later join can otherwise
        // repeat an indexed-prefix scan even when EXPLAIN reports no whole-table scan.
        if (native_source(sql) || emitted.statements[consumer].contains(" JOIN ")) && !projection {
            continue;
        }
        let Some((at, _)) = references(&emitted.statements[consumer])
            .into_iter()
            .find(|(_, table)| *table == name)
        else {
            continue;
        };
        let before = &emitted.statements[consumer][..at];
        if !before.ends_with("FROM ") && !before.ends_with("JOIN ") {
            continue;
        }
        let source = format!("sf_src{combined_depth}");
        let frame = format!("sf_frame{combined_depth}");
        // Unary + removes expression affinity without converting values. All non-key frame
        // columns have no affinity, even when their producer selected a typed SQL expression.
        let projected = decl
            .columns
            .iter()
            .enumerate()
            .map(|(index, column)| {
                format!(
                    "{}{source}.{column} AS {column}",
                    if index == 0 { "" } else { "+" }
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        let definitions = format!(
            "{source}({}) AS NOT MATERIALIZED ({with}{select}), \
             {frame} AS NOT MATERIALIZED (SELECT {projected} FROM {source})",
            decl.columns.join(", ")
        );
        let mut body = emitted.statements[consumer].clone();
        body.replace_range(at..at + name.len(), &frame);
        let fused = if let Some(rest) = body.strip_prefix("WITH ") {
            format!("WITH {definitions}, {rest}")
        } else {
            format!("WITH {definitions} {body}")
        };
        if fused.len() > MAX_SQL_BYTES {
            continue;
        }
        let mut inputs = Vec::new();
        for input in &emitted.effects[consumer].inputs {
            let replacing = if input == &name {
                emitted.effects[producer].inputs.as_slice()
            } else {
                std::slice::from_ref(input)
            };
            for input in replacing {
                if !inputs.contains(input) {
                    inputs.push(input.clone());
                }
            }
        }
        emitted.statements[consumer] = fused;
        emitted.effects[consumer].inputs = inputs;
        emitted.effects[consumer].rule = Rule::EmptyIfAnyInputEmpty;
        depth[consumer] = combined_depth;
        removed[producer] = true;
        for &step in decl.steps.iter().chain(&decl.clears) {
            removed[step] = true;
        }
    }
    emitted.statements = emitted
        .statements
        .into_iter()
        .enumerate()
        .filter_map(|(i, sql)| (!removed[i]).then_some(sql))
        .collect();
    emitted.effects = emitted
        .effects
        .into_iter()
        .enumerate()
        .filter_map(|(i, effect)| (!removed[i]).then_some(effect))
        .collect();
    emitted
}

#[cfg(test)]
mod tests {
    use super::super::{Effect, Reads};
    use super::*;
    use rusqlite::{Connection, Result};

    fn run(emitted: &Emitted) -> Result<Vec<i64>> {
        let connection = Connection::open_in_memory()?;
        connection.execute_batch(
            "CREATE TEMP TABLE source(k INTEGER PRIMARY KEY, file, node, s0 TEXT); \
             INSERT INTO source VALUES (1, 0, 0, '1'); \
             CREATE TEMP TABLE fact(x); INSERT INTO fact VALUES (0);",
        )?;
        for sql in &emitted.statements[..emitted.statements.len() - 1] {
            connection.execute_batch(sql)?;
        }
        connection
            .prepare(emitted.statements.last().unwrap())?
            .query_map([], |row| row.get(0))?
            .collect()
    }

    fn create(emitted: &mut Emitted, name: &str) {
        emitted.push(
            format!("CREATE TEMP TABLE {name}(k INTEGER PRIMARY KEY, file, node, s0)"),
            Effect::default(),
        );
    }

    #[test]
    fn inlining_keeps_the_frames_column_affinity() {
        let mut original = Emitted::default();
        let name = format!("{TEMP}p1");
        create(&mut original, &name);
        original.push(
            format!(
                "INSERT INTO {name} SELECT f.k, f.file, f.node, CAST(f.s0 AS INTEGER) FROM source f"
            ),
            Reads::join(&["source"]).writes(Writes::Insert(name.clone())),
        );
        original.push(
            format!("SELECT k FROM {name} WHERE s0 = '1'"),
            Reads::join(&[&name]).writes(Writes::None),
        );
        let fused = fuse(original.clone());
        assert!(fused.statements.len() < original.statements.len());
        assert!(run(&original).unwrap().is_empty());
        assert_eq!(run(&fused).unwrap(), run(&original).unwrap());
    }

    #[test]
    fn a_projection_can_fold_but_a_later_filter_cannot_hide_a_terminal_error() {
        let mut original = Emitted::default();
        let (producer, projected) = (format!("{TEMP}p1"), format!("{TEMP}p2"));
        create(&mut original, &producer);
        original.push(
            format!(
                "INSERT INTO {producer} SELECT f.k, f.file, f.node, abs(-9223372036854775808) \
             FROM source f CROSS JOIN fact t"
            ),
            Reads::join(&["source"]).writes(Writes::Insert(producer.clone())),
        );
        create(&mut original, &projected);
        original.push(
            format!("INSERT INTO {projected} SELECT o.k, o.file, o.node, o.s0 FROM {producer} o"),
            Reads::join(&[&producer]).writes(Writes::Insert(projected.clone())),
        );
        original.push(
            format!("SELECT k FROM {projected} WHERE 0"),
            Reads::join(&[&projected]).writes(Writes::None),
        );
        let fused = fuse(original.clone());
        assert!(fused.statements.len() < original.statements.len());
        assert!(
            run(&original)
                .unwrap_err()
                .to_string()
                .contains("integer overflow")
        );
        assert!(
            run(&fused)
                .unwrap_err()
                .to_string()
                .contains("integer overflow")
        );
    }
}
