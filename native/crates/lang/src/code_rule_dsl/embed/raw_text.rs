//! `raw-text`: an HTML file's `<script>` and `<style>` texts, each a program of its own (decision 0431 §4). A
//! script is JavaScript or JSON by its `type`, and a style is CSS:
//!
//! ```text
//! <script>let a = f(1);</script>                 JavaScript: no type, or an empty one
//! <script type="module">…</script>               JavaScript: a JavaScript type, or "module"
//! <script type="application/ld+json">…</script>  JSON: a JSON type, "importmap" or "speculationrules"
//! <script type="text/template">…</script>        no program: a type no grammar reads
//! <style>p { color: red; }</style>               CSS
//! ```
//!
//! Each program's text is its element's `raw_text`, byte for byte, so its nodes stand at the file's bytes, and its
//! host node is that `raw_text` (the innermost host node over the bytes its map names). A program is read from the
//! host's recovered tree: the host's syntax errors are not its own.

use super::host::HostNode;
use super::segments::SegmentMap;
use super::{Embedder, Program};
use crate::code_rule_dsl::language::GrammarDialect;

/// The embedder of `Via::RawText`.
pub struct RawText;

/// The grammar a script's `type` names (the HTML standard's JavaScript MIME types, and the JSON ones a page holds);
/// `None` for a type no grammar reads.
fn script_dialect(kind: Option<&str>) -> Option<GrammarDialect> {
    match kind.map(str::trim) {
        None | Some("") => Some(GrammarDialect::JavaScript),
        Some("module" | "text/javascript" | "application/javascript" | "text/ecmascript" | "application/ecmascript" | "text/jsx" | "text/babel") => {
            Some(GrammarDialect::JavaScript)
        }
        Some("application/json" | "application/ld+json" | "importmap" | "speculationrules") => Some(GrammarDialect::Json),
        Some(_) => None,
    }
}

impl Embedder for RawText {
    fn build(&self, root: HostNode<'_>, source: &str, budget: &dyn Fn() -> bool) -> Option<Program> {
        self.build_all(root, source, budget).into_iter().next()
    }

    fn build_all(&self, root: HostNode<'_>, source: &str, budget: &dyn Fn() -> bool) -> Vec<Program> {
        let mut programs = Vec::new();
        let mut stack = vec![root];
        while let Some(node) = stack.pop() {
            let dialect = match node.kind() {
                "script_element" => script_dialect(attribute(node, source, "type")),
                "style_element" => Some(GrammarDialect::Css),
                _ => {
                    let children = node.named_children().collect::<Vec<_>>();
                    stack.extend(children.into_iter().rev());
                    continue;
                }
            };
            let (Some(dialect), Some(body)) = (dialect, node.named_children().find(|c| c.kind() == "raw_text")) else { continue };
            let range = body.byte_range();
            let text = source[range.clone()].to_owned();
            let mut segments = SegmentMap::new();
            segments.push(0, text.len(), range, true, false);
            segments.index();
            let Some(parse) = super::Parse::read(dialect, &text, budget) else { continue };
            programs.push(Program { dialect, parse, text, segments, host_errors: Vec::new(), at: None });
        }
        programs
    }

    fn per_place(&self) -> bool {
        true
    }
}

/// The value of element `element`'s attribute `name` (quoted or not); an empty value (`type=""`) is `""`.
fn attribute<'s>(element: HostNode<'_>, source: &'s str, name: &str) -> Option<&'s str> {
    let start = element.named_children().find(|c| c.kind() == "start_tag")?;
    start.named_children().filter(|c| c.kind() == "attribute").find_map(|a| {
        let n = a.named_child(0)?;
        if &source[n.byte_range()] != name {
            return None;
        }
        Some(match a.named_child(1) {
            Some(v) if v.kind() == "quoted_attribute_value" => v.named_child(0).map_or("", |v| &source[v.byte_range()]),
            Some(v) => &source[v.byte_range()],
            None => "",
        })
    })
}
