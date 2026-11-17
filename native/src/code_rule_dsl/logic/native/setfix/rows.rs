//! Setfix's rows: the rows' storage is the facts layer's (`facts::rows`, decision 0325), shared with the vtab's
//! `Relation`; here, how setfix's readers read them (`RowData`).

pub use crate::code_rule_dsl::facts::rows::*;

use super::runs::RowData;

impl RowData for Rows {
    #[inline(always)]
    fn row(&self, id: u32, k: usize) -> Row {
        Rows::row(self, id, k)
    }
    /// Row `id`'s value in column `c`: one read where it lies.
    #[inline(always)]
    fn value(&self, id: u32, _k: usize, c: usize) -> i64 {
        Rows::get(self, id, c)
    }
}

impl RowData for Vec<i64> {
    #[inline(always)]
    fn row(&self, id: u32, k: usize) -> Row {
        Row::of(&self[id as usize * k..][..k])
    }
    #[inline(always)]
    fn value(&self, id: u32, k: usize, c: usize) -> i64 {
        self[id as usize * k + c]
    }
}

