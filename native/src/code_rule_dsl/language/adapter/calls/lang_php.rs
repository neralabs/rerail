//! Moved from `calls.rs` by the crate split (tools/cratify/moves.tsv).

use super::*;
use crate::code_rule_dsl::language::adapter::php::Php;
use crate::code_rule_dsl::calls::closure::lang_php::PhpLinks as _;

impl CallRules for Php {
    const SUPPORTED: bool = true;
    const NAMES: Option<Family> = Some(Family::Php);
    const CALL_KINDS: &'static [&'static str] = &php::CALL_KINDS;
    const FUNCTION_KINDS: &'static [&'static str] = &php::FUNCTION_KINDS;
    const SEED_LANGUAGE: Option<&'static str> = None;
    const SUBCLASSES: bool = false;

    fn feature(_: CallFeature) -> Result<(), Refusal> {
        Ok(())
    }

    fn summary<D: Doc>(
        root: &Node<'_, D>,
        path: &str,
        _: Option<&ModulePath>,
        table: Option<&ScopeTable>,
        has_errors: bool,
    ) -> Summary {
        crate::code_rule_dsl::calls::php::extract_program(root, path, table, has_errors)
    }

    /// A PHP file names nothing.
    fn unread(_: Option<&ModulePath>) -> Summary {
        Summary::default()
    }

    fn tables(
        table: impl FnOnce() -> Option<Arc<ScopeTable>>,
        _: impl FnOnce() -> Option<Arc<ModulePath>>,
        names: impl FnOnce() -> Option<crate::code_rule_dsl::scope::NameTable>,
    ) -> Option<FileTables> {
        let table = table()?;
        Some(FileTables { table, module: None, names: Some(names()?) })
    }

    fn callee_at<'t, D: Doc>(
        node: &Node<'t, D>,
        summary: &Summary,
        tables: impl FnOnce() -> Option<FileTables>,
    ) -> Option<Option<Callee>> {
        let Some(FileTables { table, names: Some(names), .. }) = tables() else {
            return None;
        };
        let names = crate::code_rule_dsl::scope::php_names::PhpNames::of(&names)?;
        let reader = php::Reader {
            names: &names,
            table: &table,
            root: node.root().root(),
        };
        Some(php::callee_at(&reader, summary, node))
    }

    /// PHP compares function, class and method names case-insensitively: a definition is linked
    /// by its lowercased name.
    fn definition_keys<I: Iterator<Item = Name>>(
        name: &Name,
        _: u8,
        _: impl FnOnce(&Name) -> I,
        mut key: impl FnMut(Name),
    ) {
        let text = name.text();
        key(if text.bytes().any(|byte| byte.is_ascii_uppercase()) {
            Name::new(name.family(), &text.to_ascii_lowercase())
        } else {
            name.clone()
        });
    }

    fn label(name: &Name) -> Cow<'_, str> {
        Cow::Owned(php::label(name.text()))
    }

    fn link(graph: &Graph, name: &Name, hops: u32, followed: &mut Vec<Name>) -> Link {
        graph.link_php(name, hops, followed)
    }
}
