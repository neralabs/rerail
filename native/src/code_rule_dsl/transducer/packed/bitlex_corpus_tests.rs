//! The Bitlex corpus (`docs/parser-generators/20-the-ir.md` §7): Bitlex files that exercise the
//! compiler and every backend's code generation, each checked against the reference evaluator
//! (`tools/bitlex`), which runs Bitlex without the compiler.
//!
//! The files are in `bitlex_corpus/`, written by `generated()` (run `write_the_corpus` by name
//! with `RERAIL_WRITE_CORPUS=1`); `the_corpus_is_the_generators` keeps the two equal. Families
//! build up from one construct alone to combinations of many:
//!
//! - `class`: one byte, ranges, high bytes, `0x00`, and many products (several tables);
//! - `logic`: `~ & | ^` and `empty`, nested;
//! - `prev`, `next`, `span`: every distance 1 to 63, and outputs at several delays;
//! - `edge`: `starts`, `lasts`, `after`, `aligned`;
//! - `carry`: `toggle`, `escaped`, `after_odd_run`, `fill`;
//! - `count`: counts of each bound, and their frames;
//! - `fallback`: errors and escapes;
//! - `mix`: seeded random programs over all of these.
//!
//! For each file and input, two checks:
//!
//! 1. every executor (Cranelift, the Metal interpreter and generated MSL at each lane width and
//!    class strategy) gives the CPU interpreter's outputs, bit for bit;
//! 2. the CPU interpreter gives the evaluator's masks.

use super::*;
use ::bitlex::syntax::{Stmt, parse};
use std::collections::HashMap;

const DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/src/code_rule_dsl/transducer/packed/bitlex_corpus");

/// A seeded xorshift: the corpus is the same on every run.
struct Rng(u64);

impl Rng {
    fn below(&mut self, n: u64) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 % n
    }

    fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[self.below(xs.len() as u64) as usize]
    }
}

/// The classes most programs read: bytes the inputs hold often, a high range, `0x00`.
const CLASSES: &str = "class quote = '\"'
class bs    = '\\\\'
class ws    = ' ' '\\t' '\\n' '\\r'
class digit = '0'..'9'
class alpha = 'a'..'z' 'A'..'Z'
class high  = 0x80..0xff
class nul   = 0x00
class lf    = '\\n'
class op    = '{' '}' '[' ']' ',' ':'
class x     = 'x'
";

const POOL: [&str; 10] = ["quote", "bs", "ws", "digit", "alpha", "high", "nul", "lf", "op", "x"];

fn file(family: &str, what: &str, body: &str) -> String {
    format!("# corpus: {family}: {what}\nlexer corpus\n{CLASSES}{body}")
}

/// A Bitlex byte item: a quoted ASCII byte, or hex.
fn item(b: u8) -> String {
    match b {
        b'\'' => "'\\''".to_owned(),
        b'\\' => "'\\\\'".to_owned(),
        0x21..=0x7e => format!("'{}'", b as char),
        _ => format!("{b:#04x}"),
    }
}

