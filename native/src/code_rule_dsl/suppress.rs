//! `suppress` (a Rerail extension, opt-in per violation): another linter's suppression comments,
//! honoured as that linter honours them. The engine never reads them otherwise.

use super::json::{invalid, object, reject_unknown};
use super::lines::LineTable;
use super::literal::eslint_directive::Directive;
use ast_grep_core::{Doc, Node};
use napi::Result;
pub(crate) use noqa::{NoqaIndex, parent_statement};
pub(crate) use phpcs::{PhpcsIndex, PhpcsVersion};
pub(crate) use phpstan::PhpstanIndex;
pub(crate) use psalm::PsalmIndex;
use serde_json::Value;

/// Names a `rules` or `codes` list may hold.
const MAX_NAMES: usize = 256;

/// `suppress` on a violation spec.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Suppress {
    /// `eslint.rules`: the rule names an `eslint-disable` comment must name (or name none) to
    /// suppress the violation.
    pub(super) eslint: Option<Vec<String>>,
    pub(super) noqa: Option<Noqa>,
    /// `phpstan.identifiers`: PHPStan diagnostic identifiers a local ignore must name.
    pub(super) phpstan: Option<Vec<String>>,
    pub(super) psalm: Option<Psalm>,
    pub(super) phpcs: Option<Phpcs>,
}

/// `psalm`: Psalm's `@psalm-suppress` tags.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Psalm {
    /// The issues the violation is, and their `Config::getParentIssueType` parents: a tag naming
    /// any of them suppresses it.
    pub(super) names: Vec<String>,
    /// `suppressAll: true`: `@psalm-suppress all` counts, as with `disableSuppressAll="false"`.
    pub(super) suppress_all: bool,
}

/// `phpcs`: PHP_CodeSniffer's `phpcs:` annotations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Phpcs {
    /// The sniff or message codes the violation reports as.
    pub(super) codes: Vec<String>,
    /// `version`: the PHP_CodeSniffer major whose reading applies (4 unless 3).
    pub(super) version: PhpcsVersion,
}

/// `noqa`: Ruff's `# noqa` and file-level `# ruff: noqa` comments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Noqa {
    /// The codes a directive must name (or name none) to suppress the violation.
    pub(super) codes: Vec<String>,
    /// `parent: "statement"`: a directive where the enclosing statement starts also suppresses
    /// it (Ruff's `set_parent`).
    pub(super) parent: bool,
}

/// A non-empty list of at most `MAX_NAMES` strings, each passing `valid`.
fn names(
    value: Option<&Value>,
    path: &str,
    valid: fn(&str) -> bool,
    what: &str,
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
                .ok_or_else(|| invalid(format!("{path}/{index} must be {}", singular(what))))
        })
        .collect()
}

fn singular(what: &str) -> &str {
    match what {
        "rule names" => "a rule name (non-empty, without commas or surrounding white space)",
        "PHPStan identifiers" => "a PHPStan identifier (letters or digits, with internal dots)",
        "Psalm issue names" => {
            "a Psalm issue name (letters, digits, `_` or `-`; `all` is suppressAll)"
        }
        "PHP_CodeSniffer codes" => {
            "a PHP_CodeSniffer sniff or message code (Standard.Category.Sniff or Standard.Category.Sniff.Code)"
        }
        _ => "a rule code (uppercase letters, then digits: F401)",
    }
}

/// A rule name an `eslint-disable` list can name: ESLint splits the list on commas and trims.
fn rule_name(name: &str) -> bool {
    !name.is_empty() && !name.contains(',') && name.trim() == name
}

/// A code a `# noqa` list can name: Ruff's lexer reads uppercase letters, then digits.
fn rule_code(code: &str) -> bool {
    let letters = code.trim_start_matches(|c: char| c.is_ascii_uppercase());
    letters.len() < code.len() && !letters.is_empty() && letters.chars().all(|c| c.is_ascii_digit())
}

/// A PHPStan diagnostic identifier (`variable.undefined`, for example).
fn phpstan_identifier(identifier: &str) -> bool {
    phpstan::phpstan_identifier(identifier)
}

/// An issue name a `@psalm-suppress` list can hold (`all` aside, which `suppressAll` names).
fn psalm_issue(issue: &str) -> bool {
    psalm::psalm_issue(issue) && issue != "all"
}

/// A sniff (`Standard.Category.Sniff`) or message (`Standard.Category.Sniff.Code`) code: parts of
/// letters, digits and `_`, as PHP_CodeSniffer names standards, sniffs and codes.
fn phpcs_code(code: &str) -> bool {
    let parts = code.split('.');
    (3..=4).contains(&parts.clone().count())
        && parts.into_iter().all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        })
}

