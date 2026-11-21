//! A JSON file's tape from simdjson (`rerail_simdjson`, the C++ library): simdjson's stage 1, and a
//! stage 2 of ours that validates the file as simdjson's does and writes the tape's columns in place
//! (`rerail_simdjson::tape_with`), tree-sitter-json's nodes in preorder. Nothing is packed again
//! (`TapeImage::written`). The tape is the one tree-sitter-json's tree encodes to, byte for byte
//! (`json_tapes_from_simdjson_are_tree_sitters`).
//!
//! JSON's implementations, registered with the parser router best first (`registrations`, decision
//! 0148): a kernel for each CPU family (`rerail_simdjson::Kernel`: arm64; icelake, haswell, westmere),
//! with the CPU's SHA-256 instructions and without; then simdjson's DOM parser and the index writer,
//! which run on any CPU simdjson does.
//!
//! simdjson reads strict JSON. A file the strict read refuses is read again as JSONC
//! (`rerail_simdjson::tape_jsonc_with`, decision 0158): its comments blanked for stage 1 and written to
//! the tape as tree-sitter-json places them (extras), and a trailing comma passed over, as TypeScript
//! reads it (rerail reads source as it is written: permissive). A file still refused (a number past
//! simdjson's range, nesting past 1,024, invalid JSON) goes to tree-sitter-json.
use crate::code_rule_dsl::tape::{Code, FIELD_LIMIT, FileImage, Grammar, KIND_LIMIT, ParserImpl, TapeImage, TemplateError, TreeParser};
pub use rerail_simdjson::Refusal;

/// tree-sitter-json's ids of the tree's kinds and fields, and of the `_value` supertype.
pub fn kinds() -> Option<&'static (rerail_simdjson::Kinds, u16)> {
    static KINDS: std::sync::OnceLock<Option<(rerail_simdjson::Kinds, u16)>> = std::sync::OnceLock::new();
    KINDS
        .get_or_init(|| {
            use crate::code_rule_dsl::language::GrammarDialect;
            use ast_grep_language::LanguageExt;
            let language = GrammarDialect::Json.get_ts_language();
            let kind = |n: &str| super::kind_id(&language, n).filter(|&k| (k as usize) < KIND_LIMIT && k != 0);
            let field = |n: &str| language.field_id_for_name(n).map(|f| f.get()).filter(|&f| (f as usize) < FIELD_LIMIT);
            let kinds = rerail_simdjson::Kinds {
                document: kind("document")?,
                object: kind("object")?,
                pair: kind("pair")?,
                array: kind("array")?,
                string: kind("string")?,
                string_content: kind("string_content")?,
                escape_sequence: kind("escape_sequence")?,
                number: kind("number")?,
                true_: kind("true")?,
                false_: kind("false")?,
                null: kind("null")?,
                comment: kind("comment")?,
                key_field: field("key")?,
                value_field: field("value")?,
            };
            Some((kinds, kind("_value")?))
        })
        .as_ref()
}

/// `source`'s tape, as `route` writes it (a kernel, or with `None` simdjson's DOM parser and the index
/// writer): `Err` where simdjson refuses the file; `Ok(Err)` where the tape cannot be made.
pub fn encode_with(route: Option<rerail_simdjson::Kernel>, grammar: &Grammar, source: &str, budget: &dyn Fn() -> bool) -> Result<Result<TapeImage, String>, Refusal> {
    let Some((kinds, value)) = kinds() else {
        return Ok(Err("tree-sitter-json lacks a kind or field simdjson's tape names".into()));
    };
    let src = source.as_bytes();
    // A run that stores format 16 or 16z: the kernel's walk gives its nodes to format 16's ops (decision 0434).
    if let Some(kernel) = route
        && let Some(image) = encode_16(kernel, kinds, *value, source, budget)?
    {
        return Ok(image);
    }
    let t = match route {
        Some(kernel) => match rerail_simdjson::tape_with(kernel, src, kinds) {
            // Read again as JSONC (comments, trailing commas): a second read only of files the strict
            // one refused.
            Err(rerail_simdjson::Refusal::Simdjson(why)) => {
                let t = rerail_simdjson::tape_jsonc_with(kernel, src, kinds).map_err(|_| rerail_simdjson::Refusal::Simdjson(why))?;
                crate::code_rule_dsl::logic::profile::decision("JSON parse", || format!("refused as JSON ({why}): read as JSONC"));
                t
            }
            t => t?,
        },
        None => rerail_simdjson::tape_from_index(src, kinds)?,
    };
    let codes: Result<Vec<Code>, String> = t
        .codes
        .iter()
        .map(|&(kind, field, in_value)| {
            let mut code = Code::plain(kind, field, if in_value { std::slice::from_ref(value) } else { &[] })?;
            // A comment is an extra, as tree-sitter-json's are.
            if kind == kinds.comment {
                code.flags |= crate::code_rule_dsl::tape::CODE_EXTRA;
            }
            Ok(code)
        })
        .collect();
    Ok(codes.and_then(|codes| TapeImage::written(t.code, t.size, t.base, t.off, t.tail, t.widths, codes, t.digest, grammar, source, budget)))
}

