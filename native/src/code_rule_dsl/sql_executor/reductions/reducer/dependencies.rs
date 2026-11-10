//! Index aggregate-dependent correlation keys by their current context value.
use super::*;

impl Reducer {
    pub fn dynamic(&self) -> bool {
        self.native_keys || self.outer_keys.iter().any(|e| !plain(e))
    }
    fn dependencies(&self) -> String {
        self.dependencies.clone()
    }
    pub(super) fn initialize_dependencies(
        &self,
        db: &rusqlite::Connection,
        rule: &Rule,
        storage: Option<&versions::Store>,
    ) -> Result<()> {
        let table = self.dependencies();
        let keys = (0..self.keys())
            .map(|i| format!("k{i} INTEGER"))
            .collect::<Vec<_>>()
            .join(",");
        if let Some(storage) = storage {
            let readonly=super::super::super::super::facts::vtab::bound_source(db,&rule.contexts).is_some_and(|s|!s.writable());
            let create=if readonly {versions::Store::create_readonly} else {versions::Store::create};
            create(storage,
                db,
                &table,
                versions::Schema::generated(&format!(
                    "{},{keys},PRIMARY KEY({})",
                    schema(rule.width),
                    names("", rule.width)
                ))?,
            )?;
            storage.restrict_indexes(
                &table,
                &(rule.width..rule.width + self.keys()).collect::<Vec<_>>(),
            )?;
        } else {
            db.execute_batch(&format!("CREATE TEMP TABLE {table}({},{keys},PRIMARY KEY({})) WITHOUT ROWID; CREATE INDEX {table}_keys ON {table}({})", schema(rule.width), names("",rule.width), (0..self.keys()).map(|i|format!("k{i}")).collect::<Vec<_>>().join(","))).map_err(error)?;
        }
        if let Some(storage)=storage {
            storage.load_select(db,&table,&self.dependency_query(rule,&rule.contexts),false)?;
        } else {
            execute(db, &self.dependency_insert(rule, &rule.contexts))?;
        }
        Ok(())
    }
    pub fn dependency_object(&self) -> Option<String> {
        self.dynamic().then(|| self.dependencies())
    }
    fn dependency_insert(&self, rule: &Rule, table: &str) -> String {
        format!("INSERT INTO {} {}",self.dependencies(),self.dependency_query(rule,table))
    }
    fn dependency_query(&self, rule: &Rule, table: &str) -> String {
        let keys = self
            .context_keys(rule, "rr_context")
            .iter()
            .map(E::render)
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "SELECT {},{keys} FROM {table} rr_context ORDER BY {}",
            names("rr_context", rule.width)
            ,names("rr_context",rule.width)
        )
    }
    pub fn mark_contexts(&self, w: &Work<'_>, rule: &Rule, keys: &str, dirty: &str) -> Result<()> {
        execute(
            w.db,
            &format!(
                "INSERT OR IGNORE INTO {dirty} {}",
                self.affected_contexts(rule, keys)
            ),
        )?;
        Ok(())
    }
    pub fn affected_contexts(&self, rule: &Rule, keys: &str) -> String {
        let same = if self.dynamic() {
            (0..self.keys())
                .map(|i| format!("c.k{i}=k.c{i}"))
                .collect::<Vec<_>>()
                .join(" AND ")
        } else {
            let positions = self.context_keys(rule, "c");
            if positions.is_empty() {
                "k.c0=1".into()
            } else {
                positions
                    .iter()
                    .enumerate()
                    .map(|(i, e)| {
                        format!(
                            "{}={}k.c{i}",
                            e.render(),
                            if matches!(e, E::Qualified { .. }) {
                                ""
                            } else {
                                "+"
                            }
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(" AND ")
            }
        };
        let context = if self.dynamic() {
            self.dependencies()
        } else {
            rule.contexts.clone()
        };
        format!(
            "SELECT {} FROM {keys} k CROSS JOIN {context} c WHERE {same}",
            names("c", rule.width)
        )
    }
    pub fn refresh_dependencies(
        &self,
        w: &Work<'_>,
        rule: &Rule,
        dirty: &str,
        active: &str,
    ) -> Result<()> {
        if self.dynamic() {
            if let Some(store)=w.versions.filter(|s|s.available(&self.dependencies).is_some()) {
                let mut phase=phase::Phase::default();
                let values=phase.relation(rule.width+self.keys(),&self.dependency_query(rule,active));
                phase.patch(store,&self.dependencies,dirty,&values)?;
                phase.execute(w)?;
                return Ok(());
            }
            execute(
                w.db,
                &format!(
                    "DELETE FROM {} WHERE ({}) IN (SELECT {} FROM {dirty})",
                    self.dependencies(),
                    names("", rule.width),
                    names("", rule.width)
                ),
            )?;
            execute(w.db, &self.dependency_insert(rule, active))?;
        }
        Ok(())
    }
}
