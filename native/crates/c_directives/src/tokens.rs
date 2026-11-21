//! Every preprocessing token of a file (decision 0348 §1): the tape mode (b) expands from, made
//! once a file's content by `bitlex_tokens/tokens.bitlex` (C++'s; C's leaves out its lines marked
//! `C++ only`). Its reference is `rerail_c_lean::pp::PpLexer` (C23 and C++26, GNU); a file the
//! grammar refuses goes to it.

use crate::{Compiled, compile_kinds};

/// The grammar, C++'s.
pub const TOKENS: &str = include_str!("../bitlex_tokens/tokens.bitlex");

/// The tape's kinds, by id: the root, a line's node, the tokens'.
pub const KINDS: &[&str] = &["tokens", "line", "identifier", "number", "literal", "punct", "digraph", "hash", "hashhash", "other"];

/// A token's flags: the lean lexer's `BOL` and `SPACE`.
pub const BOL: u8 = rerail_c_lean::pp::BOL;
pub const SPACE: u8 = rerail_c_lean::pp::SPACE;

/// A token: its kind (`KINDS`), its bytes (the file's) and its flags.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tok {
    pub kind: &'static str,
    pub start: u32,
    pub end: u32,
    pub flags: u8,
}

/// The grammar's text for C (`cpp` false: its `C++ only` lines left out) or C++ (its `C only`
/// lines left out).
pub fn grammar(cpp: bool) -> String {
    let drop = if cpp { "# C only" } else { "# C++ only" };
    TOKENS.lines().filter(|l| !l.trim_end().ends_with(drop)).map(|l| format!("{l}\n")).collect()
}

static C: std::sync::OnceLock<Result<Compiled, String>> = std::sync::OnceLock::new();
static CPP: std::sync::OnceLock<Result<Compiled, String>> = std::sync::OnceLock::new();

/// The process's compiled token grammar for C or C++, compiled the first time.
pub fn compiled(cpp: bool) -> Result<&'static Compiled, &'static str> {
    let cell = if cpp { &CPP } else { &C };
    cell.get_or_init(|| compile_kinds(&grammar(cpp), KINDS, &[])).as_ref().map_err(String::as_str)
}

static C_WORDS: std::sync::OnceLock<Result<rerail_compiler::program::Words, String>> = std::sync::OnceLock::new();
static CPP_WORDS: std::sync::OnceLock<Result<rerail_compiler::program::Words, String>> = std::sync::OnceLock::new();

/// The grammar lowered to the transducer's words only (no Cranelift), the first time: the
/// interpreter's input.
pub fn lowered(cpp: bool) -> Result<&'static rerail_compiler::program::Words, &'static str> {
    let cell = if cpp { &CPP_WORDS } else { &C_WORDS };
    cell.get_or_init(|| crate::lower_kinds(&grammar(cpp), KINDS, &[]).map(|l| l.0)).as_ref().map_err(String::as_str)
}

/// `src`'s tokens by the interpreter, or `None` where it refuses the file.
pub fn interpreted(src: &[u8], cpp: bool) -> Option<Vec<Tok>> {
    let program = lowered(cpp).ok()?;
    let run = program.block_programs[0].run(src);
    let nodes = rerail_compiler::stream::run(program, src, &run).ok()?;
    read(&rerail_compiler::stream::image(&nodes, &program.readings.extras, 0, src), src).ok()
}

/// A token tape's tokens (of file `src`), with their flags: `BOL` a line's first, `SPACE` where
/// the byte before is white space or a comment's, not the line break the token starts after.
pub fn read(image: &[u8], src: &[u8]) -> Result<Vec<Tok>, String> {
    let v = rerail_tape_format::View::new(image)?;
    let mut out = Vec::with_capacity(v.len());
    let (mut bol, mut end) = (false, 0);
    for i in 1..v.len() {
        let kind = KINDS.get((v.key(i) & 0x3ff) as usize).copied().ok_or_else(|| format!("node {i}: kind {:#x}", v.key(i)))?;
        if kind == "line" {
            bol = true;
            continue;
        }
        let start = v.start(i);
        // A byte order mark is no white space.
        let space = start > end && !line_break_before(src, start as usize) && !(start == 3 && src.starts_with(b"\xef\xbb\xbf"));
        out.push(Tok { kind, start, end: v.end(i), flags: if bol { BOL } else { 0 } | if space { SPACE } else { 0 } });
        (bol, end) = (false, v.end(i));
    }
    Ok(out)
}

