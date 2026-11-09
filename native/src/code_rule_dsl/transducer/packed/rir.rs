//! `rir`, the Rerail IR's text form (list step 18; 05 Phase 4 (h), L2; P `spec-tooling.md`
//! §2.2.1–2.2.2): one stable format every tool reads and writes. A program prints as UTF-8 lines,
//! its tables in index order, then its words, with no addresses and no hashes but content
//! digests, so two dumps diff cleanly. The assembler reads it back: `assemble(disassemble(p))` is
//! `p` (equal digests; the digest covers every component a program is built from), and it builds
//! through the verifier (`Program::validate`, which every constructor runs), so a hand-written
//! program cannot bypass it.
//!
//! ```text
//! program sha256:<plain digest> ir=7 grammar=python@<digest> caps=interp,micro
//!   mode set=0 query=0
//!   string 0 "match"
//!   label 0 s0
//!   long 0 100000
//!   predicate 0 regex s3
//!   assert 0 kinds L1 [identifier "("] negated=0
//!   directive 0 set s4 -
//!   pattern (internal call (terminal "(" any "(") (single "A" named) (ellipsis -))
//!   0000 KIND function_definition
//!   0001 FIELD name
//!   0002 FORK @0004
//!   0003 CAPTURE L0  ; match
//! ```
//!
//! - **Names.** With a grammar (`Names`) a kind prints as its name, bare where named and quoted
//!   where anonymous, and a field as its name, wherever the name maps back to that one id (the
//!   grammar's other names for an id, aliases, print as `#id`, as does a derived class). The
//!   header names the grammar and its digest (over its kind and field tables); the assembler
//!   reads names with that grammar and refuses a dump whose grammar's tables have changed.
//!   Without one every kind and field is `#id`.
//! - **Capabilities** (`caps`): what the program's shape admits, statically. `interp`, always (the
//!   reference); `micro` where it is a micro walk (`Micro::of`: host label walks and Metal's
//!   device passes); `pattern` where it carries an ast-grep pattern (the host's matcher). Whether
//!   the CPU or Metal JIT compiles a program is decided by compiling it (`compilation`), not
//!   from its text, so it is not a capability here.
//! - Text after `;` is a comment. A header of `program -` takes no digest (a hand-written
//!   program); otherwise the assembled program's digest must equal it, and a stated `caps` must
//!   be the program's.

use super::*;
use crate::code_rule_dsl::language::{GrammarDialect, KindCatalog};
use crate::code_rule_dsl::tape::DERIVED_CLASS_BASE;
use std::collections::HashMap as Map;

/// The format's version: the program digest's (`tempo-program-v7`).
pub(in crate::code_rule_dsl) const IR: u32 = 7;

/// The opcodes' mnemonics.
const OPCODES: &[(u8, &str)] = &[
    (STOP, "STOP"),
    (KIND, "KIND"),
    (FIELD, "FIELD"),
    (FLAGS, "FLAGS"),
    (TEXT, "TEXT"),
    (SIZE, "SIZE"),
    (ASSERT, "ASSERT"),
    (FIRST, "FIRST"),
    (LAST, "LAST"),
    (ABSENT_FIELD, "ABSENT_FIELD"),
    (SUPERTYPE, "SUPERTYPE"),
    (CHILD, "CHILD"),
    (NEXT, "NEXT"),
    (PARENT, "PARENT"),
    (DESCEND, "DESCEND"),
    (FOLLOWING, "FOLLOWING"),
    (SAVE_CURSOR, "SAVE_CURSOR"),
    (RESTORE_CURSOR, "RESTORE_CURSOR"),
    (CAPTURE, "CAPTURE"),
    (REPEAT_BEGIN, "REPEAT_BEGIN"),
    (REPEAT_BUMP, "REPEAT_BUMP"),
    (REPEAT_END, "REPEAT_END"),
    (FORK, "FORK"),
    (GOTO, "GOTO"),
    (EMIT, "EMIT"),
    (TOKEN, "TOKEN"),
    (SWITCH, "SWITCH"),
    (WORD_IS, "WORD"),
    (OPEN, "OPEN"),
    (CLOSE, "CLOSE"),
    (LEAF, "LEAF"),
    (NODE_FIELD, "NODE_FIELD"),
    (NODE_SUPER, "NODE_SUPER"),
    (PUSH, "PUSH"),
    (RETURN, "RETURN"),
    (ACCEPT, "ACCEPT"),
    (SPAN, "SPAN"),
    (EMPTY, "EMPTY"),
    (MARK, "MARK"),
    (ROW, "ROW"),
    (RETAG, "RETAG"),
    (MERGE, "MERGE"),
    (WRAP, "WRAP"),
];

/// How an opcode's operand prints.
#[derive(Clone, Copy, PartialEq)]
enum Operand {
    Number,
    Target,
    Label,
    Predicate,
    Assert,
    Kind,
    Field,
    /// A string: `sN`.
    String,
    /// A switch table: `TN`.
    Switch,
}

fn operand(opcode: u8) -> Operand {
    match opcode {
        FORK | GOTO | PUSH => Operand::Target,
        WORD_IS => Operand::String,
        SWITCH => Operand::Switch,
        OPEN | LEAF | NODE_SUPER | RETAG | MERGE | WRAP => Operand::Kind,
        NODE_FIELD => Operand::Field,
        CAPTURE | EMIT => Operand::Label,
        TEXT => Operand::Predicate,
        ASSERT => Operand::Assert,
        KIND | SUPERTYPE => Operand::Kind,
        FIELD | ABSENT_FIELD => Operand::Field,
        _ => Operand::Number,
    }
}

