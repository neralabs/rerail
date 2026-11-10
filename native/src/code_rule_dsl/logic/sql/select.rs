//! The planner's SQL as a typed query (list step 20): a rule body's SELECT block and its CTEs.
//! It is built from `SqlExpression`s over named relations, checked, then rendered. The check
//! requires that the statement reads only relations the program declares, CTEs defined before
//! they are read, and columns of the FROM items in scope, including correlated subqueries.
//!
//! N's relational graph (`sql_ast::SqlAst`, one node per operator) suits N's fact families. The
//! planner already emits flat Selinger-ordered blocks whose join order SQLite must keep
//! (`CROSS JOIN`), so it gets a block form here; the expressions and values are shared.

use super::sql_ast::SqlExpression;
use std::collections::{BTreeMap, BTreeSet};

/// What a FROM item reads.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(in crate::code_rule_dsl) enum Relation {
    /// A base relation of the program's facts, in schema `schema` (T's attached file `f.`, or the bound
    /// vtabs `main.`).
    Input { schema: String, rel: String },
    /// A derived relation's table, or its semi-naive delta (`delta_<rel>`).
    Derived { rel: String, delta: bool },
    /// A wave's batch input, read from its source.
    Batch { rel: String, source: Source },
    /// A table the caller binds to a relation (the native set fixpoint's full or delta vtab).
    Bound { rel: String, table: String },
    /// The statistics' document intervals (`stat_docrange`: `d`, `lo`, `hi`).
    DocRange { schema: String },
    /// `rdl_int(i)`: the integers 1..=4096, for splits at every `.`.
    Integers,
    /// A CTE of the same query.
    Cte(String),
}

/// Where a batch input is read: `columns` of `table`, each as `c{i}` (`positional`) or under its own name,
/// and where `batched` only the rows of the chunk's batch (`batch = :batch`).
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(in crate::code_rule_dsl) struct Source {
    pub(in crate::code_rule_dsl) table: String,
    pub(in crate::code_rule_dsl) columns: Vec<String>,
    pub(in crate::code_rule_dsl) positional: bool,
    pub(in crate::code_rule_dsl) batched: bool,
}

impl Source {
    /// `columns` of `table`, read as `c0`, `c1`, ….
    pub(in crate::code_rule_dsl) fn positional(table: &str, columns: &[&str]) -> Self {
        Self { table: table.to_owned(), columns: columns.iter().map(|c| (*c).to_owned()).collect(), positional: true, batched: false }
    }

    /// `columns` of `table` under their own names.
    pub(in crate::code_rule_dsl) fn named(table: &str, columns: &[&str]) -> Self {
        Self { positional: false, ..Self::positional(table, columns) }
    }

    /// Only the rows of the chunk's batch.
    pub(in crate::code_rule_dsl) fn batched(self) -> Self {
        Self { batched: true, ..self }
    }

    pub(in crate::code_rule_dsl) fn render(&self) -> String {
        let columns = self
            .columns
            .iter()
            .enumerate()
            .map(|(i, column)| if self.positional && *column != format!("c{i}") { format!("{column} AS c{i}") } else { column.clone() })
            .collect::<Vec<_>>()
            .join(", ");
        let filter = if self.batched { " WHERE batch = :batch" } else { "" };
        format!("(SELECT {columns} FROM {}{filter})", self.table)
    }

    /// Its table and columns are identifiers (a table may be schema-qualified).
    fn check(&self) -> Result<(), String> {
        let identifier = |name: &str| {
            let mut bytes = name.bytes();
            bytes.next().is_some_and(|b| b.is_ascii_alphabetic() || b == b'_') && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
        };
        if self.columns.is_empty() {
            return Err(format!("the source {} has no columns", self.table));
        }
        if !self.table.split('.').all(identifier) || self.table.split('.').count() > 2 {
            return Err(format!("{:?} is not a table", self.table));
        }
        match self.columns.iter().find(|column| !identifier(column)) {
            Some(column) => Err(format!("{column:?} is not a column of {}", self.table)),
            None => Ok(()),
        }
    }
}

