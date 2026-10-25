//! Bounded physical cuts shared by ordinary and recursive pure producers.
use super::super::utility_projection_producer::StoredSummary;
use super::Sql;

fn flags(count: usize, word: usize) -> String {
    let bits = (word * 64..count.min((word + 1) * 64))
        .map(|i| format!("(coalesce(reached{i},0)<<{})", i % 64))
        .collect::<Vec<_>>();
    if bits.is_empty() {
        "0".into()
    } else {
        bits.join("|")
    }
}

impl Sql {
    pub(super) fn cut_projection_definitions(
        &mut self,
        initial: Vec<String>,
        source: Vec<String>,
        summaries: &[StoredSummary],
        physical: bool,
    ) -> Result<Vec<String>, String> {
        if !physical || !summaries.iter().any(|s| s.cut) {
            return Ok(initial.into_iter().chain(source).collect());
        }
        let table = format!("{}_stored", summaries[0].name);
        let create = format!(
            "CREATE TEMP TABLE IF NOT EXISTS {table}(owner INTEGER NOT NULL,file INTEGER NOT NULL,node INTEGER NOT NULL,incoming INTEGER NOT NULL,truth INTEGER,cost INTEGER,peak INTEGER,flags0 INTEGER,flags1 INTEGER,flags2 INTEGER,flags3 INTEGER,PRIMARY KEY(owner,file,node,incoming)) WITHOUT ROWID"
        );
        let mut stages = vec![create, format!("DELETE FROM {table}")];
        let offset = initial.len();
        let mut definitions = initial.into_iter().map(Some).collect::<Vec<_>>();
        let mut pending = Vec::new();
        for (index, definition) in source.into_iter().enumerate() {
            definitions.push(Some(definition));
            let Some((owner, summary)) =
                summaries.iter().enumerate().find(|(_, s)| s.index == index)
            else {
                continue;
            };
            if summary.flags > 256 {
                return Err("projection summary flags exceed256".into());
            }
            pending.push((owner, summary));
            if !summary.cut {
                continue;
            }
            let rows = pending
                .iter()
                .map(|(owner, s)| {
                    let packed = (0..4)
                        .map(|w| flags(s.flags, w))
                        .collect::<Vec<_>>()
                        .join(",");
                    format!(
                        "SELECT {owner},file,node,incoming,truth,cost,peak,{packed} FROM {}",
                        s.name
                    )
                })
                .collect::<Vec<_>>()
                .join(" UNION ALL ");
            let prefix = definitions
                .iter()
                .filter_map(Option::as_deref)
                .collect::<Vec<_>>()
                .join(",");
            let insert = format!("INSERT INTO {table} WITH RECURSIVE {prefix} {rows}");
            if insert.len() > 256000 {
                return Err("SQL text exceeds256000 bytes in projection summary cut".into());
            }
            stages.push(insert);
            for (owner, s) in pending.drain(..) {
                let reached = (0..s.flags)
                    .map(|i| format!(",(flags{}>>{})&1 reached{i}", i / 64, i % 64))
                    .collect::<String>();
                definitions[offset + s.index] = Some(format!(
                    "{} AS NOT MATERIALIZED (SELECT file,node,incoming,truth,cost,peak{reached} FROM {table} WHERE owner={owner})",
                    s.name
                ));
                // This aggregate belongs only to the stored summary. The raw
                // child rows remain available to the ordered entry recipe.
                for reduction in &s.reductions {
                    definitions[offset + reduction] = None;
                }
            }
        }
        self.generation
            .components
            .borrow_mut()
            .physical_stages
            .extend(stages);
        Ok(definitions.into_iter().flatten().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::super::Generation;
    use super::*;
    use rusqlite::{Connection, types::Value};

    fn read(db: &Connection, prefix: &[String], relation: &str) -> Vec<Vec<Value>> {
        let mut statement = db
            .prepare(&format!(
                "WITH RECURSIVE {} SELECT * FROM {relation} ORDER BY node",
                prefix.join(",")
            ))
            .unwrap();
        let columns = statement.column_count();
        statement
            .query_map([], |row| (0..columns).map(|i| row.get(i)).collect())
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }

    #[test]
    fn projection_storage_retains_all_flag_bits_and_resets_every_owner() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE inputs(node INTEGER)")
            .unwrap();
        let flags = (0..256)
            .map(|i| format!(",(node+{i})%2 reached{i}"))
            .collect::<String>();
        let definitions = vec![
            format!(
                "a AS MATERIALIZED (SELECT 0 file,node,0 incoming,1 truth,2 cost,3 peak{flags} FROM inputs)"
            ),
            "reduce_a AS MATERIALIZED (SELECT file,node,incoming,truth,cost+1 cost,peak FROM a)"
                .into(),
            "b AS MATERIALIZED (SELECT *,1 reached0,0 reached1,1 reached2 FROM reduce_a)".into(),
            "c AS MATERIALIZED (SELECT file,node,incoming,truth,cost+1 cost,peak FROM b)".into(),
        ];
        let summaries = vec![
            StoredSummary {
                index: 0,
                name: "a".into(),
                flags: 256,
                reductions: vec![],
                cut: false,
            },
            StoredSummary {
                index: 2,
                name: "b".into(),
                flags: 3,
                reductions: vec![1],
                cut: true,
            },
            StoredSummary {
                index: 3,
                name: "c".into(),
                flags: 0,
                reductions: vec![],
                cut: true,
            },
        ];
        let mut sql = Sql {
            definitions: vec![],
            profile: vec![],
            correlated: false,
            generation: Generation::default(),
        };
        let compiled = sql
            .cut_projection_definitions(vec![], definitions.clone(), &summaries, true)
            .unwrap();
        let stages = sql.generation.components.borrow().physical_stages.clone();
        assert_eq!(
            stages.len(),
            4,
            "one schema/reset pair, then two batched cuts"
        );
        assert!(
            compiled.iter().all(|d| !d.contains("reduce_a")),
            "stored reductions must not reappear in final SQL"
        );
        for inputs in [vec![1, 2], vec![3], vec![]] {
            db.execute("DELETE FROM inputs", []).unwrap();
            for node in inputs {
                db.execute("INSERT INTO inputs VALUES(?)", [node]).unwrap();
            }
            let expected = ["a", "b", "c"].map(|name| read(&db, &definitions, name));
            for stage in &stages {
                db.execute(stage, []).unwrap();
            }
            let actual = ["a", "b", "c"].map(|name| read(&db, &compiled, name));
            assert_eq!(actual, expected);
        }
        assert_eq!(
            db.query_row("SELECT count(*) FROM a_stored", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}