/// A grammar's kind and field names, as `rir` prints and reads them.
pub(in crate::code_rule_dsl) struct Names {
    dialect: GrammarDialect,
    catalog: KindCatalog,
    /// (name, named) to the one id it prints for.
    kinds: Map<(&'static str, bool), u16>,
    digest: String,
}

/// A dialect's name in a header: its file label.
fn dialect_name(dialect: GrammarDialect) -> String {
    crate::code_rule_dsl::language::label(dialect).map_or_else(|| format!("{dialect:?}").to_lowercase(), str::to_owned)
}

impl Names {
    pub(in crate::code_rule_dsl) fn of(dialect: GrammarDialect) -> Self {
        let catalog = dialect.kinds();
        let mut kinds = Map::new();
        let mut digest = Sha256::new();
        digest.update(b"rir-grammar-v1\0");
        for id in 0..catalog.kind_count() as u16 {
            let named = catalog.kind_is_named(id);
            if let Some(name) = catalog.kind_name(id) {
                kinds.entry((name, named)).or_insert(id);
                digest.update(id.to_le_bytes());
                digest.update([u8::from(named)]);
                digest.update(name.as_bytes());
                digest.update([0]);
            }
        }
        for id in 1..=catalog.field_count() as u16 {
            if let Some(name) = catalog.field_name(id) {
                digest.update(id.to_le_bytes());
                digest.update(name.as_bytes());
                digest.update([0]);
            }
        }
        let digest = format!("{:x}", digest.finalize())[..16].to_owned();
        Self { dialect, catalog, kinds, digest }
    }

    /// The header's `grammar=` value.
    fn header(&self) -> String {
        format!("{}@{}", dialect_name(self.dialect), self.digest)
    }

    fn bare(name: &str) -> bool {
        name.chars().next().is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    }

    /// A kind as it prints.
    fn kind(&self, id: u16) -> String {
        let named = self.catalog.kind_is_named(id);
        match self.catalog.kind_name(id) {
            Some(name) if self.kinds.get(&(name, named)) == Some(&id) && (!named || Self::bare(name)) => {
                if named { name.to_owned() } else { quote(name.as_bytes()) }
            }
            _ => format!("#{id}"),
        }
    }

    fn field(&self, id: u16) -> String {
        match self.catalog.field_name(id) {
            Some(name) if self.catalog.field_id(name) == Some(id) && Self::bare(name) => name.to_owned(),
            _ => format!("#{id}"),
        }
    }
}

/// A kind as it prints with `names` (a derived class, or without a grammar, as `#id`).
pub(in crate::code_rule_dsl) fn kind(names: Option<&Names>, id: u16) -> String {
    match names {
        Some(names) if id < DERIVED_CLASS_BASE => names.kind(id),
        _ => format!("#{id}"),
    }
}

fn field(names: Option<&Names>, id: u16) -> String {
    names.map_or_else(|| format!("#{id}"), |names| names.field(id))
}

/// Kinds as a list.
pub(in crate::code_rule_dsl) fn kinds(names: Option<&Names>, ids: &[u16]) -> String {
    format!("[{}]", ids.iter().map(|&id| kind(names, id)).collect::<Vec<_>>().join(" "))
}

/// `bytes` as a quoted string: printable ASCII as itself, `"` and `\` escaped, the rest `\xNN`.
pub(in crate::code_rule_dsl) fn quote(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() + 2);
    out.push('"');
    for &byte in bytes {
        match byte {
            b'"' => out.push_str("\\\""),
            b'\\' => out.push_str("\\\\"),
            0x20..=0x7e => out.push(byte as char),
            _ => {
                let _ = write!(out, "\\x{byte:02x}");
            }
        }
    }
    out.push('"');
    out
}

fn flag(value: bool) -> u8 {
    u8::from(value)
}

fn strings(items: &[u16]) -> String {
    format!("[{}]", items.iter().map(|item| format!("s{item}")).collect::<Vec<_>>().join(" "))
}

/// The program's capabilities (the module's `caps`).
pub(in crate::code_rule_dsl) fn caps(program: &Program) -> String {
    let mut caps = vec!["interp"];
    if Micro::of(program).is_some() {
        caps.push("micro");
    }
    if program.ast_pattern.is_some() {
        caps.push("pattern");
    }
    caps.join(",")
}

fn pattern(out: &mut String, node: &PackedPattern, names: Option<&Names>) {
    let name = |name: &Option<String>| name.as_ref().map_or("-".to_owned(), |name| quote(name.as_bytes()));
    let named = |named: bool| if named { "named" } else { "any" };
    match node {
        PackedPattern::Terminal { text, named: is_named, kind: id } => {
            let _ = write!(out, "(terminal {} {} {})", kind(names, *id), named(*is_named), quote(text.as_bytes()));
        }
        PackedPattern::Single { name: single, named: is_named } => {
            let _ = write!(out, "(single {} {})", name(single), named(*is_named));
        }
        PackedPattern::Ellipsis(ellipsis) => {
            let _ = write!(out, "(ellipsis {})", name(ellipsis));
        }
        PackedPattern::Internal { kind: id, children } => {
            let _ = write!(out, "(internal {}", kind(names, *id));
            for child in children {
                out.push(' ');
                pattern(out, child, names);
            }
            out.push(')');
        }
    }
}

/// Block program `at`'s rows: tables, operations, outputs, extracts, delay, and the tree section (`tree_rows`).
pub(super) fn block_rows(out: &mut String, at: usize, block: &block::BlockProgram) {
    let hex = |bytes: &[u8]| bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
    for (t, table) in block.tables.iter().enumerate() {
        let _ = writeln!(out, "  block {at} table {t} lo={} hi={}", hex(&table.lo), hex(&table.hi));
    }
    for (value, op) in block.ops.iter().enumerate() {
        let _ = writeln!(out, "  block {at} %{value} = {}", op.text());
    }
    // `block N rounds %c %n …`: one fixed point's carries, each with its next.
    for group in &block.rounds {
        let pairs: Vec<String> = group.iter().map(|(c, x)| format!("%{c} %{x}")).collect();
        let _ = writeln!(out, "  block {at} rounds {}", pairs.join(" "));
    }
    for (k, value) in block.outputs.iter().enumerate() {
        match block.output_delay(k) {
            d if d == block.delay => {
                let _ = writeln!(out, "  block {at} output %{value}");
            }
            d => {
                let _ = writeln!(out, "  block {at} output %{value} delay {d}");
            }
        }
    }
    for value in &block.extracts {
        let _ = writeln!(out, "  block {at} extract %{value}");
    }
    for value in &block.totals {
        let _ = writeln!(out, "  block {at} total %{value}");
    }
    for value in &block.masks {
        let _ = writeln!(out, "  block {at} mask %{value}");
    }
    if block.delay != 0 {
        let _ = writeln!(out, "  block {at} delay {}", block.delay);
    }
}

