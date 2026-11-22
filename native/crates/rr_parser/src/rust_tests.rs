//! The `.rerail` parser as Rust (`rerail-rr-aot`, written by the Bitlex compiler from the grammar, decision 0560)
//! against the JIT's compile of the same grammar: the files are the compiler's output, and tier 1's records are the
//! interpreter's.

use crate::{Compiled, SAMPLES, compiled};

/// `rerail_rr_aot`'s `tier1.rs` and `tier2.rs` are the Bitlex compiler's output for the grammar, in the compile
/// `compiled` makes (its codes and ids are the JIT's); `RERAIL_BLESS=1` writes them again.
#[test]
fn rust_is_current() {
    let c = compiled().expect("the grammar compiles");
    let tier1 = c.lexer_rust.clone().unwrap_or_else(|e| panic!("tier 1 as Rust: {e}"));
    let tier2 = c.rust.clone().unwrap_or_else(|e| panic!("tier 2 as one loop: {e}"));
    for (file, rust) in [("tier1.rs", tier1), ("tier2.rs", tier2)] {
        let path = format!("{}/../rr_aot/src/{file}", env!("CARGO_MANIFEST_DIR"));
        if std::env::var_os("RERAIL_BLESS").is_some() {
            std::fs::write(&path, &rust).expect("writes");
        }
        assert!(std::fs::read_to_string(&path).is_ok_and(|have| have == rust), "rr_aot's {file} is not the compiler's: RERAIL_BLESS=1");
    }
}

/// A tape's nodes, in order: each one's key (its kind, field, flags and supertype classes), start, end and size.
type Nodes = Vec<(u32, u32, u32, usize)>;

/// The JIT's image's nodes.
fn image_nodes(image: &[u8]) -> Nodes {
    let v = rerail_tape_format::View::new(image).expect("an image");
    (0..v.len()).map(|i| (v.key(i), v.start(i), v.end(i), v.size(i))).collect()
}

/// The parser as Rust's tape's nodes (`rerail_rr_aot::parse`), as `image_nodes` reads an image's.
fn columns_nodes(t: &rerail_tape_format::Columns) -> Nodes {
    let cell = |column: &[u8], width: u8, i: usize| -> u32 {
        let w = usize::from(width & 0x7f);
        let b = &column[i * w..i * w + w];
        match w {
            1 => u32::from(b[0]),
            2 => u32::from(u16::from_le_bytes([b[0], b[1]])),
            _ => u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
        }
    };
    let start = |i: usize| {
        let b = i / rerail_tape_format::CHECKPOINT_STRIDE;
        u32::from_le_bytes(t.base[4 * b..4 * b + 4].try_into().expect("a base")) + cell(&t.off, t.widths[1], i)
    };
    (0..t.code.len() / 2)
        .map(|i| {
            let code = u16::from_le_bytes([t.code[2 * i], t.code[2 * i + 1]]);
            let size = cell(&t.size, t.widths[0], i) as usize;
            let s = start(i);
            let from = if t.widths[2] & rerail_tape_format::TAIL_FROM_START != 0 { s } else { start(i + size - 1) };
            (t.keys[usize::from(code)], s, from + cell(&t.tail, t.widths[2], i), size)
        })
        .collect()
}

/// Where the parser as Rust's tape is not the JIT's: whether it refuses the file, and where neither does, the first
/// node that differs.
fn tape_differs(c: &Compiled, text: &str) -> Option<String> {
    let theirs = c.parser.image(text.as_bytes()).ok().map(|i| image_nodes(&i));
    let ours = rerail_rr_aot::parse(&rerail_rr_aot::padded(text.as_bytes()), text.len()).ok().map(|t| columns_nodes(&t));
    match (theirs, ours) {
        (None, None) => None,
        (Some(_), None) => Some("the JIT parses it, the parser as Rust refuses it".into()),
        (None, Some(_)) => Some("the parser as Rust parses it, the JIT refuses it".into()),
        (Some(a), Some(b)) => {
            let k = (0..a.len().max(b.len())).find(|&k| a.get(k) != b.get(k))?;
            let at = |n: Option<&(u32, u32, u32, usize)>| n.map_or(String::new(), |n| text[n.1 as usize..].chars().take(16).collect::<String>().replace('\n', "⏎"));
            Some(format!("node {k} of {}/{}: the JIT {:x?} `{}`, the parser as Rust {:x?} `{}`", a.len(), b.len(), a.get(k), at(a.get(k)), b.get(k), at(b.get(k))))
        }
    }
}

