//! Tests' own embedding (`Via::TestScript`, `language::test_embeds`): an HTML file's `<script>`
//! texts, then its `on*` event handlers, as one TypeScript, TSX or JavaScript program. A handler
//! runs after every script, so the program reads it after them, wherever it stands in the file: the
//! program's text stands out of host order, as a Vue template expression's does. Its first script's
//! `lang` names the dialect (`ts`, `tsx`, else JavaScript). The program is read from the host's
//! recovered tree: the host's syntax errors are not its own.

use super::segments::SegmentMap;
use super::{Embedder, Program};
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::tape::{Grammar, TapeDoc, TapeImage};
use ast_grep_core::AstGrep;
use ast_grep_language::LanguageExt;
use std::ops::Range;
use std::sync::Arc;
use tree_sitter::Node as TsNode;

/// The tape of the HTML file `source`, persisted and restored: the file's tree and the program its
/// scripts and handlers make.
pub(crate) fn html(source: &str) -> AstGrep<TapeDoc> {
    let grammar = Grammar::new(GrammarDialect::Html).expect("grammar");
    let image = TapeImage::parse(&grammar, source, &|| false)
        .expect("parses")
        .expect("encodes");
    let image =
        TapeImage::from_bytes(&image.to_bytes().expect("persists"), &grammar).expect("reads back");
    AstGrep::doc(
        TapeDoc::restore(Arc::new(source.to_owned()), grammar, &image, &|| false)
            .expect("restores"),
    )
}

/// The embedder of `Via::TestScript`.
pub(crate) struct TestScript;

impl Embedder for TestScript {
    fn build(root: TsNode<'_>, source: &str) -> Option<Program> {
        let mut scripts = Vec::new();
        let mut handlers = Vec::new();
        let mut cursor = root.walk();
        let mut stack = vec![root];
        while let Some(node) = stack.pop() {
            match node.kind() {
                "script_element" => scripts.push(node),
                "attribute" => {
                    let mut named = node.named_children(&mut cursor);
                    let name = named.next();
                    let value = named.next().map(|value| {
                        if value.kind() == "quoted_attribute_value" {
                            value.named_child(0).unwrap_or(value)
                        } else {
                            value
                        }
                    });
                    if let (Some(name), Some(value)) = (name, value)
                        && source[name.byte_range()]
                            .get(..2)
                            .is_some_and(|on| on.eq_ignore_ascii_case("on"))
                        && value.kind() == "attribute_value"
                    {
                        handlers.push(value.byte_range());
                    }
                }
                _ => {}
            }
            let children = node.named_children(&mut cursor).collect::<Vec<_>>();
            stack.extend(children.into_iter().rev());
        }
        let dialect = match scripts.first().and_then(|script| lang(*script, source)) {
            Some("ts") => GrammarDialect::TypeScript,
            Some("tsx") => GrammarDialect::Tsx,
            _ => GrammarDialect::JavaScript,
        };
        let mut text = String::new();
        let mut segments = SegmentMap::new();
        let mut copy = |range: Range<usize>, written: &str| {
            let at = text.len();
            text.push_str(&source[range.clone()]);
            segments.push(at, text.len(), range.clone(), true, false);
            let at = text.len();
            text.push_str(written);
            segments.push(at, text.len(), range.end..range.end, false, false);
        };
        for script in &scripts {
            let mut cursor = script.walk();
            if let Some(body) = script
                .named_children(&mut cursor)
                .find(|child| child.kind() == "raw_text")
            {
                copy(body.byte_range(), "\n");
            }
        }
        for handler in handlers {
            copy(handler, ";\n");
        }
        if segments.is_empty() {
            return None;
        }
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&dialect.get_ts_language()).ok()?;
        let tree = parser.parse(&text, None)?;
        segments.index();
        Some(Program {
            dialect,
            tree,
            text,
            segments,
            host_errors: Vec::new(),
        })
    }
}

/// The value of a script element's `lang` attribute.
fn lang<'s>(script: TsNode<'_>, source: &'s str) -> Option<&'s str> {
    let mut cursor = script.walk();
    let start = script
        .named_children(&mut cursor)
        .find(|child| child.kind() == "start_tag")?;
    let mut cursor = start.walk();
    let attributes = start
        .named_children(&mut cursor)
        .filter(|child| child.kind() == "attribute")
        .collect::<Vec<_>>();
    attributes.into_iter().find_map(|attribute| {
        let name = attribute.named_child(0)?;
        let value = attribute.named_child(1)?;
        let value = value.named_child(0).unwrap_or(value);
        (&source[name.byte_range()] == "lang").then(|| &source[value.byte_range()])
    })
}