pub(in crate::code_rule_dsl) fn disassemble(program: &Program, names: Option<&Names>) -> String {
    let mut out = String::new();
    let grammar = names.map_or("none".to_owned(), Names::header);
    let _ = writeln!(out, "program sha256:{} ir={IR} grammar={grammar} caps={}", program.plain_digest(), caps(program));
    let _ = writeln!(out, "  mode set={} query={}", flag(program.set_mode), flag(program.tree_sitter_query));
    for (at, string) in program.strings.iter().enumerate() {
        let _ = writeln!(out, "  string {at} {}", quote(string));
    }
    for (at, label) in program.labels.iter().enumerate() {
        let _ = writeln!(out, "  label {at} s{label}");
    }
    for (at, long) in program.longs.iter().enumerate() {
        let _ = writeln!(out, "  long {at} {long}");
    }
    for (at, s) in program.switches.iter().enumerate() {
        switch_rows(&mut out, at, s);
    }
    for (at, predicate) in program.predicates.iter().enumerate() {
        let (kind, source) = match predicate {
            TextPredicate::Equal(source) => ("equal", source),
            TextPredicate::Prefix(source) => ("prefix", source),
            TextPredicate::Regex { source, .. } => ("regex", source),
            TextPredicate::LuaPattern { source, .. } => ("lua", source),
        };
        let _ = writeln!(out, "  predicate {at} {kind} s{source}");
    }
    for (at, block) in program.block_programs.iter().enumerate() {
        block_rows(&mut out, at, block);
    }
    for (at, predicate) in program.capture_predicates.iter().enumerate() {
        let body = match predicate {
            CapturePredicate::Equal { left, right, negated, any } => {
                format!("equal L{left} L{right} negated={} any={}", flag(*negated), flag(*any))
            }
            CapturePredicate::Text { capture, predicate, negated, any } => {
                format!("text L{capture} P{predicate} negated={} any={}", flag(*negated), flag(*any))
            }
            CapturePredicate::AnyOf { capture, strings: of, negated, any } => {
                format!("any-of L{capture} {} negated={} any={}", strings(of), flag(*negated), flag(*any))
            }
            CapturePredicate::Kind { capture, kind: id, negated, any } => {
                format!("kind L{capture} {} negated={} any={}", kind(names, *id), flag(*negated), flag(*any))
            }
            CapturePredicate::Kinds { capture, kinds: ids, negated } => format!("kinds L{capture} {} negated={}", kinds(names, ids), flag(*negated)),
            CapturePredicate::Contains { capture, strings: of, negated, any } => {
                format!("contains L{capture} {} negated={} any={}", strings(of), flag(*negated), flag(*any))
            }
            CapturePredicate::Relation { capture, kinds: ids, ancestor, negated } => {
                format!("relation L{capture} {} ancestor={} negated={}", kinds(names, ids), flag(*ancestor), flag(*negated))
            }
            CapturePredicate::SameLine { left, right, negated } => format!("same-line L{left} L{right} negated={}", flag(*negated)),
        };
        let _ = writeln!(out, "  assert {at} {body}");
    }
    let value = |value: &Option<u16>| value.map_or("-".to_owned(), |value| format!("s{value}"));
    for (at, directive) in program.directives.iter().enumerate() {
        let body = match directive {
            Directive::Set { key, value: set } => format!("set s{key} {}", value(set)),
            Directive::SetCapture { capture, key, value: set } => format!("set-capture L{capture} s{key} {}", value(set)),
            Directive::Offset { capture, offsets } => {
                format!("offset L{capture} {}", offsets.iter().map(i32::to_string).collect::<Vec<_>>().join(" "))
            }
            Directive::Strip { capture, source, .. } => format!("strip L{capture} s{source}"),
            Directive::SelectAdjacent { capture, target } => format!("select-adjacent L{capture} L{target}"),
            Directive::Downcase { capture } => format!("downcase L{capture}"),
            Directive::SetLanguageFromMimeType { capture } => format!("language-from-mime L{capture}"),
        };
        let _ = writeln!(out, "  directive {at} {body}");
    }
    if let Some(node) = &program.ast_pattern {
        out.push_str("  pattern ");
        pattern(&mut out, node, names);
        out.push('\n');
    }
    for (pc, &instruction) in program.words.iter().enumerate() {
        let (opcode, mode, arg) = split(instruction);
        let name = OPCODES.iter().find(|(code, _)| *code == opcode).map(|(_, name)| (*name).to_owned());
        let name = name.unwrap_or_else(|| format!("OP_{opcode:02x}"));
        let _ = write!(out, "  {pc:04} {name}");
        if mode != 0 {
            let _ = write!(out, " m={mode}");
        }
        let _ = match operand(opcode) {
            Operand::Target => write!(out, " @{arg:04}"),
            Operand::Label => write!(out, " L{arg}"),
            Operand::Predicate => write!(out, " P{arg}"),
            Operand::Assert => write!(out, " A{arg}"),
            Operand::Kind => write!(out, " {}", kind(names, arg)),
            Operand::Field => write!(out, " {}", field(names, arg)),
            Operand::Number => write!(out, " {arg}"),
            Operand::String => write!(out, " s{arg}"),
            Operand::Switch => write!(out, " T{arg}"),
        };
        if operand(opcode) == Operand::Label
            && let Some(name) = program.label_name(arg)
        {
            let _ = write!(out, "  ; {name}");
        }
        out.push('\n');
    }
    out
}

