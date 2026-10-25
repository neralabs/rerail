//! Historical control IR is retained for diagnosis. No SQL interpreter can be emitted.
use super::super::{
    finite_lower,
    lower::{mixed_recursive_lower, parse_recursive_lower},
};
use super::{Demand, Output, Sql};

impl Sql {
    pub(super) fn finite_program(
        &mut self,
        _: &finite_lower::Program,
        _: &Demand,
    ) -> Result<Output, String> {
        Err("SQL state machine removed: use a batched matcher fragment contract".into())
    }
    pub(super) fn mixed_recursive(
        &mut self,
        _: &mixed_recursive_lower::Program,
        _: usize,
        _: &Demand,
    ) -> Result<Output, String> {
        Err("SQL recursive interpreter removed: use a batched matcher fragment contract".into())
    }
    pub(super) fn parsed_recursive(
        &mut self,
        _: &parse_recursive_lower::Program,
        _: usize,
        _: &Demand,
    ) -> Result<Output, String> {
        Err("SQL parsed interpreter removed: use a batched matcher fragment contract".into())
    }
}
