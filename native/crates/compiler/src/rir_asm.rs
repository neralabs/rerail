//! The transducer's assembler, the part of it a parser's program uses: a `rir` line's tokens
//! (`tokens`, `Line`), the opcodes' mnemonics, the rows of the word program (`word_row`: the
//! words, strings, labels, longs, tier 1's block programs and tier 2's tables), and a parser's
//! program assembled and verified (`assemble`). The engine's assembler (`rir`) reads the
//! matcher's rows beside these and the grammars' names.

use crate::Error;
use crate::rir_text::{quote, reading_rows, switch_rows};
use std::fmt::Write as _;
use crate::program::*;

/// The format's version: the program digest's (`tempo-program-v7`).
pub const IR: u32 = 7;

/// The opcodes' mnemonics.
pub const OPCODES: &[(u8, &str)] = &[
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
    (REMEMBER, "REMEMBER"),
    (FLAG, "FLAG"),
];

/// How an opcode's operand prints.
#[derive(Clone, Copy, PartialEq)]
pub enum Operand {
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

/// An operand's shape by opcode and mode: `RETAG m=3`'s is a reading's index.
pub fn operand(opcode: u8, mode: u8) -> Operand {
    match opcode {
        RETAG if mode == 3 => Operand::Number,
        FORK | GOTO | PUSH => Operand::Target,
        WORD_IS => Operand::String,
        SWITCH => Operand::Switch,
        OPEN | LEAF | NODE_SUPER | RETAG | MERGE | WRAP | REMEMBER => Operand::Kind,
        NODE_FIELD => Operand::Field,
        CAPTURE | EMIT => Operand::Label,
        TEXT => Operand::Predicate,
        ASSERT => Operand::Assert,
        KIND | SUPERTYPE => Operand::Kind,
        FIELD | ABSENT_FIELD => Operand::Field,
        _ => Operand::Number,
    }
}

/// A token of a `rir` line: a word, a quoted string, a parenthesis or a bracket.
#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Word(String),
    Quoted(Vec<u8>),
    Open,
    Close,
    OpenList,
    CloseList,
}

pub fn tokens(line: &str) -> Result<Vec<Token>, String> {
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

/// A grammar's kind and field names, as a line reads them (the engine's `Names`).
pub trait NameTable {
    fn kind_id(&self, name: &str, named: bool) -> Result<u16, String>;
    fn field_id(&self, name: &str) -> Result<u16, String>;
}

/// A line's tokens, read in order, with the grammar's names.
pub struct Line<'a> {
    pub tokens: std::iter::Peekable<std::vec::IntoIter<Token>>,
    pub names: Option<&'a dyn NameTable>,
}

