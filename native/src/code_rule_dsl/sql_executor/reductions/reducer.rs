//! Group counts, exact sums, and indexed value multiplicities.
use super::*;
#[derive(Clone, Copy)]
enum Kind {
    Count,
    Sum,
    Min,
    Max,
    Matches(bool),
}
#[derive(Clone)]
pub(super) struct Reducer {
    pub original: E,
    kind: Kind,
    pub relation: String,
    pub outer_keys: Vec<E>,
    projection: Query,
    columns: Vec<String>,
    groups: String,
    values: String,
}
impl Reducer {
    pub fn plan(
        db: &rusqlite::Connection,
        prefix: &str,
        original: &E,
        outer: &BTreeMap<(String, String), usize>,
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
                            name: "sum",
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
            if !plain(predicate) {
                return Ok(None);
            }
            let mut correlated = false;
            predicate.visit(&mut |e| {
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
            let pair=[(left.as_ref(),right.as_ref()),(right.as_ref(),left.as_ref())].into_iter().find(|(inner,other)|{
                matches!(inner,E::Qualified{alias,..} if alias==&item.alias)&&matches!(other,E::Qualified{alias,column} if outer.contains_key(&(alias.clone(),column.clone())))
            });
            let Some((inner, other)) = pair else {
                return Ok(None);
            };
            keys.push(inner.clone());
            outer_keys.push(other.clone());
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
        let mut columns: Vec<_> = keys
            .into_iter()
            .enumerate()
            .map(|(i, e)| (e, Some(format!("c{i}"))))
            .collect();
        columns.push((value, Some(format!("c{}", columns.len()))));
        let projection = Query {
            ctes: vec![],
            body: Select {
                distinct: false,
                columns,
                from: vec![item.clone()],
                predicates: local,
            },
        };
        Ok(Some(Self {
            original: original.clone(),
            kind,
            relation: relation.into(),
            outer_keys,
            projection,
            columns: plan::fields(db, &item.relation)?,
            groups: format!("{prefix}_groups"),
            values: format!("{prefix}_values"),
        }))
    }
    fn keys(&self) -> usize {
        self.outer_keys.len().max(1)
    }
    pub fn context_keys(&self, rule: &Rule) -> Vec<usize> {
        self.outer_keys
            .iter()
            .map(|e| rule.context_field(e))
            .collect()
    }
    pub fn lookup(&self, rule: &Rule) -> E {
        let alias = "rr_reducer";
        let conditions = if self.outer_keys.is_empty() {
            vec![E::Binary {
                operator: B::Equal,
                left: Box::new(column(alias, "c0")),
                right: Box::new(integer(1)),
            }]
        } else {
            self.context_keys(rule)
                .iter()
                .enumerate()
                .map(|(i, c)| E::Binary {
                    operator: B::Equal,
                    left: Box::new(column(alias, &format!("c{i}"))),
                    right: Box::new(column("rr_context", &format!("c{c}"))),
                })
                .collect()
        };
        let scalar = E::Scalar(Box::new(Select {
            distinct: false,
            columns: vec![(column(alias, "value"), None)],
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
    ) -> Result<()> {
        let keys = self.keys();
        let cols = names("", keys);
        let value = format!("c{keys}");
        owned(
            db,
            objects,
            &self.groups,
            &format!(
                "{},n INTEGER NOT NULL CHECK(n>0),value INTEGER NOT NULL,PRIMARY KEY({cols})",
                schema(keys)
            ),
        )?;
        let reduction = match self.kind {
            Kind::Count | Kind::Matches(_) => "count(*)".into(),
            Kind::Sum => format!("sum({value})"),
            Kind::Min => format!("min({value})"),
            Kind::Max => format!("max({value})"),
        };
        execute(
            db,
            &format!(
                "INSERT INTO {} SELECT {cols},count(*),{reduction} FROM ({}) GROUP BY {cols}",
                self.groups,
                self.projection.render()
            ),
        )?;
        if matches!(self.kind, Kind::Min | Kind::Max) {
            owned(
                db,
                objects,
                &self.values,
                &format!(
                    "{},n INTEGER NOT NULL CHECK(n>0),PRIMARY KEY({})",
                    schema(keys + 1),
                    names("", keys + 1)
                ),
            )?;
            execute(
                db,
                &format!(
                    "INSERT INTO {} SELECT {},count(*) FROM ({}) GROUP BY {}",
                    self.values,
                    names("", keys + 1),
                    self.projection.render(),
                    names("", keys + 1)
                ),
            )?;
        }
        Ok(())
    }
    pub fn apply(
        &self,
        w: &mut Work<'_>,
        changes: &BTreeMap<String, Delta>,
    ) -> Result<Option<(String, u64)>> {
        let Some(delta) = changes
            .get(&self.relation)
            .filter(|d| d.inserted + d.deleted > 0)
        else {
            return Ok(None);
        };
        let k = self.keys();
        let cols = names("", k);
        let val = format!("c{k}");
        let signed = temporary(w, k + 1, "weight INTEGER NOT NULL", None)?;
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
        let totals = temporary(
            w,
            k,
            "n INTEGER NOT NULL CHECK(n>=0),value INTEGER,old_value INTEGER",
            Some(k),
        )?;
        if matches!(self.kind, Kind::Min | Kind::Max) {
            self.values_delta(w, &signed)?;
        }
        let sum_values = if matches!(self.kind, Kind::Sum) {
            let sums = temporary(w, k, "value INTEGER NOT NULL", Some(k))?;
            execute(
                w.db,
                &format!(
                    "INSERT INTO {sums} SELECT {cols},rr_update_sum(v,weight) FROM (SELECT {cols},{val} AS v,weight FROM {signed} UNION ALL SELECT {},g.value,1 FROM {keys} k CROSS JOIN {} g WHERE {}) GROUP BY {cols}",
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
            Kind::Count | Kind::Matches(_) => n.to_owned(),
            Kind::Sum => format!(
                "(SELECT s.value FROM {} s WHERE {})",
                sum_values.unwrap(),
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
        execute(
            w.db,
            &format!(
                "INSERT INTO {totals} SELECT {},{n},{next},g.value FROM (SELECT {cols},sum(weight) AS n FROM {signed} GROUP BY {cols}) d LEFT JOIN {} g ON {group_join}",
                names("d", k),
                self.groups
            ),
        )?;
        let changed = w.table(k)?;
        let differs = match self.kind {
            Kind::Count | Kind::Sum => "coalesce(old_value,0) IS NOT coalesce(value,0)",
            Kind::Matches(_) => "(coalesce(old_value,0)>0) <> (coalesce(value,0)>0)",
            _ => "old_value IS NOT value",
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
        execute(
            w.db,
            &format!(
                "INSERT OR REPLACE INTO {} SELECT {cols},n,value FROM {totals} WHERE n>0",
                self.groups
            ),
        )?;
        Ok(Some((changed, touched)))
    }
    fn values_delta(&self, w: &mut Work<'_>, signed: &str) -> Result<()> {
        let width = self.keys() + 1;
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
