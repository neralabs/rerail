//! Moved from `calls.rs` by the crate split (tools/cratify/moves.tsv).

use super::*;
use crate::code_rule_dsl::language::adapter::python::Python;
use crate::code_rule_dsl::calls::closure::lang_python::PythonLinks as _;

impl CallRules for Python {
    const SUPPORTED: bool = true;
    const NAMES: Option<Family> = Some(Family::Python);
    const CALL_KINDS: &'static [&'static str] = &["call"];
    const FUNCTION_KINDS: &'static [&'static str] = &["function_definition"];
    const SEED_LANGUAGE: Option<&'static str> = None;
    const SUBCLASSES: bool = true;

    fn feature(_: CallFeature) -> Result<(), Refusal> {
        Ok(())
    }

    fn summary<D: Doc>(
        root: &Node<'_, D>,
        _: &str,
        module: Option<&ModulePath>,
        table: Option<&ScopeTable>,
        has_errors: bool,
    ) -> Summary {
        crate::code_rule_dsl::calls::summary::python::extract(root, table, module, has_errors)
    }

    /// Python names the module, whose members may hide in it.
    fn unread(module: Option<&ModulePath>) -> Summary {
        crate::code_rule_dsl::calls::summary::python::extract(
            &GrammarDialect::Python.ast_grep("").root(),
            None,
            module,
            true,
        )
    }

    fn tables(
        table: impl FnOnce() -> Option<Arc<ScopeTable>>,
        module: impl FnOnce() -> Option<Arc<ModulePath>>,
        _: impl FnOnce() -> Option<crate::code_rule_dsl::scope::NameTable>,
    ) -> Option<FileTables> {
        Some(FileTables { table: table()?, module: module(), names: None })
    }

    fn callee_at<'t, D: Doc>(
        node: &Node<'t, D>,
        summary: &Summary,
        tables: impl FnOnce() -> Option<FileTables>,
    ) -> Option<Option<Callee>> {
        let Some(FileTables { table, module, .. }) = tables() else {
            return None;
        };
        Some(crate::code_rule_dsl::calls::summary::python::callee_at(
            &table,
            module.as_deref(),
            summary,
            node,
        ))
    }

    /// A definition a later binding replaces is no longer what its name denotes.
    fn definition_keys<I: Iterator<Item = Name>>(
        name: &Name,
        flags: u8,
        spellings: impl FnOnce(&Name) -> I,
        key: impl FnMut(Name),
    ) {
        if flags & SHADOWED == 0 {
            spellings(name).for_each(key);
        }
    }

    fn label(name: &Name) -> Cow<'_, str> {
        Cow::Borrowed(name.text())
    }

    fn link(graph: &Graph, name: &Name, hops: u32, followed: &mut Vec<Name>) -> Link {
        graph.link_python(name, hops, followed)
    }
}