/// A token of a `rir` line: a word, a quoted string, a parenthesis or a bracket.
#[derive(Debug, Clone, PartialEq)]
enum Token {
    Word(String),
    Quoted(Vec<u8>),
    Open,
    Close,
    OpenList,
    CloseList,
}

fn tokens(line: &str) -> Result<Vec<Token>, String> {
    let bytes = line.as_bytes();
    let (mut out, mut at) = (Vec::new(), 0);
    while at < bytes.len() {
        let single = match bytes[at] {
            b';' => break,
            b' ' | b'\t' => {
                at += 1;
                continue;
            }
            b'(' => Some(Token::Open),
            b')' => Some(Token::Close),
            b'[' => Some(Token::OpenList),
            b']' => Some(Token::CloseList),
            _ => None,
        };
        if let Some(token) = single {
            out.push(token);
            at += 1;
            continue;
        }
        if bytes[at] == b'"' {
            at += 1;
            let mut text = Vec::new();
            loop {
                match bytes.get(at) {
                    None => return Err("an unterminated string".into()),
                    Some(b'"') => {
                        at += 1;
                        break;
                    }
                    Some(b'\\') => {
                        match bytes.get(at + 1) {
                            Some(b'"') => text.push(b'"'),
                            Some(b'\\') => text.push(b'\\'),
                            Some(b'x') => {
                                let hex = line.get(at + 2..at + 4).ok_or("a short \\x escape")?;
                                text.push(u8::from_str_radix(hex, 16).map_err(|_| "a bad \\x escape")?);
                                at += 2;
                            }
                            _ => return Err("an unknown escape".into()),
                        }
                        at += 2;
                    }
                    Some(&byte) => {
                        text.push(byte);
                        at += 1;
                    }
                }
            }
            out.push(Token::Quoted(text));
            continue;
        }
        let start = at;
        while at < bytes.len() && !matches!(bytes[at], b' ' | b'\t' | b'(' | b')' | b'[' | b']' | b'"' | b';') {
            at += 1;
        }
        out.push(Token::Word(line.get(start..at).ok_or("a word splits a character")?.to_owned()));
    }
    Ok(out)
}

/// A line's tokens, read in order, with the grammar's names.
struct Line<'a> {
    tokens: std::iter::Peekable<std::vec::IntoIter<Token>>,
    names: Option<&'a Names>,
}

impl Line<'_> {
    fn next(&mut self) -> Result<Token, String> {
        self.tokens.next().ok_or_else(|| "a missing operand".to_owned())
    }

    /// The next token, if it is a word, without reading it.
    fn peek_word(&mut self) -> Option<String> {
        match self.tokens.peek() {
            Some(Token::Word(word)) => Some(word.clone()),
            _ => None,
        }
    }

    fn word(&mut self) -> Result<String, String> {
        match self.next()? {
            Token::Word(word) => Ok(word),
            other => Err(format!("expected a word, found {other:?}")),
        }
    }

    fn quoted(&mut self) -> Result<Vec<u8>, String> {
        match self.next()? {
            Token::Quoted(text) => Ok(text),
            other => Err(format!("expected a string, found {other:?}")),
        }
    }

    fn number<T: std::str::FromStr>(&mut self, prefix: &str) -> Result<T, String> {
        let word = self.word()?;
        word.strip_prefix(prefix).ok_or_else(|| format!("expected `{prefix}…`, found `{word}`"))?.parse().map_err(|_| format!("a bad number `{word}`"))
    }

    fn keyed(&mut self, key: &str) -> Result<bool, String> {
        match self.number::<u8>(&format!("{key}="))? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(format!("`{key}=` is 0 or 1")),
        }
    }

    fn optional(&mut self, prefix: &str) -> Result<Option<u16>, String> {
        let word = self.word()?;
        if word == "-" {
            return Ok(None);
        }
        word.strip_prefix(prefix).and_then(|number| number.parse().ok()).map(Some).ok_or_else(|| format!("a bad operand `{word}`"))
    }

    /// A kind: `#id`, a named kind's name, or an anonymous kind's quoted name.
    fn kind(&mut self) -> Result<u16, String> {
        match self.next()? {
            Token::Word(word) if word.starts_with('#') => word[1..].parse().map_err(|_| format!("a bad kind `{word}`")),
            Token::Word(word) => self.lookup(&word, true),
            Token::Quoted(text) => self.lookup(&String::from_utf8(text).map_err(|_| "a kind's name is UTF-8")?, false),
            other => Err(format!("expected a kind, found {other:?}")),
        }
    }

    fn lookup(&self, name: &str, named: bool) -> Result<u16, String> {
        let names = self.names.ok_or_else(|| format!("kind `{name}` without a grammar"))?;
        names.kinds.get(&(name, named)).copied().ok_or_else(|| format!("no {} kind `{name}` in {}", if named { "named" } else { "anonymous" }, names.header()))
    }

    fn field(&mut self) -> Result<u16, String> {
        let word = self.word()?;
        if let Some(id) = word.strip_prefix('#') {
            return id.parse().map_err(|_| format!("a bad field `{word}`"));
        }
        let names = self.names.ok_or_else(|| format!("field `{word}` without a grammar"))?;
        names.catalog.field_id(&word).ok_or_else(|| format!("no field `{word}` in {}", names.header()))
    }

    /// A bracketed list of items read by `item`.
    fn list(&mut self, mut item: impl FnMut(&mut Self) -> Result<u16, String>) -> Result<Vec<u16>, String> {
        if self.next()? != Token::OpenList {
            return Err("a list opens with `[`".into());
        }
        let mut out = Vec::new();
        while self.tokens.peek() != Some(&Token::CloseList) {
            if self.tokens.peek().is_none() {
                return Err("a list closes with `]`".into());
            }
            out.push(item(self)?);
        }
        self.next()?;
        Ok(out)
    }

    /// The next word, which must be `key`.
    fn want(&mut self, key: &str) -> Result<(), String> {
        let word = self.word()?;
        if word == key { Ok(()) } else { Err(format!("expected `{key}`, found `{word}`")) }
    }

    fn end(&mut self) -> Result<(), String> {
        match self.tokens.next() {
            None => Ok(()),
            Some(extra) => Err(format!("unexpected {extra:?}")),
        }
    }
}

