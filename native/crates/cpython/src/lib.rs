//! CPython's own facts about Python source (`cpython`): `ast.parse`'s verdict, `literal_eval`, the `ast` tree,
//! `unparse` and `dump`, computed natively over Ruff's parser. It reads only `identifier::nfkc` (rerail-adapters'); the
//! Python pack (rerail-lang-python) re-exports it, with the parity oracle (`cpython::parity`, which reads the facts and
//! runs real CPython) over it.
//!
//! Its sources are in its own `src/`, at their module paths in the engine's tree (the crate split, step 9).
#![allow(dead_code, unused_imports)]
#![allow(clippy::len_without_is_empty, clippy::new_without_default, clippy::type_complexity, clippy::too_many_arguments)]
#[path = "code_rule_dsl"]
pub mod code_rule_dsl {
    pub use rerail_adapters::code_rule_dsl::identifier;
    pub mod cpython;
}
