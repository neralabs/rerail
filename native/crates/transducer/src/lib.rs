//! The transducer: W's matchers and waves lowered to label passes and programs (`transducer::packed::compile_matcher`,
//! with the device's side of them, `WaveDevice`) and the `demand(batch)` relation (`demand`), over the compiled CPU
//! executor (rerail-transducer-cpu) and the transducer's base (rerail-transducer-base), which it re-exports, so a
//! reader above reads every transducer path here, as before. It is the transducer's one crate that reads W
//! (rerail-rules).
//!
//! Its sources stay where they were (native/src): this crate mounts them in place, as the engine did.
// A mounted module re-exports its dependency's by glob, and its sources import the same items
// under the same names: the glob's name and the private one are one item. The mounts also widen what the crates
// above read (the split's facades, not a design).
#![allow(dead_code, unused_imports, hidden_glob_reexports, private_interfaces, private_bounds)]
// What the split widened for the crates above is no public API (`len`, `()` errors, `new`); the long
// signatures and tuple types are the sources' own.
#![allow(clippy::len_without_is_empty, clippy::result_unit_err, clippy::new_without_default)]
#![allow(clippy::type_complexity, clippy::too_many_arguments)]
// The browser build has no JIT: wasm/cranelift-jit stands in under the same name (wasm/README.md).
#[cfg(target_family = "wasm")]
extern crate cranelift_jit_wasm as cranelift_jit;
pub use rerail_transducer_cpu::{content_store, git_access, git_workspace};
#[path = "../../../src/code_rule_dsl"]
pub mod code_rule_dsl {
    pub use rerail_transducer_cpu::code_rule_dsl::*;
    pub use rerail_rules::code_rule_dsl::calls;
    pub use rerail_rules::code_rule_dsl::compile;
    pub use rerail_rules::code_rule_dsl::crypto;
    pub use rerail_rules::code_rule_dsl::eval;
    pub use rerail_rules::code_rule_dsl::file_input;
    pub use rerail_rules::code_rule_dsl::hooks;
    pub use rerail_rules::code_rule_dsl::identifier;
    pub use rerail_rules::code_rule_dsl::literal;
    pub use rerail_rules::code_rule_dsl::matcher;
    pub use rerail_rules::code_rule_dsl::model;
    pub use rerail_rules::code_rule_dsl::origin;
    #[cfg(feature="code-rule-profile")]
    pub use rerail_rules::code_rule_dsl::profile;
    pub use rerail_rules::code_rule_dsl::query;
    pub use rerail_rules::code_rule_dsl::resolution;
    pub use rerail_rules::code_rule_dsl::scope;
    pub use rerail_rules::code_rule_dsl::soundness;
    pub use rerail_rules::code_rule_dsl::suppress;
    pub use rerail_rules::code_rule_dsl::value;
    pub use rerail_rules::code_rule_dsl::violations;
    #[cfg(any(test, feature = "test-hooks"))]
    pub use rerail_rules::code_rule_dsl::walks;
    pub mod language {
        pub use rerail_transducer_cpu::code_rule_dsl::language::*;
        pub use rerail_rules::code_rule_dsl::language::adapter;
    }
    pub mod waves {
        #[cfg(any(test, feature = "test-hooks"))]
        pub use rerail_rules::code_rule_dsl::waves::relational;
    }
    pub mod demand;
    pub mod facts {
        pub use rerail_transducer_cpu::code_rule_dsl::facts::*;
        // The census of the rules' sets (a test: it reads the rules' model).
        #[cfg(test)]
        #[path = "set_census.rs"]
        pub mod set_census;
    }
    // The residency's admission (rerail-rules'), as the transducer's sources read it.
    pub mod residency;    #[path = "transducer"]
    pub mod transducer {
        pub use rerail_transducer_cpu::code_rule_dsl::transducer::*;
        pub use rerail_rules::code_rule_dsl::transducer::semantic_facts;
        #[path = "packed"]
        pub mod packed {
            pub use rerail_transducer_cpu::code_rule_dsl::transducer::packed::*;
            // W's composite matcher as label passes and programs (06-first-slice S1).
            #[path = "compile/matcher.rs"]
            #[cfg_attr(not(any(test, feature = "test-hooks")), allow(dead_code))]
            pub mod compile_matcher;
            pub use compile_matcher::{Backend as MatcherBackend, LoweredMatcher, LoweredWave, WaveRun};
            // `packed.rs`'s own imports, which its children read (`use super::*`).
            #[allow(unused_imports)]
            use super::super::tape::TapeDoc;
            #[allow(unused_imports)]
            use ast_grep_core::MatchStrictness;
            #[allow(unused_imports)]
            use ast_grep_core::matcher::{Pattern, PatternNode};
            #[allow(unused_imports)]
            use ast_grep_core::meta_var::{MetaVarEnv, MetaVariable};
            #[allow(unused_imports)]
            use ast_grep_core::{Doc, Node};
            #[allow(unused_imports)]
            use regex::Regex;
            #[allow(unused_imports)]
            use sha2::{Digest as _, Sha256};
            #[allow(unused_imports)]
            use std::borrow::Cow;
            #[allow(unused_imports)]
            use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
            #[allow(unused_imports)]
            use std::fmt::Write as _;
            #[allow(unused_imports)]
            use std::ops::Range;
            #[allow(unused_imports)]
            use std::sync::Arc;
            #[allow(unused_imports)]
            use rerail_compiler::native_cpu_codegen;
            #[allow(unused_imports)]
            use rerail_compiler::program::*;
        }
    }
}
// cratify: mounts end

// The crate's own tests make their folders the user's alone, as the engine's test binaries do (decision 0574): a
// host's umask of `002` made a fixture's folder `775`, which the mapped files' rules refuse.
#[cfg(test)]
#[path = "../../../src/test_private_folders.rs"]
mod test_private_folders;

/// This crate's own test binary, which boot does not load: the compiled CPU executor its tests run is registered as the
/// binary loads, as boot registers it (`install_devices`).
#[cfg(test)]
mod test_load {
    extern "C" fn load() {
        crate::code_rule_dsl::transducer::packed::native_cpu::register();
    }
    #[used]
    #[cfg_attr(target_vendor = "apple", unsafe(link_section = "__DATA,__mod_init_func"))]
    #[cfg_attr(not(target_vendor = "apple"), unsafe(link_section = ".init_array"))]
    static LOAD: extern "C" fn() = load;
}
