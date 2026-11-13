//! A shared connection's progress handler and authorizer. rusqlite sets SQLite's hooks through `&mut Connection`, and
//! a relational pool's connection is shared by its owner and its leases. So the connection registers one dispatcher
//! of each while it is still owned (`Hooks::register`), and whoever holds it points them at a closure, or at none,
//! through `&self` after. With no closure, the progress dispatcher interrupts nothing and the authorizer allows all,
//! as with no hook.
use parking_lot::Mutex;
use rusqlite::{
    Connection,
    hooks::{AuthContext, Authorization},
};
use std::sync::Arc;

type Progress = Box<dyn FnMut() -> bool + Send>;
type Authorizer = Box<dyn for<'r> FnMut(AuthContext<'r>) -> Authorization + Send>;

/// Virtual machine instructions between two calls of the progress closure: one relational work unit (a bounded
/// batch of 64 SQLite VM instructions).
pub(in crate::code_rule_dsl) const PROGRESS_STEPS: i32 = 64;

/// A connection's hook slots: none until `register`, and setting one then is an error, not a hook that never runs.
#[derive(Default)]
pub(in crate::code_rule_dsl) struct Hooks(Option<Slots>);

struct Slots {
    progress: Arc<Mutex<Option<Progress>>>,
    authorizer: Arc<Mutex<Option<Authorizer>>>,
}

impl Hooks {
    /// Registers the dispatchers on `connection`. Its owner calls this once, before sharing it.
    pub(in crate::code_rule_dsl) fn register(connection: &mut Connection) -> rusqlite::Result<Self> {
        let progress = Arc::new(Mutex::new(None::<Progress>));
        let slot = Arc::clone(&progress);
        connection.progress_handler(PROGRESS_STEPS, Some(move || slot.lock().as_mut().is_some_and(|handler| handler())))?;
        let authorizer = Arc::new(Mutex::new(None::<Authorizer>));
        let slot = Arc::clone(&authorizer);
        connection.authorizer(Some(move |context: AuthContext<'_>| match slot.lock().as_mut() {
            Some(authorize) => authorize(context),
            None => Authorization::Allow,
        }))?;
        Ok(Self(Some(Slots { progress, authorizer })))
    }

    /// Calls `handler` every `PROGRESS_STEPS` instructions, which interrupts the statement when it returns true;
    /// `None`: no progress closure.
    pub(in crate::code_rule_dsl) fn progress<F>(&self, handler: Option<F>) -> rusqlite::Result<()>
    where
        F: FnMut() -> bool + Send + 'static,
    {
        *self.slots()?.progress.lock() = handler.map(|handler| Box::new(handler) as Progress);
        Ok(())
    }

    /// Authorizes each statement prepared from now on with `authorize`; `None`: every action is allowed.
    pub(in crate::code_rule_dsl) fn authorizer<F>(&self, authorize: Option<F>) -> rusqlite::Result<()>
    where
        F: for<'r> FnMut(AuthContext<'r>) -> Authorization + Send + 'static,
    {
        *self.slots()?.authorizer.lock() = authorize.map(|authorize| Box::new(authorize) as Authorizer);
        Ok(())
    }

    fn slots(&self) -> rusqlite::Result<&Slots> {
        self.0.as_ref().ok_or_else(|| {
            rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_MISUSE),
                Some("the connection's hooks were not registered while it was owned".to_owned()),
            )
        })
    }
}
