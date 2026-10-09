//! The adapter test kit: every family's adapter against known-good literals (`kit`), and the
//! registry's capabilities against the concerns the adapters read.

use super::{
    Adapter, ArgumentRules, Blade, CallRules, Ecma, FormatArgRules, Php, Python, ScopeRules,
    ValueRules, with_adapter,
};
use crate::code_rule_dsl::language::{CodeRuleLanguage, Family, GrammarDialect, Semantic, Support};
use crate::code_rule_dsl::lines::Terminators;
use crate::code_rule_dsl::tape::{Grammar, TapeDoc, TapeImage};
use ast_grep_core::AstGrep;
use ast_grep_core::tree_sitter::StrDoc;
use std::borrow::Cow;
use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;

/// A document, and the kind and range of each of its regions' roots.
type Document = (&'static str, &'static [(&'static str, Range<usize>)]);

/// What one family's adapter answers.
struct Expect {
    family: Family,
    label: &'static str,
    terminators: Terminators,
    /// The dialect the family's documents are read in.
    dialect: GrammarDialect,
    /// The name of the identifier `ﬁle`.
    name: &'static str,
    regions: &'static [Document],
    /// The suffix of the family's files, and the module of `pkg/mod<suffix>` in a package `pkg`.
    module: (&'static str, Option<&'static str>),
    syntax: super::syntax::kit::Expect,
}

/// A tape of `source` in `dialect`, as files are read.
fn tape(source: &str, dialect: GrammarDialect) -> AstGrep<TapeDoc> {
    let grammar = Grammar::new(dialect).expect("grammar");
    let native = StrDoc::new(source, dialect);
    let image =
        TapeImage::encode(native.tree.root_node(), &grammar, source, &|| false).expect("encodes");
    AstGrep::doc(
        TapeDoc::restore(Arc::new(source.to_owned()), grammar, &image, &|| false)
            .expect("restores"),
    )
}

/// A directory holding a package `pkg`, removed when dropped.
struct Package(PathBuf);

impl Package {
    fn new(label: &str, suffix: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("rerail-adapter-kit-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("pkg")).expect("package directory");
        std::fs::write(root.join("pkg/__init__.py"), "").expect("package marker");
        std::fs::write(root.join(format!("pkg/mod{suffix}")), "").expect("module");
        Self(root)
    }
}

impl Drop for Package {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn kit<A: Adapter>(expect: &Expect) {
    assert_eq!(A::FAMILY, expect.family);
    assert_eq!(with_adapter!(expect.family, B => B::LABEL), expect.label);
    assert_eq!(A::LABEL, expect.label);
    assert_eq!(A::TERMINATORS, expect.terminators);
    assert_eq!(A::name(Cow::Borrowed("ﬁle")), expect.name);
    assert!(matches!(
        A::name(Cow::Borrowed("file")),
        Cow::Borrowed("file")
    ));
    for (source, regions) in expect.regions {
        let doc = tape(source, expect.dialect);
        let root = doc.root();
        let read = A::regions(&root)
            .map(|region| (region.root.kind().into_owned(), region.root.range()))
            .collect::<Vec<_>>();
        let expected = regions
            .iter()
            .map(|(kind, range)| ((*kind).to_owned(), range.clone()))
            .collect::<Vec<_>>();
        assert_eq!(read, expected, "{source:?}");
    }
    let (suffix, module) = expect.module;
    let package = Package::new(expect.label, suffix);
    assert_eq!(
        A::module_path(&package.0.join(format!("pkg/mod{suffix}"))),
        module.map(|module| serde_json::Value::String(module.to_owned()))
    );
    with_adapter!(expect.family, B => super::syntax::kit::check::<B>(&expect.syntax, expect.dialect, |s| tape(s, expect.dialect)));
}

#[test]
fn python() {
    kit::<Python>(&Expect {
        family: Family::Python,
        label: "python",
        terminators: Terminators::Newlines,
        dialect: GrammarDialect::Python,
        name: "file",
        regions: &[("x = 1\n", &[("module", 0..6)])],
        module: (".py", Some("pkg.mod")),
        syntax: super::syntax::kit::Expect {
            option_keys: super::OptionKeys::Python,
            dollar_names: true,
            language_options: None,
            parse: [
                None,
                None,
                None,
                Some("`eslintDirective` parses TypeScript comments"),
                Some(
                    "`printfFormat` parses PHP strings; Python's printf formats are `percentFormat`",
                ),
                Some("`scanfFormat` parses PHP strings; Python has no scanf formats"),
                Some("`phpDoc` parses PHP doc comments"),
            ],
            embeds: &[("sprintf($F, $$$A)", false)],
            php_comments: ("# c\nx = 1\n", 1),
        },
    });
}

#[test]
fn ecma() {
    kit::<Ecma>(&Expect {
        family: Family::Ecma,
        label: "typescript",
        terminators: Terminators::Eslint,
        dialect: GrammarDialect::TypeScript,
        name: "ﬁle",
        regions: &[("let x = 1;\n", &[("program", 0..11)])],
        module: (".ts", None),
        syntax: super::syntax::kit::Expect {
            option_keys: super::OptionKeys::Ecma,
            dollar_names: false,
            language_options: None,
            parse: [
                Some("`percentFormat` parses Python literals"),
                Some("`braceFormat` parses Python literals"),
                Some("`annotation` parses Python literals"),
                None,
                Some("`printfFormat` parses PHP strings"),
                Some("`scanfFormat` parses PHP strings"),
                Some("`phpDoc` parses PHP doc comments"),
            ],
            embeds: &[("sprintf($F, $$$A)", false)],
            php_comments: ("// c\nlet x = 1;\n", 1),
        },
    });
}

#[test]
fn php() {
    kit::<Php>(&Expect {
        family: Family::Php,
        label: "php",
        terminators: Terminators::Newlines,
        dialect: GrammarDialect::Php,
        name: "ﬁle",
        regions: &[("<?php $x = 1;\n", &[("program", 0..14)])],
        module: (".php", None),
        syntax: super::syntax::kit::Expect {
            option_keys: super::OptionKeys::Ecma,
            dollar_names: true,
            language_options: Some("`languageOptions` is not available for php policies"),
            parse: [
                Some(
                    "`percentFormat` parses Python literals; PHP's printf formats are `printfFormat`",
                ),
                Some(
                    "`braceFormat` parses Python literals; PHP has no str.format templates (its printf formats are `printfFormat`)",
                ),
                Some(
                    "`annotation` parses Python literals; PHP holds no type in a string: its declared types are syntax, and its PHPDoc types are `phpDoc`",
                ),
                Some("`eslintDirective` parses TypeScript comments"),
                None,
                None,
                None,
            ],
            embeds: &[("sprintf($F, $$$A)", false)],
            php_comments: ("<?php /** d */ // c\n", 2),
        },
    });
}

#[test]
fn blade() {
    kit::<Blade>(&Expect {
        family: Family::Blade,
        label: "blade",
        terminators: Terminators::Newlines,
        dialect: GrammarDialect::Blade,
        name: "ﬁle",
        regions: &[
            ("<p>{{ $x }}</p>\n", &[("program", 3..11)]),
            ("<p>x</p>\n", &[]),
        ],
        module: (".blade.php", None),
        syntax: super::syntax::kit::Expect {
            option_keys: super::OptionKeys::Ecma,
            dollar_names: false,
            language_options: Some("`languageOptions` is not available for blade policies"),
            parse: [
                Some(
                    "`percentFormat` parses Python literals; blade embeds PHP, whose printf formats are `printfFormat`",
                ),
                Some(
                    "`braceFormat` parses Python literals; blade embeds PHP, which has no str.format templates (its printf formats are `printfFormat`)",
                ),
                Some(
                    "`annotation` parses Python literals; blade embeds PHP, which holds no type in a string: its declared types are syntax, and its PHPDoc types are `phpDoc`",
                ),
                Some("`eslintDirective` parses TypeScript comments"),
                None,
                None,
                None,
            ],
            embeds: &[("sprintf($F, $$$A)", true), ("<p>{{ $x }}</p>", false)],
            php_comments: ("{{-- a --}}\n@php /** b */ @endphp\n", 1),
        },
    });
}

/// Whether `A` reads `semantic`'s concern.
fn reads<A: Adapter>(semantic: Semantic) -> bool {
    match semantic {
        Semantic::Scope => <A::Scope as ScopeRules>::SUPPORTED,
        Semantic::Value => <A::Value as ValueRules>::SUPPORTED,
        Semantic::Arguments => <A::Arguments as ArgumentRules>::SUPPORTED,
        Semantic::Calls => <A::Calls as CallRules>::SUPPORTED,
        Semantic::FormatArgs => <A::FormatArgs as FormatArgRules>::SUPPORTED,
    }
}

/// A language's registry entry supports a capability exactly where its family's adapter reads
/// the concern.
#[test]
fn adapters_read_the_capabilities_the_registry_supports() {
    for language in CodeRuleLanguage::ALL {
        for semantic in [
            Semantic::Scope,
            Semantic::Value,
            Semantic::Arguments,
            Semantic::Calls,
            Semantic::FormatArgs,
        ] {
            assert_eq!(
                language.capability(semantic) == Support::Supported,
                with_adapter!(language.family(), A => reads::<A>(semantic)),
                "{} {semantic:?}",
                language.name()
            );
        }
    }
}
