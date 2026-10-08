//! Psalm 6.19.1's `@psalm-suppress` tags.
//!
//! Psalm reads the tag only in doc comments (`/**` then white space, PHP's `T_DOC_COMMENT`),
//! from the comments PHP-Parser attaches to the node that starts at the next token. Two readers
//! apply them:
//! - `StatementsAnalyzer::analyzeStatement`, for each statement of top-level code, a
//!   function-like body or a control structure's body (functions declared as statements
//!   included): every doc comment before the statement, over the statement. Afterwards it removes
//!   only the last suppressing doc comment's issues, so an earlier one stays in force to the end of
//!   the enclosing function-like, or, at the top level, to the end of the file and in every
//!   class-like (Psalm analyses those after the top-level code).
//! - Declaration storage (functions, methods, closures, arrow functions, class-likes,
//!   properties, class constants): the last doc comment before the declaration, over the
//!   declaration (a class's covers its members).
//!
//! An issue is suppressed by its own name, by its `Config::getParentIssueType` parent, or by
//! `all` where the configuration sets `disableSuppressAll="false"`.

use ast_grep_core::{Doc, Node};
use regex::Regex;
use std::ops::Range;
use std::sync::LazyLock;

/// Statement lists: each child (but a `case` value or a `declare` directive) is a statement
/// `analyzeStatement` reads.
const STATEMENT_LISTS: &[&str] = &[
    "program",
    "compound_statement",
    "colon_block",
    "case_statement",
    "default_statement",
    "declare_statement",
];

/// Control structures whose unbraced `body` is a statement of its own.
const CONTROL: &[&str] = &[
    "if_statement",
    "else_clause",
    "else_if_clause",
    "while_statement",
    "for_statement",
    "foreach_statement",
    "do_statement",
];

/// Function-likes that own the statement list a leaked suppression stays in.
const FUNCTION_LIKES: &[&str] = &[
    "function_definition",
    "method_declaration",
    "anonymous_function",
];

/// Class-likes Psalm reads from storage and analyses after a file's top-level statements.
const CLASS_LIKES: &[&str] = &[
    "class_declaration",
    "interface_declaration",
    "enum_declaration",
];

#[derive(Debug, Default)]
pub(crate) struct PsalmIndex {
    /// Each suppressing doc comment: the spans it covers and the issue names it lists.
    directives: Vec<(Vec<Range<usize>>, Vec<String>)>,
}

impl PsalmIndex {
    pub(crate) fn read<D: Doc>(root: &Node<'_, D>, source: &str) -> Self {
        let mut index = Self::default();
        let mut top_level: Option<TopLevel> = None;
        for comment in root.dfs().filter(|node| node.kind() == "comment") {
            let text = &source[comment.range()];
            if !doc_comment(text) {
                continue;
            }
            let Some(issues) = suppressions(text).filter(|issues| !issues.is_empty()) else {
                continue;
            };
            // The node PHP-Parser attaches the comment to, and whether a later doc comment (or a
            // later suppressing one) stands between them.
            let (mut later_doc, mut later_suppressing) = (false, false);
            let mut target = comment.next();
            while let Some(node) = target.as_ref().filter(|node| node.kind() == "comment") {
                let text = &source[node.range()];
                if doc_comment(text) {
                    later_doc = true;
                    later_suppressing |= suppressions(text).is_some();
                }
                target = node.next();
            }
            let (Some(target), Some(parent)) = (target, comment.parent()) else {
                continue;
            };
            let kind = target.kind();
            let parent_kind = parent.kind();
            let span = target.range();
            let declaration = matches!(
                &*kind,
                "function_definition"
                    | "method_declaration"
                    | "property_declaration"
                    | "const_declaration"
                    | "anonymous_function"
                    | "arrow_function"
            ) || CLASS_LIKES.contains(&&*kind);
            // A file's class-likes are not top-level statements; a trait is, but Psalm analyses
            // its methods apart from it.
            let statement = !CLASS_LIKES.contains(&&*kind)
                && if STATEMENT_LISTS.contains(&&*parent_kind) {
                    kind != "declare_directive"
                        && parent
                            .field("value")
                            .is_none_or(|value| value.range() != span)
                } else {
                    CONTROL.contains(&&*parent_kind)
                        && !matches!(&*kind, "compound_statement" | "colon_block")
                        && parent
                            .field("body")
                            .is_some_and(|body| body.range() == span)
                };
            let mut spans = Vec::new();
            if statement {
                if !later_suppressing {
                    if kind != "trait_declaration" {
                        spans.push(span.clone());
                    }
                } else if let Some(function) = target
                    .ancestors()
                    .find(|node| FUNCTION_LIKES.contains(&&*node.kind()))
                {
                    spans.push(span.start..function.range().end);
                } else {
                    let top_level = top_level.get_or_insert_with(|| TopLevel::read(root));
                    // The rest of the file but its traits, and every class-like.
                    let mut at = span.start;
                    for later in &top_level.traits {
                        if later.end > at {
                            spans.push(at..later.start.max(at));
                            at = later.end;
                        }
                    }
                    spans.push(at..source.len());
                    spans.extend(top_level.class_likes.iter().cloned());
                }
            }
            // Declaration storage reads the last doc comment only.
            if declaration && !later_doc {
                spans.push(span);
            }
            if !spans.is_empty() {
                index.directives.push((spans, issues));
            }
        }
        index
    }