fn parse_pattern(line: &mut Line<'_>, depth: usize) -> Result<PackedPattern, String> {
    if depth > 256 {
        return Err("a pattern nested past 256 levels".into());
    }
    if line.next()? != Token::Open {
        return Err("a pattern node opens with `(`".into());
    }
    let name = |line: &mut Line<'_>| -> Result<Option<String>, String> {
        match line.next()? {
            Token::Word(word) if word == "-" => Ok(None),
            Token::Quoted(text) => String::from_utf8(text).map(Some).map_err(|_| "a capture name is UTF-8".to_owned()),
            other => Err(format!("expected a name, found {other:?}")),
        }
    };
    let named = |line: &mut Line<'_>| -> Result<bool, String> {
        match line.word()?.as_str() {
            "named" => Ok(true),
            "any" => Ok(false),
            other => Err(format!("expected `named` or `any`, found `{other}`")),
        }
    };
    let node = match line.word()?.as_str() {
        "terminal" => {
            let kind = line.kind()?;
            let named = named(line)?;
            let text = String::from_utf8(line.quoted()?).map_err(|_| "a terminal's text is UTF-8")?;
            PackedPattern::Terminal { text, named, kind }
        }
        "single" => {
            let single = name(line)?;
            PackedPattern::Single { name: single, named: named(line)? }
        }
        "ellipsis" => PackedPattern::Ellipsis(name(line)?),
        "internal" => {
            let kind = line.kind()?;
            let mut children = Vec::new();
            loop {
                match line.tokens.peek() {
                    Some(Token::Close) => break,
                    Some(Token::Open) => children.push(parse_pattern(line, depth + 1)?),
                    other => return Err(format!("expected a child or `)`, found {other:?}")),
                }
            }
            PackedPattern::Internal { kind, children }
        }
        other => return Err(format!("an unknown pattern node `{other}`")),
    };
    if line.next()? != Token::Close {
        return Err("a pattern node closes with `)`".into());
    }
    Ok(node)
}

/// The parts of a program as read, before the verifier.
#[derive(Default)]
struct Parts {
    digest: Option<String>,
    caps: Option<String>,
    set_mode: bool,
    query: bool,
    strings: Vec<Vec<u8>>,
    labels: Vec<u16>,
    longs: Vec<u64>,
    predicates: Vec<(String, u16)>,
    capture_predicates: Vec<CapturePredicate>,
    directives: Vec<(String, u16, u16, Option<u16>, [i32; 4])>,
    pattern: Option<PackedPattern>,
    block_programs: Vec<block::BlockProgram>,
    switches: Vec<stream::ByteSwitch>,
    words: Vec<u32>,
}

/// Checks a table row's index: the rows are in order from 0.
fn index(line: &mut Line<'_>, count: usize, what: &str) -> Result<(), String> {
    let at: usize = line.number("")?;
    if at != count {
        return Err(format!("{what} {at} out of order (expected {count})"));
    }
    Ok(())
}