/// The columns of the planner's base relations (T's input schema). Others are `c0`, `c1`, ….
pub(in crate::code_rule_dsl) fn input_columns(rel: &str) -> Option<&'static [&'static str]> {
    Some(match rel {
        "node" => &["d", "n", "kind", "start", "end_"],
        "line" => &["n", "l"],
        "doc" => &["d", "path", "module"],
        "child" => &["p", "c", "idx"],
        "field" => &["p", "f", "c"],
        "text" | "name" | "str" | "strprefix" | "value" => &["n", "s"],
        "intval" => &["n", "v"],
        "op_at" => &["n", "i", "s"],
        "op" | "op_t" | "comment" | "static_value" => &["n", "s"],
        // `core.rr`'s (`facts::vtab::syntax`, decision 0046).
        "syntax_error" => &["d", "l", "msg"],
        "value_type" => &["v", "t"],
        "spread" => &["n"],
        "next_sibling" => &["a", "b"],
        "visit_index" => &["n", "i"],
        "node_order" => &["n", "k"],
        "ast_depth" => &["n", "k"],
        "line_count" => &["d", "k"],
        "end_line" => &["n", "l"],
        "col" => &["n", "c"],
        "docof" => &["n", "d"],
        "source" => &["d", "sp"],
        "src_line" => &["d", "l", "sp"],
        "split_line" | "lf_line" => &["d", "l", "t"],
        "str_part" => &["lit", "i", "v", "at", "spec"],
        "str_part_word" => &["lit", "i", "w", "at", "spec"],
        "str_part_start" => &["lit", "i", "l", "c"],
        "first_token" | "last_token" => &["n", "t"],
        "token_before" | "token_after" => &["x", "t"],
        "tree_digest" | "def_digest" => &["n", "s"],
        "rx_span" => &["p", "c", "j", "g", "m"],
        "ast_match" => &["p", "n", "g", "c"],
        // Decision 0055's (`facts::vtab::syntax::guests`).
        "guest" => &["g", "host", "at", "grammar"],
        "doc_role" => &["d", "r"],
        "doc_root" => &["d", "n"],
        "tape_of" => &["d", "t"],
        "parsed" => &["s", "grammar", "root"],
        "sub_tape" => &["at", "grammar", "t"],
        "host_node" => &["n", "h"],
        // Decision 0058's tape operations.
        "parse_of" => &["x", "g", "t"],
        "unparse_of" => &["t", "x"],
        "splice_of" => &["t", "n", "t2", "t3"],
        // The repository inputs (`facts::repository`).
        "option" => &["name", "v"],
        "option_item" => &["name", "i", "v"],
        "file" => &["path"],
        "package" => &["dir", "name"],
        "package_field" => &["dir", "key", "v"],
        "package_for" => &["path", "dir"],
        "pkg" => &["d", "p"],
        "path_part" | "file_name" => &["d", "s"],
        "tape" => &["n", "par", "size", "depth"],
        "core_node" => &["d", "t", "n"],
        // Containment read off the tree (`facts::vtab::syntax::within`, decision 0050).
        "within" => &["n", "a"],
        "span_within" => &["inner", "outer"],
        // The transducer's `nearest` rows, and the tree axes' (`facts::vtab::syntax::with_nearest`).
        rel if crate::code_rule_dsl::logic::front::is_transduced(rel) => &["n", "a"],
        rel if rel.starts_with(crate::code_rule_dsl::logic::strings::RELATION) => &["s"],
        // An `ast` matcher's matched nodes (decision 0106).
        rel if rel.starts_with(crate::code_rule_dsl::logic::plan::corpus::AST_MATCHES) => &["n"],
        // The `py.*` families (`facts::vtab::syntax::PY`).
        rel if rel.starts_with("py_") => crate::code_rule_dsl::facts::vtab::syntax::PY.iter().position(|&p| p == rel).map(|i| crate::code_rule_dsl::facts::vtab::syntax::PY_COLUMNS[i])?,
        _ => return None,
    })
}

impl Relation {
    /// The relation the program names, if it is one (`Input`, `Derived`, `Batch`, `Bound`).
    pub(in crate::code_rule_dsl) fn program_relation(&self) -> Option<&str> {
        match self {
            Self::Input { rel, .. } | Self::Derived { rel, .. } | Self::Batch { rel, .. } | Self::Bound { rel, .. } => Some(rel),
            _ => None,
        }
    }

    pub(in crate::code_rule_dsl) fn render(&self) -> String {
        match self {
            Self::Input { schema, rel } => format!("{schema}{}", super::ident(rel)),
            Self::Derived { rel, delta: false } => super::ident(rel).into_owned(),
            Self::Derived { rel, delta: true } => format!("delta_{rel}"),
            Self::Batch { source, .. } => source.render(),
            Self::Bound { table, .. } => table.clone(),
            Self::DocRange { schema } => format!("{schema}stat_docrange"),
            Self::Integers => "rdl_int".to_owned(),
            Self::Cte(name) => name.clone(),
        }
    }
}

