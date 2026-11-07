//! rrlab-facts: Soufflé facts for core.rr's tape relations, from tree-sitter trees.
//!
//!   rrlab-facts facts OUTDIR [--root DIR] [--lang LANG] FILE...   one document per FILE, ids 1.. in order
//!   rrlab-facts tree LANG < code                                  the parse tree as JSON (for pattern compiling)
//!   rrlab-facts graft OUTDIR --root DIR --next-id N --visit V < spec  trees for strings that hold code (`parsed`)
//!   rrlab-facts langs                                             the languages this build parses
//!
//! Every relation is a TSV file `<relation>.facts`, fields quoted as RFC 4180 (Soufflé's `rfc4180=true`), so texts
//! keep tabs and newlines. Nodes, tokens and spans share one id space; id 0 is core's `unset` value.
use std::collections::HashMap;
use std::fmt::Write as _;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use tree_sitter::{Language, Node, Parser};
use tree_sitter_language::LanguageFn;

unsafe extern "C" {
    fn tree_sitter_javascript() -> *const ();
    fn tree_sitter_typescript() -> *const ();
    fn tree_sitter_tsx() -> *const ();
    fn tree_sitter_python() -> *const ();
    fn tree_sitter_vue() -> *const ();
    fn tree_sitter_sql() -> *const ();
}

const LANGS: &[&str] = &["javascript", "typescript", "tsx", "python", "rust", "sql", "php", "vue"];

fn language(name: &str) -> Option<Language> {
    let f = match name {
        "javascript" => unsafe { LanguageFn::from_raw(tree_sitter_javascript) },
        "typescript" => unsafe { LanguageFn::from_raw(tree_sitter_typescript) },
        "tsx" => unsafe { LanguageFn::from_raw(tree_sitter_tsx) },
        "python" => unsafe { LanguageFn::from_raw(tree_sitter_python) },
        "vue" => unsafe { LanguageFn::from_raw(tree_sitter_vue) },
        "sql" => unsafe { LanguageFn::from_raw(tree_sitter_sql) },
        "rust" => tree_sitter_rust::LANGUAGE,
        "php" => tree_sitter_php::LANGUAGE_PHP,
        _ => return None,
    };
    Some(Language::new(f))
}

/// The grammar a file is parsed with, by extension; None for a file read as text only.
fn language_of(path: &str) -> Option<&'static str> {
    let ext = Path::new(path).extension().and_then(|e| e.to_str()).unwrap_or("");
    Some(match ext {
        "js" | "mjs" | "cjs" | "jsx" => "javascript",
        "ts" | "mts" | "cts" => "typescript",
        "tsx" => "tsx",
        "py" | "pyi" => "python",
        "rs" => "rust",
        "sql" => "sql",
        "php" => "php",
        "vue" => "vue",
        _ => return None,
    })
}

// ---- output -------------------------------------------------------------------------------------------------------

struct Out {
    dir: PathBuf,
    files: HashMap<&'static str, String>,
}

impl Out {
    fn row(&mut self, rel: &'static str, fields: &[&dyn std::fmt::Display]) {
        let buf = self.files.entry(rel).or_default();
        for (i, f) in fields.iter().enumerate() {
            if i > 0 {
                buf.push('\t');
            }
            let s = f.to_string();
            if s.contains(['\t', '\n', '\r', '"']) {
                buf.push('"');
                buf.push_str(&s.replace('"', "\"\""));
                buf.push('"');
            } else {
                buf.push_str(&s);
            }
        }
        buf.push('\n');
    }
    fn flush(&self) {
        std::fs::create_dir_all(&self.dir).unwrap();
        for rel in RELATIONS {
            let body = self.files.get(rel).map(String::as_str).unwrap_or("");
            std::fs::write(self.dir.join(format!("{rel}.facts")), body).unwrap();
        }
    }
}

/// Every relation written, so a relation with no rows still has its (empty) file.
const RELATIONS: &[&str] = &[
    "doc", "pkg", "path_part", "file_name", "node", "kind", "field", "child", "text", "op", "op_at", "docof", "line",
    "end_line", "col", "unset", "list_len", "value", "str", "strprefix", "name", "comment", "value_type", "source",
    "src_line", "line_count", "syntax_error", "first_token", "last_token", "token_before", "token_after",
    "next_sibling", "precedes", "spread", "visit_index", "ast_depth", "rrlab_parent", "rrlab_lang", "parsed",
    "str_part",
];

// ---- one document ---------------------------------------------------------------------------------------------

struct Doc<'a> {
    d: u32,
    src: &'a str,
    lang: &'a str,
    out: &'a mut Out,
    next_id: &'a mut u64,
    tokens: Vec<(u64, usize, usize)>,   // (id, start, end) in source order
    nodes: Vec<(u64, usize, usize)>,    // named nodes
    visit: u64,
    first_error: Option<(usize, String)>,
    ts_ids: HashMap<usize, u64>,        // tree-sitter node id -> fact id (a Vue host's guests look them up)
    fixed: Option<(usize, usize, usize, usize)>,  // a graft read from a decoded value: every node at (row, col, a, b)
    lone_cr: Option<Vec<usize>>,        // line starts, when a lone \r makes tree-sitter's rows differ from Ruff's
}

fn is_comment_kind(k: &str) -> bool {
    k.contains("comment")
}

/// Punctuation that op_at leaves out: brackets and separators are structure, not operators.
fn is_punct(t: &str) -> bool {
    matches!(t, "(" | ")" | "[" | "]" | "{" | "}" | "," | ";" | ":" | "." | "\"" | "'" | "`")
}

impl<'a> Doc<'a> {
    fn fresh(&mut self) -> u64 {
        *self.next_id += 1;
        *self.next_id
    }

    fn place(&mut self, id: u64, n: &Node) {
        let (a, b) = (n.start_byte(), n.end_byte());
        let text = &self.src[a..b];
        let (mut s, mut e) = (n.start_position(), n.end_position());
        if let Some(st) = &self.lone_cr {
            // Ruff's lines: a lone \r ends one too (tree-sitter counts only \n)
            (s, e) = (point_at(st, a), point_at(st, b));
        }
        if let Some((row, col, _, _)) = self.fixed {
            s = tree_sitter::Point { row, column: col };
            e = s;
        }
        // end_line: the line of the last character, so a node ending at column 0 ends on the line before.
        let end_row = if e.column == 0 && e.row > s.row { e.row - 1 } else { e.row };
        let d = self.d;
        self.out.row("text", &[&id, &text]);
        self.out.row("line", &[&id, &(s.row + 1)]);
        self.out.row("end_line", &[&id, &(end_row + 1)]);
        self.out.row("col", &[&id, &s.column]);
        self.out.row("docof", &[&id, &d]);
    }

