//! The reference catalog lowering that the demand tests drive: N's catalog lowering and its driver
//! (`demand::driver::catalog`), which rerail-tests-demand's contract and policy suites run (docs/final-shape/proposals/
//! crate-split/incremental-plan.md, step 7i). Test-only code: production statements come from the logic planner.
//!
//! A test crate's support beside boot: it mounts the lowering in place, under the facades the engine hosted it
//! through, and reads what the lowering reads below boot (the rule API, the transducer, the typed SQL and the packs'
//! suppressions). What its suites read is widened to `pub` (the split's facades, not a design); the items only they
//! call are `test-hooks`. Its own tests are the lowering's inline ones, and its test binary is booted as it loads, as
//! the test crates' are (boot is a dev-dependency).
#![allow(dead_code, unused_imports, hidden_glob_reexports, ambiguous_glob_reexports, private_interfaces, private_bounds)]
#![allow(clippy::len_without_is_empty, clippy::result_unit_err, clippy::new_without_default)]
#![allow(clippy::type_complexity, clippy::too_many_arguments)]
pub use rerail_prefilter::{content_store, git_access, git_workspace};

// The engine's test-binary rules: each test's time limit (`RERAIL_TEST_TIMEOUT`), and stores that flush nothing
// (`RERAIL_FLUSH=none`, decision 0481), both set before `main`.
#[cfg(test)]
#[path = "../../../src/test_watchdog.rs"]
mod test_watchdog;
#[cfg(test)]
#[path = "../../../src/test_flush.rs"]
mod test_flush;

/// The program, booted as the engine boots it (`boot::install`), with this binary's own build identities.
#[cfg(test)]
pub(crate) fn install_store_functions() {
    rerail_boot::code_rule_dsl::boot::install_for_tests();
}

// This crate's own test binary's allocator, the engine's (`rerail_prof::heap::Sampled`, as `rerail-engine`'s
// `lib.rs` declares it): without it the binary ran on the system allocator (`test_flush`'s jemalloc test). Only its
// test build: rerail-tests-demand, which links this crate, declares its own.
#[cfg(all(test, not(any(windows, target_family = "wasm", feature = "system-allocator"))))]
#[global_allocator]
static GLOBAL: rerail_prof::heap::Sampled = rerail_prof::heap::Sampled;

#[cfg(test)]
mod test_load {
    extern "C" fn load() {
        super::install_store_functions();
    }
    #[used]
    #[cfg_attr(target_vendor = "apple", unsafe(link_section = "__DATA,__mod_init_func"))]
    #[cfg_attr(windows, unsafe(link_section = ".CRT$XCU"))]
    #[cfg_attr(not(any(target_vendor = "apple", windows)), unsafe(link_section = ".init_array"))]
    static LOAD: extern "C" fn() = load;
}

#[path = "code_rule_dsl"]
pub mod code_rule_dsl {
    // The rule API, the matcher and the stats (rerail-workspace, with the transducer's `space` and `transducer`, as
    // rerail-prefilter mounts them), the typed SQL (rerail-logic-core) and the language packs' suppressions: what the
    // lowering reads. It is below boot, which only its tests boot.
    pub use rerail_prefilter::code_rule_dsl::*;
    pub mod logic {
        pub use rerail_prefilter::code_rule_dsl::logic::*;
        pub use rerail_logic_core::code_rule_dsl::logic::sql;
    }
    pub mod suppress {
        pub use rerail_prefilter::code_rule_dsl::suppress::*;
        pub use rerail_lang_ecma::code_rule_dsl::suppress::eslint;
        pub use rerail_lang_php::code_rule_dsl::suppress::{phpcs, phpstan, psalm};
        pub use rerail_lang_python::code_rule_dsl::suppress::{noqa, nosec};
    }
    #[path = "demand"]
    pub mod demand {
        pub use rerail_transducer::code_rule_dsl::demand::*;
        #[path = "driver"]
        pub mod driver {
            pub use rerail_transducer::code_rule_dsl::demand::driver::*;
            pub mod catalog;
            // `driver.rs`'s own imports, which its children read.
            #[allow(unused_imports)]
            use crate::code_rule_dsl::demand::vtab::{MatchRegistry, PackedDemand, register_module};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::transducer::packed::ExecutionBackend;
            #[allow(unused_imports)]
            use crate::content_store::{ContentStore, StoreResult};
            #[allow(unused_imports)]
            use rusqlite::{Connection, OptionalExtension as _, Row};
            #[allow(unused_imports)]
            use serde_json::{Value, json};
            #[allow(unused_imports)]
            use std::collections::BTreeMap;
            #[allow(unused_imports)]
            use std::sync::{Arc, LazyLock};
        }
    }
}
