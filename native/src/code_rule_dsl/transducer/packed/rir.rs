//! `rir`, the Rerail IR's text form (list step 18; 05 Phase 4 (h), L2; P `spec-tooling.md`
//! §2.2.1–2.2.2): one stable format every tool reads and writes. A program prints as UTF-8 lines,
//! its tables in index order, then its words, with no addresses and no hashes but its content
//! digest, so two dumps diff cleanly. The assembler reads it back: `assemble(disassemble(p))` is
//! `p` (equal digests; the digest covers every component a program is built from), and it builds
//! through the verifier (`Program::validate`, which every constructor runs), so a hand-written
//! program cannot bypass it.
//!
//! ```text
//! program sha256:<plain digest>
//!   mode set=0 query=0
//!   string 0 "match"
//!   label 0 s0
//!   long 0 100000
//!   predicate 0 regex s3
//!   assert 0 text L1 P0 negated=0 any=0
//!   directive 0 set s4 -
//!   pattern (internal 12 (terminal 3 named "x") (single $A named) (ellipsis -))
//!   0000 KIND 145
//!   0001 FORK @0004
//!   0002 CAPTURE L0  ; match
//! ```
//!
//! Text after `;` is a comment. A header of `program -` takes no digest (a hand-written program);
//! otherwise the assembled program's digest must equal it.

use super::*;

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
];

/// An operand's prefix by opcode: a jump target, a label, a text predicate, a capture assertion.
fn prefix(opcode: u8) -> &'static str {
    match opcode {
        FORK | GOTO => "@",
        CAPTURE | EMIT => "L",
        TEXT => "P",
        ASSERT => "A",
        _ => "",
    }
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

fn list(prefix: &str, items: &[u16]) -> String {
    format!("[{}]", items.iter().map(|item| format!("{prefix}{item}")).collect::<Vec<_>>().join(" "))
}

fn pattern(out: &mut String, node: &PackedPattern) {
    let name = |name: &Option<String>| name.as_ref().map_or("-".to_owned(), |name| quote(name.as_bytes()));
    match node {
        PackedPattern::Terminal { text, named, kind } => {
            let _ = write!(out, "(terminal {kind} {} {})", if *named { "named" } else { "any" }, quote(text.as_bytes()));
        }
        PackedPattern::Single { name: single, named } => {
            let _ = write!(out, "(single {} {})", name(single), if *named { "named" } else { "any" });
        }
        PackedPattern::Ellipsis(ellipsis) => {
            let _ = write!(out, "(ellipsis {})", name(ellipsis));
        }
        PackedPattern::Internal { kind, children } => {
            let _ = write!(out, "(internal {kind}");
            for child in children {
                out.push(' ');
                pattern(out, child);
            }
            out.push(')');
        }
    }
}