pub(super) fn parse(value: &Value, path: &str) -> Result<Suppress> {
    let suppress = object(value, path)?;
    reject_unknown(
        suppress,
        &["eslint", "noqa", "phpstan", "psalm", "phpcs"],
        path,
    )?;
    let eslint = suppress
        .get("eslint")
        .map(|eslint| {
            let at = format!("{path}/eslint");
            reject_unknown(object(eslint, &at)?, &["rules"], &at)?;
            names(
                eslint.get("rules"),
                &format!("{at}/rules"),
                rule_name,
                "rule names",
            )
        })
        .transpose()?;
    let noqa = suppress
        .get("noqa")
        .map(|noqa| {
            let at = format!("{path}/noqa");
            reject_unknown(object(noqa, &at)?, &["codes", "parent"], &at)?;
            let codes = names(
                noqa.get("codes"),
                &format!("{at}/codes"),
                rule_code,
                "rule codes",
            )?;
            let parent = match noqa.get("parent") {
                None => false,
                Some(Value::String(parent)) if parent == "statement" => true,
                Some(_) => {
                    return Err(invalid(format!("{at}/parent must be \"statement\"")));
                }
            };
            Ok(Noqa { codes, parent })
        })
        .transpose()?;
    let phpstan = suppress
        .get("phpstan")
        .map(|phpstan| {
            let at = format!("{path}/phpstan");
            reject_unknown(object(phpstan, &at)?, &["identifiers"], &at)?;
            names(
                phpstan.get("identifiers"),
                &format!("{at}/identifiers"),
                phpstan_identifier,
                "PHPStan identifiers",
            )
        })
        .transpose()?;
    let psalm = suppress
        .get("psalm")
        .map(|psalm| {
            let at = format!("{path}/psalm");
            reject_unknown(object(psalm, &at)?, &["issues", "suppressAll"], &at)?;
            let issues = names(
                psalm.get("issues"),
                &format!("{at}/issues"),
                psalm_issue,
                "Psalm issue names",
            )?;
            let suppress_all = match psalm.get("suppressAll") {
                None => false,
                Some(Value::Bool(all)) => *all,
                Some(_) => {
                    return Err(invalid(format!("{at}/suppressAll must be a boolean")));
                }
            };
            let mut names = issues.clone();
            for parent in issues
                .iter()
                .filter_map(|issue| psalm::parent_issue_type(issue))
            {
                if !names.contains(&parent) {
                    names.push(parent);
                }
            }
            Ok(Psalm {
                names,
                suppress_all,
            })
        })
        .transpose()?;
    let phpcs = suppress
        .get("phpcs")
        .map(|phpcs| {
            let at = format!("{path}/phpcs");
            reject_unknown(object(phpcs, &at)?, &["sniffs", "version"], &at)?;
            let codes = names(
                phpcs.get("sniffs"),
                &format!("{at}/sniffs"),
                phpcs_code,
                "PHP_CodeSniffer codes",
            )?;
            let version = match phpcs.get("version").map(Value::as_u64) {
                None | Some(Some(4)) => PhpcsVersion::Four,
                Some(Some(3)) => PhpcsVersion::Three,
                Some(_) => return Err(invalid(format!("{at}/version must be 3 or 4"))),
            };
            Ok(Phpcs { codes, version })
        })
        .transpose()?;
    if eslint.is_none() && noqa.is_none() && phpstan.is_none() && psalm.is_none() && phpcs.is_none()
    {
        return Err(invalid(format!(
            "{path} must hold eslint, noqa, phpstan, psalm, or phpcs"
        )));
    }
    Ok(Suppress {
        eslint,
        noqa,
        phpstan,
        psalm,
        phpcs,
    })
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

/// A block directive: `eslint-disable` or `eslint-enable` where its comment starts.
#[derive(Debug)]
struct Block {
    start: usize,
    disable: bool,
    /// The rule ids it names; none names every rule.
    rules: Vec<String>,
}

/// A file's ESLint directives that disable or enable rules, as ESLint 10.11.0's
/// `getDisableDirectives` and `applyDisableDirectives` read them.
#[derive(Debug, Default)]
pub(crate) struct EslintDirectives {
    /// Block directives, in source order.
    blocks: Vec<Block>,
    /// Line directives: the line each applies to, and the rule ids it names.
    lines: Vec<(usize, Vec<String>)>,
}

impl EslintDirectives {
    /// Reads every comment of the file `node` belongs to.
    pub(crate) fn read<D: Doc>(node: &Node<'_, D>, source: &str, lines: &LineTable) -> Self {
        let mut directives = Self::default();
        for comment in root(node).dfs().filter(|node| node.kind() == "comment") {
            let range = comment.range();
            let text = &source[range.clone()];
            let Some(directive) = Directive::parse(text).filter(Directive::disables) else {
                continue;
            };
            let rules = directive
                .items(text)
                .into_iter()
                .map(|item| item.name)
                .collect::<Vec<_>>();
            let (first, last) = (lines.line(range.start), lines.line(range.end));
            match directive.label.as_str() {
                "eslint-disable" | "eslint-enable" => directives.blocks.push(Block {
                    start: range.start,
                    disable: directive.label == "eslint-disable",
                    rules,
                }),
                // A `-line` directive over several lines is a problem, not a directive.
                "eslint-disable-line" if first == last => directives.lines.push((first, rules)),
                "eslint-disable-next-line" => directives.lines.push((last + 1, rules)),
                _ => {}
            }
        }
        directives
    }

    /// Whether a problem of a rule named `rules` (any of its names) starting at byte `at` is
    /// suppressed: the last block directive at or before it that names the rule (or none) is a
    /// disable, or a line directive naming it applies to its line.
    pub(crate) fn suppresses(&self, rules: &[String], at: usize, lines: &LineTable) -> bool {
        let names = |ids: &[String]| ids.is_empty() || ids.iter().any(|id| rules.contains(id));
        if self
            .blocks
            .iter()
            .rev()
            .find(|block| block.start <= at && names(&block.rules))
            .is_some_and(|block| block.disable)
        {
            return true;
        }
        let line = lines.line(at);
        self.lines
            .iter()
            .any(|(target, ids)| *target == line && names(ids))
    }
}

#[cfg(test)]
mod eslint;
mod noqa;
#[cfg(test)]
mod php_corpus_tests;
mod phpcs;
mod phpstan;
mod psalm;
#[cfg(test)]
mod ruff;

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

    #[test]
    fn suppress_is_checked_and_opt_in_per_linter() {
        let at = "/clauses/0/violation/suppress";
        for (language, suppress, expected) in [
            (
                "typescript",
                json!({}),
                format!("{at} must hold eslint, noqa, phpstan, psalm, or phpcs"),
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
