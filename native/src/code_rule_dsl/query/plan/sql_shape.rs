//! Typed, integer-only relational SQL shapes. This is not a grammar compiler.
use std::collections::{BTreeMap, BTreeSet};

type Result<T> = std::result::Result<T, String>;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Materialization {
    Materialized,
    Inline,
}
#[derive(Clone, Debug)]
pub(crate) struct Column {
    pub alias: String,
    pub name: String,
}
impl Column {
    pub(crate) fn new(alias: &str, name: &str) -> Self {
        Self {
            alias: alias.into(),
            name: name.into(),
        }
    }
}
#[derive(Clone, Debug)]
pub(crate) enum Expr {
    Column(Column),
    Parameter(u32),
    Integer(i64),
}
impl Expr {
    pub(crate) fn column(alias: &str, name: &str) -> Self {
        Self::Column(Column::new(alias, name))
    }
}
#[derive(Clone, Debug)]
pub(crate) struct Binding {
    pub relation: String,
    pub alias: String,
}
impl Binding {
    pub(crate) fn new(relation: &str, alias: &str) -> Self {
        Self {
            relation: relation.into(),
            alias: alias.into(),
        }
    }
}
#[derive(Clone, Debug)]
pub(crate) struct Join {
    pub source: Binding,
    pub on: Vec<(Column, Column)>,
}
#[derive(Clone, Debug)]
pub(crate) struct Select {
    pub from: Binding,
    pub joins: Vec<Join>,
    pub projection: Vec<Expr>,
    pub predicates: Vec<(Expr, Expr)>,
}
#[derive(Clone, Debug)]
pub(crate) struct Query {
    pub columns: Vec<String>,
    pub branches: Vec<Select>,
}
#[derive(Debug)]
pub(crate) struct SourceUse {
    /// Syntactic references in the emitted reachable definitions, counted once per definition.
    pub references: usize,
    pub materialization: Materialization,
    pub cte: Option<String>,
}
#[derive(Debug)]
pub(crate) struct Emission {
    pub sql: String,
    pub sources: BTreeMap<String, SourceUse>,
    pub derived_ctes: BTreeMap<String, String>,
    pub relation_uses: BTreeMap<String, usize>,
    pub parameters: BTreeSet<u32>,
}
struct Source {
    columns: Vec<String>,
    policy: Materialization,
}
#[derive(Default)]
pub(crate) struct Shape {
    sources: BTreeMap<String, Source>,
    queries: BTreeMap<String, Query>,
}
fn identifier(name: &str) -> Result<()> {
    let mut chars = name.bytes();
    if !chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == b'_')
        || !chars.all(|c| c.is_ascii_alphanumeric() || c == b'_')
    {
        return Err(format!("invalid identifier {name:?}"));
    }
    Ok(())
}
fn schema(columns: &[String]) -> Result<()> {
    if columns.is_empty() {
        return Err("an integer relation needs at least one column".into());
    }
    let mut seen = BTreeSet::new();
    for column in columns {
        identifier(column)?;
        if !seen.insert(column.to_ascii_lowercase()) {
            return Err(format!("duplicate column {column}"));
        }
    }
    Ok(())
}
fn quote(name: &str) -> String {
    format!("\"{name}\"")
}
impl Shape {
    /// Completed fact boundaries default to MATERIALIZED even with one use: an
    /// inline scan-only virtual table can otherwise be reopened per outer row.
    pub(crate) fn source(&mut self, name: &str, columns: &[&str]) -> Result<()> {
        self.source_with_policy(name, columns, Materialization::Materialized)
    }
    pub(crate) fn source_with_policy(
        &mut self,
        name: &str,
        columns: &[&str],
        policy: Materialization,
    ) -> Result<()> {
        self.available(name)?;
        let columns: Vec<_> = columns.iter().map(|c| (*c).to_owned()).collect();
        schema(&columns)?;
        self.sources.insert(name.into(), Source { columns, policy });
        Ok(())
    }
    pub(crate) fn define(&mut self, name: &str, query: Query) -> Result<()> {
        self.available(name)?;
        schema(&query.columns)?;
        self.queries.insert(name.into(), query);
        Ok(())
    }
    fn available(&self, name: &str) -> Result<()> {
        identifier(name)?;
        if self
            .sources
            .keys()
            .chain(self.queries.keys())
            .any(|other| other.eq_ignore_ascii_case(name))
        {
            return Err(format!("duplicate relation {name}"));
        }
        Ok(())
    }
    fn columns(&self, name: &str) -> Result<&[String]> {
        self.sources
            .get(name)
            .map(|s| s.columns.as_slice())
            .or_else(|| self.queries.get(name).map(|q| q.columns.as_slice()))
            .ok_or_else(|| format!("unbound relation {name}"))
    }
    fn visit(
        &self,
        name: &str,
        state: &mut BTreeMap<String, u8>,
        order: &mut Vec<String>,
        uses: &mut BTreeMap<String, usize>,
    ) -> Result<()> {
        self.columns(name)?;
        match state.get(name) {
            Some(1) => return Err(format!("relation cycle at {name}")),
            Some(2) => return Ok(()),
            _ => {}
        }
        state.insert(name.into(), 1);
        if let Some(query) = self.queries.get(name) {
            for branch in &query.branches {
                for binding in
                    std::iter::once(&branch.from).chain(branch.joins.iter().map(|j| &j.source))
                {
                    *uses.entry(binding.relation.clone()).or_default() += 1;
                    self.visit(&binding.relation, state, order, uses)?;
                }
            }
            order.push(name.into());
        }
        state.insert(name.into(), 2);
        Ok(())
    }
    pub(crate) fn emit(&self, root: &str) -> Result<Emission> {
        if !self.queries.contains_key(root) {
            return Err(format!("root {root} is not a query"));
        }
        // Validate unreachable definitions too, so malformed plans never become latent.
        let mut parameters = BTreeSet::new();
        for query in self.queries.values() {
            self.validate(query, &mut parameters)?;
        }
        let mut all_state = BTreeMap::new();
        for name in self.queries.keys() {
            self.visit(name, &mut all_state, &mut Vec::new(), &mut BTreeMap::new())?;
        }
        let (mut order, mut uses) = (Vec::new(), BTreeMap::new());
        self.visit(root, &mut BTreeMap::new(), &mut order, &mut uses)?;
        let mut reserved: BTreeSet<_> = self
            .sources
            .keys()
            .chain(self.queries.keys())
            .map(|name| name.to_ascii_lowercase())
            .collect();
        let mut names = BTreeMap::new();
        let mut sources = BTreeMap::new();
        let mut ctes = Vec::new();
        for (name, source) in &self.sources {
            let Some(&references) = uses.get(name) else {
                continue;
            };
            let materialized = source.policy == Materialization::Materialized || references > 1;
            let cte = materialized.then(|| fresh_cte(&mut reserved, "shape_source"));
            if let Some(cte) = &cte {
                let columns = source
                    .columns
                    .iter()
                    .map(|c| quote(c))
                    .collect::<Vec<_>>()
                    .join(", ");
                ctes.push(format!(
                    "{} AS MATERIALIZED (SELECT {columns} FROM {})",
                    quote(cte),
                    quote(name)
                ));
                names.insert(name.clone(), cte.clone());
            } else {
                names.insert(name.clone(), name.clone());
            }
            sources.insert(
                name.clone(),
                SourceUse {
                    references,
                    materialization: if materialized {
                        Materialization::Materialized
                    } else {
                        Materialization::Inline
                    },
                    cte,
                },
            );
        }
        let mut derived_ctes = BTreeMap::new();
        parameters.clear();
        for name in order.iter().filter(|name| name.as_str() != root) {
            let cte = fresh_cte(&mut reserved, "shape_query");
            self.validate(&self.queries[name], &mut parameters)?;
            ctes.push(format!(
                "{} AS MATERIALIZED ({})",
                quote(&cte),
                self.render(&self.queries[name], &names)
            ));
            names.insert(name.clone(), cte.clone());
            derived_ctes.insert(name.clone(), cte);
        }
        self.validate(&self.queries[root], &mut parameters)?;
        let body = self.render(&self.queries[root], &names);
        Ok(Emission {
            sql: if ctes.is_empty() {
                body
            } else {
                format!("WITH {} {body}", ctes.join(", "))
            },
            sources,
            derived_ctes,
            relation_uses: uses,
            parameters,
        })
    }
    fn validate(&self, query: &Query, parameters: &mut BTreeSet<u32>) -> Result<()> {
        if query.branches.is_empty() {
            return Err("a union needs at least one branch".into());
        }
        for branch in &query.branches {
            if branch.projection.len() != query.columns.len() {
                return Err("projection row width differs from schema".into());
            }
            let mut aliases = BTreeMap::new();
            let bind =
                |source: &Binding, aliases: &mut BTreeMap<String, Vec<String>>| -> Result<()> {
                    identifier(&source.alias)?;
                    if aliases
                        .keys()
                        .any(|alias| alias.eq_ignore_ascii_case(&source.alias))
                    {
                        return Err(format!("duplicate alias {}", source.alias));
                    }
                    aliases.insert(
                        source.alias.clone(),
                        self.columns(&source.relation)?.to_vec(),
                    );
                    Ok(())
                };
            bind(&branch.from, &mut aliases)?;
            for join in &branch.joins {
                let prior: BTreeSet<_> = aliases.keys().cloned().collect();
                bind(&join.source, &mut aliases)?;
                if join.on.is_empty() {
                    return Err("an equijoin requires keys".into());
                }
                for (a, b) in &join.on {
                    column(a, &aliases)?;
                    column(b, &aliases)?;
                    if !((a.alias == join.source.alias && prior.contains(&b.alias))
                        || (b.alias == join.source.alias && prior.contains(&a.alias)))
                    {
                        return Err(
                            "join keys must connect the new alias to an earlier alias".into()
                        );
                    }
                }
            }
            for expr in branch
                .projection
                .iter()
                .chain(branch.predicates.iter().flat_map(|(a, b)| [a, b]))
            {
                match expr {
                    Expr::Column(c) => column(c, &aliases)?,
                    Expr::Parameter(0) => return Err("parameter indices begin at 1".into()),
                    Expr::Parameter(index) => {
                        parameters.insert(*index);
                    }
                    Expr::Integer(_) => {}
                }
            }
        }
        Ok(())
    }
    fn render(&self, query: &Query, names: &BTreeMap<String, String>) -> String {
        let expr = |expr: &Expr| match expr {
            Expr::Column(c) => format!("{}.{}", quote(&c.alias), quote(&c.name)),
            Expr::Parameter(i) => format!("?{i}"),
            Expr::Integer(i) => i.to_string(),
        };
        let binding = |binding: &Binding| {
            format!(
                "{} AS {}",
                quote(&names[&binding.relation]),
                quote(&binding.alias)
            )
        };
        query
            .branches
            .iter()
            .map(|branch| {
                let projection = branch
                    .projection
                    .iter()
                    .zip(&query.columns)
                    .map(|(e, c)| format!("{} AS {}", expr(e), quote(c)))
                    .collect::<Vec<_>>()
                    .join(", ");
                let distinct = if query.branches.len() == 1 {
                    "DISTINCT "
                } else {
                    ""
                };
                let mut sql = format!(
                    "SELECT {distinct}{projection} FROM {}",
                    binding(&branch.from)
                );
                for join in &branch.joins {
                    sql.push_str(&format!(
                        " JOIN {} ON {}",
                        binding(&join.source),
                        join.on
                            .iter()
                            .map(|(a, b)| format!(
                                "{} = {}",
                                expr(&Expr::Column(a.clone())),
                                expr(&Expr::Column(b.clone()))
                            ))
                            .collect::<Vec<_>>()
                            .join(" AND ")
                    ));
                }
                if !branch.predicates.is_empty() {
                    sql.push_str(&format!(
                        " WHERE {}",
                        branch
                            .predicates
                            .iter()
                            .map(|(a, b)| format!("{} = {}", expr(a), expr(b)))
                            .collect::<Vec<_>>()
                            .join(" AND ")
                    ));
                }
                sql
            })
            .collect::<Vec<_>>()
            .join(" UNION ")
    }
}
fn column(column: &Column, aliases: &BTreeMap<String, Vec<String>>) -> Result<()> {
    identifier(&column.name)?;
    if !aliases
        .get(&column.alias)
        .is_some_and(|cols| cols.contains(&column.name))
    {
        return Err(format!(
            "out-of-scope column {}.{}",
            column.alias, column.name
        ));
    }
    Ok(())
}