/// The code of a node of kind `kind` and field `field` (`in_value`: a member of `_value`), as format 15's code table
/// holds it (`encode_with`): a comment is an extra.
fn code(kinds: &rerail_simdjson::Kinds, value: u16, kind: u16, field: u16, in_value: bool) -> Result<Code, String> {
    let mut code = Code::plain(kind, field, if in_value { std::slice::from_ref(&value) } else { &[] })?;
    if kind == kinds.comment {
        code.flags |= crate::code_rule_dsl::tape::CODE_EXTRA;
    }
    Ok(code)
}

/// `source`'s tape in format 16 or 16z, where the run stores one (`None`: format 15's): the kernel's walk writes no
/// column, and its events, a window at a time, go to format 16's ops, each code numbered in JSON's table once. The
/// walk never edits a node it wrote, so each window is committed and 16z's blocks are compressed as they close. A
/// file the strict read refuses is read again as JSONC, as `encode_with` reads it.
fn encode_16(kernel: rerail_simdjson::Kernel, kinds: &rerail_simdjson::Kinds, value: u16, source: &str, budget: &dyn Fn() -> bool) -> Result<Option<Result<TapeImage, String>>, Refusal> {
    use crate::code_rule_dsl::language::GrammarDialect;
    use crate::code_rule_dsl::tape::formats;
    use crate::code_rule_dsl::tape::f16;
    let Some((lang, tag, z)) = formats::run_16(GrammarDialect::Json) else { return Ok(None) };
    let src = source.as_bytes();
    let walk = |jsonc: bool| -> Result<(f16::Writer, [u8; 32], Option<String>), Refusal> {
        // About a node for every 8 bytes: the writer grows past it.
        let mut w = f16::Writer::new(lang, src, tag, z, true, src.len() / 8 + 16);
        let mut numbers: Vec<u32> = Vec::new();
        let mut bad: Option<String> = None;
        let digest = rerail_simdjson::events_with(kernel, src, kinds, jsonc, |codes, words| {
            if bad.is_some() {
                return;
            }
            while numbers.len() < codes.len() {
                let (kind, field, in_value) = codes.get(numbers.len());
                match code(kinds, value, kind, field, in_value).and_then(|c| lang.number(&c.entry()).ok_or_else(|| format!("code {:?} is not in JSON's table", c.entry()))) {
                    Ok(n) => numbers.push(n),
                    Err(why) => {
                        bad = Some(why);
                        return;
                    }
                }
            }
            for e in words.as_chunks::<3>().0 {
                let number = numbers[(e[0] >> 8) as usize];
                match e[0] & 0xff {
                    rerail_simdjson::EV_OPEN => {
                        w.open(number, u64::from(e[1]));
                    }
                    rerail_simdjson::EV_LEAF => {
                        w.leaf(number, u64::from(e[1]), u64::from(e[2]));
                    }
                    _ => w.close(u64::from(e[1])),
                }
            }
            w.commit();
        })?;
        Ok((w, digest, bad))
    };
    let (mut w, digest, bad) = match walk(false) {
        // Read again as JSONC (comments, trailing commas), as `encode_with` does.
        Err(rerail_simdjson::Refusal::Simdjson(why)) => {
            let t = walk(true).map_err(|_| rerail_simdjson::Refusal::Simdjson(why))?;
            crate::code_rule_dsl::logic::profile::decision("JSON parse", || format!("refused as JSON ({why}): read as JSONC"));
            t
        }
        t => t?,
    };
    let why = match bad {
        Some(why) => why,
        None => match w.finish() {
            Ok(f) => return Ok(Some(TapeImage::written_16(f, GrammarDialect::Json, source, Some(digest), budget))),
            Err(why) => why,
        },
    };
    formats::keeps_15(GrammarDialect::Json, tag, &why);
    Ok(None)
}

