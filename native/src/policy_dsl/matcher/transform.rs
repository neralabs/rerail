//! An ast query's `transform` (ast-grep-config 0.45.3 `transform/`): text computed from a capture
//! after the rule and its constraints matched, as upstream's `RuleCore` does, and read by messages,
//! fixes and text comparisons. `rewrite` is not supported. `line`, `lines` and `count` (Rerail's)
//! read where a capture stands and how many nodes it holds.

mod semantic;
mod string_case;

use super::exported::exported_span;
use super::host_lines;
use super::value::Reading;
use crate::policy_dsl::lines::LineTable;
use ast_grep_core::meta_var::{MetaVarEnv, MetaVariable};
use ast_grep_core::source::Content;
use ast_grep_core::{Doc, Node};
use regex::Regex;
use semantic::value_text;
#[cfg(test)]
use std::cell::Cell;
use std::collections::BTreeSet;
use std::ops::Range;
pub(crate) use string_case::{Separator, StringCase};

#[cfg(test)]
thread_local! {
    static LINE_TABLE_CONSTRUCTIONS: Cell<usize> = const { Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn reset_line_table_constructions() {
    LINE_TABLE_CONSTRUCTIONS.with(|count| count.set(0));
}

#[cfg(test)]
pub(crate) fn note_line_table_construction() {
    LINE_TABLE_CONSTRUCTIONS.with(|count| count.set(count.get() + 1));
}

#[cfg(test)]
pub(crate) fn line_table_constructions() -> usize {
    LINE_TABLE_CONSTRUCTIONS.with(Cell::get)
}

/// What a transformation reads: a capture or another transform variable, and (Rerail extensions)
/// the text it reads as where it is unbound (`default`) and whether it reads the capture's value
/// rather than its source text (`value`, with how values are read).
#[derive(Clone)]
pub(crate) struct Source {
    pub(crate) variable: MetaVariable,
    pub(crate) default: Option<String>,
    pub(crate) value: Option<Reading>,
}

/// A `value` source whose value is not known and that has no default.
pub(crate) struct UnknownValue;

pub(crate) struct TransformLimit;

/// Why a transformation computed no text: its source's value is not known, or its output exceeds
/// the budget.
enum ComputeError {
    Unknown,
    Limit,
}

impl From<TransformLimit> for ComputeError {
    fn from(TransformLimit: TransformLimit) -> Self {
        Self::Limit
    }
}

impl Source {
    /// The source's text: the capture's (a `$$$` capture's from its first node to its last), its
    /// value's (`value`), or the transform variable's; else the default; else `Ok(None)`
    /// (upstream's unbound source). A value that is not known reads as unbound where there is a
    /// default, else it is `UnknownValue`.
    fn text<D: Doc>(&self, env: &MetaVarEnv<'_, D>) -> Result<Option<String>, UnknownValue> {
        let text = match &self.value {
            Some(reading) => match &self.variable {
                MetaVariable::Capture(name, _) => match env.get_match(name) {
                    Some(node) => match value_text(reading, node) {
                        Some(text) => Some(text),
                        None if self.default.is_none() => return Err(UnknownValue),
                        None => None,
                    },
                    None => None,
                },
                _ => None,
            },
            None => env
                .get_var_bytes(&self.variable)
                .map(|bytes| <D::Source as Content>::encode_bytes(bytes).into_owned()),
        };
        Ok(text.or_else(|| self.default.clone()))
    }
}

#[derive(Clone)]
pub(crate) enum Trans {
    /// Characters (not bytes) from `start` to `end`; a negative index counts from the end.
    Substring {
        source: Source,
        start: Option<i32>,
        end: Option<i32>,
    },
    /// Every match of the regex replaced by `by` (`$1`, `${name}` expand), as `Regex::replace_all`.
    Replace {
        source: Source,
        regex: Regex,
        by: String,
    },
    Convert {
        source: Source,
        case: StringCase,
        separators: Option<Vec<Separator>>,
    },
    /// The 1-based line of the capture's start (or, with `end`, its end), with the terminators of
    /// the linter a port follows.
    Line { source: Source, end: bool },
    /// The number of lines the capture spans.
    Lines { source: Source },
    /// The number of nodes the capture holds: a `$$$` capture's named nodes other than comments
    /// (tree-sitter's extras, which the capture holds as upstream's does), 1 for a single one.
    Count { source: Source },
}

impl Trans {
    fn source_spec(&self) -> &Source {
        match self {
            Self::Substring { source, .. }
            | Self::Replace { source, .. }
            | Self::Convert { source, .. }
            | Self::Line { source, .. }
            | Self::Lines { source }
            | Self::Count { source } => source,
        }
    }

    pub(crate) fn source(&self) -> &MetaVariable {
        &self.source_spec().variable
    }

    /// Whether the transformation reads its source's value (`value`).
    pub(crate) fn reads_value(&self) -> bool {
        self.source_spec().value.is_some()
    }

    /// The source's name, `$` sigils aside.
    pub(crate) fn source_name(&self) -> &str {
        match self.source() {
            MetaVariable::Capture(name, _) | MetaVariable::MultiCapture(name) => name,
            MetaVariable::Dropped(_) | MetaVariable::Multiple => "",
        }
    }

    /// Whether the transformation reads the capture's node rather than its text.
    pub(crate) fn reads_node(&self) -> bool {
        matches!(
            self,
            Self::Line { .. } | Self::Lines { .. } | Self::Count { .. }
        )
    }

    /// Upstream's `compute`: `Ok(None)` where the source is unbound and has no default. `line`,
    /// `lines` and `count` read the capture's node; an unbound one reads as the default.
    fn compute<D: Doc>(
        &self,
        env: &MetaVarEnv<'_, D>,
        max_output_bytes: usize,
    ) -> Result<Option<String>, ComputeError> {
        let or_default =
            |source: &Source, text: Option<String>| text.or_else(|| source.default.clone());
        let output = match self {
            Self::Line { source, end } => {
                let text = placed(env, &source.variable).map(|(range, lines)| {
                    let at = if *end { range.end } else { range.start };
                    (lines.line(at) + 1).to_string()
                });
                or_default(source, text)
            }
            Self::Lines { source } => {
                let text = placed(env, &source.variable).map(|(range, lines)| {
                    (lines.line(range.end) - lines.line(range.start) + 1).to_string()
                });
                or_default(source, text)
            }
            Self::Count { source } => {
                let text = match &source.variable {
                    MetaVariable::MultiCapture(name) => Some(
                        env.get_multiple_matches(name)
                            .iter()
                            .filter(|node| node.is_named() && !node.is_extra())
                            .count()
                            .to_string(),
                    ),
                    MetaVariable::Capture(name, _) => (env.get_match(name).is_some()
                        || exported_span(env, name).is_some())
                    .then(|| "1".to_owned()),
                    MetaVariable::Dropped(_) | MetaVariable::Multiple => None,
                };
                or_default(source, text)
            }
            Self::Substring { .. } | Self::Replace { .. } | Self::Convert { .. } => {
                let Some(text) = self
                    .source_spec()
                    .text(env)
                    .map_err(|UnknownValue| ComputeError::Unknown)?
                else {
                    return Ok(None);
                };
                Some(match self {
                    Self::Substring { start, end, .. } => {
                        let chars = text.chars().collect::<Vec<_>>();
                        let len = chars.len() as i32;
                        let start = resolve_char(*start, 0, len);
                        let end = resolve_char(*end, len, len);
                        if start > end || start >= len as usize || end > len as usize {
                            String::new()
                        } else {
                            chars[start..end].iter().collect()
                        }
                    }
                    Self::Replace { regex, by, .. } => {
                        replace_bounded(&text, regex, by, max_output_bytes)?
                    }
                    Self::Convert {
                        case, separators, ..
                    } => case.apply(&text, separators.as_deref()),
                    Self::Line { .. } | Self::Lines { .. } | Self::Count { .. } => unreachable!(),
                })
            }
        };
        if output
            .as_ref()
            .is_some_and(|text| text.len() > max_output_bytes)
        {
            return Err(ComputeError::Limit);
        }
        Ok(output)
    }
}

fn replace_bounded(
    text: &str,
    regex: &Regex,
    by: &str,
    max_output_bytes: usize,
) -> Result<String, TransformLimit> {
    fn append(output: &mut String, text: &str, limit: usize) -> Result<(), TransformLimit> {
        if output
            .len()
            .checked_add(text.len())
            .is_none_or(|bytes| bytes > limit)
        {
            return Err(TransformLimit);
        }
        output.push_str(text);
        Ok(())
    }

    let mut output = String::new();
    let mut end = 0;
    for captures in regex.captures_iter(text) {
        let matched = captures.get(0).expect("regex capture has whole match");
        append(&mut output, &text[end..matched.start()], max_output_bytes)?;
        let mut cursor = 0;
        while let Some(dollar) = by[cursor..].find('$') {
            let dollar = cursor + dollar;
            append(&mut output, &by[cursor..dollar], max_output_bytes)?;
            let reference = dollar + 1;
            let Some(first) = by[reference..].chars().next() else {
                append(&mut output, "$", max_output_bytes)?;
                cursor = reference;
                break;
            };
            if first == '$' {
                append(&mut output, "$", max_output_bytes)?;
                cursor = reference + first.len_utf8();
                continue;
            }
            let (name, next) = if first == '{' {
                let body = reference + 1;
                let Some(close) = by[body..].find('}') else {
                    append(&mut output, "$", max_output_bytes)?;
                    cursor = reference;
                    continue;
                };
                (&by[body..body + close], body + close + 1)
            } else if first.is_ascii_alphanumeric() || first == '_' {
                let next = by[reference..]
                    .find(|character: char| !character.is_ascii_alphanumeric() && character != '_')
                    .map_or(by.len(), |offset| reference + offset);
                (&by[reference..next], next)
            } else {
                append(&mut output, "$", max_output_bytes)?;
                cursor = reference;
                continue;
            };
            let capture = name
                .parse::<usize>()
                .ok()
                .and_then(|index| captures.get(index))
                .or_else(|| captures.name(name));
            if let Some(capture) = capture {
                append(&mut output, capture.as_str(), max_output_bytes)?;
            }
            cursor = next;
        }
        if cursor < by.len() {
            append(&mut output, &by[cursor..], max_output_bytes)?;
        }
        end = matched.end();
    }
    append(&mut output, &text[end..], max_output_bytes)?;
    Ok(output)
}

/// Where a capture stands in its file, and the file's lines: a node's range, a `$$$` capture's
/// from its first node to its last, or the host range a parsed tree exported. `None` where it is
/// unbound or no node of the host file is bound to place it.
fn placed<'t, D: Doc>(
    env: &MetaVarEnv<'t, D>,
    source: &MetaVariable,
) -> Option<(Range<usize>, &'t LineTable)> {
    let (range, node) = match source {
        MetaVariable::Capture(name, _) => match env.get_match(name) {
            Some(node) => (node.range(), Some(node.clone())),
            None => (exported_span(env, name)?, None),
        },
        MetaVariable::MultiCapture(name) => {
            let nodes = env.get_multiple_matches(name);
            let (first, last) = (nodes.first()?, nodes.last()?);
            (first.range().start..last.range().end, Some(first.clone()))
        }
        MetaVariable::Dropped(_) | MetaVariable::Multiple => return None,
    };
    // An exported capture has no node: any node of the host places the file.
    let node = node.or_else(|| any_node(env))?;
    Some((range, host_lines(&node)?))
}

/// A node some single capture of `env` holds.
fn any_node<'t, D: Doc>(env: &MetaVarEnv<'t, D>) -> Option<Node<'t, D>> {
    env.get_matched_variables()
        .find_map(|variable| match variable {
            MetaVariable::Capture(name, _) => env.get_match(&name).cloned(),
            MetaVariable::MultiCapture(name) => env.get_multiple_matches(&name).first().cloned(),
            MetaVariable::Dropped(_) | MetaVariable::Multiple => None,
        })
}

