//! A family's syntax beyond its semantics: the literals `parse` reads, the options a policy
//! declares (`languageOptions`, `when`), and how its patterns are spelled and parsed.

use super::{Adapter, Blade, Ecma, Php, Python, Refusal};
use crate::code_rule_dsl::compile::config::Options;
use crate::code_rule_dsl::compile::options;
use crate::code_rule_dsl::language::{GrammarDialect, blade_text};
use crate::code_rule_dsl::literal::{Grammar, LiteralHost};
use crate::code_rule_dsl::scope::options::{LanguageOptions, PythonOptions};
use serde_json::Value;
use std::borrow::Cow;
use std::sync::Arc;

/// An option of a policy a family may not read.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum SyntaxFeature {
    /// `parse` with a grammar: the family's literals the grammar's trees are parsed from.
    Parse(Grammar),
    /// A policy's `languageOptions`.
    LanguageOptions,
}

/// An ast query's compiled `languageOptions`: ESLint's settings, and Ruff's.
pub(crate) type QueryOptions = (Option<Arc<LanguageOptions>>, Option<Arc<PythonOptions>>);

/// A family's syntax.
pub(crate) trait SyntaxRules: Adapter {
    /// Whether a pattern's `$`-prefixed leaf that is no metavariable is a name of the language
    /// (Python's `$At` an identifier, PHP's `$x` a variable), spelled back with `$`.
    const DOLLAR_NAMES: bool;
    /// Whether the family reads `feature`, or why not.
    fn feature(feature: SyntaxFeature) -> Result<(), Refusal>;
    /// The `languageOptions` (`value`) of the ast query at `path`, where `feature` reads them:
    /// ESLint's settings, or Ruff's.
    fn compile_language_options(value: &Value, path: &str) -> napi::Result<QueryOptions>;
    /// Whether the ast query at `path` applies, by its `when` (`value`) read against its
    /// compiled `languageOptions` (`options`, `python`) and the document's options.
    fn compile_when(
        value: &Value,
        path: &str,
        options: Option<&LanguageOptions>,
        python: Option<&PythonOptions>,
        document: &Options,
    ) -> napi::Result<bool>;
    /// Whether a pattern of `text` (a pattern's source, or a contextual pattern's context) is
    /// the language the template embeds rather than the template's own: Blade's text with no
    /// Blade construct is PHP (`rule_pattern`).
    fn embeds_pattern(text: &str) -> bool;
    /// Whether the `comment` node of kind `id` in `language` is a comment of the PHP the family's
    /// documents hold, a PHP doc comment's host: Blade's own `{{-- --}}` comments are not.
    fn php_comment(language: GrammarDialect, id: u16) -> bool;
}

/// The refusal of `parse` with `grammar` by a family whose literals it does not read (`reason`
/// naming the family's counterpart, if any).
fn unhosted(grammar: Grammar, reason: &str) -> Result<(), Refusal> {
    Err(Refusal::Refused(Cow::Owned(format!(
        "`{}` parses {}{reason}",
        grammar.name(),
        grammar.host()
    ))))
}

fn refused(message: &'static str) -> Result<(), Refusal> {
    Err(Refusal::Refused(Cow::Borrowed(message)))
}

/// String literals: formats and annotations.
impl SyntaxRules for Python {
    const DOLLAR_NAMES: bool = true;

    fn feature(feature: SyntaxFeature) -> Result<(), Refusal> {
        match feature {
            SyntaxFeature::LanguageOptions => Ok(()),
            SyntaxFeature::Parse(grammar) => match grammar.literal_host() {
                LiteralHost::PythonString => Ok(()),
                LiteralHost::EcmaComment | LiteralHost::PhpComment => unhosted(grammar, ""),
                LiteralHost::PhpString => match grammar {
                    Grammar::Scanf => unhosted(grammar, "; Python has no scanf formats"),
                    _ => unhosted(grammar, "; Python's printf formats are `percentFormat`"),
                },
            },
        }
    }

    fn compile_language_options(value: &Value, path: &str) -> napi::Result<QueryOptions> {
        let python = options::compile_python_options(value, path)?;
        Ok((None, Some(Arc::new(python))))
    }

    fn compile_when(
        value: &Value,
        path: &str,
        _: Option<&LanguageOptions>,
        python: Option<&PythonOptions>,
        document: &Options,
    ) -> napi::Result<bool> {
        options::compile_python_when(value, path, python, document)
    }

    fn embeds_pattern(_: &str) -> bool {
        false
    }

    fn php_comment(_: GrammarDialect, _: u16) -> bool {
        true
    }
}

