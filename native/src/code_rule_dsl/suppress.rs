//! `suppress` (a Rerail extension, opt-in per violation): another linter's suppressions, honoured
//! as that linter honours them. The engine never reads them otherwise.

use super::json::{invalid, object, reject_unknown};
use ast_grep_core::{Doc, Node};
pub(crate) use eslint::EslintDirectives;
use napi::Result;
use noqa::Noqa;
pub(crate) use noqa::{NoqaIndex, parent_statement};
pub(crate) use nosec::NosecIndex;
use phpcs::Phpcs;
pub(crate) use phpcs::PhpcsIndex;
pub(crate) use phpstan::PhpstanIndex;
use psalm::Psalm;
pub(crate) use psalm::PsalmIndex;
use serde_json::Value;
pub(crate) use vue::{VueDirectives, script_directives as vue_script_directives};

/// Names a `rules` or `codes` list may hold.
const MAX_NAMES: usize = 256;

/// Declares the suppression kinds from the rows below: the `Suppression` enum, the `Suppress` a
/// violation's `suppress` parses to, and `parse`.
macro_rules! suppression_kinds {
    ($($(#[doc = $doc:literal])* $variant:ident $key:ident: $spec:ty = $parse:path, $detail:literal;)+) => {
        /// A linter's suppressions a family may honour (the registry's `suppressions`, each the
        /// PascalCase of its name).
        #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
        pub(crate) enum Suppression {
            $($(#[doc = $doc])* $variant,)+
        }

        impl Suppression {
            /// Every kind, in the table's order: the order `suppress` is parsed and checked in.
            pub(crate) const ALL: &'static [Self] = &[$(Self::$variant,)+];

            /// Its `suppress` key, which is also its name in the registry's `suppressions`.
            pub(crate) const fn key(self) -> &'static str {
                match self {
                    $(Self::$variant => stringify!($key),)+
                }
            }

            /// Why only a family that honours it may ask for it: `key: detail`.
            pub(crate) const fn detail(self) -> &'static str {
                match self {
                    $(Self::$variant => concat!(stringify!($key), ": ", $detail),)+
                }
            }
        }

        /// `suppress` on a violation spec: per kind, what the violation asks for.
        #[derive(Clone, Debug, Default, PartialEq, Eq)]
        pub(super) struct Suppress {
            $($(#[doc = $doc])* pub(super) $key: Option<$spec>,)+
        }

        impl Suppress {
            /// The kinds the violation asks for, in the table's order.
            pub(super) fn requested(&self) -> impl Iterator<Item = Suppression> {
                [$((self.$key.is_some(), Suppression::$variant),)+]
                    .into_iter()
                    .filter_map(|(set, kind)| set.then_some(kind))
            }
        }

        pub(super) fn parse(value: &Value, path: &str) -> Result<Suppress> {
            let suppress = object(value, path)?;
            reject_unknown(suppress, &[$(stringify!($key),)+], path)?;
            // A struct expression evaluates its fields in order: kinds are parsed in table order.
            let parsed = Suppress {
                $($key: suppress
                    .get(stringify!($key))
                    .map(|spec| $parse(spec, &format!("{path}/{}", stringify!($key))))
                    .transpose()?,)+
            };
            if parsed.requested().next().is_none() {
                return Err(invalid(format!("{path} must hold {}", one_of(Suppression::ALL))));
            }
            Ok(parsed)
        }
    };
}

// The suppression kinds, one row each, in the order `suppress` is parsed and checked. A row is
// `Variant key: Spec = linter::spec, "detail";` under the kind's doc comment:
// - `Variant` is the `Suppression` variant, the PascalCase of the registry name, which the
//   generated `Family::honours` names;
// - `key` is the `suppress` key and the registry name (`languages/*.json` `suppressions`);
// - `Spec` is what `suppress.<key>` parses to: the `Suppress` field the family adapter's
//   `Adapter::suppresses` reads;
// - `linter::spec`, `fn(&Value, at: &str) -> napi::Result<Spec>`, parses it, in
//   `suppress/<linter>.rs` beside the reader of the linter's suppressions;
// - `"detail"` says whose they are, for a query of a family that does not honour them.
// Nothing here assumes comments. Clippy's `#[allow(clippy::..)]` attributes would be
//     /// Clippy's `#[allow]`, `#[expect]` attributes.
//     Clippy clippy: Clippy = clippy::spec, "#[allow] attributes are Clippy's";
// and SQLFluff's `-- noqa` comments, a linter other than Ruff, their own kind:
//     /// SQLFluff's `-- noqa` comments.
//     Sqlfluff sqlfluff: Vec<String> = sqlfluff::spec, "-- noqa comments are SQLFluff's";
// The host's twin is `suppressionKinds` in `src/engine/code-rule/code-rule-schema-semantic.ts`.
suppression_kinds! {
    /// ESLint's `eslint-disable` comments: the rule names a comment must name (or name none).
    Eslint eslint: Vec<String> = eslint::spec, "eslint-disable comments are ESLint's";
    /// Ruff's `# noqa` comments.
    Noqa noqa: Noqa = noqa::spec, "noqa comments are Ruff's";
    /// Bandit's `# nosec` comments: the test ids a list must name (or name none).
    Nosec nosec: Vec<String> = nosec::spec, "nosec comments are Bandit's";
    /// PHPStan's `@phpstan-ignore` comments: the identifiers a local ignore must name.
    Phpstan phpstan: Vec<String> = phpstan::spec, "@phpstan-ignore comments are PHPStan's";
    /// Psalm's `@psalm-suppress` tags.
    Psalm psalm: Psalm = psalm::spec, "@psalm-suppress tags are Psalm's";
    /// PHP_CodeSniffer's `phpcs:` annotations.
    Phpcs phpcs: Phpcs = phpcs::spec, "phpcs: annotations are PHP_CodeSniffer's";
}

/// `a, b, or c`: the keys a `suppress` may hold.
fn one_of(kinds: &[Suppression]) -> String {
    let keys = kinds.iter().map(|kind| kind.key()).collect::<Vec<_>>();
    match keys.split_last() {
        Some((last, [])) => (*last).to_owned(),
        Some((last, rest)) => format!("{}, or {last}", rest.join(", ")),
        None => String::new(),
    }
}

/// A non-empty list of at most `MAX_NAMES` strings, each passing `valid`: `what` names the list's
/// items, `each` one item.
fn names(
    value: Option<&Value>,
    path: &str,
    valid: fn(&str) -> bool,
    what: &str,
    each: &str,
) -> Result<Vec<String>> {
    let items = value
        .and_then(Value::as_array)
        .filter(|items| !items.is_empty() && items.len() <= MAX_NAMES)
        .ok_or_else(|| invalid(format!("{path} must be a list of 1 to {MAX_NAMES} {what}")))?;
    items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            item.as_str()
                .filter(|name| valid(name))
                .map(str::to_owned)
                .ok_or_else(|| invalid(format!("{path}/{index} must be {each}")))
        })
        .collect()
}

/// The root of the tree a node belongs to.
pub(crate) fn root<'t, D: Doc>(node: &Node<'t, D>) -> Node<'t, D> {
    let mut root = node.clone();
    while let Some(parent) = root.parent() {
        root = parent;
    }
    root
}

