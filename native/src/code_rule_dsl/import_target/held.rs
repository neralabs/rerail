//! ECMAScript's resolutions as W holds them (`resolution`): the held files (`fs::HeldFiles`), their view
//! (`fs::View`) and its slot on the evaluating thread (`ViewSlot`).

use super::ViewSlot;
use super::fs::{HeldFiles, View};
use crate::code_rule_dsl::model::Snapshot;
use crate::code_rule_dsl::resolution::{HeldForResolution, ResolutionView, Resolutions};
use std::any::Any;
use std::path::Path;
use std::sync::Arc;

/// A workspace's held files, shared with the views made of them until the next change.
#[derive(Default)]
struct Held(Arc<HeldFiles>);

impl HeldForResolution for Held {
    fn hold(&mut self, path: &str, source: &Arc<String>, root: Option<&Path>) -> bool {
        Arc::make_mut(&mut self.0).hold(path, source, root)
    }
    fn release(&mut self, path: &str, root: Option<&Path>) -> bool {
        Arc::make_mut(&mut self.0).release(path, root)
    }
    fn view(&self, root: Option<&Path>) -> Arc<dyn ResolutionView> {
        Arc::new(View::new(Arc::clone(&self.0), root))
    }
}

impl ResolutionView for View {
    fn install(self: Arc<Self>) -> Box<dyn Any> {
        Box::new(ViewSlot::install(Some(self)))
    }
}

/// What a snapshot's import targets resolve among: its files, with no disk below them.
fn snapshot_view(snapshot: &Snapshot) -> Arc<dyn ResolutionView> {
    let mut held = HeldFiles::default();
    for file in &snapshot.files {
        held.hold(&file.path, &file.source, None);
    }
    Arc::new(View::new(Arc::new(held), None))
}

/// ECMAScript's resolutions (`resolution::install`).
pub const RESOLUTIONS: Resolutions = Resolutions {
    held: || Box::new(Held::default()),
    snapshot: snapshot_view,
    installed: || super::current_view().is_some(),
};
