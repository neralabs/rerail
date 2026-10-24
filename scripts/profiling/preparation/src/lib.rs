//! Small replay host. All SQLite adapters and codecs come from production sources.
#[path = "../../../../native/src/code_rule_dsl/profile/clock.rs"]
mod clock;
#[cfg(test)]
use clock::thread_cpu_ns;
#[path = "../../../../native/src/code_rule_dsl/matcher/anchors/sql/functions.rs"]
mod functions;
#[path = "../../../../native/src/code_rule_dsl/matcher/anchors/sql/lower/lifetime.rs"]
mod lifetime;
#[path = "../../../../native/src/code_rule_dsl/profile/memory.rs"]
mod memory;
#[path = "../../../../native/src/code_rule_dsl/matcher/anchors/sql/nodes.rs"]
mod nodes;
#[cfg(test)]
#[path = "../../../../native/src/code_rule_dsl/matcher/anchors/sql/store/publication.rs"]
mod publication;
#[cfg(test)]
#[path = "../../../../native/src/code_rule_dsl/profile/trace.rs"]
#[allow(dead_code)]
mod trace;
// Replay reads stored segments; production also uses this module's encoder.
#[allow(dead_code)]
#[path = "../../../../native/src/code_rule_dsl/matcher/anchors/sql/postings.rs"]
mod postings;
#[allow(dead_code)]
#[path = "../../../../native/src/code_rule_dsl/matcher/anchors/sql/reader.rs"]
mod reader;
#[cfg(test)]
#[path = "../../../../native/src/code_rule_dsl/matcher/anchors/sql/lower/render.rs"]
mod render;
#[path = "../../../../native/src/code_rule_dsl/matcher/anchors/sql/replay.rs"]
mod replay;
#[path = "../../../../native/src/code_rule_dsl/matcher/anchors/sql/session.rs"]
mod session;
#[path = "../../../../native/src/code_rule_dsl/tape/wire.rs"]
mod wire;
#[path = "../../../../native/src/code_rule_dsl/matcher/anchors/sql/lower/working_set.rs"]
mod working_set;

pub mod composition;
pub mod fts;
pub mod residency;

mod code_rule_dsl {
    pub(crate) mod profile {
        pub(crate) use crate::clock::thread_cpu_ns;
        pub(crate) use crate::memory;
    }
    pub(crate) mod tape {
        pub(crate) use crate::wire::index_span;
    }
    pub(crate) mod budget {
        // Captured SQL has no active rule context. The controller supplies the hard deadline.
        pub(crate) fn step_budget_exhausted() -> bool {
            false
        }
    }
}

mod store {
    pub(super) struct Store {
        pub readers: super::reader::Readers,
        pub db: rusqlite::Connection,
    }
    impl Store {
        pub fn open(path: &std::path::Path) -> rusqlite::Result<Self> {
            let db = rusqlite::Connection::open(path)?;
            super::session::configure(&db)?;
            db.execute_batch(include_str!(
                "../../../../native/src/code_rule_dsl/matcher/anchors/sql/vocabulary.sql"
            ))?;
            let readers = super::functions::install(&db)?;
            db.progress_handler(16_384, Some(|| false))?;
            Ok(Self { readers, db })
        }
    }
}

pub fn run(directory: &std::path::Path, trial: usize) {
    replay::experiment(directory, trial);
}

#[cfg(test)]
mod tests;
