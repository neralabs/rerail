//! The layouts every reader of the engine's data reads where it lies (decision 0353), and their reads written
//! directly in Rust (decisions 0534, 0536), with no dependency of their own, so every crate that reads them may sit
//! above them: the facts and the tape's writer, rerail-clif (whose CLIF of the same reads the JIT and the kernels
//! run, and which re-exports these where they were), and the CPU's join program (rerail-ukl-rust), below the device
//! crate that runs it.
//!
//! - [`view`]: a tape's view (`TapeView`), its formats' numbers, the rolling cache's layout and host lookup, and a
//!   reader's view;
//! - [`tape`]: a tape's reads, of a slice or where it lies (formats 16 and 15; 16z through its batch's cache);
//! - [`ids`]: a projection's numbering (`Numbering`) and its reads;
//! - [`views`]: a batch's views' entries (`ViewDoc`), its directory (`ViewDir`, `ViewRow`, `ViewHome`) and the
//!   directory's searches.
pub mod ids;
pub mod tape;
pub mod view;
pub mod views;