fn resolve_char(index: Option<i32>, default: i32, len: i32) -> usize {
    let index = index.unwrap_or(default);
    if index >= len {
        len as usize
    } else if index >= 0 {
        index as usize
    } else if len + index < 0 {
        0
    } else {
        (len + index) as usize
    }
}

/// The query's transforms, in an order where each source is computed before it is read.
#[derive(Clone, Default)]
pub(crate) struct Transform {
    pub(crate) steps: Vec<(String, Trans)>,
    pub(crate) max_output_bytes: usize,
}

impl Transform {
    pub(crate) fn contains(&self, name: &str) -> bool {
        self.steps.iter().any(|(key, _)| key == name)
    }

    /// Whether a step reads the variable `name`: each step reads its source alone.
    pub(crate) fn reads(&self, name: &str) -> bool {
        self.steps
            .iter()
            .any(|(_, trans)| trans.source_name() == name)
    }

    pub(crate) fn names(&self) -> impl Iterator<Item = &str> {
        self.steps.iter().map(|(key, _)| key.as_str())
    }

    /// Upstream's `apply_transform`: each variable in order, the empty text where its source is
    /// unbound (or its value unknown), de-indented relative to its source node.
    pub(crate) fn apply<D: Doc>(&self, env: &mut MetaVarEnv<'_, D>) -> Result<(), TransformLimit> {
        self.apply_steps(env, |_| true)
    }