/// The mark bits of the hidden marks only refused (`refuse M in L`): the lowering checks their local token tests in
/// tier 2, in the refusing levels (`bitlex_masks`, `kind_checks`), so the interpreter's records lack those bits; tier
/// 1 as Rust tests them in the token loop and records them. Each bit is the one tier 1 as Rust gives the mark.
fn tier2_checked_marks() -> u32 {
    let stmts = ::bitlex::syntax::parse(crate::GRAMMAR).expect("parses");
    let tier1 = std::fs::read_to_string(format!("{}/../rr_aot/src/tier1.rs", env!("CARGO_MANIFEST_DIR"))).expect("reads");
    let mut out = 0;
    for s in &stmts {
        let ::bitlex::syntax::Stmt::Tree { body, .. } = s else { continue };
        for t in body {
            let ::bitlex::syntax::Stmt::Refuse { mark, .. } = t else { continue };
            let at = tier1.find(&format!("// `{mark}`\n")).unwrap_or_else(|| panic!("mark {mark} in tier1.rs"));
            let rest = &tier1[at..];
            let b = rest.find("marks |= 1 << ").map(|i| &rest[i + 14..]).and_then(|r| r[..r.find(';')?].parse::<u32>().ok()).expect("the mark's bit");
            out |= 1 << b;
        }
    }
    out
}

/// Where tier 1 as Rust (`rerail_rr_aot::lex`) is not the interpreter's tier 1: whether it sends the file to the
/// engine, and where neither does, each token's first and last byte and code. In the bits of the marks tier 2 checks
/// (`tier2_checked_marks`), the Rust's are the interpreter's and those tier 2 tests.
fn tier1_differs(c: &Compiled, text: &str) -> Option<String> {
    let late = tier2_checked_marks() << 16;
    let run = c.program.block_programs[0].run(text.as_bytes());
    let lx = rerail_rr_aot::lex(&rerail_rr_aot::padded(text.as_bytes()), text.len());
    let refused = run.totals.first().is_some_and(|&t| t != 0);
    // An escape the lowering makes a check of tier 2's at a token (`pending_checks`): the interpreter's tier 1 does
    // not refuse there, tier 1 as Rust does; either way the file is refused.
    if lx.refused && !refused && c.parser.image(text.as_bytes()).is_err() {
        return None;
    }
    if refused != lx.refused {
        return Some(format!("refused: the interpreter {refused}, tier 1 as Rust {}", lx.refused));
    }
    if refused {
        return None;
    }
    let (starts, lasts) = (&run.positions[0], &run.positions[1]);
    for k in 0..starts.len().max(lx.starts.len()) {
        let theirs = (starts.get(k), lasts.get(k), run.codes.get(k));
        let ours = (lx.starts.get(k), lx.lasts.get(k), lx.codes.get(k));
        let kept = |c: Option<&u32>| c.map(|&c| c & !late);
        let late_ok = run.codes.get(k).zip(lx.codes.get(k)).is_none_or(|(&t, &o)| t & late & !o == 0);
        if (theirs.0, theirs.1, kept(theirs.2)) != (ours.0, ours.1, kept(ours.2)) || !late_ok {
            let at = |s: Option<&u32>| s.map_or(String::new(), |&s| text[s as usize..].chars().take(16).collect::<String>().replace('\n', "⏎"));
            return Some(format!("record {k}: the interpreter {theirs:x?} `{}`, tier 1 as Rust {ours:x?} `{}`", at(theirs.0), at(ours.0)));
        }
    }
    None
}

/// Files that reach tier 1's rules one at a time: the depth's count at and past its bound, the floats (`toggle`'s
/// parity of a chain's dots, and a float the file ends with: a mark's `else escape`), the regions (an empty
/// backtick literal: a region's `else`), the marks.
fn cases() -> Vec<String> {
    let mut out: Vec<String> = SAMPLES.iter().map(|s| (*s).to_owned()).collect();
    for n in [126, 127, 128, 200] {
        out.push(format!("a(X) :- b(X), X = {}1{}.\n", "[".repeat(n), "]".repeat(n)));
        out.push(format!(".policy {}{}\n", "{\"a\": ".repeat(n), "}".repeat(n)));
    }
    for t in [
        "a(X) :- b(X), X = 1.5.\n",
        "a(X) :- b(X), X = 1.5",
        "a(X) :- b(X), X = 1.2.3.4.\n",
        "a(X) :- X = 12.\n",
        "a(1.5, 2.25, 3).\n",
        "a(X) :- X = 1.",
        "a(\"s\\u00e9\\u{1F600}\", `x`, ``y``, ```z```).\n",
        "a(````q````).\n",
        "a(\"open\n",
        "/* open\n",
        "a(X) :- b(X). // line\n/* block */ c(Y) :- d(Y).\n",
        "#include \"x.rerail\"\n.decl a(x: number)\n",
        ".type T = <a: number>\n.input a, b(x)\n.output c\n",
        ".violation v(x: number) code \"C1\" tag {t} \"message\" fix safe r \"d\" suppress tool \"r\", \"s\" over {o}\n",
        "a(X) :- count(Y) : { b(Y) } = X, min(Z) by (W) : { c(Z, W) } > 1.\n",
        // An aggregate's call read again inside it (`min(…) : {…}`) takes the aggregate's word (`remember`): its own
        // again after an aggregate in its arguments has set its own.
        "a(X) :- X = min(Y) : { b(Y) }.\n",
        "a(X) :- X = min(max(Y) : { c(Y) }) : { b(Y) }.\n",
        "a(X) :- X = sum(Y, mean(Z) : { c(Z) }) : { b(Y) }.\n",
        "a(X) :- min(max(Y) : { c(Y) }, Z) : { b(Z) } = X.\n",
        // A second argument there: its `,` dropped, a dead node the reading reaches, refused (`drop`).
        "a(X) :- X = sum(Y, Z) : { b(Y) }.\n",
        "a(X) :- !b(X), (c(X); d(X)), X ~ `p`.\n",
        "a(1234567890123456789).\n",
        "a(123456789012345678).\n",
        "\u{0c}a(1).\n",
        // A string's escapes a JSON document refuses (`json_bad`, refused in its levels only): in a clause, and in a
        // policy's document.
        "a(\"\\q\", 1).\n",
        "a(\"\\u12\", 1).\n",
        "a(\"\\u{1F600}\").\n",
        ".policy {\"a\": \"\\q\"}\n",
        ".policy {\"a\": \"\\u{1F600}\"}\n",
        ".policy {\"a\": \"x\", \"b\": [1, {\"c\": \"\\q\"}]}\n",
        ".policy {\"a\": \"x\"}\na(\"\\q\").\n",
        // `else e_t2 bytes 2`: two backticks with no closer after them, a token of their two bytes.
        "a(b) :- r(c, ``).\n",
        "a(b) :- r(o, ` `), r(c, ``), r(d, ``x``).\n",
    ] {
        out.push(t.to_owned());
    }
    out
}

