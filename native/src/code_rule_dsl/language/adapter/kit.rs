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
    scope: super::scope::kit::Expect,
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
    super::scope::kit::kit::<A>(expect.dialect, expect.module.0, &expect.scope);
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
        scope: super::scope::kit::Expect {
            flow: false,
            globals: super::Globals::Ruff,
            stubs: true,
            redefines_shadowed: true,
            reference_kinds: &["identifier"],
            name_kinds: &["identifier", "dotted_name"],
            resolves: (
                "x = 1\nprint(x)\n",
                "x",
                Some(crate::code_rule_dsl::scope::Form::Assignment),
            ),
            refuses: &[
                (
                    super::ScopeFeature::This,
                    "`this` is TypeScript's; Python has none",
                ),
                (
                    super::ScopeFeature::Every,
                    "Python's redefines reads the one binding shadowed; `every` is TypeScript's",
                ),
            ],
            reads: &[
                super::ScopeFeature::Imports,
                super::ScopeFeature::StopAtGlobal,
            ],
            unpackaged: Some("mod"),
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
        scope: super::scope::kit::Expect {
            flow: false,
            globals: super::Globals::Eslint,
            stubs: false,
            redefines_shadowed: false,
            reference_kinds: &[
                "identifier",
                "type_identifier",
                "shorthand_property_identifier",
                "shorthand_property_identifier_pattern",
                "undefined",
            ],
            name_kinds: &[
                "identifier",
                "type_identifier",
                "property_identifier",
                "shorthand_property_identifier_pattern",
                "undefined",
            ],
            resolves: (
                "let x = 1;\nx;\n",
                "x",
                Some(crate::code_rule_dsl::scope::Form::Assignment),
            ),
            refuses: &[
                (
                    super::ScopeFeature::Imports,
                    "import resolution is Python-only for now; TypeScript rules can use `global`",
                ),
                (
                    super::ScopeFeature::Filter("latest"),
                    "`latest` reads Python's binding order and declarations; TypeScript has none",
                ),
            ],
            reads: &[super::ScopeFeature::This, super::ScopeFeature::LastWrite],
            unpackaged: None,
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
        scope: super::scope::kit::Expect {
            flow: true,
            globals: super::Globals::Superglobals,
            stubs: false,
            redefines_shadowed: false,
            reference_kinds: &["variable_name"],
            name_kinds: &["variable_name"],
            resolves: (
                "<?php $x = 1;\necho $x;\n",
                "$x",
                Some(crate::code_rule_dsl::scope::Form::Assignment),
            ),
            refuses: &[
                (
                    super::ScopeFeature::StarImports,
                    "php policies ask `unresolved: true`",
                ),
                (
                    super::ScopeFeature::SameImport,
                    "qualified import names are Python's; a PHP variable is never bound by an import",
                ),
            ],
            reads: &[super::ScopeFeature::Imports, super::ScopeFeature::Every],
            unpackaged: None,
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
        scope: super::scope::kit::Expect {
            flow: true,
            globals: super::Globals::Superglobals,
            stubs: false,
            redefines_shadowed: false,
            reference_kinds: &["variable_name"],
            name_kinds: &["variable_name"],
            resolves: (
                "@php\n    $x = 1;\n@endphp\n{{ $x }}\n",
                "$x",
                Some(crate::code_rule_dsl::scope::Form::Assignment),
            ),
            refuses: &[
                (
                    super::ScopeFeature::StarImports,
                    "blade policies ask `unresolved: true`",
                ),
                (
                    super::ScopeFeature::SameImport,
                    "qualified import names are Python's; a blade variable is never bound by an import",
                ),
            ],
            reads: &[super::ScopeFeature::Imports, super::ScopeFeature::Every],
            unpackaged: None,
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
