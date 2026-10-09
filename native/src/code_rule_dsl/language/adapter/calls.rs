//! The workspace call graph (`calls`): `Semantic::Calls`.

use super::{Blade, Php, Python, Refusal, Refused};
use crate::code_rule_dsl::calls::closure::{Graph, Link};
use crate::code_rule_dsl::calls::names::Name;
use crate::code_rule_dsl::calls::summary::{Callee, SHADOWED, Summary};
use crate::code_rule_dsl::calls::{FileTables, php};
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::module_path::ModulePath;
use crate::code_rule_dsl::scope::ScopeTable;
use crate::code_rule_dsl::scope::php_names::PhpNames;
use ast_grep_core::{Doc, Node};
use ast_grep_language::LanguageExt;
use std::borrow::Cow;
use std::sync::Arc;

/// A DSL feature of `calls` a family may refuse.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CallFeature {
    /// The `calls` term itself.
    Calls,
}

/// A family's calls: what a call is, what it reaches, how a file's calls are summarised, and how
/// the workspace graph names and links the family's definitions.
pub(crate) trait CallRules {
    /// Whether the family's calls join the call graph: its languages' registry capability,
    /// which the adapter kit checks.
    const SUPPORTED: bool;
    /// The node kinds of a call: the only nodes a `calls` term holds at.
    const CALL_KINDS: &'static [&'static str];
    /// The node kinds of what a call may reach: the definitions a seed query runs on.
    const FUNCTION_KINDS: &'static [&'static str];
    /// The language a seed query of the family's rule runs in, where not the rule's own.
    const SEED_LANGUAGE: Option<&'static str>;
    /// Whether a file's calls are read from its tape rather than a parse of its source: an
    /// embedding family's regions are its tape's.
    const TAPED: bool;
    /// Whether the graph indexes the family's workspace classes' direct subclasses and orphans,
    /// which its late-bound names read.
    const SUBCLASSES: bool;

    /// Whether the family reads `feature`.
    fn feature(feature: CallFeature) -> Result<(), Refusal>;

    /// The summary of a file's calls from `root`, a region of it, through `table`, the file's
    /// scope table where one was built.
    fn summary<D: Doc>(
        root: &Node<'_, D>,
        path: &str,
        module: Option<&ModulePath>,
        table: Option<&ScopeTable>,
        has_errors: bool,
    ) -> Summary;

    /// The open summary of a file that could not be read: what it names regardless.
    fn unread(module: Option<&ModulePath>) -> Summary;

    /// What `calls` reads in a file to name a callee its summary did not record, from the file's
    /// scope table, module path and PHP names; `None` where one it reads could not be built, or
    /// where it reads none.
    fn tables(
        table: impl FnOnce() -> Option<Arc<ScopeTable>>,
        module: impl FnOnce() -> Option<Arc<ModulePath>>,
        names: impl FnOnce() -> Option<Arc<PhpNames>>,
    ) -> Option<FileTables>;

    /// The callee of the call `node`, in a file whose summary is `summary`, through `tables`;
    /// `None` where they could not be built.
    fn callee_at<'t, D: Doc>(
        node: &Node<'t, D>,
        summary: &Summary,
        tables: impl FnOnce() -> Option<FileTables>,
    ) -> Option<Option<Callee>>;

    /// Gives `key` each name a definition named `name` (with `flags`) is linked by;
    /// `spellings` gives a name's spellings (by its module's name and by its path's).
    fn definition_keys<I: Iterator<Item = Name>>(
        name: &Name,
        flags: u8,
        spellings: impl FnOnce(&Name) -> I,
        key: impl FnMut(Name),
    );

    /// A definition's qualified name `name`, as its language writes it.
    fn label(name: &Name) -> Cow<'_, str>;

    /// What a call of `name` links to in `graph`, `hops` aliases in, noting the names it read in
    /// `followed`.
    fn link(graph: &Graph, name: &Name, hops: u32, followed: &mut Vec<Name>) -> Link;
}