impl Line<'_> {
    pub fn next(&mut self) -> Result<Token, String> {
        self.tokens.next().ok_or_else(|| "a missing operand".to_owned())
    }

    /// The next token, if it is a word, without reading it.
    pub fn peek_word(&mut self) -> Option<String> {
        match self.tokens.peek() {
            Some(Token::Word(word)) => Some(word.clone()),
            _ => None,
        }
    }

    pub fn word(&mut self) -> Result<String, String> {
        match self.next()? {
            Token::Word(word) => Ok(word),
            other => Err(format!("expected a word, found {other:?}")),
        }
    }

    pub fn quoted(&mut self) -> Result<Vec<u8>, String> {
        match self.next()? {
            Token::Quoted(text) => Ok(text),
            other => Err(format!("expected a string, found {other:?}")),
        }
    }

    pub fn number<T: std::str::FromStr>(&mut self, prefix: &str) -> Result<T, String> {
        let word = self.word()?;
        word.strip_prefix(prefix).ok_or_else(|| format!("expected `{prefix}…`, found `{word}`"))?.parse().map_err(|_| format!("a bad number `{word}`"))
    }

    pub fn keyed(&mut self, key: &str) -> Result<bool, String> {
        match self.number::<u8>(&format!("{key}="))? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(format!("`{key}=` is 0 or 1")),
        }
    }

    pub fn optional(&mut self, prefix: &str) -> Result<Option<u16>, String> {
        let word = self.word()?;
        if word == "-" {
            return Ok(None);
        }
        word.strip_prefix(prefix).and_then(|number| number.parse().ok()).map(Some).ok_or_else(|| format!("a bad operand `{word}`"))
    }

    /// A kind: `#id`, a named kind's name, or an anonymous kind's quoted name.
    pub fn kind(&mut self) -> Result<u16, String> {
        match self.next()? {
            Token::Word(word) if word.starts_with('#') => word[1..].parse().map_err(|_| format!("a bad kind `{word}`")),
            Token::Word(word) => self.lookup(&word, true),
            Token::Quoted(text) => self.lookup(&String::from_utf8(text).map_err(|_| "a kind's name is UTF-8")?, false),
            other => Err(format!("expected a kind, found {other:?}")),
        }
    }

    pub fn lookup(&self, name: &str, named: bool) -> Result<u16, String> {
        let names = self.names.ok_or_else(|| format!("kind `{name}` without a grammar"))?;
        names.kind_id(name, named)
    }

    pub fn field(&mut self) -> Result<u16, String> {
        let word = self.word()?;
        if let Some(id) = word.strip_prefix('#') {
            return id.parse().map_err(|_| format!("a bad field `{word}`"));
        }
        let names = self.names.ok_or_else(|| format!("field `{word}` without a grammar"))?;
        names.field_id(&word)
    }

    /// A bracketed list of items read by `item`.
    pub fn list(&mut self, mut item: impl FnMut(&mut Self) -> Result<u16, String>) -> Result<Vec<u16>, String> {
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
    pub fn want(&mut self, key: &str) -> Result<(), String> {
        let word = self.word()?;
        if word == key { Ok(()) } else { Err(format!("expected `{key}`, found `{word}`")) }
    }

    pub fn end(&mut self) -> Result<(), String> {
        match self.tokens.next() {
            None => Ok(()),
            Some(extra) => Err(format!("unexpected {extra:?}")),
        }
    }
}

/// Checks a table row's index: the rows are in order from 0.
pub fn index(line: &mut Line<'_>, count: usize, what: &str) -> Result<(), String> {
    let at: usize = line.number("")?;
    if at != count {
        return Err(format!("{what} {at} out of order (expected {count})"));
    }
    Ok(())
}

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

fn reading_row(parts: &mut Words, line: &mut Line<'_>) -> Result<(), String> {
    let at: usize = line.number("")?;
    let maps = &mut parts.readings.maps;
    if at == maps.len() {
        maps.push(crate::stream_ir::Reading::default());
    } else if at + 1 != maps.len() {
        return Err(format!("reading {at} out of order (expected {})", maps.len()));
    }
    let r = &mut maps[at];
    if line.peek_word().as_deref() == Some("root") {
        line.next()?;
        if r.root.is_some() {
            return Err(format!("reading {at}: two roots"));
        }
        r.root = Some(match line.word()?.as_str() {
            "gone" => crate::stream_ir::GONE,
            "as" => line.number("#")?,
            w => return Err(format!("a reading's root row is `reading N root gone` or `root as #M`, not `{w}`")),
        });
        return Ok(());
    }
    if line.peek_word().as_deref() == Some("field") {
        line.next()?;
        let f: u16 = line.number("#")?;
        if line.word()? != "as" {
            return Err("a reading's field row is `reading N field #F as #G`".into());
        }
        r.fields.push((f, line.number("#")?));
        return Ok(());
    }
    let k: u16 = line.number("#")?;
    let m = match line.word()?.as_str() {
        "keep" => crate::stream_ir::KEEP,
        "gone" => crate::stream_ir::GONE,
        "as" => line.number("#")?,
        w => return Err(format!("a reading's row is `#K as #M`, `#K gone` or `#K keep`, not `{w}`")),
    };
    r.kinds.push((k, m));
    Ok(())
}

