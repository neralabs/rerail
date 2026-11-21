//! The Bitlex ladder's checks (the engine's tests and this crate's use them): thirteen languages, from one keyword to a data language with nested
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
//! Tier 1 on Metal (generated MSL) equals its interpreter bit for bit, and on Cranelift by the
//! contract between the tiers (decision 0177, `tier1_agrees`): the records, the totals and every
//! mask the CPU makes. And each step has the
//! shape the compiler promises: tier 1's body has no branch but the loop's and the extracts'
//! flattening (it is branchless per block; a CPU form that makes no mask is the record pass's
//! token loop alone), and tier 2 has one loop, over the tokens (and one over 64-byte blocks
//! inside each `check`); neither calls anything.


use crate::native_cpu_shape::Step;
use crate::native_ir::Structure;
use crate::program::Words;
use ::bitlex::syntax::{Gram, Stmt, parse};
use std::collections::HashMap;

/// A seeded xorshift: the corpus is the same on every run.
pub struct Rng(pub u64);

impl Rng {
    pub fn below(&mut self, n: u64) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 % n
    }

    pub fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[self.below(xs.len() as u64) as usize]
    }
}

/// Ids by name, for a corpus grammar: each kind is numbered at its first use, from 1, and each
/// field (`field NAME`) apart, from 1, as a tree-sitter grammar numbers them. A field's id is
/// a code's bits above its kind on the tape tier 2 writes (decision 0140), under
/// `rerail_tape_format::FIELD_LIMIT`.
#[derive(Default)]
pub struct Ids(pub std::cell::RefCell<HashMap<String, u16>>);

impl Ids {
    pub fn of(&self, name: &str) -> u16 {
        let mut m = self.0.borrow_mut();
        if let Some(&id) = m.get(name) {
            return id;
        }
        let field = name.starts_with("field ");
        let id = m.keys().filter(|k| k.starts_with("field ") == field).count() as u16 + 1;
        m.insert(name.to_owned(), id);
        id
    }

    /// The ids back to their names: the kinds', and the fields' (without `field `).
    pub fn names(&self) -> (HashMap<u16, String>, HashMap<u16, String>) {
        let (mut kinds, mut fields) = (HashMap::new(), HashMap::new());
        for (n, &id) in self.0.borrow().iter() {
            match n.strip_prefix("field ") {
                Some(f) => fields.insert(id, f.to_owned()),
                None => kinds.insert(id, n.clone()),
            };
        }
        (kinds, fields)
    }
}

/// The ladder's directory (`bitlex_ladder/` beside this crate's `src/`), found as the program runs: above the program,
/// `CARGO_MANIFEST_DIR` or the working directory. Not written into the build (decision 0224): with its checkout's path
/// in, this crate and every crate above it were a different crate in every checkout, for every profile.
pub fn dir() -> String {
    let ladder = std::path::Path::new(file!()).parent().and_then(std::path::Path::parent).expect("src/ladder.rs").join("bitlex_ladder");
    let starts = [std::env::current_exe().ok(), std::env::var_os("CARGO_MANIFEST_DIR").map(Into::into), std::env::current_dir().ok()];
    starts
        .into_iter()
        .flatten()
        .find_map(|start| start.ancestors().map(|a| a.join(&ladder)).find(|d| d.is_dir()))
        .unwrap_or_else(|| panic!("no directory above the program, CARGO_MANIFEST_DIR or the working directory holds {}", ladder.display()))
        .to_string_lossy()
        .into_owned()
}

/// The ladder's files, in order.
pub fn steps() -> Vec<(String, String)> {
    steps_in(&dir())
}

/// The steps of a ladder in `dir`, in order: its `.bitlex` files (not those of its
/// subdirectories, which are other ladders).
pub fn steps_in(dir: &str) -> Vec<(String, String)> {
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
pub fn seed(name: &str) -> u64 {
    name.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3))
}

/// What a step's documents are drawn from: its tree rules, and its tokens' keywords and checks.
pub struct Lang {
    pub rules: HashMap<String, Gram>,
    pub root: String,
    /// A keyword's kind to its bytes (a keyword without `->` is its own kind).
    pub keywords: HashMap<String, Vec<Vec<u8>>>,
    pub checks: HashMap<String, Gram>,
    pub classes: HashMap<String, Vec<(u32, u32)>>,
    /// The fewest tokens each rule can match, for a document's last level.
    pub least: HashMap<String, usize>,
    /// Kinds that may come between any two tokens (`extra`).
    pub extras: Vec<String>,
    /// Sample texts of token kinds, from the ladder (`check_ladder`): drawn before any other.
    pub samples: HashMap<String, Vec<Vec<u8>>>,
    /// The marks' kinds (`mark K = E`): line breaks, here; a sentence writes one as a `\n`.
    pub marks: Vec<String>,
}