    fn walk(&mut self, n: Node, parent: Option<u64>, depth: u32) -> u64 {
        let id = self.fresh();
        let k = n.kind();
        let (a, b) = (n.start_byte(), n.end_byte());
        self.place(id, &n);
        let d = self.d;
        let (na, nb) = self.fixed.map(|f| (f.2, f.3)).unwrap_or((a, b));
        self.out.row("node", &[&d, &id, &k, &na, &nb]);
        self.out.row("kind", &[&id, &k]);
        let vi = self.visit;
        self.visit += 1;
        self.out.row("visit_index", &[&id, &vi]);
        self.out.row("ast_depth", &[&id, &depth]);
        if let Some(p) = parent {
            self.out.row("rrlab_parent", &[&id, &p]);
        }
        self.nodes.push((id, a, b));
        self.ts_ids.insert(n.id(), id);
        if (n.is_error() || n.is_missing()) && self.first_error.is_none() {
            let msg = if n.is_missing() { format!("missing {k}") } else { "syntax error".to_string() };
            self.first_error = Some((n.start_position().row + 1, msg));
        }
        if n.child_count() == 0 && !is_comment_kind(k) && b > a {
            self.tokens.push((id, a, b));
        }
        let mut named: Vec<u64> = Vec::new();
        let mut ops: Vec<String> = Vec::new();
        for i in 0..n.child_count() as u32 {
            let c = n.child(i).unwrap();
            let field = n.field_name_for_child(i);
            if c.is_named() {
                let cid = self.walk(c, Some(id), depth + 1);
                let idx = named.len();
                self.out.row("child", &[&id, &cid, &idx]);
                if let Some(f) = field {
                    self.out.row("field", &[&id, &f, &cid]);
                }
                named.push(cid);
            } else {
                let t = c.kind();
                if c.end_byte() > c.start_byte() {
                    let tid = self.fresh();
                    self.place(tid, &c);
                    self.tokens.push((tid, c.start_byte(), c.end_byte()));
                }
                if c.is_missing() && self.first_error.is_none() {
                    self.first_error = Some((c.start_position().row + 1, format!("missing {t}")));
                }
                self.out.row("op", &[&id, &t]);
                if !is_punct(t) {
                    // Python's two-word operators are one token: "not in", "is not".
                    let two = self.lang == "python" && matches!((ops.last().map(String::as_str), t), (Some("not"), "in") | (Some("is"), "not"));
                    if two {
                        let l = ops.pop().unwrap();
                        ops.push(format!("{l} {t}"));
                    } else {
                        ops.push(t.to_string());
                    }
                }
            }
        }
        for (i, t) in ops.iter().enumerate() {
            self.out.row("op_at", &[&id, &i, &t]);
            if t.contains(' ') {
                self.out.row("op", &[&id, &t]);
            }
        }
        for w in named.windows(2) {
            self.out.row("next_sibling", &[&w[0], &w[1]]);
        }
        for i in 0..named.len() {
            for j in i + 1..named.len() {
                self.out.row("precedes", &[&named[i], &named[j]]);
            }
        }
        self.literal(id, &n);
        if self.lang == "python" && self.fixed.is_none() {
            let top = k == "concatenated_string"
                || k == "string" && n.parent().map(|p| p.kind()) != Some("concatenated_string");
            if top {
                self.str_parts(id, &n, &named);
            }
        }
        id
    }

