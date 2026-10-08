//! T1 grammar measurement for a Rust pack (offline; see `generate.py`).
//!
//! Reads a JSON array `[{ "path": <abs>, "rel": <corpus-relative>, "edition": "2015"|"2018"|"2021"|"2024" }]` on
//! stdin and writes one JSON document on stdout:
//! - per file: bytes, tree-sitter-rust 0.24.2 error state (first ERROR/MISSING node), rust-analyzer
//!   (ra_ap_syntax) and syn 2 parse errors, parse times;
//! - structural agreement between tree-sitter and rust-analyzer: per comparable category, the multiset of
//!   normalized node spans each grammar builds, and their intersection;
//! - macro bodies: every outermost macro invocation's token tree, classified by what syn can read it as;
//! - for tree-sitter error files, the rust-analyzer construct enclosing the first tree-sitter error.
//!
//! rustc's own verdict (the acceptance oracle) is added by `generate.py`, which runs
//! `rustc +1.99.0 -Zparse-crate-root-only` on every file.

use std::collections::BTreeMap;
use std::io::Read;
use std::time::Instant;

use ra_ap_syntax::{AstNode, Edition, NodeOrToken, SourceFile, SyntaxKind as K, SyntaxNode};
use serde::{Deserialize, Serialize};
use syn::visit::Visit;

#[derive(Deserialize)]
struct Input {
    path: String,
    rel: String,
    edition: String,
    /// rustc 1.99.0's parser accepts the file (`-Zparse-crate-root-only`); structure is compared only then.
    accepted: bool,
}

#[derive(Serialize, Default)]
struct Position {
    line: usize,
    column: usize,
    byte: usize,
}

#[derive(Serialize)]
struct FileRecord {
    rel: String,
    edition: String,
    bytes: usize,
    lines: usize,
    ts_nodes: usize,
    ts_error: Option<TsError>,
    ra_errors: usize,
    ra_first_error: Option<RaError>,
    syn_error: Option<SynError>,
    ts_ns: u128,
    ra_ns: u128,
    syn_ns: u128,
    /// Bytes inside the token trees of outermost macro invocations (not macro_rules definitions).
    macro_body_bytes: usize,
    macro_rules_bytes: usize,
    /// Functions tree-sitter sees as `function_item` outside any token tree.
    ts_fns: usize,
    /// Functions syn finds inside outermost macro bodies that parse as items.
    fns_in_macro_bodies: usize,
}

#[derive(Serialize)]
struct TsError {
    kind: String,
    at: Position,
    /// The first error lies inside a token tree (a macro body or attribute arguments).
    in_token_tree: bool,
    error_nodes: usize,
    missing_nodes: usize,
    /// rust-analyzer node kinds enclosing the error position, innermost first (only when ra parsed cleanly).
    ra_context: Vec<String>,
    snippet: String,
}

#[derive(Serialize)]
struct RaError {
    message: String,
    at: Position,
}

#[derive(Serialize)]
struct SynError {
    message: String,
    line: usize,
    column: usize,
}

#[derive(Serialize, Default, Clone)]
struct Category {
    ts: usize,
    ra: usize,
    matched: usize,
}

#[derive(Serialize)]
struct Mismatch {
    rel: String,
    category: String,
    side: &'static str,
    line: usize,
    column: usize,
    text: String,
}

#[derive(Serialize, Default)]
struct MacroStats {
    invocations: usize,
    body_bytes: usize,
    /// Classification of outermost macro bodies by what syn reads them as.
    classes: BTreeMap<String, usize>,
    class_bytes: BTreeMap<String, usize>,
    /// name -> (invocations, body bytes, class of the first body)
    by_name: BTreeMap<String, (usize, usize, BTreeMap<String, usize>)>,
}

#[derive(Serialize)]
struct Output {
    files: Vec<FileRecord>,
    /// Structural agreement over files where rustc's verdict is not yet known: the driver filters by
    /// `comparable_files` (both tree-sitter and rust-analyzer error-free).
    categories: BTreeMap<String, Category>,
    comparable_files: usize,
    mismatches: Vec<Mismatch>,
    macros: MacroStats,
}

fn edition(e: &str) -> Edition {
    match e {
        "2015" => Edition::Edition2015,
        "2018" => Edition::Edition2018,
        "2021" => Edition::Edition2021,
        _ => Edition::Edition2024,
    }
}

struct LineIndex {
    starts: Vec<usize>,
}

impl LineIndex {
    fn new(text: &str) -> Self {
        let mut starts = vec![0];
        for (i, b) in text.bytes().enumerate() {
            if b == b'\n' {
                starts.push(i + 1);
            }
        }
        LineIndex { starts }
    }
    fn pos(&self, text: &str, byte: usize) -> Position {
        let line = match self.starts.binary_search(&byte) {
            Ok(i) => i,
            Err(i) => i - 1,
        };
        let start = self.starts[line];
        let column = text.get(start..byte).map(|s| s.chars().count()).unwrap_or(byte - start) + 1;
        Position { line: line + 1, column, byte }
    }
}

