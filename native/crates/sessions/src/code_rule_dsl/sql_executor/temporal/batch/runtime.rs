//! Connection-local callbacks and schemas stay installed between batch executions.
use super::super::cold_loop::rows::{Roots, Rows};
use super::*;

#[derive(Clone, Default)]
pub struct Active(Arc<parking_lot::Mutex<Option<Arc<parking_lot::Mutex<Arena>>>>>);
impl Active {
    pub fn arena(&self) -> rusqlite::Result<Arc<parking_lot::Mutex<Arena>>> {
        self.0
            .lock()
            .clone()
            .ok_or_else(|| sql_error("SQL wave runtime is idle"))
    }
}
enum Kind {
    Current,
    Prior,
    FoldChanges,
}
struct View {
    active: Active,
    kind: Kind,
    failure: Option<String>,
}
impl Roots for View {
    fn sources_at(
        &mut self,
        state: i64,
        slot: usize,
    ) -> rusqlite::Result<Option<(usize, Vec<Arc<dyn Source>>)>> {
        let arena = self.active.arena()?;
        let mut arena = arena.lock();
        match self.kind {
            Kind::Current => arena.sources_at(state, slot),
            Kind::FoldChanges => arena.fold_sources(state, slot),
            Kind::Prior => Ok(None),
        }
    }
    fn root_at(&mut self, state: i64, slot: usize) -> rusqlite::Result<versions::Version> {
        let arena = self.active.arena()?;
        let mut arena = arena.lock();
        match self.kind {
            Kind::Current => arena.root_at(state, slot),
            Kind::Prior => {
                arena.check_handle(state)?;
                arena
                    .previous_candidate(slot)
                    .cloned()
                    .ok_or_else(|| sql_error("missing SQL wave candidate history"))
            }
            Kind::FoldChanges => Err(sql_error("fold differences require indexed sources")),
        }
    }
    fn failure(&mut self) -> &mut Option<String> {
        &mut self.failure
    }
}
pub struct Runtime {
    _modules: Modules,
    active: Active,
    views: Vec<Arc<parking_lot::Mutex<View>>>,
}
pub struct Guard(Active);
impl Drop for Guard {
    fn drop(&mut self) {
        self.0.0.lock().take();
    }
}
impl Runtime {
    pub fn new(db: &rusqlite::Connection) -> Result<Self> {
        let modules = Modules::default();
        let active = Active::default();
        let mut views = Vec::new();
        for (name, estimate, kind) in [
            (ROWS, 4096, Kind::Current),
            (FRONTIER, 16, Kind::Current),
            (CANDIDATES, 128, Kind::Current),
            (CHANGES, 16, Kind::Current),
            (PRIOR, 128, Kind::Prior),
            (FOLD_CHANGES, 16, Kind::FoldChanges),
        ] {
            let view = Arc::new(parking_lot::Mutex::new(View {
                active: active.clone(),
                kind,
                failure: None,
            }));
            modules
                .bind(db, name, Arc::new(Rows(view.clone(), 120, Some(estimate))))
                .map_err(error)?;
            views.push(view);
        }
        let next = active.clone();
        db.create_scalar_function(NEXT, 1, FunctionFlags::SQLITE_UTF8, move |c| {
            next.arena()?.lock().next(c.get(0)?)
        })
        .map_err(error)?;
        let reuse = active.clone();
        db.create_scalar_function(REUSE, 1, FunctionFlags::SQLITE_UTF8, move |c| {
            reuse.arena()?.lock().reuse(c.get(0)?)
        })
        .map_err(error)?;
        db.create_aggregate_function(
            COLLECT,
            -1,
            FunctionFlags::SQLITE_UTF8,
            Collect(active.clone()),
        )
        .map_err(error)?;
        Ok(Self {
            _modules: modules,
            active,
            views,
        })
    }
    pub fn enter(&self, arena: Arc<parking_lot::Mutex<Arena>>) -> Result<Guard> {
        if self.active.0.lock().is_some() {
            return Err(error("SQL wave runtime already has an active execution"));
        }
        for view in &self.views {
            view.lock().failure = None;
        }
        *self.active.0.lock() = Some(arena);
        Ok(Guard(self.active.clone()))
    }
}