/// The end of PHP's comment token for a tree-sitter-php `comment` node: the grammar's `//` or `#`
/// comment that ends in `?` takes the line break after it, which PHP leaves to white space.
fn php_comment_end(source: &str, range: &std::ops::Range<usize>) -> usize {
    let text = &source[range.clone()];
    if text.starts_with("/*") {
        return range.end;
    }
    let text = text
        .strip_suffix('\n')
        .map_or(text, |text| text.strip_suffix('\r').unwrap_or(text));
    range.start + text.len()
}

mod eslint;
#[cfg(test)]
mod eslint_tests;
mod noqa;
mod nosec;
#[cfg(test)]
mod php_corpus_tests;
mod phpcs;
mod phpstan;
mod psalm;
#[cfg(test)]
mod ruff;
mod vue;

#[cfg(test)]
mod tests {
    use crate::code_rule_dsl::test_support::*;
    use serde_json::{Value, json};

    fn document(language: &str, suppress: Value) -> Value {
        let mut violation = violation();
        violation["suppress"] = suppress;
        let kind = if language == "php" {
            "variable_name"
        } else {
            "identifier"
        };
        clause_document(json!({ "id": "c", "violation": violation,
            "forbid": { "ast": { "language": language, "rule": { "kind": kind } } } }))
    }

