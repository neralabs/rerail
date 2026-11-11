//! The workspace call graph (`calls`): `Semantic::Calls`.

use super::{Blade, Php, Python, Refusal, Refused};
use crate::code_rule_dsl::calls::closure::{Graph, Link};
use crate::code_rule_dsl::calls::names::Name;
use crate::code_rule_dsl::calls::summary::{Callee, SHADOWED, Summary};
use crate::code_rule_dsl::calls::{FileTables, php};
use crate::code_rule_dsl::language::{Family, GrammarDialect};
use crate::code_rule_dsl::module_path::ModulePath;
use crate::code_rule_dsl::scope::ScopeTable;
use crate::code_rule_dsl::scope::php_names::PhpNames;
use ast_grep_core::{Doc, Node};
use ast_grep_language::LanguageExt;
use std::borrow::Cow;
use std::sync::Arc;
pub(in crate::code_rule_dsl) mod lang_python;
pub(in crate::code_rule_dsl) mod lang_php;
pub(in crate::code_rule_dsl) mod lang_blade;

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
    /// The namespace used by names held in this family's summaries.
    const NAMES: Option<Family>;
    /// The node kinds of a call: the only nodes a `calls` term holds at.
    const CALL_KINDS: &'static [&'static str];
    /// The node kinds of what a call may reach: the definitions a seed query runs on.
    const FUNCTION_KINDS: &'static [&'static str];
    /// The language a seed query of the family's rule runs in, where not the rule's own.
    const SEED_LANGUAGE: Option<&'static str>;
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

/// TypeScript, TSX and JavaScript: the registry refuses `calls`.
impl CallRules for Refused {
    const SUPPORTED: bool = false;
    const NAMES: Option<Family> = None;
    const CALL_KINDS: &'static [&'static str] = &[];
    const FUNCTION_KINDS: &'static [&'static str] = &[];
    const SEED_LANGUAGE: Option<&'static str> = None;
    const SUBCLASSES: bool = false;

    fn feature(feature: CallFeature) -> Result<(), Refusal> {
        match feature {
            CallFeature::Calls => Err(Refusal::Refused(Cow::Borrowed(
                "calls follows Python, PHP and Rust calls only",
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
    use std::sync::Arc;

    /// What one family's calls answer.
    pub(crate) struct Calls {
        pub(crate) call_kinds: &'static [&'static str],
        pub(crate) function_kinds: &'static [&'static str],
        /// `CallFeature::Calls`'s refusal, if the family refuses it.
        pub(crate) refusal: Option<&'static str>,
        pub(crate) seed_language: Option<&'static str>,
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
        assert_eq!(<C<A> as CallRules>::SUBCLASSES, expect.subclasses);

        let (source, defs) = expect.summary;
        let grammar = Grammar::new(dialect).expect("grammar");
        let image = TapeImage::parse(&grammar, source, &|| false)
            .expect("parses")
            .expect("encodes");
        let doc = TapeDoc::restore(Arc::new(source.to_owned()), grammar, &image, &|| false)
            .expect("restores");
        // A file's calls are its own tree's, or its embedded program's (`calls_document`).
        let read = crate::code_rule_dsl::calls::calls_document(doc, dialect)
            .map(|(doc, _)| {
                let doc = AstGrep::doc(doc);
                let summary = <C<A> as CallRules>::summary(&doc.root(), "kit", None, None, false);
                summary
                    .defs
                    .iter()
                    .map(|def| summary.name(def.name).to_owned())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        assert_eq!(read, defs, "{source:?}");

        let module = ModulePath::dotted("pkg.mod").expect("module");
        let unread = <C<A> as CallRules>::unread(Some(&module));
        assert_eq!(
            unread.module.map(|module| unread.name(module)),
            expect.unread
        );

        let (name, label) = expect.label;
        assert_eq!(
            <C<A> as CallRules>::label(&Name::new(A::FAMILY, name)),
            label
        );

        for (name, flags, keys) in expect.keys {
            let mut read = Vec::new();
            <C<A> as CallRules>::definition_keys(
                &Name::new(A::FAMILY, name),
                *flags,
                |name| [name.clone(), Name::new(A::FAMILY, "alt.f")].into_iter(),
                |key| read.push(key.text().to_owned()),
            );
            assert_eq!(read, *keys, "{name} {flags}");
        }
    }
}
