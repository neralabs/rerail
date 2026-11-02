//! A Python document's scope table and usage index under one reading: what the semantic
//! producers (`resolves`, `binds`) read, built once per document and reading, whatever clauses
//! read the rows.

// Read only by the gates until a lowering reads the labels (O41).
#![allow(dead_code)]

use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::matcher::table_reading;
use crate::code_rule_dsl::module_path::ModulePath;
use crate::code_rule_dsl::scope::options::PythonOptions;
use crate::code_rule_dsl::scope::usage::Usage;
use crate::code_rule_dsl::scope::{BuildError, Reading, ScopeTable};
use crate::code_rule_dsl::tape::TapeDoc;
use ast_grep_core::AstGrep as Root;

pub(crate) struct Scopes {
    pub(crate) table: ScopeTable,
    pub(crate) usage: Usage,
}

/// The reading a predicate with these Python options reads a file under, as W's `file_table`
/// keys it: the target version only for a stub (`stub`, the host's path says so).
pub(crate) fn reading(python: Option<&PythonOptions>, stub: bool) -> Reading {
    let reading = table_reading(None, python);
    Reading {
        stub,
        python_minor: if stub { reading.python_minor } else { 0 },
        ..reading
    }
}

impl Scopes {
    /// The table and usage index of `root`'s document, in at most `cap` steps each. `module` is
    /// the file's module path, which qualifies its definitions and relative imports.
    pub(crate) fn build(
        root: &Root<TapeDoc>,
        reading: Reading,
        module: Option<&ModulePath>,
        cap: usize,
        deadline: &dyn Fn() -> bool,
    ) -> Result<Self, BuildError> {
        let table = ScopeTable::build_with_names(
            root.root(),
            GrammarDialect::Python,
            cap,
            deadline,
            reading,
            module,
            None,
        )?;
        let usage = Usage::build(&table, &root.root(), cap, deadline)?;
        Ok(Self { table, usage })
    }
}
