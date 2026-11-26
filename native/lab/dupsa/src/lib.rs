//! Duplicated code by suffix arrays: SA-IS over a text read in place (`sais`), MLDB's adapters over many documents
//! (`multi`), and two texts: format 16 tape nodes (`nodes`) and Jeff's normalised lines (`lines`). `report` turns a
//! suffix array into Jeff's blocks.

pub mod incr;
pub mod lines;
pub mod multi;
pub mod nodes;
pub mod report;
pub mod sais;
pub mod store;