    /// str_part(lit, i, v, at, spec): the str constants CPython builds from literal lit (a string, or adjacent ones
    /// as a concatenated_string): plain runs join across adjacent literals; an f-string's interpolations split
    /// them; a format spec's literal runs are pieces of their own with spec = 1. Bytes and t-strings give none.
    fn str_parts(&mut self, id: u64, n: &Node, named: &[u64]) {
        let parts: Vec<(u64, &str)> = if n.kind() == "string" {
            vec![(id, &self.src[n.start_byte()..n.end_byte()])]
        } else {
            let mut v = Vec::new();
            let mut j = 0;
            for c in kids(*n).into_iter().filter(|c| c.is_named()) {
                if c.kind() == "string" {
                    v.push((named[j], &self.src[c.start_byte()..c.end_byte()]));
                }
                j += 1;
            }
            v
        };
        let mut pieces: Vec<(String, u64, u8)> = Vec::new();
        let mut cur: Option<(String, u64)> = None;
        let mut any_f = false;
        for &(sid, t) in &parts {
            let pre: String = t.chars().take_while(|c| c.is_ascii_alphabetic()).collect::<String>().to_lowercase();
            if pre.contains('b') || pre.contains('t') {
                return;
            }
            let rest = &t[pre.len()..];
            let Some(q) = ["\"\"\"", "'''", "\"", "'"].into_iter()
                .find(|q| rest.len() >= 2 * q.len() && rest.starts_with(q) && rest.ends_with(q)) else { return };
            let body = &rest[q.len()..rest.len() - q.len()];
            let raw = pre.contains('r');
            let dec = |x: &str| if raw { x.to_string() } else { py_unescape(x) };
            let push = |cur: &mut Option<(String, u64)>, v: &str| match cur {
                Some((s, _)) => s.push_str(v),
                None => *cur = Some((v.to_string(), sid)),
            };
            if !pre.contains('f') {
                push(&mut cur, &dec(body));
                continue;
            }
            any_f = true;
            let b: Vec<char> = body.chars().collect();
            let mut i = 0;
            let mut run = String::new();
            while i < b.len() {
                let c = b[i];
                if c == '\\' && !raw && i + 1 < b.len() {
                    // an escape, kept whole for py_unescape: \N{…} holds braces that are not a field
                    let mut j = i + 2;
                    if b[i + 1] == 'N' && b.get(i + 2) == Some(&'{') {
                        while j < b.len() && b[j - 1] != '}' {
                            j += 1;
                        }
                    }
                    run.extend(&b[i..j]);
                    i = j;
                } else if (c == '{' || c == '}') && b.get(i + 1) == Some(&c) {
                    run.push_str(if c == '{' { "{{" } else { "}}" });
                    i += 2;
                } else if c == '{' {
                    let lit = dec(&run).replace("{{", "{").replace("}}", "}");
                    run.clear();
                    // the field: an expression to a depth-0 `!`, `:` or `}`
                    let mut j = i + 1;
                    let mut depth = 0i32;
                    let mut quote: Option<char> = None;
                    while j < b.len() {
                        let d = b[j];
                        if let Some(qc) = quote {
                            if d == qc { quote = None; }
                        } else if d == '\'' || d == '"' {
                            quote = Some(d);
                        } else if "([{".contains(d) {
                            depth += 1;
                        } else if ")]}".contains(d) {
                            if depth == 0 { break; }
                            depth -= 1;
                        } else if depth == 0 && (d == ':' || d == '!' && b.get(j + 1) != Some(&'=')) {
                            break;
                        }
                        j += 1;
                    }
                    let expr: String = b[i + 1..j.min(b.len())].iter().collect();
                    let te = expr.trim_end();
                    let debug = te.ends_with('=') && !["==", "!=", "<=", ">="].iter().any(|o| te.ends_with(o));
                    let mut text = lit;
                    if debug {
                        text.push_str(&expr);
                    }
                    if !text.is_empty() {
                        push(&mut cur, &text);
                    }
                    if let Some((v, at)) = cur.take() {
                        if !v.is_empty() { pieces.push((v, at, 0)); }
                    }
                    // a conversion, then a format spec up to the field's closing brace
                    if j < b.len() && b[j] == '!' {
                        while j < b.len() && b[j] != ':' && b[j] != '}' { j += 1; }
                    }
                    if j < b.len() && b[j] == ':' {
                        j += 1;
                        let mut spec = String::new();
                        let mut depth = 0i32;
                        while j < b.len() {
                            let d = b[j];
                            if d == '{' {
                                if depth == 0 && !spec.is_empty() {
                                    pieces.push((dec(&spec), sid, 1));
                                    spec.clear();
                                }
                                depth += 1;
                            } else if d == '}' {
                                if depth == 0 { break; }
                                depth -= 1;
                            } else if depth == 0 {
                                spec.push(d);
                            }
                            j += 1;
                        }
                        if !spec.is_empty() {
                            pieces.push((dec(&spec), sid, 1));
                        }
                    }
                    i = j + 1;
                } else {
                    run.push(c);
                    i += 1;
                }
            }
            if !run.is_empty() {
                let lit = dec(&run).replace("{{", "{").replace("}}", "}");
                push(&mut cur, &lit);
            }
        }
        if let Some((v, at)) = cur.take() {
            if !any_f || !v.is_empty() {
                pieces.push((v, at, 0));
            }
        }
        for (i, (v, at, spec)) in pieces.iter().enumerate() {
            self.out.row("str_part", &[&id, &i, v, at, spec]);
        }
    }

    /// value, str, strprefix, name, comment, spread, value_type: what is derivable from the node alone.
    fn literal(&mut self, id: u64, n: &Node) {
        let k = n.kind();
        let text = &self.src[n.start_byte()..n.end_byte()];
        let lang = self.lang;
        if is_comment_kind(k) {
            let body = strip_comment(text);
            self.out.row("comment", &[&id, &body.trim()]);
            return;
        }
        if matches!(k, "spread_element" | "list_splat" | "dictionary_splat" | "spread_expression" | "variadic_unpacking") {
            self.out.row("spread", &[&id]);
        }
        if k.contains("identifier") || matches!(k, "member_expression" | "attribute" | "dotted_name"
            | "scoped_identifier" | "field_expression" | "qualified_name" | "nested_identifier" | "name") {
            let s: String = text.chars().filter(|c| !c.is_whitespace()).collect();
            if !s.is_empty() && s.chars().all(|c| c.is_alphanumeric() || matches!(c, '_' | '$' | '.' | ':' | '\\')) {
                self.out.row("name", &[&id, &s]);
            }
        }
        let val = |out: &mut Out, v: &str, t: &str| {
            out.row("value", &[&id, &v]);
            out.row("value_type", &[&v, &t]);
        };
        match (lang, k) {
            (_, "string" | "string_literal" | "raw_string_literal" | "encapsed_string" | "interpreted_string_literal") => {
                // A literal with interpolations (a Python f-string with `{x}`, a PHP "…$x…") has no str or value:
                // its text is not one constant (core.rr; the engine's str is the same).
                let mut cursor = n.walk();
                let interpolated = n.named_children(&mut cursor).any(|c| match (lang, k) {
                    ("python", _) => c.kind() == "interpolation",
                    ("php", "encapsed_string") => !matches!(c.kind(), "string_content" | "escape_sequence"),
                    _ => false,
                });
                if let Some((prefix, body)) = string_contents(lang, text) {
                    self.out.row("strprefix", &[&id, &prefix]);
                    let t = if lang == "python" { "str" } else { "string" };
                    if !interpolated {
                        self.out.row("str", &[&id, &body]);
                        if !prefix.contains('f') {
                            val(self.out, &body, t);
                        }
                    }
                }
            }
            ("javascript" | "typescript" | "tsx", "template_string") => {
                // a template with no ${…}: its value is its cooked text
                if !text.contains("${") && text.len() >= 2 {
                    let body = unescape(&text[1..text.len() - 1]);
                    self.out.row("str", &[&id, &body]);
                    val(self.out, &body, "string");
                }
            }
            ("javascript" | "typescript" | "tsx", "number") => {
                if let Some(v) = js_number(text) {
                    val(self.out, &v, "number");
                }
            }
            ("python", "integer") => {
                if let Some(v) = int_value(&text.replace('_', "").to_lowercase().trim_end_matches(['l', 'j'])) {
                    val(self.out, &v, "int");
                }
            }
            ("python", "float") => val(self.out, text, "float"),
            ("rust" | "php", "integer_literal" | "integer") => {
                let t: String = text.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
                if let Some(v) = int_value(&t.replace('_', "").to_lowercase()) {
                    val(self.out, &v, "int");
                }
            }
            (_, "true" | "false") => val(self.out, k, "boolean"),
            ("python", "none") => val(self.out, "None", "NoneType"),
            ("javascript" | "typescript" | "tsx", "null" | "undefined") => val(self.out, k, k),
            ("rust", "boolean_literal") => val(self.out, text, "bool"),
            _ => {}
        }
    }
}

