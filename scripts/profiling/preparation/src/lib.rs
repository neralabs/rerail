//! Small replay host. All SQLite adapters and codecs come from production sources.
// The engine's sources keep their own layout: the lab's `cargo fmt` checks only the lab's.
#[rustfmt::skip]
#[path = "../../../../native/src/code_rule_dsl/profile/clock.rs"]
mod clock;
#[cfg(test)]
use clock::thread_cpu_ns;
#[rustfmt::skip]
#[path = "../../../../native/src/code_rule_dsl/matcher/p_anchors/sql/functions.rs"]
mod functions;
#[rustfmt::skip]
#[path = "../../../../native/src/code_rule_dsl/matcher/p_anchors/sql/lower/lifetime.rs"]
mod lifetime;
#[rustfmt::skip]
#[path = "../../../../native/src/code_rule_dsl/profile/memory.rs"]
mod memory;
#[rustfmt::skip]
#[path = "../../../../native/src/code_rule_dsl/matcher/p_anchors/sql/nodes.rs"]
mod nodes;
#[cfg(test)]
#[rustfmt::skip]
#[path = "../../../../native/src/code_rule_dsl/matcher/p_anchors/sql/store/publication.rs"]
mod publication;
#[allow(dead_code)]
#[rustfmt::skip]
#[path = "../../../../native/src/code_rule_dsl/residency/admission/relational/schema.rs"]
mod schema;
#[cfg(test)]
#[rustfmt::skip]
#[path = "../../../../native/src/code_rule_dsl/profile/trace.rs"]
#[allow(dead_code)]
mod trace;
// Replay reads stored segments; production also uses this module's encoder.
#[allow(dead_code)]
#[rustfmt::skip]
#[path = "../../../../native/src/code_rule_dsl/matcher/p_anchors/sql/postings.rs"]
mod postings;
#[allow(dead_code)]
#[rustfmt::skip]
#[path = "../../../../native/src/code_rule_dsl/matcher/p_anchors/sql/reader.rs"]
mod reader;
// The replay CLI uses the same owner-projection rewrite as production.
#[allow(dead_code)]
#[rustfmt::skip]
#[path = "../../../../native/src/code_rule_dsl/matcher/p_anchors/sql/lower/render.rs"]
mod render;
#[rustfmt::skip]
#[path = "../../../../native/src/code_rule_dsl/matcher/p_anchors/sql/replay.rs"]
mod replay;
#[rustfmt::skip]
#[path = "../../../../native/src/code_rule_dsl/matcher/p_anchors/sql/session.rs"]
mod session;
#[rustfmt::skip]
#[path = "../../../../native/src/code_rule_dsl/matcher/p_anchors/sql/tree.rs"]
mod tree;
#[rustfmt::skip]
#[path = "../../../../native/src/code_rule_dsl/tape/wire.rs"]
mod wire;
#[rustfmt::skip]
#[path = "../../../../native/src/code_rule_dsl/matcher/p_anchors/sql/lower/working_set.rs"]
mod working_set;

#[cfg(test)]
#[rustfmt::skip]
#[path = "../../../../native/src/code_rule_dsl/producers/mod.rs"]
mod producers;

#[cfg(test)]
mod shared_storage;

#[allow(dead_code)]
#[rustfmt::skip]
#[path = "../../../../native/src/code_rule_dsl/matcher/p_anchors/sql/lower/ownership.rs"]
mod ownership;

#[allow(dead_code)]
#[rustfmt::skip]
#[path = "../../../../native/src/code_rule_dsl/matcher/p_anchors/sql/lower/waves.rs"]
mod waves;

#[cfg(test)]
#[allow(dead_code)]
#[rustfmt::skip]
#[path = "../../../../native/src/code_rule_dsl/matcher/p_anchors/sql/graph.rs"]
mod graph;

#[cfg(test)]
#[rustfmt::skip]
#[path = "../../../../native/src/code_rule_dsl/matcher/p_anchors/sql/lower/children.rs"]
mod children;

pub mod candidate_owner;
pub mod composition;
pub mod fts;
pub mod owner_projection;
pub mod post_c;
pub mod residency;

mod code_rule_dsl;

mod store {
    pub(super) struct Store {
        pub readers: super::reader::Readers,
        pub db: rusqlite::Connection,
    }
    impl Store {
        pub fn open(path: &std::path::Path) -> rusqlite::Result<Self> {
            let mut db = rusqlite::Connection::open(path)?;
            super::session::configure(&db)?;
            db.execute_batch(include_str!(
                "../../../../native/src/code_rule_dsl/matcher/p_anchors/sql/vocabulary.sql"
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
