//! SQL-driven waves (docs/final-shape/03-architecture.md §4, §7; decision 0010): one owner
//! connection on the content store, the temp schema's lifecycle, and the frontier loop that SQL
//! steers, wave by wave, until no relation changes.
mod owner;
mod schedule;
mod storage;