    fn compile(document: Value) -> napi::Result<crate::code_rule_dsl::CompiledCodeRuleDsl> {
        compile_document_with(
            document,
            &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
        )
    }

    /// The table's kinds, in order, each named as the registry names it: its key, the PascalCase
    /// of which is its variant (the generated `Family::honours` relies on it).
    #[test]
    fn every_kind_is_one_row_named_as_the_registry_names_it() {
        use super::Suppression;
        let keys = Suppression::ALL
            .iter()
            .map(|kind| kind.key())
            .collect::<Vec<_>>();
        assert_eq!(
            keys,
            ["eslint", "noqa", "nosec", "phpstan", "psalm", "phpcs"]
        );
        for kind in Suppression::ALL {
            let key = kind.key();
            let pascal = key[..1].to_ascii_uppercase() + &key[1..];
            assert_eq!(format!("{kind:?}"), pascal);
            assert!(kind.detail().starts_with(&format!("{key}: ")), "{key}");
        }
    }

    #[test]
    fn suppress_is_checked_and_opt_in_per_linter() {
        let at = "/clauses/0/violation/suppress";
        for (language, suppress, expected) in [
            (
                "typescript",
                json!({}),
                format!("{at} must hold eslint, noqa, nosec, phpstan, psalm, or phpcs"),
            ),
            (
                "typescript",
                json!({ "jshint": {} }),
                format!("unknown field `jshint` at {at}"),
            ),
            (
                "typescript",
                json!({ "eslint": {} }),
                format!("{at}/eslint/rules must be a list of 1 to 256 rule names"),
            ),
            (
                "typescript",
                json!({ "eslint": { "rules": [] } }),
                format!("{at}/eslint/rules must be a list of 1 to 256 rule names"),
            ),
            (
                "typescript",
                json!({ "eslint": { "rules": ["a, b"] } }),
                format!(
                    "{at}/eslint/rules/0 must be a rule name (non-empty, without commas or surrounding white space)"
                ),
            ),
            (
                "typescript",
                json!({ "eslint": { "rules": ["strict"], "all": true } }),
                format!("unknown field `all` at {at}/eslint"),
            ),
            (
                "python",
                json!({ "noqa": { "codes": ["f401"] } }),
                format!(
                    "{at}/noqa/codes/0 must be a rule code (uppercase letters, then digits: F401)"
                ),
            ),
            (
                "python",
                json!({ "noqa": { "codes": ["F"] } }),
                format!(
                    "{at}/noqa/codes/0 must be a rule code (uppercase letters, then digits: F401)"
                ),
            ),
            (
                "python",
                json!({ "noqa": { "codes": ["F401"], "parent": "import" } }),
                format!("{at}/noqa/parent must be \"statement\""),
            ),
            (
                "python",
                json!({ "nosec": { "codes": ["b101"] } }),
                format!("{at}/nosec/codes/0 must be a Bandit test id (B, then digits: B101)"),
            ),
            (
                "python",
                json!({ "nosec": { "codes": ["B101"], "names": ["assert_used"] } }),
                format!("unknown field `names` at {at}/nosec"),
            ),
            (
                "typescript",
                json!({ "nosec": { "codes": ["B101"] } }),
                format!("{at}/nosec: nosec comments are Bandit's; a typescript query has none"),
            ),
            (
                "php",
                json!({ "nosec": { "codes": ["B101"] } }),
                format!("{at}/nosec: nosec comments are Bandit's; a php query has none"),
            ),
            (
                "python",
                json!({ "eslint": { "rules": ["strict"] } }),
                format!(
                    "{at}/eslint: eslint-disable comments are ESLint's; a python query has none"
                ),
            ),
            (
                "typescript",
                json!({ "noqa": { "codes": ["F401"] } }),
                format!("{at}/noqa: noqa comments are Ruff's; a typescript query has none"),
            ),
            (
                "php",
                json!({ "phpstan": { "identifiers": ["echo."] } }),
                format!(
                    "{at}/phpstan/identifiers/0 must be a PHPStan identifier (letters or digits, with internal dots)"
                ),
            ),
            (
                "php",
                json!({ "eslint": { "rules": ["strict"] } }),
                format!("{at}/eslint: eslint-disable comments are ESLint's; a php query has none"),
            ),
            (
                "php",
                json!({ "noqa": { "codes": ["F401"] } }),
                format!("{at}/noqa: noqa comments are Ruff's; a php query has none"),
            ),
            (
                "python",
                json!({ "phpstan": { "identifiers": ["variable.undefined"] } }),
                format!(
                    "{at}/phpstan: @phpstan-ignore comments are PHPStan's; a python query has none"
                ),
            ),
            (
                "typescript",
                json!({ "phpstan": { "identifiers": ["variable.undefined"] } }),
                format!(
                    "{at}/phpstan: @phpstan-ignore comments are PHPStan's; a typescript query has none"
                ),
            ),
            (
                "php",
                json!({ "psalm": { "issues": ["all"] } }),
                format!(
                    "{at}/psalm/issues/0 must be a Psalm issue name (letters, digits, `_` or `-`; `all` is suppressAll)"
                ),
            ),
            (
                "php",
                json!({ "psalm": { "issues": ["UndefinedVariable"], "suppressAll": "yes" } }),
                format!("{at}/psalm/suppressAll must be a boolean"),
            ),
            (
                "php",
                json!({ "phpcs": { "sniffs": ["Squiz.PHP"] } }),
                format!(
                    "{at}/phpcs/sniffs/0 must be a PHP_CodeSniffer sniff or message code (Standard.Category.Sniff or Standard.Category.Sniff.Code)"
                ),
            ),
            (
                "php",
                json!({ "phpcs": { "sniffs": ["Squiz.PHP.DiscouragedFunctions"], "version": 2 } }),
                format!("{at}/phpcs/version must be 3 or 4"),
            ),
            (
                "python",
                json!({ "psalm": { "issues": ["UndefinedVariable"] } }),
                format!("{at}/psalm: @psalm-suppress tags are Psalm's; a python query has none"),
            ),
            (
                "typescript",
                json!({ "phpcs": { "sniffs": ["Squiz.PHP.DiscouragedFunctions"] } }),
                format!(
                    "{at}/phpcs: phpcs: annotations are PHP_CodeSniffer's; a typescript query has none"
                ),
            ),
        ] {
            assert_eq!(
                rejection(compile(document(language, suppress.clone()))),
                expected,
                "{suppress}"
            );
        }
        let mut regex = document("typescript", json!({ "eslint": { "rules": ["strict"] } }));
        regex["clauses"][0]["forbid"] =
            json!({ "regex": { "source": "file.content", "pattern": "x" } });
        assert_eq!(
            rejection(compile(regex)),
            format!("{at}: suppress removes an ast query's violations")
        );
        for (language, suppress) in [
            (
                "typescript",
                json!({ "eslint": { "rules": ["strict", "rule-to-test/strict"] } }),
            ),
            (
                "python",
                json!({ "noqa": { "codes": ["F401", "TC002", "TCH002"], "parent": "statement" } }),
            ),
            (
                "python",
                json!({ "noqa": { "codes": ["S101"] }, "nosec": { "codes": ["B101"] } }),
            ),
            (
                "php",
                json!({ "phpstan": { "identifiers": ["variable.undefined", "echo.nonString"] } }),
            ),
            (
                "php",
                json!({
                    "phpstan": { "identifiers": ["variable.undefined"] },
                    "psalm": { "issues": ["UndefinedVariable", "PossiblyUndefinedVariable"], "suppressAll": true },
                    "phpcs": { "sniffs": ["Squiz.PHP.DiscouragedFunctions.Discouraged"], "version": 3 }
                }),
            ),
        ] {
            compile(document(language, suppress))
                .unwrap_or_else(|error| panic!("{}", error.reason));
        }
    }
}
