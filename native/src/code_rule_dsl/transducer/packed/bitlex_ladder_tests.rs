//! The Bitlex ladder: thirteen languages, from one keyword to a data language with nested
//! records, lists, texts, escapes, numbers and two kinds of comment, a step at a time. Each is
//! parsed by its Bitlex file alone (`bitlex_ladder/`), with no Rust between the bytes and the
//! nodes.
//!
//! The languages are made up, so that no step is a known format: records are `< >`, lists
//! `( )`, items are separated by `;`, an entry is `#key# = value`, texts are between `#`, a caret
//! escapes, the booleans are `vrai` and `fausse`, nothing is `nul`, a number has a tilde for minus
//! and a decimal comma (`~3,14e~2`), and comments are `-- …` and `{- … -}`.
//!
//! Each step adds one construct to the one before. No step's test code knows its language: the
//! documents are drawn from the step's own tree grammar, its tokens' keywords and checks, and
//! then mutated a byte at a time.
//!
//! For each step and document, the same nodes (or the same refusal, where the file goes to
//! tree-sitter) from:
//!
//! 1. the reference: the evaluator and tree parser of `tools/bitlex`, which run Bitlex without
//!    the compiler;
//! 2. the interpreters: tier 1's block program, then tier 2's words;
//! 3. the compiled path, `native_cpu_stream::Parser`: one Cranelift function per tier, tier 2
//!    reading tier 1's buffers as written.
//!
//! Tier 1 on Cranelift and on Metal (generated MSL) equals its interpreter bit for bit. And each
//! step has the shape the compiler promises: tier 1's body has no branch but the loop's and the
//! extracts' flattening (it is branchless per block), and tier 2 has one loop, over the tokens
//! (and one over 64-byte blocks inside each `check`); neither calls anything.

use super::bitlex_corpus_tests::{Ids, Rng};
use super::native_cpu_shape::Step;
use super::*;
use ::bitlex::syntax::{Gram, Stmt, parse};
use std::collections::HashMap;

pub(super) const DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/src/code_rule_dsl/transducer/packed/bitlex_ladder");

/// The ladder's files, in order.
fn steps() -> Vec<(String, String)> {
    steps_in(DIR)
}

/// The steps of a ladder in `dir`, in order: its `.bitlex` files (not those of its
/// subdirectories, which are other ladders).
pub(super) fn steps_in(dir: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = std::fs::read_dir(dir)
        .expect("the ladder's directory")
        .map(|e| e.expect("an entry").path())
        .filter(|p| p.extension().is_some_and(|x| x == "bitlex"))
        .map(|p| (p.file_stem().unwrap().to_string_lossy().into_owned(), std::fs::read_to_string(&p).expect("reads")))
        .collect();
    out.sort();
    // `RERAIL_LADDER`: only the steps whose names hold it.
    let only = std::env::var("RERAIL_LADDER").ok();
    out.retain(|(n, _)| only.as_deref().is_none_or(|o| n.contains(o)));
    out
}

/// A step's seed, from its name: the same documents whichever steps run.
pub(super) fn seed(name: &str) -> u64 {
    name.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3))
}

/// What a step's documents are drawn from: its tree rules, and its tokens' keywords and checks.
pub(super) struct Lang {
    rules: HashMap<String, Gram>,
    root: String,
    /// A keyword's kind to its bytes (a keyword without `->` is its own kind).
    keywords: HashMap<String, Vec<Vec<u8>>>,
    checks: HashMap<String, Gram>,
    classes: HashMap<String, Vec<(u32, u32)>>,
    /// The fewest tokens each rule can match, for a document's last level.
    least: HashMap<String, usize>,
    /// Kinds that may come between any two tokens (`extra`).
    extras: Vec<String>,
    /// Sample texts of token kinds, from the ladder (`check_ladder`): drawn before any other.
    samples: HashMap<String, Vec<Vec<u8>>>,
    /// The marks' kinds (`mark K = E`): line breaks, here; a sentence writes one as a `\n`.
    marks: Vec<String>,
}