fn row(parts: &mut Parts, line: &mut Line<'_>, keyword: &str) -> Result<(), String> {
    let string = |line: &mut Line<'_>| line.number::<u16>("s");
    match keyword {
        "mode" => {
            parts.set_mode = line.keyed("set")?;
            parts.query = line.keyed("query")?;
        }
        "string" => {
            index(line, parts.strings.len(), "string")?;
            parts.strings.push(line.quoted()?);
        }
        "label" => {
            index(line, parts.labels.len(), "label")?;
            parts.labels.push(line.number("s")?);
        }
        "switch" => switch_row(parts, line)?,
        "long" => {
            index(line, parts.longs.len(), "long")?;
            parts.longs.push(line.number("")?);
        }
        "predicate" => {
            index(line, parts.predicates.len(), "predicate")?;
            let kind = line.word()?;
            parts.predicates.push((kind, line.number("s")?));
        }
        "assert" => {
            index(line, parts.capture_predicates.len(), "assert")?;
            let predicate = match line.word()?.as_str() {
                "equal" => CapturePredicate::Equal { left: line.number("L")?, right: line.number("L")?, negated: line.keyed("negated")?, any: line.keyed("any")? },
                "text" => CapturePredicate::Text { capture: line.number("L")?, predicate: line.number("P")?, negated: line.keyed("negated")?, any: line.keyed("any")? },
                "any-of" => CapturePredicate::AnyOf { capture: line.number("L")?, strings: line.list(string)?, negated: line.keyed("negated")?, any: line.keyed("any")? },
                "kind" => CapturePredicate::Kind { capture: line.number("L")?, kind: line.kind()?, negated: line.keyed("negated")?, any: line.keyed("any")? },
                "kinds" => CapturePredicate::Kinds { capture: line.number("L")?, kinds: line.list(Line::kind)?, negated: line.keyed("negated")? },
                "contains" => CapturePredicate::Contains { capture: line.number("L")?, strings: line.list(string)?, negated: line.keyed("negated")?, any: line.keyed("any")? },
                "relation" => CapturePredicate::Relation { capture: line.number("L")?, kinds: line.list(Line::kind)?, ancestor: line.keyed("ancestor")?, negated: line.keyed("negated")? },
                "same-line" => CapturePredicate::SameLine { left: line.number("L")?, right: line.number("L")?, negated: line.keyed("negated")? },
                other => return Err(format!("an unknown assertion `{other}`")),
            };
            parts.capture_predicates.push(predicate);
        }
        "directive" => {
            index(line, parts.directives.len(), "directive")?;
            let kind = line.word()?;
            let directive = match kind.as_str() {
                "set" => (kind, 0, line.number("s")?, line.optional("s")?, [0; 4]),
                "set-capture" => {
                    let capture = line.number("L")?;
                    (kind, capture, line.number("s")?, line.optional("s")?, [0; 4])
                }
                "offset" => {
                    let capture = line.number("L")?;
                    let offsets = [line.number("")?, line.number("")?, line.number("")?, line.number("")?];
                    (kind, capture, 0, None, offsets)
                }
                "strip" => {
                    let capture = line.number("L")?;
                    (kind, capture, line.number("s")?, None, [0; 4])
                }
                "select-adjacent" => {
                    let capture = line.number("L")?;
                    (kind, capture, line.number("L")?, None, [0; 4])
                }
                "downcase" | "language-from-mime" => {
                    let capture = line.number("L")?;
                    (kind, capture, 0, None, [0; 4])
                }
                other => return Err(format!("an unknown directive `{other}`")),
            };
            parts.directives.push(directive);
        }
        "block" => {
            // `block N %i = op …`, `block N output %v` or `block N extract %v`: program N's rows
            // in order (tables, operations, outputs, extracts), a new
            // program at the next index.
            let at: usize = line.number("")?;
            if at == parts.block_programs.len() {
                parts.block_programs.push(block::BlockProgram::default());
            } else if at + 1 != parts.block_programs.len() {
                return Err(format!("block {at} out of order (expected {})", parts.block_programs.len()));
            }
            let program = parts.block_programs.last_mut().expect("pushed");
            let first = line.word()?;
            if first == "output" {
                if !program.extracts.is_empty() || !program.totals.is_empty() || !program.masks.is_empty() {
                    return Err(format!("block {at} output after an extract or a total"));
                }
                program.outputs.push(line.number("%")?);
                // `output %v delay d`: its own delay; recorded for every output once one has one
                // (`u8::MAX` until the program's delay is read).
                let own = if line.peek_word().as_deref() == Some("delay") {
                    line.word()?;
                    let d: u64 = line.number("")?;
                    Some(u8::try_from(d).map_err(|_| format!("block {at} output delay {d} is more than 63"))?)
                } else {
                    None
                };
                if own.is_some() || !program.output_delays.is_empty() {
                    let k = program.outputs.len() - 1;
                    program.output_delays.resize(k, u8::MAX);
                    program.output_delays.push(own.unwrap_or(u8::MAX));
                }
            } else if first == "delay" {
                // `block N delay D`: the outputs are D bytes late (`block::BlockProgram::delay`).
                let d: u64 = line.number("")?;
                program.delay = u8::try_from(d).map_err(|_| format!("block {at} delay {d} is more than 63"))?;
            } else if first == "rounds" {
                // `block N rounds %c %n …`: a fixed point's carries, each with its next, after
                // the operations.
                if !program.outputs.is_empty() {
                    return Err(format!("block {at} rounds after an output"));
                }
                let mut group = Vec::new();
                while line.peek_word().is_some() {
                    let c: u64 = line.number("%")?;
                    let x: u64 = line.number("%")?;
                    let value = |v: u64| u16::try_from(v).map_err(|_| format!("block {at} rounds value %{v} too large"));
                    group.push((value(c)?, value(x)?));
                }
                if group.is_empty() {
                    return Err(format!("block {at} rounds with no carry"));
                }
                program.rounds.push(group);
            } else if first == "extract" {
                program.extracts.push(line.number("%")?);
            } else if first == "total" {
                program.totals.push(line.number("%")?);
            } else if first == "mask" {
                program.masks.push(line.number("%")?);
            } else if first == "table" {
                let t: usize = line.number("")?;
                if t != program.tables.len() || !program.ops.is_empty() {
                    return Err(format!("block {at} table {t} out of order (expected {}, before the operations)", program.tables.len()));
                }
                let lo = table_half(line, "lo=")?;
                let hi = table_half(line, "hi=")?;
                program.tables.push(block::ClassTable { lo, hi });
            } else {
                let value: usize = first.strip_prefix('%').and_then(|v| v.parse().ok()).ok_or_else(|| format!("a bad value `{first}`"))?;
                if value != program.ops.len() || !program.outputs.is_empty() || !program.rounds.is_empty() {
                    return Err(format!("block {at} value %{value} out of order (expected %{})", program.ops.len()));
                }
                if line.word()? != "=" {
                    return Err("a block row is `%i = op …`".into());
                }
                let mut words = Vec::new();
                while line.tokens.peek().is_some() {
                    words.push(line.word()?);
                }
                program.ops.push(block::BlockOp::parse(&words)?);
            }
        }
        "pattern" => {
            if parts.pattern.is_some() {
                return Err("a second pattern".into());
            }
            parts.pattern = Some(parse_pattern(line, 0)?);
        }
        pc => {
            let pc: usize = pc.parse().map_err(|_| format!("an unknown row `{pc}`"))?;
            if pc != parts.words.len() {
                return Err(format!("word {pc} out of order (expected {})", parts.words.len()));
            }
            let name = line.word()?;
            let opcode = match OPCODES.iter().find(|(_, mnemonic)| *mnemonic == name) {
                Some((opcode, _)) => *opcode,
                None => name
                    .strip_prefix("OP_")
                    .and_then(|hex| u8::from_str_radix(hex, 16).ok())
                    .ok_or_else(|| format!("an unknown opcode `{name}`"))?,
            };
            let mut mode = 0_u8;
            if let Some(Token::Word(word)) = line.tokens.peek()
                && let Some(value) = word.strip_prefix("m=")
            {
                mode = value.parse().map_err(|_| format!("a bad mode `{word}`"))?;
                line.next()?;
            }
            let prefixed = |line: &mut Line<'_>, prefix: &str| line.number::<u16>(prefix);
            let arg = match operand(opcode) {
                Operand::Target => prefixed(line, "@")?,
                Operand::Label => prefixed(line, "L")?,
                Operand::Predicate => prefixed(line, "P")?,
                Operand::Assert => prefixed(line, "A")?,
                Operand::Kind => line.kind()?,
                Operand::Field => line.field()?,
                Operand::Number => prefixed(line, "")?,
                Operand::String => prefixed(line, "s")?,
                Operand::Switch => prefixed(line, "T")?,
            };
            parts.words.push(word(opcode, mode, arg));
        }
    }
    line.end()
}