fn strip_comment(t: &str) -> &str {
    let t = t.trim_end();
    if let Some(r) = t.strip_prefix("/*") {
        return r.strip_suffix("*/").unwrap_or(r);
    }
    if let Some(r) = t.strip_prefix("<!--") {
        return r.strip_suffix("-->").unwrap_or(r);
    }
    for m in ["//", "#", "--"] {
        if let Some(r) = t.strip_prefix(m) {
            return r;
        }
    }
    t
}

/// (prefix, unescaped contents) of a string literal's source text.
fn string_contents(lang: &str, text: &str) -> Option<(String, String)> {
    let p: String = text.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
    let rest = &text[p.len()..];
    let raw = p.to_lowercase().contains('r');
    let rest = if lang == "rust" && raw { rest.trim_matches('#') } else { rest };
    for q in ["\"\"\"", "'''", "\"", "'", "`"] {
        if rest.len() >= 2 * q.len() && rest.starts_with(q) && rest.ends_with(q) {
            let body = &rest[q.len()..rest.len() - q.len()];
            let v = if raw { body.to_string() } else if lang == "python" { py_unescape(body) } else { unescape(body) };
            return Some((p.to_lowercase(), v));
        }
    }
    None
}

/// A Python str literal's escapes, as the language reference gives them: unknown escapes keep their backslash,
/// octal up to three digits, \x, \u, \U, a backslash-newline joins lines. \N{name} is kept as written (no name
/// table here).
fn py_unescape(s: &str) -> String {
    let c: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    let hex = |c: &[char], i: usize, n: usize| -> Option<char> {
        if i + n > c.len() { return None; }
        let h: String = c[i..i + n].iter().collect();
        u32::from_str_radix(&h, 16).ok().and_then(char::from_u32)
    };
    while i < c.len() {
        if c[i] != '\\' || i + 1 >= c.len() {
            out.push(c[i]);
            i += 1;
            continue;
        }
        let e = c[i + 1];
        i += 2;
        match e {
            'n' => out.push('\n'),
            't' => out.push('\t'),
            'r' => out.push('\r'),
            'a' => out.push('\u{7}'),
            'b' => out.push('\u{8}'),
            'f' => out.push('\u{c}'),
            'v' => out.push('\u{b}'),
            '\\' | '\'' | '"' => out.push(e),
            '\n' => {}
            '\r' => {
                if c.get(i) == Some(&'\n') { i += 1; }
            }
            '0'..='7' => {
                let mut v = e.to_digit(8).unwrap();
                let mut k = 0;
                while k < 2 && i < c.len() && c[i].is_digit(8) {
                    v = v * 8 + c[i].to_digit(8).unwrap();
                    i += 1;
                    k += 1;
                }
                out.push(char::from_u32(v).unwrap_or('\u{fffd}'));
            }
            'x' | 'u' | 'U' => {
                let n = match e { 'x' => 2, 'u' => 4, _ => 8 };
                match hex(&c, i, n) {
                    Some(ch) => { out.push(ch); i += n; }
                    None => { out.push('\\'); out.push(e); }
                }
            }
            _ => { out.push('\\'); out.push(e); }
        }
    }
    out
}

fn unescape(s: &str) -> String {
    let mut out = String::new();
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('0') => out.push('\0'),
            Some('b') => out.push('\u{8}'),
            Some('f') => out.push('\u{c}'),
            Some('v') => out.push('\u{b}'),
            Some('x') => {
                let h: String = it.by_ref().take(2).collect();
                if let Some(c) = u32::from_str_radix(&h, 16).ok().and_then(char::from_u32) {
                    out.push(c)
                }
            }
            Some('u') => {
                let h: String = if it.peek() == Some(&'{') {
                    it.next();
                    it.by_ref().take_while(|c| *c != '}').collect()
                } else {
                    it.by_ref().take(4).collect()
                };
                if let Some(c) = u32::from_str_radix(&h, 16).ok().and_then(char::from_u32) {
                    out.push(c)
                }
            }
            Some('\n') => {}
            Some(o) => out.push(o),
            None => out.push('\\'),
        }
    }
    out
}

fn int_value(t: &str) -> Option<String> {
    let (radix, digits) = if let Some(r) = t.strip_prefix("0x") {
        (16, r)
    } else if let Some(r) = t.strip_prefix("0o") {
        (8, r)
    } else if let Some(r) = t.strip_prefix("0b") {
        (2, r)
    } else {
        (10, t)
    };
    u128::from_str_radix(digits, radix).ok().map(|v| v.to_string())
}

/// A JavaScript number literal's value as Number.prototype.toString writes it (integers exactly; others as Rust's
/// shortest round-trip form, which matches for ordinary literals).
fn js_number(t: &str) -> Option<String> {
    let t = t.replace('_', "");
    if let Some(b) = t.strip_suffix('n') {
        return int_value(&b.to_lowercase());
    }
    let l = t.to_lowercase();
    if l.starts_with("0x") || l.starts_with("0o") || l.starts_with("0b") {
        return int_value(&l);
    }
    if l.len() > 1 && l.starts_with('0') && l.chars().all(|c| c.is_ascii_digit()) {
        return if l.chars().any(|c| c == '8' || c == '9') { Some(l.trim_start_matches('0').to_string()) }
            else { u128::from_str_radix(&l[1..], 8).ok().map(|v| v.to_string()) };
    }
    let f: f64 = l.parse().ok()?;
    Some(if f.fract() == 0.0 && f.abs() < 1e21 { format!("{}", f as i128) } else { format!("{f}") })
}