type Spans = BTreeMap<&'static str, Vec<(usize, usize)>>;

fn ts_category(node: &tree_sitter::Node) -> Option<&'static str> {
    Some(match node.kind() {
        "function_item" | "function_signature_item" => "item:fn",
        "struct_item" => "item:struct",
        "enum_item" => "item:enum",
        "union_item" => "item:union",
        "trait_item" => "item:trait",
        "impl_item" => "item:impl",
        "mod_item" => "item:mod",
        "use_declaration" => "item:use",
        "const_item" => "item:const",
        "static_item" => "item:static",
        "type_item" | "associated_type" => "item:type_alias",
        "macro_definition" => "item:macro_rules",
        "extern_crate_declaration" => "item:extern_crate",
        "foreign_mod_item" => "item:extern_block",
        "enum_variant" => "item:variant",
        "field_declaration" => "item:record_field",
        "macro_invocation" => "macro:call",
        "token_tree" => "macro:token_tree",
        "attribute_item" | "inner_attribute_item" => "attr",
        "call_expression" => {
            let callee = node.child_by_field_name("function")?;
            let callee = if callee.kind() == "generic_function" {
                callee.child_by_field_name("function")?
            } else {
                callee
            };
            if callee.kind() == "field_expression" {
                "expr:method_call"
            } else {
                "expr:call"
            }
        }
        "field_expression" => {
            // The callee of a method call is not a field access in rust-analyzer's tree.
            let is_callee = |child: tree_sitter::Node, parent: tree_sitter::Node| {
                parent.kind() == "call_expression"
                    && parent.child_by_field_name("function").map(|f| f.id()) == Some(child.id())
            };
            let parent = node.parent()?;
            let callee = is_callee(*node, parent)
                || (parent.kind() == "generic_function"
                    && parent.child_by_field_name("function").map(|f| f.id()) == Some(node.id())
                    && parent.parent().map(|g| is_callee(parent, g)).unwrap_or(false));
            if callee {
                return None;
            }
            "expr:field"
        }
        "closure_expression" => "expr:closure",
        "if_expression" => "expr:if",
        "match_expression" => "expr:match",
        "match_arm" => "expr:match_arm",
        "while_expression" => "expr:while",
        "loop_expression" => "expr:loop",
        "for_expression" => "expr:for",
        "let_declaration" => "stmt:let",
        "let_condition" => "expr:let",
        "binary_expression" | "assignment_expression" | "compound_assignment_expr" => "expr:binary",
        "unary_expression" => "expr:prefix",
        "reference_expression" => "expr:ref",
        "index_expression" => "expr:index",
        "return_expression" => "expr:return",
        "break_expression" => "expr:break",
        "continue_expression" => "expr:continue",
        "struct_expression" => "expr:record",
        "tuple_expression" | "unit_expression" => "expr:tuple",
        "array_expression" => "expr:array",
        "try_expression" => "expr:try",
        "await_expression" => "expr:await",
        "range_expression" => "expr:range",
        "type_cast_expression" => "expr:cast",
        "parenthesized_expression" => "expr:paren",
        "block" => "expr:block",
        // `x.0`: the tuple index is a NAME_REF in rust-analyzer; a receiver `64_usize.pow(2)` is a literal in both.
        "integer_literal"
            if node.parent().and_then(|p| p.child_by_field_name("field")).map(|f| f.id()) == Some(node.id()) =>
        {
            return None
        }
        // `extern "C"`: an ABI string is a token in rust-analyzer's ABI node.
        "string_literal" if node.parent().map(|p| p.kind()) == Some("extern_modifier") => return None,
        "string_literal" | "raw_string_literal" | "char_literal" | "integer_literal" | "float_literal"
        | "boolean_literal" => "expr:literal",
        "let_chain" => "expr:let_chain",
        "tuple_struct_pattern" => "pat:tuple_struct",
        "struct_pattern" => "pat:record",
        // `(P)` and `(T)`: tree-sitter has no parenthesized pattern/type; it reads a one-element tuple without
        // a trailing comma, which rust-analyzer calls PAREN_PAT / PAREN_TYPE.
        "tuple_pattern" if is_paren_tuple(node) => "pat:paren",
        "tuple_pattern" => "pat:tuple",
        "slice_pattern" => "pat:slice",
        "range_pattern" => "pat:range",
        "or_pattern" => {
            if node.parent().map(|p| p.kind()) == Some("or_pattern") {
                return None;
            }
            "pat:or"
        }
        "reference_type" => "type:ref",
        "pointer_type" => "type:ptr",
        // `Fn(A) -> B` sugar is a path type with parenthesized arguments in rust-analyzer's tree.
        "function_type" if node.child_by_field_name("trait").is_some() => return None,
        "function_type" => "type:fn_ptr",
        "tuple_type" if is_paren_tuple(node) => "type:paren",
        "tuple_type" | "unit_type" => "type:tuple",
        "array_type" => "type:array_or_slice",
        "dynamic_type" => "type:dyn",
        "abstract_type" => "type:impl",
        "never_type" => "type:never",
        "parameters"
            if node.parent().map(|p| p.kind() == "function_type" && p.child_by_field_name("trait").is_some())
                == Some(true) =>
        {
            return None
        }
        "parameters" | "closure_parameters" => "sig:param_list",
        // Closure parameters: tree-sitter lists bare patterns, rust-analyzer wraps each in PARAM.
        "parameter" if node.parent().map(|p| p.kind()) == Some("closure_parameters") => return None,
        // `self: Pin<&mut Self>`: a typed self is a plain parameter in tree-sitter, SELF_PARAM in rust-analyzer.
        "parameter" if node.child_by_field_name("pattern").map(|p| p.kind()) == Some("self") => "sig:self_param",
        "parameter" => "sig:param",
        "self_parameter" => "sig:self_param",
        "type_parameters" => "sig:generic_params",
        "type_arguments" => "sig:generic_args",
        "where_clause" => "sig:where_clause",
        "where_predicate" => "sig:where_pred",
        "visibility_modifier" => "sig:visibility",
        "arguments" => "expr:arg_list",
        "field_initializer" | "shorthand_field_initializer" => "expr:record_field",
        "field_pattern" => "pat:record_field",
        "match_block" => "expr:match_arm_list",
        "enum_variant_list" => "item:variant_list",
        "field_declaration_list" => "item:record_field_list",
        "ordered_field_declaration_list" => "item:tuple_field_list",
        "declaration_list" => "item:item_list",
        "use_list" => "item:use_tree_list",
        "line_comment" | "block_comment" => "trivia:comment",
        _ => return None,
    })
}