/// Whether the byte before `at` is a line break no `\` splices.
fn line_break_before(src: &[u8], at: usize) -> bool {
    let b = |k: usize| at.checked_sub(k).map(|k| src[k]);
    b(1) == Some(b'\n') && b(2) != Some(b'\\') && !(b(2) == Some(b'\r') && b(3) == Some(b'\\'))
}

/// `src`'s tokens by the compiled grammar, or `None` where it refuses the file.
pub fn tokens(src: &[u8], cpp: bool) -> Option<Vec<Tok>> {
    read(&compiled(cpp).ok()?.image(src)?, src).ok()
}

/// `src`'s tokens by the lean lexer, in the file's bytes (its phase-2 text mapped back).
pub fn lean(src: &[u8], cpp: bool) -> Vec<Tok> {
    use rerail_c_lean::pp::Options;
    let opts = if cpp { Options::cpp(26, true) } else { Options::c(23, true) };
    let kinds = rerail_c_pp::tape::KINDS;
    rerail_c_pp::tape::tokens(src, opts).into_iter().map(|t| Tok { kind: kinds[t.kind as usize], start: t.start, end: t.end, flags: t.flags & (BOL | SPACE) }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tokens by the compiled grammar (every backend) equal the lean lexer's, or the file is
    /// refused; `Err` with the first difference.
    fn check(src: &[u8], cpp: bool) -> Result<bool, String> {
        let c = compiled(cpp).unwrap_or_else(|e| panic!("compiles: {e}"));
        let image = c.image(src);
        let interp = c.interpreter_image(src);
        if interp != image {
            return Err(format!("the interpreter's tape differs ({} / {})", interp.is_some(), image.is_some()));
        }
        // The device's tape: on a Mac it always runs, so a failure is the test's (a failed device compile is fatal,
        // never skipped). A Linux host's GPU lane (decision 0440 step E) may have no device (`UnsupportedBackend`),
        // and is then skipped; where there is one, any other failure is the test's there too.
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        match c.metal_images(&[src]) {
            Ok(m) if m[0] != image => return Err(format!("Metal's tape differs ({} / {})", m[0].is_some(), image.is_some())),
            Ok(_) | Err(rerail_compiler::Error::UnsupportedBackend) => {}
            Err(e) => return Err(format!("the device runs it: {e:?}")),
        }
        let Some(image) = image else { return Ok(false) };
        let (got, want) = (read(&image, src)?, lean(src, cpp));
        match got.iter().zip(&want).position(|(a, b)| a != b) {
            None if got.len() == want.len() => Ok(true),
            at => {
                let at = at.unwrap_or(got.len().min(want.len()));
                let show = |t: Option<&Tok>| t.map(|t| format!("{t:?} {:?}", String::from_utf8_lossy(&src[t.start as usize..(t.end as usize).max(t.start as usize)])));
                Err(format!("token {at}: {:?} against {:?}", show(got.get(at)), show(want.get(at))))
            }
        }
    }

    #[test]
    fn tokens_read_as_the_lean_lexer_reads_them() {
        let cases = [
            "int main(void) { return a<<=2 >>= b... ; }\n",
            "#define X(a, ...) a ## __VA_ARGS__ # a\n#include <a/b.h>\nx->y++ --z;\n",
            "  a /* c */ b\n\tc // d\nd\n",
            "x = 1.5e+3 + .5 - 0x1p-3 + 1..2 + a.5 + 12abc + 1.e5;\n",
            "s = L\"a\" u8\"b\" u'c' U\"d\" xL\"e\" R\"(f\n)\" u8R\"(g)\" \"h\"\"i\" 'j';\n",
            "a = b ? c : d; e[1] = {2}; f(g, h); ~i; !j; k%l; m^n; o&p | q;\n",
            "a <: b :> <% %> ### # ## @ `\n",
            "\u{feff}int a;\n",
            "a\r\nb \r\n c\r\n",
            "x \\\n y\n",
            "a \\ b\\n c\\ \\\n d \\",
            "",
            "a",
            // Digit separators (C23, C++14) and `u8'…'` (C23, C++17).
            "n = 1'000'000 + 0'7 + 0x'1 + 1'a'b + 1.5'e+3 + .5'0 + x.5'1; c = u8'a' + x1'b' + 1' ' + '1''2';\n",
        ];
        for cpp in [false, true] {
            for src in cases {
                assert_eq!(check(src.as_bytes(), cpp), Ok(true), "{src:?} (C++ {cpp})");
            }
        }
        // C++: `->*`, `<=>`, `.*` and a literal's suffix; C: each is two tokens, or three.
        for src in ["a->*b <=> c.*d; s = \"x\"_s + 'c'_u + 1_km;\n"] {
            for cpp in [false, true] {
                assert_eq!(check(src.as_bytes(), cpp), Ok(true), "{src:?} (C++ {cpp})");
            }
        }
    }

    #[test]
    fn what_the_grammar_does_not_read_is_refused() {
        for src in ["a = \\u00e9;\n", "a = é;\n", "a+\\\n+b;\n", "x<::y> z;\n", "??=define X\n", "s = R\"x(a)x\";\n", "a\rb\n", "c = '.5'a + 1;\n"] {
            assert_eq!(check(src.as_bytes(), true), Ok(false), "{src:?}");
        }
    }

    /// The corpus (`~/orca/rerail-shared/lean-corpus/{c,cpp}`): each file read as the lean lexer
    /// reads it, or refused (counted by reason); none differs. Ignored: it reads 3,000 files.
    #[test]
    #[ignore]
    fn the_corpus_tokens_are_the_lean_lexers() {
        let root = std::env::var("RERAIL_C_CORPUS").map(std::path::PathBuf::from).unwrap_or_else(|_| std::path::PathBuf::from(std::env::var("HOME").unwrap()).join("orca/rerail-shared/lean-corpus"));
        let (mut same, mut refused, mut differ) = (0, std::collections::BTreeMap::<String, usize>::new(), Vec::new());
        for (dir, cpp) in [("c", false), ("cpp", true)] {
            let Ok(entries) = std::fs::read_dir(root.join(dir)) else { continue };
            let mut paths: Vec<_> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
            paths.sort();
            for p in paths {
                let src = std::fs::read(&p).unwrap();
                match check(&src, cpp) {
                    Ok(true) => same += 1,
                    Ok(false) => {
                        let why = compiled(cpp).unwrap().parser.image(&src).err().map(|e| format!("{e:?}")).unwrap_or_default();
                        *refused.entry(why.chars().take(90).collect()).or_default() += 1;
                    }
                    Err(e) => differ.push(format!("{}: {e}", p.display())),
                }
            }
        }
        eprintln!("the same {same}, refused {}, differ {}", refused.values().sum::<usize>(), differ.len());
        for (why, n) in &refused {
            eprintln!("  {n:5} {why}");
        }
        for d in differ.iter().take(15) {
            eprintln!("  {d}");
        }
        assert!(differ.is_empty());
    }
}

#[cfg(test)]
mod lowering_time {
    /// The token grammar's lowering, step by step (work queue Q131): run it with `--nocapture`.
    #[test]
    #[ignore = "a measurement"]
    fn lowering_steps() {
        use rerail_compiler::bitlex_masks as bc;
        for cpp in [false, true] {
            let t = std::time::Instant::now();
            let stmts = ::bitlex::syntax::parse(&super::grammar(cpp)).unwrap();
            let parse = t.elapsed();
            let kind = |name: &str| super::KINDS.iter().position(|&k| k == name).map(|i| i as u16).or(Some(900));
            let field = |_: &str| None;
            let symbols = bc::Symbols { kind: &kind, field: &field };
            let t = std::time::Instant::now();
            let compiled = bc::compile_shaped(&stmts, &[bc::FALLBACK], Some(&symbols), bc::Shape::Rules).unwrap();
            let compile = t.elapsed();
            let t = std::time::Instant::now();
            let text = compiled.text();
            let program = rerail_compiler::rir_asm::assemble(&text).unwrap();
            let assemble = t.elapsed();
            let t = std::time::Instant::now();
            let _native = rerail_compiler::native_ir::lower(&program, 0, 0).unwrap();
            let lower = t.elapsed();
            eprintln!("cpp {cpp}: parse {parse:?}, compile {compile:?}, assemble {assemble:?} ({} bytes of rir, {} ops), lower {lower:?}", text.len(), program.block_programs[0].ops.len());
        }
    }
}
