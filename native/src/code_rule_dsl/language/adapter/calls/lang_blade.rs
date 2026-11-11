//! Moved from `calls.rs` by the crate split (tools/cratify/moves.tsv).

use super::*;

/// A template's calls are those of the PHP it embeds (`blade::view`), read from its embedded
/// program (`calls::calls_document`); its top level is no class's, so `$this`, `self` and `static`
/// there are unknown. Its seeds are the workspace's PHP definitions, which its calls reach: its
/// seed query is PHP's.
impl CallRules for Blade {
    const SUPPORTED: bool = <Php as CallRules>::SUPPORTED;
    const NAMES: Option<Family> = <Php as CallRules>::NAMES;
    const CALL_KINDS: &'static [&'static str] = <Php as CallRules>::CALL_KINDS;
    const FUNCTION_KINDS: &'static [&'static str] = <Php as CallRules>::FUNCTION_KINDS;
    const SEED_LANGUAGE: Option<&'static str> = Some("php");
    const SUBCLASSES: bool = <Php as CallRules>::SUBCLASSES;

    fn feature(feature: CallFeature) -> Result<(), Refusal> {
        <Php as CallRules>::feature(feature)
    }

    fn summary<D: Doc>(
        root: &Node<'_, D>,
        path: &str,
        module: Option<&ModulePath>,
        table: Option<&ScopeTable>,
        has_errors: bool,
    ) -> Summary {
        <Php as CallRules>::summary(root, path, module, table, has_errors)
    }

    fn unread(module: Option<&ModulePath>) -> Summary {
        <Php as CallRules>::unread(module)
    }

    fn tables(
        table: impl FnOnce() -> Option<Arc<ScopeTable>>,
        module: impl FnOnce() -> Option<Arc<ModulePath>>,
        names: impl FnOnce() -> Option<Arc<PhpNames>>,
    ) -> Option<FileTables> {
        <Php as CallRules>::tables(table, module, names)
    }

    fn callee_at<'t, D: Doc>(
        node: &Node<'t, D>,
        summary: &Summary,
        tables: impl FnOnce() -> Option<FileTables>,
    ) -> Option<Option<Callee>> {
        <Php as CallRules>::callee_at(node, summary, tables)
    }

    fn definition_keys<I: Iterator<Item = Name>>(
        name: &Name,
        flags: u8,
        spellings: impl FnOnce(&Name) -> I,
        key: impl FnMut(Name),
    ) {
        <Php as CallRules>::definition_keys(name, flags, spellings, key)
    }

    fn label(name: &Name) -> Cow<'_, str> {
        <Php as CallRules>::label(name)
    }

    fn link(graph: &Graph, name: &Name, hops: u32, followed: &mut Vec<Name>) -> Link {
        <Php as CallRules>::link(graph, name, hops, followed)
    }
}
