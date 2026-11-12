//! Set mode's executor freedoms (t1 spec-transducer §1 :95-100, §3 :368-377; list step 14).
//!
//! A program in set mode (`Program::in_set_mode`) is read as a set of (task, label) keys. An
//! executor may then stop a label after its first emit in a task, and drop a path whose reachable
//! labels have all fired. `Reach` is that analysis: each pc's labels reachable along the
//! program's control flow (fall-through, `FORK`'s two targets, `GOTO`), as bits of the program's
//! own labels; `Fired` is a task's emitted labels. Every backend keeps the first freedom (a task
//! ends once every label fired); the host's executors (the interpreter, the micro walk) also prune
//! by reach.

// The analysis is the compiler crate's (`rerail_compiler::set_mode`).
pub use rerail_compiler::set_mode::*;

// The tests read the transducer's names as they did when this file held the code.
#[cfg(any(test, feature = "test-hooks"))]
#[allow(unused_imports)]
use super::*;

// cratify: `tests` is rerail-engine's (it mounts it)
