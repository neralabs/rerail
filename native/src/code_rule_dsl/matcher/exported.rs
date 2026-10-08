//! Captures a rule exports as text rather than as nodes: a parsed tree's captures (`parse`,
//! `formatArgs`) and a witness's regex groups that a transform reads. Each is a transformed
//! variable holding its text, beside a reserved variable holding its source range.

use ast_grep_core::Doc;
use ast_grep_core::meta_var::{MetaVarEnv, MetaVariable};
use ast_grep_core::source::Content;
use std::borrow::Cow;
use std::ops::Range;

/// The reserved variable that holds an exported capture's source range: a NUL, then the name. No
/// capture name holds a NUL, so rules and templates cannot name it.
pub(super) fn span_variable(name: &str) -> String {
    format!("\0{name}")
}

#[cfg(test)]
thread_local! {
    static EXPORTED_TEXT_BYTES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn reset_exported_text_bytes() {
    EXPORTED_TEXT_BYTES.set(0);
}

/// Bytes of group text `export_text` copied into environments on this thread.
#[cfg(test)]
pub(crate) fn exported_text_bytes() -> usize {
    EXPORTED_TEXT_BYTES.get()
}

/// Exports a parsed tree's capture to the host: its decoded text as a transformed variable, which
/// messages and fix templates interpolate, and its source range under the reserved name.
pub(super) fn export<D: Doc>(
    env: &mut Cow<MetaVarEnv<'_, D>>,
    name: &str,
    text: &str,
    range: Range<usize>,
) {
    export_text(env.to_mut(), name, text, range);
}

/// `export` on an environment the caller owns: a regex group's text and range, which a transform
/// then reads as it reads an exported capture.
pub(crate) fn export_text<D: Doc>(
    env: &mut MetaVarEnv<'_, D>,
    name: &str,
    text: &str,
    range: Range<usize>,
) {
    #[cfg(test)]
    EXPORTED_TEXT_BYTES.set(EXPORTED_TEXT_BYTES.get() + text.len());
    let bytes = <D::Source as Content>::decode_str(text).into_owned();
    env.insert_transformation(&MetaVariable::Capture(name.to_owned(), false), name, bytes);
    let span = span_variable(name);
    let bytes =
        <D::Source as Content>::decode_str(&format!("{}:{}", range.start, range.end)).into_owned();
    env.insert_transformation(&MetaVariable::Capture(span.clone(), false), &span, bytes);
}

/// The decoded text of a capture a parsed tree exported, if `name` is one.
pub(crate) fn exported_text<D: Doc>(env: &MetaVarEnv<'_, D>, name: &str) -> Option<String> {
    env.get_match(name).is_none().then_some(())?;
    env.get_transformed(&span_variable(name))?;
    let bytes = env.get_transformed(name)?;
    Some(<D::Source as Content>::encode_bytes(bytes).into_owned())
}

/// The source range of a capture a parsed tree exported, if `name` is one.
pub(crate) fn exported_span<D: Doc>(env: &MetaVarEnv<'_, D>, name: &str) -> Option<Range<usize>> {
    let bytes = env.get_transformed(&span_variable(name))?;
    let text = <D::Source as Content>::encode_bytes(bytes);
    let (start, end) = text.split_once(':')?;
    Some(start.parse().ok()?..end.parse().ok()?)
}

/// The reserved capture that holds the witness where a transform reads regex groups, so that
/// `line` and `lines` can place a group in its file. No rule or template can name it.
pub(crate) const WITNESS_NODE: &str = "\0\0witness";

/// Whether a variable name is an exported capture's reserved range.
pub(crate) fn is_span_variable(name: &str) -> bool {
    name.starts_with('\0')
}
