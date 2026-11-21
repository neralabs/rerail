//! JSON's parser for a file simdjson refuses (`json_parser::simdjson`): the Bitlex parser (`json_parser::tape::Bitlex`,
//! the ladder's JSONC grammar: comments, and an exponent's `+`, which tree-sitter-json refuses), then tree-sitter-json
//! over the text with each trailing comma blanked (as simdjson's JSONC read passes over one, decision 0158); a file
//! both refuse goes to tree-sitter-json as it is. simdjson is JSON's route on every target, MSVC's included (clang-cl
//! compiles its C++ there).
use crate::code_rule_dsl::tape::{FileImage, Grammar, TapeImage, TemplateError, TreeParser};

/// JSON's tree parser for what simdjson refuses.
pub struct Fallback;

impl TreeParser for Fallback {
    fn parse(&self, grammar: &Grammar, source: &str, budget: &dyn Fn() -> bool) -> Option<Result<FileImage, TemplateError>> {
        if !grammar.lang.embeds().is_empty()
            || !matches!(crate::code_rule_dsl::language::base(crate::code_rule_dsl::language::Family::of(grammar.lang)).parse_input(source), std::borrow::Cow::Borrowed(_))
        {
            return None;
        }
        if let Some(parsed) = super::tape::Bitlex.parse(grammar, source, budget) {
            return Some(parsed);
        }
        // A trailing comma: tree-sitter-json's tape of the text without it, which is as long (its spans are the
        // source's). Where that text has an error too, the file is tree-sitter-json's as it is.
        let text = without_trailing_commas(source);
        if text == source {
            return None;
        }
        use ast_grep_language::LanguageExt;
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&grammar.lang.get_ts_language()).ok()?;
        let tree = parser.parse(&text, None)?;
        if tree.root_node().has_error() {
            return None;
        }
        crate::code_rule_dsl::logic::profile::decision("JSON parse", || "trailing commas passed over: tree-sitter-json".to_owned());
        Some(TapeImage::encode(tree.root_node(), grammar, source, &budget).map_err(TemplateError::Encode))
    }
}

/// `source` with a space for each trailing comma (a `,` after a value and before `]` or `}`, comments between),
/// outside strings and comments: the text whose tree-sitter-json tape a JSONC file's simdjson tape is (decision
/// 0158). A comma with no value before it (`{,}`, `[1,,]`) stays: simdjson refuses the file.
pub fn without_trailing_commas(source: &str) -> String {
    let b = source.as_bytes();
    let mut out = b.to_vec();
    // The last byte outside strings, comments and whitespace (a string's closing quote for a string).
    let mut last = None;
    // The end of the string, comment or whitespace run at `i`, or `i` where none starts there.
    let skip = |i: usize| -> usize {
        match (b[i], b.get(i + 1)) {
            (b'/', Some(b'/')) => b[i..].iter().position(|&c| c == b'\n').map_or(b.len(), |n| i + n),
            (b'/', Some(b'*')) => source[i + 2..].find("*/").map_or(b.len(), |n| i + 2 + n + 2),
            (c, _) if c.is_ascii_whitespace() => i + 1,
            _ => i,
        }
    };
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'"' {
            i += 1;
            while i < b.len() && b[i] != b'"' {
                i += if b[i] == b'\\' { 2 } else { 1 };
            }
            i += 1;
            last = Some(b'"');
            continue;
        }
        let after = skip(i);
        if after != i {
            i = after;
            continue;
        }
        if b[i] == b',' {
            let mut j = i + 1;
            while j < b.len() && skip(j) != j {
                j = skip(j);
            }
            if j < b.len() && matches!(b[j], b']' | b'}') && last.is_some_and(|c| !matches!(c, b'{' | b'[' | b',')) {
                out[i] = b' ';
            }
        }
        last = Some(b[i]);
        i += 1;
    }
    // Only an ASCII comma became a space: the bytes are still the source's UTF-8.
    String::from_utf8(out).expect("UTF-8")
}