/// Every corpus file: its name and its source.
fn generated() -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut add = |family: &str, what: &str, body: String| {
        let name = format!("{:04}-{family}-{}.bitlex", out.len(), what.replace([' ', ',', '(', ')'], "_").replace("__", "_").trim_end_matches('_'));
        out.push((name, file(family, what, &body)));
    };

    // --- class ------------------------------------------------------------------------------
    for name in POOL {
        add("class", &format!("{name} alone"), format!("mask m = {name}\n"));
    }
    for b in [0x00u8, 0x01, 0x09, 0x0a, 0x20, 0x22, 0x27, 0x30, 0x5c, 0x61, 0x7e, 0x7f, 0x80, 0xc3, 0xfe, 0xff] {
        add("class", &format!("byte {b:#04x}"), format!("class c = {}\nmask m = c\n", item(b)));
    }
    for (lo, hi) in [(0x00u8, 0x1f), (0x20, 0x7f), (0x30, 0x39), (0x41, 0x5a), (0x0f, 0x10), (0x7f, 0x80), (0x00, 0xff), (0xf0, 0xff), (0x08, 0x47)] {
        add("class", &format!("range {lo:#04x} to {hi:#04x}"), format!("class c = {}..{}\nmask m = c\n", item(lo), item(hi)));
    }
    // Classes of `k` products: `k` bytes, each its own high nibble and low nibble, so no two share
    // a product; past eight, a second table, past sixteen a third.
    for k in [1usize, 2, 3, 7, 8, 9, 15, 16, 17, 24] {
        let items: Vec<String> = (0..k).map(|i| item(((i % 16) as u8) << 4 | ((i * 7 + 3) % 16) as u8)).collect();
        add("class", &format!("{k} products"), format!("class c = {}\nmask m = c\n", items.join(" ")));
    }
    // Many classes in one program, each output.
    for n in [4usize, 12, 20, 33] {
        let mut body = String::new();
        let mut rng = Rng(0x1234_5678 + n as u64);
        for i in 0..n {
            let a = rng.below(256) as u8;
            let b = a.saturating_add(rng.below(40) as u8);
            let _ = writeln!(body, "class c{i} = {}..{} {}", item(a), item(b), item(rng.below(256) as u8));
            let _ = writeln!(body, "mask m{i} = c{i}");
        }
        add("class", &format!("{n} classes in one program"), body);
    }

    // --- logic ------------------------------------------------------------------------------
    for (a, b) in [("quote", "bs"), ("ws", "lf"), ("digit", "alpha"), ("high", "nul"), ("op", "x")] {
        for (op, sym) in [("and", "&"), ("or", "|"), ("xor", "^"), ("andnot", "& ~")] {
            add("logic", &format!("{a} {op} {b}"), format!("mask m = {a} {sym} {b}\n"));
        }
    }
    for name in ["quote", "nul", "high"] {
        add("logic", &format!("not {name}"), format!("mask m = ~{name}\nmask n = ~~{name}\n"));
    }
    add("logic", "empty", "mask m = empty\nmask n = ~empty\nmask o = quote | empty\nmask p = quote & ~empty\n".to_owned());
    add("logic", "self", "mask m = quote ^ quote\nmask n = quote & ~quote\nmask o = quote | quote\n".to_owned());
    for depth in 2..=7u64 {
        let mut rng = Rng(0xabc0 + depth);
        add("logic", &format!("nested depth {depth}"), format!("mask m = {}\n", logic(&mut rng, depth)));
    }

    // --- prev, next, span -------------------------------------------------------------------
    for k in 1..=63u64 {
        let c = POOL[k as usize % POOL.len()];
        add("prev", &format!("{c} by {k}"), format!("mask m = prev({c}, {k})\n"));
    }
    add("prev", "default distance", "mask m = prev(quote)\nmask n = prev(prev(quote))\n".to_owned());
    for (a, b) in [(1u64, 63u64), (32, 32), (63, 63), (7, 9), (40, 30)] {
        add("prev", &format!("of prev {a} then {b}"), format!("mask m = prev(prev(digit, {a}), {b})\n"));
    }
    add("prev", "of a carried value", "mask m = prev(toggle(quote), 5)\nmask n = prev(fill(quote, ws), 63)\n".to_owned());
    add("prev", "of nul at the end", "mask m = prev(nul, 1)\nmask n = prev(~nul, 63)\n".to_owned());
    for k in 1..=63u64 {
        let c = POOL[(k as usize * 3) % POOL.len()];
        add("next", &format!("{c} by {k}"), format!("mask m = next({c}, {k})\n"));
    }
    add("next", "default distance", "mask m = next(quote)\nmask n = next(next(quote))\n".to_owned());
    // Outputs at several delays in one program.
    for ks in [[1u64, 2, 3], [5, 17, 63], [63, 1, 32], [8, 8, 9]] {
        add(
            "next",
            &format!("outputs at {} {} {}", ks[0], ks[1], ks[2]),
            format!("mask a = quote\nmask b = next(digit, {})\nmask c = next(ws, {}) & prev(alpha, {})\nmask d = next(nul, {})\n", ks[0], ks[1], ks[2], ks[2]),
        );
    }
    add("next", "of carried values", "mask m = next(toggle(quote), 3)\nmask n = next(fill(quote, ws), 9)\nmask o = next(escaped(bs))\n".to_owned());
    add("next", "of dirty values", "mask m = next(nul, 4)\nmask n = next(~quote, 2)\nmask o = next(~empty)\n".to_owned());
    add("next", "inside prev", "mask m = prev(next(digit, 3), 2)\nmask n = next(prev(digit, 5), 5)\n".to_owned());
    for n in 1..=63u64 {
        let c = POOL[(n as usize * 7) % POOL.len()];
        add("span", &format!("{c} over {n}"), format!("mask m = span({c}, {n})\n"));
    }

    // --- edge -------------------------------------------------------------------------------
    for c in POOL {
        add("edge", &format!("starts lasts after of {c}"), format!("mask s = starts({c})\nmask l = lasts({c})\nmask a = after({c})\n"));
    }
    for n in [1u64, 2, 3, 4, 5, 7, 8, 16, 31, 32, 63] {
        add("edge", &format!("aligned by {n}"), format!("mask m = aligned(alpha | digit, {n})\nmask o = aligned(bs, {n})\n"));
    }
    add("edge", "of carried values", "mask s = starts(toggle(quote))\nmask l = lasts(toggle(quote))\nmask a = after(fill(quote, ws))\n".to_owned());

    // --- carry ------------------------------------------------------------------------------
    for c in POOL {
        add("carry", &format!("toggle of {c}"), format!("mask m = toggle({c})\n"));
        add("carry", &format!("after_odd_run of {c}"), format!("mask m = after_odd_run({c})\nmask e = escaped({c})\n"));
    }
    for (s, e) in [("quote", "quote"), ("quote", "ws"), ("digit", "~digit"), ("starts(alpha)", "~alpha"), ("op", "op | ws"), ("nul", "~nul"), ("high", "empty"), ("empty", "ws")] {
        add("carry", &format!("fill from {s} to {e}"), format!("mask m = fill({s}, {e})\n"));
    }
    add("carry", "toggle of toggle", "mask m = toggle(toggle(quote))\nmask n = toggle(toggle(quote) & bs)\n".to_owned());
    add("carry", "strings", "mask esc = escaped(bs)\nmask q = quote & ~esc\nmask inside = toggle(q)\nmask code = ~inside & ~q\n".to_owned());
    add("carry", "fill after prev", "mask m = fill(prev(quote) & alpha, ~alpha)\nmask n = fill(next(quote) & digit, ~digit)\n".to_owned());
    // Each carry after a `prev` at every distance: a lane's carry summary reads the `prev`'s
    // word, from the lanes before at 16-, 32- and 64-byte lanes (generated MSL once counted bits
    // above a 16-bit lane's word here).
    for (what, carry) in [("toggle", "toggle(P)"), ("fill", "fill(P, ws | op)"), ("escaped", "escaped(P)")] {
        let mut body = String::from("mask base = after(toggle(nul | quote))\n");
        for k in 1..=63 {
            let _ = writeln!(body, "mask c{k} = {}", carry.replace('P', &format!("prev(base, {k})")));
        }
        add("carry", &format!("{what} after prev at every distance"), body);
    }
    // Carried state of one word and of more than one: many `prev`s at long distances.
    for n in [1usize, 2, 3, 4, 6] {
        let mut body = String::new();
        for i in 0..n {
            let _ = writeln!(body, "mask p{i} = prev({}, {}) ^ toggle({})", POOL[i % 10], 63 - i, POOL[(i + 3) % 10]);
        }
        add("carry", &format!("{n} long carries"), body);
    }

    // --- count ------------------------------------------------------------------------------
    for max in [1u32, 2, 3, 4, 6, 7, 8, 15, 16] {
        add(
            "count",
            &format!("brackets max {max}"),
            format!("class open = '[' '{{'\nclass close = ']' '}}'\nscan depth = count(open, close) max {max}\nmask f = depth.frame(quote | open & ~x)\nmask g = depth.frame(open)\n"),
        );
    }

    // --- fallback ---------------------------------------------------------------------------
    add("fallback", "one error", "error \"a nul\" when nul\nmask m = quote\n".to_owned());
    add("fallback", "one escape", "escape \"high bytes\" when high\nmask m = quote\n".to_owned());
    add("fallback", "error and escape", "mask inside = toggle(quote & ~escaped(bs))\nerror \"newline in a string\" when inside & lf\nescape \"a slash\" when ~inside & x\n".to_owned());
    add("fallback", "delayed error", "error \"digit then alpha\" when digit & next(alpha, 2)\nmask m = digit\n".to_owned());
    add("fallback", "errors at several delays", "error \"a\" when quote & next(quote)\nerror \"b\" when bs & next(x, 9)\nescape \"c\" when prev(nul, 3) & op\n".to_owned());

    // --- mix --------------------------------------------------------------------------------
    for seed in 0..220u64 {
        let mut rng = Rng(0x5eed_0000_0000 + seed * 0x9e37_79b9);
        let masks = 2 + rng.below(6) as usize;
        let mut names: Vec<String> = POOL.iter().map(|s| (*s).to_owned()).collect();
        let mut body = String::new();
        for i in 0..masks {
            let depth = 1 + rng.below(4);
            let e = expr(&mut rng, &names, depth);
            let _ = writeln!(body, "mask m{i} = {e}");
            names.push(format!("m{i}"));
        }
        add("mix", &format!("seed {seed}"), body);
    }

    // --- tree -------------------------------------------------------------------------------
    for p in tree_params() {
        let (what, source) = tree_file(&p);
        let name = format!("{:04}-tree-{}.bitlex", out.len(), what.replace(' ', "_"));
        out.push((name, format!("# corpus: tree: {what}\n{source}")));
    }
    out
}