impl CallRules for Python {
    const SUPPORTED: bool = true;
    const CALL_KINDS: &'static [&'static str] = &["call"];
    const FUNCTION_KINDS: &'static [&'static str] = &["function_definition"];
    const SEED_LANGUAGE: Option<&'static str> = None;
    const TAPED: bool = false;
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
        Summary::extract(root, table, module, has_errors)
    }

    /// Python names the module, whose members may hide in it.
    fn unread(module: Option<&ModulePath>) -> Summary {
        Summary::extract(
            &GrammarDialect::Python.ast_grep("").root(),
            None,
            module,
            true,
        )
    }

    fn tables(
        table: impl FnOnce() -> Option<Arc<ScopeTable>>,
        module: impl FnOnce() -> Option<Arc<ModulePath>>,
        _: impl FnOnce() -> Option<Arc<PhpNames>>,
    ) -> Option<FileTables> {
        Some(FileTables::Python(table()?, module()))
    }

    fn callee_at<'t, D: Doc>(
        node: &Node<'t, D>,
        summary: &Summary,
        tables: impl FnOnce() -> Option<FileTables>,
    ) -> Option<Option<Callee>> {
        let Some(FileTables::Python(table, module)) = tables() else {
            return None;
        };
        Some(crate::code_rule_dsl::calls::summary::callee_at(
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

impl CallRules for Php {
    const SUPPORTED: bool = true;
    const CALL_KINDS: &'static [&'static str] = &php::CALL_KINDS;
    const FUNCTION_KINDS: &'static [&'static str] = &php::FUNCTION_KINDS;
    const SEED_LANGUAGE: Option<&'static str> = None;
    const TAPED: bool = false;
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
        crate::code_rule_dsl::calls::extract_php(root, path, table, has_errors)
    }

    /// A PHP file names nothing.
    fn unread(_: Option<&ModulePath>) -> Summary {
        Summary::default()
    }

    fn tables(
        table: impl FnOnce() -> Option<Arc<ScopeTable>>,
        _: impl FnOnce() -> Option<Arc<ModulePath>>,
        names: impl FnOnce() -> Option<Arc<PhpNames>>,
    ) -> Option<FileTables> {
        let table = table()?;
        Some(FileTables::Php(table, names()?))
    }

    fn callee_at<'t, D: Doc>(
        node: &Node<'t, D>,
        summary: &Summary,
        tables: impl FnOnce() -> Option<FileTables>,
    ) -> Option<Option<Callee>> {
        let Some(FileTables::Php(table, names)) = tables() else {
            return None;
        };
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
            Name::new(&text.to_ascii_lowercase())
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

/// A template's calls are those of the PHP it embeds (`blade::view`), read from its tape; its
/// top level is no class's, so `$this`, `self` and `static` there are unknown. Its seeds are the
/// workspace's PHP definitions, which its calls reach: its seed query is PHP's.
impl CallRules for Blade {
    const SUPPORTED: bool = <Php as CallRules>::SUPPORTED;
    const CALL_KINDS: &'static [&'static str] = <Php as CallRules>::CALL_KINDS;
    const FUNCTION_KINDS: &'static [&'static str] = <Php as CallRules>::FUNCTION_KINDS;
    const SEED_LANGUAGE: Option<&'static str> = Some("php");
    const TAPED: bool = true;
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

/// TypeScript, TSX and JavaScript: the registry refuses `calls`.
impl CallRules for Refused {
    const SUPPORTED: bool = false;
    const CALL_KINDS: &'static [&'static str] = &[];
    const FUNCTION_KINDS: &'static [&'static str] = &[];
    const SEED_LANGUAGE: Option<&'static str> = None;
    const TAPED: bool = false;
    const SUBCLASSES: bool = false;

    fn feature(feature: CallFeature) -> Result<(), Refusal> {
        match feature {
            CallFeature::Calls => Err(Refusal::Refused(Cow::Borrowed(
                "calls follows Python and PHP calls only",
            ))),
        }
    }

    fn summary<D: Doc>(
        _: &Node<'_, D>,
        _: &str,
        _: Option<&ModulePath>,
        _: Option<&ScopeTable>,
        _: bool,
    ) -> Summary {
        Summary::default()
    }

    fn unread(_: Option<&ModulePath>) -> Summary {
        Summary::default()
    }

    fn tables(
        _: impl FnOnce() -> Option<Arc<ScopeTable>>,
        _: impl FnOnce() -> Option<Arc<ModulePath>>,
        _: impl FnOnce() -> Option<Arc<PhpNames>>,
    ) -> Option<FileTables> {
        None
    }

    fn callee_at<'t, D: Doc>(
        _: &Node<'t, D>,
        _: &Summary,
        _: impl FnOnce() -> Option<FileTables>,
    ) -> Option<Option<Callee>> {
        None
    }

    fn definition_keys<I: Iterator<Item = Name>>(
        _: &Name,
        _: u8,
        _: impl FnOnce(&Name) -> I,
        _: impl FnMut(Name),
    ) {
    }

    fn label(name: &Name) -> Cow<'_, str> {
        Cow::Borrowed(name.text())
    }

    fn link(_: &Graph, _: &Name, _: u32, _: &mut Vec<Name>) -> Link {
        Link::default()
    }
}

