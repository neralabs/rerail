//! Vue single-file component block metadata, matching compiler-sfc 3.5.43.

use crate::code_rule_dsl::language::GrammarDialect;
use std::ops::Range;
use tree_sitter::Node;

/// One top-level SFC block as compiler-sfc 3.5.43 reads it.
pub(crate) struct Block<'t> {
    /// The top-level element node.
    pub(crate) node: Node<'t>,
    /// The tag name as written.
    pub(crate) name: &'t str,
    /// The optional `lang` attribute value.
    pub(crate) lang: Option<&'t str>,
    /// Whether the block has a `setup` attribute.
    pub(crate) setup: bool,
    /// The optional `src` attribute value.
    pub(crate) src: Option<&'t str>,
    /// The content range between start and end tags.
    pub(crate) content: Range<usize>,
}

fn attr<'t>(start: Node<'t>, source: &'t str, wanted: &str) -> Option<&'t str> {
    let mut cursor = start.walk();
    start.named_children(&mut cursor).find_map(|n| {
        if n.kind() != "attribute" {
            return None;
        }
        let name = n.named_child(0)?;
        if &source[name.byte_range()] != wanted {
            return None;
        }
        let value = n.named_child(1)?;
        Some(unquote(&source[value.byte_range()]))
    })
}
fn has_attr(start: Node<'_>, source: &str, wanted: &str) -> bool {
    let mut c = start.walk();
    start.named_children(&mut c).any(|n| {
        n.kind() == "attribute"
            && n.named_child(0)
                .map(|x| &source[x.byte_range()] == wanted)
                .unwrap_or(false)
    })
}
fn unquote(value: &str) -> &str {
    value
        .strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .or_else(|| value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
        .unwrap_or(value)
}

/// Extracts every top-level element in source order, as compiler-sfc `parse` does.
pub(crate) fn blocks<'t>(root: Node<'t>, source: &'t str) -> Vec<Block<'t>> {
    let mut c = root.walk();
    root.named_children(&mut c)
        .filter_map(|node| {
            let start = node
                .named_children(&mut node.walk())
                .find(|n| n.kind() == "start_tag" || n.kind() == "self_closing_tag")?;
            let tag = start.named_child(0)?;
            let end = node
                .named_children(&mut node.walk())
                .find(|n| n.kind() == "end_tag");
            let content = start.end_byte()..end.map_or(node.end_byte(), |n| n.start_byte());
            Some(Block {
                node,
                name: &source[tag.byte_range()],
                lang: attr(start, source, "lang"),
                setup: has_attr(start, source, "setup"),
                src: attr(start, source, "src"),
                content,
            })
        })
        .collect()
}

/// Selects the ECMAScript dialect used by the script block, or JavaScript by default.
pub(crate) fn script_dialect(blocks: &[Block<'_>]) -> Option<GrammarDialect> {
    let script = blocks
        .iter()
        .find(|b| b.name == "script" && b.setup)
        .or_else(|| blocks.iter().find(|b| b.name == "script"));
    let Some(script) = script else {
        return Some(GrammarDialect::JavaScript);
    };
    match script.lang.unwrap_or("js") {
        "ts" => Some(GrammarDialect::TypeScript),
        "tsx" => Some(GrammarDialect::Tsx),
        "js" | "jsx" => Some(GrammarDialect::JavaScript),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dialect_defaults_to_javascript() {
        assert_eq!(script_dialect(&[]), Some(GrammarDialect::JavaScript));
    }
}
