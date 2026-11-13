//! Shared execution qualification, following decisions 0001 and 0005–0007.
//! Native semantic producers return facts; SQL composes them. No matcher
//! continuation, rule callback, or captured environment crosses this boundary.

mod async_reference;
mod call_batch;
pub(crate) mod contract;
mod import_screen;
pub(crate) mod python_structure;
mod relation;
mod sql_shape;