/// doc's module name: Python's dotted module (a leading `src/` dropped, `__init__` is its package), else the path
/// without its extension, as tsx/names resolves modules.
fn module_name(lang: &str, path: &str) -> (String, String) {
    let stem = match path.rfind('.') {
        Some(i) if !path[i..].contains('/') => &path[..i],
        _ => path,
    };
    if lang != "python" {
        return (stem.to_string(), String::new());
    }
    let s = stem.strip_prefix("src/").unwrap_or(stem);
    let mut parts: Vec<&str> = s.split('/').collect();
    if parts.last() == Some(&"__init__") {
        parts.pop();
        let m = parts.join(".");
        return (m.clone(), m);
    }
    let m = parts.join(".");
    let pkg = parts[..parts.len().saturating_sub(1)].join(".");
    (m, pkg)
}

/// Line starts when a Python source has a lone \r, which ends a line for Ruff but not for tree-sitter; None when
/// tree-sitter's rows are already Ruff's (or the language is not Python).
fn lone_cr_starts(src: &str, lang: &str) -> Option<Vec<usize>> {
    let b = src.as_bytes();
    if lang != "python" || !(0..b.len()).any(|i| b[i] == b'\r' && b.get(i + 1) != Some(&b'\n')) {
        return None;
    }
    let mut v = vec![0];
    for (i, &c) in b.iter().enumerate() {
        if c == b'\n' || (c == b'\r' && b.get(i + 1) != Some(&b'\n')) {
            v.push(i + 1);
        }
    }
    Some(v)
}

fn point_at(starts: &[usize], at: usize) -> tree_sitter::Point {
    let row = starts.partition_point(|&x| x <= at) - 1;
    tree_sitter::Point { row, column: at - starts[row] }
}

fn python_lines(s: &str) -> Vec<&str> {
    // The line terminators of CPython's tokenizer and Ruff's line index: \r\n, \n and a lone \r. A form feed
    // or \v is whitespace inside a line, so line numbers agree with the tools' (str.splitlines() would not).
    let mut out = Vec::new();
    let mut start = 0;
    let b: Vec<(usize, char)> = s.char_indices().collect();
    let mut i = 0;
    while i < b.len() {
        let (at, c) = b[i];
        if matches!(c, '\n' | '\r') {
            out.push(&s[start..at]);
            let mut next = at + c.len_utf8();
            if c == '\r' && i + 1 < b.len() && b[i + 1].1 == '\n' {
                next += 1;
                i += 1;
            }
            start = next;
        }
        i += 1;
    }
    if start < s.len() {
        out.push(&s[start..]);
    }
    out
}

fn document(out: &mut Out, next_id: &mut u64, d: u32, path: &str, lang: Option<&str>, src: &str) {
    let rel = path.trim_start_matches("./");
    let (module, pkg) = module_name(lang.unwrap_or(""), rel);
    out.row("doc", &[&d, &rel, &module]);
    if !pkg.is_empty() {
        out.row("pkg", &[&d, &pkg]);
    }
    for part in rel.split('/') {
        out.row("path_part", &[&d, &part]);
    }
    out.row("file_name", &[&d, &rel.rsplit('/').next().unwrap_or(rel)]);
    // Source text: the file as one span, and each physical line.
    *next_id += 1;
    let sp = *next_id;
    out.row("source", &[&d, &sp]);
    out.row("text", &[&sp, &src]);
    out.row("line", &[&sp, &1]);
    out.row("col", &[&sp, &0]);
    out.row("docof", &[&sp, &d]);
    let lines: Vec<&str> = if lang == Some("python") { python_lines(src) } else {
        let mut v: Vec<&str> = src.split('\n').collect();
        if src.ends_with('\n') { v.pop(); }
        v
    };
    out.row("end_line", &[&sp, &lines.len().max(1)]);
    for (i, l) in lines.iter().enumerate() {
        *next_id += 1;
        let id = *next_id;
        let l = l.strip_suffix('\r').unwrap_or(l);
        out.row("src_line", &[&d, &(i + 1), &id]);
        out.row("text", &[&id, &l]);
        out.row("line", &[&id, &(i + 1)]);
        out.row("end_line", &[&id, &(i + 1)]);
        out.row("col", &[&id, &0]);
        out.row("docof", &[&id, &d]);
    }
    out.row("line_count", &[&d, &lines.len()]);
    let Some(lang) = lang else { return };
    out.row("rrlab_lang", &[&d, &lang]);
    let mut parser = Parser::new();
    parser.set_language(&language(lang).unwrap()).unwrap();
    let tree = parser.parse(src, None).expect("parse");
    let mut doc = Doc { d, src, lang, out, next_id, tokens: vec![], nodes: vec![], visit: 0, first_error: None,
        ts_ids: HashMap::new(), fixed: None, lone_cr: lone_cr_starts(src, lang) };
    doc.walk(tree.root_node(), None, 0);
    if let Some((l, msg)) = doc.first_error.take() {
        doc.out.row("syntax_error", &[&d, &l, &msg]);
    }
    token_facts(&mut doc);
    if lang != "vue" {
        return;
    }
    // A Vue component's code: each <script> block, and each template expression, is a guest tree in the same
    // document, at the component's own positions, and core's parsed(host, grammar, root) names it.
    let guests = vue_guests(tree.root_node(), src);
    let mut visit = doc.visit;
    let ids = std::mem::take(&mut doc.ts_ids);
    let (out, next_id) = (doc.out, doc.next_id);
    for g in guests {
        let Some(&host) = ids.get(&g.host) else { continue };
        let buf = g.buffer(src);
        let mut parser = Parser::new();
        parser.set_language(&language(g.lang).unwrap()).unwrap();
        let Some(tree) = parser.parse(&buf, None) else { continue };
        let mut gd = Doc { d, src: &buf, lang: g.lang, out: &mut *out, next_id: &mut *next_id, tokens: vec![],
            nodes: vec![], visit, first_error: None, ts_ids: HashMap::new(), fixed: None, lone_cr: None };
        let root = gd.walk(tree.root_node(), None, 0);
        gd.out.row("parsed", &[&host, &g.grammar, &root]);
        token_facts(&mut gd);
        visit = gd.visit;
    }
}

