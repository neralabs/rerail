//! The preprocessor's evaluator over tapes (decision 0436) on Metal (`rerail_metal::pp_eval`),
//! held to the lean reading: its passes' arrays against the tapes' and the serial pass's.

#[cfg(any(target_os = "macos", target_os = "linux", windows))]
pub mod rounds;

#[cfg(all(test, target_os = "macos"))]
#[path = "eval/tests.rs"]
mod tests;
