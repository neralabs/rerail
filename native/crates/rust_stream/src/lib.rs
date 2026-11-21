//! The parser's tiers as Rust source, which rustc and LLVM compile ahead of time: tier 2's stream program
//! (`rust_stream`) and tier 1's block program (`rust_block`). Both are written from the compiler's front end
//! (`rerail-compiler`): the program's facts (`stream_facts`), which the JIT reads too, decide where a format 16 run
//! passes each node at its word. Tier 1's lowering (`rerail-cpu-block`) gives a local check's mask as the JIT's
//! window function, which is translated. The JIT (`rerail-cpu-stream`) is apart: an edit to either does not rebuild
//! the other. The sources stay in the compiler's folder (`crates/compiler/src`); this crate mounts them.

// The compiler's passes hand each other plain tuples of tables; naming each would add nothing.
#![allow(clippy::type_complexity)]

// The crates below under their modules' names, so the mounted sources' `crate::records`, `crate::native_ir`, ...
// read them.
#[allow(unused_imports)]
use rerail_compiler::*;
#[allow(unused_imports)]
use rerail_cpu_block::*;

#[path = "../../compiler/src/rust_stream.rs"]
pub mod rust_stream;
#[path = "../../compiler/src/rust_block.rs"]
pub mod rust_block;
