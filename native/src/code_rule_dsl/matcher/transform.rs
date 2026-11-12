//! An ast query's `transform` (ast-grep-config 0.45.3 `transform/`): text computed from a capture
//! after the rule and its constraints matched, as upstream's `RuleCore` does, and read by messages,
//! fixes and text comparisons. `rewrite` is not supported. `line`, `lines` and `count` (Rerail's)
//! read where a capture stands and how many nodes it holds.

mod semantic;
mod string_case;

use super::exported::exported_span;
use super::value::Reading;
use crate::code_rule_dsl::origin::InFile;
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
pub struct UnknownValue;

pub struct TransformLimit;

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
    /// ECMAScript's `Number.prototype.toPrecision(digits)` of the number the source's text reads
    /// as (a numeric literal, or a value's text), exactly; empty where it reads as no number or
    /// `digits` as no integer from 1 to 100.
    ToPrecision { source: Source, digits: Digits },
}

/// `toPrecision`'s significant digits: a number written in the rule, or the text of a capture or
/// transform variable (`$L`), read as an integer.
#[derive(Clone)]
pub(crate) enum Digits {
    Fixed(u32),
    Variable(MetaVariable),
}

impl Trans {
    /// The kinds a transformation is written as: the key of its one entry.
    pub(crate) const KINDS: [&'static str; 7] = [
        "replace",
        "substring",
        "convert",
        "line",
        "lines",
        "count",
        "toPrecision",
    ];

    /// `KINDS` as a message lists them: `replace, substring, ... or toPrecision`.
    pub(crate) fn kinds_listed() -> String {
        let (last, rest) = Self::KINDS.split_last().expect("transformation kinds");
        format!("{} or {last}", rest.join(", "))
    }

    fn source_spec(&self) -> &Source {
        match self {
            Self::Substring { source, .. }
            | Self::Replace { source, .. }
            | Self::Convert { source, .. }
            | Self::Line { source, .. }
            | Self::Lines { source }
            | Self::Count { source }
            | Self::ToPrecision { source, .. } => source,
        }
    }

