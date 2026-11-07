//! Normalize checked rules into indexed positive contexts and scalar reducer lookups.
use super::super::super::logic::sql::sql_ast::SqlUnaryOperator as U;
use super::*;

#[derive(Clone)]
pub(super) struct Atom {
    pub at: usize,
    pub relation: String,
    pub offset: usize,
    pub columns: Vec<String>,
}
#[derive(Clone)]
pub(super) struct Rule {
    pub contexts: String,
    pub contributions: String,
    pub width: usize,
    pub atoms: Vec<Atom>,
    pub context_query: Query,
    pub reducers: Vec<Reducer>,
    evaluation: Query,
    fields: BTreeMap<(String, String), usize>,
}

pub(super) fn column(alias: &str, name: &str) -> E {
    E::Qualified {
        alias: alias.into(),
        column: name.into(),
    }
}
pub(super) fn integer(n: i64) -> E {
    E::Value(V::Integer(n))
}
pub(super) fn names(alias: &str, width: usize) -> String {
    (0..width)
        .map(|i| {
            if alias.is_empty() {
                format!("c{i}")
            } else {
                format!("{alias}.c{i}")
            }
        })
        .collect::<Vec<_>>()
        .join(",")
}
pub(super) fn expressions(block: &Select) -> Vec<&E> {
    block
        .columns
        .iter()
        .map(|(e, _)| e)
        .chain(&block.predicates)
        .chain(block.from.iter().flat_map(|f| match &f.join {
            Join::On(on) => on.iter().collect(),
            Join::Cross => vec![],
        }))
        .collect()
}
pub(super) fn replace(block: &mut Select, f: &mut dyn FnMut(&mut E)) {
    for (e, _) in &mut block.columns {
        e.visit_mut(f);
    }
    for e in &mut block.predicates {
        e.visit_mut(f);
    }
    for item in &mut block.from {
        if let Join::On(on) = &mut item.join {
            for e in on {
                e.visit_mut(f);
            }
        }
    }
}
pub(super) fn plain(e: &E) -> bool {
    let mut yes = true;
    e.visit(&mut |e| {
        if matches!(e, E::Scalar(_) | E::Exists { .. } | E::Parameter(_)) {
            yes = false;
        }
    });
    yes
}
pub(super) fn fields(db: &rusqlite::Connection, r: &Rel) -> Result<Vec<String>> {
    Ok(db
        .prepare(&format!("SELECT * FROM {} LIMIT 0", r.render()))
        .map_err(error)?
        .column_names()
        .iter()
        .map(|s| s.to_string())
        .collect())
}
/// A delta has positional columns even when the original source exposes native column names.
pub(super) fn source(table: &str, columns: &[String]) -> Rel {
    let projection = columns
        .iter()
        .enumerate()
        .map(|(i, c)| format!("c{i} AS {}", quote(c)))
        .collect::<Vec<_>>()
        .join(",");
    Rel::Bound {
        rel: table.into(),
        table: format!("(SELECT {projection} FROM {})", quote(table)),
    }
}