fn ra_category(node: &SyntaxNode) -> Option<&'static str> {
    Some(match node.kind() {
        K::FN => "item:fn",
        K::STRUCT => "item:struct",
        K::ENUM => "item:enum",
        K::UNION => "item:union",
        K::TRAIT => "item:trait",
        K::IMPL => "item:impl",
        K::MODULE => "item:mod",
        K::USE => "item:use",
        K::CONST => "item:const",
        K::STATIC => "item:static",
        K::TYPE_ALIAS => "item:type_alias",
        K::MACRO_RULES => "item:macro_rules",
        K::EXTERN_CRATE => "item:extern_crate",
        K::EXTERN_BLOCK => "item:extern_block",
        K::VARIANT => "item:variant",
        K::RECORD_FIELD => "item:record_field",
        // `#[doc = include_str!(…)]`: tree-sitter keeps attribute arguments as tokens.
        K::MACRO_CALL if node.ancestors().skip(1).any(|a| a.kind() == K::ATTR) => return None,
        K::MACRO_CALL => "macro:call",
        // A macro_rules body (tree-sitter reads its rules); the attributes on the macro_rules keep their trees.
        K::TOKEN_TREE if node.ancestors().take_while(|a| a.kind() != K::ATTR).any(|a| a.kind() == K::MACRO_RULES) => {
            return None
        }
        K::TOKEN_TREE => "macro:token_tree",
        K::ATTR => "attr",
        K::CALL_EXPR => "expr:call",
        K::METHOD_CALL_EXPR => "expr:method_call",
        K::FIELD_EXPR => "expr:field",
        K::CLOSURE_EXPR => "expr:closure",
        K::IF_EXPR => "expr:if",
        K::MATCH_EXPR => "expr:match",
        K::MATCH_ARM => "expr:match_arm",
        K::WHILE_EXPR => "expr:while",
        K::LOOP_EXPR => "expr:loop",
        K::FOR_EXPR => "expr:for",
        K::LET_STMT => "stmt:let",
        K::LET_EXPR => "expr:let",
        K::BIN_EXPR => match let_chain_role(node) {
            LetChain::No => "expr:binary",
            LetChain::Top => "expr:let_chain",
            LetChain::Inner => return None,
        },
        K::PREFIX_EXPR => "expr:prefix",
        K::REF_EXPR => "expr:ref",
        K::INDEX_EXPR => "expr:index",
        K::RETURN_EXPR => "expr:return",
        K::BREAK_EXPR => "expr:break",
        K::CONTINUE_EXPR => "expr:continue",
        K::RECORD_EXPR => "expr:record",
        K::TUPLE_EXPR => "expr:tuple",
        K::ARRAY_EXPR => "expr:array",
        K::TRY_EXPR => "expr:try",
        K::AWAIT_EXPR => "expr:await",
        K::RANGE_EXPR => "expr:range",
        K::CAST_EXPR => "expr:cast",
        K::PAREN_EXPR => "expr:paren",
        K::STMT_LIST => "expr:block",
        // `#[must_use = "…"]`: tree-sitter keeps attribute arguments as tokens.
        K::LITERAL if node.ancestors().any(|a| a.kind() == K::ATTR) => return None,
        K::LITERAL => "expr:literal",
        K::TUPLE_STRUCT_PAT => "pat:tuple_struct",
        K::RECORD_PAT => "pat:record",
        K::TUPLE_PAT => "pat:tuple",
        K::PAREN_PAT => "pat:paren",
        K::SLICE_PAT => "pat:slice",
        K::RANGE_PAT => "pat:range",
        K::OR_PAT => {
            if node.parent().map(|p| p.kind()) == Some(K::OR_PAT) {
                return None;
            }
            "pat:or"
        }
        K::REF_TYPE => "type:ref",
        K::PTR_TYPE => "type:ptr",
        K::FN_PTR_TYPE => "type:fn_ptr",
        K::TUPLE_TYPE => "type:tuple",
        K::PAREN_TYPE => "type:paren",
        K::ARRAY_TYPE | K::SLICE_TYPE => "type:array_or_slice",
        K::DYN_TRAIT_TYPE => "type:dyn",
        K::IMPL_TRAIT_TYPE => "type:impl",
        K::NEVER_TYPE => "type:never",
        K::PARAM_LIST => "sig:param_list",
        K::PARAM if node.parent().and_then(|p| p.parent()).map(|g| g.kind()) == Some(K::CLOSURE_EXPR) => return None,
        // `fn(u8) -> bool`: tree-sitter lists an unnamed fn-pointer parameter as a bare type.
        K::PARAM if node.children().all(|c| !ast_pat(c.kind())) => return None,
        K::PARAM => "sig:param",
        K::SELF_PARAM => "sig:self_param",
        K::GENERIC_PARAM_LIST => "sig:generic_params",
        K::GENERIC_ARG_LIST => "sig:generic_args",
        K::WHERE_CLAUSE => "sig:where_clause",
        K::WHERE_PRED => "sig:where_pred",
        K::VISIBILITY => "sig:visibility",
        K::ARG_LIST => "expr:arg_list",
        K::RECORD_EXPR_FIELD => "expr:record_field",
        K::RECORD_PAT_FIELD => "pat:record_field",
        K::MATCH_ARM_LIST => "expr:match_arm_list",
        K::VARIANT_LIST => "item:variant_list",
        K::RECORD_FIELD_LIST => "item:record_field_list",
        K::TUPLE_FIELD_LIST => "item:tuple_field_list",
        K::ITEM_LIST | K::ASSOC_ITEM_LIST | K::EXTERN_ITEM_LIST => "item:item_list",
        K::USE_TREE_LIST => "item:use_tree_list",
        _ => return None,
    })
}