/// The program as `rir`.
pub(in crate::code_rule_dsl) fn disassemble(program: &Program) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "program sha256:{}", program.plain_digest());
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
    for (at, predicate) in program.predicates.iter().enumerate() {
        let (kind, source) = match predicate {
            TextPredicate::Equal(source) => ("equal", source),
            TextPredicate::Prefix(source) => ("prefix", source),
            TextPredicate::Regex { source, .. } => ("regex", source),
            TextPredicate::LuaPattern { source, .. } => ("lua", source),
        };
        let _ = writeln!(out, "  predicate {at} {kind} s{source}");
    }
    for (at, predicate) in program.capture_predicates.iter().enumerate() {
        let body = match predicate {
            CapturePredicate::Equal { left, right, negated, any } => {
                format!("equal L{left} L{right} negated={} any={}", flag(*negated), flag(*any))
            }
            CapturePredicate::Text { capture, predicate, negated, any } => {
                format!("text L{capture} P{predicate} negated={} any={}", flag(*negated), flag(*any))
            }
            CapturePredicate::AnyOf { capture, strings, negated, any } => {
                format!("any-of L{capture} {} negated={} any={}", list("s", strings), flag(*negated), flag(*any))
            }
            CapturePredicate::Kind { capture, kind, negated, any } => {
                format!("kind L{capture} {kind} negated={} any={}", flag(*negated), flag(*any))
            }
            CapturePredicate::Kinds { capture, kinds, negated } => format!("kinds L{capture} {} negated={}", list("", kinds), flag(*negated)),
            CapturePredicate::Contains { capture, strings, negated, any } => {
                format!("contains L{capture} {} negated={} any={}", list("s", strings), flag(*negated), flag(*any))
            }
            CapturePredicate::Relation { capture, kinds, ancestor, negated } => {
                format!("relation L{capture} {} ancestor={} negated={}", list("", kinds), flag(*ancestor), flag(*negated))
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
        pattern(&mut out, node);
        out.push('\n');
    }
    for (pc, &instruction) in program.words.iter().enumerate() {
        let (opcode, mode, operand) = split(instruction);
        let name = OPCODES.iter().find(|(code, _)| *code == opcode).map(|(_, name)| (*name).to_owned());
        let name = name.unwrap_or_else(|| format!("OP_{opcode:02x}"));
        let _ = write!(out, "  {pc:04} {name}");
        if mode != 0 {
            let _ = write!(out, " m={mode}");
        }
        if matches!(opcode, FORK | GOTO) {
            let _ = write!(out, " @{operand:04}");
        } else {
            let _ = write!(out, " {}{operand}", prefix(opcode));
        }
        if matches!(opcode, CAPTURE | EMIT)
            && let Some(name) = program.label_name(operand)
        {
            let _ = write!(out, "  ; {name}");
        }
        out.push('\n');
    }
    out
}

/// A token of a `rir` line: a word, a quoted string, or a parenthesis.
#[derive(Debug, Clone, PartialEq)]
enum Token {
    Word(String),
    Quoted(Vec<u8>),
    Open,
    Close,
}

fn tokens(line: &str) -> Result<Vec<Token>, String> {
    let bytes = line.as_bytes();
    let (mut out, mut at) = (Vec::new(), 0);
    while at < bytes.len() {
        match bytes[at] {
            b';' => break,
            b' ' | b'\t' => at += 1,
            b'(' => {
                out.push(Token::Open);
                at += 1;
            }
            b')' => {
                out.push(Token::Close);
                at += 1;
            }
            b'"' => {
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
            }
            _ => {
                let start = at;
                while at < bytes.len() && !matches!(bytes[at], b' ' | b'\t' | b'(' | b')' | b'"' | b';') {
                    at += 1;
                }
                out.push(Token::Word(line[start..at].to_owned()));
            }
        }
    }
    Ok(out)
}

/// A line's tokens, read in order.
struct Line {
    tokens: std::vec::IntoIter<Token>,
}

impl Line {
    fn next(&mut self) -> Result<Token, String> {
        self.tokens.next().ok_or_else(|| "a missing operand".to_owned())
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

    fn list(&mut self, prefix: &str) -> Result<Vec<u16>, String> {
        let first = self.word()?;
        let mut words = vec![first];
        while !words.last().is_some_and(|word| word.ends_with(']')) {
            words.push(self.word()?);
        }
        let joined = words.join(" ");
        let inner = joined.strip_prefix('[').and_then(|rest| rest.strip_suffix(']')).ok_or_else(|| format!("a bad list `{joined}`"))?;
        inner
            .split_whitespace()
            .map(|item| item.strip_prefix(prefix).and_then(|number| number.parse().ok()).ok_or_else(|| format!("a bad list item `{item}`")))
            .collect()
    }

    fn end(&mut self) -> Result<(), String> {
        match self.tokens.next() {
            None => Ok(()),
            Some(extra) => Err(format!("unexpected {extra:?}")),
        }
    }
}

fn parse_pattern(line: &mut Line) -> Result<PackedPattern, String> {
    if line.next()? != Token::Open {
        return Err("a pattern node opens with `(`".into());
    }
    let name = |line: &mut Line| -> Result<Option<String>, String> {
        match line.next()? {
            Token::Word(word) if word == "-" => Ok(None),
            Token::Quoted(text) => String::from_utf8(text).map(Some).map_err(|_| "a capture name is UTF-8".to_owned()),
            other => Err(format!("expected a name, found {other:?}")),
        }
    };
    let named = |line: &mut Line| -> Result<bool, String> {
        match line.word()?.as_str() {
            "named" => Ok(true),
            "any" => Ok(false),
            other => Err(format!("expected `named` or `any`, found `{other}`")),
        }
    };
    let node = match line.word()?.as_str() {
        "terminal" => {
            let kind = line.number("")?;
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
            let kind = line.number("")?;
            let mut children = Vec::new();
            loop {
                let mut ahead = line.tokens.clone();
                match ahead.next() {
                    Some(Token::Close) => break,
                    Some(Token::Open) => children.push(parse_pattern(line)?),
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
    set_mode: bool,
    query: bool,
    strings: Vec<Vec<u8>>,
    labels: Vec<u16>,
    longs: Vec<u64>,
    predicates: Vec<(String, u16)>,
    capture_predicates: Vec<CapturePredicate>,
    directives: Vec<(String, u16, u16, Option<u16>, [i32; 4])>,
    pattern: Option<PackedPattern>,
    words: Vec<u32>,
}

/// Checks a table row's index: the rows are in order from 0.
fn index(line: &mut Line, count: usize, what: &str) -> Result<(), String> {
    let at: usize = line.number("")?;
    if at != count {
        return Err(format!("{what} {at} out of order (expected {count})"));
    }
    Ok(())
}

fn row(parts: &mut Parts, line: &mut Line, keyword: &str) -> Result<(), String> {
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
                "any-of" => CapturePredicate::AnyOf { capture: line.number("L")?, strings: line.list("s")?, negated: line.keyed("negated")?, any: line.keyed("any")? },
                "kind" => CapturePredicate::Kind { capture: line.number("L")?, kind: line.number("")?, negated: line.keyed("negated")?, any: line.keyed("any")? },
                "kinds" => CapturePredicate::Kinds { capture: line.number("L")?, kinds: line.list("")?, negated: line.keyed("negated")? },
                "contains" => CapturePredicate::Contains { capture: line.number("L")?, strings: line.list("s")?, negated: line.keyed("negated")?, any: line.keyed("any")? },
                "relation" => CapturePredicate::Relation { capture: line.number("L")?, kinds: line.list("")?, ancestor: line.keyed("ancestor")?, negated: line.keyed("negated")? },
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
        "pattern" => {
            if parts.pattern.is_some() {
                return Err("a second pattern".into());
            }
            parts.pattern = Some(parse_pattern(line)?);
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
            let mut operand = line.word()?;
            let mut mode = 0_u8;
            if let Some(value) = operand.strip_prefix("m=") {
                mode = value.parse().map_err(|_| format!("a bad mode `{operand}`"))?;
                operand = line.word()?;
            }
            let operand = operand
                .strip_prefix(prefix(opcode))
                .and_then(|number| number.parse::<u16>().ok())
                .ok_or_else(|| format!("a bad operand `{operand}` for {name}"))?;
            parts.words.push(word(opcode, mode, operand));
        }
    }
    line.end()
}

/// One program from `rir`, through the verifier.
pub(in crate::code_rule_dsl) fn assemble(text: &str) -> Result<Program, String> {
    let mut parts = Parts::default();
    let mut header = false;
    for (number, raw) in text.lines().enumerate() {
        let context = |error: String| format!("line {}: {error}", number + 1);
        let mut line = Line { tokens: tokens(raw).map_err(context)?.into_iter() };
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
            line.end().map_err(context)?;
            header = true;
            continue;
        }
        row(&mut parts, &mut line, &keyword).map_err(context)?;
    }
    if !header {
        return Err("no program".into());
    }
    build(parts)
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
        digest: DigestMemo::default(),
    };
    program.validate().map_err(verifier)?;
    if let Some(digest) = parts.digest
        && digest != program.plain_digest()
    {
        return Err(format!("the program's digest is sha256:{}, not sha256:{digest}", program.plain_digest()));
    }
    Ok(program)
}

/// Every `program` block of `text` (a dump may hold other sections): each from its `program`
/// line to the next line that is not indented.
pub(in crate::code_rule_dsl) fn assemble_all(text: &str) -> Result<Vec<Program>, String> {
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
        .map(|(number, block)| assemble(&block).map_err(|error| format!("the program at line {}: {error}", number + 1)))
        .collect()
}

/// A fused pack (`fusion::Fused`) as `rir`: its members (by digest), its emit table (each fused
/// label's member and the member's own label), where each member's captures start, the members'
/// shared prefix, then the fused program.
pub(in crate::code_rule_dsl) fn fused(pack: &super::fusion::Fused, members: &[&Program]) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "group members={} prefix={}", members.len(), pack.prefix);
    for (at, member) in members.iter().enumerate() {
        let _ = writeln!(out, "  member {at} sha256:{} captures-from=L{}", member.plain_digest(), pack.captures.get(at).copied().unwrap_or(0));
    }
    for (at, (member, label)) in pack.emits.iter().enumerate() {
        let _ = writeln!(out, "  emit L{at} member={member} L{label}");
    }
    out.push_str(&disassemble(&pack.program));
    out
}

#[cfg(test)]
#[path = "rir_tests.rs"]
mod tests;