    /// Runs, in order, the steps whose index `run` selects.
    pub(crate) fn apply_steps<D: Doc>(
        &self,
        env: &mut MetaVarEnv<'_, D>,
        mut run: impl FnMut(usize) -> bool,
    ) -> Result<(), TransformLimit> {
        for (index, (key, trans)) in self.steps.iter().enumerate() {
            if !run(index) {
                continue;
            }
            let text = match trans.compute(env, self.max_output_bytes) {
                Ok(text) => text.unwrap_or_default(),
                Err(ComputeError::Unknown) => String::new(),
                Err(ComputeError::Limit) => return Err(TransformLimit),
            };
            let bytes = <D::Source as Content>::decode_str(&text).to_vec();
            env.insert_transformation(trans.source(), key, bytes);
        }
        Ok(())
    }

    /// Whether a variable reads a capture's value (`value`), which may not be known.
    pub(crate) fn reads_values(&self) -> bool {
        self.steps.iter().any(|(_, trans)| trans.reads_value())
    }

    /// A transform variable's text over the bindings so far, for a comparison; `Ok(None)` where a
    /// capture it derives from is unbound, `Err` where a value it reads is not known.
    pub(crate) fn text<D: Doc>(
        &self,
        name: &str,
        env: &MetaVarEnv<'_, D>,
    ) -> Result<Option<String>, UnknownValue> {
        let chain = self.chain(name);
        if chain.is_empty() {
            return Ok(None);
        }
        let mut scratch = env.clone();
        for (key, trans) in self
            .steps
            .iter()
            .filter(|(key, _)| chain.contains(key.as_str()))
        {
            let Some(text) = trans
                .compute(&scratch, self.max_output_bytes)
                .map_err(|_| UnknownValue)?
            else {
                return Ok(None);
            };
            let bytes = <D::Source as Content>::decode_str(&text).to_vec();
            scratch.insert_transformation(trans.source(), key, bytes);
        }
        Ok(scratch
            .get_transformed(name)
            .map(|bytes| <D::Source as Content>::encode_bytes(bytes).into_owned()))
    }

    /// The transform variables `name` is computed from, itself included.
    fn chain(&self, name: &str) -> BTreeSet<&str> {
        let mut chain = BTreeSet::new();
        let mut next = Some(name);
        while let Some(key) = next.take() {
            let Some((key, trans)) = self.steps.iter().find(|(known, _)| known == key) else {
                break;
            };
            if chain.insert(key.as_str()) {
                next = Some(trans.source_name());
            }
        }
        chain
    }

    /// The captures `name` is computed from, following transform variables to their sources.
    pub(crate) fn roots(&self, name: &str) -> BTreeSet<&str> {
        let mut roots = BTreeSet::new();
        for key in self.chain(name) {
            if let Some((_, trans)) = self.steps.iter().find(|(known, _)| known == key)
                && !self.contains(trans.source_name())
            {
                roots.insert(trans.source_name());
            }
        }
        roots
    }
}

#[cfg(test)]
mod tests {
    use super::replace_bounded;
    use crate::policy_dsl::CompiledPolicyDsl;
    use crate::policy_dsl::test_support::*;
    use regex::Regex;
    use serde_json::{Value, json};
    use std::sync::Arc;

    fn compile(ast: Value, finding: Value) -> napi::Result<CompiledPolicyDsl> {
        let mut ast = ast;
        if ast.get("language").is_none() {
            ast["language"] = json!("typescript");
        }
        compile_document_with(
            check_document(json!({ "id": "c", "forbid": { "ast": ast }, "finding": finding })),
            &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
        )
    }

    #[test]
    fn line_transforms_place_a_capture_with_its_documents_line_table() {
        use crate::policy_dsl::language::GrammarDialect;
        use crate::policy_dsl::tape::{Grammar, TapeDoc, TapeImage};
        use ast_grep_core::matcher::Pattern;
        use ast_grep_core::meta_var::{MetaVarEnv, MetaVariable};
        use ast_grep_core::tree_sitter::StrDoc;
        use ast_grep_core::{AstGrep, Doc};
        let source = "a;\nb;\nlast;\n";
        let grammar = Grammar::new(GrammarDialect::TypeScript).unwrap();
        let native = StrDoc::new(source, GrammarDialect::TypeScript);
        let image =
            TapeImage::encode(native.tree.root_node(), &grammar, source, &|| false).unwrap();
        let tape =
            TapeDoc::restore(Arc::new(source.to_owned()), grammar, &image, &|| false).unwrap();
        let root = AstGrep::doc(tape);
        let pattern = Pattern::try_new("last", GrammarDialect::TypeScript).unwrap();
        let node = root
            .root()
            .find(&pattern)
            .expect("identifier")
            .get_node()
            .clone();
        assert!(node.get_doc().get_source().ends_with("last;\n"));
        let mut env = MetaVarEnv::new();
        env.insert("X", node);
        let line = super::Trans::Line {
            source: super::Source {
                variable: MetaVariable::Capture("X".to_owned(), false),
                default: None,
                value: None,
            },
            end: false,
        };
        super::reset_line_table_constructions();
        assert_eq!(
            line.compute(&env, 1024).ok().flatten().as_deref(),
            Some("3")
        );
        assert_eq!(
            super::line_table_constructions(),
            0,
            "a tape document's own line table places the capture; none is built"
        );
    }