fn is_paren_tuple(node: &tree_sitter::Node) -> bool {
    let mut c = node.walk();
    let children: Vec<_> = node.children(&mut c).collect();
    let named: Vec<_> =
        children.iter().filter(|n| n.is_named() && !matches!(n.kind(), "line_comment" | "block_comment")).collect();
    // `(..)` is a tuple pattern, not a parenthesized rest pattern.
    named.len() == 1 && named[0].kind() != "remaining_field_pattern" && !children.iter().any(|n| n.kind() == ",")
}

fn ast_pat(kind: K) -> bool {
    ra_ap_syntax::ast::Pat::can_cast(kind)
}

enum LetChain {
    No,
    Top,
    Inner,
}

fn is_and(node: &SyntaxNode) -> bool {
    node.kind() == K::BIN_EXPR && node.children_with_tokens().any(|c| c.kind() == K::AMP2)
}

fn spine_has_let(node: &SyntaxNode) -> bool {
    node.children().any(|c| c.kind() == K::LET_EXPR || (is_and(&c) && spine_has_let(&c)))
}

/// tree-sitter reads `a && let P = e && b` as one flat let_chain; rust-analyzer as nested `&&` BIN_EXPRs. The
/// outermost `&&` of a chain holding a `let` corresponds to the let_chain; the inner ones have no counterpart.
fn let_chain_role(node: &SyntaxNode) -> LetChain {
    if !is_and(node) {
        return LetChain::No;
    }
    let mut top = node.clone();
    while let Some(parent) = top.parent() {
        if is_and(&parent) {
            top = parent;
        } else {
            break;
        }
    }
    if !spine_has_let(&top) {
        LetChain::No
    } else if &top == node {
        LetChain::Top
    } else if spine_has_let(node) {
        // An inner `&&` holding a `let` is flattened into tree-sitter's let_chain; one without stays a plain
        // binary_expression operand there too.
        LetChain::Inner
    } else {
        LetChain::No
    }
}