/// One piece of a Vue component's code, parsed on its own: the host node it is read from (a tree-sitter id), its
/// grammar label for `parsed`, the dialect it is parsed with, its byte range, and the text written around it.
struct VueGuest {
    host: usize,
    grammar: &'static str,
    lang: &'static str,
    lo: usize,
    hi: usize,
    wrap: Vec<(usize, &'static str)>,
}

impl VueGuest {
    /// The component's text with everything outside [lo, hi) blanked (line breaks kept, so rows and columns are
    /// the component's), and the wrapper written into the blanks: `(` … `)` makes an expression of `{ a: 1 }`,
    /// `(` … `)=>0` makes parameters of a slot's `{ item }`.
    fn buffer(&self, src: &str) -> String {
        let b = src.as_bytes();
        let mut buf: Vec<u8> = b.iter().enumerate()
            .map(|(i, &c)| if (self.lo..self.hi).contains(&i) || c == b'\n' || c == b'\r' { c } else { b' ' })
            .collect();
        buf.extend_from_slice(b"        ");
        for &(at, t) in &self.wrap {
            for (j, c) in t.bytes().enumerate() {
                buf[at + j] = c;
            }
        }
        String::from_utf8(buf).unwrap_or_default()
    }
}

/// The guests of a Vue file: its scripts (as their `lang` says) and its template's expressions, read the way
/// vue-eslint-parser reads them. Grammar labels:
///   "javascript" | "typescript" | "tsx"   a <script> block's raw text
///   "vue-expression"   a directive's value or a {{ }}'s contents, as one expression: program > ( … )
///   "vue-statements"   a v-on value, as statements: @click="a = 1; go()"
///   "vue-params"       v-slot, #name, slot-scope, scope: parameters, program > (…)=>0
///   "vue-for-alias"    v-for's `(item, , i)`, as an array pattern: program > let [item, , i]=0, holes kept as
///                      vue-eslint-parser keeps them;  "vue-for-source": its `in items`, an expression
fn vue_guests(root: Node, src: &str) -> Vec<VueGuest> {
    let text = |n: Node| &src[n.start_byte()..n.end_byte()];
    // before(lo): a blank before lo that is not a line break, for an opening wrapper
    let bytes = src.as_bytes();
    let before = |lo: usize| -> usize {
        let mut p = lo.saturating_sub(1);
        while p > 0 && matches!(bytes[p], b'\n' | b'\r') {
            p -= 1;
        }
        p
    };
    // run_before(lo, k): the start of k bytes before lo with no line break in them, for a wrapper written there
    let run_before = |lo: usize, k: usize| -> usize {
        let mut p = lo.saturating_sub(k);
        while p > 0 && bytes[p..p + k].iter().any(|&c| c == b'\n' || c == b'\r') {
            p -= 1;
        }
        p
    };
    let mut scripts = Vec::new();
    let mut all = Vec::new();
    let mut stack = vec![root];
    while let Some(n) = stack.pop() {
        all.push(n);
        for c in kids(n).into_iter().rev() {
            stack.push(c);
        }
    }
    let mut ts = "javascript";
    for &n in &all {
        if n.kind() != "script_element" {
            continue;
        }
        let mut lang = "javascript";
        for c in kids(n) {
            if c.kind() != "start_tag" {
                continue;
            }
            for a in kids(c) {
                let name = kids(a).into_iter().find(|x| x.kind() == "attribute_name").map(text);
                if a.kind() == "attribute" && name == Some("lang") {
                    lang = match value_of(a).map(text) {
                        Some("ts") => "typescript",
                        Some("tsx") => "tsx",
                        _ => "javascript",
                    };
                }
            }
        }
        if lang != "javascript" {
            ts = "typescript";
        }
        if let Some(raw) = kids(n).into_iter().find(|c| c.kind() == "raw_text") {
            scripts.push(VueGuest { host: raw.id(), grammar: lang, lang, lo: raw.start_byte(), hi: raw.end_byte(), wrap: vec![] });
        }
    }
    let mut out = scripts;
    let expr = |v: Node, grammar: &'static str| VueGuest {
        host: v.id(), grammar, lang: ts, lo: v.start_byte(), hi: v.end_byte(),
        wrap: vec![(before(v.start_byte()), "("), (v.end_byte(), ")")],
    };
    let params = |v: Node, grammar: &'static str, lo: usize, hi: usize| VueGuest {
        host: v.id(), grammar, lang: ts, lo, hi, wrap: vec![(before(lo), "("), (hi, ")=>0")],
    };
    for &n in &all {
        match n.kind() {
            "interpolation" => {
                if let Some(raw) = kids(n).into_iter().find(|c| c.kind() == "raw_text") {
                    if !text(raw).trim().is_empty() {
                        out.push(expr(raw, "vue-expression"));
                    }
                }
            }
            "attribute" => {
                let name = kids(n).into_iter().find(|x| x.kind() == "attribute_name").map(text);
                if let (Some("slot-scope" | "scope"), Some(v)) = (name, value_of(n)) {
                    if !text(v).trim().is_empty() {
                        out.push(params(v, "vue-params", v.start_byte(), v.end_byte()));
                    }
                }
            }
            "directive_attribute" => {
                // a dynamic argument, :[key]="…", is an expression too
                for c in kids(n).into_iter().filter(|c| c.kind() == "dynamic_directive_value") {
                    if let Some(iv) = kids(c).into_iter().find(|x| x.kind() == "dynamic_directive_inner_value") {
                        out.push(expr(iv, "vue-expression"));
                    }
                }
                let Some(v) = value_of(n) else { continue };
                let body = text(v);
                if body.trim().is_empty() {
                    continue;
                }
                let cs = kids(n);
                let name = match cs.iter().find(|c| c.kind() == "directive_name").map(|c| text(*c)) {
                    Some(x) => x,
                    None => match cs.first().map(|c| c.kind()) {
                        Some("@") => "v-on",
                        Some("#") => "v-slot",
                        _ => "v-bind",
                    },
                };
                match name {
                    "v-on" => out.push(VueGuest { host: v.id(), grammar: "vue-statements", lang: ts,
                        lo: v.start_byte(), hi: v.end_byte(), wrap: vec![] }),
                    "v-slot" => out.push(params(v, "vue-params", v.start_byte(), v.end_byte())),
                    "v-for" => {
                        let Some((alias, source)) = vue_for_split(body) else {
                            out.push(expr(v, "vue-expression"));
                            continue;
                        };
                        let base = v.start_byte();
                        let (mut alo, mut ahi) = (base + alias.0, base + alias.1);
                        if alo < ahi && bytes[alo] == b'(' && bytes[ahi - 1] == b')' {
                            alo += 1;
                            ahi -= 1;
                        }
                        if alo < ahi {
                            out.push(VueGuest { host: v.id(), grammar: "vue-for-alias", lang: ts, lo: alo, hi: ahi,
                                wrap: vec![(run_before(alo, 5), "let ["), (ahi, "]=0")] });
                        }
                        let (slo, shi) = (base + source.0, base + source.1);
                        out.push(VueGuest { host: v.id(), grammar: "vue-for-source", lang: ts, lo: slo, hi: shi,
                            wrap: vec![(before(slo), "("), (shi, ")")] });
                    }
                    _ => out.push(expr(v, "vue-expression")),
                }
            }
            _ => {}
        }
    }
    out
}

