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
#[allow(dead_code)]
#[path = "../../../../native/src/code_rule_dsl/relational/schema.rs"]
mod schema;
// The live scheduler also uses CLEAR; replay imports the production relation and schema.
#[allow(dead_code)]
#[path = "../../../../native/src/code_rule_dsl/matcher/anchors/sql/functions/source_batch.rs"]
mod source_batch;
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
// The replay CLI uses the same owner-projection rewrite as production.
#[allow(dead_code)]
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

#[cfg(test)]
#[path = "../../../../native/src/code_rule_dsl/producers/mod.rs"]
mod producers;

#[allow(dead_code)]
#[path = "../../../../native/src/code_rule_dsl/relational/authorizer.rs"]
mod authorizer;
#[cfg(test)]
mod shared_storage;

#[allow(dead_code)]
#[path = "../../../../native/src/code_rule_dsl/matcher/anchors/sql/lower/ownership.rs"]
mod ownership;

#[allow(dead_code)]
#[path = "../../../../native/src/code_rule_dsl/matcher/anchors/sql/lower/waves.rs"]
mod waves;

#[cfg(test)]
#[allow(dead_code)]
#[path = "../../../../native/src/code_rule_dsl/matcher/anchors/sql/graph.rs"]
mod graph;

#[cfg(test)]
#[path = "../../../../native/src/code_rule_dsl/matcher/anchors/sql/lower/children.rs"]
mod children;

pub mod candidate_owner;
pub mod cold_cpu;
pub mod composition;
pub mod fts;
pub mod owner_projection;
pub mod post_c;
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
        pub(crate) fn matcher_budget_exhausted() -> bool {
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

#[cfg(test)]
#[allow(dead_code)]
#[path = "../../../../native/src/code_rule_dsl/relational/storage.rs"]
mod relational_storage;
