//! The lean parsers' adapters (`code_rule_dsl::tape::<language>_lean`): each lean parser's keys read as the engine's
//! tape codes, and its tape of a file (`TreeParser::parse`), a fragment (`parse_from`) and an embedded program
//! (`program`), the tape tree-sitter's tree encodes to (decision 0198). boot registers them in the parser router as
//! the engine loads (`tape::install_parsers`); lang reads a dialect through the router, so it stands on no lean
//! parser, and a lean parser's edit rebuilds this crate and the ones above it, not lang's.
//!
//! Its sources are in its own `src/`, at their module paths in the engine's tree (the crate split, 10e).
pub use rerail_lang::content_store;
extern crate self as rerail_lean_tapes;
#[path = "code_rule_dsl"]
pub mod code_rule_dsl {
    pub use rerail_lang::code_rule_dsl::*;
    #[path = "tape"]
    pub mod tape {
        pub use rerail_lang::code_rule_dsl::tape::*;
        pub mod c_lean;
        pub mod csharp_lean;
        pub mod css_lean;
        pub mod ecma_lean;
        pub mod go_lean;
        pub mod html_lean;
        pub mod java_lean;
        pub mod kotlin_lean;
        pub mod markdown_lean;
        pub mod markdown_inline_lean;
        pub mod php_lean;
        pub mod python_lean;
        pub mod rust_lean;
        pub mod scala_lean;
        pub mod sql_lean;
        pub mod swift_lean;
        pub mod vue_lean;
    }
}