/// `source`'s tape by the fastest route this CPU runs (`encode_with`).
pub fn encode(grammar: &Grammar, source: &str, budget: &dyn Fn() -> bool) -> Result<Result<TapeImage, String>, Refusal> {
    encode_with(rerail_simdjson::Kernel::best(), grammar, source, budget)
}

/// JSON's tree parser: simdjson, standing in for tree-sitter-json, by a kernel (`Some`) or by its DOM
/// parser and the index writer (`None`).
pub struct Simdjson(pub Option<rerail_simdjson::Kernel>);

/// A parser for each kernel, in `Kernel::ALL`'s order, and the DOM parser's.
static KERNELS: [Simdjson; 8] = {
    use rerail_simdjson::Kernel;
    [
        Simdjson(Some(Kernel::ALL[0])),
        Simdjson(Some(Kernel::ALL[1])),
        Simdjson(Some(Kernel::ALL[2])),
        Simdjson(Some(Kernel::ALL[3])),
        Simdjson(Some(Kernel::ALL[4])),
        Simdjson(Some(Kernel::ALL[5])),
        Simdjson(Some(Kernel::ALL[6])),
        Simdjson(Some(Kernel::ALL[7])),
    ]
};
static DOM: Simdjson = Simdjson(None);

/// JSON's implementations for the parser router, best first: each kernel in `Kernel::ALL`'s order
/// (wider vectors first; the SHA-256 instructions before portable code), then the DOM parser and the
/// index writer (with the source hashed after, by `TapeImage::written`). The router takes the first
/// this CPU runs.
pub fn registrations() -> Vec<ParserImpl> {
    use crate::code_rule_dsl::language::GrammarDialect;
    let kernels = KERNELS.iter().enumerate().map(|(i, parser)| {
        let kernel = parser.0.expect("a kernel");
        ParserImpl { dialect: GrammarDialect::Json, name: kernel.name(), rank: 10 + i as u32, runs: kernel.runs(), parser }
    });
    let dom = ParserImpl { dialect: GrammarDialect::Json, name: "simdjson DOM, the index writer", rank: 100, runs: true, parser: &DOM };
    kernels.chain(std::iter::once(dom)).collect()
}

impl TreeParser for Simdjson {
    fn parse(&self, grammar: &Grammar, source: &str, budget: &dyn Fn() -> bool) -> Option<Result<FileImage, TemplateError>> {
        if !matches!(crate::code_rule_dsl::language::base(crate::code_rule_dsl::language::Family::of(grammar.lang)).parse_input(source), std::borrow::Cow::Borrowed(_))
        {
            return None;
        }
        let span = crate::code_rule_dsl::logic::profile::span(|| "simdjson parse and tape".into());
        match encode_with(self.0, grammar, source, &budget) {
            Ok(image) => {
                drop(span);
                crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::SourceByte, source.len() as u64);
                Some(image.map(|image| FileImage::single(image, source)).map_err(TemplateError::Encode))
            }
            // A file simdjson refuses (a number past its range, nesting past 1,024) is read as MSVC's parser reads it
            // (`fallback::Fallback`: the Bitlex parser, then tree-sitter-json past trailing commas), and one that
            // refuses too goes to tree-sitter-json. Straight to tree-sitter-json, an exponent's `+` (`1e+21`), which
            // its grammar refuses, was an error in a file that also held `-9223372036854778000`.
            Err(why) => {
                drop(span);
                crate::code_rule_dsl::logic::profile::decision("JSON parse", || format!("a file simdjson refused ({why}): the Bitlex parser, else tree-sitter-json"));
                super::fallback::Fallback.parse(grammar, source, budget)
            }
        }
    }
}
