//! Front ends: SQL stages (`sql`), policy documents (`rule`) and their matcher queries lowered to
//! the algebra over core relations (`pattern`).
pub mod pattern;
pub mod rule;
pub mod sql;