/// A tree grammar's parameters: its brackets, separator, atoms, pairs and root.
#[derive(Clone, Copy)]
struct TreeParams {
    /// `[]` only, or `[]` and `{}`.
    two_brackets: bool,
    /// `,`, `;`, or none (values follow each other).
    sep: Option<char>,
    atoms: Atoms,
    /// `{}` holds `key: atom ':' value: value` pairs (with two brackets).
    pairs: bool,
    /// The root is one value, or any number.
    many: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum Atoms {
    Words,
    /// Words, with keywords `true` and `null`.
    Keywords,
    /// Numbers, checked by a grammar over their bytes (tier 2's `CHECK`).
    Numbers,
}

fn tree_params() -> Vec<TreeParams> {
    let mut out = Vec::new();
    for two_brackets in [false, true] {
        for sep in [Some(','), Some(';'), None] {
            for atoms in [Atoms::Words, Atoms::Keywords, Atoms::Numbers] {
                for pairs in [false, true] {
                    for many in [false, true] {
                        if pairs && !two_brackets {
                            continue;
                        }
                        out.push(TreeParams { two_brackets, sep, atoms, pairs, many });
                    }
                }
            }
        }
    }
    out
}

fn tree_file(p: &TreeParams) -> (String, String) {
    let what = format!(
        "{} sep {} {} {} {}",
        if p.two_brackets { "two brackets" } else { "one bracket" },
        match p.sep {
            Some(',') => "comma",
            Some(_) => "semicolon",
            None => "none",
        },
        match p.atoms {
            Atoms::Words => "words",
            Atoms::Keywords => "keywords",
            Atoms::Numbers => "numbers",
        },
        if p.pairs { "pairs" } else { "values" },
        if p.many { "many roots" } else { "one root" },
    );
    let mut s = String::from("lexer corpus\n");
    let _ = writeln!(s, "class open  = '['{}", if p.two_brackets { " '{'" } else { "" });
    let _ = writeln!(s, "class close = ']'{}", if p.two_brackets { " '}'" } else { "" });
    let mut punct = vec!["open", "close"];
    if let Some(c) = p.sep {
        let _ = writeln!(s, "class sep   = '{c}'");
        punct.push("sep");
    }
    if p.pairs {
        let _ = writeln!(s, "class colon = ':'");
        punct.push("colon");
    }
    s.push_str("class ws    = ' ' '\\n' '\\t'\nclass digit = '0'..'9'\nclass w     = 'a'..'z' '0'..'9' '-' '.'\n");
    let _ = writeln!(s, "token punct = start {}", punct.join(" | "));
    let mut words = vec!["\"[\"", "\"]\""];
    if p.two_brackets {
        words.extend(["\"{\"", "\"}\""]);
    }
    let sep = p.sep.map(|c| format!("\"{c}\""));
    if let Some(c) = &sep {
        words.push(c);
    }
    if p.pairs {
        words.push("\":\"");
    }
    let _ = writeln!(s, "keywords punct {{ {} }} required", words.join(" "));
    s.push_str("token atom  = start starts(w) last lasts(w)\n");
    match p.atoms {
        Atoms::Words => {}
        Atoms::Keywords => s.push_str("keywords atom { \"true\" -> true  \"null\" -> null }\n"),
        Atoms::Numbers => s.push_str("check atom = ['-'] digit+ ['.' digit*]\n"),
    }
    s.push_str("skip ws\nerror \"a stray byte\" when ~(open | close | ws | w");
    if p.sep.is_some() {
        s.push_str(" | sep");
    }
    if p.pairs {
        s.push_str(" | colon");
    }
    s.push_str(")\n");
    let list = |o: &str, c: &str, item: &str| match p.sep {
        Some(x) => format!("'{o}' [{item} ('{x}' {item})*] '{c}'"),
        None => format!("'{o}' {item}* '{c}'"),
    };
    s.push_str("tree t {\n");
    let _ = writeln!(s, "  node doc = {}", if p.many { "value*" } else { "value" });
    let _ = writeln!(s, "  node list nested = {}", list("[", "]", "value"));
    let mut choices = vec!["list"];
    if p.two_brackets {
        let _ = writeln!(s, "  node obj nested = {}", list("{", "}", if p.pairs { "pair" } else { "value" }));
        choices.push("obj");
    }
    if p.pairs {
        s.push_str("  node pair = key: atom ':' value: value\n");
    }
    choices.push("atom");
    if p.atoms == Atoms::Keywords {
        choices.extend(["true", "null"]);
    }
    let _ = writeln!(s, "  choice value supertype _value = {}", choices.join(" | "));
    s.push_str("}\n");
    (what, s)
}

/// A logic expression over the pool, `depth` deep.
fn logic(rng: &mut Rng, depth: u64) -> String {
    if depth == 0 {
        return (*rng.pick(&POOL)).to_owned();
    }
    let a = logic(rng, depth - 1);
    let other = rng.below(depth);
    let b = logic(rng, other);
    match rng.below(5) {
        0 => format!("({a} & {b})"),
        1 => format!("({a} | {b})"),
        2 => format!("({a} ^ {b})"),
        3 => format!("({a} & ~{b})"),
        _ => format!("~{a}"),
    }
}

/// A random mask expression over `names`, about `depth` deep, using every operation.
fn expr(rng: &mut Rng, names: &[String], depth: u64) -> String {
    if depth == 0 || rng.below(5) == 0 {
        return rng.pick(names).clone();
    }
    let d = depth - 1;
    let k = 1 + rng.below(63);
    match rng.below(16) {
        0 => format!("({} & {})", expr(rng, names, d), expr(rng, names, d)),
        1 => format!("({} | {})", expr(rng, names, d), expr(rng, names, d)),
        2 => format!("({} ^ {})", expr(rng, names, d), expr(rng, names, d)),
        3 => format!("~{}", expr(rng, names, d)),
        4 => format!("prev({}, {k})", expr(rng, names, d)),
        5 => format!("next({}, {})", expr(rng, names, d), 1 + rng.below(12)),
        6 => format!("starts({})", expr(rng, names, d)),
        7 => format!("lasts({})", expr(rng, names, d)),
        8 => format!("after({})", expr(rng, names, d)),
        9 => format!("span({}, {})", expr(rng, names, d), 1 + rng.below(20)),
        10 => format!("toggle({})", expr(rng, names, d)),
        11 => format!("fill({}, {})", expr(rng, names, d), expr(rng, names, d)),
        12 => format!("after_odd_run({})", expr(rng, names, d)),
        13 => format!("escaped({})", expr(rng, names, d)),
        14 => format!("aligned({}, {})", expr(rng, names, d), 1 + rng.below(9)),
        _ => format!("({} & ~{})", expr(rng, names, d), expr(rng, names, d)),
    }
}

/// The corpus as it is on disk, in name order.
fn on_disk() -> Vec<(String, String)> {
    let mut files: Vec<(String, String)> = std::fs::read_dir(DIR)
        .unwrap_or_else(|e| panic!("{DIR}: {e}"))
        .map(|e| e.expect("an entry").path())
        .filter(|p| p.extension().is_some_and(|x| x == "bitlex"))
        .map(|p| (p.file_name().expect("a name").to_string_lossy().into_owned(), std::fs::read_to_string(&p).expect("reads")))
        .collect();
    files.sort();
    files
}

#[test]
#[ignore = "writes the corpus; run by name with RERAIL_WRITE_CORPUS=1"]
fn write_the_corpus() {
    assert!(std::env::var_os("RERAIL_WRITE_CORPUS").is_some(), "set RERAIL_WRITE_CORPUS=1");
    std::fs::create_dir_all(DIR).expect("makes the directory");
    for (name, _) in on_disk() {
        std::fs::remove_file(format!("{DIR}/{name}")).expect("removes an old file");
    }
    for (name, source) in generated() {
        std::fs::write(format!("{DIR}/{name}"), source).expect("writes");
    }
}

#[test]
fn the_corpus_is_the_generators() {
    let (disk, want) = (on_disk(), generated());
    let names = |f: &[(String, String)]| f.iter().map(|x| x.0.clone()).collect::<Vec<_>>();
    assert_eq!(names(&disk), names(&want), "rewrite the corpus (write_the_corpus)");
    for ((name, got), (_, want)) in disk.iter().zip(&want) {
        assert_eq!(got, want, "{name}: rewrite the corpus (write_the_corpus)");
    }
    assert!(disk.len() >= 500, "{} files", disk.len());
}

/// The inputs every file runs on: every length to 200, lengths around blocks and Metal's group
/// spans, and runs that make carries long.
fn corpus_inputs() -> Vec<Vec<u8>> {
    const BYTES: &[u8] = b"\"\\ \t\n\r0123456789azAZ{}[],:x\0\x80\xff\xc3";
    let mut rng = Rng(0xc0ff_ee00_1234_5678);
    let mut make = |len: usize| -> Vec<u8> { (0..len).map(|_| BYTES[rng.below(BYTES.len() as u64) as usize]).collect() };
    let mut out: Vec<Vec<u8>> = (0..=200).map(&mut make).collect();
    for edge in [256usize, 512, 1024, 2048, 4096] {
        for len in [edge - 1, edge, edge + 1] {
            out.push(make(len));
        }
    }
    out.extend(super::block_tests::runs());
    out
}

/// A compiled corpus file: its program, and each checked name's output.
struct Case {
    name: String,
    stmts: Vec<Stmt>,
    program: Program,
    checked: Vec<(String, usize)>,
    fallbacks: Vec<usize>,
    /// It has a count: below zero the compiled program falls back where the evaluator holds 0.
    counts: bool,
}

fn compile_case(name: &str, source: &str) -> Result<Case, String> {
    let stmts = parse(source).unwrap_or_else(|e| panic!("{name}: parses: {e}"));
    let mut names: Vec<String> = Vec::new();
    for s in &stmts {
        match s {
            Stmt::Class(n, _) | Stmt::Mask(n, _) => names.push(n.clone()),
            _ => {}
        }
    }
    let falls = stmts.iter().any(|s| matches!(s, Stmt::Error(..) | Stmt::Escape(..) | Stmt::Count { .. }));
    let mut outputs: Vec<&str> = names.iter().map(String::as_str).collect();
    if falls {
        outputs.push(bitlex_compile::FALLBACK);
    }
    let compiled = bitlex_compile::compile(&stmts, &outputs, None).map_err(|e| format!("{name}: {e}"))?;
    let text = format!("program -\n  mode set=0 query=0\n{}  0000 STOP 0\n", compiled.rows);
    let program = rir::assemble(&text).unwrap_or_else(|e| panic!("{name}: assembles: {e}\n{text}"));
    let label = |n: &str| usize::from(compiled.labels.iter().find(|l| l.0 == n).unwrap_or_else(|| panic!("{name}: no output {n}")).1);
    let checked = names.iter().map(|n| (n.clone(), label(n))).collect();
    let fallbacks = compiled.fallbacks.iter().map(|&k| usize::from(k)).collect();
    let counts = stmts.iter().any(|s| matches!(s, Stmt::Count { .. }));
    Ok(Case { name: name.to_owned(), stmts, program, checked, fallbacks, counts })
}

fn cut(mut words: Vec<u64>, n: usize) -> Vec<u64> {
    words.truncate(n.div_ceil(64));
    if n % 64 != 0
        && let Some(last) = words.last_mut()
    {
        *last &= (1u64 << (n % 64)) - 1;
    }
    words
}

/// Check 2: the interpreter's outputs for `case` over `bytes` against the evaluator's masks:
/// whether the file fell back, or the first difference.
fn against_the_evaluator(case: &Case, bytes: &[u8], got: &[Vec<u64>]) -> Result<bool, String> {
    let n = bytes.len();
    let at = || format!("{n} bytes: {:?}", String::from_utf8_lossy(&bytes[..n.min(80)]));
    let out = ::bitlex::eval::Ev::run(&case.stmts, bytes, &HashMap::new()).map_err(|e| format!("evaluates: {e}"))?;
    // The corpus has no tokens: the evaluator's "in no token" errors are not the program's.
    let errors: Vec<&String> = out.errors.iter().filter(|e| !e.contains("is in no token")).collect();
    let fell = case.fallbacks.iter().any(|&k| cut(got[k].clone(), n).iter().any(|&w| w != 0));
    if case.counts {
        // Above the bound both reject; below zero only the compiled program does (it escapes,
        // `bitlex_compile`'s counts). Masks are compared where neither falls back.
        if !errors.is_empty() && !fell {
            return Err(format!("the evaluator rejects ({errors:?}) and the program does not, {}", at()));
        }
        if fell {
            return Ok(true);
        }
    } else if !case.fallbacks.is_empty() && fell == errors.is_empty() {
        return Err(format!("falls back: {fell}, the evaluator's errors: {errors:?}, {}", at()));
    }
    for (name, k) in &case.checked {
        let want = match out.vars.get(name) {
            Some(::bitlex::eval::Val::M(m)) => m.w.clone(),
            _ => return Err(format!("{name} is not a mask")),
        };
        let got = cut(got[*k].clone(), n);
        if got != want {
            return Err(format!("{name}: {got:x?} against {want:x?}, {}", at()));
        }
    }
    Ok(fell)
}

/// The corpus compiled: every file, or the ones the compiler refuses.
fn compiled_corpus() -> Vec<Case> {
    let (mut cases, mut refused) = (Vec::new(), Vec::new());
    // The tree files are checked whole, through tier 2 (`corpus_trees_equal_the_evaluator`).
    // `RERAIL_CORPUS_FILES`: only the files whose names hold one of its comma-separated parts.
    let only = std::env::var("RERAIL_CORPUS_FILES").ok();
    let wanted = |n: &str| only.as_deref().is_none_or(|o| o.split(',').any(|p| n.contains(p)));
    for (n, s) in on_disk().into_iter().filter(|(n, _)| !n.contains("-tree-") && wanted(n)) {
        match compile_case(&n, &s) {
            Ok(c) => cases.push(c),
            Err(e) => refused.push(e),
        }
    }
    assert!(refused.is_empty(), "{} files the compiler refuses:\n{}", refused.len(), refused.join("\n"));
    cases
}

/// Each file's runs: a batch of every file over every input (decision 0044).
fn batch<'a>(cases: &[Case], inputs: &'a [Vec<u8>]) -> Vec<block::BatchFile<'a>> {
    (0..cases.len()).flat_map(|p| inputs.iter().map(move |b| block::BatchFile { bytes: b, program: p })).collect()
}