    /// Whether an issue named one of `names` (its names and their parents) starting at byte `at`
    /// is suppressed; `all` counts where `suppress_all`.
    pub(crate) fn suppresses(&self, names: &[String], suppress_all: bool, at: usize) -> bool {
        self.directives.iter().any(|(spans, issues)| {
            spans.iter().any(|span| span.contains(&at))
                && issues
                    .iter()
                    .any(|issue| (suppress_all && issue == "all") || names.contains(issue))
        })
    }

    /// Whether the file holds no tag Psalm would apply.
    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.directives.is_empty()
    }
}

/// A file's class-likes and traits outside every function-like: Psalm analyses the class-likes
/// after the top-level code, and trait methods apart from the file's top-level suppressions.
struct TopLevel {
    class_likes: Vec<Range<usize>>,
    traits: Vec<Range<usize>>,
}

impl TopLevel {
    fn read<D: Doc>(root: &Node<'_, D>) -> Self {
        let mut top_level = Self {
            class_likes: Vec::new(),
            traits: Vec::new(),
        };
        for node in root.dfs() {
            let kind = node.kind();
            let list = if CLASS_LIKES.contains(&&*kind) {
                &mut top_level.class_likes
            } else if kind == "trait_declaration" {
                &mut top_level.traits
            } else {
                continue;
            };
            if !node
                .ancestors()
                .any(|ancestor| FUNCTION_LIKES.contains(&&*ancestor.kind()))
            {
                list.push(node.range());
            }
        }
        top_level
    }
}

/// PHP's `T_DOC_COMMENT`: `/**` followed by white space.
pub(super) fn doc_comment(text: &str) -> bool {
    text.strip_prefix("/**")
        .is_some_and(|rest| rest.starts_with([' ', '\t', '\n', '\r']))
}

/// PCRE's `\s` without the UTF flag.
pub(super) fn pcre_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\x0B' | '\x0C' | '\r')
}

/// PHP's `trim` character list.
const PHP_TRIM: &[char] = &[' ', '\t', '\n', '\r', '\0', '\x0B'];