    fn output(ast: Value, message: &str, fix: Option<&str>, source: &str) -> Vec<(String, Value)> {
        let mut finding = json!({ "anchor": "witness.span", "code": "X", "message": message, "severity": "error" });
        if let Some(fix) = fix {
            finding["fix"] = json!(fix);
        }
        let compiled = compile(ast, finding).unwrap_or_else(|error| panic!("{}", error.reason));
        let result = evaluate_file(&compiled, "typescript", "a.ts", source);
        assert_eq!(result["failures"], json!([]), "{result}");
        result["findings"]
            .as_array()
            .expect("findings")
            .iter()
            .map(|finding| {
                (
                    finding["message"].as_str().expect("message").to_owned(),
                    finding["fix"]["text"].clone(),
                )
            })
            .collect()
    }

    #[test]
    fn transforms_feed_messages_and_fixes() {
        let ast = json!({
            "rule": { "pattern": "console.log($A)" },
            "transform": {
                "TRIMMED": { "replace": { "source": "$A", "replace": "^['\"]\\s*|\\s*['\"]$", "by": "" } },
                "SHOUT": { "convert": { "source": "$TRIMMED", "toCase": "upperCase" } },
                "HEAD": { "substring": { "source": "$SHOUT", "endChar": 3 } },
                "SNAKE": { "convert": { "source": "$A", "toCase": "snakeCase", "separatedBy": ["caseChange"] } },
            },
        });
        let source = "console.log(' hello world ');\nconsole.log(userName);\n";
        assert_eq!(
            output(
                ast,
                "log $TRIMMED as $SHOUT ($HEAD, $SNAKE)",
                Some("logger.info(\"$SHOUT\")"),
                source
            ),
            [
                (
                    "log hello world as HELLO WORLD (HEL, '_ hello world ')".to_owned(),
                    json!("logger.info(\"HELLO WORLD\")")
                ),
                (
                    "log userName as USERNAME (USE, user_name)".to_owned(),
                    json!("logger.info(\"USERNAME\")")
                ),
            ]
        );
        // A source an `any` branch left unbound transforms to the empty text, as upstream.
        let ast = json!({
            "rule": { "any": [{ "pattern": "f($A)" }, { "pattern": "g()" }] },
            "transform": { "B": { "substring": { "source": "$A", "startChar": 1, "endChar": -1 } } },
        });
        assert_eq!(
            output(ast, "[$B]", None, "f(abcd);\ng();\n"),
            [
                ("[bc]".to_owned(), Value::Null),
                ("[]".to_owned(), Value::Null)
            ]
        );
    }

    /// `default` (a Rerail extension): the text an unbound source reads as, which the operation then
    /// transforms; without it an unbound source is the empty text and no operation runs, as
    /// upstream.
    #[test]
    fn a_default_is_the_text_of_an_unbound_source() {
        let ast = |default: Option<&str>| {
            let mut replace = json!({ "source": "$A", "replace": "^$", "by": "none" });
            if let Some(default) = default {
                replace["default"] = json!(default);
            }
            json!({
                "rule": { "any": [{ "pattern": "f($A)" }, { "pattern": "g()" }] },
                "transform": {
                    "B": { "replace": replace },
                    "C": { "substring": { "source": "$A", "startChar": 1, "default": "<unset>" } },
                },
            })
        };
        let source = "f(abcd);\ng();\nf('');\n";
        let messages = |default| {
            output(ast(default), "[$B] [$C]", None, source)
                .into_iter()
                .map(|(message, _)| message)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            messages(Some("")),
            ["[abcd] [bcd]", "[none] [unset>]", "[''] [']"]
        );
        assert_eq!(messages(None), ["[abcd] [bcd]", "[] [unset>]", "[''] [']"]);
        let ast = json!({
            "rule": { "any": [{ "pattern": "f($A)" }, { "pattern": "g()" }] },
            "transform": { "B": { "replace": { "source": "$A", "replace": "^$", "by": "none" } } },
        });
        assert_eq!(
            output(ast, "[$B]", Some("h($B)"), "f(abcd);\ng();\n"),
            [
                ("[abcd]".to_owned(), json!("h(abcd)")),
                ("[]".to_owned(), json!("h()"))
            ]
        );
        // A comparison reads the default where the source is unbound.
        let compared = json!({
            "rule": { "kind": "call_expression", "any": [{ "pattern": "f($A)" }, { "pattern": "g()" }],
                      "has": { "kind": "arguments", "not": { "compare": { "left": "B", "op": "ne", "right": "U", "text": "source" } },
                               "pattern": "$U" } },
            "transform": { "B": { "replace": { "source": "$A", "replace": "^$", "by": "()", "default": "" } } },
        });
        assert_eq!(
            output(compared, "$B", None, "f(abcd);\ng();\n"),
            [("()".to_owned(), Value::Null)]
        );
    }

