//! A relation's columns as written, beside the physical sentinel column `passes::normalize` gives logical
//! zero-column relations and zero-key lattices.

/// The columns relation `name` has as written: none for a relation of no columns (its presence column the
/// emitter's), its declared columns otherwise.
pub fn logical_arity(program: &(impl crate::code_rule_dsl::logic::program_image::fields::ProgramFields + ?Sized), name: &str) -> usize {
    if program.nullary().contains(name) { 0 } else { program.columns_of(name).map_or(0, |c| c.len()) }
}