fn kids<'t>(n: Node<'t>) -> Vec<Node<'t>> {
    (0..n.child_count() as u32).filter_map(|i| n.child(i)).collect()
}

/// An attribute's value node: `x` of `a="x"` or of `a=x`.
fn value_of<'t>(n: Node<'t>) -> Option<Node<'t>> {
    for c in kids(n) {
        if c.kind() == "attribute_value" {
            return Some(c);
        }
        if c.kind() == "quoted_attribute_value" {
            return kids(c).into_iter().find(|v| v.kind() == "attribute_value");
        }
    }
    None
}

/// v-for's value split as Vue's forAliasRE does, /([\s\S]*?)\s+(?:in|of)\s+(\S[\s\S]*)/: the alias range
/// (trimmed) and the source range (trimmed), as offsets into the value.
fn vue_for_split(v: &str) -> Option<((usize, usize), (usize, usize))> {
    let b = v.as_bytes();
    let ws = |c: u8| c.is_ascii_whitespace();
    for i in 0..b.len() {
        if !ws(b[i]) {
            continue;
        }
        let mut j = i;
        while j < b.len() && ws(b[j]) {
            j += 1;
        }
        if j + 2 > b.len() || !(&b[j..j + 2] == b"in" || &b[j..j + 2] == b"of") {
            continue;
        }
        let mut k = j + 2;
        if k >= b.len() || !ws(b[k]) {
            continue;
        }
        while k < b.len() && ws(b[k]) {
            k += 1;
        }
        if k >= b.len() {
            continue;
        }
        let mut alo = 0;
        while alo < i && ws(b[alo]) {
            alo += 1;
        }
        let mut shi = b.len();
        while shi > k && ws(b[shi - 1]) {
            shi -= 1;
        }
        return Some(((alo, i), (k, shi)));
    }
    None
}

/// Tokens: first/last per node, before/after per node and per token (binary search over the sorted tokens).
fn token_facts(doc: &mut Doc) {
    let mut toks = std::mem::take(&mut doc.tokens);
    toks.sort_by_key(|t| (t.1, t.2));
    let starts: Vec<usize> = toks.iter().map(|t| t.1).collect();
    let ends: Vec<usize> = toks.iter().map(|t| t.2).collect();
    let mut items: Vec<(u64, usize, usize)> = std::mem::take(&mut doc.nodes);
    items.extend(toks.iter().copied());
    for (id, a, b) in items {
        let first = starts.partition_point(|&s| s < a);
        if first < toks.len() && toks[first].2 <= b && doc_is_node(&toks, id) {
            let last = ends.partition_point(|&e| e <= b) - 1;
            doc.out.row("first_token", &[&id, &toks[first].0]);
            doc.out.row("last_token", &[&id, &toks[last].0]);
        }
        // before: the last token ending at or before a, other than id itself.
        let k = ends.partition_point(|&e| e <= a);
        if k > 0 && toks[k - 1].0 != id {
            doc.out.row("token_before", &[&id, &toks[k - 1].0]);
        }
        let j = starts.partition_point(|&s| s < b);
        if j < toks.len() && toks[j].0 != id {
            doc.out.row("token_after", &[&id, &toks[j].0]);
        }
    }
}

/// first/last_token are for nodes; a token that is not also a node gets none.
fn doc_is_node(_toks: &[(u64, usize, usize)], _id: u64) -> bool {
    true
}

// ---- grafts: strings parsed as code, on request ----------------------------------------------------------------

fn named_kids<'t>(n: Node<'t>) -> Vec<Node<'t>> {
    kids(n).into_iter().filter(|c| c.is_named() && !c.is_extra()).collect()
}

/// n's one named child, if it is of kind k.
fn only<'t>(n: Node<'t>, k: &str) -> Option<Node<'t>> {
    let c = named_kids(n);
    (c.len() == 1 && c[0].kind() == k).then(|| c[0])
}

