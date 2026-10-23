//! The facts engine's compiler (Phase 2): the rule IR (`ir`), its front ends (`front`: SQL
//! stages and policy documents with pattern leaves), classification (`classify`), the entry
//! point (`compile`), the 60-rule census (`census`), and the leaf modules moved from Spike A
//! (SQLite values and affinity, the SQL-subset parser, `$option` resolution and scopes).
pub mod census;
pub mod classify;
pub mod compile;
pub mod explain;
pub mod front;
pub mod ir;
pub mod mutant;
pub mod phys;
pub mod policy;
pub mod reference;
pub mod sql;
#[cfg(test)]
pub mod testkit;