/// How a FROM item after the first is joined: `CROSS JOIN` keeps the planner's order (its conditions are in
/// the WHERE clause); `JOIN … ON` leaves the order to SQLite.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::code_rule_dsl) enum Join {
    Cross,
    On(Vec<SqlExpression>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::code_rule_dsl) struct FromItem {
    pub(in crate::code_rule_dsl) relation: Relation,
    pub(in crate::code_rule_dsl) alias: String,
    pub(in crate::code_rule_dsl) join: Join,
}

/// One SELECT block.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(in crate::code_rule_dsl) struct Select {
    pub(in crate::code_rule_dsl) distinct: bool,
    pub(in crate::code_rule_dsl) columns: Vec<(SqlExpression, Option<String>)>,
    pub(in crate::code_rule_dsl) from: Vec<FromItem>,
    pub(in crate::code_rule_dsl) predicates: Vec<SqlExpression>,
}

/// A CTE: its arms are joined by `UNION ALL` (or `UNION` where `distinct`); a later arm may read the CTE
/// itself (a recursive walk). `materialized` fences it (`AS MATERIALIZED`): evaluated once, not inlined.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::code_rule_dsl) struct Cte {
    pub(in crate::code_rule_dsl) name: String,
    pub(in crate::code_rule_dsl) columns: Option<Vec<String>>,
    pub(in crate::code_rule_dsl) arms: Vec<Select>,
    pub(in crate::code_rule_dsl) distinct: bool,
    pub(in crate::code_rule_dsl) materialized: bool,
}