impl Lang {
    pub(super) fn of(stmts: &[Stmt]) -> Lang {
        let (mut rules, mut order, mut keywords, mut checks, mut classes) = (HashMap::new(), Vec::new(), HashMap::new(), HashMap::new(), HashMap::new());
        let mut extras = Vec::new();
        let mut marks = Vec::new();
        for s in stmts {
            match s {
                Stmt::Mark { kind, .. } => marks.push(kind.clone()),
                Stmt::Class(n, r) => {
                    classes.insert(n.clone(), r.clone());
                }
                Stmt::Keywords { kind, words, .. } => {
                    for (w, to) in words {
                        let _ = kind;
                        let k = to.clone().unwrap_or_else(|| String::from_utf8_lossy(w).into_owned());
                        keywords.entry(k).or_insert_with(Vec::new).push(w.clone());
                    }
                }
                Stmt::Tree { body, .. } => {
                    for b in body {
                        match b {
                            Stmt::Node { kind, body, .. } | Stmt::Choice { name: kind, body, .. } => {
                                order.push(kind.clone());
                                rules.insert(kind.clone(), body.clone());
                            }
                            Stmt::Check { kind, body } => {
                                checks.insert(kind.clone(), body.clone());
                            }
                            Stmt::Extra(kinds) => extras.extend(kinds.iter().cloned()),
                            _ => {}
                        }
                    }
                }
                Stmt::Check { kind, body } => {
                    checks.insert(kind.clone(), body.clone());
                }
                _ => {}
            }
        }
        let mut lang = Lang { rules, root: order[0].clone(), keywords, checks, classes, least: HashMap::new(), extras, samples: HashMap::new(), marks };
        // The fewest tokens of each rule: a fixpoint from "unreachable".
        for _ in 0..32 {
            for name in &order {
                let n = lang.cost(&lang.rules[name]);
                lang.least.insert(name.clone(), n);
            }
        }
        lang
    }

    fn cost(&self, g: &Gram) -> usize {
        const FAR: usize = 1 << 20;
        match g {
            Gram::Lit(_) => 1,
            Gram::Sym(s) => match self.rules.get(s) {
                Some(_) => self.least.get(s).copied().unwrap_or(FAR),
                None => 1,
            },
            Gram::Seq(gs) => gs.iter().map(|g| self.cost(g)).fold(0, |a, b| (a + b).min(FAR)),
            Gram::Alt(gs) => gs.iter().map(|g| self.cost(g)).min().unwrap_or(FAR),
            Gram::Opt(_) | Gram::Star(_) => 0,
            Gram::Plus(g) | Gram::Field(_, g) | Gram::Retag(_, g) | Gram::Merge(_, g) | Gram::As(_, g) => self.cost(g),
            Gram::Wrap { body, .. } | Gram::Relabel { body, .. } => self.cost(body),
            Gram::Balanced { .. } => 2,
        }
    }

    /// A random sentence of `g`: its tokens' bytes, in order.
    fn derive(&self, rng: &mut Rng, rep: &Gram, depth: u32, out: &mut Vec<Vec<u8>>) {
        let deep = depth == 0;
        match rep {
            Gram::Lit(k) => out.push(self.keyword(rng, k)),
            Gram::Sym(s) => match self.rules.get(s) {
                Some(r) => self.derive(rng, r, depth.saturating_sub(1), out),
                None => out.push(self.token(rng, s)),
            },
            Gram::Seq(gs) => gs.iter().for_each(|g| self.derive(rng, g, depth, out)),
            Gram::Alt(gs) => {
                let g = if deep { gs.iter().min_by_key(|g| self.cost(g)).unwrap() } else { rng.pick(gs) };
                self.derive(rng, g, depth, out)
            }
            Gram::Opt(g) => {
                if !deep && rng.below(2) == 0 {
                    self.derive(rng, g, depth, out)
                }
            }
            Gram::Star(g) | Gram::Plus(g) => {
                let plus = u64::from(matches!(rep, Gram::Plus(_)));
                let n = if deep { plus } else { plus + rng.below(4) };
                for _ in 0..n {
                    self.derive(rng, g, depth, out)
                }
            }
            Gram::Field(_, g) | Gram::Retag(_, g) | Gram::Merge(_, g) | Gram::As(_, g) => self.derive(rng, g, depth, out),
            Gram::Wrap { body, .. } | Gram::Relabel { body, .. } => self.derive(rng, body, depth, out),
            // The opener, maybe a few tokens and a nested pair, the closer.
            Gram::Balanced { open, close, .. } => {
                out.push(self.keyword(rng, open));
                if !deep {
                    for _ in 0..rng.below(3) {
                        out.push(self.token(rng, "identifier"));
                    }
                    if rng.below(3) == 0 {
                        self.derive(rng, rep, depth.saturating_sub(1), out);
                    }
                }
                out.push(self.keyword(rng, close));
            }
        }
    }