/// Comments: ESLint's directives.
impl SyntaxRules for Ecma {
    const DOLLAR_NAMES: bool = false;

    fn feature(feature: SyntaxFeature) -> Result<(), Refusal> {
        match feature {
            SyntaxFeature::LanguageOptions => Ok(()),
            SyntaxFeature::Parse(grammar) => match grammar.literal_host() {
                LiteralHost::EcmaComment => Ok(()),
                LiteralHost::PythonString | LiteralHost::PhpString | LiteralHost::PhpComment => {
                    unhosted(grammar, "")
                }
            },
        }
    }

    fn compile_language_options(value: &Value, path: &str) -> napi::Result<QueryOptions> {
        let options = options::compile_language_options(value, path)?;
        Ok((Some(Arc::new(options)), None))
    }

    fn compile_when(
        value: &Value,
        path: &str,
        options: Option<&LanguageOptions>,
        _: Option<&PythonOptions>,
        document: &Options,
    ) -> napi::Result<bool> {
        options::compile_ecma_when(value, path, options, document)
    }

    fn embeds_pattern(_: &str) -> bool {
        false
    }

    fn php_comment(_: GrammarDialect, _: u16) -> bool {
        true
    }
}

/// Strings (printf and scanf formats) and doc comments.
impl SyntaxRules for Php {
    const DOLLAR_NAMES: bool = true;

    fn feature(feature: SyntaxFeature) -> Result<(), Refusal> {
        match feature {
            SyntaxFeature::LanguageOptions => {
                refused("`languageOptions` is not available for php policies")
            }
            SyntaxFeature::Parse(grammar) => match grammar.literal_host() {
                LiteralHost::PhpString | LiteralHost::PhpComment => Ok(()),
                LiteralHost::EcmaComment => unhosted(grammar, ""),
                LiteralHost::PythonString => match grammar {
                    Grammar::Percent => {
                        unhosted(grammar, "; PHP's printf formats are `printfFormat`")
                    }
                    Grammar::Brace => unhosted(
                        grammar,
                        "; PHP has no str.format templates (its printf formats are `printfFormat`)",
                    ),
                    Grammar::Annotation => unhosted(
                        grammar,
                        "; PHP holds no type in a string: its declared types are syntax, and its PHPDoc types are `phpDoc`",
                    ),
                    _ => unhosted(grammar, ""),
                },
            },
        }
    }

    fn compile_language_options(value: &Value, path: &str) -> napi::Result<QueryOptions> {
        <Ecma as SyntaxRules>::compile_language_options(value, path)
    }

    fn compile_when(
        value: &Value,
        path: &str,
        options: Option<&LanguageOptions>,
        python: Option<&PythonOptions>,
        document: &Options,
    ) -> napi::Result<bool> {
        <Ecma as SyntaxRules>::compile_when(value, path, options, python, document)
    }

    fn embeds_pattern(_: &str) -> bool {
        false
    }

    fn php_comment(_: GrammarDialect, _: u16) -> bool {
        true
    }
}

/// The literals of the PHP a template embeds (`blade::view`); a pattern with no Blade construct
/// is that PHP's.
impl SyntaxRules for Blade {
    const DOLLAR_NAMES: bool = false;

    fn feature(feature: SyntaxFeature) -> Result<(), Refusal> {
        match feature {
            SyntaxFeature::LanguageOptions => {
                refused("`languageOptions` is not available for blade policies")
            }
            SyntaxFeature::Parse(grammar) => match grammar.literal_host() {
                LiteralHost::PhpString | LiteralHost::PhpComment => Ok(()),
                LiteralHost::EcmaComment => unhosted(grammar, ""),
                LiteralHost::PythonString => match grammar {
                    Grammar::Percent => unhosted(
                        grammar,
                        "; blade embeds PHP, whose printf formats are `printfFormat`",
                    ),
                    Grammar::Brace => unhosted(
                        grammar,
                        "; blade embeds PHP, which has no str.format templates (its printf formats are `printfFormat`)",
                    ),
                    Grammar::Annotation => unhosted(
                        grammar,
                        "; blade embeds PHP, which holds no type in a string: its declared types are syntax, and its PHPDoc types are `phpDoc`",
                    ),
                    _ => unhosted(grammar, ""),
                },
            },
        }
    }

    fn compile_language_options(value: &Value, path: &str) -> napi::Result<QueryOptions> {
        <Php as SyntaxRules>::compile_language_options(value, path)
    }

