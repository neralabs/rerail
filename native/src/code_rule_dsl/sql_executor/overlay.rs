//! Old-input probes combine the current native access paths with a small indexed deletion set.
use super::super::facts::vtab::{Modules, bound_source};
use super::*;

struct Overlay {
    current: Arc<dyn Source>,
    deleted: live::Live,
    probes: parking_lot::Mutex<(Vec<(bool, usize)>, HashMap<(bool, usize), usize>)>,
}
impl Overlay {
    fn ids(&self, current: Vec<usize>, deleted: Vec<usize>) -> Vec<usize> {
        let mut rows = self.probes.lock();
        current
            .into_iter()
            .map(|r| (false, r))
            .chain(deleted.into_iter().map(|r| (true, r)))
            .map(|key| {
                let id = match rows.1.get(&key) {
                    Some(&id) => id,
                    None => {
                        let id = rows.0.len();
                        rows.0.push(key);
                        rows.1.insert(key, id);
                        id
                    }
                };
                (1usize << 31) | id
            })
            .collect()
    }
}
impl Source for Overlay {
    fn columns(&self) -> Vec<String> {
        (0..self.current.columns().len())
            .map(|c| format!("c{c}"))
            .collect()
    }
    fn len(&self) -> usize {
        self.current.len() + self.deleted.len()
    }
    fn estimated_len(&self) -> usize {
        self.current
            .estimated_len()
            .saturating_add(self.deleted.len())
    }
    fn estimated_matches(&self, c: usize, batch: bool) -> usize {
        self.current
            .estimated_matches(c, batch)
            .saturating_add(self.deleted.estimated_matches(c, batch))
    }
    fn indexed(&self, c: usize) -> bool {
        self.current.indexed(c)
    }
    fn made_on_lookup(&self) -> bool {
        self.current.made_on_lookup()
    }
    fn lookup(&self, c: usize, values: &[i64]) -> Option<Vec<usize>> {
        Some(self.ids(
            self.current.lookup(c, values)?,
            self.deleted.lookup(c, values)?,
        ))
    }

    fn range_indexed(&self, c: usize) -> bool {
        self.current.range_indexed(c)
    }
    fn range_lookup(
        &self,
        c: usize,
        lo: Option<(i64, bool)>,
        hi: Option<(i64, bool)>,
    ) -> Option<Vec<usize>> {
        let deleted = (0..self.deleted.len())
            .filter(|&r| {
                let v = self.deleted.value(r, c);
                lo.is_none_or(|(b, inc)| v > b || inc && v == b)
                    && hi.is_none_or(|(b, inc)| v < b || inc && v == b)
            })
            .collect();
        Some(self.ids(self.current.range_lookup(c, lo, hi)?, deleted))
    }

    fn value(&self, r: usize, c: usize) -> i64 {
        if r & (1usize << 31) != 0 {
            let (deleted, row) = self.probes.lock().0[r & !(1usize << 31)];
            if deleted {
                self.deleted.value(row, c)
            } else {
                self.current.value(row, c)
            }
        } else if r < self.current.len() {
            self.current.value(r, c)
        } else {
            self.deleted.value(r - self.current.len(), c)
        }
    }
}

/// Scratch modules are removed on success, failure, or a budget interruption.
pub(super) struct Overlays<'a> {
    db: &'a rusqlite::Connection,
    modules: Modules,
    names: Vec<String>,
}
impl<'a> Overlays<'a> {
    pub fn new(db: &'a rusqlite::Connection) -> Self {
        Self {
            db,
            modules: Modules::default(),
            names: vec![],
        }
    }
    pub fn bind(&mut self, relation: &str, minus: &str) -> Result<Option<(String, usize)>> {
        let Some(current) = bound_source(self.db, relation) else {
            return Ok(None);
        };
        let width = current.columns().len();
        let mut deleted = Relation::new(
            &(0..width)
                .map(|c| format!("c{c}"))
                .collect::<Vec<_>>()
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
        );
        let mut stmt = self
            .db
            .prepare(&format!("SELECT * FROM {}", quote(minus)))
            .map_err(error)?;
        let rows = stmt
            .query_map([], |r| {
                (0..width)
                    .map(|c| r.get::<_, i64>(c))
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .map_err(error)?;
        for row in rows {
            deleted.push(&row.map_err(error)?);
        }
        let source = Arc::new(Overlay {
            current,
            deleted: live::Live::from_source(&deleted),
            probes: parking_lot::Mutex::new((vec![], HashMap::new())),
        });
        let name = format!("__rr_old_input_{}", self.names.len());
        self.modules.bind(self.db, &name, source).map_err(error)?;
        self.names.push(name.clone());
        Ok(Some((name, width)))
    }
}
impl Drop for Overlays<'_> {
    fn drop(&mut self) {
        for name in &self.names {
            let _ = self.modules.unregister(self.db, name);
        }
    }
}