    /// A literal's bytes: its keyword's, or the literal itself (`'['`, `'"'`).
    fn keyword(&self, rng: &mut Rng, k: &str) -> Vec<u8> {
        match self.keywords.get(k) {
            Some(ws) => rng.pick(ws).clone(),
            None => k.as_bytes().to_vec(),
        }
    }

    /// A token of kind `k`: one its check accepts, one of its keywords, or text.
    fn token(&self, rng: &mut Rng, k: &str) -> Vec<u8> {
        let long = rng.below(12) == 0;
        if self.marks.iter().any(|m| m == k) {
            return b"\n".to_vec();
        }
        if let Some(s) = self.samples.get(k) {
            return rng.pick(s).clone();
        }
        if let Some(c) = self.checks.get(k) {
            let mut out = Vec::new();
            self.bytes(rng, c, long, &mut out);
            return out;
        }
        if let Some(ws) = self.keywords.get(k) {
            return rng.pick(ws).clone();
        }
        // An extra (a comment): line and block comments, with bytes that look like other tokens.
        if self.extras.iter().any(|e| e == k) {
            let samples = comments(k);
            let (clean, odd) = samples.split_at(samples.len() - 4);
            let text = if rng.below(8) == 0 { rng.pick(odd).0 } else { rng.pick(clean).0 };
            return text.as_bytes().to_vec();
        }
        let n = if long { 60 + rng.below(100) } else { 1 + rng.below(6) };
        (0..n).flat_map(|_| rng.pick(&["a", "Z", "0", " ", "_", ".", "-", "é", "<", ";", "(", "{"]).as_bytes().to_vec()).collect()
    }

    /// Bytes a check's grammar matches: each class name a byte of the class.
    fn bytes(&self, rng: &mut Rng, g: &Gram, long: bool, out: &mut Vec<u8>) {
        match g {
            Gram::Lit(s) => out.extend(s.as_bytes()),
            Gram::Sym(c) => {
                let ranges = &self.classes[c];
                let (a, b) = *rng.pick(ranges);
                out.push((a + rng.below(u64::from(b - a + 1)) as u32) as u8);
            }
            Gram::Seq(gs) => gs.iter().for_each(|g| self.bytes(rng, g, long, out)),
            Gram::Alt(gs) => {
                let g = rng.pick(gs);
                self.bytes(rng, g, long, out)
            }
            Gram::Opt(g) => {
                if rng.below(2) == 0 {
                    self.bytes(rng, g, long, out)
                }
            }
            Gram::Star(r) | Gram::Plus(r) => {
                let plus = u64::from(matches!(g, Gram::Plus(_)));
                let n = plus + if long { 60 + rng.below(100) } else { rng.below(4) };
                let g = r;
                for _ in 0..n {
                    self.bytes(rng, g, long, out)
                }
            }
            Gram::Field(_, g) | Gram::Retag(_, g) | Gram::Merge(_, g) | Gram::As(_, g) => self.bytes(rng, g, long, out),
            Gram::Wrap { body, .. } | Gram::Relabel { body, .. } => self.bytes(rng, body, long, out),
            Gram::Balanced { .. } => {}
        }
    }
}