    /// `value: true` (a Rerail extension): the source capture's value (M9's `value`, not followed)
    /// as the host language converts it to a string, JavaScript's `String(v)` or Python's `str(v)`;
    /// a JavaScript identifier reads as its name, escapes decoded. An unknown value reads as an
    /// unbound source: the default, else the empty text.
    #[test]
    fn a_transform_reads_a_captures_value() {
        let ast = |language: &str| {
            json!({
                "language": language,
                "rule": { "kind": "call", "pattern": "f($A)" },
                "transform": {
                    "V": { "substring": { "source": "$A", "value": true, "default": "?" } },
                    "T": { "replace": { "source": "$A", "value": true, "replace": "^\\s+|\\s+$", "by": "" } },
                },
            })
        };
        let messages = |language: &str, source: &str| {
            let mut ast = ast(language);
            if language == "typescript" {
                ast["rule"]["kind"] = json!("call_expression");
            }
            let finding = json!({ "anchor": "witness.span", "code": "X", "message": "[$V] [$T]", "severity": "error" });
            let compiled = compile(ast, finding).unwrap_or_else(|error| panic!("{}", error.reason));
            let path = if language == "python" { "a.py" } else { "a.ts" };
            let result = evaluate_file(&compiled, language, path, source);
            assert_eq!(result["failures"], json!([]), "{result}");
            result["findings"]
                .as_array()
                .expect("findings")
                .iter()
                .map(|finding| finding["message"].as_str().expect("message").to_owned())
                .collect::<Vec<_>>()
        };
        let source = "f('\\x61 b ');\nf(1e2);\nf(0x10);\nf(\\u0061);\nf({});\nf(null);\nf(`t${x}`);\nf(-0);\n";
        assert_eq!(
            messages("typescript", source),
            [
                "[a b ] [a b]",
                "[100] [100]",
                "[16] [16]",
                "[a] [a]",
                "[?] []",
                "[null] [null]",
                "[?] []",
                "[0] [0]"
            ]
        );
        let source =
            "f('a' 'b ')\nf(1e2)\nf(0x10)\nf(True)\nf(0.1 + 0.2)\nf(1e16)\nf(x)\nf(None)\nf(-7)\n";
        assert_eq!(
            messages("python", source),
            [
                "[ab ] [ab]",
                "[100.0] [100.0]",
                "[16] [16]",
                "[True] [True]",
                "[0.30000000000000004] [0.30000000000000004]",
                "[1e+16] [1e+16]",
                "[?] []",
                "[None] [None]",
                "[-7] [-7]",
            ]
        );
        // A comparison of a value that is not known is unknown, so a negated one reports nothing
        // there; with a default it reads the default.
        let differs = |default: Option<&str>| {
            let mut substring = json!({ "source": "$A", "value": true });
            if let Some(default) = default {
                substring["default"] = json!(default);
            }
            let ast = json!({
                "rule": { "pattern": "f($A, $B)",
                          "not": { "compare": { "left": "V", "op": "eq", "right": "B", "text": "source" } } },
                "transform": { "V": { "substring": substring } },
            });
            output(ast, "$A", None, "f(1e2, 100);\nf(1e2, 5);\nf(a.b, 5);\n")
                .into_iter()
                .map(|(message, _)| message)
                .collect::<Vec<_>>()
        };
        assert_eq!(differs(None), ["1e2"]);
        assert_eq!(differs(Some("")), ["1e2", "a.b"]);
        let reject = |transform: Value| {
            rejection(compile(
                json!({ "rule": { "pattern": "f($A, $$$R)" }, "transform": transform }),
                json!({ "anchor": "witness.span", "code": "X", "message": "m", "severity": "error" }),
            ))
        };
        let prefix = "/checks/0/forbid/ast/transform";
        assert_eq!(
            reject(json!({ "B": { "substring": { "source": "$$$R", "value": true } } })),
            format!("{prefix}/B/substring/value: a value is read from a single capture (`$NAME`)")
        );
        assert_eq!(
            reject(
                json!({ "B": { "substring": { "source": "$A" } }, "C": { "substring": { "source": "$B", "value": true } } })
            ),
            format!(
                "{prefix}/C/substring/value: `$B` is a transform variable, which is text; read the value of a capture"
            )
        );
        assert_eq!(
            reject(json!({ "B": { "substring": { "source": "$A", "value": "yes" } } })),
            format!("{prefix}/B/substring/value must be a boolean")
        );
    }

    #[test]
    fn line_lines_and_count_read_where_a_capture_stands() {
        // typescript-eslint 8.70.1's unified-signatures, a RuleTester case of
        // `tests/rules/unified-signatures.test.ts` ("For 3 or more overloads, mentions the line"):
        // the report at 5:6 names the other overload's line.
        let source = "\ninterface I {\n  a0(): void;\n  a0(x: string): string;\n  a0(x: number): void;\n}\n      ";
        let void = json!({ "has": { "kind": "type_annotation", "regex": "void" } });
        let mut other = json!({ "stopBy": "end", "kind": "method_signature", "pattern": "$O",
            "has": { "kind": "formal_parameters", "regex": "^\\(\\)$" } });
        other["all"] = json!([void.clone()]);
        let mut overload = json!({ "kind": "method_signature", "follows": other });
        overload["all"] = json!([void]);
        let ast = json!({
            "rule": { "kind": "required_parameter", "inside": { "kind": "formal_parameters", "inside": overload } },
            "transform": { "L": { "line": { "source": "$O" } } },
        });
        let message = "This overload and the one on line $L can be combined into one signature with an optional parameter.";
        let compiled = compile(ast, json!({ "anchor": "witness.span", "code": "X", "message": message, "severity": "error" }))
            .unwrap_or_else(|error| panic!("{}", error.reason));
        let result = evaluate_file(&compiled, "typescript", "a.ts", source);
        let findings = result["findings"].as_array().expect("findings");
        assert_eq!(findings.len(), 1, "{result}");
        assert_eq!(
            findings[0]["message"],
            "This overload and the one on line 3 can be combined into one signature with an optional parameter."
        );
        assert_eq!(
            (
                &findings[0]["span"]["startLine"],
                &findings[0]["span"]["startColumn"]
            ),
            (&json!(4), &json!(5)),
            "0-based: ESLint's 5:6"
        );
        // Counts: max-params' parameters, max-lines-per-function's lines; lines end where the
        // linter ends them (U+2028 in TypeScript).
        let ast = json!({
            "rule": { "kind": "function_declaration", "pattern": "function $F($$$P) { $$$ }" },
            "transform": {
                "N": { "count": { "source": "$$$P" } },
                "LINES": { "lines": { "source": "$F" } },
                "END": { "line": { "source": "$$$P", "edge": "end" } },
            },
        });
        let source =
            "function f(a, b,\n  c) {\n}\nfunction g() {}\nfunction h(x) { return `\u{2028}`; }\n";
        assert_eq!(
            output(
                ast,
                "$F: $N parameters, name on $LINES line, parameters end on line $END",
                None,
                source
            ),
            [
                (
                    "f: 3 parameters, name on 1 line, parameters end on line 2".to_owned(),
                    Value::Null
                ),
                (
                    "g: 0 parameters, name on 1 line, parameters end on line ".to_owned(),
                    Value::Null
                ),
                (
                    "h: 1 parameters, name on 1 line, parameters end on line 5".to_owned(),
                    Value::Null
                ),
            ]
        );
        let lines = json!({ "rule": { "kind": "function_declaration", "pattern": "$D" },
            "transform": { "N": { "lines": { "source": "$D" } } } });
        assert_eq!(
            output(lines, "$N", None, source)
                .into_iter()
                .map(|(message, _)| message)
                .collect::<Vec<_>>(),
            ["3", "1", "2"]
        );
        // Python's lines end at CR, LF and CRLF, not at U+2028.
        let python = compile(
            json!({ "language": "python", "rule": { "kind": "function_definition", "pattern": "$D" },
                "transform": { "S": { "line": { "source": "$D" } }, "E": { "line": { "source": "$D", "edge": "end" } } } }),
            json!({ "anchor": "witness.span", "code": "X", "message": "$S-$E", "severity": "error" }),
        )
        .unwrap_or_else(|error| panic!("{}", error.reason));
        let source = "x = '\u{2028}'\rdef f():\r\n    pass\ndef g(): pass\n";
        let result = evaluate_file(&python, "python", "a.py", source);
        let messages = result["findings"]
            .as_array()
            .expect("findings")
            .iter()
            .map(|finding| finding["message"].as_str().expect("message").to_owned())
            .collect::<Vec<_>>();
        assert_eq!(messages, ["2-3", "4-4"]);
        // Text has no lines: a transform variable is refused as their source.
        let refused = rejection(compile(
            json!({ "rule": { "pattern": "f($A)" }, "transform": {
                "T": { "substring": { "source": "$A" } }, "L": { "line": { "source": "$T" } } } }),
            json!({ "anchor": "witness.span", "code": "X", "message": "m", "severity": "error" }),
        ));
        assert_eq!(
            refused,
            "/checks/0/forbid/ast/transform/L: `$T` is a transform variable, text; line, lines and count read a capture"
        );
    }