fn is_trivia(kind: K) -> bool {
    matches!(kind, K::WHITESPACE | K::COMMENT | K::DOC_COMMENT | K::OUTER_DOC_COMMENT | K::INNER_DOC_COMMENT)
}

/// rust-analyzer node span without leading attributes, doc comments and trivia, and without trailing trivia: the
/// span tree-sitter gives the same construct (tree-sitter keeps attributes as preceding siblings).
fn ra_span(node: &SyntaxNode) -> (usize, usize) {
    let children: Vec<_> = node.children_with_tokens().collect();
    let first = children
        .iter()
        .find(|c| !(is_trivia(c.kind()) || c.kind() == K::ATTR))
        .map(|c| usize::from(c.text_range().start()))
        .unwrap_or_else(|| usize::from(node.text_range().start()));
    let last = children
        .iter()
        .rev()
        .find(|c| !is_trivia(c.kind()))
        .map(|c| usize::from(c.text_range().end()))
        .unwrap_or_else(|| usize::from(node.text_range().end()));
    let mut span = (first, last.max(first));
    // A turbofish `::<T>`: tree-sitter's type_arguments starts at `<`.
    if node.kind() == K::GENERIC_ARG_LIST {
        if let Some(lt) = node.children_with_tokens().find(|c| c.kind() == K::L_ANGLE) {
            span.0 = usize::from(lt.text_range().start());
        }
    }
    span
}

fn collect_ra(root: &SyntaxNode, spans: &mut Spans, text: &str) {
    for event in root.preorder_with_tokens() {
        if let ra_ap_syntax::WalkEvent::Enter(element) = event {
            match element {
                NodeOrToken::Node(node) => {
                    if let Some(cat) = ra_category(&node) {
                        let mut span = ra_span(&node);
                        if cat == "expr:match_arm" {
                            span = strip_trailing_comma(text, span);
                        }
                        if cat == "macro:call" {
                            span = strip_trailing_semicolon(text, span);
                        }
                        spans.entry(cat).or_default().push(span);
                    }
                }
                NodeOrToken::Token(token) => {
                    if matches!(token.kind(), K::COMMENT | K::OUTER_DOC_COMMENT | K::INNER_DOC_COMMENT) {
                        let r = token.text_range();
                        spans
                            .entry("trivia:comment")
                            .or_default()
                            .push((usize::from(r.start()), trim_newline(text, usize::from(r.end()))));
                    }
                }
            }
        }
    }
}

fn strip_trailing_comma(text: &str, (s, e): (usize, usize)) -> (usize, usize) {
    if e > s && text.as_bytes()[e - 1] == b',' {
        (s, e - 1)
    } else {
        (s, e)
    }
}

fn strip_trailing_semicolon(text: &str, (s, e): (usize, usize)) -> (usize, usize) {
    if e > s && text.as_bytes()[e - 1] == b';' {
        (s, e - 1)
    } else {
        (s, e)
    }
}

fn trim_newline(text: &str, mut end: usize) -> usize {
    while end > 0 && matches!(text.as_bytes()[end - 1], b'\n' | b'\r') {
        end -= 1;
    }
    end
}

struct TsWalk<'a> {
    text: &'a str,
    spans: Spans,
    error_nodes: usize,
    missing_nodes: usize,
    first_error: Option<(usize, String, bool)>,
    nodes: usize,
    macro_bodies: Vec<(String, usize, usize)>,
    macro_rules_bytes: usize,
    fns: usize,
}

