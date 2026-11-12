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

// The assembler's reader and the word program's rows, the compiler crate's (`rerail_compiler::rir_asm`).
use rerail_compiler::rir_asm::{Line, NameTable, Token, block_programs_rows, head_rows, index, tokens, word_row, word_rows};
pub use rerail_compiler::rir_asm::IR;
use super::*;
use crate::code_rule_dsl::language::{GrammarDialect, KindCatalog};
use crate::code_rule_dsl::tape::DERIVED_CLASS_BASE;
use std::collections::HashMap as Map;





/// A grammar's kind and field names, as `rir` prints and reads them.
pub struct Names {
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

impl NameTable for Names {
    fn kind_id(&self, name: &str, named: bool) -> Result<u16, String> {
        self.kinds.get(&(name, named)).copied().ok_or_else(|| format!("no {} kind `{name}` in {}", if named { "named" } else { "anonymous" }, self.header()))
    }

    fn field_id(&self, name: &str) -> Result<u16, String> {
        self.catalog.field_id(name).ok_or_else(|| format!("no field `{name}` in {}", self.header()))
    }
}

impl Names {
    pub fn of(dialect: GrammarDialect) -> Self {
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
        let digest = crate::code_rule_dsl::prof::lower_hex(&digest.finalize())[..16].to_owned();
        Self { dialect, catalog, kinds, digest }
    }

    /// The header's `grammar=` value.
    pub fn header(&self) -> String {
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
pub fn kinds(names: Option<&Names>, ids: &[u16]) -> String {
    format!("[{}]", ids.iter().map(|&id| kind(names, id)).collect::<Vec<_>>().join(" "))
}


fn flag(value: bool) -> u8 {
    u8::from(value)
}

fn strings(items: &[u16]) -> String {
    format!("[{}]", items.iter().map(|item| format!("s{item}")).collect::<Vec<_>>().join(" "))
}

/// The program's capabilities (the module's `caps`).
pub fn caps(program: &Program) -> String {
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

// Quoted strings, the compiler's (`rerail_compiler::rir_text`).
pub use rerail_compiler::rir_text::quote;

pub fn disassemble(program: &Program, names: Option<&Names>) -> String {
    let mut out = String::new();
    let grammar = names.map_or("none".to_owned(), Names::header);
    let _ = writeln!(out, "program sha256:{} ir={IR} grammar={grammar} caps={}", program.plain_digest(), caps(program));
    head_rows(&mut out, program);
    for (at, predicate) in program.predicates.iter().enumerate() {
        let (kind, source) = match predicate {
            TextPredicate::Equal(source) => ("equal", source),
            TextPredicate::Prefix(source) => ("prefix", source),
            TextPredicate::Regex { source, .. } => ("regex", source),
            TextPredicate::LuaPattern { source, .. } => ("lua", source),
        };
        let _ = writeln!(out, "  predicate {at} {kind} s{source}");
    }
    block_programs_rows(&mut out, program);
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
    word_rows(&mut out, program, &|id| kind(names, id), &|id| field(names, id));
    out
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
    /// The words and the tables they index, as `word_row` reads them.
    ir: Words,
    predicates: Vec<(String, u16)>,
    capture_predicates: Vec<CapturePredicate>,
    directives: Vec<(String, u16, u16, Option<u16>, [i32; 4])>,
    pattern: Option<PackedPattern>,
}


fn row(parts: &mut Parts, line: &mut Line<'_>, keyword: &str) -> Result<(), String> {
    let string = |line: &mut Line<'_>| line.number::<u16>("s");
    match keyword {
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
        "pattern" => {
            if parts.pattern.is_some() {
                return Err("a second pattern".into());
            }
            parts.pattern = Some(parse_pattern(line, 0)?);
        }
        // The words and the tables they index: the compiler crate's rows (`rir_asm::word_row`).
        _ => word_row(&mut parts.ir, line, keyword)?,
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
pub fn assemble(text: &str) -> Result<Program, String> {
    assemble_named(text).map(|(program, _)| program)
}

/// One program from `rir`, through the verifier, with the grammar its header names.
pub fn assemble_named(text: &str) -> Result<(Program, Option<Names>), String> {
    let mut parts = Parts::default();
    let mut names = None;
    let mut header = false;
    for (number, raw) in text.lines().enumerate() {
        let context = |error: String| format!("line {}: {error}", number + 1);
        let tokens = tokens(raw).map_err(context)?;
        let mut line = Line { tokens: tokens.into_iter().peekable(), names: names.as_ref().map(|n| n as &dyn NameTable) };
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
    let source = |at: u16| parts.ir.strings.get(usize::from(at)).ok_or_else(|| format!("string s{at} is missing"));
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
        .map(|predicate| value::ValueProgram::lower(predicate, &parts.ir.strings))
        .collect::<Result<Vec<_>, _>>()
        .map_err(verifier)?;
    let capture_programs = parts.capture_predicates.iter().map(value::CaptureProgram::lower).collect();
    let output_program = output::OutputProgram::lower(&directives, &parts.ir.strings).map_err(verifier)?;
    let program = Program {
        ir: parts.ir,
        predicates,
        value_programs,
        capture_predicates: parts.capture_predicates,
        capture_programs,
        directives,
        output_program,
        ast_pattern: parts.pattern,
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
pub fn assemble_all(text: &str) -> Result<Vec<(Program, Option<Names>)>, String> {
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

// cratify: `tests` is rerail-engine's (it mounts it)