    #[test]
    fn count_passes_over_comments() {
        // A `$$$` capture holds the comments between its nodes, as upstream's does; `count` counts
        // what ESLint's max-params and max-statements count, the parameters and statements.
        let ast = json!({
            "rule": {
                "kind": "function_declaration",
                "has": { "field": "parameters", "pattern": { "context": "function f($$$P) {}", "selector": "formal_parameters" } },
                "all": [{ "has": { "field": "body", "pattern": { "context": "function f() { $$$S }", "selector": "statement_block" } } }],
            },
            "transform": { "N": { "count": { "source": "$$$P" } }, "M": { "count": { "source": "$$$S" } } },
        });
        let source = "function f(a, /* c */ b) {}\nfunction g() { a; /* c */ b; // d\n}\nfunction h(/* only */) { /* none */ }\n";
        assert_eq!(
            output(ast, "$N $M ($$$P) {$$$S}", None, source),
            [
                ("2 0 (a, /* c */ b) {}".to_owned(), Value::Null),
                ("0 2 () {a; /* c */ b; // d}".to_owned(), Value::Null),
                ("0 0 (/* only */) {/* none */}".to_owned(), Value::Null),
            ]
        );
        // Python: a comment among the parameters.
        let python = compile(
            json!({ "language": "python",
                "rule": { "kind": "function_definition", "has": { "field": "parameters",
                    "pattern": { "context": "def f($$$P): pass", "selector": "parameters" } } },
                "transform": { "N": { "count": { "source": "$$$P" } } } }),
            json!({ "anchor": "witness.span", "code": "X", "message": "$N", "severity": "error" }),
        )
        .unwrap_or_else(|error| panic!("{}", error.reason));
        let source = "def f(a,  # c\n      b):\n    x = 1\n";
        let result = evaluate_file(&python, "python", "a.py", source);
        assert_eq!(result["findings"][0]["message"], "2", "{result}");
    }

