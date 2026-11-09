//! Discover changed partitions from positive old/new witnesses, including scalar read sites.
use super::*;
impl Partitioned {
    fn allowed(&self, e: &E, block: &Select) -> bool {
        let mut valid = plain(e);
        e.visit(&mut |e| {
            if let E::Qualified { alias, column } = e {
                if let Some(r) = block
                    .from
                    .iter()
                    .find(|i| &i.alias == alias)
                    .and_then(|i| i.relation.program_relation())
                    .filter(|r| self.keys.contains_key(*r))
                {
                    valid &= column
                        .strip_prefix('c')
                        .and_then(|c| c.parse::<usize>().ok())
                        .is_some_and(|c| c < self.keys[r]);
                }
            }
        });
        valid
    }
    fn relax(&self, body: &mut Select) {
        for item in &mut body.from {
            if let Join::On(on) = std::mem::replace(&mut item.join, Join::Cross) {
                body.predicates.extend(on);
            }
        }
        body.predicates = body
            .predicates
            .iter()
            .filter(|e| self.allowed(e, body))
            .cloned()
            .collect();
    }
    fn variants(
        &self,
        body: &Select,
        head: &E,
        changes: &BTreeMap<String, Delta>,
        old: bool,
        out: &mut Vec<Query>,
    ) {
        for (at, item) in body.from.iter().enumerate() {
            let Some(r) = item
                .relation
                .program_relation()
                .filter(|r| !self.columns.contains_key(*r))
            else {
                continue;
            };
            let Some(d) = changes
                .get(r)
                .filter(|d| if old { d.deleted > 0 } else { d.inserted > 0 })
            else {
                continue;
            };
            let mut body = body.clone();
            self.relax(&mut body);
            body.columns = vec![(head.clone(), Some("c0".into()))];
            body.distinct = true;
            body.from[at].relation = source(if old { &d.minus } else { &d.plus }, &self.fields[r]);
            let anchor = body.from.remove(at);
            body.from.insert(0, anchor);
            out.push(Query { ctes: vec![], body });
        }
        // Only the selected scalar read site becomes a positive delta witness. Other
        // scalar sites stay absent: empty groups and negation must not require a row.
        for e in body
            .columns
            .iter()
            .map(|(e, _)| e)
            .chain(&body.predicates)
            .chain(body.from.iter().flat_map(|i| match &i.join {
                Join::On(on) => on.iter(),
                Join::Cross => [].iter(),
            }))
        {
            e.visit(&mut |e| {
                if let E::Scalar(inner) | E::Exists { query: inner, .. } = e {
                    let mut nested = body.clone();
                    self.relax(&mut nested);
                    let inner = inner.as_ref().clone();
                    nested.from.extend(inner.from);
                    nested.predicates.extend(inner.predicates);
                    // Recurse only into the inner expression sites, rather than revisiting
                    // the outer scalar expression that led here.
                    nested.columns = inner.columns;
                    self.variants(&nested, head, changes, old, out);
                }
            });
        }
    }
    pub(super) fn seed(
        &self,
        w: &mut Work<'_>,
        changes: &BTreeMap<String, Delta>,
        parts: &str,
        old: bool,
    ) -> Result<()> {
        let mut queries = Vec::new();
        for (s, q) in &self.rules {
            self.variants(
                &q.body,
                &q.body.columns[self.columns[&s.output]].0,
                changes,
                old,
                &mut queries,
            );
        }
        if queries.is_empty() {
            return Ok(());
        }
        let mut overlays = overlay::Overlays::new(w.db,w.profiler);
        let mut bindings = BTreeMap::new();
        let mut restored = Vec::new();
        if old {
            for (r, columns) in &self.fields {
                if self.columns.contains_key(r) {
                    continue;
                }
                let Some(d) = changes.get(r).filter(|d| d.inserted + d.deleted > 0) else {
                    continue;
                };
                if let Some((name, _)) = overlays.bind(r, &d.minus)? {
                    bindings.insert(r.clone(), name);
                } else {
                    let cols = names("", columns.len());
                    w.db.execute(
                        &format!(
                            "DELETE FROM {} WHERE ({cols}) IN(SELECT {cols} FROM {})",
                            quote(r),
                            quote(&d.plus)
                        ),
                        [],
                    )
                    .map_err(error)?;
                    w.db.execute(
                        &format!(
                            "INSERT OR IGNORE INTO {} SELECT * FROM {}",
                            quote(r),
                            quote(&d.minus)
                        ),
                        [],
                    )
                    .map_err(error)?;
                    restored.push(r.clone());
                }
            }
        }
        for mut q in queries {
            for item in &mut q.body.from {
                if let Some(r) = item.relation.program_relation() {
                    if let Some(name) = bindings.get(r) {
                        item.relation = source(name, &self.fields[r]);
                    }
                }
            }
            w.db.execute(&format!("INSERT OR IGNORE INTO {parts} {}", q.render()), [])
                .map_err(error)?;
        }
        for r in restored {
            let d = &changes[&r];
            let cols = names("", self.fields[&r].len());
            w.db.execute(
                &format!(
                    "DELETE FROM {} WHERE ({cols}) IN(SELECT {cols} FROM {})",
                    quote(&r),
                    quote(&d.minus)
                ),
                [],
            )
            .map_err(error)?;
            w.db.execute(
                &format!(
                    "INSERT OR IGNORE INTO {} SELECT * FROM {}",
                    quote(&r),
                    quote(&d.plus)
                ),
                [],
            )
            .map_err(error)?;
        }
        Ok(())
    }
}
