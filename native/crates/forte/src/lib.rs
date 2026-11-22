//! Blade's Forte parser (`blade::forte`): its lexer, parser, directives, scan and trees. It reads the language's
//! metavariables and the literal trees' ids (rerail-adapters'), nothing of PHP; the PHP pack (rerail-lang-php)
//! re-exports it under `blade`, where its view and tape read it. Its tests are rerail-tests-value's (they compile and
//! evaluate rules over Blade documents).
//!
//! Its sources stay where they were (native/src): this crate mounts them in place, as the engine did.
#![allow(dead_code, unused_imports)]
#![allow(clippy::len_without_is_empty, clippy::new_without_default, clippy::type_complexity, clippy::too_many_arguments)]
#[path = "../../../src/code_rule_dsl"]
pub mod code_rule_dsl {
    pub use rerail_adapters::code_rule_dsl::literal;
    pub use rerail_lang::code_rule_dsl::language;
    // Blade's rows, tree-sitter's `Lf` lines (decision 0612).
    pub use rerail_lang::code_rule_dsl::lines;
    pub mod blade {
        pub mod forte;
    }
}