fn switch_row(parts: &mut Words, line: &mut Line<'_>) -> Result<(), String> {
    let at: usize = line.number("")?;
    let target = |line: &mut Line<'_>| -> Result<u16, String> {
        match line.peek_word().as_deref() {
            Some("refuse") => {
                line.next()?;
                Ok(crate::stream_ir::REFUSE)
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
        parts.switches.push(crate::stream_ir::ByteSwitch { end, ..crate::stream_ir::ByteSwitch::refusing(offset) });
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

/// A row of the word program (`Words`): the mode, a string, a label, a long, a switch's or a
/// reading's row, a block program's row, or a word. The matcher's rows (predicates, assertions,
/// directives, the pattern) are the engine's assembler's.
pub fn word_row(parts: &mut Words, line: &mut Line<'_>, keyword: &str) -> Result<(), String> {
    match keyword {
        "mode" => {
            parts.set_mode = line.keyed("set")?;
            parts.tree_sitter_query = line.keyed("query")?;
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
        "reading" => reading_row(parts, line)?,
        "rule" => {
            index(line, parts.rules.len(), "rule")?;
            parts.rules.push(line.number("@")?);
        }
        "provisional" => {
            let f: u16 = line.number("#")?;
            parts.readings.provisional.push(f);
        }
        "extra" => {
            let k: u16 = line.number("#")?;
            parts.readings.extras.push(k);
        }
        "long" => {
            index(line, parts.longs.len(), "long")?;
            parts.longs.push(line.number("")?);
        }
        "block" => {
            // `block N %i = op …`, `block N output %v` or `block N extract %v`: program N's rows
            // in order (tables, operations, outputs, extracts), a new
            // program at the next index.
            let at: usize = line.number("")?;
            if at == parts.block_programs.len() {
                parts.block_programs.push(crate::block::BlockProgram::default());
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
            } else if first == "rows" {
                // `block N rows C P S`: a row program, before its operations.
                if !program.ops.is_empty() || program.rows.is_some() {
                    return Err(format!("block {at} rows after the operations"));
                }
                let (columns, params, sets): (u16, u16, u16) = (line.number("")?, line.number("")?, line.number("")?);
                program.rows = Some(crate::block::RowShape { columns, params, sets });
            } else if first == "delay" {
                // `block N delay D`: the outputs are D bytes late (`crate::block::BlockProgram::delay`).
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
            } else if first == "records" {
                let (starts, lasts): (u16, u16) = (line.number("")?, line.number("")?);
                let kinds = line.word()?;
                let hex = kinds.strip_prefix("kinds=").ok_or("`records S L kinds=…`")?;
                let kinds: Vec<u8> = (0..hex.len() / 2).map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16)).collect::<Result<_, _>>().map_err(|_| "a bad kind")?;
                program.records = Some(crate::records::Records { starts, lasts, kinds, words: Vec::new() });
            } else if first == "word" {
                let r = program.records.as_mut().ok_or(format!("block {at} word before its records"))?;
                r.words.push(line.quoted()?);
            } else if first == "table" {
                let t: usize = line.number("")?;
                if t != program.tables.len() || !program.ops.is_empty() {
                    return Err(format!("block {at} table {t} out of order (expected {}, before the operations)", program.tables.len()));
                }
                let lo = table_half(line, "lo=")?;
                let hi = table_half(line, "hi=")?;
                program.tables.push(crate::block::ClassTable { lo, hi });
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
                program.ops.push(crate::block::BlockOp::parse(&words)?);
            }
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
            let arg = match operand(opcode, mode) {
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
    Ok(())
}

/// A parser's program from `rir` (a Bitlex file's, `bitlex_masks::Compiled::text`), through the
/// verifier: its words are tier 2's, or a lone `STOP` (a lexer's). A matcher's program, a
/// digest or a grammar's names are the engine's assembler's (`rir::assemble`).
pub fn assemble(text: &str) -> Result<Words, String> {
    let mut parts = Words::default();
    let mut header = false;
    for (number, raw) in text.lines().enumerate() {
        let context = |error: String| format!("line {}: {error}", number + 1);
        let tokens = tokens(raw).map_err(context)?;
        let mut line = Line { tokens: tokens.into_iter().peekable(), names: None };
        let Some(first) = line.tokens.next() else { continue };
        let Token::Word(keyword) = first else { return Err(context("a row starts with a word".into())) };
        if !header {
            if keyword != "program" {
                return Err(context("a program starts with `program`".into()));
            }
            if line.word().map_err(context)? != "-" {
                return Err(context("a parser's program has no digest (the engine's assembler checks one)".into()));
            }
            while line.tokens.peek().is_some() {
                let field = line.word().map_err(context)?;
                match field.split_once('=') {
                    Some(("ir", value)) if value == IR.to_string() => {}
                    Some(("grammar", "none")) => {}
                    _ => return Err(context(format!("a parser's program has no header field `{field}`"))),
                }
            }
            header = true;
            continue;
        }
        word_row(&mut parts, &mut line, &keyword).map_err(context)?;
        line.end().map_err(context)?;
    }
    if !header {
        return Err("no program".into());
    }
    validate(&parts).map_err(|error| format!("the verifier refused it: {error:?}"))?;
    Ok(parts)
}

/// A parser's program, verified: its block programs, its labels, and its words, tier 2's
/// (`stream::validate_tier2`) or a lone `STOP`.
pub fn validate(program: &Words) -> Result<(), Error> {
    if program.words.is_empty() || program.words.len() > usize::from(u16::MAX) + 1 {
        return Err(Error::InvalidTarget(0));
    }
    for block in &program.block_programs {
        block.verify()?;
    }
    if program.labels.iter().any(|&label| program.strings.get(usize::from(label)).is_none_or(|s| std::str::from_utf8(s).is_err())) {
        return Err(Error::InvalidOperand(0));
    }
    if program.words.iter().any(|&w| crate::stream::is_stream(split(w).0)) {
        return crate::stream::validate_tier2(program);
    }
    if program.words == [word(STOP, 0, 0)] && program.switches.is_empty() {
        return Ok(());
    }
    Err(Error::InvalidOpcode(0))
}

/// The word program's head rows, in the order the assembler reads them: the mode, the
/// strings, the labels, the longs, tier 2's switches and readings.
pub fn head_rows(out: &mut String, program: &Words) {
    let flag = u8::from;
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
        switch_rows(out, at, s);
    }
    reading_rows(out, &program.readings);
    crate::rir_text::rule_rows(out, &program.rules);
}

/// Each block program's rows (`block::block_rows`).
pub fn block_programs_rows(out: &mut String, program: &Words) {
    for (at, block) in program.block_programs.iter().enumerate() {
        crate::block::block_rows(out, at, block);
    }
}

/// The words, one row each, with a kind and a field as `kind` and `field` print them (a
/// grammar's names, or `#id`), and a label's name after it.
pub fn word_rows(out: &mut String, program: &Words, kind: &dyn Fn(u16) -> String, field: &dyn Fn(u16) -> String) {
    for (pc, &instruction) in program.words.iter().enumerate() {
        let (opcode, mode, arg) = split(instruction);
        let name = OPCODES.iter().find(|(code, _)| *code == opcode).map(|(_, name)| (*name).to_owned());
        let name = name.unwrap_or_else(|| format!("OP_{opcode:02x}"));
        let _ = write!(out, "  {pc:04} {name}");
        if mode != 0 {
            let _ = write!(out, " m={mode}");
        }
        let _ = match operand(opcode, mode) {
            Operand::Target => write!(out, " @{arg:04}"),
            Operand::Label => write!(out, " L{arg}"),
            Operand::Predicate => write!(out, " P{arg}"),
            Operand::Assert => write!(out, " A{arg}"),
            Operand::Kind => write!(out, " {}", kind(arg)),
            Operand::Field => write!(out, " {}", field(arg)),
            Operand::Number => write!(out, " {arg}"),
            Operand::String => write!(out, " s{arg}"),
            Operand::Switch => write!(out, " T{arg}"),
        };
        if operand(opcode, mode) == Operand::Label
            && let Some(name) = label_name(program, arg)
        {
            let _ = write!(out, "  ; {name}");
        }
        out.push('\n');
    }
}

/// Label `label`'s name: the string it names.
pub fn label_name(program: &Words, label: u16) -> Option<&str> {
    let string = *program.labels.get(usize::from(label))?;
    std::str::from_utf8(program.strings.get(usize::from(string))?).ok()
}

/// A parser's program as `rir` (the inverse of `assemble`): no digest, no grammar, kinds and
/// fields as `#id`. The engine's disassembler (`rir::disassemble`) prints a matcher's program,
/// a digest and a grammar's names.
pub fn disassemble(program: &Words) -> String {
    let mut out = format!("program - ir={IR} grammar=none\n");
    head_rows(&mut out, program);
    block_programs_rows(&mut out, program);
    word_rows(&mut out, program, &|id| format!("#{id}"), &|id| format!("#{id}"));
    out
}
