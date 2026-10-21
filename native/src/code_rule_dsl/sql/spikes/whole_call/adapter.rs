//! Migration adapter: semantic SQL emitted today -> immutable SSA relations -> one statement.
//! It rejects mutable/fixed-point steps. It does not execute them behind a virtual table.
use super::{
    graph::{Boundary, Column, Graph},
    lex,
};
use crate::code_rule_dsl::sql::{
    emit::{self, Emitted},
    lower::Stage,
};
use std::collections::HashMap;

#[derive(Clone, Default)]
pub(super) struct Compiler {
    pub graph: Graph,
    pub stages: HashMap<(usize, String), usize>,
    pub stage_files: Option<String>,
    pub selected_roots: Option<String>,
    bridges: HashMap<usize, usize>,
}
impl Compiler {
    pub fn stage(&mut self, index: usize, stages: &[Stage], files: &str) -> Result<usize, String> {
        let files = self.stage_files.clone().unwrap_or_else(|| files.to_owned());
        let key = (index, files.clone());
        if let Some(&id) = self.stages.get(&key) {
            return Ok(id);
        }
        let stage = &stages[index];
        let columns = if stage.three {
            "file,node,unk"
        } else {
            "file,node"
        };
        let selected = self.selected_roots.take();
        let result = self.emitted(&stage.sql, columns, stages, &files);
        self.selected_roots = selected;
        let id = result?;
        self.stages.insert(key, id);
        Ok(id)
    }
    fn descriptor(&mut self, input: usize, tail: &str) -> String {
        let mut needed = vec!["k", "file", "node"];
        for name in tail.split(',').filter(|s| !s.is_empty()) {
            if !needed.contains(&name) {
                needed.push(name);
            }
        }
        let columns = needed
            .iter()
            .map(|name| {
                self.graph.relations[input]
                    .columns
                    .iter()
                    .find(|c| c.name == *name)
                    .expect("declared terminal input")
                    .clone()
            })
            .collect();
        let input = self.graph.add(
            columns,
            format!("SELECT {} FROM {}", needed.join(","), Graph::name(input)),
            Boundary::None,
        );
        let bridge = if let Some(&id) = self.bridges.get(&input) {
            id
        } else {
            let handle = format!("demand_{input}");
            let columns = &self.graph.relations[input].columns;
            let args = columns
                .iter()
                .map(|c| format!("f.{}", c.name))
                .collect::<Vec<_>>()
                .join(",");
            let body = format!(
                "SELECT publish_{handle}({args}) FROM {} f",
                Graph::name(input)
            );
            let id = self.graph.add(
                vec![Column {
                    name: "descriptor".into(),
                    integer: false,
                }],
                body,
                Boundary::Demand,
            );
            self.graph.handles.push((handle, input));
            self.bridges.insert(input, id);
            id
        };
        format!(
            "(SELECT descriptor FROM {}) || '{tail}'",
            Graph::name(bridge)
        )
    }
    fn resolve(
        &mut self,
        sql: &str,
        versions: &HashMap<String, usize>,
        stages: &[Stage],
        files: &str,
    ) -> Result<String, String> {
        let mut sql = sql.to_owned();
        // Native input descriptors carry actual relations, not opaque CTE names. Publish only
        // the demanded columns once through a typed aggregate, then pass its integer-keyed set.
        while let Some(start) = sql.find("frame = '") {
            let from = start + "frame = '".len();
            let end = from
                + sql[from..]
                    .find('\'')
                    .ok_or("unterminated frame descriptor")?;
            let descriptor = &sql[from..end];
            let split = descriptor.find(',').unwrap_or(descriptor.len());
            let name = &descriptor[..split];
            let input = *versions
                .get(name)
                .ok_or_else(|| format!("unpublished terminal input {name}"))?;
            let expr = self.descriptor(input, &descriptor[split..]);
            sql.replace_range(start..=end, &format!("frame = {expr}"));
        }
        let mut names: HashMap<_, _> = versions
            .iter()
            .map(|(name, &id)| (name.clone(), Graph::name(id)))
            .collect();
        // Stages are globally shared across clauses, not copied into clause namespaces.
        for word in lex::identifiers(&sql) {
            if let Some(id) = word
                .strip_prefix("stage_")
                .and_then(|id| id.parse::<usize>().ok())
            {
                if id >= stages.len() {
                    return Err(format!("unknown stage {id}"));
                }
                if self.selected_roots.is_some() && self.stage_files.is_none() {
                    return Err("selected-root stages need the whole-call file domain".into());
                }
                names.insert(word.to_owned(), Graph::name(self.stage(id, stages, files)?));
            }
        }
        Ok(lex::replace(&sql, &names))
    }
    pub fn emitted(
        &mut self,
        emitted: &Emitted,
        output_columns: &str,
        stages: &[Stage],
        files: &str,
    ) -> Result<usize, String> {
        let mut declarations: HashMap<String, Vec<Column>> = HashMap::new();
        let mut versions: HashMap<String, usize> = HashMap::new();
        let mut last = None;
        for original in &emitted.statements {
            if original.starts_with("/*") && !original.starts_with("/*T*/") {
                return Err(format!(
                    "stateful instruction {}",
                    original.split_whitespace().next().unwrap_or(original)
                ));
            }
            let sql = original
                .replace(emit::TEMP, "t_")
                .replace(emit::NAMESPACE, "ns_")
                .replace(emit::FILES, files)
                .replace(
                    emit::ROOTS,
                    self.selected_roots
                        .as_deref()
                        .unwrap_or("SELECT NULL AS file,NULL AS node WHERE 0"),
                );
            if let Some(rest) = sql.strip_prefix("CREATE TEMP TABLE ") {
                let rest = rest.strip_prefix("IF NOT EXISTS ").unwrap_or(rest);
                let open = rest.find('(').ok_or("table declaration")?;
                let name = rest[..open].trim().to_owned();
                let close = rest.rfind(')').ok_or("table close")?;
                let columns = lex::arguments(&rest[open + 1..close])
                    .into_iter()
                    .filter(|s| !s.starts_with("PRIMARY KEY") && !s.starts_with("UNIQUE"))
                    .map(|s| Column {
                        name: s
                            .split_whitespace()
                            .next()
                            .unwrap_or_default()
                            .trim_matches('"')
                            .to_owned(),
                        integer: s.contains("INTEGER"),
                    })
                    .collect();
                declarations.insert(name, columns);
                continue;
            }
            if sql.starts_with("CREATE INDEX ") || sql.starts_with("CREATE UNIQUE INDEX ") {
                continue;
            }
            if let Some(name) = sql.strip_prefix("DELETE FROM ") {
                if name.contains(' ') {
                    return Err("conditional delete".into());
                }
                versions.remove(name);
                continue;
            }
            if sql.starts_with("UPDATE ") {
                return Err("mutable update".into());
            }
            let insert = lex::top(&sql)
                .into_iter()
                .find(|&at| sql[at..].starts_with("INSERT INTO "));
            if let Some(at) = insert {
                let start = at + "INSERT INTO ".len();
                let end = start + sql[start..].find(' ').ok_or("insert target")?;
                let target = &sql[start..end];
                if target.contains('(') {
                    return Err("column-specific insert / mutable demand".into());
                }
                let columns = declarations
                    .get(target)
                    .cloned()
                    .ok_or_else(|| format!("no schema for {target}"))?;
                let body = format!("{}{}", &sql[..at], &sql[end + 1..]);
                let body = self.resolve(&body, &versions, stages, files)?;
                let old = versions.get(target).copied();
                let key = columns
                    .iter()
                    .position(|c| c.integer && matches!(c.name.as_str(), "k" | "pk"));
                let has_null_key = key == Some(0) && body.contains("SELECT NULL,");
                let boundary = if body.contains(" FROM pattern ")
                    || body.contains(" FROM fact ")
                    || body.contains(" FROM text_match ")
                {
                    Boundary::Terminal
                } else if has_null_key {
                    Boundary::Identity
                } else {
                    Boundary::None
                };
                let labels = columns
                    .iter()
                    .map(|c| c.name.as_str())
                    .collect::<Vec<_>>()
                    .join(",");
                let offset = old.filter(|_| has_null_key).map_or("0".into(), |id| {
                    format!("(SELECT COALESCE(MAX(k),0) FROM {})", Graph::name(id))
                });
                let fields = columns
                    .iter()
                    .enumerate()
                    .map(|(i, c)| {
                        let value = if has_null_key && i == 0 {
                            format!("COALESCE(p.{},ROW_NUMBER() OVER () + {offset})", c.name)
                        } else {
                            format!("p.{}", c.name)
                        };
                        if c.integer {
                            format!("CAST({value} AS INTEGER)")
                        } else {
                            format!("+({value})")
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(",");
                let mut definition =
                    format!("WITH payload({labels}) AS ({body}) SELECT {fields} FROM payload p");
                if let Some(previous) = old {
                    definition = format!(
                        "SELECT {labels} FROM {} UNION ALL SELECT * FROM ({definition})",
                        Graph::name(previous)
                    );
                }
                let id = self.graph.add(columns, definition, boundary);
                versions.insert(target.to_owned(), id);
                continue;
            }
            if sql.starts_with("SELECT ") || sql.starts_with("WITH ") {
                let body = self.resolve(&sql, &versions, stages, files)?;
                let columns = output_columns
                    .split(',')
                    .map(|s| Column {
                        name: s.to_owned(),
                        integer: false,
                    })
                    .collect();
                last = Some(self.graph.add(columns, body, Boundary::None));
                continue;
            }
            return Err(format!(
                "unsupported SQL instruction {}",
                &sql[..sql.len().min(100)]
            ));
        }
        last.ok_or_else(|| "no output relation".into())
    }
}
