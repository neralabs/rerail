//! The C and C++ preprocessor (decision 0327), in two modes.
//!
//! - **Recording** (`directives`): every directive of a file as rows, whatever the configuration.
//!   The rows are the file's alone: its defines, includes, conditionals (their chains and nesting),
//!   pragmas and line directives, and its include guard.
//! - **Applying**: a translation unit read in one forward pass under a configuration.
//!
//! Both read a file with the shared lexer's preprocessing mode (`rerail_c_lean::pp`).

pub mod directives;
pub mod expr;
pub mod files;
pub mod macros;
pub mod sink;
pub mod stream;
pub mod tokens;

// The oracle's profile reads the crate by its name, as the example that includes it does.
#[cfg(test)]
extern crate self as rerail_c_pp;
#[cfg(test)]
mod oracle_profile;

pub use rerail_c_lean::pp::Options;
pub use stream::{Config, Pp};