pub(super) fn rule(
    db: &rusqlite::Connection,
    output: &str,
    id: usize,
    q: &Query,
) -> Result<Option<Rule>> {
    if !q.ctes.is_empty() {
        return Ok(None);
    }
    let mut atoms = Vec::new();
    let mut fields_by_name = BTreeMap::new();
    let mut width = 0;
    for (at, item) in q.body.from.iter().enumerate() {
        let Some(relation) = item.relation.program_relation() else {
            return Ok(None);
        };
        if matches!(item.relation, Rel::Derived { delta: true, .. }) {
            return Ok(None);
        }
        let columns = fields(db, &item.relation)?;
        for (i, c) in columns.iter().enumerate() {
            fields_by_name.insert((item.alias.clone(), c.clone()), width + i);
        }
        atoms.push(Atom {
            at,
            relation: relation.into(),
            offset: width,
            columns,
        });
        width += atoms.last().unwrap().columns.len();
    }
    let prefix = format!("__sql_state_{output}_r{id}");
    let mut occurrences = Vec::new();
    fn collect(e: &E, occurrences: &mut Vec<E>) {
        e.visit(&mut |e| {
            if let E::Scalar(query) | E::Exists { query, .. } = e {
                for nested in expressions(query) {
                    collect(nested, occurrences);
                }
                if !occurrences.contains(e) {
                    occurrences.push(e.clone());
                }
            }
        });
    }
    for e in expressions(&q.body) {
        collect(e, &mut occurrences);
    }
    let mut sites = Vec::new();
    for e in occurrences {
        let Some(reducer) = Reducer::plan(
            db,
            &format!("{prefix}_s{}", sites.len()),
            &e,
            &fields_by_name,
            &sites,
        )?
        else {
            return Ok(None);
        };
        sites.push(reducer);
    }
    let mut context = q.clone();
    context.body.distinct = true;
    context.body.columns = q
        .body
        .from
        .iter()
        .flat_map(|item| {
            fields_by_name
                .iter()
                .filter(|((alias, _), _)| alias == &item.alias)
                .map(|((alias, c), i)| (column(alias, c), Some(format!("c{i}"))))
        })
        .collect();
    // Map order must follow physical source positions, rather than lexical column names.
    context
        .body
        .columns
        .sort_by_key(|(_, name)| name.as_ref().unwrap()[1..].parse::<usize>().unwrap());
    if width == 0 {
        width = 1;
        context.body.columns = vec![(integer(1), Some("c0".into()))];
    }
    context.body.predicates.retain(plain);
    for item in &mut context.body.from {
        if let Join::On(on) = &mut item.join {
            on.retain(plain);
        }
    }
    let mut rule = Rule {
        contexts: format!("{prefix}_contexts"),
        contributions: format!("{prefix}_contributions"),
        width,
        atoms,
        context_query: context,
        reducers: sites,
        evaluation: q.clone(),
        fields: fields_by_name,
    };
    let mut evaluation = q.clone();
    replace(&mut evaluation.body, &mut |e| {
        if let Some(reducer) = rule.reducers.iter().find(|r| r.original == *e) {
            *e = reducer.lookup(&rule);
        }
        if let E::Qualified { alias, column } = e
            && let Some(i) = rule.fields.get(&(alias.clone(), column.clone()))
        {
            *e = plan::column("rr_context", &format!("c{i}"));
        }
    });
    for (head, _) in evaluation.body.columns.clone() {
        evaluation.body.predicates.push(E::Unary {
            operator: U::IsNotNull,
            input: Box::new(head),
        });
    }
    let mut columns = (0..width)
        .map(|i| {
            (
                column("rr_context", &format!("c{i}")),
                Some(format!("c{i}")),
            )
        })
        .collect::<Vec<_>>();
    columns.extend(
        evaluation
            .body
            .columns
            .into_iter()
            .enumerate()
            .map(|(i, (e, _))| (e, Some(format!("h{i}")))),
    );
    evaluation.body.columns = columns;
    evaluation.body.distinct = false;
    // Preserve ON predicates when replacing the positive join with its retained context.
    for item in &evaluation.body.from {
        if let Join::On(on) = &item.join {
            evaluation.body.predicates.extend(on.iter().cloned());
        }
    }
    evaluation.body.from = vec![];
    rule.evaluation = evaluation;
    Ok(Some(rule))
}

impl Rule {
    pub fn in_context(&self, e: &E, context: &str) -> E {
        let mut expression = e.clone();
        expression.visit_mut(&mut |e| {
            if let Some(reducer) = self.reducers.iter().find(|r| r.original == *e) {
                *e = reducer.lookup_at(self, context);
            }
            if let E::Qualified {
                alias,
                column: name,
            } = e
            {
                *e = column(
                    context,
                    &format!("c{}", self.fields[&(alias.clone(), name.clone())]),
                );
            }
        });
        expression
    }
    pub fn evaluation(&self, table: &str) -> Query {
        let mut query = self.evaluation.clone();
        query.body.from = vec![FromItem::cross(
            Rel::Bound {
                rel: table.into(),
                table: quote(table),
            },
            "rr_context",
        )];
        query
    }
    pub fn index(&self, db: &rusqlite::Connection, columns: &[usize]) -> Result<()> {
        let columns: Vec<_> = columns
            .iter()
            .copied()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        if columns.is_empty() || columns.iter().copied().eq(0..columns.len()) {
            return Ok(());
        }
        let suffix = columns
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("_");
        let cols = columns
            .iter()
            .map(|i| format!("c{i}"))
            .collect::<Vec<_>>()
            .join(",");
        db.execute_batch(&format!(
            "CREATE INDEX IF NOT EXISTS temp.{} ON {}({cols})",
            quote(&format!("{}_by_{suffix}", self.contexts)),
            quote(&self.contexts)
        ))
        .map_err(error)
    }
}