#[test]
fn tier1s_records_are_the_interpreters() {
    let c = compiled().expect("the grammar compiles");
    let (mut same, mut refused) = (0, 0);
    for (k, t) in cases().iter().enumerate() {
        if let Some(d) = tier1_differs(c, t) {
            panic!("case {k} ({:?}): {d}", &t[..t.len().min(60)]);
        }
        if rerail_rr_aot::lex(&rerail_rr_aot::padded(t.as_bytes()), t.len()).refused { refused += 1 } else { same += 1 }
    }
    eprintln!("{same} files' records the interpreter's, {refused} refused by both");
    assert!(same > 0 && refused > 0);
}

#[test]
fn tapes_are_the_jits() {
    let c = compiled().expect("the grammar compiles");
    let (mut same, mut refused) = (0, 0);
    // Every case that differs, each named, so that a run counts them all.
    let mut differ = Vec::new();
    let cases = cases();
    for (k, t) in cases.iter().enumerate() {
        if let Some(d) = tape_differs(c, t) {
            eprintln!("differs: case {k} ({:?}): {d}", &t[..t.len().min(60)]);
            differ.push(format!("case {k} ({:?}): {d}", &t[..t.len().min(60)]));
            continue;
        }
        if c.parser.image(t.as_bytes()).is_ok() { same += 1 } else { refused += 1 }
    }
    assert!(differ.is_empty(), "{} of {} cases differ; the first: {}", differ.len(), cases.len(), differ[0]);
    eprintln!("{same} tapes the JIT's, {refused} refused by both");
    assert!(same > 0 && refused > 0);
}

/// `tier1s_records_are_the_interpreters` over the repository's `.rerail` files.
#[test]
#[ignore = "the corpus: run by name"]
fn corpus_tier1s_records_are_the_interpreters() {
    let c = compiled().expect("the grammar compiles");
    let files = crate::tests::corpus();
    for (k, t) in files.iter().enumerate() {
        if let Some(d) = tier1_differs(c, t) {
            if let Ok(dir) = std::env::var("RERAIL_RR_DIFFERS") {
                std::fs::write(format!("{dir}/differs.rerail"), t).expect("writes");
            }
            panic!("file {k}: {d}");
        }
    }
    eprintln!("{} files", files.len());
}

/// `tapes_are_the_jits` over the repository's `.rerail` files. `RERAIL_RR_DIFFERS=<dir>`: the first file that
/// differs written there.
#[test]
#[ignore = "the corpus: run by name"]
fn corpus_tapes_are_the_jits() {
    let c = compiled().expect("the grammar compiles");
    let files = crate::tests::corpus();
    let mut parsed = 0;
    // Every file that differs, each named, so that a run counts them all.
    let mut differ = Vec::new();
    for (k, t) in files.iter().enumerate() {
        if let Some(d) = tape_differs(c, t) {
            if differ.is_empty()
                && let Ok(dir) = std::env::var("RERAIL_RR_DIFFERS")
            {
                std::fs::write(format!("{dir}/differs.rerail"), t).expect("writes");
            }
            eprintln!("differs: file {k}: {d}");
            differ.push(format!("file {k}: {d}"));
            continue;
        }
        parsed += usize::from(c.parser.image(t.as_bytes()).is_ok());
    }
    assert!(differ.is_empty(), "{} of {} files differ; the first: {}", differ.len(), files.len(), differ[0]);
    eprintln!("{} files, {parsed} parsed", files.len());
}