#[cfg(test)]
pub(super) mod kit {
    use super::{CallFeature, CallRules};
    use crate::code_rule_dsl::calls::names::Name;
    use crate::code_rule_dsl::language::GrammarDialect;
    use crate::code_rule_dsl::language::adapter::{Adapter, Refusal};
    use crate::code_rule_dsl::module_path::ModulePath;
    use crate::code_rule_dsl::tape::{Grammar, TapeDoc, TapeImage};
    use ast_grep_core::AstGrep;
    use ast_grep_core::tree_sitter::StrDoc;
    use std::sync::Arc;

    /// What one family's calls answer.
    pub(crate) struct Calls {
        pub(crate) call_kinds: &'static [&'static str],
        pub(crate) function_kinds: &'static [&'static str],
        /// `CallFeature::Calls`'s refusal, if the family refuses it.
        pub(crate) refusal: Option<&'static str>,
        pub(crate) seed_language: Option<&'static str>,
        pub(crate) taped: bool,
        pub(crate) subclasses: bool,
        /// A source in the family's dialect, and the names of the definitions its regions'
        /// summaries record.
        pub(crate) summary: (&'static str, &'static [&'static str]),
        /// The module an unread file of module `pkg.mod` names.
        pub(crate) unread: Option<&'static str>,
        /// A definition's name and its label.
        pub(crate) label: (&'static str, &'static str),
        /// Definitions' names and flags, and the keys each is linked by where its spellings are
        /// itself and `alt.f`.
        pub(crate) keys: &'static [(&'static str, u8, &'static [&'static str])],
    }

    /// Checks `A`'s calls against `expect`, its documents read in `dialect`.
    pub(crate) fn check<A: Adapter>(expect: &Calls, dialect: GrammarDialect) {
        type C<A> = <A as Adapter>::Calls;
        assert_eq!(<C<A> as CallRules>::CALL_KINDS, expect.call_kinds);
        assert_eq!(<C<A> as CallRules>::FUNCTION_KINDS, expect.function_kinds);
        let refusal = match <C<A> as CallRules>::feature(CallFeature::Calls) {
            Ok(()) => None,
            Err(Refusal::Refused(message)) => Some(message.into_owned()),
            Err(Refusal::NotSupportedYet(message)) => panic!("not supported yet: {message}"),
        };
        assert_eq!(refusal.as_deref(), expect.refusal);
        assert_eq!(<C<A> as CallRules>::SEED_LANGUAGE, expect.seed_language);
        assert_eq!(<C<A> as CallRules>::TAPED, expect.taped);
        assert_eq!(<C<A> as CallRules>::SUBCLASSES, expect.subclasses);

        let (source, defs) = expect.summary;
        let grammar = Grammar::new(dialect).expect("grammar");
        let native = StrDoc::new(source, dialect);
        let image = TapeImage::encode(native.tree.root_node(), &grammar, source, &|| false)
            .expect("encodes");
        let doc = AstGrep::doc(
            TapeDoc::restore(Arc::new(source.to_owned()), grammar, &image, &|| false)
                .expect("restores"),
        );
        let root = doc.root();
        let read = A::regions(&root)
            .flat_map(|region| {
                let summary = <C<A> as CallRules>::summary(&region.root, "kit", None, None, false);
                summary
                    .defs
                    .iter()
                    .map(|def| summary.name(def.name).to_owned())
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        assert_eq!(read, defs, "{source:?}");

        let module = ModulePath::dotted("pkg.mod").expect("module");
        let unread = <C<A> as CallRules>::unread(Some(&module));
        assert_eq!(
            unread.module.map(|module| unread.name(module)),
            expect.unread
        );

        let (name, label) = expect.label;
        assert_eq!(<C<A> as CallRules>::label(&Name::new(name)), label);

        for (name, flags, keys) in expect.keys {
            let mut read = Vec::new();
            <C<A> as CallRules>::definition_keys(
                &Name::new(name),
                *flags,
                |name| [name.clone(), Name::new("alt.f")].into_iter(),
                |key| read.push(key.text().to_owned()),
            );
            assert_eq!(read, *keys, "{name} {flags}");
        }
    }
}
