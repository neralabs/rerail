//! A family's syntax beyond its semantics: the literals `parse` reads, the options a policy
//! declares (`languageOptions`, `when`), and how its patterns are spelled and parsed.

use super::{Adapter, Refusal};
use crate::code_rule_dsl::compile::config::Options;
use crate::code_rule_dsl::compile::options;
use crate::code_rule_dsl::language::SourceType;
use crate::code_rule_dsl::literal::Grammar;
use crate::code_rule_dsl::scope::options::{LanguageOptions, PythonOptions};
use ast_grep_core::{Doc, Node};
use serde_json::Value;
use std::borrow::Cow;
use std::sync::Arc;
pub(in crate::code_rule_dsl) mod lang_python;
pub(in crate::code_rule_dsl) mod lang_ecma;
pub(in crate::code_rule_dsl) mod lang_php;
pub(in crate::code_rule_dsl) mod lang_blade;

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

/// Whether an ast query applies by its `when`, and the source types a file must have where the
/// query leaves `sourceType` to each file (`SourceType::of_file`; `run_clause` checks them).
pub(crate) type QueryWhen = (bool, Option<Vec<SourceType>>);

/// A family's syntax.
pub(crate) trait SyntaxRules: Adapter {
    /// `node`'s tree under `grammar`, a grammar of this family's literals (`LiteralGrammar::host_language`):
    /// `interpolation` marks an interpolated literal's fields. Spends one step of `spend` per byte and tree node;
    /// `None` when the budget ran out.
    fn parse_literal<D: Doc>(
        grammar: Grammar,
        node: &Node<'_, D>,
        interpolation: &str,
        spend: &mut dyn FnMut(usize) -> bool,
    ) -> Option<crate::code_rule_dsl::literal::Parsed> {
        let _ = (grammar, node, interpolation, spend);
        Some(crate::code_rule_dsl::literal::Parsed::NotString)
    }

    /// Whether a pattern's `$`-prefixed leaf that is no metavariable is a name of the language
    /// (Python's `$At` an identifier, PHP's `$x` a variable), spelled back with `$`.
    const DOLLAR_NAMES: bool;
    /// Whether the family reads `feature`, or why not.
    fn feature(feature: SyntaxFeature) -> Result<(), Refusal>;
    /// The `languageOptions` (`value`) of the ast query at `path`, where `feature` reads them:
    /// ESLint's settings, or Ruff's.
    fn compile_language_options(value: &Value, path: &str) -> napi::Result<QueryOptions>;
    /// Whether the ast query at `path` applies, by its `when` (`value`) read against its
    /// compiled `languageOptions` (`options`, `python`) and the document's options, and the
    /// source types it leaves to each file.
    fn compile_when(
        value: &Value,
        path: &str,
        options: Option<&LanguageOptions>,
        python: Option<&PythonOptions>,
        document: &Options,
    ) -> napi::Result<QueryWhen>;
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

/// The kit's check of a family's `SyntaxRules`.
#[cfg(test)]
pub(super) mod kit {
    use super::{SyntaxFeature, SyntaxRules};
    use crate::code_rule_dsl::compile::config::Options;
    use crate::code_rule_dsl::language::SourceType;
    use crate::code_rule_dsl::language::adapter::Refusal;
    use crate::code_rule_dsl::literal::Grammar;

    /// A `when`, and whether a query without `languageOptions` applies under it with the source
    /// types it leaves to each file, or the error compiling it.
    pub(in super::super) type When = (
        &'static str,
        Result<(bool, Option<&'static [SourceType]>), &'static str>,
    );

    /// What a family's `SyntaxRules` answer.
    pub(in super::super) struct Expect {
        pub(in super::super) dollar_names: bool,
        /// The refusal of `languageOptions`; `None` where read.
        pub(in super::super) language_options: Option<&'static str>,
        /// Where read, a `languageOptions`, and the ESLint `ecmaVersion` and Python minor it
        /// compiles to.
        pub(in super::super) compiled_options: Option<(&'static str, Option<u16>, Option<u8>)>,
        pub(in super::super) when: &'static [When],
        /// The refusals of `parse` with each grammar, in the registration order (`Grammar::all`); `None` where
        /// the family's literals host it.
        pub(in super::super) parse: [Option<&'static str>; 8],
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

    pub(in super::super) fn check<A: SyntaxRules>(expect: &Expect) {
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
                applies
                    .map(|(applies, types)| (applies, types.map(<[SourceType]>::to_vec)))
                    .map_err(str::to_owned),
                "{source}"
            );
        }
        assert_eq!(A::DOLLAR_NAMES, expect.dollar_names);
        assert_eq!(
            refusal(A::feature(SyntaxFeature::LanguageOptions)),
            expect.language_options.map(str::to_owned)
        );
        let parse = [
            "percentFormat",
            "braceFormat",
            "annotation",
            "eslintDirective",
            "printfFormat",
            "scanfFormat",
            "phpDoc",
            "rustFormat",
        ]
        .map(Grammar::expect)
        .map(|grammar| refusal(A::feature(SyntaxFeature::Parse(grammar))));
        assert_eq!(parse, expect.parse.map(|m| m.map(str::to_owned)));
    }
}