    #[test]
    fn a_regex_group_of_the_witness_is_a_transform_source() {
        // capitalized-comments' fix: the comment's first letter, lowercased, over the group.
        let ast = json!({
            "rule": { "kind": "comment", "regex": "^(?://|/\\*)[\\s*]*(?<L>\\p{Lu})(?<REST>x)?" },
            "transform": {
                "C": { "convert": { "source": "$L", "toCase": "lowerCase" } },
                "R": { "replace": { "source": "$REST", "default": "-", "replace": "^$", "by": "none" } },
                "AT": { "line": { "source": "$L" } },
            },
        });
        let mut finding = json!({ "anchor": "capture.L", "code": "X", "severity": "error",
            "message": "$L to $C on line $AT, rest $R", "fix": { "template": "$C", "range": "capture.L" } });
        let compiled = compile(ast.clone(), finding.clone())
            .unwrap_or_else(|error| panic!("{}", error.reason));
        let source = "// Abc\n/* Éx */\n// abc\n";
        let result = evaluate_file(&compiled, "typescript", "a.ts", source);
        let rows = result["findings"]
            .as_array()
            .expect("findings")
            .iter()
            .map(|finding| {
                let start = finding["fix"]["span"]["startByte"].as_u64().expect("fix") as usize;
                let end = finding["fix"]["span"]["endByte"].as_u64().expect("fix") as usize;
                (
                    finding["message"].as_str().expect("message").to_owned(),
                    source[start..end].to_owned(),
                    finding["fix"]["text"].as_str().expect("fix").to_owned(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            rows,
            [
                (
                    "A to a on line 1, rest -".to_owned(),
                    "A".to_owned(),
                    "a".to_owned()
                ),
                (
                    "É to é on line 2, rest x".to_owned(),
                    "É".to_owned(),
                    "é".to_owned()
                ),
            ]
        );
        // With `each`, each match of the regex computes the transform from its own groups.
        let each = json!({ "rule": { "kind": "string", "regex": "(?<D>[0-9])" },
            "transform": { "N": { "replace": { "source": "$D", "replace": "^(.)$", "by": "<$1>" } } } });
        finding = json!({ "anchor": "capture.D", "code": "X", "severity": "error", "each": "D", "message": "$N" });
        let compiled = compile(each, finding).unwrap_or_else(|error| panic!("{}", error.reason));
        let result = evaluate_file(&compiled, "typescript", "a.ts", "f('1a2');\n");
        let mut messages = result["findings"]
            .as_array()
            .expect("findings")
            .iter()
            .map(|finding| finding["message"].as_str().expect("message").to_owned())
            .collect::<Vec<_>>();
        messages.sort();
        assert_eq!(messages, ["<1>", "<2>"]);
        // A group is text: no value; and it is known only once the rule matched, so a comparison
        // cannot read a transform of it.
        let plain =
            json!({ "anchor": "witness.span", "code": "X", "message": "m", "severity": "error" });
        assert_eq!(
            rejection(compile(
                json!({ "rule": { "kind": "comment", "regex": "(?<L>a)" },
                    "transform": { "V": { "substring": { "source": "$L", "value": true } } } }),
                plain.clone(),
            )),
            "/checks/0/forbid/ast/transform/V/substring/value: `$L` is a regex group, text; read the value of a capture"
        );
        assert_eq!(
            rejection(compile(
                json!({ "rule": { "kind": "comment", "regex": "(?<L>a)", "has": { "stopBy": "end", "pattern": "$X" },
                    "compare": { "left": "C", "op": "eq", "right": "X", "text": "source" } },
                    "transform": { "C": { "substring": { "source": "$L" } } } }),
                plain,
            )),
            "/checks/0/forbid/ast/rule/compare/left: `$C` transforms `$L`, which is never bound where this comparison runs"
        );
    }

    #[test]
    fn transforms_compare_as_text() {
        // no-duplicate-imports: module names compare trimmed.
        let ast = json!({
            "rule": {
                "kind": "import_statement",
                "has": { "kind": "string", "has": { "kind": "string_fragment", "pattern": "$M" } },
                "follows": { "stopBy": "end", "kind": "import_statement",
                    "has": { "kind": "string", "has": { "kind": "string_fragment", "pattern": "$P" } },
                    "compare": { "left": "PT", "op": "eq", "right": "MT" } },
            },
            "transform": {
                "MT": { "replace": { "source": "$M", "replace": "^\\s+|\\s+$", "by": "" } },
                "PT": { "replace": { "source": "$P", "replace": "^\\s+|\\s+$", "by": "" } },
            },
        });
        let source = "import a from ' m ';\nimport b from 'n';\nimport c from 'm';\n";
        assert_eq!(
            output(ast, "'$MT' import is duplicated.", None, source),
            [("'m' import is duplicated.".to_owned(), Value::Null)]
        );
    }

    /// A transform named like a capture: said plainly, before any use of the name is checked as
    /// a transform variable's (unified-signatures named its line `LA` beside a capture `$LA`, and
    /// its `index` comparison of `LA` was refused as a comparison of text).
    #[test]
    fn a_transform_named_like_a_capture_is_refused_as_such() {
        let finding =
            json!({ "anchor": "witness.span", "code": "X", "message": "m", "severity": "error" });
        let rule = json!({ "kind": "call_expression", "has": { "field": "arguments",
            "pattern": { "context": "f($LA, $LB)", "selector": "arguments" } },
            "all": [{ "compare": { "left": "LA", "op": "eq", "right": "LB", "text": "index" } }] });
        assert_eq!(
            rejection(compile(
                json!({ "rule": rule, "transform": { "LA": { "line": { "source": "$LB" } } } }),
                finding.clone()
            )),
            "/checks/0/forbid/ast/transform/LA: `LA` names a capture of the query (`$LA` at /checks/0/forbid/ast/rule/has/pattern) and a transform variable; rename one"
        );
        // A capture a utility binds, a parse captures, or a call returns collides too.
        for (ast, at) in [
            (
                json!({ "rule": { "matches": "arg" }, "utils": { "arg": { "pattern": "f($N)" } },
                    "transform": { "N": { "line": { "source": "$N2" } } } }),
                "/checks/0/forbid/ast/utils/arg/pattern",
            ),
            (
                json!({ "rule": { "kind": "comment", "parse": { "grammar": "eslintDirective", "has": { "kind": "item", "capture": "N" } } },
                    "transform": { "N": { "substring": { "source": "$X" } } } }),
                "/checks/0/forbid/ast/rule/parse/has/capture",
            ),
        ] {
            let message = rejection(compile(ast, finding.clone()));
            assert!(
                message.starts_with(
                    "/checks/0/forbid/ast/transform/N: `N` names a capture of the query (`$N` at "
                ) && message.contains(at),
                "{message}"
            );
        }
    }

    #[test]
    fn transforms_are_checked_against_the_query() {
        let finding = || json!({ "anchor": "witness.span", "code": "X", "message": "m", "severity": "error" });
        let reject = |ast: Value, finding: Value| rejection(compile(ast, finding));
        let pattern = json!({ "pattern": "f($A, $C)" });
        let prefix = "/checks/0/forbid/ast/transform";
        for (transform, expected) in [
            (
                json!({ "A": { "substring": { "source": "$C" } } }),
                format!(
                    "{prefix}/A: `A` names a capture of the query (`$A` at /checks/0/forbid/ast/rule/pattern) and a transform variable; rename one"
                ),
            ),
            (
                json!({ "B": { "substring": { "source": "$E" } } }),
                format!("{prefix}/B/source: `$E` names no capture the query can bind"),
            ),
            (
                json!({ "B": { "substring": { "source": "$D" } }, "D": { "substring": { "source": "$B" } } }),
                format!("{prefix}/B: `B` has a cyclic dependency"),
            ),
            (
                json!({ "B": { "substring": { "source": "A" } } }),
                format!(
                    "{prefix}/B/substring/source: `A` should be a $-prefixed metavariable (`$NAME` or `$$$NAME`)"
                ),
            ),
            (
                json!({ "B": "substring($A, startChar=1)" }),
                format!(
                    "{prefix}/B must be an object with one of replace, substring, convert, line, lines or count; the string form is not supported"
                ),
            ),
            (
                json!({ "B": { "rewrite": { "source": "$A", "rewriters": ["r"] } } }),
                format!(
                    "{prefix}/B/rewrite: rewrite is not supported yet; use replace, substring or convert"
                ),
            ),
            (
                json!({ "B": { "replace": { "source": "$A", "replace": "(", "by": "" } } }),
                format!("{prefix}/B/replace/replace: Parsing text matcher fails."),
            ),
            (
                json!({ "B": { "convert": { "source": "$A", "toCase": "titleCase" } } }),
                format!(
                    "{prefix}/B/convert/toCase: unknown value \"titleCase\"; expected one of lowerCase, upperCase, capitalize, camelCase, snakeCase, kebabCase, pascalCase"
                ),
            ),
            (
                json!({ "B": { "substring": { "source": "$A", "startChar": 1.5 } } }),
                format!("{prefix}/B/substring/startChar must be an integer"),
            ),
            (
                json!({ "b": { "substring": { "source": "$A" } } }),
                format!("{prefix}/b: `b` is not a metavariable name (A-Z, 0-9, _, without `$`)"),
            ),
            (
                json!({ "B": { "substring": { "source": "$A" }, "convert": { "source": "$A", "toCase": "lowerCase" } } }),
                format!(
                    "{prefix}/B must hold exactly one of replace, substring, convert, line, lines or count"
                ),
            ),
            (
                json!({ "B": { "substring": { "source": "$A", "default": 1 } } }),
                format!("{prefix}/B/substring/default must be a string"),
            ),
        ] {
            let ast = json!({ "rule": pattern, "transform": transform });
            assert_eq!(reject(ast, finding()), expected);
        }
        let transform = json!({ "B": { "substring": { "source": "$A" } } });
        let mut multi = finding();
        multi["message"] = json!("$$$B");
        assert_eq!(
            reject(json!({ "rule": pattern, "transform": transform }), multi),
            "/checks/0/finding/message: `$$$B`: a transform variable is text; use `$B`"
        );
        let mut anchor = finding();
        anchor["anchor"] = json!("capture.B");
        assert!(
            reject(json!({ "rule": pattern, "transform": transform }), anchor)
                .starts_with("/checks/0/finding/anchor: `capture.B` names no capture")
        );
        let compare = |text: &str| json!({ "pattern": "f($A, $C)", "compare": { "left": "B", "op": "eq", "right": "C", "text": text } });
        assert_eq!(
            reject(
                json!({ "rule": compare("tokens"), "transform": transform }),
                finding()
            ),
            "/checks/0/forbid/ast/rule/compare/text: a transform variable is text; compare it as source or unquoted"
        );
        compile(
            json!({ "rule": compare("unquoted"), "transform": transform }),
            finding(),
        )
        .unwrap_or_else(|error| panic!("{}", error.reason));
        // A comparison sees a transform only where its source is bound.
        let hidden = json!({ "pattern": "f($A, $C)", "inside": { "stopBy": {
            "kind": "program", "compare": { "left": "B", "op": "eq", "right": "C" },
        }, "kind": "program" } });
        assert_eq!(
            reject(json!({ "rule": hidden, "transform": transform }), finding()),
            "/checks/0/forbid/ast/rule/inside/stopBy/compare/left: `$B` transforms `$A`, which is never bound where this comparison runs"
        );
    }

    #[test]
    fn transform_output_is_limited_by_max_source_bytes() {
        let mut transform = serde_json::Map::new();
        let mut source = "$A".to_owned();
        for index in 0..20 {
            let name = format!("T{index}");
            transform.insert(
                name.clone(),
                json!({ "replace": { "source": source, "replace": "^.*$", "by": "$0$0" } }),
            );
            source = format!("${name}");
        }
        let ast = json!({
            "language": "typescript",
            "rule": { "pattern": "console.log($A)" },
            "transform": transform,
        });
        let capabilities = capabilities_with(|capabilities| {
            capabilities["limits"]["maxSourceBytes"] = json!(64);
        });
        let compiled = compile_document_with(
            check_document(json!({
                "id": "c",
                "forbid": { "ast": ast },
                "finding": { "anchor": "witness.span", "code": "X", "message": source, "severity": "error" }
            })),
            &capabilities,
        )
        .unwrap_or_else(|error| panic!("{}", error.reason));
        let result = evaluate_file(&compiled, "typescript", "a.ts", "console.log(x);\n");
        assert_eq!(
            result["failures"],
            json!([{
                "checkId": "c",
                "code": "budget-exceeded",
                "message": "`matches` and `count` exceeded their work budget (maxVisitedNodes utility evaluations and counted nodes per file)",
                "path": "a.ts"
            }]),
            "{result}"
        );
    }

    #[test]
    fn replacement_budget_counts_expanded_output_not_match_length() {
        let capabilities = capabilities_with(|capabilities| {
            capabilities["limits"]["maxSourceBytes"] = json!(64);
        });
        let compiled = compile_document_with(
            check_document(json!({
                "id": "c",
                "forbid": { "ast": {
                    "language": "typescript",
                    "rule": { "pattern": "console.log($A)" },
                    "transform": {
                        "B": { "replace": { "source": "$A", "replace": ".*", "by": "ok" } }
                    }
                }},
                "finding": { "anchor": "witness.span", "code": "X", "message": "$B", "severity": "error" }
            })),
            &capabilities,
        )
        .unwrap_or_else(|error| panic!("{}", error.reason));
        let result = evaluate_file(
            &compiled,
            "typescript",
            "a.ts",
            "console.log(aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa);\n",
        );
        assert_eq!(result["failures"], json!([]), "{result}");
        assert_eq!(result["findings"][0]["message"], "ok");
    }

    #[test]
    fn bounded_replacement_matches_regex_replacement_syntax() {
        let regex = Regex::new("(?P<name>x)").expect("regex");
        for replacement in [
            "$", "$$", "$$1", "$1", "${1}", "$name", "${name}", "cost$", "$!", "é$name$",
        ] {
            assert_eq!(
                replace_bounded("x", &regex, replacement, 64)
                    .unwrap_or_else(|_| panic!("within budget")),
                regex.replace_all("x", replacement),
                "{replacement}",
            );
        }
    }
}