impl<'a> TsWalk<'a> {
    fn walk(&mut self, node: tree_sitter::Node, in_tt: bool, in_def: bool) {
        self.nodes += 1;
        if node.is_error() || node.is_missing() {
            if node.is_error() {
                self.error_nodes += 1;
            } else {
                self.missing_nodes += 1;
            }
            let at = node.start_byte();
            if self.first_error.as_ref().map(|(b, _, _)| at < *b).unwrap_or(true) {
                let kind = if node.is_missing() { format!("MISSING {}", node.kind()) } else { "ERROR".into() };
                self.first_error = Some((at, kind, in_tt));
            }
        }
        // Comments are extras anywhere; token trees nest inside token trees (both grammars keep them), except
        // in macro_rules definitions, which tree-sitter reads as rules and patterns.
        let counted_anyway = matches!(node.kind(), "line_comment" | "block_comment")
            || (node.kind() == "token_tree" && !in_def);
        if node.is_named() && (!in_tt || counted_anyway) {
            if let Some(cat) = ts_category(&node) {
                let mut span = self.ts_span(node);
                match cat {
                    "expr:block" => {
                        // `'label: { … }`: rust-analyzer's STMT_LIST starts at the brace.
                        let mut c = node.walk();
                        let brace = node.children(&mut c).find(|n| n.kind() == "{").map(|b| b.start_byte());
                        if let Some(brace) = brace {
                            span.0 = brace;
                        }
                    }
                    "expr:match_arm" => span = strip_trailing_comma(self.text, span),
                    "macro:call" => span = strip_trailing_semicolon(self.text, span),
                    "trivia:comment" => span.1 = trim_newline(self.text, span.1),
                    "type:dyn" | "type:impl" => {
                        // `dyn A + 'a`: tree-sitter nests the bounds as bounded_type(dynamic_type, +, …).
                        let mut top = node;
                        while let Some(parent) = top.parent() {
                            if parent.kind() == "bounded_type" && parent.child(0).map(|c| c.id()) == Some(top.id()) {
                                top = parent;
                            } else {
                                break;
                            }
                        }
                        span.1 = top.end_byte();
                    }
                    _ => {}
                }
                if !(cat == "macro:token_tree" && self.attr_meta_tree(node)) {
                    self.spans.entry(cat).or_default().push(span);
                }
            }
        }
        if node.is_named() && !in_tt {
            match node.kind() {
                "function_item" => self.fns += 1,
                "macro_definition" => self.macro_rules_bytes += node.end_byte() - node.start_byte(),
                "macro_invocation" => {
                    let name = node
                        .child_by_field_name("macro")
                        .map(|m| {
                            let t = &self.text[m.start_byte()..m.end_byte()];
                            t.rsplit("::").next().unwrap_or(t).to_string()
                        })
                        .unwrap_or_default();
                    let mut c = node.walk();
                    let body = node
                        .children(&mut c)
                        .find(|n| n.kind() == "token_tree")
                        .map(|tt| (tt.start_byte(), tt.end_byte()));
                    if let Some((s, e)) = body {
                        if e >= s + 2 {
                            self.macro_bodies.push((name, s + 1, e - 1));
                        }
                    }
                }
                _ => {}
            }
        }
        // Inside a macro body or attribute arguments, tree-sitter's nodes are tokens, not syntax.
        let child_in_tt = in_tt
            || matches!(node.kind(), "token_tree" | "macro_definition" | "token_tree_pattern" | "attribute");
        let child_in_def = in_def || node.kind() == "macro_definition";
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.walk(child, child_in_tt, child_in_def);
        }
    }

    /// Token trees rust-analyzer parses as attribute syntax rather than keeping as TOKEN_TREE: everything in
    /// `#[cfg(...)]`, the argument list of `#[cfg_attr(...)]` with the predicate's nested trees (the attributes
    /// after the predicate keep theirs), and the parentheses of `#[unsafe(...)]`.
    fn attr_meta_tree(&self, node: tree_sitter::Node) -> bool {
        let mut child = node;
        let mut up = node.parent();
        while let Some(n) = up {
            if n.kind() == "attribute" {
                let text = &self.text[n.start_byte()..n.end_byte()];
                let head: String = text.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
                let direct = child;
                return match head.as_str() {
                    "cfg" => true,
                    "unsafe" => direct.id() == node.id(),
                    "cfg_attr" => {
                        if direct.id() == node.id() {
                            return true;
                        }
                        let mut c = direct.walk();
                        let comma = direct.children(&mut c).find(|k| k.kind() == ",").map(|k| k.start_byte());
                        comma.map(|comma| node.start_byte() < comma).unwrap_or(true)
                    }
                    _ => false,
                };
            }
            child = n;
            up = n.parent();
        }
        false
    }

    /// The node's span without leading attributes and comments (rust-analyzer's spans are normalized the same
    /// way): tree-sitter puts the attributes of match arms, fields and parameters inside the node.
    fn ts_span(&self, node: tree_sitter::Node) -> (usize, usize) {
        let mut start = node.start_byte();
        let mut end = node.end_byte();
        let trivia = |n: &tree_sitter::Node| matches!(n.kind(), "line_comment" | "block_comment");
        let mut c = node.walk();
        let children: Vec<_> = node.children(&mut c).collect();
        let leading = children.first().map(|n| trivia(n) || n.kind() == "attribute_item").unwrap_or(false);
        if leading {
            if let Some(first) = children.iter().find(|n| !(trivia(n) || n.kind() == "attribute_item")) {
                start = first.start_byte();
            }
        }
        if children.last().map(|n| trivia(n)).unwrap_or(false) {
            if let Some(last) = children.iter().rev().find(|n| !trivia(n)) {
                end = last.end_byte();
            }
        }
        (start, end.max(start))
    }
}

