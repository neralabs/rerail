//! A JSON file's tape from simdjson (`rerail_simdjson`, the C++ library): simdjson's stage 1, and a
//! stage 2 of ours that validates the file as simdjson's does and gives tree-sitter-json's nodes in preorder, as
//! events, to format 16's ops (`rerail_simdjson::events_with`, decision 0434). Nothing is packed again. The tape is
//! the one tree-sitter-json's tree encodes to, byte for byte (`json_tapes_from_simdjson_are_tree_sitters`).
//!
//! JSON's implementations, registered with the parser router best first (`registrations`, decision
//! 0148): a kernel for each CPU family (`rerail_simdjson::Kernel`: arm64; icelake, haswell, westmere),
//! with the CPU's SHA-256 instructions and without; then simdjson's DOM parser and the index writer,
//! which run on any CPU simdjson does.
//!
//! simdjson reads strict JSON. A file the strict read refuses is read again as JSONC
//! (decision 0158): its comments blanked for stage 1 and written to
//! the tape as tree-sitter-json places them (extras), and a trailing comma passed over, as TypeScript
//! reads it (rerail reads source as it is written: permissive). A file still refused (a number past
//! simdjson's range, nesting past 1,024, invalid JSON) goes to tree-sitter-json.
use rerail_blocks::vector::Vector;
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

/// `source`'s tape, as `route` writes it (a kernel, or with `None` simdjson's DOM parser and the index writer), in the
/// run's format: `Err` where simdjson refuses the file; `Ok(Err)` where the tape cannot be made. The walk's events, a
/// window at a time, go to format 16's ops, each code numbered in JSON's table once. The walk never edits a node it
/// wrote, so each window is committed and 16z's blocks are compressed as they close. A file the strict read refuses
/// is read again as JSONC.
pub fn encode_with(route: Option<rerail_simdjson::Kernel>, grammar: &Grammar, source: &str, budget: &dyn Fn() -> bool) -> Result<Result<TapeImage, String>, Refusal> {
    use crate::code_rule_dsl::language::GrammarDialect;
    use crate::code_rule_dsl::tape::formats;
    use crate::code_rule_dsl::tape::f16;
    let Some((kinds, value)) = kinds() else {
        return Ok(Err("tree-sitter-json lacks a kind or field simdjson's tape names".into()));
    };
    let (lang, tag, z) = match formats::run_16(grammar.lang) {
        Ok(l) => l,
        Err(e) => return Ok(Err(e)),
    };
    let src = source.as_bytes();
    let walk = |jsonc: bool| -> Result<(f16::Writer, Option<[u8; 32]>, Option<String>), Refusal> {
        // About a node for every 8 bytes: the writer grows past it.
        let mut w = f16::Writer::placing(crate::code_rule_dsl::tape::placed::rooms());
        w.begin(lang, src, crate::code_rule_dsl::lines::terminators(GrammarDialect::Json), tag, z, true, src.len() / 8 + 16);
        let mut numbers: Vector<u32> = rerail_blocks::vector::Vector::new();
        let mut bad: Option<String> = None;
        let on = |codes: &rerail_simdjson::Codes, words: &[u32]| {
            if bad.is_some() {
                return;
            }
            while numbers.len() < codes.len() {
                let (kind, field, in_value) = codes.get(numbers.len());
                match code(kinds, *value, kind, field, in_value).and_then(|c| lang.number(&c.entry()).ok_or_else(|| format!("code {:?} is not in JSON's table", c.entry()))) {
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
        };
        // A kernel hashes the source as it reads it; the index writer does not (`written_16` hashes it).
        let digest = match route {
            Some(kernel) => Some(rerail_simdjson::events_with(kernel, src, kinds, jsonc, on)?),
            None if jsonc => return Err(Refusal::Shape("the index writer reads strict JSON")),
            None => {
                rerail_simdjson::events_from_index(src, kinds, on)?;
                None
            }
        };
        Ok((w, digest, bad))
    };
    let (mut w, digest, bad) = match walk(false) {
        // Read again as JSONC (comments, trailing commas): a second read only of files the strict one refused.
        Err(rerail_simdjson::Refusal::Simdjson(why)) if route.is_some() => {
            let t = walk(true).map_err(|_| rerail_simdjson::Refusal::Simdjson(why))?;
            crate::code_rule_dsl::logic::profile::decision("JSON parse", || format!("refused as JSON ({why}): read as JSONC"));
            t
        }
        t => t?,
    };
    if let Some(why) = bad {
        return Ok(Err(why));
    }
    // The source's digest (the kernel's, where it hashed the source as it read it): the tape's auxiliary index is keyed
    // by it, and laid by its writer (`aux_of`).
    let digest = match digest {
        Some(d) => d,
        None => match crate::code_rule_dsl::tape::source_digest(source, &budget) {
            Ok(d) => d,
            Err(e) => return Ok(Err(e)),
        },
    };
    let lay = |t: &f16::Written| crate::code_rule_dsl::tape::formats::aux_of(t, GrammarDialect::Json, source, &digest);
    Ok(w.finish_laid(Some(&lay)).and_then(|f| TapeImage::written_16(f, GrammarDialect::Json, source, Some(digest), budget)))
}

/// The code of a node of kind `kind` and field `field` (`in_value`: a member of `_value`): a comment is an extra.
fn code(kinds: &rerail_simdjson::Kinds, value: u16, kind: u16, field: u16, in_value: bool) -> Result<Code, String> {
    let mut code = Code::plain(kind, field, if in_value { std::slice::from_ref(&value) } else { &[] })?;
    if kind == kinds.comment {
        code.flags |= crate::code_rule_dsl::tape::CODE_EXTRA;
    }
    Ok(code)
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
/// index writer (with the source hashed after, by `TapeImage::written_16`). The router takes the first
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
            // A file simdjson refuses (a number past its range, nesting past 1,024) is read by JSON's fallback
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