fn seconds(since: std::time::Instant) -> f64 {
    since.elapsed().as_secs_f64()
}

/// Check 2: on every input, the CPU interpreter gives the evaluator's masks for every file.
#[test]
fn corpus_equals_the_evaluator() {
    let t = std::time::Instant::now();
    let cases = compiled_corpus();
    let inputs = corpus_inputs();
    let blocks: Vec<&block::BlockProgram> = cases.iter().map(|c| &c.program.block_programs()[0]).collect();
    let files = batch(&cases, &inputs);
    let runs = block::run_batch(&blocks, &files);
    eprintln!("compiled and interpreted in {:.1} s", seconds(t));
    let mut differ: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    let mut fell = 0usize;
    for (f, run) in files.iter().zip(&runs) {
        let case = &cases[f.program];
        if differ.contains_key(&case.name) {
            continue;
        }
        match against_the_evaluator(case, f.bytes, &run.outputs) {
            Ok(x) => fell += usize::from(x),
            Err(e) => {
                differ.insert(case.name.clone(), e);
            }
        }
    }
    eprintln!("{} files, {} inputs, {fell} runs fell back, {:.1} s", cases.len(), inputs.len(), seconds(t));
    let list: Vec<String> = differ.iter().map(|(n, e)| format!("{n}: {e}")).collect();
    assert!(list.is_empty(), "{} files differ from the evaluator:\n{}", list.len(), list.join("\n"));
}