/// The issues a doc comment's `@psalm-suppress` tags list, as `DocblockParser::parse` and
/// `DocComment::parseSuppressList` read them; `None` without such a tag.
fn suppressions(doc: &str) -> Option<Vec<String>> {
    static STAR_LINE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?m)^ *\*[\t\n\x0B\x0C\r ]*$").expect("star line"));
    let mut doc = doc.trim_matches(PHP_TRIM);
    doc = doc.strip_prefix("/**").unwrap_or(doc);
    if let Some(rest) = doc.strip_suffix("*/") {
        doc = rest.strip_suffix('*').unwrap_or(rest);
    }
    let doc = doc.replace('\t', " ");
    let has_r = doc.contains('\r');
    // Lines that do not start a tag continue the tag above them until a blank line.
    let mut lines: Vec<String> = Vec::new();
    let mut last: Option<usize> = None;
    for line in doc.split('\n') {
        if line.contains('@') && tag_start(line) {
            last = Some(lines.len());
            lines.push(line.to_owned());
        } else if line.trim_matches(PHP_TRIM).is_empty() {
            last = None;
            lines.push(line.to_owned());
        } else if let Some(last) = last {
            lines[last].push('\n');
            lines[last].push_str(line);
        } else {
            lines.push(line.to_owned());
        }
    }
    let mut found = None;
    for mut line in lines {
        if has_r {
            line.retain(|c| c != '\r');
        }
        let Some((name, data)) = tag(&line) else {
            continue;
        };
        if name != "psalm-suppress" {
            continue;
        }
        let data = if data.contains('*') {
            STAR_LINE
                .replace_all(data, "")
                .trim_end_matches(PHP_TRIM)
                .to_owned()
        } else {
            data.to_owned()
        };
        found
            .get_or_insert_with(Vec::new)
            .extend(suppress_list(&data));
    }
    found
}

/// `^ *\*?\s*@\w`: a line that starts a tag.
fn tag_start(line: &str) -> bool {
    let rest = line.trim_start_matches(' ');
    let rest = rest.strip_prefix('*').unwrap_or(rest);
    let rest = rest.trim_start_matches(pcre_space);
    rest.strip_prefix('@')
        .is_some_and(|rest| rest.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_'))
}

/// `^ *\*?\s*@([\w\-\\:]+) *(.*)$`: a tag's name and the text after it.
fn tag(line: &str) -> Option<(&str, &str)> {
    let rest = line.trim_start_matches(' ');
    let rest = rest.strip_prefix('*').unwrap_or(rest);
    let rest = rest.trim_start_matches(pcre_space).strip_prefix('@')?;
    let end = rest
        .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '\\' | ':')))
        .unwrap_or(rest.len());
    (end > 0).then(|| (&rest[..end], rest[end..].trim_start_matches(' ')))
}

/// `DocComment::parseSuppressList`: from the first line of `entry` that starts with an issue
/// name, the comma-separated names there (white space, line breaks included, around commas).
fn suppress_list(entry: &str) -> Vec<String> {
    let issue_char = |c: char| c.is_ascii_alphanumeric() || matches!(c, '_' | '-');
    let start = std::iter::once(0)
        .chain(entry.match_indices('\n').map(|(at, _)| at + 1))
        .find(|at| entry[*at..].starts_with(issue_char));
    let Some(start) = start else {
        return Vec::new();
    };
    let mut rest = &entry[start..];
    let mut issues = Vec::new();
    loop {
        let end = rest.find(|c: char| !issue_char(c)).unwrap_or(rest.len());
        issues.push(rest[..end].to_owned());
        let after = rest[end..].trim_start_matches(pcre_space);
        let Some(after) = after.strip_prefix(',') else {
            return issues;
        };
        let after = after.trim_start_matches(pcre_space);
        if !after.starts_with(issue_char) {
            return issues;
        }
        rest = after;
    }
}