#[derive(Default)]
struct FnCounter(usize);
impl<'ast> Visit<'ast> for FnCounter {
    fn visit_item_fn(&mut self, i: &'ast syn::ItemFn) {
        self.0 += 1;
        syn::visit::visit_item_fn(self, i);
    }
    fn visit_impl_item_fn(&mut self, i: &'ast syn::ImplItemFn) {
        self.0 += 1;
        syn::visit::visit_impl_item_fn(self, i);
    }
    fn visit_trait_item_fn(&mut self, i: &'ast syn::TraitItemFn) {
        self.0 += 1;
        syn::visit::visit_trait_item_fn(self, i);
    }
}

/// What syn reads a macro body as, and how many functions it holds when it reads as items.
fn classify_macro_body(name: &str, body: &str) -> (&'static str, usize) {
    use syn::parse::Parser;
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return ("empty", 0);
    }
    let tokens: proc_macro2::TokenStream = match body.parse() {
        Ok(t) => t,
        Err(_) => return ("unlexable", 0),
    };
    if trimmed.contains('$') {
        // A macro_rules transcriber fragment (`$x`), or a nested macro definition.
        return ("metavariables", 0);
    }
    let format_like = matches!(
        name,
        "format" | "print" | "println" | "eprint" | "eprintln" | "write" | "writeln" | "panic" | "assert"
            | "assert_eq" | "assert_ne" | "debug_assert" | "debug_assert_eq" | "debug_assert_ne" | "unreachable"
            | "todo" | "unimplemented" | "format_args" | "trace" | "debug" | "info" | "warn" | "error" | "bail"
            | "anyhow" | "ensure" | "vec" | "matches" | "dbg"
    );
    let exprs = syn::punctuated::Punctuated::<syn::Expr, syn::Token![,]>::parse_terminated;
    if format_like {
        if exprs.parse2(tokens.clone()).is_ok() {
            return ("expressions", 0);
        }
    }
    if let Ok(file) = syn::parse2::<syn::File>(tokens.clone()) {
        if !file.items.is_empty() {
            let mut counter = FnCounter::default();
            counter.visit_file(&file);
            return ("items", counter.0);
        }
    }
    if exprs.parse2(tokens.clone()).is_ok() {
        return ("expressions", 0);
    }
    if syn::Block::parse_within.parse2(tokens.clone()).is_ok() {
        return ("statements", 0);
    }
    // `vec![x; n]`, `matches!(e, Pat)`, `assert!(c, "..", args)` handled above; the rest is a DSL.
    ("custom", 0)
}