impl Lang {
    pub fn of(stmts: &[Stmt]) -> Lang {
        let (mut rules, mut order, mut keywords, mut checks, mut classes) = (HashMap::new(), Vec::new(), HashMap::new(), HashMap::new(), HashMap::new());
        let mut extras = Vec::new();
        let mut marks = Vec::new();
        for s in stmts {
            match s {
                Stmt::Mark { kind, .. } | Stmt::TokenMark { kind, .. } => marks.push(kind.clone()),
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
            Gram::Plus(g) | Gram::Field(_, g) | Gram::Retag(_, g) | Gram::Merge(_, g) | Gram::As(_, g) | Gram::Flag { body: g, .. } | Gram::Unless { body: g, .. } => self.cost(g),
            Gram::Wrap { body, .. } | Gram::Relabel { body, .. } | Gram::Fold { body, .. } | Gram::Reread { body, .. } => self.cost(body),
            Gram::Balanced { .. } => 2,
            Gram::Any(_) => 1,
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
            Gram::Field(_, g) | Gram::Retag(_, g) | Gram::Merge(_, g) | Gram::As(_, g) | Gram::Flag { body: g, .. } | Gram::Unless { body: g, .. } => self.derive(rng, g, depth, out),
            Gram::Wrap { body, .. } | Gram::Relabel { body, .. } | Gram::Fold { body, .. } | Gram::Reread { body, .. } => self.derive(rng, body, depth, out),
            // A word: a token of most kinds.
            Gram::Any(_) => out.push(self.token(rng, "identifier")),
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
            Gram::Field(_, g) | Gram::Retag(_, g) | Gram::Merge(_, g) | Gram::As(_, g) | Gram::Flag { body: g, .. } | Gram::Unless { body: g, .. } => self.bytes(rng, g, long, out),
            Gram::Wrap { body, .. } | Gram::Relabel { body, .. } | Gram::Fold { body, .. } | Gram::Reread { body, .. } => self.bytes(rng, body, long, out),
            Gram::Balanced { .. } | Gram::Any(_) => {}
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

pub fn comments(kind: &str) -> &'static [(&'static str, bool); 13] {
    &COMMENTS.iter().find(|c| c.0 == kind).unwrap_or_else(|| panic!("no comment samples for `{kind}`")).1
}

/// A step's documents: sentences of its grammar with random white space between tokens (and a
/// space where two would run together), then each with one byte changed, dropped or doubled, and
/// the empty file and one space. With how many come before the mutations.
pub fn documents(lang: &Lang, seed: u64, accepts: &dyn Fn(&[u8]) -> bool) -> (Vec<Vec<u8>>, usize) {
    documents_with(lang, seed, accepts, b"<>();=#^ \nav9~,eE-{}")
}

/// `documents`, with the bytes a mutation writes drawn from `alphabet`.
pub fn documents_with(lang: &Lang, seed: u64, accepts: &dyn Fn(&[u8]) -> bool, alphabet: &[u8]) -> (Vec<Vec<u8>>, usize) {
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
pub type IdNode = (u16, u16, Vec<u16>, usize, usize, usize);

pub fn as_ids(nodes: Vec<crate::stream::Node>) -> Vec<IdNode> {
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
pub struct Compiled {
    pub stmts: Vec<Stmt>,
    pub ids: Ids,
    pub program: Words,
    pub native: Structure,
    pub parser: crate::native_cpu_stream::Parser,
}

pub fn compile_step(name: &str, source: &str) -> Compiled {
    let stmts = parse(source).unwrap_or_else(|e| panic!("{name}: parses: {e}"));
    let ids = Ids::default();
    let compiled = {
        let kind = |n: &str| Some(ids.of(n));
        // A field's id goes on the tape in a code's bits above its kind (`native_cpu_tape::kf`).
        let field = |n: &str| {
            let id = ids.of(&format!("field {n}"));
            assert!(usize::from(id) < rerail_tape_format::FIELD_LIMIT, "{name}: field {n} past the tape's {}", rerail_tape_format::FIELD_LIMIT);
            Some(id)
        };
        let symbols = crate::bitlex_masks::Symbols { kind: &kind, field: &field };
        crate::bitlex_masks::compile(&stmts, &[crate::bitlex_masks::FALLBACK], Some(&symbols)).unwrap_or_else(|e| panic!("{name}: compiles: {e}"))
    };
    let text = compiled.text();
    // `RERAIL_LADDER_DUMP`: the program as text, for a step whose name holds it.
    if std::env::var("RERAIL_LADDER_DUMP").is_ok_and(|d| name.contains(&d)) {
        eprintln!("{name}:\n{}\n{text}", compiled.machine);
    }
    let program = crate::rir_asm::assemble(&text).unwrap_or_else(|e| panic!("{name}: assembles: {e}\n{text}"));
    let native = crate::native_ir::lower(&program, 0, 0).unwrap_or_else(|e| panic!("{name}: lowers: {e:?}"));
    let parser = crate::native_cpu_stream::Parser::new(&native).unwrap_or_else(|e| panic!("{name}: Cranelift refuses: {e:?}"));
    Compiled { stmts, ids, program, native, parser }
}

/// Why the compiler refuses `source`; `None` where it compiles.
pub fn compile_error(source: &str) -> Option<String> {
    let stmts = parse(source).unwrap_or_else(|e| panic!("parses: {e}"));
    let ids = Ids::default();
    let kind = |n: &str| Some(ids.of(n));
    let field = |n: &str| Some(ids.of(&format!("field {n}")));
    let symbols = crate::bitlex_masks::Symbols { kind: &kind, field: &field };
    crate::bitlex_masks::compile(&stmts, &[crate::bitlex_masks::FALLBACK], Some(&symbols)).err()
}

/// Why the reference sends `bytes` to tree-sitter: its first error, or why the tree does not
/// parse.
pub fn refusal(c: &Compiled, bytes: &[u8]) -> String {
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
pub fn reference(c: &Compiled, bytes: &[u8]) -> Option<Vec<IdNode>> {
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

/// What tier 1 on the CPU owes tier 2, as decision 0177 states the contract between the tiers:
/// the records (every position, every code), every total, and every output the CPU's form
/// makes. The outputs its form makes a zero constant (the masks the record pass and its token
/// loop make from the bytes where they read them: `BlockNative::cpu_form`) are not made, and
/// the gap marks tier 2 on the CPU tests itself (`native_cpu_block::lazy_marks`) are not in its
/// codes, where the interpreter's codes have them; except a lazy mark with token tests
/// (`native_cpu_block::lazy_coded`), whose tests' hits the token loop writes in the code: there a
/// code holds the hit where the interpreter's holds the mark.
pub struct Contract {
    unmade: Vec<bool>,
    lazy: u32,
    /// The lazy marks whose tests' hits are in the code (`lazy_coded`), as code bits.
    coded: u32,
}

impl Contract {
    pub fn of(block: &crate::block_native::BlockNative) -> Self {
        let cpu = block.cpu_form();
        let unmade = cpu.outputs.iter().map(|&o| matches!(cpu.insts[usize::from(o)], crate::block_native::BlockInst::Const(0))).collect();
        let (lazy, coded) = block.records.as_ref().map_or((0, 0), |r| {
            let bytes = crate::native_cpu_bytes::Bytewise::of(block);
            let coded = crate::native_cpu_block::lazy_coded(r);
            crate::native_cpu_block::lazy_marks(r, &|o| bytes.can(o)).iter().fold((0, 0), |(a, c), &(k, _)| {
                let bit = 1 << (crate::records::MARK_SHIFT + u32::from(k));
                (a | bit, if coded >> k & 1 == 1 { c | bit } else { c })
            })
        });
        Self { unmade, lazy, coded }
    }
}

/// Whether tier 1 on the CPU (`cpu`) keeps the contract with its interpreter's run (`interp`).
/// A file whose totals send it to the fallback owes tier 2 nothing but them: tier 2's first
/// words are its `EMPTY`s (`bitlex_tier2`), which refuse it before any record is read, and the
/// token loop stops where it refuses, its codes from there not made.
pub fn tier1_agrees(contract: &Contract, cpu: &crate::block::BlockRun, interp: &crate::block::BlockRun) -> bool {
    let falls = interp.totals.iter().any(|&t| t != 0);
    // Sent to the fallback: the totals alone are owed (the token loop left at its refusal, its records from there
    // not made, so fewer positions and codes than the interpreter's, which goes on).
    if falls {
        return cpu.totals == interp.totals;
    }
    cpu.positions == interp.positions
        && cpu.totals == interp.totals
        && cpu.codes.len() == interp.codes.len()
        // A code's lazy bits: none but a coded mark's hit, where the interpreter's code has the mark.
        && (cpu.codes.iter().zip(&interp.codes).all(|(&c, &i)| c & !contract.lazy == i & !contract.lazy && c & contract.lazy & !(contract.coded & i) == 0))
        && cpu.outputs.len() == interp.outputs.len()
        && cpu.outputs.iter().zip(&interp.outputs).zip(&contract.unmade).all(|((c, i), &unmade)| unmade || c == i)
}

/// The step's shape: tier 1's body branches only for its loop, its extracts' flattening and, when
/// it has values known on ASCII bytes, one all-ASCII test, and one branch per gated region (a
/// program whose CPU form makes no mask is its record pass's token loop alone, decision 0177);
/// tier 2 loops only over tokens, over a mask's words, 64 bytes a turn (a `check` is tier 1's
/// masks), over the calls it pops, and (a grammar with wraps) over the node table once, at the
/// end; nothing calls.
pub fn check_shape(name: &str, c: &Compiled) {
    // Each pass of tier 1 (one, without `back_fill`): a forward pass's body as above; a
    // backward pass's function branches only for its files, its blocks and their ends.
    let block = &c.native.block_programs[0];
    let cpu = block.cpu_form();
    let makes_masks = cpu.insts.iter().any(|i| !matches!(i, crate::block_native::BlockInst::Const(_)));
    assert!(makes_masks || block.records.is_some(), "{name}: tier 1 on the CPU makes no mask and has no record pass");
    for (k, pass) in c.parser.tier1.passes.iter().enumerate() {
        let branches = |lines: &[String]| lines.iter().filter(|l| crate::clif_text::opcode(l) == "brif").count();
        if pass.back {
            let lines: Vec<String> = pass.clif.lines().map(|l| l.trim().to_owned()).collect();
            assert_eq!(branches(&lines), 4, "{name}: tier 1's backward pass {k} branches beyond its loops");
            continue;
        }
        // A program whose CPU form makes no mask (decision 0177: its token loop reads its
        // tokens' bytes where it reads them) has no full blocks' body: it is the record pass's
        // token loop alone, whose loops the verifier states (`native_cpu_shape::check`: over the
        // blocks, their tokens and their readers' words).
        if !makes_masks {
            continue;
        }
        let body = crate::clif_text::body_of(&pass.clif);
        let want = 1 + 2 * pass.extracts + usize::from(pass.ascii_branch) + pass.gate_branches + pass.round_branches;
        assert_eq!(branches(&body), want, "{name}: tier 1's pass {k} branches beyond its loop, its flattening, its ASCII test and its gates");
    }
    let heads = &c.parser.tier2.heads;
    assert!(heads.iter().any(|h| h.1 == Step::Token), "{name}: tier 2 has no token loop");
    for (b, step) in heads {
        let ok = match step {
            Step::Token | Step::Block | Step::Return => true,
            // The table's final pass (parents, extras), and the wraps' placing.
            Step::Node => true,
            // No loop over bits: a local check (`CHECK`) is word-parallel, a loop over words.
            Step::Bits | Step::File | Step::Round | Step::Region | Step::Word | Step::Bytes => false,
        };
        assert!(ok, "{name}: tier 2 loops at {b} over {step:?}");
    }
    // Each tier calls only its own outlined functions (`native_cpu_block::general`,
    // `native_cpu_stream::outline`).
    // An indirect jump only through a jump table of its own: tier 2's shared arms
    // (`native_cpu_stream::shared_runs`) end in one, a `br` each; tier 1's token loop takes a
    // token's rule by one, as the lean lexer's `match` on its first byte (decision 0177).
    use crate::clif_text::{Transfer, count_transfers, transfers};
    let tables_in = |clif: &str| clif.lines().filter(|l| l.contains(" br_table ")).count();
    let tables1 = c.parser.tier1.passes.iter().map(|p| tables_in(&p.clif) + p.outlined.iter().map(|o| tables_in(&o.clif)).sum::<usize>()).sum();
    let tables = tables_in(&c.parser.tier2.clif);
    for (tier, asm, own, tables) in [("tier 1", &c.parser.tier1.disassembly, c.parser.tier1.own_calls(), tables1), ("tier 2", &c.parser.tier2.disassembly, c.parser.tier2.own_calls, tables)] {
        // A jump table's sequence is one line, its indirect jump inside, its entries the
        // function's labels.
        let out: Vec<&str> = asm.lines().filter(|l| transfers(l).iter().any(|t| matches!(t, Transfer::NearCall | Transfer::Jump))).take(4).collect();
        assert!(out.is_empty(), "{name}: {tier} jumps out: {out:?}");
        assert_eq!(count_transfers(asm, Transfer::Table), tables, "{name}: {tier} jumps other than by its own tables");
        assert_eq!(count_transfers(asm, Transfer::FarCall), own, "{name}: {tier} calls out: {:?}", asm.lines().filter(|l| transfers(l).iter().any(|t| matches!(t, Transfer::FarCall))).collect::<Vec<_>>());
    }
}

/// Tier 1 compiled agrees with its interpreter on a document: the same run, or, for a file both
/// refuse, the same but for the codes past the token that refused (decision 0177, "A refusal
/// leaves the loop"): the compiled loop leaves there and writes no code after it (each is 0), while
/// the interpreter goes on. The stage's outputs and positions, made before the loop, are the same.
fn refused_alike(compiled: &crate::block::BlockRun, interpreted: &crate::block::BlockRun) -> bool {
    let refused = |r: &crate::block::BlockRun| r.totals.iter().any(|&t| t != 0);
    if compiled == interpreted {
        return true;
    }
    let left = compiled.codes.iter().zip(&interpreted.codes).position(|(a, b)| a != b).unwrap_or(compiled.codes.len());
    refused(compiled)
        && refused(interpreted)
        && compiled.totals == interpreted.totals
        && compiled.outputs == interpreted.outputs
        && compiled.positions == interpreted.positions
        && compiled.codes.len() == interpreted.codes.len()
        && compiled.codes[left..].iter().all(|&c| c == 0)
}

/// A ladder's checks (`every_step_parses_as_the_reference`) over `steps`, mutations drawn from
/// `alphabet`, and `fixed` documents besides; and `also`, for each document, given the
/// reference's nodes: a difference with another parser, if any. The differences, one per step at
/// most.
pub fn check_ladder(
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
        let contract = Contract::of(&c.native.block_programs[0]);
        let (mut docs, before) = documents_with(&lang, seed(name), &|b| reference(&c, b).is_some(), alphabet);
        docs.extend(fixed.iter().map(|d| d.as_bytes().to_vec()));
        for (i, bytes) in docs.into_iter().enumerate() {
            let show = || format!("{name}: {:?}", String::from_utf8_lossy(&bytes));
            let want = reference(&c, &bytes);
            let run = c.program.block_programs[0].run_files(&[&bytes]).remove(0);
            let interp = crate::stream::run(&c.program, &bytes, &run).ok().map(as_ids);
            let compiled = c.parser.parse(&bytes).ok().map(as_ids);
            let tier1 = c.parser.tier1.extract(&bytes);
            if !tier1_agrees(&contract, &tier1, &run) && !refused_alike(&tier1, &run) {
                differ.push(format!("{}: tier 1 compiled differs from its interpreter", show()));
                break;
            }
            if interp != want || compiled != want {
                let why = crate::stream::run(&c.program, &bytes, &run).err().map(|r| format!("word {}: {}", r.pc, r.why));
                let fell = run.totals.iter().any(|&t| t != 0);
                differ.push(format!("{}:\n  reference {want:?}\n  interpreters {interp:?} ({why:?}, tier 1 falls back: {fell})\n  compiled {compiled:?}", show()));
                break;
            }
            // The tape's bytes: the compiled code's, the interpreter's nodes through the tape's
            // writer (`stream::image`), each extra's code flagged.
            if let (Ok(image), Ok(nodes)) = (c.parser.image(&bytes), crate::stream::run(&c.program, &bytes, &run)) {
                let want = crate::stream::image(&nodes, &c.program.readings.extras, c.parser.tier2.widths_tag().1, &bytes);
                if image != want {
                    let at = image.iter().zip(&want).position(|(a, b)| a != b).unwrap_or(image.len().min(want.len()));
                    differ.push(format!("{}: the tape differs from the interpreter's at {}", show(), rerail_tape_format::place(&want, at)));
                    break;
                }
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

