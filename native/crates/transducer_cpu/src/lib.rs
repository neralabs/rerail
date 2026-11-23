//! The transducer's compiled CPU executor (`transducer::packed::native_cpu` and its parts: a batch's programs
//! compiled to native code by Cranelift, their scheduler, outputs, captures and regexes), with the capture runs and the
//! admission spike, over the transducer's base (rerail-transducer-base), which it re-exports. It registers itself
//! (`native_cpu::register`, into `packed::CPU`).
//!
//! Its sources are in its own `src/`, at their module paths in the engine's tree (the crate split, step 9).
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
pub use rerail_transducer_base::{content_store, git_access, git_workspace};
extern crate self as rerail_transducer;
#[path = "code_rule_dsl"]
pub mod code_rule_dsl {
    pub use rerail_transducer_base::code_rule_dsl::*;
    #[path = "transducer"]
    pub mod transducer {
        pub use rerail_transducer_base::code_rule_dsl::transducer::*;
        #[path = "packed"]
        pub mod packed {
            pub use rerail_transducer_base::code_rule_dsl::transducer::packed::*;
            pub mod native_cpu;
            pub mod native_cpu_capture;
            pub mod native_cpu_capture_pair;
            pub mod native_cpu_lua;
            pub mod native_cpu_output;
            pub mod native_cpu_output_sort;
            pub mod native_cpu_output_transform;
            pub mod native_cpu_reduction;
            pub mod native_cpu_regex;
            pub mod native_cpu_scheduler;
            pub mod native_cpu_tape;
            pub mod native_cpu_value;
            pub mod capture_run;
            // The query front end's audit through the compiled CPU (`scm`'s, which the base mounts).
            #[cfg(test)]
            #[path = "compile/scm_audit_tests.rs"]
            mod scm_audit_tests;
            #[cfg(all(feature = "test-hooks", not(test)))]
            pub mod admission_spike;
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
