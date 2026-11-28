//! The layouts every reader of the engine's data reads where it lies (decision 0353), and their reads written
//! directly in Rust (decisions 0534, 0536), with no dependency of the engine's (memchr alone), so every crate that reads them may sit
//! above them: the facts and the tape's writer, rerail-clif (whose CLIF of the same reads the JIT and the kernels
//! run, and which re-exports these where they were), and the CPU's join program (rerail-ukl-rust), below the device
//! crate that runs it.
//!
//! - [`view`]: a tape's view (`TapeView`), its formats' numbers, the rolling cache's layout and host lookup, and a
//!   reader's view;
//! - [`tape`]: a tape's reads (format 16; 16z through its batch's cache), and where its presence's parts lie;
//! - [`lines`]: a document's line index's reads;
//! - [`aux`]: a tape's auxiliary index's reads (`AuxIndex`);
//! - [`ids`]: a projection's numbering (`Numbering`) and its reads;
//! - [`packed`]: a packed map of u32s, a byte an entry (the numbering's most asked answers);
//! - [`views`]: a batch's views' entries (`ViewDoc`), its directory (`ViewDir`, `ViewRow`, `ViewHome`) and the
//!   directory's searches, and the codes' directory's runs;
//! - [`view_reads`]: a document's views' rows (`Doc`), read through its entry, tape, line index and source where
//!   they lie: the host executor's reads and the CPU's join program's.
pub mod aux;
pub mod ids;
pub mod packed;
pub mod lines;
pub mod tape;
pub mod view;
pub mod view_reads;
pub mod views;