/// Check 1 on Cranelift.
#[test]
fn corpus_cranelift_equals_the_interpreter() {
    executors_equal_the_interpreter(false);
}

/// Check 1 on the Metal executors (minutes: Metal compiles generated MSL slowly).
#[test]
#[ignore = "minutes of Metal compiling; run by name"]
fn corpus_metal_equals_the_interpreter() {
    executors_equal_the_interpreter(true);
}

/// Check 1: every compiled executor gives the CPU interpreter's outputs, bit for bit, for every
/// file on every input. Each executor runs the corpus as batches (decision 0044).
fn executors_equal_the_interpreter(metal: bool) {
    let t = std::time::Instant::now();
    let cases = compiled_corpus();
    let inputs = corpus_inputs();
    let blocks: Vec<&block::BlockProgram> = cases.iter().map(|c| &c.program.block_programs()[0]).collect();
    let files = batch(&cases, &inputs);
    let want: Vec<Vec<Vec<u64>>> = block::run_batch(&blocks, &files).into_iter().map(|r| r.outputs).collect();
    let natives: Vec<native_ir::BlockNative> = cases.iter().map(|c| native_ir::lower(&c.program).expect("lowers").block_programs.remove(0)).collect();
    let compiled: Vec<native_cpu_block::CompiledBlock> =
        natives.iter().zip(&cases).map(|(n, c)| native_cpu_block::compile(n).unwrap_or_else(|e| panic!("{}: cranelift: {e:?}", c.name))).collect();
    let refs: Vec<&native_cpu_block::CompiledBlock> = compiled.iter().collect();
    eprintln!("interpreted and compiled in {:.1} s", seconds(t));
    let mut runs: Vec<(String, Vec<Vec<Vec<u64>>>)> =
        vec![("cranelift".to_owned(), native_cpu_block::run_batch(&refs, &files).into_iter().map(|r| r.outputs).collect())];
    #[cfg(target_os = "macos")]
    if metal {
        use metal_program_backend::block_program::{ClassStrategy, Executor, LaneWidth, run_split};
        let forms: Vec<&native_ir::BlockNative> = natives.iter().collect();
        for width in LaneWidth::ALL {
            let mut executors = vec![(format!("metal interpreter, {}-byte lanes", width.bytes()), Executor::Interpreter)];
            for s in ClassStrategy::ALL {
                executors.push((format!("generated msl, {}-byte lanes, {s:?}", width.bytes()), Executor::Generated(s)));
            }
            // `RERAIL_CORPUS_EXECUTORS`: only the executors whose names hold it.
            let only = std::env::var("RERAIL_CORPUS_EXECUTORS").ok();
            executors.retain(|(n, _)| only.as_deref().is_none_or(|o| n.contains(o)));
            for (name, executor) in executors {
                // Generated MSL in chunks of programs: one source of every program takes Metal's
                // compiler minutes; its time grows faster than the source.
                let chunk = std::env::var("RERAIL_CORPUS_CHUNK").ok().and_then(|c| c.parse().ok()).unwrap_or(32usize);
                let mut got: Vec<Vec<Vec<u64>>> = Vec::with_capacity(files.len());
                for (c, programs) in forms.chunks(chunk).enumerate() {
                    let base = c * chunk;
                    let mine: Vec<block::BatchFile<'_>> = files
                        .iter()
                        .filter(|f| (base..base + programs.len()).contains(&f.program))
                        .map(|f| block::BatchFile { bytes: f.bytes, program: f.program - base })
                        .collect();
                    let run = run_split(programs, &mine, width, executor, 64).unwrap_or_else(|e| panic!("{name}: {e:?}"));
                    got.extend(run.files.into_iter().map(|r| r.outputs));
                }
                eprintln!("{name}: {:.1} s", seconds(t));
                runs.push((name, got));
            }
        }
    }
    let mut differ: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    for (executor, got) in &runs {
        assert_eq!(got.len(), files.len(), "{executor}");
        for ((f, got), want) in files.iter().zip(got).zip(&want) {
            let case = &cases[f.program];
            if let Some(k) = (0..want.len()).find(|&k| got[k] != want[k]) {
                differ.entry(format!("{}, {executor}", case.name)).or_insert_with(|| format!("output {k}: {:x?} against {:x?}, {} bytes: {:?}", got[k], want[k], f.bytes.len(), String::from_utf8_lossy(f.bytes)));
            }
        }
    }
    eprintln!("{} files, {} inputs, {} executors, {:.1} s", cases.len(), inputs.len(), runs.len() + 1, seconds(t));
    let list: Vec<String> = differ.iter().map(|(n, e)| format!("{n}: {e}")).collect();
    assert!(list.is_empty(), "{} runs differ from the interpreter:\n{}", list.len(), list.join("\n"));
}