fn analyse(input: &Input, stats: &mut MacroStats, cats: &mut BTreeMap<String, Category>,
           mismatches: &mut Vec<Mismatch>, comparable: &mut usize) -> FileRecord {
    let text = std::fs::read_to_string(&input.path).unwrap_or_else(|e| panic!("{}: {e}", input.path));
    let lines = LineIndex::new(&text);

    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_rust::LANGUAGE.into()).unwrap();
    let t0 = Instant::now();
    let tree = parser.parse(&text, None).unwrap();
    let ts_ns = t0.elapsed().as_nanos();

    let t0 = Instant::now();
    let parse = SourceFile::parse(&text, edition(&input.edition));
    let ra_errors = parse.errors();
    let ra_ns = t0.elapsed().as_nanos();

    let t0 = Instant::now();
    let syn_result = syn::parse_file(&text);
    let syn_ns = t0.elapsed().as_nanos();

    let mut walk = TsWalk {
        text: &text,
        spans: Spans::new(),
        error_nodes: 0,
        missing_nodes: 0,
        first_error: None,
        nodes: 0,
        macro_bodies: vec![],
        macro_rules_bytes: 0,
        fns: 0,
    };
    walk.walk(tree.root_node(), false, false);
    let ra_root = parse.syntax_node();

    let ts_error = walk.first_error.as_ref().map(|(byte, kind, in_tt)| {
        let mut ra_context = vec![];
        if ra_errors.is_empty() {
            let offset = ra_ap_syntax::TextSize::from((*byte).min(text.len()) as u32);
            if let Some(token) = ra_root.token_at_offset(offset).right_biased() {
                let mut node = token.parent();
                while let Some(n) = node {
                    ra_context.push(format!("{:?}", n.kind()));
                    node = n.parent();
                }
            }
        }
        let end = text[*byte..].find('\n').map(|n| byte + n).unwrap_or(text.len()).min(byte + 80);
        let start = text[..*byte].rfind('\n').map(|n| n + 1).unwrap_or(0).max(byte.saturating_sub(40));
        let start = (start..=*byte).find(|i| text.is_char_boundary(*i)).unwrap_or(*byte);
        let end = (end..=text.len()).find(|i| text.is_char_boundary(*i)).unwrap_or(text.len());
        TsError {
            kind: kind.clone(),
            at: lines.pos(&text, *byte),
            in_token_tree: *in_tt,
            error_nodes: walk.error_nodes,
            missing_nodes: walk.missing_nodes,
            ra_context,
            snippet: text[start..end].to_string(),
        }
    });

    // Structural agreement on files rustc accepts and both grammars read without error.
    if input.accepted && ts_error.is_none() && ra_errors.is_empty() {
        *comparable += 1;
        let mut ra_spans = Spans::new();
        collect_ra(&ra_root, &mut ra_spans, &text);
        let mut keys: Vec<&'static str> = walk.spans.keys().chain(ra_spans.keys()).copied().collect();
        keys.sort();
        keys.dedup();
        for key in keys {
            let mut a = walk.spans.get(key).cloned().unwrap_or_default();
            let mut b = ra_spans.get(key).cloned().unwrap_or_default();
            a.sort();
            b.sort();
            let (mut i, mut j, mut matched) = (0, 0, 0);
            let mut only_ts = vec![];
            let mut only_ra = vec![];
            while i < a.len() && j < b.len() {
                match a[i].cmp(&b[j]) {
                    std::cmp::Ordering::Equal => {
                        matched += 1;
                        i += 1;
                        j += 1;
                    }
                    std::cmp::Ordering::Less => {
                        only_ts.push(a[i]);
                        i += 1;
                    }
                    std::cmp::Ordering::Greater => {
                        only_ra.push(b[j]);
                        j += 1;
                    }
                }
            }
            only_ts.extend_from_slice(&a[i..]);
            only_ra.extend_from_slice(&b[j..]);
            let c = cats.entry(key.to_string()).or_default();
            c.ts += a.len();
            c.ra += b.len();
            c.matched += matched;
            for (side, list) in [("tree-sitter", &only_ts), ("rust-analyzer", &only_ra)] {
                for &(s, e) in list.iter() {
                    let p = lines.pos(&text, s);
                    let snippet: String = text[s..e].chars().take(100).collect();
                    mismatches.push(Mismatch {
                        rel: input.rel.clone(),
                        category: key.to_string(),
                        side,
                        line: p.line,
                        column: p.column,
                        text: snippet,
                    });
                }
            }
        }
    }

    let mut macro_body_bytes = 0;
    let mut fns_in_macro_bodies = 0;
    for (name, s, e) in &walk.macro_bodies {
        let body = &text[*s..*e];
        let bytes = e - s;
        macro_body_bytes += bytes;
        let (class, fns) = classify_macro_body(name, body);
        fns_in_macro_bodies += fns;
        stats.invocations += 1;
        stats.body_bytes += bytes;
        *stats.classes.entry(class.into()).or_default() += 1;
        *stats.class_bytes.entry(class.into()).or_default() += bytes;
        let entry = stats.by_name.entry(name.clone()).or_default();
        entry.0 += 1;
        entry.1 += bytes;
        *entry.2.entry(class.into()).or_default() += 1;
    }

    let ra_first_error = ra_errors.first().map(|e| RaError {
        message: e.to_string(),
        at: lines.pos(&text, usize::from(e.range().start())),
    });
    let syn_error = syn_result.err().map(|e| {
        let start = e.span().start();
        SynError { message: e.to_string(), line: start.line, column: start.column + 1 }
    });

    FileRecord {
        rel: input.rel.clone(),
        edition: input.edition.clone(),
        bytes: text.len(),
        lines: lines.starts.len(),
        ts_nodes: walk.nodes,
        ts_error,
        ra_errors: ra_errors.len(),
        ra_first_error,
        syn_error,
        ts_ns,
        ra_ns,
        syn_ns,
        macro_body_bytes,
        macro_rules_bytes: walk.macro_rules_bytes,
        ts_fns: walk.fns,
        fns_in_macro_bodies,
    }
}

fn main() {
    let mut raw = String::new();
    std::io::stdin().read_to_string(&mut raw).unwrap();
    let inputs: Vec<Input> = serde_json::from_str(&raw).unwrap();
    let mut stats = MacroStats::default();
    let mut cats = BTreeMap::new();
    let mut mismatches = vec![];
    let mut comparable = 0;
    // Sequential on purpose: the per-parser timings are the throughput measurement.
    let files: Vec<FileRecord> = inputs
        .iter()
        .map(|input| analyse(input, &mut stats, &mut cats, &mut mismatches, &mut comparable))
        .collect();
    let out = Output { files, categories: cats, comparable_files: comparable, mismatches, macros: stats };
    serde_json::to_writer(std::io::stdout().lock(), &out).unwrap();
}
