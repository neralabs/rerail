//! The facts' in-memory overlay over the content store (`facts::overlay`) and its frozen layers (`facts::runs`), crate
//! split step 10c. It was facts-base's;
//! only boot's slices write through it, so it sits above the facts, and an edit to it rebuilds boot and the crates
//! above it alone. It holds the store through `FactStore`; its tests open the store itself.
#[cfg(not(test))]
pub use rerail_observe::content_store;
#[cfg(test)]
pub use rerail_store::content_store;

#[path = "code_rule_dsl"]
pub mod code_rule_dsl {
    #[path = "facts"]
    pub mod facts {
        pub use rerail_facts_base::code_rule_dsl::facts::{Graph, observe, propagate};
        pub mod overlay;
        // The overlay's frozen layers: columnar runs.
        pub mod runs;
    }
}
