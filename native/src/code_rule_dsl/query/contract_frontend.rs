//! Test frontend for the compact relational contract language.
//!
//! The contract language is not policy syntax. Production policy documents reach the same
//! relational IR through the compiled-policy adapter.

use super::relational_ir::RelationalIr;

pub(super) fn lower(source: &str) -> Result<RelationalIr, String> {
    let source = source.trim();
    if source.is_empty() {
        return Err("empty relational contract".to_owned());
    }
    let operation = source.split_once('(').map_or(source, |(head, _)| head);
    Err(format!(
        "relational IR lowering is not implemented for `{operation}`"
    ))
}
