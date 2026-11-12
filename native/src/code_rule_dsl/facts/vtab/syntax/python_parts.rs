//! Decode Python literal runs and retain each constant's source byte start.
use super::core_rr::py_unescape;

#[cfg(any(test, feature = "test-hooks"))]
pub fn with_offsets<N: Copy>(parts: &[(N, &str)]) -> Vec<(String, N, u8, usize)> {
    use crate::code_rule_dsl::{facts::core::{self, Built}, model::FileInput};
    use std::sync::Arc;
    if parts.is_empty() { return Vec::new(); }
    let mut source = "x = ".to_owned();
    let mut positions = Vec::new();
    for &(node, text) in parts {
        positions.push((source.len() as u32, node));
        source.push_str(text);
        source.push(' ');
    }
    source.push('\n');
    let input = FileInput::new("a.py".into(), "python".into(), Arc::new(source), None, None, "core").unwrap();
    let Built::Facts(facts) = core::build(&input).unwrap() else { panic!("literal test failed to parse") };
    let dialect = Dialect::new(crate::code_rule_dsl::facts::core::facts_tag(&facts)).unwrap();
    let p = Projected::new(0, Arc::new(facts), Some(&dialect), super::Shape::Core, false, None);
    let local = p.core.iter().position(|&id| matches!(dialect.name(&p.facts, id), "string" | "concatenated_string")).unwrap() as u32;
    from_tape(&p, &dialect, local, false).into_iter().map(|(v, at, spec, offset)| {
        // `at` is a node id: its low bits are a local id only under preorder ids.
        let at = p.local_of(at as u32).expect("a node of the document");
        let start = p.facts.start[p.core[at as usize] as usize];
        let node = positions.iter().find(|&&(s, _)| s == start).unwrap().1;
        (v, node, spec, offset)
    }).collect()
}

// Production constants follow parsed interpolation and format-expression boundaries.
// No expression lexer, per-literal parse, or source-sized character/offset array is needed.
use super::{Dialect, Projected};

type Piece = (String, i64, u8, usize);

pub(crate) fn from_tape(p: &Projected, dialect: &Dialect, local: u32, words: bool) -> Vec<Piece> {
    let mut reader = Constants { p, dialect, words, pieces: Vec::new() };
    let id = p.core[local as usize];
    let mut current = None;
    let mut formatted = false;
    let valid = if dialect.name(&p.facts, id) == "string" {
        reader.literal(id, &mut current, &mut formatted)
    } else {
        p.facts.children(id).iter().copied().filter(|&c| dialect.name(&p.facts, c) == "string")
            .all(|c| reader.literal(c, &mut current, &mut formatted))
    };
    if !valid { return Vec::new(); }
    reader.flush(&mut current, !formatted);
    reader.pieces
}

struct Constants<'a> {
    p: &'a Projected,
    dialect: &'a Dialect,
    words: bool,
    pieces: Vec<Piece>,
}

impl Constants<'_> {
    fn kind(&self, id: u32) -> &str { self.dialect.name(&self.p.facts, id) }

    fn append(&self, current: &mut Option<Piece>, value: String, owner: u32, start: u32, spec: u8) {
        if let Some((text, _, _, _)) = current { text.push_str(&value); }
        else {
            let at = self.p.global(self.p.local[owner as usize]);
            *current = Some((value, at, spec, (start - self.p.facts.start[owner as usize]) as usize));
        }
    }

    fn flush(&mut self, current: &mut Option<Piece>, keep_empty: bool) {
        if let Some(piece) = current.take() && (keep_empty || !piece.0.is_empty()) { self.pieces.push(piece); }
    }

    fn segment(&self, current: &mut Option<Piece>, owner: u32, start: u32, end: u32, raw: bool, braces: bool, spec: u8) {
        if start == end { return; }
        let source = &self.p.facts.source[start as usize..end as usize];
        let source = source.replace("\r\n", "\n").replace('\r', "\n");
        let value = if self.words { crate::code_rule_dsl::value::python::words::delimit(&source, raw) }
            else if raw { source } else { py_unescape(&source) };
        let value = if braces { value.replace("{{", "{").replace("}}", "}") } else { value };
        self.append(current, value, owner, start, spec);
    }

    fn literal(&mut self, id: u32, current: &mut Option<Piece>, formatted: &mut bool) -> bool {
        let p = self.p;
        let f = &p.facts;
        let text = f.text(id);
        let prefix: String = text.chars().take_while(char::is_ascii_alphabetic).collect::<String>().to_ascii_lowercase();
        if prefix.contains(['b', 't']) { return false; }
        let rest = &text[prefix.len()..];
        let Some(quote) = ["\"\"\"", "'''", "\"", "'"].into_iter().find(|q| rest.len() >= 2 * q.len() && rest.starts_with(q) && rest.ends_with(q)) else { return false; };
        let raw = prefix.contains('r');
        let begin = f.start[id as usize] + (prefix.len() + quote.len()) as u32;
        let end = f.end[id as usize] - quote.len() as u32;
        if !prefix.contains('f') {
            let mut own = None;
            self.segment(&mut own, id, begin, end, raw, false, 0);
            let value = own.map_or_else(String::new, |p| p.0);
            self.append(current, value, id, f.start[id as usize], 0);
            return true;
        }
        *formatted = true;
        let mut previous = begin;
        for &child in f.children(id) {
            if self.kind(child) != "interpolation" { continue; }
            self.segment(current, id, previous, f.start[child as usize], raw, true, 0);
            self.field(child, id, current, raw, 0);
            previous = f.end[child as usize];
        }
        self.segment(current, id, previous, end, raw, true, 0);
        true
    }

    fn field(&mut self, id: u32, owner: u32, current: &mut Option<Piece>, raw: bool, spec: u8) {
        let p = self.p;
        let f = &p.facts;
        let children = f.children(id);
        if f.tokens(id).iter().any(|t| self.dialect.atom_name(t.kind) == "=") {
            let end = children.iter().copied().find(|&c| matches!(self.kind(c), "type_conversion" | "format_specifier"))
                .map_or(f.end[id as usize] - 1, |c| f.start[c as usize]);
            // A debug label is source text, not a string-literal body: escapes stay literal.
            self.append(current, f.source[f.start[id as usize] as usize + 1..end as usize].to_owned(), owner, f.start[id as usize], spec);
        }
        self.flush(current, false);
        if let Some(&format) = children.iter().find(|&&c| self.kind(c) == "format_specifier") {
            self.format(format, owner, raw);
        }
    }

    fn format(&mut self, id: u32, owner: u32, raw: bool) {
        let p = self.p;
        let f = &p.facts;
        let mut current = None;
        let mut previous = f.start[id as usize] + 1; // skip ':'
        for &child in f.children(id) {
            if self.kind(child) != "format_expression" { continue; }
            self.segment(&mut current, owner, previous, f.start[child as usize], raw, false, 1);
            self.field(child, owner, &mut current, raw, 1);
            previous = f.end[child as usize];
        }
        self.segment(&mut current, owner, previous, f.end[id as usize], raw, false, 1);
        self.flush(&mut current, false);
    }
}

// tests: a test the engine hosts (it reads above this crate; tools/cratify/test-hosts.tsv)