/// Comment samples by the extra's kind, and whether each runs to the end of its line. The last
/// four of each hold what the lexers send to tree-sitter: the text quote, or one kind's opener
/// in the other.
const COMMENTS: [(&str, [(&str, bool); 13]); 2] = [
    (
        "commentaire",
        [
            ("-- c", true),
            ("--", true),
            ("-- a; (1) x", true),
            ("{- c -}", false),
            ("{--}", false),
            ("{---}", false),
            ("{- a - b } c -}", false),
            ("{- ( -}", false),
            ("{-\n\n-}", false),
            ("-- #x", true),
            ("{- # -}", false),
            ("{- -- -}", false),
            ("-- {- x", true),
        ],
    ),
    (
        "comment",
        [
            ("// c", true),
            ("//", true),
            ("// a, [1] é\r", true),
            ("/* c */", false),
            ("/**/", false),
            ("/***/", false),
            ("/* a * b / c */", false),
            ("/* [ */", false),
            ("/*\n\n*/", false),
            ("// \"x", true),
            ("/* \" */", false),
            ("/* // */", false),
            ("// /* x", true),
        ],
    ),
];

fn comments(kind: &str) -> &'static [(&'static str, bool); 13] {
    &COMMENTS.iter().find(|c| c.0 == kind).unwrap_or_else(|| panic!("no comment samples for `{kind}`")).1
}

/// A step's documents: sentences of its grammar with random white space between tokens (and a
/// space where two would run together), then each with one byte changed, dropped or doubled, and
/// the empty file and one space. With how many come before the mutations.
fn documents(lang: &Lang, seed: u64, accepts: &dyn Fn(&[u8]) -> bool) -> (Vec<Vec<u8>>, usize) {
    documents_with(lang, seed, accepts, b"<>();=#^ \nav9~,eE-{}")
}