    fn compile_when(
        value: &Value,
        path: &str,
        options: Option<&LanguageOptions>,
        python: Option<&PythonOptions>,
        document: &Options,
    ) -> napi::Result<bool> {
        <Php as SyntaxRules>::compile_when(value, path, options, python, document)
    }

    fn embeds_pattern(text: &str) -> bool {
        blade_text(text)
    }

    fn php_comment(language: GrammarDialect, id: u16) -> bool {
        language.is_embedded_kind(id)
    }
}

/// The kit's check of a family's `SyntaxRules`.
#[cfg(test)]
pub(super) mod kit {
    use super::{SyntaxFeature, SyntaxRules};
    use crate::code_rule_dsl::compile::config::Options;
    use crate::code_rule_dsl::language::GrammarDialect;
    use crate::code_rule_dsl::language::adapter::Refusal;
    use crate::code_rule_dsl::literal::Grammar;
    use crate::code_rule_dsl::tape::TapeDoc;
    use ast_grep_core::AstGrep;

    /// What a family's `SyntaxRules` answer.
    pub(in super::super) struct Expect {
        pub(in super::super) dollar_names: bool,
        /// The refusal of `languageOptions`; `None` where read.
        pub(in super::super) language_options: Option<&'static str>,
        /// Where read, a `languageOptions`, and the ESLint `ecmaVersion` and Python minor it
        /// compiles to.
        pub(in super::super) compiled_options: Option<(&'static str, Option<u16>, Option<u8>)>,
        /// `when`s, and whether a query without `languageOptions` applies under each, or the
        /// error compiling it.
        pub(in super::super) when: &'static [(&'static str, Result<bool, &'static str>)],
        /// The refusals of `parse` with each grammar, in `Grammar::ALL`'s order; `None` where
        /// the family's literals host it.
        pub(in super::super) parse: [Option<&'static str>; 7],
        /// Patterns' texts, and whether each is the embedded language's.
        pub(in super::super) embeds: &'static [(&'static str, bool)],
        /// A document, and how many of its `comment` nodes are PHP comments.
        pub(in super::super) php_comments: (&'static str, usize),
    }

    /// A verdict as the kit compares it: `None` where read, else the refusal's message,
    /// marked where it is not-supported-yet.
    pub(in super::super) fn refusal(verdict: Result<(), Refusal>) -> Option<String> {
        match verdict {
            Ok(()) => None,
            Err(Refusal::Refused(message)) => Some(message.into_owned()),
            Err(Refusal::NotSupportedYet(message)) => Some(format!("not supported yet: {message}")),
        }
    }

    pub(in super::super) fn check<A: SyntaxRules>(
        expect: &Expect,
        dialect: GrammarDialect,
        tape: impl Fn(&str) -> AstGrep<TapeDoc>,
    ) {
        if let Some((source, ecma_version, python_minor)) = expect.compiled_options {
            let value = serde_json::from_str(source).expect("JSON");
            let (options, python) =
                A::compile_language_options(&value, "/q").expect("languageOptions compile");
            assert_eq!(
                (
                    options.map(|options| options.ecma_version),
                    python.map(|python| python.minor)
                ),
                (ecma_version, python_minor),
                "{source}"
            );
        }
        for (source, applies) in expect.when {
            let value = serde_json::from_str(source).expect("JSON");
            assert_eq!(
                A::compile_when(&value, "/q", None, None, &Options::default())
                    .map_err(|error| error.reason),
                applies.map_err(str::to_owned),
                "{source}"
            );
        }
        assert_eq!(A::DOLLAR_NAMES, expect.dollar_names);
        assert_eq!(
            refusal(A::feature(SyntaxFeature::LanguageOptions)),
            expect.language_options.map(str::to_owned)
        );
        let parse = [
            Grammar::Percent,
            Grammar::Brace,
            Grammar::Annotation,
            Grammar::EslintDirective,
            Grammar::Printf,
            Grammar::Scanf,
            Grammar::PhpDoc,
        ]
        .map(|grammar| refusal(A::feature(SyntaxFeature::Parse(grammar))));
        assert_eq!(parse, expect.parse.map(|m| m.map(str::to_owned)));
        for (text, embeds) in expect.embeds {
            assert_eq!(A::embeds_pattern(text), *embeds, "{text:?}");
        }
        let (source, count) = expect.php_comments;
        let doc = tape(source);
        let comments = doc
            .root()
            .dfs()
            .filter(|node| node.kind() == "comment" && A::php_comment(dialect, node.kind_id()))
            .count();
        assert_eq!(comments, count, "{source:?}");
    }
}