/// The tree grammars the compiler refuses today, by reason (docs/parser-generators/20-the-ir.md §7).
const GAPS: [&str; 0] = [];

/// Ids by name, for a corpus grammar: each kind or field is numbered at its first use, from 1.
#[derive(Default)]
struct Ids(std::cell::RefCell<HashMap<String, u16>>);

impl Ids {
    fn of(&self, name: &str) -> u16 {
        let mut m = self.0.borrow_mut();
        let n = m.len() as u16 + 1;
        *m.entry(name.to_owned()).or_insert(n)
    }
}

/// A node as ids: kind, field, supertypes (sorted), start, end, size.
type IdNode = (u16, u16, Vec<u16>, usize, usize, usize);

/// Documents of `p`'s language: random values, nested, with random spacing; then each with one
/// byte changed, dropped or doubled, so that many go to tree-sitter; and long atoms, deep
/// nesting and the empty file.
fn tree_inputs(p: &TreeParams, seed: u64) -> Vec<Vec<u8>> {
    let mut rng = Rng(0x7a3e_0000 ^ seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
    fn value(rng: &mut Rng, p: &TreeParams, depth: u32, out: &mut String) {
        let space = |rng: &mut Rng, out: &mut String| {
            for _ in 0..rng.below(3) {
                out.push(*rng.pick(&[' ', '\n', '\t']));
            }
        };
        space(rng, out);
        let kind = if depth == 0 { 0 } else { rng.below(if p.two_brackets { 4 } else { 3 }) };
        match kind {
            1 | 3 => {
                let (o, c) = if kind == 1 { ('[', ']') } else { ('{', '}') };
                out.push(o);
                let n = rng.below(4);
                for i in 0..n {
                    if i > 0 {
                        match p.sep {
                            Some(x) => out.push(x),
                            None => out.push(' '),
                        }
                    }
                    if kind == 3 && p.pairs {
                        space(rng, out);
                        atom(rng, p, out);
                        space(rng, out);
                        out.push(':');
                    }
                    value(rng, p, depth - 1, out);
                }
                space(rng, out);
                out.push(c);
            }
            _ => atom(rng, p, out),
        }
        space(rng, out);
    }
    fn atom(rng: &mut Rng, p: &TreeParams, out: &mut String) {
        let words: &[&str] = match p.atoms {
            Atoms::Words => &["a", "x1", "abc", "-z.", "9"],
            Atoms::Keywords => &["true", "null", "truex", "nul", "q"],
            Atoms::Numbers => &["0", "-12", "3.5", "7.", "-", "1a", "00", "4.2.1"],
        };
        out.push_str(rng.pick(words));
    }
    let mut out: Vec<Vec<u8>> = vec![Vec::new(), b" ".to_vec()];
    for _ in 0..40 {
        let mut doc = String::new();
        let roots = if p.many { rng.below(3) + 1 } else { 1 };
        for _ in 0..roots {
            let depth = 1 + rng.below(4) as u32;
            value(&mut rng, p, depth, &mut doc);
        }
        out.push(doc.into_bytes());
    }
    let valid = out.len();
    for i in 2..valid {
        let mut d = out[i].clone();
        if d.is_empty() {
            continue;
        }
        let at = rng.below(d.len() as u64) as usize;
        match rng.below(3) {
            0 => d[at] = *rng.pick(b"[]{},;: a9-?"),
            1 => {
                d.remove(at);
            }
            _ => d.insert(at, d[at]),
        }
        out.push(d);
    }
    let long = match p.atoms {
        Atoms::Numbers => "7".repeat(150),
        _ => "a".repeat(150),
    };
    out.push(format!("[{long}]").into_bytes());
    out.push(format!("{}{long}{}", "[".repeat(40), "]".repeat(40)).into_bytes());
    out
}

/// The reference's nodes for `bytes`, or `None` where the file goes to tree-sitter.
fn reference_nodes(stmts: &[Stmt], bytes: &[u8], ids: &Ids) -> Option<Vec<IdNode>> {
    let out = ::bitlex::eval::Ev::run(stmts, bytes, &HashMap::new()).ok()?;
    if !out.errors.is_empty() {
        return None;
    }
    let nodes = ::bitlex::tree::nodes(out.tree.as_ref()?, &out.tokens, bytes.len()).ok()?;
    Some(
        nodes
            .iter()
            .map(|n| {
                let field = n.field.as_deref().map_or(0, |f| ids.of(&format!("field {f}")));
                let mut supers: Vec<u16> = n.supertypes.iter().map(|s| ids.of(s)).collect();
                supers.sort_unstable();
                (ids.of(&n.kind), field, supers, n.start, n.end, n.size)
            })
            .collect(),
    )
}

/// Tier 1 on the interpreter, then tier 2's words on theirs: the nodes, or `None` where the
/// file goes to tree-sitter (a fallback bit, or a word that refuses).
fn tier2_nodes(p: &Program, fallbacks: &[u16], bytes: &[u8]) -> Option<Vec<IdNode>> {
    let run = p.block_programs()[0].run_files(&[bytes]).remove(0);
    if fallbacks.iter().any(|&k| run.outputs[usize::from(k)].iter().any(|&w| w != 0)) {
        return None;
    }
    let nodes = stream::run(p, bytes, &run.positions[0], &run.positions[1]).ok()?;
    Some(
        nodes
            .into_iter()
            .map(|n| {
                let mut supers = n.supers;
                supers.sort_unstable();
                (n.kind, n.field, supers, n.start as usize, n.end as usize, n.size as usize)
            })
            .collect(),
    )
}

/// Every tree file of the corpus, compiled whole (tier 1's block program and tier 2's words),
/// writes the reference parser's nodes on its documents, and goes to tree-sitter exactly where
/// the reference does.
#[test]
fn corpus_trees_equal_the_evaluator() {
    let params = tree_params();
    let files: Vec<(String, String)> = on_disk().into_iter().filter(|(n, _)| n.contains("-tree-")).collect();
    assert_eq!(files.len(), params.len());
    let (mut refused, mut differ) = (Vec::new(), Vec::new());
    let (mut parsed, mut fell) = (0usize, 0usize);
    for (k, ((name, source), p)) in files.iter().zip(&params).enumerate() {
        let stmts = parse(source).unwrap_or_else(|e| panic!("{name}: parses: {e}"));
        let ids = Ids::default();
        let kind = |n: &str| Some(ids.of(n));
        let field = |n: &str| Some(ids.of(&format!("field {n}")));
        let symbols = bitlex_compile::Symbols { kind: &kind, field: &field };
        let compiled = match bitlex_compile::compile(&stmts, &[bitlex_compile::FALLBACK], Some(&symbols)) {
            Ok(c) => c,
            Err(e) => {
                refused.push(format!("{name}: {e}"));
                continue;
            }
        };
        let text = compiled.text();
        let program = rir::assemble(&text).unwrap_or_else(|e| panic!("{name}: assembles: {e}\n{text}"));
        for bytes in tree_inputs(p, k as u64) {
            let want = reference_nodes(&stmts, &bytes, &ids);
            let got = tier2_nodes(&program, &compiled.fallbacks, &bytes);
            if got != want {
                differ.push(format!("{name}: {:?}: tier 2 {got:?}, the reference {want:?}", String::from_utf8_lossy(&bytes)));
                break;
            }
            if want.is_some() { parsed += 1 } else { fell += 1 }
        }
    }
    eprintln!("{} tree files, {parsed} documents parsed, {fell} to tree-sitter", files.len());
    assert!(differ.is_empty(), "{} tree files differ from the reference:\n{}", differ.len(), differ.join("\n"));
    // The compiler's known gaps, each by its reason: a grammar the compiler refuses for any other
    // reason fails here, and one it compiles now must leave this list.
    let gap = |e: &String| GAPS.iter().any(|g| e.contains(g));
    let unknown: Vec<&String> = refused.iter().filter(|e| !gap(e)).collect();
    assert!(unknown.is_empty(), "tree files the compiler refuses:\n{unknown:#?}");
    for g in GAPS {
        assert!(refused.iter().any(|e| e.contains(g)), "no file is refused for `{g}`: the gap is closed, remove it");
    }
    eprintln!("{} tree files refused for the known gaps", refused.len());
    let compiled = files.len() - refused.len();
    assert!(parsed > 10 * compiled && fell > 10 * compiled, "{parsed} parsed, {fell} to tree-sitter, {compiled} files");
}

/// For a difference found above: one Bitlex source (`RERAIL_BITLEX`, a path) on one input
/// (`RERAIL_INPUT`, a path), each mask on the interpreter and generated MSL at every width.
#[test]
#[ignore = "a debugging aid; run by name with RERAIL_BITLEX and RERAIL_INPUT"]
fn a_file_on_every_executor() {
    let path = std::env::var("RERAIL_BITLEX").expect("RERAIL_BITLEX");
    let bytes = std::fs::read(std::env::var("RERAIL_INPUT").expect("RERAIL_INPUT")).expect("reads");
    let source = std::fs::read_to_string(&path).expect("reads");
    // A `.rir` file is a program as it is, each output by its number.
    let case = if path.ends_with(".rir") {
        let program = rir::assemble(&source).unwrap_or_else(|e| panic!("{e}"));
        let checked = (0..program.block_programs()[0].outputs.len()).map(|k| (format!("out {k}"), k)).collect();
        Case { name: path.clone(), stmts: Vec::new(), program, checked, fallbacks: Vec::new(), counts: false }
    } else {
        compile_case(&path, &source).unwrap_or_else(|e| panic!("{e}"))
    };
    let want = case.program.block_programs()[0].interpret(&bytes);
    let native = native_ir::lower(&case.program).expect("lowers").block_programs.remove(0);
    for (name, k) in &case.checked {
        eprintln!("{name:>8} interpreter {:x?}", want[*k]);
    }
    #[cfg(target_os = "macos")]
    {
        use metal_program_backend::block_program::{ClassStrategy, LaneWidth, generate_with, run_generated_with};
        if let Some(dump) = std::env::var_os("RERAIL_DUMP_MSL") {
            std::fs::write(&dump, generate_with(&native.planes_form(), LaneWidth::W16, ClassStrategy::Planes)).expect("writes");
        }
        for width in LaneWidth::ALL {
            let (got, _) = run_generated_with(&native, &bytes, width, ClassStrategy::Planes).expect("runs");
            for (name, k) in &case.checked {
                if got[*k] != want[*k] {
                    eprintln!("{name:>8} msl {:>2} bytes {:x?}", width.bytes(), got[*k]);
                }
            }
        }
    }
}