/// `documents`, with the bytes a mutation writes drawn from `alphabet`.
pub(super) fn documents_with(lang: &Lang, seed: u64, accepts: &dyn Fn(&[u8]) -> bool, alphabet: &[u8]) -> (Vec<Vec<u8>>, usize) {
    let mut rng = Rng(0x1add_e700 ^ seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
    let mut out: Vec<Vec<u8>> = vec![Vec::new(), b" ".to_vec()];
    let root = Gram::Sym(lang.root.clone());
    let (mut accepted, mut rejected) = (0, 0);
    for _ in 0..600 {
        if accepted >= 80 {
            break;
        }
        let mut tokens = Vec::new();
        let depth = 1 + rng.below(5) as u32;
        lang.derive(&mut rng, &root, depth, &mut tokens);
        let mut doc: Vec<u8> = Vec::new();
        let mut tokens: Vec<Vec<u8>> = tokens;
        // The extras, between tokens and at either end.
        if !lang.extras.is_empty() {
            let mut with = Vec::new();
            for t in tokens.drain(..).chain([Vec::new()]) {
                while rng.below(3) == 0 {
                    let e = lang.token(&mut rng, &lang.extras[0]);
                    with.push(e);
                }
                if !t.is_empty() {
                    with.push(t);
                }
            }
            tokens = with;
        }
        let mut line = false;
        for t in tokens {
            // A line comment ends at the line's end.
            // With marks, a line break is the grammar's: only a mark's sentence writes one.
            let spaces: &[&str] = if lang.marks.is_empty() { &["", "", " ", "\n", " \t "] } else { &["", "", " ", " \t "] };
            let space = if line { "\n" } else { *rng.pick(spaces) };
            // A line comment: a sample of the extras' table that runs to its line's end, or a
            // ladder's sample (`check_ladder`) that begins as one.
            line = lang.extras.iter().any(|k| {
                let opens = [&b"//"[..], b"--", b"#"].iter().any(|o| t.starts_with(o)) && !t.contains(&b'\n');
                (lang.samples.contains_key(k) && lang.samples[k].contains(&t) && opens) || COMMENTS.iter().any(|(n, s)| n == k && s.iter().any(|(c, l)| *l && c.as_bytes() == &t[..]))
            });
            let joins = doc.last().is_some_and(u8::is_ascii_alphanumeric) && t.first().is_some_and(u8::is_ascii_alphanumeric);
            doc.extend(if joins && space.is_empty() { " " } else { space }.as_bytes());
            doc.extend(t);
        }
        doc.extend(if line { "\n" } else { *rng.pick(&["", "\n", " "]) }.as_bytes());
        let _ = &lang.marks;
        // Sentences the reference accepts, and some it does not (a sample token its lexer
        // refuses, say): the mutations below make many more of those.
        if accepts(&doc) {
            accepted += 1;
        } else if rejected < 20 {
            rejected += 1;
        } else {
            continue;
        }
        out.push(doc);
    }
    let valid = out.len();
    for i in 2..valid {
        let mut d = out[i].clone();
        if d.is_empty() {
            continue;
        }
        let at = rng.below(d.len() as u64) as usize;
        match rng.below(3) {
            0 => d[at] = *rng.pick(alphabet),
            1 => {
                d.remove(at);
            }
            _ => d.insert(at, d[at]),
        }
        out.push(d);
    }
    (out, valid)
}

/// Nodes as ids: kind, field, supertypes (sorted), start, end, size.
pub(super) type IdNode = (u16, u16, Vec<u16>, usize, usize, usize);

pub(super) fn as_ids(nodes: Vec<stream::Node>) -> Vec<IdNode> {
    nodes
        .into_iter()
        .map(|n| {
            let mut supers = n.supers;
            supers.sort_unstable();
            (n.kind, n.field, supers, n.start as usize, n.end as usize, n.size as usize)
        })
        .collect()
}

/// A step compiled: its program, tier 1 and tier 2 on Cranelift, and its kinds' names by id.
pub(super) struct Compiled {
    pub(super) stmts: Vec<Stmt>,
    pub(super) ids: Ids,
    pub(super) program: Program,
    pub(super) native: native_ir::NativeModule,
    pub(super) parser: native_cpu_stream::Parser,
}

pub(super) fn compile_step(name: &str, source: &str) -> Compiled {
    let stmts = parse(source).unwrap_or_else(|e| panic!("{name}: parses: {e}"));
    let ids = Ids::default();
    let compiled = {
        let kind = |n: &str| Some(ids.of(n));
        let field = |n: &str| Some(ids.of(&format!("field {n}")));
        let symbols = bitlex_compile::Symbols { kind: &kind, field: &field };
        bitlex_compile::compile(&stmts, &[bitlex_compile::FALLBACK], Some(&symbols)).unwrap_or_else(|e| panic!("{name}: compiles: {e}"))
    };
    let text = compiled.text();
    // `RERAIL_LADDER_DUMP`: the program as text, for a step whose name holds it.
    if std::env::var("RERAIL_LADDER_DUMP").is_ok_and(|d| name.contains(&d)) {
        eprintln!("{name}:\n{}\n{text}", compiled.machine);
    }
    let program = rir::assemble(&text).unwrap_or_else(|e| panic!("{name}: assembles: {e}\n{text}"));
    let native = native_ir::lower(&program).unwrap_or_else(|e| panic!("{name}: lowers: {e:?}"));
    let parser = native_cpu_stream::Parser::new(&native).unwrap_or_else(|e| panic!("{name}: Cranelift refuses: {e:?}"));
    Compiled { stmts, ids, program, native, parser }
}

/// Why the reference sends `bytes` to tree-sitter: its first error, or why the tree does not
/// parse.
pub(super) fn refusal(c: &Compiled, bytes: &[u8]) -> String {
    match ::bitlex::eval::Ev::run(&c.stmts, bytes, &HashMap::new()) {
        Err(e) => e,
        Ok(out) if !out.errors.is_empty() => out.errors[0].split(" at ").next().unwrap_or("").to_owned(),
        Ok(out) => match out.tree.as_ref().map(|t| ::bitlex::tree::nodes(t, &out.tokens, bytes.len())) {
            Some(Err(e)) => e.split(" at byte").next().unwrap_or("").to_owned(),
            _ => "no tree".to_owned(),
        },
    }
}

/// The reference's nodes for `bytes` as ids, or `None` where the file goes to tree-sitter.
pub(super) fn reference(c: &Compiled, bytes: &[u8]) -> Option<Vec<IdNode>> {
    let out = ::bitlex::eval::Ev::run(&c.stmts, bytes, &HashMap::new()).ok()?;
    if !out.errors.is_empty() {
        return None;
    }
    let nodes = ::bitlex::tree::nodes(out.tree.as_ref()?, &out.tokens, bytes.len()).ok()?;
    Some(
        nodes
            .iter()
            .map(|n| {
                let field = n.field.as_deref().map_or(0, |f| c.ids.of(&format!("field {f}")));
                let mut supers: Vec<u16> = n.supertypes.iter().map(|s| c.ids.of(s)).collect();
                supers.sort_unstable();
                (c.ids.of(&n.kind), field, supers, n.start, n.end, n.size)
            })
            .collect(),
    )
}

/// The step's shape: tier 1's body branches only for its loop, its extracts' flattening and, when
/// it has values known on ASCII bytes, one all-ASCII test, and one branch per gated region;
/// tier 2 loops only over tokens, over a mask's words, 64 bytes a turn (a `check` is tier 1's
/// masks), over the calls it pops, and (a grammar with wraps) over the node table once, at the
/// end; nothing calls.
pub(super) fn check_shape(name: &str, c: &Compiled) {
    let block = &c.native.block_programs[0];
    let body = super::block_tests::body_of(&c.parser.tier1.clif);
    let branches = body.iter().filter(|l| super::block_tests::opcode(l) == "brif").count();
    let ascii = usize::from(c.parser.tier1.ascii_branch);
    assert_eq!(branches, 1 + 2 * block.extracts.len() + ascii + c.parser.tier1.gate_branches + c.parser.tier1.round_branches, "{name}: tier 1 branches beyond its loop, its flattening, its ASCII test and its gates");
    let heads = &c.parser.tier2.heads;
    assert!(heads.iter().any(|h| h.1 == Step::Token), "{name}: tier 2 has no token loop");
    for (b, step) in heads {
        let ok = match step {
            Step::Token | Step::Block | Step::Return => true,
            Step::Node => c.parser.tier2.wraps,
            Step::File | Step::Bits | Step::Round => false,
        };
        assert!(ok, "{name}: tier 2 loops at {b} over {step:?}");
    }
    for (tier, asm) in [("tier 1", &c.parser.tier1.disassembly), ("tier 2", &c.parser.tier2.disassembly)] {
        assert!(!asm.contains(" bl ") && !asm.contains("blr") && !asm.contains(" br "), "{name}: {tier} calls or jumps out");
    }
}

/// Every step: the reference's nodes equal the interpreters' and the compiled path's on every
/// document; tier 1 compiled equals its interpreter; each step parses some documents and sends
/// some to tree-sitter. Printed per step: the grammar's sentences and the mutated copies apart,
/// and why the reference refused each sentence it refused.
#[test]
fn every_step_parses_as_the_reference() {
    let differ = check_ladder(&steps(), b"<>();=#^ \nav9~,eE-{}", &[], &[], &|_, _, _, _| None);
    assert!(differ.is_empty(), "{} steps differ:\n{}", differ.len(), differ.join("\n"));
}

/// A ladder's checks (`every_step_parses_as_the_reference`) over `steps`, mutations drawn from
/// `alphabet`, and `fixed` documents besides; and `also`, for each document, given the
/// reference's nodes: a difference with another parser, if any. The differences, one per step at
/// most.
pub(super) fn check_ladder(
    steps: &[(String, String)],
    alphabet: &[u8],
    samples: &[(&str, &[&str])],
    fixed: &[&str],
    also: &dyn Fn(&str, &Compiled, &[u8], &Option<Vec<IdNode>>) -> Option<String>,
) -> Vec<String> {
    let mut differ: Vec<String> = Vec::new();
    for (name, source) in steps {
        let c = compile_step(name, source);
        check_shape(name, &c);
        let mut lang = Lang::of(&c.stmts);
        for (kind, texts) in samples {
            lang.samples.insert((*kind).to_owned(), texts.iter().map(|t| t.as_bytes().to_vec()).collect());
        }
        let (mut parsed, mut fell, mut with_extras) = (0usize, 0usize, 0usize);
        let (mut sentences, mut mutations) = ([0usize; 2], [0usize; 2]);
        let mut reasons: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
        let extra_ids: Vec<u16> = lang.extras.iter().map(|e| c.ids.of(e)).collect();
        let (mut docs, before) = documents_with(&lang, seed(name), &|b| reference(&c, b).is_some(), alphabet);
        docs.extend(fixed.iter().map(|d| d.as_bytes().to_vec()));
        for (i, bytes) in docs.into_iter().enumerate() {
            let show = || format!("{name}: {:?}", String::from_utf8_lossy(&bytes));
            let want = reference(&c, &bytes);
            let run = c.program.block_programs()[0].run_files(&[&bytes]).remove(0);
            let interp = stream::run(&c.program, &bytes, &run).ok().map(as_ids);
            let compiled = c.parser.parse(&bytes).ok().map(as_ids);
            let tier1 = c.parser.tier1.extract(&bytes);
            if tier1 != run {
                differ.push(format!("{}: tier 1 compiled differs from its interpreter", show()));
                break;
            }
            if interp != want || compiled != want {
                let why = stream::run(&c.program, &bytes, &run).err().map(|r| format!("word {}: {}", r.pc, r.why));
                let fell = run.totals.iter().any(|&t| t != 0);
                differ.push(format!("{}:\n  reference {want:?}\n  interpreters {interp:?} ({why:?}, tier 1 falls back: {fell})\n  compiled {compiled:?}", show()));
                break;
            }
            if let Some(d) = also(name, &c, &bytes, &want) {
                differ.push(format!("{}:\n  {d}", show()));
                break;
            }
            let bucket = if i < before { &mut sentences } else { &mut mutations };
            bucket[usize::from(want.is_none())] += 1;
            match &want {
                Some(nodes) => {
                    parsed += 1;
                    with_extras += usize::from(nodes.iter().any(|n| extra_ids.contains(&n.0)));
                }
                None => {
                    fell += 1;
                    if i < before {
                        *reasons.entry(refusal(&c, &bytes)).or_default() += 1;
                    }
                }
            }
        }
        eprintln!(
            "{name}: {} words. sentences: {} parsed, {} refused {reasons:?}. mutated: {} parsed, {} refused. {with_extras} parsed with extras",
            c.native.structural.len(),
            sentences[0],
            sentences[1],
            mutations[0],
            mutations[1]
        );
        if parsed < 20 || fell < 10 || (!extra_ids.is_empty() && with_extras < 20) {
            differ.push(format!("{name}: {parsed} parsed, {fell} to tree-sitter: too few to test"));
        }
    }
    differ
}

/// Every step's tier 1 on Metal, generated MSL (one lane width, the class planes), equals the
/// CPU interpreter on every document: outputs and extracted positions.
#[test]
#[cfg(target_os = "macos")]
fn every_step_on_metal_equals_the_interpreter() {
    use metal_program_backend::block_program::{ClassStrategy, Executor, LaneWidth, run_split};
    let steps = steps();
    let compiled: Vec<Compiled> = steps.iter().map(|(n, s)| compile_step(n, s)).collect();
    let docs: Vec<Vec<Vec<u8>>> = compiled.iter().zip(&steps).map(|(c, (n, _))| documents(&Lang::of(&c.stmts), seed(n), &|b| reference(c, b).is_some()).0).collect();
    let forms: Vec<&native_ir::BlockNative> = compiled.iter().map(|c| &c.native.block_programs[0]).collect();
    let files: Vec<block::BatchFile<'_>> = docs.iter().enumerate().flat_map(|(p, ds)| ds.iter().map(move |b| block::BatchFile { bytes: b, program: p })).collect();
    let run = run_split(&forms, &files, LaneWidth::W32, Executor::Generated(ClassStrategy::Planes), 64).unwrap_or_else(|e| panic!("metal: {e:?}"));
    let mut differ = Vec::new();
    for (f, got) in files.iter().zip(&run.files) {
        let want = compiled[f.program].program.block_programs()[0].run_files(&[f.bytes]).remove(0);
        if got.outputs != want.outputs || got.positions != want.positions {
            differ.push(format!("{}: {:?}", steps[f.program].0, String::from_utf8_lossy(f.bytes)));
        }
    }
    eprintln!("{} files over {} steps on Metal", files.len(), steps.len());
    assert!(differ.is_empty(), "{} files differ:\n{}", differ.len(), differ.join("\n"));
}
