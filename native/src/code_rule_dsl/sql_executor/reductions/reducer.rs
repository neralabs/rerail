//! Group counts, exact sums, and indexed value multiplicities.
use super::*;
mod dependencies;
mod lookup;
pub(in crate::code_rule_dsl::sql_executor) mod transition;
pub(super) mod combined;
#[derive(Clone, Copy)]
enum Kind {
    Count,
    Sum,
    Min,
    Max,
    Join,
    Matches(bool),
}
#[derive(Clone)]
pub(super) struct Reducer {
    combined: Option<combined::Shared>,
    pool_consumers: usize,
    transition_key: Option<Arc<str>>,
    pub original: E,
    kind: Kind,
    pub relation: String,
    pub outer_keys: Vec<E>,
    projection: Query,
    columns: Vec<String>,
    groups: String,
    dependencies: String,
    values: String,
    join: Option<E>,
    native_keys: bool,
}
impl Reducer {
    pub fn plan(
        db: &rusqlite::Connection,
        prefix: &str,
        original: &E,
        outer: &BTreeMap<(String, String), usize>,
        previous: &[Reducer],
        native: bool,
    ) -> Result<Option<Self>> {
        let (inner, kind, value) = match original {
            E::Exists { negated, query } => (query.as_ref(), Kind::Matches(*negated), integer(1)),
            E::Scalar(query) => {
                let [(value, _)] = query.columns.as_slice() else {
                    return Ok(None);
                };
                let (kind, term) = match value {
                    E::Call {
                        name: "count",
                        arguments,
                    } if arguments == &[E::All] => (Kind::Count, integer(1)),
                    E::Call {
                        name: "min",
                        arguments,
                    } if arguments.len() == 1 => (Kind::Min, arguments[0].clone()),
                    E::Call {
                        name: "max",
                        arguments,
                    } if arguments.len() == 1 => (Kind::Max, arguments[0].clone()),
                    E::Call {
                        name: "coalesce",
                        arguments,
                    } if arguments.len() == 2 && arguments[1] == integer(0) => {
                        let E::Call {
                            name: "sum" | "rr_exact_sum",
                            arguments,
                        } = &arguments[0]
                        else {
                            return Ok(None);
                        };
                        if arguments.len() != 1 {
                            return Ok(None);
                        }
                        (Kind::Sum, arguments[0].clone())
                    }
                    E::Call {
                        name: "sym_atom",
                        arguments,
                    } if arguments.len() == 1 => {
                        let E::Call {
                            name: "coalesce",
                            arguments,
                        } = &arguments[0]
                        else {
                            return Ok(None);
                        };
                        if arguments.len() != 2
                            || !matches!(
                                &arguments[0],
                                E::Call {
                                    name: "rr_join",
                                    ..
                                }
                            )
                        {
                            return Ok(None);
                        }
                        (Kind::Join, value.clone())
                    }
                    _ => return Ok(None),
                };
                (query.as_ref(), kind, term)
            }
            _ => return Ok(None),
        };
        let [item] = inner.from.as_slice() else {
            return Ok(None);
        };
        let Some(relation) = item.relation.program_relation() else {
            return Ok(None);
        };
        let mut local = Vec::new();
        let mut keys = Vec::new();
        let mut outer_keys = Vec::new();
        for predicate in &inner.predicates {
            let mut checked = predicate.clone();
            checked.visit_mut(&mut |e| {
                if previous.iter().any(|r| r.original == *e) {
                    *e = integer(1);
                }
            });
            if !plain(&checked) {
                return Ok(None);
            }
            let mut correlated = false;
            predicate.visit(&mut |e| {
                if matches!(e, E::Scalar(_) | E::Exists { .. }) {
                    correlated = true;
                }
                if let E::Qualified { alias, .. } = e
                    && alias != &item.alias
                {
                    correlated = true;
                }
            });
            if !correlated {
                local.push(predicate.clone());
                continue;
            }
            let E::Binary {
                operator: B::Equal,
                left,
                right,
            } = predicate
            else {
                return Ok(None);
            };
            let valid = |e: &E, inner: bool| {
                let mut e = e.clone();
                let mut dependency = false;
                if !inner {
                    e.visit_mut(&mut |e| {
                        if previous.iter().any(|r| r.original == *e) {
                            *e = integer(1);
                            dependency = true;
                        }
                    });
                }
                let mut valid = plain(&e);
                let mut field = false;
                e.visit(&mut |e| {
                    if let E::Qualified { alias, column } = e {
                        field = true;
                        valid &= if inner {
                            alias == &item.alias
                        } else {
                            outer.contains_key(&(alias.clone(), column.clone()))
                        };
                    }
                });
                valid && (field || dependency)
            };
            let pair = [
                (left.as_ref(), right.as_ref()),
                (right.as_ref(), left.as_ref()),
            ]
            .into_iter()
            .find(|(inner, other)| valid(inner, true) && valid(other, false));
            let Some((inner, other)) = pair else {
                return Ok(None);
            };
            keys.push(inner.clone());
            outer_keys.push(other.clone());
        }
        let mut value = value;
        // An outer ordering term is constant within each correlated group. Omitting that
        // constant preserves rr_join's ordering and duplicate elimination.
        if matches!(kind, Kind::Join) {
            value.visit_mut(&mut |e| {
                if let E::Call {
                    name: "rr_join",
                    arguments,
                } = e
                {
                    for order in arguments.iter_mut().skip(2) {
                        let mut outer_only = true;
                        order.visit(&mut |e| {
                            if let E::Qualified { alias, .. } = e {
                                outer_only &= alias != &item.alias;
                            }
                        });
                        if outer_only {
                            *order = integer(0);
                        }
                    }
                }
            });
        }
        let mut valid = plain(&value);
        value.visit(&mut |e| {
            if let E::Qualified { alias, .. } = e
                && alias != &item.alias
            {
                valid = false;
            }
        });
        if !valid {
            return Ok(None);
        }
        if keys.is_empty() {
            keys.push(integer(1));
        }
        // A NULL computed key cannot satisfy an equality correlation. Do not create an
        // unreachable group for it (group primary keys are non-null).
        for key in &keys {
            if !matches!(key, E::Qualified { .. } | E::Value(_)) {
                local.push(E::Unary {
                    operator: super::super::super::logic::sql::sql_ast::SqlUnaryOperator::IsNotNull,
                    input: Box::new(key.clone()),
                });
            }
        }
        let mut columns: Vec<_> = keys
            .into_iter()
            .enumerate()
            .map(|(i, e)| (e, Some(format!("c{i}"))))
            .collect();
        let fields = plan::fields(db, &item.relation)?;
        let join = if matches!(kind, Kind::Join) {
            let keys = columns.len();
            for field in &fields {
                columns.push((
                    column(&item.alias, field),
                    Some(format!("c{}", columns.len())),
                ));
            }
            value.visit_mut(&mut |e| {
                if let E::Qualified { column: field, .. } = e {
                    let at = fields
                        .iter()
                        .position(|f| f == field)
                        .expect("checked aggregate field");
                    *e = column("rr_join_values", &format!("c{}", keys + at));
                }
            });
            Some(value)
        } else {
            columns.push((value, Some(format!("c{}", columns.len()))));
            None
        };
        let projection = Query {
            ctes: vec![],
            body: Select {
                distinct: false,
                columns,
                from: vec![item.clone()],
                predicates: local,
            },
        };
        let native_keys = native && outer_keys.iter().any(|e| !matches!(e, E::Qualified { .. }));
        Ok(Some(Self {
            combined: None,
            pool_consumers: 1,
            transition_key: None,
            original: original.clone(),
            kind,
            relation: relation.into(),
            outer_keys,
            projection,
            columns: fields,
            groups: format!("{prefix}_groups"),
            dependencies: format!("{prefix}_dependencies"),
            values: format!("{prefix}_values"),
            join,
            native_keys,
        }))
    }
    pub(super) fn prepare_transition(&mut self) { self.transition_key=transition::descriptor(self); }
    fn keys(&self) -> usize {
        self.outer_keys.len().max(1)
    }
    fn width(&self) -> usize {
        self.projection.body.columns.len()
    }
    fn empty_join() -> E {
        E::Call {
            name: "sym_atom",
            arguments: vec![E::Value(V::Text(String::new()))],
        }
    }
    pub fn context_keys(&self, rule: &Rule, alias: &str) -> Vec<E> {
        self.outer_keys
            .iter()
            .map(|e| rule.in_context(e, alias))
            .collect()
    }
    pub fn index_context(
        &self,
        db: &rusqlite::Connection,
        rule: &Rule,
        storage: Option<&versions::Store>,
    ) -> Result<()> {
        if self.combined.as_ref().is_some_and(|c| !c.owner) { return Ok(()); }
        if self.dynamic() {
            return self.initialize_dependencies(db, rule, storage);
        }
        let expressions = self.context_keys(rule, "rr_context");
        if expressions.is_empty() {
            return Ok(());
        }
        if storage.is_some() {
            let mut columns = BTreeSet::new();
            for e in &expressions {
                e.visit(&mut |e| {
                    if let E::Qualified { alias, column } = e
                        && alias == "rr_context"
                        && let Some(c) = column
                            .strip_prefix('c')
                            .and_then(|s| s.parse::<usize>().ok())
                    {
                        columns.insert(c);
                    }
                });
            }
            return rule.index(db, &columns.into_iter().collect::<Vec<_>>());
        }
        let columns = expressions
            .into_iter()
            .map(|mut e| {
                e.visit_mut(&mut |e| {
                    if let E::Qualified { column, .. } = e {
                        *e = E::Column(super::super::super::logic::sql::sql_ast::SqlColumn {
                            relation: None,
                            name: column.clone(),
                        });
                    }
                });
                e.render()
            })
            .collect::<Vec<_>>()
            .join(",");
        db.execute_batch(&format!(
            "CREATE INDEX {} ON {}({columns})",
            quote(&self.combined.as_ref().map_or_else(||format!("{}_contexts",self.groups),|s|format!("{}_s{}_contexts",rule.contexts,s.members[0]))),
            quote(&rule.contexts)
        ))
        .map_err(error)
    }
    pub fn lookup(&self, rule: &Rule) -> E {
        self.lookup_at(rule, "rr_context")
    }
    pub fn lookup_at(&self, rule: &Rule, context: &str) -> E {
        let alias = "rr_reducer";
        let conditions = if self.outer_keys.is_empty() {
            vec![E::Binary {
                operator: B::Equal,
                left: Box::new(column(alias, "c0")),
                right: Box::new(integer(1)),
            }]
        } else {
            self.context_keys(rule, context)
                .iter()
                .enumerate()
                .map(|(i, e)| E::Binary {
                    operator: B::Equal,
                    left: Box::new(column(alias, &format!("c{i}"))),
                    right: Box::new(e.clone()),
                })
                .collect()
        };
        let scalar = E::Scalar(Box::new(Select {
            distinct: false,
            columns: vec![(
                if matches!(self.kind, Kind::Sum) {
                    E::Call {
                        name: "rr_sum_value",
                        arguments: vec![column(alias, "high"), column(alias, "value")],
                    }
                } else if self.combined.is_some() {
                    column(alias, match self.kind { Kind::Count => "n", Kind::Min => "minimum", Kind::Max => "maximum", _ => "value" })
                } else {
                    column(alias, "value")
                },
                None,
            )],
            from: vec![FromItem::cross(
                Rel::Bound {
                    rel: self.groups.clone(),
                    table: self.groups.clone(),
                },
                alias,
            )],
            predicates: conditions,
        }));
        match self.kind {
            Kind::Join => E::Call {
                name: "coalesce",
                arguments: vec![scalar, Self::empty_join()],
            },
            Kind::Count | Kind::Sum => E::Call {
                name: "coalesce",
                arguments: vec![scalar, integer(0)],
            },
            Kind::Matches(negated) => E::Binary {
                operator: if negated { B::Equal } else { B::NotEqual },
                left: Box::new(E::Call {
                    name: "coalesce",
                    arguments: vec![scalar, integer(0)],
                }),
                right: Box::new(integer(0)),
            },
            _ => scalar,
        }
    }
    pub fn initialize(
        &self,
        db: &rusqlite::Connection,
        objects: &mut Vec<(String, bool)>,
        storage: Option<&versions::Store>,
    ) -> Result<()> {
        if let Some(shared) = &self.combined { return combined::initialize(self, shared, db, objects, storage); }
        let keys = self.keys();
        let cols = names("", keys);
        let value = format!("c{keys}");
        let wide = if matches!(self.kind, Kind::Sum) {
            ",high INTEGER NOT NULL"
        } else {
            ""
        };
        owned(
            db,
            objects,
            &self.groups,
            &format!(
                "{},n INTEGER NOT NULL CHECK(n>0),value INTEGER NOT NULL{wide},PRIMARY KEY({cols})",
                schema(keys)
            ),
            storage,
        )?;
        let reduction = match self.kind {
            Kind::Count | Kind::Matches(_) => "count(*)".into(),
            Kind::Sum => format!("rr_sum_low({value},1,NULL),rr_sum_high({value},1,NULL)"),
            Kind::Min => format!("min({value})"),
            Kind::Max => format!("max({value})"),
            Kind::Join => self.join.as_ref().unwrap().render(),
        };
        insert(
            db,
            &self.groups,
            &format!(
                "SELECT {cols},count(*),{reduction} FROM ({}) rr_join_values GROUP BY {cols}",
                self.projection.render()
            ),
            storage,
        )?;
        if matches!(self.kind, Kind::Min | Kind::Max | Kind::Join) {
            owned(
                db,
                objects,
                &self.values,
                &format!(
                    "{},n INTEGER NOT NULL CHECK(n>0),PRIMARY KEY({})",
                    schema(self.width()),
                    names("", self.width())
                ),
                storage,
            )?;
            insert(
                db,
                &self.values,
                &format!(
                    "SELECT {},count(*) FROM ({}) GROUP BY {}",
                    names("", self.width()),
                    self.projection.render(),
                    names("", self.width())
                ),
                storage,
            )?;
        }
        Ok(())
    }
    pub fn apply(
        &self,
        w: &mut Work<'_>,
        changes: &BTreeMap<String, Delta>,
    ) -> Result<Option<(String, u64)>> {
        transition::apply(self, w, changes)
    }
    fn apply_uncached(&self, w: &mut Work<'_>, changes: &BTreeMap<String, Delta>) -> Result<Option<(String, u64)>> {
        if let Some(shared) = &self.combined { return combined::apply(self, shared, w, changes); }
        let Some(delta) = changes
            .get(&self.relation)
            .filter(|d| d.inserted + d.deleted > 0)
        else {
            return Ok(None);
        };
        let k = self.keys();
        let cols = names("", k);
        let val = format!("c{k}");
        let signed = temporary(w, self.width(), "weight INTEGER NOT NULL", None)?;
        for (table, sign, count) in [
            (&delta.plus, 1, delta.inserted),
            (&delta.minus, -1, delta.deleted),
        ] {
            if count == 0 {
                continue;
            }
            let mut q = self.projection.clone();
            q.body.from[0].relation = source(table, &self.columns);
            execute(
                w.db,
                &format!(
                    "INSERT INTO {signed} SELECT p.*,{sign} FROM ({}) p",
                    q.render()
                ),
            )?;
        }
        let keys = w.table(k)?;
        let touched = execute(
            w.db,
            &format!("INSERT INTO {keys} SELECT DISTINCT {cols} FROM {signed}"),
        )? as u64;
        if touched == 0 {
            return Ok(None);
        }
        let group_join = equal("g", "d", k);
        let wide = if matches!(self.kind, Kind::Sum) {
            ",high INTEGER,old_high INTEGER"
        } else {
            ""
        };
        let totals = temporary(
            w,
            k,
            &format!("n INTEGER NOT NULL CHECK(n>=0),value INTEGER,old_value INTEGER{wide}"),
            Some(k),
        )?;
        if matches!(self.kind, Kind::Min | Kind::Max | Kind::Join) {
            self.values_delta(w, &signed)?;
        }
        let sum_values = if matches!(self.kind, Kind::Sum) {
            let sums = temporary(
                w,
                k,
                "value INTEGER NOT NULL,high INTEGER NOT NULL",
                Some(k),
            )?;
            execute(
                w.db,
                &format!(
                    "INSERT INTO {sums} SELECT {cols},rr_sum_low(v,weight,high),rr_sum_high(v,weight,high) FROM (SELECT {cols},{val} AS v,weight,NULL AS high FROM {signed} UNION ALL SELECT {},g.value,1,g.high FROM {keys} k CROSS JOIN {} g WHERE {}) GROUP BY {cols}",
                    names("g", k),
                    self.groups,
                    equal("g", "k", k)
                ),
            )?;
            Some(sums)
        } else {
            None
        };
        let n = "(coalesce(g.n,0)+d.n)";
        let next = match self.kind {
            Kind::Join => format!(
                "(SELECT {} FROM {} rr_join_values WHERE {})",
                self.join.as_ref().unwrap().render(),
                self.values,
                equal("rr_join_values", "d", k)
            ),
            Kind::Count | Kind::Matches(_) => n.to_owned(),
            Kind::Sum => format!(
                "(SELECT s.value FROM {} s WHERE {})",
                sum_values.as_ref().unwrap(),
                equal("s", "d", k)
            ),
            Kind::Min | Kind::Max => format!(
                "(SELECT v.{val} FROM {} v WHERE {} ORDER BY v.{val} {} LIMIT 1)",
                self.values,
                equal("v", "d", k),
                if matches!(self.kind, Kind::Min) {
                    "ASC"
                } else {
                    "DESC"
                }
            ),
        };
        let high = sum_values
            .as_ref()
            .map(|table| {
                format!(
                    ",(SELECT s.high FROM {table} s WHERE {}),g.high",
                    equal("s", "d", k)
                )
            })
            .unwrap_or_default();
        execute(
            w.db,
            &format!(
                "INSERT INTO {totals} SELECT {},{n},{next},g.value{high} FROM (SELECT {cols},sum(weight) AS n FROM {signed} GROUP BY {cols}) d LEFT JOIN {} g ON {group_join}",
                names("d", k),
                self.groups
            ),
        )?;
        let changed = w.table(k)?;
        let differs = match self.kind {
            Kind::Join => format!(
                "coalesce(old_value,{0}) IS NOT coalesce(value,{0})",
                Self::empty_join().render()
            ),
            Kind::Count => "coalesce(old_value,0) IS NOT coalesce(value,0)".into(),
            Kind::Sum => "coalesce(old_value,0) IS NOT coalesce(value,0) OR coalesce(old_high,0) IS NOT coalesce(high,0)".into(),
            Kind::Matches(_) => "(coalesce(old_value,0)>0) <> (coalesce(value,0)>0)".into(),
            _ => "old_value IS NOT value".into(),
        };
        execute(
            w.db,
            &format!("INSERT INTO {changed} SELECT {cols} FROM {totals} WHERE {differs}"),
        )?;
        execute(
            w.db,
            &format!(
                "DELETE FROM {} WHERE ({cols}) IN (SELECT {cols} FROM {totals} WHERE n=0)",
                self.groups
            ),
        )?;
        let high = if matches!(self.kind, Kind::Sum) {
            ",high"
        } else {
            ""
        };
        execute(
            w.db,
            &format!(
                "INSERT OR REPLACE INTO {} SELECT {cols},n,value{high} FROM {totals} WHERE n>0",
                self.groups
            ),
        )?;
        Ok(Some((changed, touched)))
    }
    fn values_delta(&self, w: &mut Work<'_>, signed: &str) -> Result<()> {
        let width = self.width();
        let cols = names("", width);
        let totals = temporary(w, width, "n INTEGER NOT NULL CHECK(n>=0)", Some(width))?;
        execute(
            w.db,
            &format!(
                "INSERT INTO {totals} SELECT {},coalesce(v.n,0)+d.n FROM (SELECT {cols},sum(weight) AS n FROM {signed} GROUP BY {cols}) d LEFT JOIN {} v ON {}",
                names("d", width),
                self.values,
                equal("v", "d", width)
            ),
        )?;
        execute(
            w.db,
            &format!(
                "DELETE FROM {} WHERE ({cols}) IN (SELECT {cols} FROM {totals} WHERE n=0)",
                self.values
            ),
        )?;
        execute(
            w.db,
            &format!(
                "INSERT OR REPLACE INTO {} SELECT * FROM {totals} WHERE n>0",
                self.values
            ),
        )?;
        Ok(())
    }
}