/// The header's grammar: `none`, or a dialect and the digest of the tables it was dumped with.
fn grammar(value: &str) -> Result<Option<Names>, String> {
    if value == "none" {
        return Ok(None);
    }
    let (name, digest) = value.split_once('@').ok_or("a grammar is `name@digest` or `none`")?;
    let dialect = crate::code_rule_dsl::language::dialect_by_label(name).ok_or_else(|| format!("an unknown grammar `{name}`"))?;
    let names = Names::of(dialect);
    if names.digest != digest {
        return Err(format!("the grammar's tables changed since the dump ({} now, {value} then)", names.header()));
    }
    Ok(Some(names))
}

/// One program from `rir`, through the verifier.
pub(in crate::code_rule_dsl) fn assemble(text: &str) -> Result<Program, String> {
    assemble_named(text).map(|(program, _)| program)
}

/// One program from `rir`, through the verifier, with the grammar its header names.
pub(in crate::code_rule_dsl) fn assemble_named(text: &str) -> Result<(Program, Option<Names>), String> {
    let mut parts = Parts::default();
    let mut names = None;
    let mut header = false;
    for (number, raw) in text.lines().enumerate() {
        let context = |error: String| format!("line {}: {error}", number + 1);
        let tokens = tokens(raw).map_err(context)?;
        let mut line = Line { tokens: tokens.into_iter().peekable(), names: names.as_ref() };
        let Some(first) = line.tokens.next() else { continue };
        let Token::Word(keyword) = first else { return Err(context("a row starts with a word".into())) };
        if !header {
            if keyword != "program" {
                return Err(context("a program starts with `program`".into()));
            }
            let digest = line.word().map_err(context)?;
            parts.digest = match digest.as_str() {
                "-" => None,
                _ => Some(digest.strip_prefix("sha256:").ok_or_else(|| context("a digest is `sha256:…` or `-`".into()))?.to_owned()),
            };
            let mut found = None;
            while line.tokens.peek().is_some() {
                let field = line.word().map_err(context)?;
                let (key, value) = field.split_once('=').ok_or_else(|| context(format!("a header field is `key=value`, not `{field}`")))?;
                match key {
                    "ir" if value == IR.to_string() => {}
                    "ir" => return Err(context(format!("ir={value}; this assembler reads ir={IR}"))),
                    "grammar" => found = grammar(value).map_err(context)?,
                    "caps" => parts.caps = Some(value.to_owned()),
                    other => return Err(context(format!("an unknown header field `{other}`"))),
                }
            }
            names = found;
            header = true;
            continue;
        }
        row(&mut parts, &mut line, &keyword).map_err(context)?;
    }
    if !header {
        return Err("no program".into());
    }
    Ok((build(parts)?, names))
}

