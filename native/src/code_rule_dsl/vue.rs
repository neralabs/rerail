//! Vue single-file components (`.vue`): their blocks (`sfc`), template directives (`template`)
//! and the program a component embeds (`view`), a guest document of the component's file
//! (`embed::ScriptBlock`). The grammar is the registry's (`GrammarDialect::Vue`, `vendored:vue`).
//!
//! A Vue file is the component's tree-sitter-vue tree and the TypeScript, TSX or JavaScript
//! program its scripts and template expressions make (`view`), each a document with its own
//! grammar and spans; the program's text maps back to the `.vue` bytes it came from.
#[cfg(test)]
pub mod oracle_tests;
pub mod sfc;
pub mod template;
pub mod view;

#[cfg(test)]
pub mod tests;