impl Cte {
    /// A walk's CTE: its arms under `UNION ALL`, inlined.
    pub(in crate::code_rule_dsl) fn walk(name: impl Into<String>, columns: Option<Vec<String>>, arms: Vec<Select>) -> Self {
        Self { name: name.into(), columns, arms, distinct: false, materialized: false }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(in crate::code_rule_dsl) struct Query {
    pub(in crate::code_rule_dsl) ctes: Vec<Cte>,
    pub(in crate::code_rule_dsl) body: Select,
}

impl FromItem {
    pub(in crate::code_rule_dsl) fn cross(relation: Relation, alias: impl Into<String>) -> Self {
        Self { relation, alias: alias.into(), join: Join::Cross }
    }
}

impl Select {
    /// Session plans precede materialization. Leave their join order to SQLite, whose tables
    /// and indexes are available when each wave is prepared. Correlated subqueries keep scope.
    pub(in crate::code_rule_dsl) fn sqlite_order(&mut self) {
        let derived = self.from.iter().filter(|i| matches!(i.relation, Relation::Derived { .. })).count();
        let delta = self.from.iter().any(|i| matches!(i.relation, Relation::Derived { delta: true, .. }));
        let cte = self.from.iter().any(|i| matches!(i.relation, Relation::Cte(_)));
        // Base sources already supply native probe estimates. A single delta is the wave's
        // anchor: scanning the entire base relation each wave loses semi-naive scaling.
        let keep_anchor = !cte && (derived == 0 || (derived == 1 && delta));
        for item in &mut self.from {
            if !keep_anchor && item.join == Join::Cross { item.join = Join::On(Vec::new()); }
        }
        let ons = self.from.iter_mut().flat_map(|item| match &mut item.join {
            Join::On(on) => on.iter_mut().collect::<Vec<_>>(), Join::Cross => Vec::new(),
        });
        for expression in self.columns.iter_mut().map(|(e, _)| e).chain(&mut self.predicates).chain(ons) {
            expression.visit_mut(&mut |e| {
                if let SqlExpression::Exists { query, .. } | SqlExpression::Scalar(query) = e { query.sqlite_order(); }
            });
        }
    }

    pub(in crate::code_rule_dsl) fn render(&self) -> String {
        let columns = self
            .columns
            .iter()
            .map(|(expression, alias)| match alias {
                Some(alias) => format!("{} AS {alias}", expression.render()),
                None => expression.render(),
            })
            .collect::<Vec<_>>()
            .join(", ");
        let mut sql = format!("SELECT {}{columns}", if self.distinct { "DISTINCT " } else { "" });
        for (at, item) in self.from.iter().enumerate() {
            let source = format!("{} {}", item.relation.render(), item.alias);
            match (at, &item.join) {
                (0, _) => sql += &format!(" FROM {source}"),
                (_, Join::Cross) => sql += &format!(" CROSS JOIN {source}"),
                (_, Join::On(on)) if on.is_empty() => sql += &format!(" JOIN {source}"),
                (_, Join::On(on)) => sql += &format!(" JOIN {source} ON {}", conjunction(on)),
            }
        }
        if !self.predicates.is_empty() {
            sql += &format!(" WHERE {}", conjunction(&self.predicates));
        }
        sql
    }

    /// Each FROM item's relation, in subqueries too.
    pub(in crate::code_rule_dsl) fn relations_mut(&mut self, f: &mut dyn FnMut(&mut Relation)) {
        for item in &mut self.from {
            f(&mut item.relation);
        }
        let Self { columns, from, predicates, .. } = self;
        let ons = from.iter_mut().flat_map(|item| match &mut item.join {
            Join::On(on) => on.iter_mut().collect::<Vec<_>>(),
            Join::Cross => Vec::new(),
        });
        for expression in columns.iter_mut().map(|(expression, _)| expression).chain(predicates.iter_mut()).chain(ons) {
            expression.visit_mut(&mut |inner| {
                if let SqlExpression::Exists { query, .. } | SqlExpression::Scalar(query) = inner {
                    query.relations_mut(f);
                }
            });
        }
    }

    /// The output columns' names: the aliases, or a bare column's name.
    fn names(&self) -> Vec<Option<String>> {
        self.columns
            .iter()
            .map(|(expression, alias)| match (alias, expression) {
                (Some(alias), _) => Some(alias.clone()),
                (None, SqlExpression::Qualified { column, .. }) => Some(column.clone()),
                _ => None,
            })
            .collect()
    }
}

fn conjunction(predicates: &[SqlExpression]) -> String {
    predicates.iter().map(SqlExpression::render).collect::<Vec<_>>().join(" AND ")
}

impl Query {
    pub(in crate::code_rule_dsl) fn render(&self) -> String {
        if self.ctes.is_empty() {
            return self.body.render();
        }
        let ctes = self
            .ctes
            .iter()
            .map(|cte| {
                let columns = cte.columns.as_ref().map_or_else(String::new, |columns| format!("({})", columns.join(", ")));
                let arms = cte.arms.iter().map(Select::render).collect::<Vec<_>>().join(if cte.distinct { " UNION " } else { " UNION ALL " });
                format!("{}{columns} AS {}({arms})", cte.name, if cte.materialized { "MATERIALIZED " } else { "" })
            })
            .collect::<Vec<_>>()
            .join(", ");
        format!("WITH RECURSIVE {ctes} {}", self.body.render())
    }

    /// The program relations the statement reads, sorted.
    pub(in crate::code_rule_dsl) fn reads(&self) -> BTreeSet<&Relation> {
        let mut reads = BTreeSet::new();
        fn select<'a>(block: &'a Select, reads: &mut BTreeSet<&'a Relation>) {
            reads.extend(block.from.iter().map(|item| &item.relation).filter(|relation| relation.program_relation().is_some()));
            let mut expressions = block.columns.iter().map(|(expression, _)| expression).chain(&block.predicates).collect::<Vec<_>>();
            expressions.extend(block.from.iter().flat_map(|item| match &item.join {
                Join::On(on) => on.iter().collect(),
                Join::Cross => Vec::new(),
            }));
            for expression in expressions {
                expression.visit(&mut |inner| {
                    if let SqlExpression::Exists { query, .. } | SqlExpression::Scalar(query) = inner {
                        select(query, reads);
                    }
                });
            }
        }
        self.ctes.iter().flat_map(|cte| &cte.arms).chain([&self.body]).for_each(|block| select(block, &mut reads));
        reads
    }

    /// The statement reads only relations `declared` accepts, CTEs before or while they are defined, and
    /// columns its FROM items have, in scope.
    pub(in crate::code_rule_dsl) fn check(&self, declared: &dyn Fn(&Relation) -> bool) -> Result<(), String> {
        let mut ctes = BTreeMap::<&str, Vec<Option<String>>>::new();
        for cte in &self.ctes {
            if ctes.contains_key(cte.name.as_str()) {
                return Err(format!("CTE {} is defined twice", cte.name));
            }
            let first = cte.arms.first().ok_or_else(|| format!("CTE {} has no arms", cte.name))?;
            let columns = cte.columns.clone().map_or_else(|| first.names(), |columns| columns.into_iter().map(Some).collect());
            if let Some(arm) = cte.arms.iter().find(|arm| arm.columns.len() != columns.len()) {
                return Err(format!("CTE {}: an arm has {} columns, not {}", cte.name, arm.columns.len(), columns.len()));
            }
            // The first arm is its seed; later arms may read the CTE (recursion).
            check_select(first, &[], &ctes, declared)?;
            ctes.insert(&cte.name, columns);
            for arm in &cte.arms[1..] {
                check_select(arm, &[], &ctes, declared)?;
            }
        }
        check_select(&self.body, &[], &ctes, declared)
    }
}

/// The columns of a FROM item: named, or positional (`c0`, `c1`, …).
enum Columns<'a> {
    Named(Vec<&'a str>),
    Positional,
}

fn columns<'a>(relation: &'a Relation, ctes: &'a BTreeMap<&str, Vec<Option<String>>>) -> Result<Columns<'a>, String> {
    Ok(match relation {
        Relation::Input { rel, .. } => input_columns(rel).map_or(Columns::Positional, |names| Columns::Named(names.to_vec())),
        // A batch source declares its own SQL projection. Even a familiar fact relation may
        // be supplied positionally, and an extension may supply arbitrary named columns.
        Relation::Batch { source, .. } if source.positional => Columns::Positional,
        Relation::Batch { source, .. } => Columns::Named(source.columns.iter().map(String::as_str).collect()),
        Relation::Derived { .. } | Relation::Bound { .. } => Columns::Positional,
        Relation::DocRange { .. } => Columns::Named(vec!["d", "lo", "hi"]),
        Relation::Integers => Columns::Named(vec!["i"]),
        Relation::Cte(name) => Columns::Named(
            ctes.get(name.as_str()).ok_or_else(|| format!("CTE {name} is read before it is defined"))?.iter().flatten().map(String::as_str).collect(),
        ),
    })
}

fn check_select(
    block: &Select,
    outer: &[(&str, &Columns<'_>)],
    ctes: &BTreeMap<&str, Vec<Option<String>>>,
    declared: &dyn Fn(&Relation) -> bool,
) -> Result<(), String> {
    let mut frame = Vec::new();
    for item in &block.from {
        if item.relation.program_relation().is_some() && !declared(&item.relation) {
            return Err(format!("{:?} is not a relation of the program", item.relation));
        }
        if let Relation::Batch { source, .. } = &item.relation {
            source.check()?;
        }
        if frame.iter().any(|(alias, _)| *alias == item.alias) {
            return Err(format!("alias {} is used twice", item.alias));
        }
        frame.push((item.alias.as_str(), columns(&item.relation, ctes)?));
    }
    let scope = frame.iter().rev().map(|(alias, columns)| (*alias, columns)).chain(outer.iter().copied()).collect::<Vec<_>>();
    let mut expressions = block.columns.iter().map(|(expression, _)| expression).chain(&block.predicates).collect::<Vec<_>>();
    expressions.extend(block.from.iter().flat_map(|item| match &item.join {
        Join::On(on) => on.iter().collect(),
        Join::Cross => Vec::new(),
    }));
    let mut failure = None;
    let mut inner = Vec::new();
    for expression in expressions {
        expression.visit(&mut |expression| match expression {
            SqlExpression::Qualified { alias, column } => {
                let found = scope.iter().find(|(name, _)| name == alias);
                let known = match found {
                    None => false,
                    Some((_, Columns::Named(names))) => names.contains(&column.as_str()),
                    Some((_, Columns::Positional)) => column.strip_prefix('c').is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())),
                };
                if !known && failure.is_none() {
                    failure = Some(format!("{alias}.{column} names no column in scope"));
                }
            }
            SqlExpression::Exists { query, .. } | SqlExpression::Scalar(query) => inner.push(query),
            SqlExpression::Column(_) | SqlExpression::Function { .. } => {
                if failure.is_none() {
                    failure = Some("the planner's SQL names columns by alias and functions by bare name".to_owned());
                }
            }
            _ => {}
        });
    }
    if let Some(failure) = failure {
        return Err(failure);
    }
    inner.into_iter().try_for_each(|query| check_select(query, &scope, ctes, declared))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_rule_dsl::logic::sql::sql_ast::{SqlBinaryOperator, SqlValue};

    fn column(alias: &str, column: &str) -> SqlExpression {
        SqlExpression::Qualified { alias: alias.to_owned(), column: column.to_owned() }
    }

    fn equal(left: SqlExpression, right: SqlExpression) -> SqlExpression {
        SqlExpression::Binary { operator: SqlBinaryOperator::Equal, left: Box::new(left), right: Box::new(right) }
    }

    #[test]
    fn a_block_renders_checks_and_lists_its_reads() {
        let input = Relation::Input { schema: "f.".to_owned(), rel: "node".to_owned() };
        let derived = Relation::Derived { rel: "calls".to_owned(), delta: true };
        let query = Query {
            ctes: Vec::new(),
            body: Select {
                distinct: true,
                columns: vec![(column("t1", "n"), Some("c0".to_owned()))],
                from: vec![FromItem::cross(input.clone(), "t1"), FromItem::cross(derived.clone(), "t2")],
                predicates: vec![
                    equal(column("t2", "c0"), column("t1", "n")),
                    SqlExpression::Exists {
                        negated: true,
                        query: Box::new(Select {
                            columns: vec![(SqlExpression::Value(SqlValue::Integer(1)), None)],
                            from: vec![FromItem::cross(Relation::Derived { rel: "seen".to_owned(), delta: false }, "t3")],
                            predicates: vec![equal(column("t3", "c0"), column("t1", "d"))],
                            ..Select::default()
                        }),
                    },
                ],
            },
        };
        assert_eq!(
            query.render(),
            "SELECT DISTINCT t1.n AS c0 FROM f.node t1 CROSS JOIN delta_calls t2 WHERE (t2.c0 = t1.n) AND NOT EXISTS (SELECT 1 FROM seen t3 WHERE (t3.c0 = t1.d))"
        );
        query.check(&|_| true).unwrap();
        assert_eq!(query.reads().into_iter().filter_map(Relation::program_relation).collect::<Vec<_>>(), ["node", "calls", "seen"]);
        // An undeclared relation, an unknown column and an alias out of scope are refused.
        assert!(query.check(&|relation| relation.program_relation() != Some("seen")).unwrap_err().contains("seen"));
        let mut wrong = query.clone();
        wrong.body.columns[0].0 = column("t1", "nope");
        assert_eq!(wrong.check(&|_| true).unwrap_err(), "t1.nope names no column in scope");
        let mut outside = query;
        outside.body.predicates[0] = equal(column("t3", "c0"), column("t1", "n"));
        assert!(outside.check(&|_| true).is_err());
    }

    #[test]
    fn a_recursive_cte_reads_itself_and_nothing_reads_it_before() {
        let tape = || Relation::Input { schema: "main.".to_owned(), rel: "tape".to_owned() };
        let walk = Cte::walk(
            "walk0",
            Some(vec!["v".to_owned(), "cur".to_owned()]),
            vec![
                Select {
                    columns: vec![(column("t", "n"), None), (column("t", "par"), None)],
                    from: vec![FromItem::cross(tape(), "t")],
                    ..Select::default()
                },
                Select {
                    columns: vec![(column("w", "v"), None), (column("t", "par"), None)],
                    from: vec![
                        FromItem::cross(Relation::Cte("walk0".to_owned()), "w"),
                        FromItem { relation: tape(), alias: "t".to_owned(), join: Join::On(vec![equal(column("t", "n"), column("w", "cur"))]) },
                    ],
                    ..Select::default()
                },
            ],
        );
        let body = Select {
            columns: vec![(column("walk0", "cur"), Some("c0".to_owned()))],
            from: vec![FromItem::cross(Relation::Cte("walk0".to_owned()), "walk0")],
            ..Select::default()
        };
        let query = Query { ctes: vec![walk.clone()], body };
        query.check(&|_| true).unwrap();
        assert_eq!(
            query.render(),
            "WITH RECURSIVE walk0(v, cur) AS (SELECT t.n, t.par FROM main.tape t UNION ALL SELECT w.v, t.par FROM walk0 w JOIN main.tape t ON (t.n = w.cur)) SELECT walk0.cur AS c0 FROM walk0 walk0"
        );
        let mut early = query;
        early.ctes[0].arms[0].from[0] = FromItem::cross(Relation::Cte("walk0".to_owned()), "t");
        assert!(early.check(&|_| true).unwrap_err().contains("before it is defined"));
    }
}