/// One graft per JSON line of `spec`: {"d", "host", "grammar", "lang", "path", "lo", "hi"} parses bytes [lo, hi)
/// of the file in place (the rest blanked, line breaks kept, `(` … `)` written around it), so the tree has the
/// file's own lines and columns; {"d", "host", "grammar", "lang", "value", "line", "col", "a", "b"} parses a
/// decoded value (an escaped or concatenated string) with every node at the host's place. Ids follow
/// `next_id`; `parsed(host, grammar, root)` names the tree: the expression, or the module when it does not
/// parse as one expression (so the ERROR nodes are below the root).
fn graft(out: &mut Out, next_id: &mut u64, visit: u64, root_dir: &Path, spec: &str) {
    let mut visit = visit;
    let mut files: HashMap<String, String> = HashMap::new();
    for line in spec.lines().filter(|l| !l.trim().is_empty()) {
        let g: serde_json::Value = serde_json::from_str(line).expect("graft spec");
        let num = |k: &str| g[k].as_u64().unwrap_or(0) as usize;
        let (d, host) = (num("d") as u32, num("host") as u64);
        let grammar = g["grammar"].as_str().unwrap_or("").to_string();
        let lang = g["lang"].as_str().unwrap_or("python").to_string();
        let Some(language) = language(&lang) else { continue };
        let (buf, fixed) = if let Some(v) = g["value"].as_str() {
            (format!("({v})"), Some((num("line").saturating_sub(1), num("col"), num("a"), num("b"))))
        } else {
            let path = g["path"].as_str().unwrap_or("").to_string();
            let src = files.entry(path.clone()).or_insert_with(|| {
                String::from_utf8_lossy(&std::fs::read(root_dir.join(&path)).unwrap_or_default()).into_owned()
            });
            let (lo, hi) = (num("lo"), num("hi"));
            if lo == 0 || hi >= src.len() || lo > hi {
                continue;
            }
            let b = src.as_bytes();
            let mut v: Vec<u8> = b.iter().enumerate()
                .map(|(i, &c)| if (lo..hi).contains(&i) || c == b'\n' || c == b'\r' { c } else { b' ' }).collect();
            v[lo - 1] = b'(';
            v[hi] = b')';
            (String::from_utf8(v).unwrap_or_default(), None)
        };
        let mut parser = Parser::new();
        parser.set_language(&language).unwrap();
        let Some(tree) = parser.parse(&buf, None) else { continue };
        let lang_s: &str = &lang;
        let mut gd = Doc { d, src: &buf, lang: lang_s, out: &mut *out, next_id: &mut *next_id, tokens: vec![],
            nodes: vec![], visit, first_error: None, ts_ids: HashMap::new(), fixed, lone_cr: if fixed.is_none() { lone_cr_starts(&buf, lang_s) } else { None } };
        let module = tree.root_node();
        let module_id = gd.walk(module, None, 0);
        let mut root = module_id;
        if !module.has_error() {
            if let Some(p) = only(module, "expression_statement").and_then(|s| only(s, "parenthesized_expression")) {
                let c = named_kids(p);
                if c.len() == 1 {
                    root = gd.ts_ids[&c[0].id()];
                }
            }
        }
        gd.out.row("parsed", &[&host, &grammar, &root]);
        token_facts(&mut gd);
        visit = gd.visit;
        // the wrapper module is not a Python module (no docstring, no scope of its own): kind graft_module
        let (from, to) = (format!("{module_id}\tmodule\n"), format!("{module_id}\tgraft_module\n"));
        if let Some(k) = out.files.get_mut("kind") {
            *k = k.replace(&from, &to);
        }
        if let Some(n) = out.files.get_mut("node") {
            let (from, to) = (format!("\t{module_id}\tmodule\t"), format!("\t{module_id}\tgraft_module\t"));
            *n = n.replace(&from, &to);
        }
    }
}

// ---- tree as JSON (for the pattern compiler) ----------------------------------------------------------------------

fn tree_json(n: Node, src: &str, field: Option<&str>, s: &mut String) {
    let text = &src[n.start_byte()..n.end_byte()];
    write!(s, "{{\"kind\":{},\"named\":{},\"field\":{},\"text\":{},\"start\":{},\"end\":{},\"error\":{},\"extra\":{},\"children\":[",
        serde_json::to_string(n.kind()).unwrap(), n.is_named(),
        field.map(|f| serde_json::to_string(f).unwrap()).unwrap_or("null".into()),
        serde_json::to_string(text).unwrap(), n.start_byte(), n.end_byte(), n.is_error() || n.is_missing(), n.is_extra()).unwrap();
    for i in 0..n.child_count() as u32 {
        if i > 0 {
            s.push(',');
        }
        tree_json(n.child(i).unwrap(), src, n.field_name_for_child(i), s);
    }
    s.push_str("]}");
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("langs") => println!("{}", LANGS.join(" ")),
        Some("tree") => {
            let lang = args.get(2).expect("tree LANG");
            let mut src = String::new();
            std::io::stdin().read_to_string(&mut src).unwrap();
            let mut parser = Parser::new();
            parser.set_language(&language(lang).unwrap_or_else(|| panic!("unknown language {lang}"))).unwrap();
            let tree = parser.parse(&src, None).unwrap();
            let mut s = String::new();
            tree_json(tree.root_node(), &src, None, &mut s);
            std::io::stdout().write_all(s.as_bytes()).unwrap();
        }
        Some("facts") => {
            let dir = PathBuf::from(args.get(2).expect("facts OUTDIR"));
            let mut root = PathBuf::from(".");
            let mut lang: Option<String> = None;
            let mut files = Vec::new();
            let mut i = 3;
            while i < args.len() {
                match args[i].as_str() {
                    "--root" => { root = PathBuf::from(&args[i + 1]); i += 2; }
                    "--lang" => { lang = Some(args[i + 1].clone()); i += 2; }
                    f => { files.push(f.to_string()); i += 1; }
                }
            }
            let mut out = Out { dir, files: HashMap::new() };
            out.row("unset", &[&0]);
            out.row("text", &[&0, &""]);
            let mut next_id = 0u64;
            for (k, f) in files.iter().enumerate() {
                let src = std::fs::read(root.join(f)).unwrap_or_else(|e| panic!("{f}: {e}"));
                let src = String::from_utf8_lossy(&src).into_owned();
                let l = lang.clone().or_else(|| language_of(f).map(str::to_string));
                let l = l.filter(|l| language(l).is_some());
                document(&mut out, &mut next_id, k as u32 + 1, f, l.as_deref(), &src);
            }
            out.flush();
        }
        Some("graft") => {
            // graft OUTDIR --root DIR --next-id N --visit V < spec: trees for strings that hold code
            let dir = PathBuf::from(args.get(2).expect("graft OUTDIR"));
            let (mut root, mut next_id, mut visit) = (PathBuf::from("."), 0u64, 0u64);
            let mut i = 3;
            while i + 1 < args.len() {
                match args[i].as_str() {
                    "--root" => root = PathBuf::from(&args[i + 1]),
                    "--next-id" => next_id = args[i + 1].parse().expect("--next-id N"),
                    "--visit" => visit = args[i + 1].parse().expect("--visit V"),
                    a => panic!("graft: unknown argument {a}"),
                }
                i += 2;
            }
            let mut spec = String::new();
            std::io::stdin().read_to_string(&mut spec).unwrap();
            let mut out = Out { dir, files: HashMap::new() };
            graft(&mut out, &mut next_id, visit, &root, &spec);
            out.flush();
            println!("{next_id}");
        }
        _ => {
            eprintln!("usage: rrlab-facts facts OUTDIR [--root DIR] [--lang LANG] FILE... | tree LANG < code | langs");
            std::process::exit(2);
        }
    }
}
