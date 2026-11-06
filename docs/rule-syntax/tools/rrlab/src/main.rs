//! rrlab-facts: Soufflé facts for core.rr's tape relations, from tree-sitter trees.
//!
//!   rrlab-facts facts OUTDIR [--root DIR] [--lang LANG] FILE...   one document per FILE, ids 1.. in order
//!   rrlab-facts tree LANG < code                                  the parse tree as JSON (for pattern compiling)
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
    "next_sibling", "precedes", "spread", "visit_index", "ast_depth", "rrlab_parent", "rrlab_lang",
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
        let (s, e) = (n.start_position(), n.end_position());
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
        self.out.row("node", &[&d, &id, &k, &a, &b]);
        self.out.row("kind", &[&id, &k]);
        let vi = self.visit;
        self.visit += 1;
        self.out.row("visit_index", &[&id, &vi]);
        self.out.row("ast_depth", &[&id, &depth]);
        if let Some(p) = parent {
            self.out.row("rrlab_parent", &[&id, &p]);
        }
        self.nodes.push((id, a, b));
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
        id
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
                if let Some((prefix, body)) = string_contents(lang, text) {
                    self.out.row("str", &[&id, &body]);
                    self.out.row("strprefix", &[&id, &prefix]);
                    let t = if lang == "python" { "str" } else { "string" };
                    if !prefix.contains('f') {
                        val(self.out, &body, t);
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
            let v = if raw { body.to_string() } else { unescape(body) };
            return Some((p.to_lowercase(), v));
        }
    }
    None
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
    let mut doc = Doc { d, src, lang, out, next_id, tokens: vec![], nodes: vec![], visit: 0, first_error: None };
    doc.walk(tree.root_node(), None, 0);
    if let Some((l, msg)) = doc.first_error.take() {
        doc.out.row("syntax_error", &[&d, &l, &msg]);
    }
    // Tokens: first/last per node, before/after per node and per token (binary search over the sorted tokens).
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
        _ => {
            eprintln!("usage: rrlab-facts facts OUTDIR [--root DIR] [--lang LANG] FILE... | tree LANG < code | langs");
            std::process::exit(2);
        }
    }
}