fn build(parts: Parts) -> Result<Program, String> {
    let verifier = |error: Error| format!("the verifier refused it: {error:?}");
    let source = |at: u16| parts.strings.get(usize::from(at)).ok_or_else(|| format!("string s{at} is missing"));
    let regex = |at: u16| -> Result<Regex, String> {
        let text = std::str::from_utf8(source(at)?).map_err(|_| "a regex is UTF-8")?;
        Regex::new(text).map_err(|error| format!("a bad regex: {error}"))
    };
    let predicates = parts
        .predicates
        .iter()
        .map(|(kind, at)| {
            Ok(match kind.as_str() {
                "equal" => TextPredicate::Equal(*at),
                "prefix" => TextPredicate::Prefix(*at),
                "regex" => TextPredicate::Regex { source: *at, compiled: regex(*at)? },
                "lua" => TextPredicate::LuaPattern {
                    source: *at,
                    compiled: LuaPattern::parse(source(*at)?).map_err(|_| "a bad Lua pattern")?,
                },
                other => return Err(format!("an unknown predicate `{other}`")),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let directives = parts
        .directives
        .iter()
        .map(|(kind, capture, key, value, offsets)| {
            Ok(match kind.as_str() {
                "set" => Directive::Set { key: *key, value: *value },
                "set-capture" => Directive::SetCapture { capture: *capture, key: *key, value: *value },
                "offset" => Directive::Offset { capture: *capture, offsets: *offsets },
                "strip" => Directive::Strip { capture: *capture, source: *key, compiled: regex(*key)? },
                "select-adjacent" => Directive::SelectAdjacent { capture: *capture, target: *key },
                "downcase" => Directive::Downcase { capture: *capture },
                _ => Directive::SetLanguageFromMimeType { capture: *capture },
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let value_programs = predicates
        .iter()
        .map(|predicate| value::ValueProgram::lower(predicate, &parts.strings))
        .collect::<Result<Vec<_>, _>>()
        .map_err(verifier)?;
    let capture_programs = parts.capture_predicates.iter().map(value::CaptureProgram::lower).collect();
    let output_program = output::OutputProgram::lower(&directives, &parts.strings).map_err(verifier)?;
    let program = Program {
        words: parts.words,
        strings: parts.strings,
        predicates,
        value_programs,
        capture_predicates: parts.capture_predicates,
        capture_programs,
        labels: parts.labels,
        longs: parts.longs,
        directives,
        output_program,
        tree_sitter_query: parts.query,
        set_mode: parts.set_mode,
        ast_pattern: parts.pattern,
        block_programs: parts.block_programs,
        switches: parts.switches,
        digest: DigestMemo::default(),
    };
    program.validate().map_err(verifier)?;
    if let Some(digest) = parts.digest
        && digest != program.plain_digest()
    {
        return Err(format!("the program's digest is sha256:{}, not sha256:{digest}", program.plain_digest()));
    }
    if let Some(stated) = parts.caps
        && stated != caps(&program)
    {
        return Err(format!("the program's caps are {}, not {stated}", caps(&program)));
    }
    Ok(program)
}

/// Every `program` block of `text` (a dump may hold other sections): each from its `program`
/// line to the next line that is not indented.
pub(in crate::code_rule_dsl) fn assemble_all(text: &str) -> Result<Vec<(Program, Option<Names>)>, String> {
    let mut blocks: Vec<(usize, String)> = Vec::new();
    let mut open = false;
    for (number, line) in text.lines().enumerate() {
        if line.starts_with("program ") {
            blocks.push((number, format!("{line}\n")));
            open = true;
        } else if open && (line.starts_with(' ') || line.is_empty()) {
            let block = &mut blocks.last_mut().expect("an open block").1;
            block.push_str(line);
            block.push('\n');
        } else {
            open = false;
        }
    }
    blocks
        .into_iter()
        .map(|(number, block)| assemble_named(&block).map_err(|error| format!("the program at line {}: {error}", number + 1)))
        .collect()
}

/// A fused pack (`fusion::Fused`) as `rir`: its members by digest, each with its dispatch (its
/// entry test, `fusion::entry_key`, and where its branch starts past the shared prefix) and where
/// its captures start; its emit table (each fused label's member and the member's own label);
/// then the fused program.
pub(in crate::code_rule_dsl) fn fused(pack: &super::fusion::Fused, members: &[&Program], names: Option<&Names>) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "group members={} prefix={}", members.len(), pack.prefix);
    for (at, member) in members.iter().enumerate() {
        let entry = super::fusion::entry_key(member).map_or("-".to_owned(), |first| {
            let (opcode, _, arg) = split(first);
            format!("{}:{}", if opcode == KIND { "kind" } else { "supertype" }, kind(names, arg))
        });
        let start = pack.starts.get(at).copied().unwrap_or(0);
        let captures = pack.captures.get(at).copied().unwrap_or(0);
        let _ = writeln!(out, "  member {at} sha256:{} entry={entry} branch=@{start:04} captures-from=L{captures}", member.plain_digest());
    }
    for (at, (member, label)) in pack.emits.iter().enumerate() {
        let _ = writeln!(out, "  emit L{at} member={member} L{label}");
    }
    out.push_str(&disassemble(&pack.program, names));
    out
}

#[cfg(test)]
#[path = "rir_tests.rs"]
mod tests;

/// A class table's half, `lo=HEX` or `hi=HEX`: 16 bytes of hex.
fn table_half(line: &mut Line<'_>, key: &str) -> Result<[u8; 16], String> {
    let word = line.word()?;
    let hex = word.strip_prefix(key).ok_or_else(|| format!("expected `{key}…`, found `{word}`"))?;
    let mut out = [0u8; 16];
    if hex.len() != 32 {
        return Err(format!("a table half is 16 bytes of hex, not `{hex}`"));
    }
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).map_err(|_| format!("bad hex `{hex}`"))?;
    }
    Ok(out)
}

/// A target of a switch: `@PC`, or `refuse`.
fn switch_target(t: u16) -> String {
    if t == stream::REFUSE { "refuse".to_owned() } else { format!("@{t:04}") }
}

/// Switch table `at` (decision 0087): `switch N at OFFSET end TARGET`, then a row per run of
/// bytes with one target, `switch N 0xLO-0xHI TARGET`; bytes in no row refuse.
pub(super) fn switch_rows(out: &mut String, at: usize, s: &stream::ByteSwitch) {
    let _ = writeln!(out, "  switch {at} at {} end {}", s.offset, switch_target(s.end));
    let mut b = 0usize;
    while b < 256 {
        let t = s.targets[b];
        let mut e = b;
        while e + 1 < 256 && s.targets[e + 1] == t {
            e += 1;
        }
        if t != stream::REFUSE {
            let _ = writeln!(out, "  switch {at} {b:#04x}-{e:#04x} {}", switch_target(t));
        }
        b = e + 1;
    }
}

fn switch_row(parts: &mut Parts, line: &mut Line<'_>) -> Result<(), String> {
    let at: usize = line.number("")?;
    let target = |line: &mut Line<'_>| -> Result<u16, String> {
        match line.peek_word().as_deref() {
            Some("refuse") => {
                line.next()?;
                Ok(stream::REFUSE)
            }
            _ => line.number("@"),
        }
    };
    if line.peek_word().as_deref() == Some("at") {
        if at != parts.switches.len() {
            return Err(format!("switch {at} out of order (expected {})", parts.switches.len()));
        }
        line.next()?;
        let offset: u8 = line.number("")?;
        if line.word()? != "end" {
            return Err("a switch is `switch N at OFFSET end TARGET`".into());
        }
        let end = target(line)?;
        parts.switches.push(stream::ByteSwitch { end, ..stream::ByteSwitch::refusing(offset) });
        return Ok(());
    }
    if at + 1 != parts.switches.len() {
        return Err(format!("switch {at}'s bytes before its header"));
    }
    let range = line.word()?;
    let (lo, hi) = range.split_once('-').ok_or_else(|| format!("a byte range is `0xLO-0xHI`, not `{range}`"))?;
    let byte = |x: &str| x.strip_prefix("0x").and_then(|h| u8::from_str_radix(h, 16).ok()).ok_or_else(|| format!("a bad byte `{x}`"));
    let (lo, hi) = (byte(lo)?, byte(hi)?);
    let t = target(line)?;
    for b in lo..=hi {
        parts.switches[at].targets[usize::from(b)] = t;
    }
    Ok(())
}