    /// The names of every variable the transformation reads, `$` sigils aside: its source, and
    /// `toPrecision`'s digits where a variable gives them.
    pub(crate) fn source_names(&self) -> impl Iterator<Item = &str> {
        let digits = match self {
            Self::ToPrecision {
                digits: Digits::Variable(variable),
                ..
            } => Some(variable_name(variable)),
            _ => None,
        };
        std::iter::once(self.source_name()).chain(digits)
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
        variable_name(self.source())
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
            Self::ToPrecision { source, digits } => {
                let Some(text) = source
                    .text(env)
                    .map_err(|UnknownValue| ComputeError::Unknown)?
                else {
                    return Ok(None);
                };
                let digits = match digits {
                    Digits::Fixed(digits) => Some(*digits),
                    Digits::Variable(variable) => env
                        .get_var_bytes(variable)
                        .map(|bytes| <D::Source as Content>::encode_bytes(bytes).into_owned())
                        .and_then(|text| text.trim().parse::<u32>().ok()),
                };
                Some(
                    digits
                        .filter(|digits| (1..=100).contains(digits))
                        .zip(NUMBER_TEXT.get())
                        .and_then(|(digits, numbers)| numbers.to_precision(&text, digits))
                        .unwrap_or_default(),
                )
            }
            Self::Line { source, end } => {
                let text = placed(env, &source.variable).map(|(range, file)| {
                    let at = if *end { range.end } else { range.start };
                    (file.lines().line(at) + 1).to_string()
                });
                or_default(source, text)
            }
            Self::Lines { source } => {
                let text = placed(env, &source.variable).map(|(range, file)| {
                    let lines = file.lines();
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
                    Self::Line { .. }
                    | Self::Lines { .. }
                    | Self::Count { .. }
                    | Self::ToPrecision { .. } => unreachable!(),
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

/// `text` with each match of `regex` replaced by `by` (`$1`, `${name}` its groups, `$$` a dollar), unbounded: the
/// `replace` transform's rewrite, as `.rr`'s `@replace_all` computes it (`logic::plan`).
pub(in crate::code_rule_dsl) fn replace(text: &str, regex: &Regex, by: &str) -> String {
    replace_bounded(text, regex, by, usize::MAX).unwrap_or_else(|_| unreachable!("no output limit"))
}

/// A variable's name, `$` sigils aside.
fn variable_name(variable: &MetaVariable) -> &str {
    match variable {
        MetaVariable::Capture(name, _) | MetaVariable::MultiCapture(name) => name,
        MetaVariable::Dropped(_) | MetaVariable::Multiple => "",
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

/// Where a capture stands in its file (`origin`), and the file's lines: a node's bytes, a `$$$`
/// capture's from its first node to its last, or those of the range a nested document exported.
/// `None` where it is unbound or no node of a file's document is bound to place it.
fn placed<'t, D: Doc>(
    env: &MetaVarEnv<'t, D>,
    source: &MetaVariable,
) -> Option<(Range<usize>, InFile<'t>)> {
    match source {
        MetaVariable::Capture(name, _) => match env.get_match(name) {
            Some(node) => {
                let file = InFile::of(node)?;
                Some((file.node(node), file))
            }
            None => {
                let range = exported_span(env, name)?;
                // An exported capture has no node: any node of the document places the file.
                let file = InFile::of(&any_node(env)?)?;
                Some((file.range(range), file))
            }
        },
        MetaVariable::MultiCapture(name) => {
            let nodes = env.get_multiple_matches(name);
            let (first, last) = (nodes.first()?, nodes.last()?);
            let file = InFile::of(first)?;
            Some((file.node(first).start..file.node(last).end, file))
        }
        MetaVariable::Dropped(_) | MetaVariable::Multiple => None,
    }
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
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn simple_case(&self, name: &str) -> Option<(&str, StringCase)> {
        let (
            _,
            Trans::Convert {
                source,
                case,
                separators,
            },
        ) = self.steps.iter().find(|(key, _)| key == name)?
        else {
            return None;
        };
        let source_name = match &source.variable {
            MetaVariable::Capture(name, _) | MetaVariable::MultiCapture(name) => name.as_str(),
            MetaVariable::Dropped(_) | MetaVariable::Multiple => return None,
        };
        (source.default.is_none() && source.value.is_none() && separators.is_none())
            .then_some((source_name, *case))
    }

    pub(crate) fn contains(&self, name: &str) -> bool {
        self.steps.iter().any(|(key, _)| key == name)
    }

    /// Whether a step reads the variable `name`: its source, or `toPrecision`'s digits.
    pub(crate) fn reads(&self, name: &str) -> bool {
        self.steps
            .iter()
            .any(|(_, trans)| trans.source_names().any(|source| source == name))
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
        let mut pending = vec![name];
        while let Some(key) = pending.pop() {
            let Some((key, trans)) = self.steps.iter().find(|(known, _)| known == key) else {
                continue;
            };
            if chain.insert(key.as_str()) {
                pending.extend(trans.source_names());
            }
        }
        chain
    }

    /// The captures `name` is computed from, following transform variables to their sources.
    pub(crate) fn roots(&self, name: &str) -> BTreeSet<&str> {
        let mut roots = BTreeSet::new();
        for key in self.chain(name) {
            if let Some((_, trans)) = self.steps.iter().find(|(known, _)| known == key) {
                roots.extend(trans.source_names().filter(|source| !self.contains(source)));
            }
        }
        roots
    }
}

#[cfg(test)]
mod tests;

/// `Number::toString(x)` in base 10: the shortest digits that read back as `x`, laid out as
/// ECMA-262 §6.1.6.1.20 lays them out.
pub(crate) fn number_to_string(value: f64) -> String {
    if value.is_nan() {
        return "NaN".to_owned();
    }
    if value == 0.0 {
        return "0".to_owned();
    }
    if value.is_infinite() {
        return if value > 0.0 { "Infinity" } else { "-Infinity" }.to_owned();
    }
    if value < 0.0 {
        return format!("-{}", number_to_string(-value));
    }
    // Rust's shortest round-trip digits, as `d.ddde±x`.
    let scientific = format!("{value:e}");
    let (mantissa, exponent) = scientific
        .split_once('e')
        .expect("LowerExp writes an exponent");
    let digits = mantissa.replace('.', "");
    let exponent: i32 = exponent.parse().expect("an integer exponent");
    let k = digits.len() as i32;
    let n = exponent + 1;
    if k <= n && n <= 21 {
        format!("{digits}{}", "0".repeat((n - k) as usize))
    } else if 0 < n && n <= 21 {
        format!("{}.{}", &digits[..n as usize], &digits[n as usize..])
    } else if -6 < n && n <= 0 {
        format!("0.{}{digits}", "0".repeat((-n) as usize))
    } else {
        let sign = if n > 0 { '+' } else { '-' };
        let (first, rest) = digits.split_at(1);
        if rest.is_empty() {
            format!("{first}e{sign}{}", (n - 1).abs())
        } else {
            format!("{first}.{rest}e{sign}{}", (n - 1).abs())
        }
    }
}

/// The numbers `toPrecision` reads and writes (ECMAScript's: `Number.prototype.toPrecision` of the number a numeric
/// literal's text or a value's `String(v)` denotes), registered at load.
pub(crate) trait NumberText: Send + Sync {
    /// `text`'s number to `digits` significant digits (1 to 100); `None` where `text` denotes no number.
    fn to_precision(&self, text: &str, digits: u32) -> Option<String>;
}

static NUMBER_TEXT: std::sync::OnceLock<&'static dyn NumberText> = std::sync::OnceLock::new();

/// Registers the numbers `toPrecision` reads (once, at load).
pub(crate) fn install_number_text(numbers: &'static dyn NumberText) {
    let _ = NUMBER_TEXT.set(numbers);
}