/// `Config::getParentIssueType`: the issue whose suppression also suppresses `issue`.
pub(crate) fn parent_issue_type(issue: &str) -> Option<String> {
    match issue {
        "PossiblyUndefinedIntArrayOffset" | "PossiblyUndefinedStringArrayOffset" => {
            return Some("PossiblyUndefinedArrayOffset".to_owned());
        }
        "PossiblyNullReference" => return Some("NullReference".to_owned()),
        "PossiblyFalseReference" | "PossiblyUndefinedArrayOffset" => return None,
        _ => {}
    }
    if let Some(rest) = issue.strip_prefix("Possibly") {
        let stripped = rest
            .strip_prefix("False")
            .or_else(|| rest.strip_prefix("Null"))
            .unwrap_or(rest);
        return Some(
            if !stripped.contains("Invalid") && !stripped.starts_with("Un") {
                format!("Invalid{stripped}")
            } else {
                stripped.to_owned()
            },
        );
    }
    if issue.starts_with("Tainted") {
        return Some("TaintedInput".to_owned());
    }
    if let Some(rest) = issue
        .strip_prefix("False")
        .or_else(|| issue.strip_prefix("Null"))
        && rest.starts_with(|c: char| c.is_ascii_uppercase())
        && !issue.contains("Reference")
    {
        return Some(format!("Invalid{rest}"));
    }
    let parent = match issue {
        "UndefinedInterfaceMethod" | "UndefinedMagicMethod" => "UndefinedMethod",
        "UndefinedMagicPropertyFetch" => "UndefinedPropertyFetch",
        "UndefinedMagicPropertyAssignment" => "UndefinedPropertyAssignment",
        "PossibleRawObjectIteration" => "RawObjectIteration",
        "UninitializedProperty" => "PropertyNotSetInConstructor",
        "InvalidDocblockParamName" => "InvalidDocblock",
        "UnusedClosureParam" => "UnusedParam",
        "UnusedConstructor" => "UnusedMethod",
        "StringIncrement" => "InvalidOperand",
        "InvalidLiteralArgument" => "InvalidArgument",
        "RedundantConditionGivenDocblockType" => "RedundantCondition",
        "RedundantFunctionCallGivenDocblockType" => "RedundantFunctionCall",
        "RedundantCastGivenDocblockType" => "RedundantCast",
        "TraitMethodSignatureMismatch" => "MethodSignatureMismatch",
        "ImplementedParamTypeMismatch" => "MoreSpecificImplementedParamType",
        "UndefinedDocblockClass" => "UndefinedClass",
        "UnusedForeachValue" => "UnusedVariable",
        _ => return None,
    };
    Some(parent.to_owned())
}

/// A Psalm issue name a `@psalm-suppress` list can hold.
pub(crate) fn psalm_issue(issue: &str) -> bool {
    !issue.is_empty()
        && issue
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

#[cfg(test)]
mod tests {
    use super::{PsalmIndex, parent_issue_type};
    use crate::code_rule_dsl::language::GrammarDialect;
    use crate::code_rule_dsl::oracle;
    use ast_grep_core::tree_sitter::LanguageExt;
    use serde_json::Value;

    const ORACLE: &str = oracle!("php/suppress/psalm.expected.json");

    fn reports(file: &Value, key: &str) -> Vec<(String, usize)> {
        let mut reports = file[key]
            .as_array()
            .expect("reports")
            .iter()
            .map(|report| {
                (
                    report[0].as_str().expect("issue").to_owned(),
                    report[1].as_u64().expect("start") as usize,
                )
            })
            .collect::<Vec<_>>();
        reports.sort();
        reports
    }

    #[test]
    fn psalm_suppress_tags_suppress_as_psalm_honours_them() {
        let oracle: Value = serde_json::from_str(ORACLE).expect("oracle JSON");
        assert!(
            oracle["psalm"]
                .as_str()
                .is_some_and(|version| version.starts_with("Psalm 6.19.1@"))
        );
        for (name, source) in [
            (
                "statements.php",
                oracle!("php/suppress/psalm/statements.php"),
            ),
            (
                "declarations.php",
                oracle!("php/suppress/psalm/declarations.php"),
            ),
            ("leaks.php", oracle!("php/suppress/psalm/leaks.php")),
        ] {
            let root = GrammarDialect::Php.ast_grep(source);
            let index = PsalmIndex::read(&root.root(), source);
            let file = &oracle[name];
            let all = reports(file, "all");
            for (key, suppress_all) in [("kept", false), ("keptSuppressAll", true)] {
                let ours = all
                    .iter()
                    .filter(|(issue, at)| {
                        let names = [Some(issue.clone()), parent_issue_type(issue)]
                            .into_iter()
                            .flatten()
                            .collect::<Vec<_>>();
                        !index.suppresses(&names, suppress_all, *at)
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                assert_eq!(ours, reports(file, key), "{name} {key}");
            }
        }
        let parents = oracle["parents"].as_object().expect("parents");
        assert!(parents.len() > 300);
        for (issue, parent) in parents {
            assert_eq!(
                parent_issue_type(issue).as_deref(),
                parent.as_str(),
                "{issue}"
            );
        }
    }
}