fn fresh_cte(reserved: &mut BTreeSet<String>, prefix: &str) -> String {
    for index in 0.. {
        let name = format!("{prefix}_{index}");
        if reserved.insert(name.clone()) {
            return name;
        }
    }
    unreachable!()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn select(relation: &str, alias: &str, columns: &[&str]) -> Select {
        Select {
            from: Binding::new(relation, alias),
            joins: vec![],
            projection: columns.iter().map(|c| Expr::column(alias, c)).collect(),
            predicates: vec![],
        }
    }
    fn query(columns: &[&str], branches: Vec<Select>) -> Query {
        Query {
            columns: columns.iter().map(|c| (*c).into()).collect(),
            branches,
        }
    }
    #[test]
    fn joins_union_parameters_and_shared_sources_execute_as_integer_sets() {
        let mut shape = Shape::default();
        shape
            .source_with_policy("sites", &["doc", "node", "symbol"], Materialization::Inline)
            .unwrap();
        shape.source("wanted", &["label", "symbol"]).unwrap();
        let mut branch = select("sites", "s", &["doc", "node"]);
        branch.joins.push(Join {
            source: Binding::new("wanted", "w"),
            on: vec![(Column::new("s", "symbol"), Column::new("w", "symbol"))],
        });
        branch
            .predicates
            .push((Expr::column("w", "label"), Expr::Parameter(1)));
        let mut second = branch.clone();
        second.predicates = vec![(Expr::column("w", "label"), Expr::Integer(2))];
        shape
            .define("result", query(&["doc", "node"], vec![branch, second]))
            .unwrap();
        let emitted = shape.emit("result").unwrap();
        assert_eq!(emitted.sql, shape.emit("result").unwrap().sql);
        assert_eq!(emitted.sources["sites"].references, 2);
        assert_eq!(
            emitted.sources["wanted"].materialization,
            Materialization::Materialized
        );
        assert_eq!(emitted.parameters, BTreeSet::from([1]));
        assert_eq!(emitted.sql.matches("AS MATERIALIZED").count(), 2);
        assert!(
            !emitted.sql.contains("SELECT DISTINCT"),
            "UNION supplies set semantics"
        );
        let db = rusqlite::Connection::open_in_memory().unwrap();
        let mut sites = super::super::relation::Relation::new(&["doc", "node", "symbol"]);
        for row in [[1, 10, 7], [1, 10, 7], [2, 20, 8], [3, 30, 9]] {
            sites.push(&row);
        }
        let sites = sites.bind(&db, "sites").unwrap();
        let mut wanted = super::super::relation::Relation::new(&["label", "symbol"]);
        for row in [[1, 7], [2, 7], [2, 8]] {
            wanted.push(&row);
        }
        let wanted = wanted.bind(&db, "wanted").unwrap();
        let mut statement = db.prepare(&emitted.sql).unwrap();
        let mut rows = statement
            .query_map([1], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
            })
            .unwrap()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap();
        rows.sort_unstable();
        assert_eq!(rows, vec![(1, 10), (2, 20)]);
        assert_eq!(sites.filters.load(std::sync::atomic::Ordering::Relaxed), 1);
        assert_eq!(wanted.filters.load(std::sync::atomic::Ordering::Relaxed), 1);
    }
    #[test]
    fn named_derived_relations_materialize_once_and_do_not_shadow_sources() {
        let mut shape = Shape::default();
        shape.source("shape_source_0", &["x"]).unwrap();
        shape
            .define(
                "middle",
                query(&["x"], vec![select("shape_source_0", "a", &["x"])]),
            )
            .unwrap();
        shape
            .define(
                "root",
                query(
                    &["x"],
                    vec![select("middle", "m", &["x"]), select("middle", "n", &["x"])],
                ),
            )
            .unwrap();
        let emitted = shape.emit("root").unwrap();
        assert_eq!(emitted.derived_ctes.len(), 1);
        assert_eq!(emitted.relation_uses["middle"], 2);
        assert_eq!(emitted.sources["shape_source_0"].references, 1);
        assert_ne!(
            emitted.sources["shape_source_0"].cte.as_deref(),
            Some("shape_source_0")
        );
        let db = rusqlite::Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE shape_source_0(x INTEGER); INSERT INTO shape_source_0 VALUES(4),(4);",
        )
        .unwrap();
        let values = db
            .prepare(&emitted.sql)
            .unwrap()
            .query_map([], |row| row.get::<_, i64>(0))
            .unwrap()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(values, vec![4]);
        let mut inline = Shape::default();
        inline
            .source_with_policy("plain", &["x"], Materialization::Inline)
            .unwrap();
        inline
            .define("q", query(&["x"], vec![select("plain", "p", &["x"])]))
            .unwrap();
        assert!(inline.emit("q").unwrap().sources["plain"].cte.is_none());
    }
    #[test]
    fn rejects_bad_columns_scopes_widths_names_unbound_relations_and_cycles() {
        for (bad, expected) in [
            (select("a", "a", &["missing"]), "out-of-scope"),
            (select("unknown", "a", &["x"]), "unbound"),
        ] {
            let mut shape = Shape::default();
            shape.source("a", &["x"]).unwrap();
            shape.define("q", query(&["x"], vec![bad])).unwrap();
            assert!(shape.emit("q").unwrap_err().contains(expected));
        }
        let mut shape = Shape::default();
        shape.source("a", &["x"]).unwrap();
        let mut bad = select("a", "a", &["x"]);
        bad.projection.push(Expr::Integer(1));
        shape.define("width", query(&["x"], vec![bad])).unwrap();
        assert!(shape.emit("width").unwrap_err().contains("width"));
        let mut shape = Shape::default();
        shape
            .define("a", query(&["x"], vec![select("b", "b", &["x"])]))
            .unwrap();
        shape
            .define("b", query(&["x"], vec![select("a", "a", &["x"])]))
            .unwrap();
        assert!(shape.emit("a").unwrap_err().contains("cycle"));
        let mut shape = Shape::default();
        assert!(shape.source("bad-name", &["x"]).is_err());
        assert!(shape.source("valid", &["x", "X"]).is_err());
        shape.source("a", &["x"]).unwrap();
        let mut branch = select("a", "a", &["x"]);
        branch.joins.push(Join {
            source: Binding::new("a", "b"),
            on: vec![(Column::new("later", "x"), Column::new("b", "x"))],
        });
        shape.define("q", query(&["x"], vec![branch])).unwrap();
        assert!(shape.emit("q").unwrap_err().contains("out-of-scope"));
    }
}
