//! One batch scan of a clause's selected root domain, before relational filtering.
use super::*;
pub(in crate::code_rule_dsl::sql) fn register(
    db: &Connection,
    name: &CStr,
    active: Active,
    estimate: Option<u64>,
) -> Result<()> {
    const MODULE: Module<'static, Table> = Module::eponymous_only_module();
    db.create_module(name, &MODULE, Some((active, estimate, false)))
}
pub(in crate::code_rule_dsl::sql) fn register_files(
    db: &Connection,
    name: &CStr,
    active: Active,
) -> Result<()> {
    const MODULE: Module<'static, Table> = Module::eponymous_only_module();
    db.create_module(name, &MODULE, Some((active, None, true)))
}
#[repr(C)]
struct Table {
    base: sqlite3_vtab,
    active: Active,
    estimate: Option<u64>,
    files_only: bool,
}
// SAFETY: SQLite base is first and the batch slot is owned.
unsafe impl<'v> VTab<'v> for Table {
    type Aux = (Active, Option<u64>, bool);
    type Cursor = Cursor<'v>;
    fn connect(
        db: &mut VTabConnection,
        aux: Option<&Self::Aux>,
        _: &[u8],
        _: &[u8],
        _: &[u8],
        _: &[&[u8]],
    ) -> Result<(Cow<'static, CStr>, Self)> {
        db.config(VTabConfig::DirectOnly)?;
        let (active, estimate, files_only) =
            aux.ok_or_else(|| invalid("missing candidate slot"))?;
        Ok((
            Cow::Borrowed(c"CREATE TABLE x(file INTEGER,node INTEGER,request INTEGER)"),
            Self {
                base: sqlite3_vtab::default(),
                active: active.clone(),
                estimate: *estimate,
                files_only: *files_only,
            },
        ))
    }
    fn best_index(&self, info: &mut IndexInfo) -> Result<bool> {
        let constraints = info
            .constraints()
            .enumerate()
            .filter(|(_, c)| {
                c.is_usable() && c.operator() == IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_EQ
            })
            .map(|(i, c)| (i, c.column()))
            .collect::<Vec<_>>();
        let file = constraints.iter().find(|(_, c)| *c == 0);
        let node = constraints.iter().find(|(_, c)| *c == 1);
        let mut plan = 0;
        if let Some(&(i, _)) = file {
            info.constraint_usage(i).set_argv_index(1);
            info.constraint_usage(i).set_omit(true);
            plan = 1;
        }
        if !self.files_only
            && plan == 1
            && let Some(&(i, _)) = node
        {
            info.constraint_usage(i).set_argv_index(2);
            info.constraint_usage(i).set_omit(true);
            plan = 2;
        }
        info.set_idx_num(plan);
        let count = self
            .estimate
            .map_or(i64::MAX, |v| v.min(i64::MAX as u64) as i64);
        let count = if plan == 2 { 1 } else { count };
        info.set_estimated_rows(count);
        info.set_estimated_cost(count.max(1) as f64);
        Ok(true)
    }
    fn open(&'v mut self) -> Result<Cursor<'v>> {
        Ok(Cursor {
            base: sqlite3_vtab_cursor::default(),
            active: self.active.clone(),
            rows: vec![],
            at: 0,
            files_only: self.files_only,
            domains: BTreeMap::new(),
            owner: std::sync::Weak::new(),
            phantom: PhantomData,
        })
    }
}
#[repr(C)]
struct Cursor<'v> {
    base: sqlite3_vtab_cursor,
    active: Active,
    rows: Vec<Request>,
    at: usize,
    files_only: bool,
    domains: BTreeMap<usize, Option<Arc<[u32]>>>,
    owner: std::sync::Weak<Fragment>,
    phantom: PhantomData<&'v Table>,
}
// SAFETY: SQLite base is first; rows and active slot remain owned by this cursor.
unsafe impl VTabCursor for Cursor<'_> {
    fn filter(&mut self, plan: c_int, _: Option<&str>, args: &Filters<'_>) -> Result<()> {
        let (batch, f) = self
            .active
            .read()
            .unwrap()
            .clone()
            .ok_or_else(|| invalid("no candidate batch"))?;
        let file = if plan >= 1 {
            args.get::<Option<i64>>(0)?
        } else {
            None
        };
        let node = if plan == 2 {
            args.get::<Option<i64>>(1)?
        } else {
            None
        };
        if plan >= 1
            && !self
                .owner
                .upgrade()
                .is_some_and(|old| Arc::ptr_eq(&old, &f))
        {
            self.domains = f
                .scope
                .iter()
                .flatten()
                .map(|(file, roots)| {
                    let roots = roots.as_ref().map(|roots| {
                        if roots.windows(2).all(|p| p[0] <= p[1]) {
                            roots.clone()
                        } else {
                            let mut sorted = roots.to_vec();
                            sorted.sort_unstable();
                            Arc::from(sorted)
                        }
                    });
                    (*file, roots)
                })
                .collect();
            self.owner = Arc::downgrade(&f);
        }
        self.rows = if (plan >= 1 && file.is_none_or(|f| f < 0))
            || (plan == 2 && node.is_none_or(|n| n < 0 || n > u32::MAX as i64))
        {
            vec![]
        } else if let Some(scope) = &f.scope {
            if let Some(file) = file {
                let file = file as usize;
                self.rows = if let Some(roots) = self.domains.get(&file) {
                    if self.files_only {
                        vec![Request {
                            file: file as u32,
                            node: 0,
                            inputs: vec![],
                        }]
                    } else if let Some(node) = node {
                        if node as u32 >= batch.files[file].facts.len()
                            || roots
                                .as_ref()
                                .is_some_and(|r| r.binary_search(&(node as u32)).is_err())
                        {
                            vec![]
                        } else {
                            f.requests_for(&batch, &[(file, Some(Arc::from([node as u32])))])?
                        }
                    } else {
                        f.requests_for(&batch, &[(file, roots.clone())])?
                    }
                } else {
                    vec![]
                };
                self.at = 0;
                return Ok(());
            }
            let selected = scope
                .iter()
                .filter(|(f, _)| file.is_none_or(|id| *f as i64 == id));
            if self.files_only {
                selected
                    .map(|(file, _)| Request {
                        file: *file as u32,
                        node: 0,
                        inputs: vec![],
                    })
                    .collect()
            } else if let Some(node) = node {
                let mut rows = vec![];
                for (file, roots) in selected {
                    if node as u32 >= batch.files[*file].facts.len()
                        || roots.as_ref().is_some_and(|r| !r.contains(&(node as u32)))
                    {
                        continue;
                    }
                    rows.extend(
                        f.requests_for(&batch, &[(*file, Some(Arc::from([node as u32])))])?,
                    );
                }
                rows
            } else {
                f.requests_for(&batch, &selected.cloned().collect::<Vec<_>>())?
            }
        } else {
            f.requests
                .iter()
                .filter(|r| {
                    file.is_none_or(|f| r.file as i64 == f)
                        && node.is_none_or(|n| r.node as i64 == n)
                })
                .cloned()
                .collect()
        };
        self.at = 0;
        Ok(())
    }
    fn next(&mut self) -> Result<()> {
        self.at += 1;
        if self.at == self.rows.len() {
            // SQLite can keep an exhausted UNION arm open until the statement finishes.
            self.rows = Vec::new();
        }
        Ok(())
    }
    fn eof(&self) -> bool {
        self.at >= self.rows.len()
    }
    fn column(&self, ctx: &mut Context, col: c_int) -> Result<()> {
        match col {
            0 => ctx.set_result(&self.rows[self.at].file),
            1 => ctx.set_result(&self.rows[self.at].node),
            _ => ctx.set_result(
                &(((self.rows[self.at].file as i64) << 32) | self.rows[self.at].node as i64),
            ),
        }
    }
    fn rowid(&self) -> Result<i64> {
        Ok(self.at as i64)
    }
}
