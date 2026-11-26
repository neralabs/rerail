//! Tier 2 as Rust, written by the Bitlex compiler from the grammar's tree rules: a recursive
//! descent parser in the lean parsers' shape (`rerail_kotlin_lean::parse`), never lowered from an
//! intermediate form.
//!
//! - **A function a rule.** `node K = …` opens its node at its first token and closes it at its
//!   end (`self.open(k::K)`, `self.close()`); `choice N = …` makes no node. A sequence is its
//!   elements one after the other; a token is taken (`tk`, a leaf) or passed (`skip`, a token the
//!   grammar implies) where the rule says it.
//! - **Decisions at the token.** `[A]`, `A*`, `A+` and `A | B` decide at the next token, as the
//!   lean parser's `if self.peek() == …`: the marks first, in the order they are declared (a mark
//!   holds before its token), then the words, then the kinds. Where the rule may end there, a
//!   mark that ends a level (`, ends`) ends it, as the lean parser's `ends()` asks `asi`.
//! - **Token marks** are lookahead functions written from their patterns, as the lean parser's
//!   `lambda_ahead` and `type_args_end`; a mark of tier 1's is a bit of the token's code.
//! - **The tree** is written as the lean parser's `Tw` writes it (`rerail_tape_format::Writer`):
//!   the comments before a token written into the node open when the next node or token is, a
//!   node put over the one just made (`wrap`) by `Writer::insert`.

use ::bitlex::syntax::{Gram, Stmt, TokenElem};

// Mounted by `rerail-bitlex` from the compiler's folder (`#[path]`): its submodule is found beside it.
#[path = "bitlex_rust/moves.rs"]
mod moves;
#[path = "bitlex_rust/one_loop.rs"]
mod one_loop;

use moves::{Block, Cond, Op, Pat, Take, line};
pub use one_loop::{tier2_calls_reads, tier2_loop, tier2_loop_reads};
use std::collections::{HashMap, HashSet};
use std::fmt::Write;

/// A mark, as the generated code tests it.
pub struct Mark<'a> {
    pub name: String,
    /// Its bit in a record's code (`16 + bit`): a mark tier 1 makes.
    pub bit: Option<u8>,
    /// The word the token must be too (0: any), with the bit.
    pub word: u16,
    /// The file's end has it.
    pub end: bool,
    /// A token mark's tokens: the token before (`after`), then the run.
    pub tokens: Option<(&'a [TokenElem], &'a [Vec<TokenElem>])>,
    /// Its `before`s and `unless before`s.
    pub next: &'a [(bool, Vec<Vec<TokenElem>>)],
    /// Its `unless after` words.
    pub unless_after: &'a [Vec<u8>],
    pub ends: bool,
    pub settles: bool,
    /// Its questions (`, drops …`, `, loses …`, `, refuses …`, `, refuses taken …`, decision 0451).
    pub drops: &'a [::bitlex::syntax::GapPattern],
    pub loses: &'a [::bitlex::syntax::GapPattern],
    pub refuses: &'a [::bitlex::syntax::GapPattern],
    pub refuses_taken: &'a [::bitlex::syntax::GapPattern],
    pub hidden: bool,
    /// `, passes M`: the marks after it that taking it passes too, a bit each by order.
    pub passes: u64,
}

/// What the generator reads: the tree's rules and the compiler's tables.
pub struct Input<'a> {
    pub tree: &'a [Stmt],
    /// A node kind's id (named first, then anonymous).
    pub kind: &'a dyn Fn(&str) -> Option<u16>,
    pub field: &'a dyn Fn(&str) -> Option<u16>,
    /// The records' kinds: each kind byte and its token's name.
    pub tokens: &'a [(u8, String)],
    /// The words a record's code names: word `i + 1` is `words[i]`.
    pub words: &'a [Vec<u8>],
    /// Each keyword row: its kind, its hard words (with their kinds' names), whether every token is one.
    pub keywords: &'a [(String, Vec<(Vec<u8>, Option<String>)>, bool)],
    pub marks: Vec<Mark<'a>>,
    pub extras: &'a [String],
    /// The marks that tell an extra's record (`Records::extra`): the first, of the first extra
    /// kind, then each later kind's own bit.
    pub extra_bits: Vec<(u8, String)>,
    /// Each row's soft keywords (`Lowering::soft`).
    pub soft: &'a [(String, Vec<Vec<u8>>)],
    /// The lexer's statements: the regions tier 2 reads in parts (`bitlex_lex_rust::readers`).
    pub lexer: &'a [Stmt],
    /// Each token row's own record kind, where its start gives it (`bitlex_lex_rust::Input`).
    pub row_kinds: &'a [Option<u8>],
    /// Each byte's record kind as a token's first byte (`Records::kinds`).
    pub kinds: &'a [u8],
    /// The last bytes of the language's line terminators: `recover`'s `first` asks for only blanks back to one.
    pub terminators: &'a [u8],
}

/// What a decision tests at the token.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Test {
    /// A mark, by its order.
    Mark(usize),
    Word(u8),
    /// A token kind, by its name.
    Kind(String),
}

/// A test, and the flag that must be clear for it (`unless F`).
type First = (Test, Option<String>);

/// Where a test may hold (`G::holds`): at a token that is one of `.0`'s words or of one of `.1`'s kinds; None, anywhere.
type Holds = Option<(Vec<String>, Vec<String>)>;

struct G<'a> {
    x: &'a Input<'a>,
    rules: HashMap<&'a str, &'a Stmt>,
    order: Vec<&'a str>,
    flags: Vec<String>,
    readings: HashMap<&'a str, &'a ::bitlex::syntax::Reading>,
    /// The `remember` choices: a token one takes is its level's word (`P::word`), the kind a wrap's, relabel's or
    /// reading's `as C` names.
    remember: Vec<String>,
    /// `recover NODE …`: the node whose repetition is the statement loop, and its resume patterns (decision 0650).
    recover: Option<(&'a str, &'a [::bitlex::syntax::RecoverClause])>,
    /// `refuse M in L …`: each level's refused marks, their bits in a record's code: a token the level takes with one
    /// refuses the file (`P::refusing`).
    refusing: HashMap<String, u32>,
    /// `ending R K …`: what rule `R` does where it ends, by the kind `K` of the node made last at its level.
    endings: Vec<(&'a str, &'a str, &'a ::bitlex::syntax::Ending)>,
    /// The mark bits the code written so far reads of a record's code (`16 + bit`): tier 1 writes only these where its
    /// loop reads the bytes (decision 0623).
    reads: std::cell::Cell<u32>,
    /// The field ids past a key's 6 bits, each given the slot `63 - its place` (`field_id`).
    own_fields: std::cell::RefCell<Vec<u16>>,
    /// The largest field id under 64 named: the slots must be above it.
    most_field: std::cell::Cell<u16>,
    firsts: std::cell::RefCell<HashMap<String, (Vec<First>, bool)>>,
    busy: std::cell::RefCell<HashSet<String>>,
    /// The kinds the code names, by their constant's name; the fields; the words.
    kinds: std::cell::RefCell<std::collections::BTreeMap<String, u16>>,
    fields: std::cell::RefCell<std::collections::BTreeMap<String, u16>>,
    /// Each rule's follow (`follow`), once found.
    follows: std::cell::RefCell<Option<HashMap<String, Vec<usize>>>>,
    /// Where `open_mark` can hold (`open_mark_words`), for the token takes that need not ask it (`moves::takes`): the words a mark no
    /// rule takes may hold at, and the token kinds whose test (`at_K`) passes at none of them. None: a mark may hold at
    /// any token.
    mark_words: std::cell::OnceCell<Option<(HashSet<String>, HashSet<String>)>>,
    /// Whether a decision reads `marked` (`!self.marked`, `after_mark`): where none does, nothing writes it.
    marked_read: std::cell::Cell<bool>,
    /// The takes of the rules' code (`moves::Names`), once every rule is made (`printed`): the helpers it calls, and
    /// the tests they ask (`take_helpers`).
    names: std::cell::OnceCell<moves::Names>,
    /// Each rule's code where its one caller calls it (`rule`, `inlined`), by its method's name: none for a level's.
    inline: std::cell::RefCell<HashMap<String, Block>>,
    /// The rules' methods' names, in the order `inlined` gives their moves.
    methods: std::cell::RefCell<Vec<String>>,
    /// What a level's call saves of its caller's, where only some levels change it (`records`' calls form): the levels
    /// that may refuse a mark or that a rule refusing one calls, and those whose code may set the level's word. None:
    /// every call saves both.
    call_saves: std::cell::RefCell<Option<(HashSet<String>, HashSet<String>)>>,
    /// The marks the code peeks at as the lean parser does (`peeks`), a bit each by order.
    peek_marks: std::cell::Cell<Option<u64>>,
    /// The choices of tokens taken where they are used (`anon_choice`), by their table's name (`anon_table`): the words
    /// each may be, and each one's leaf's kind.
    anon: std::cell::RefCell<std::collections::BTreeMap<String, std::collections::BTreeMap<u8, String>>>,
    /// The rules a `leaf K R` reaches (`R` and those it calls): they read the tokens and make nothing, the leaf made
    /// over them after (decision 0673 step 6).
    recognizers: HashSet<String>,
    /// `refuse M in L` where every level `L` is under a `leaf`: each leaf's rule and its refused marks, the leaf's tokens
    /// checked once where it closes (`leaf_close`), as the lean parser checks a policy document's text once it has read
    /// it. `refusing` is then empty: no call saves the marks refused, no take tests them (decision 0673 step 7).
    leaf_refuses: HashMap<String, u32>,
    /// A `leaf K R`'s call of `R` being written: the one call of a rule under a leaf from outside one.
    leaf_call: std::cell::Cell<bool>,
    /// The levels that count the depth (`depth_cuts`).
    cuts: std::cell::RefCell<Option<HashSet<String>>>,
    /// The bracket scans (`scan_fn`): each one's bracket and tokens, and its function's name.
    scans: std::cell::RefCell<Vec<(String, String)>>,
}

/// `s`'s lines at `p`.
fn indent(s: &str, p: &str) -> String {
    let mut out = String::new();
    for l in s.lines() {
        if !l.is_empty() {
            out.push_str(p);
        }
        out.push_str(l);
        out.push('\n');
    }
    out
}

/// A name as a Rust identifier: letters, digits and `_`, a punctuation's name spelled out.
fn ident(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        let w = match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '_' => {
                out.push(c);
                continue;
            }
            '.' => "dot",
            ',' => "comma",
            ':' => "colon",
            ';' => "semi",
            '(' => "lparen",
            ')' => "rparen",
            '{' => "lbrace",
            '}' => "rbrace",
            '[' => "lbrack",
            ']' => "rbrack",
            '<' => "lt",
            '>' => "gt",
            '=' => "eq",
            '+' => "plus",
            '-' => "minus",
            '*' => "star",
            '/' => "slash",
            '%' => "percent",
            '!' => "bang",
            '?' => "quest",
            '&' => "amp",
            '|' => "bar",
            '@' => "at",
            '#' => "hash",
            '$' => "dollar",
            '\\' => "backslash",
            '"' => "quote",
            '\'' => "tick",
            '^' => "caret",
            '~' => "tilde",
            '`' => "backtick",
            _ => "x",
        };
        if !out.is_empty() && !out.ends_with('_') {
            out.push('_');
        }
        out.push_str(w);
        out.push('_');
    }
    let out = out.trim_end_matches('_').to_owned();
    if out.is_empty() || out.as_bytes()[0].is_ascii_digit() { format!("x{out}") } else { out }
}

fn upper(name: &str) -> String {
    ident(name).to_ascii_uppercase()
}

/// A rule's function name: the rule's, apart from Rust's keywords and the runtime's own.
fn rule_fn(name: &str) -> String {
    let n = ident(name);
    match n.as_str() {
        "type" | "fn" | "if" | "else" | "match" | "loop" | "while" | "for" | "in" | "as" | "use" | "mod" | "struct" | "enum" | "impl" | "trait" | "where" | "return" | "break" | "continue" | "self" | "super" | "crate" | "let" | "mut" | "ref" | "move" | "static" | "const" | "true" | "false" | "dyn" | "box" | "do" | "try" | "async" | "await" | "abstract" | "final" | "override" | "virtual" | "yield" | "macro" | "priv" | "typeof" | "unsized" | "become" | "open" | "close" | "block" | "flush" | "bump" | "leaf" | "tk" | "skip" | "wrap" | "word" | "kind" | "start" | "statement" | "expression" | "take" | "p" | "p_stored" | "implied" | "implied_if" | "id" | "bump_as" | "tk_as" | "merge_as" | "pass_as" | "word_pass_as" => format!("r_{n}"),
        _ => n,
    }
}

impl<'a> G<'a> {
    fn new(x: &'a Input<'a>) -> Self {
        let mut rules = HashMap::new();
        let mut order = Vec::new();
        let mut flags = Vec::new();
        let mut readings = HashMap::new();
        let (mut remember, mut recover) = (Vec::new(), None);
        let mut refusing: HashMap<String, u32> = HashMap::new();
        let mut endings = Vec::new();
        for s in x.tree {
            match s {
                Stmt::Node { kind, .. } => {
                    rules.insert(kind.as_str(), s);
                    order.push(kind.as_str());
                }
                Stmt::Choice { name, .. } => {
                    rules.insert(name.as_str(), s);
                    order.push(name.as_str());
                }
                Stmt::Flags(f) => flags.extend(f.iter().cloned()),
                Stmt::Reading { name, reading } => {
                    readings.insert(name.as_str(), reading);
                }
                Stmt::Remember(n) => remember.extend(n.iter().cloned()),
                Stmt::Recover { node, clauses } => recover = Some((node.as_str(), clauses.as_slice())),
                Stmt::Ending { rule, kind, ending } => endings.push((rule.as_str(), kind.as_str(), ending)),
                // A mark tier 1 gives no bit is refused nowhere here: `source` refuses the grammar.
                Stmt::Refuse { mark, levels } => {
                    let bit = x.marks.iter().find(|m| &m.name == mark).and_then(|m| m.bit).map_or(0, |b| 1u32 << b);
                    for l in levels {
                        *refusing.entry(l.clone()).or_default() |= if bit == 0 { u32::MAX } else { bit };
                    }
                }
                _ => {}
            }
        }
        let reads = refusing.values().filter(|&&b| b != u32::MAX).fold(0, |a, &b| a | b) | x.extra_bits.iter().fold(0, |a, (b, _)| a | 1 << b);
        // The rules each `leaf K R` reaches.
        let mut recognizers: HashSet<String> = HashSet::new();
        let mut todo: Vec<String> = Vec::new();
        fn syms(g: &Gram, leaves: bool, out: &mut Vec<String>) {
            match g {
                Gram::Leaf { rule, .. } if leaves => out.push(rule.clone()),
                Gram::Sym(n) if !leaves => out.push(n.clone()),
                Gram::Seq(v) | Gram::Alt(v) => v.iter().for_each(|x| syms(x, leaves, out)),
                Gram::Opt(a) | Gram::Star(a) | Gram::Plus(a) | Gram::Field(_, a) | Gram::Retag(_, a) | Gram::Merge(_, a) | Gram::As(_, a) => syms(a, leaves, out),
                Gram::Wrap { body, .. } | Gram::Relabel { body, .. } | Gram::Fold { body, .. } | Gram::Reread { body, .. } | Gram::Flag { body, .. } | Gram::Unless { body, .. } => syms(body, leaves, out),
                _ => {}
            }
        }
        for st in rules.values() {
            if let Stmt::Node { body, .. } | Stmt::Choice { body, .. } = st {
                syms(body, true, &mut todo);
            }
        }
        while let Some(n) = todo.pop() {
            if let Some(Stmt::Node { body, .. } | Stmt::Choice { body, .. }) = rules.get(n.as_str())
                && recognizers.insert(n.clone())
            {
                syms(body, false, &mut todo);
            }
        }
        // Refusals all under leaves: checked where each leaf closes. Every level a refusing level calls refuses the same
        // marks (`ladder`'s check), so a leaf whose rule refuses them refuses them at each of its tokens.
        let mut leaf_refuses = HashMap::new();
        let bits: HashSet<u32> = refusing.values().copied().collect();
        if bits.len() == 1 && refusing.keys().all(|l| recognizers.contains(l)) {
            let mut roots = Vec::new();
            for st in rules.values() {
                if let Stmt::Node { body, .. } | Stmt::Choice { body, .. } = st {
                    syms(body, true, &mut roots);
                }
            }
            // A leaf whose rule refuses none reaches no rule that does: its leaf checks nothing.
            let reaches = |r: &str| -> HashSet<String> {
                let mut seen: HashSet<String> = HashSet::new();
                let mut todo = vec![r.to_owned()];
                while let Some(n) = todo.pop() {
                    if let Some(Stmt::Node { body, .. } | Stmt::Choice { body, .. }) = rules.get(n.as_str())
                        && seen.insert(n.clone())
                    {
                        syms(body, false, &mut todo);
                    }
                }
                seen
            };
            if roots.iter().any(|r| refusing.contains_key(r)) && roots.iter().filter(|r| !refusing.contains_key(*r)).all(|r| reaches(r).iter().all(|n| !refusing.contains_key(n))) {
                leaf_refuses = roots.into_iter().filter(|r| refusing.contains_key(r)).map(|r| (r, bits.iter().copied().next().unwrap_or(0))).collect();
                refusing.clear();
            }
        }
        G { x, rules, order, flags, readings, remember, recover, refusing, endings, reads: std::cell::Cell::new(reads), own_fields: Default::default(), most_field: Default::default(), firsts: Default::default(), busy: Default::default(), kinds: Default::default(), fields: Default::default(), follows: Default::default(), mark_words: Default::default(), call_saves: Default::default(), peek_marks: Default::default(), anon: Default::default(), recognizers, leaf_refuses, leaf_call: Default::default(), cuts: Default::default(), scans: Default::default(), marked_read: Default::default(), names: Default::default(), inline: Default::default(), methods: Default::default() }
    }

    fn mark_of(&self, name: &str) -> Option<usize> {
        self.x.marks.iter().position(|m| m.name == name)
    }

    fn word_of(&self, text: &str) -> Option<u8> {
        self.x.words.iter().position(|w| w.as_slice() == text.as_bytes()).map(|i| (i + 1) as u8)
    }

    /// A keyword's kind of one word (`keywords number { "true" -> true }`): that word, a rule's test of the kind.
    fn keyword_word(&self, kind: &str) -> Option<u8> {
        let mut words = self.x.keywords.iter().flat_map(|r| r.1.iter()).filter(|(_, k)| k.as_deref() == Some(kind));
        let (w, _) = words.next()?;
        if words.next().is_some() {
            return None;
        }
        self.word_of(&String::from_utf8_lossy(w))
    }

    fn is_token_kind(&self, name: &str) -> bool {
        self.x.tokens.iter().any(|(_, n)| n == name || n.strip_suffix("_close") == Some(name))
    }

    /// A kind's constant, `k::NAME`, its id named as the code reads it.
    fn kind_const(&self, name: &str) -> Result<String, String> {
        let id = (self.x.kind)(name).ok_or_else(|| format!("no kind `{name}`"))?;
        let c = upper(name);
        if id > 0x3fe {
            return Err(format!("kind `{name}`: no tape kind"));
        }
        let c = if c.is_empty() { format!("K{id}") } else { c };
        let mut kinds = self.kinds.borrow_mut();
        // Two names one constant (`.` and `dot`): the id names the second.
        let c = match kinds.get(&c) {
            Some(&k) if k != id => format!("{c}_{id}"),
            _ => c,
        };
        kinds.insert(c.clone(), id);
        Ok(format!("k::{c}"))
    }

    /// Field `name`'s id in a node's key, which holds 6 bits: the grammar's id, or for one past them (a field the
    /// tape names not, which a reading renames: `rr_parser`'s own ids)
    /// a slot from the top of the 6 bits down, the same wherever it is named.
    fn field_id(&self, name: &str) -> Result<u16, String> {
        let id = (self.x.field)(name).ok_or_else(|| format!("no field `{name}`"))?;
        if id < 64 {
            self.most_field.set(self.most_field.get().max(id));
            return Ok(id);
        }
        let mut own = self.own_fields.borrow_mut();
        let k = own.iter().position(|&o| o == id).unwrap_or_else(|| {
            own.push(id);
            own.len() - 1
        });
        Ok(63 - k as u16)
    }

    fn field_const(&self, name: &str) -> Result<String, String> {
        // `decide`'s: none, its call's set where it returns.
        if name == ::bitlex::syntax::DECIDED {
            return Ok("0".into());
        }
        let id = self.field_id(name)?;
        let c = upper(name);
        self.fields.borrow_mut().insert(c.clone(), id);
        Ok(format!("f::{c}"))
    }

    /// The tests a grammar's first token can pass, and whether it may match nothing.
    fn first(&self, g: &Gram) -> Result<(Vec<First>, bool), String> {
        Ok(match g {
            Gram::Lit(t) => match self.word_of(t) {
                Some(w) => (vec![(Test::Word(w), None)], false),
                None if self.is_token_kind(t) => (vec![(Test::Kind(t.clone()), None)], false),
                None => return Err(format!("`{t}`: no word or token of that name")),
            },
            Gram::Sym(n) => {
                if let Some(m) = self.mark_of(n) {
                    (vec![(Test::Mark(m), None)], false)
                } else if self.rules.contains_key(n.as_str()) {
                    self.rule_first(n)?
                } else if self.is_token_kind(n) {
                    (vec![(Test::Kind(n.clone()), None)], false)
                } else if let Some(w) = self.keyword_word(n).or_else(|| self.word_of(n)) {
                    // A keyword named by its word (`any`'s tokens, as the build names them): that word.
                    (vec![(Test::Word(w), None)], false)
                } else {
                    return Err(format!("`{n}`: no rule, mark or token of that name"));
                }
            }
            Gram::Seq(gs) => {
                let mut out = Vec::new();
                for g in gs {
                    let (f, empty) = self.first(g)?;
                    out.extend(f);
                    if !empty {
                        return Ok((out, false));
                    }
                }
                (out, true)
            }
            Gram::Alt(gs) => {
                let mut out = Vec::new();
                let mut empty = false;
                for g in gs {
                    let (f, e) = self.first(g)?;
                    out.extend(f);
                    empty |= e;
                }
                (out, empty)
            }
            Gram::Opt(a) | Gram::Star(a) => (self.first(a)?.0, true),
            Gram::Plus(a) | Gram::Field(_, a) | Gram::Retag(_, a) | Gram::Merge(_, a) | Gram::As(_, a) => self.first(a)?,
            Gram::Wrap { body, .. } | Gram::Reread { body, .. } | Gram::Flag { body, .. } | Gram::Relabel { body, .. } | Gram::Fold { body, .. } => self.first(body)?,
            Gram::Unless { flag, body } => {
                let (f, e) = self.first(body)?;
                (f.into_iter().map(|(t, _)| (t, Some(flag.clone()))).collect(), e)
            }
            Gram::Leaf { rule, .. } => self.rule_first(rule)?,
            other => return Err(format!("not written yet: {other:?}")),
        })
    }

    fn rule_first(&self, name: &str) -> Result<(Vec<First>, bool), String> {
        if let Some(f) = self.firsts.borrow().get(name) {
            return Ok(f.clone());
        }
        if !self.busy.borrow_mut().insert(name.to_owned()) {
            return Err(format!("rule `{name}`: left-recursive"));
        }
        let body = match self.rules[name] {
            Stmt::Node { body, .. } | Stmt::Choice { body, .. } => body,
            _ => unreachable!(),
        };
        let f = self.first(body)?;
        self.busy.borrow_mut().remove(name);
        self.firsts.borrow_mut().insert(name.to_owned(), f.clone());
        Ok(f)
    }

    fn rest_first(&self, rest: &[&Gram]) -> Result<(Vec<First>, bool), String> {
        let mut out = Vec::new();
        for g in rest {
            let (f, empty) = self.first(g)?;
            out.extend(f);
            if !empty {
                return Ok((out, false));
            }
        }
        Ok((out, true))
    }

    /// The kind mark `m` holds at alone: a mark of no gap, whose tests are all at that kind's tokens (`mark M = empty,
    /// at K when …`).
    /// The tests of the tokens mark `m` may hold at (each definition's first token), its own test aside: a kind where it
    /// holds at one kind's tokens alone (`implied_kind`).
    fn mark_tests(&self, m: usize) -> Option<Vec<Test>> {
        if let Some(k) = self.implied_kind(m) {
            return Some(vec![Test::Kind(k)]);
        }
        let mut out = Vec::new();
        for d in self.x.marks.iter().filter(|x| x.name == self.x.marks[m].name) {
            for x in d.tokens.and_then(|(_, seq)| seq.first())? {
                match x {
                    TokenElem::Kind(k) => out.push(Test::Kind(k.clone())),
                    TokenElem::Word(w) => out.push(Test::Word(self.word_of(&String::from_utf8_lossy(w))?)),
                    TokenElem::With(_) | TokenElem::Without(_) => {}
                    _ => return None,
                }
            }
        }
        (!out.is_empty()).then_some(out)
    }

    fn implied_kind(&self, m: usize) -> Option<String> {
        let name = &self.x.marks.get(m)?.name;
        self.x.lexer.iter().find_map(|s| match s {
            ::bitlex::syntax::Stmt::Mark { kind, at: ::bitlex::syntax::Expr::Empty, end: false, unless, unplaced: None, checks, next, drops, loses, refuses, refuses_taken, passes, .. }
                if kind == name && unless.is_empty() && next.is_empty() && drops.is_empty() && loses.is_empty() && refuses.is_empty() && refuses_taken.is_empty() && passes.is_empty() && !checks.is_empty() && checks.iter().all(|c| !c.after && c.kind == checks[0].kind) =>
            {
                Some(checks[0].kind.clone())
            }
            _ => None,
        })
    }

    /// A test as a condition on the token.
    fn cond(&self, t: &First) -> Cond {
        let base = match &t.0 {
            Test::Mark(m) => self.mark_test(*m),
            Test::Word(w) => Cond::word(format!("w::{}", self.word_const(*w))),
            Test::Kind(k) => self.kind_cond(k),
        };
        match &t.1 {
            Some(f) => Cond::Group(Box::new(Cond::And(vec![Cond::Flag(format!("FLAG_{}", upper(f))), base]))),
            None => base,
        }
    }

    /// The token is of kind `k` (`kind_test`).
    fn kind_cond(&self, k: &str) -> Cond {
        Cond::Kind { kind: k.to_owned(), test: self.kind_test(k, "self.kind()", "self.word()") }
    }

    fn word_const(&self, w: u8) -> String {
        let text = String::from_utf8_lossy(&self.x.words[usize::from(w) - 1]).into_owned();
        format!("{}_{w}", upper(&text))
    }

    /// The code of a decision among `arms` (each its tests and its code, unindented), as the lean
    /// parser decides at a token: the marks first, in their order (a mark is a token of no width
    /// before its token, tier 2's machine sees it first), consecutive ones that go to one arm one
    /// test; then a `match` on the word, a kind's test a guard of its arm. `ex`: marks that
    /// leave the decision (`exit`, the code after it takes them, or the level ends there,
    /// `exits_ending`). None passes: `none`. An arm reached by two tests apart is numbered, its
    /// code written once. Exits where the level ends at a hidden mark that ends levels and has
    /// questions (`gated`) are tested only where the caller's level is the mark's (`asks`):
    /// elsewhere (inside parentheses) the mark is passed, as the lean parser asks `asi` only where
    /// `stmt` is set.
    fn decide(&self, arms: &[(Vec<First>, Block)], ex: &[(usize, bool)], unmarked: &[usize], exit: &Block, none: &Block) -> Block {
        let mut code = self.decide_tests(arms, ex, unmarked, exit, none);
        // A bracket's scan (`scan_fn`) its own tests ask more than once at the token: once, before them, as the lean
        // parser scans to the `)` once and asks what follows (decision 0673 step 11).
        let mut lets = Block::new();
        for (_, f) in self.scans.borrow().iter() {
            let asked: usize = code.iter_mut().map(|op| match op {
                Op::If { arms, .. } => arms.iter_mut().map(|(c, _)| c.scans(f)).sum(),
                _ => 0,
            }).sum();
            if asked < 2 {
                continue;
            }
            lets.push(line(format!("let {f} = self.{f}(self.i);")));
            // The move right after one of its tests is at the same token: a peek's test there (`no_retests`) reads it too.
            for op in &mut code {
                if let Op::If { arms, .. } = op {
                    for (c, body) in arms {
                        c.hold_scan(f);
                        if let Some(t) = Cond::first_test(body) {
                            t.hold_scan(f);
                        }
                    }
                }
            }
        }
        lets.extend(code);
        lets
    }

    fn decide_tests(&self, arms: &[(Vec<First>, Block)], ex: &[(usize, bool)], unmarked: &[usize], exit: &Block, none: &Block) -> Block {
        let exits: Vec<usize> = ex.iter().map(|e| e.0).collect();
        let gated = self.gated(ex);
        let n = arms.len();
        let mut marks: Vec<(usize, Option<String>, usize)> = Vec::new();
        let mut words: Vec<(u8, Option<String>, usize)> = Vec::new();
        let mut kinds: Vec<(String, Option<String>, usize)> = Vec::new();
        for (i, (fs, _)) in arms.iter().enumerate() {
            for (t, f) in fs {
                match t {
                    Test::Mark(m) if !marks.iter().any(|x| x.0 == *m && x.1 == *f) => marks.push((*m, f.clone(), i)),
                    Test::Word(w) if !words.iter().any(|x| x.0 == *w && x.1 == *f) => words.push((*w, f.clone(), i)),
                    Test::Kind(k) if !kinds.iter().any(|x| x.0 == *k && x.1 == *f) => kinds.push((k.clone(), f.clone(), i)),
                    _ => {}
                }
            }
        }
        // A mark that holds only at one kind's tokens, where the kind goes to the same arm: no test of its own, as the
        // lean parser's one arm for a kind (`NUM | FLOAT`).
        marks.retain(|(m, f, t)| !self.implied_kind(*m).is_some_and(|k| kinds.iter().any(|x| x.0 == k && x.1 == *f && x.2 == *t)));
        // A peek whose first words all go to its own arm: no test of its own (decision 0673 step 3). Where it holds, its
        // first word does, and the word's test takes the arm; the lean parser tests the token once.
        // Another mark tested after it (the marks are tested in their order) that may hold at its words and takes another
        // arm keeps it: there the peek's test comes first. One tested before it is asked first either way (`.end` before
        // a directive's word).
        let peeks = self.peeks();
        let firsts = |m: usize| -> Option<Vec<String>> {
            let defs = self.x.marks.iter().filter(|x| x.name == self.x.marks[m].name);
            defs.map(|d| d.tokens.and_then(|(_, seq)| self.first_words(seq))).collect::<Option<Vec<Vec<String>>>>().map(|v| v.concat())
        };
        let all = marks.clone();
        marks.retain(|(m, f, t)| {
            let Some(ws) = firsts(*m).filter(|ws| peeks >> *m & 1 == 1 && !ws.is_empty()) else { return true };
            let covered = ws.iter().all(|w| words.iter().any(|x| format!("w::{}", self.word_const(x.0)) == *w && x.1 == *f && x.2 == *t));
            let alone = all.iter().all(|o| o.0 < *m || o.2 == *t || firsts(o.0).is_some_and(|os| os.iter().all(|w| !ws.contains(w))));
            !(covered && alone)
        });
        // An exit where leaving does what passing every arm does, at whose first words no arm's test can hold: no test
        // (decision 0673 step 8). Where it holds, no arm is taken either way, as the lean parser's `if exactly { … }`
        // asks nothing of the statement that may follow. A mark tested before it takes its arm either way.
        let arm_words: Vec<String> = words.iter().map(|x| format!("w::{}", self.word_const(x.0))).collect();
        let exits: Vec<usize> = exits
            .into_iter()
            .filter(|&m| {
                if exit != none || gated.contains(&m) || unmarked.contains(&m) || self.asks(m) || marks.iter().any(|x| x.0 == m) {
                    return true;
                }
                let Some(ws) = firsts(m).filter(|ws| !ws.is_empty()) else { return true };
                ws.iter().any(|w| arm_words.contains(w))
                    || kinds.iter().any(|k| self.kind_words(&k.0).iter().any(|w| ws.contains(w)))
                    || marks.iter().any(|o| o.0 > m && firsts(o.0).is_none_or(|os| os.iter().any(|w| ws.contains(w))))
            })
            .collect();
        for &m in &exits {
            if !marks.iter().any(|x| x.0 == m) {
                marks.push((m, None, n));
            }
        }
        marks.sort_by_key(|x| x.0);
        let guard = |f: &Option<String>, c: Cond| match f {
            Some(f) => Cond::And(vec![Cond::Flag(format!("FLAG_{}", upper(f))), c]),
            None => c,
        };
        let mut mg: Vec<(Vec<Cond>, usize)> = Vec::new();
        // The marks of the last run of marks, by arm.
        let mut last_marks: Vec<usize> = Vec::new();
        for (m, f, t) in &marks {
            let c = guard(f, self.mark_test(*m));
            let c = if *t == n && unmarked.contains(m) {
                self.marked_read.set(true);
                Cond::And(vec![Cond::text("!self.marked"), c])
            } else {
                c
            };
            let c = if *t == n && gated.contains(m) { Cond::And(vec![Cond::text(format!("self.asks >> {m} & 1 != 0")), c]) } else { c };
            match mg.last_mut() {
                Some((cs, tt)) if tt == t => {
                    cs.push(c);
                    last_marks.push(*m);
                }
                _ => {
                    mg.push((vec![c], *t));
                    last_marks = vec![*m];
                }
            }
        }
        let mut wg: Vec<(Pat, usize)> = Vec::new();
        let mut order: Vec<usize> = Vec::new();
        for (_, f, t) in &words {
            if f.is_none() && !order.contains(t) {
                order.push(*t);
            }
        }
        for t in order {
            let pats: Vec<String> = words.iter().filter(|x| x.1.is_none() && x.2 == t).map(|x| format!("w::{}", self.word_const(x.0))).collect();
            wg.push((Pat::words(pats), t));
        }
        for (w, f, t) in words.iter().filter(|x| x.1.is_some()) {
            wg.push((Pat { words: vec![format!("w::{}", self.word_const(*w))], guard: Some(Cond::Flag(format!("FLAG_{}", upper(f.as_deref().unwrap_or_default())))) }, *t));
        }
        let mut kg: Vec<(Vec<Cond>, usize)> = Vec::new();
        for (k, f, t) in &kinds {
            let c = guard(f, self.kind_cond(k));
            match kg.last_mut() {
                Some((cs, tt)) if tt == t => cs.push(c),
                _ => kg.push((vec![c], *t)),
            }
        }
        let body_of = |t: usize| if t == n { exit.clone() } else { arms[t].1.clone() };
        let mut uses = vec![0usize; n + 1];
        for t in mg.iter().map(|g| g.1).chain(wg.iter().map(|g| g.1)).chain(kg.iter().map(|g| g.1)) {
            uses[t] += 1;
        }
        let numbered = (0..=n).any(|t| uses[t] > 1 && moves::print(&body_of(t), "").lines().count() > 1);
        let none_empty = moves::print(none, "").trim().is_empty();
        // One arm reached by every test, none a mark's or a flag's: one condition, as the lean parser's `if`. With one
        // arm there is no order among arms to keep, so the tests need no numbering.
        let targets: Vec<usize> = (0..n).filter(|&t| uses[t] > 0).collect();
        if numbered && mg.is_empty() && uses[n] == 0 && targets.len() == 1 && words.iter().all(|w| w.1.is_none()) && kinds.iter().all(|k| k.1.is_none()) {
            let all: Vec<String> = wg.iter().flat_map(|g| g.0.words.iter().cloned()).collect();
            let mut cs = Vec::new();
            if !all.is_empty() {
                cs.push(Cond::Word { at: 0, ws: all });
            }
            cs.extend(kg.iter().flat_map(|g| g.0.iter().cloned()));
            return vec![Op::If { arms: vec![(Cond::Or(cs), body_of(targets[0]))], els: (!none_empty).then(|| none.clone()) }];
        }
        // Each arm's tests one run in the order they are asked (the marks', then the words', of which no two hold at
        // once, then the kinds'): an `if` chain of one condition an arm, as the lean parser's (decision 0673 step 3).
        if numbered && words.iter().all(|w| w.1.is_none()) {
            let push = |c: Cond, t: usize, runs: &mut Vec<(Vec<Cond>, usize)>| match runs.last_mut() {
                Some((cs, tt)) if *tt == t => cs.push(c),
                _ => runs.push((vec![c], t)),
            };
            let mut runs: Vec<(Vec<Cond>, usize)> = Vec::new();
            let word_test = |pat: &Pat| Cond::Word { at: 0, ws: pat.words.clone() };
            // A word another arm than the last marks' takes goes before those marks where none of them can hold at it:
            // the marks' arm and its words are then one run.
            let (last, next) = (mg.last().map(|g| g.1), kg.first().map(|g| g.1));
            let held_at = |pat: &Pat| last_marks.iter().any(|&m| firsts(m).is_none_or(|ws| pat.words.iter().any(|w| ws.contains(w))));
            let early: Vec<&(Pat, usize)> = wg.iter().filter(|g| last.is_some_and(|l| l != g.1) && !held_at(&g.0)).collect();
            // So does a kind another arm takes, where none of those marks, and none of the words after, can be its
            // token: a token of a kind is one of its keywords rows' words, or no word.
            let late_words: Vec<String> = wg.iter().filter(|g| !early.contains(g)).flat_map(|g| g.0.words.iter().cloned()).collect();
            let mark_words: Option<Vec<String>> = last_marks.iter().map(|&m| firsts(m)).collect::<Option<Vec<_>>>().map(|v| v.concat());
            let early_kinds: Vec<(Cond, usize)> = match &mark_words {
                Some(mw) => kinds
                    .iter()
                    .filter(|(_, f, t)| f.is_none() && last.is_some_and(|l| l != *t) && kinds.iter().filter(|x| x.2 == *t).count() == 1)
                    .filter(|(k, _, _)| !mw.iter().chain(&late_words).any(|w| self.kind_words(k).contains(w)))
                    .map(|(k, f, t)| (guard(f, self.kind_cond(k)), *t))
                    .collect(),
                None => vec![],
            };
            for (i, (cs, t)) in mg.iter().enumerate() {
                if i + 1 == mg.len() {
                    for (pat, t) in &early {
                        push(word_test(pat), *t, &mut runs);
                    }
                    for (c, t) in &early_kinds {
                        push(c.clone(), *t, &mut runs);
                    }
                }
                cs.iter().for_each(|c| push(c.clone(), *t, &mut runs));
            }
            // The other words in any order: the one the last mark's arm takes first, the first kind's last.
            let mut ws: Vec<&(Pat, usize)> = wg.iter().filter(|g| !early.contains(g)).collect();
            ws.sort_by_key(|g| if Some(g.1) == last { 0 } else if Some(g.1) == next { 2 } else { 1 });
            for (pat, t) in ws {
                push(word_test(pat), *t, &mut runs);
            }
            for (cs, t) in &kg {
                cs.iter().filter(|c| !early_kinds.iter().any(|e| e.0 == **c)).for_each(|c| push(c.clone(), *t, &mut runs));
            }
            // A run joins an earlier run of its arm where no run between can hold at a token it holds at: their tests
            // ask apart words, a word none of a kind's tokens is, or kinds of apart tokens (a token is of one kind). The
            // lean parser tests an arm's tokens together (decision 0673 steps 8 and 11).
            let holds = |c: &Cond| self.holds(c);
            let apart = |a: &Holds, b: &Holds| self.apart(a, b);
            let mut j = 1;
            while j < runs.len() {
                let t = runs[j].1;
                let mine: Vec<Holds> = runs[j].0.iter().map(holds).collect();
                let earlier = runs[..j].iter().rposition(|r| r.1 == t);
                let clear = |i: usize, of: &[Holds]| runs[i + 1..j].iter().all(|r| r.0.iter().all(|c| of.iter().all(|m| apart(m, &holds(c)))));
                let join = earlier.filter(|&i| clear(i, &mine));
                // Else the earlier run goes down to this one, where the runs between are apart from it.
                if join.is_none()
                    && let Some(i) = earlier
                    && clear(i, &runs[i].0.iter().map(holds).collect::<Vec<_>>())
                {
                    let mut cs = runs.remove(i).0;
                    cs.extend(std::mem::take(&mut runs[j - 1].0));
                    runs[j - 1].0 = cs;
                    j = i.max(1);
                    continue;
                }
                match join {
                    Some(i) => {
                        let cs = runs.remove(j).0;
                        runs[i].0.extend(cs);
                        // The runs now side by side of one arm are one.
                        if i + 1 < runs.len() && runs[i + 1].1 == t {
                            let cs = runs.remove(i + 1).0;
                            runs[i].0.extend(cs);
                        }
                        j = i + 1;
                    }
                    None => j += 1,
                }
            }
            let short = |t: usize| {
                let b = moves::print(&body_of(t), "");
                b.lines().filter(|l| l.contains("?;") || l.contains("continue 'run;") || l.contains("return ")).count() <= 1 && b.lines().count() <= 6 && !b.contains('{')
            };
            // An arm still reached apart whose code is long: one condition, the runs between negated before its later
            // tests, as the lean parser's `if a || !e && b`. The runs between stay where they are, and decide where they hold.
            let mut j = 1;
            while j < runs.len() {
                let t = runs[j].1;
                if !short(t)
                    && let Some(i) = runs[..j].iter().rposition(|r| r.1 == t)
                {
                    let between: Vec<Cond> = runs[i + 1..j].iter().flat_map(|r| r.0.iter().cloned()).collect();
                    let mine = runs.remove(j).0;
                    runs[i].0.push(Cond::And(vec![Cond::Not(Box::new(Cond::Or(between))), Cond::Or(mine)]));
                    continue;
                }
                j += 1;
            }
            let mut seen = HashSet::new();
            // An arm the runs reach apart: its code in each, where it is one call or take (with the saves around it), as
            // the lean parser writes a call twice (decision 0673 step 11); else the arms are numbered.
            if runs.iter().all(|r| seen.insert(r.1) || short(r.1)) {
                let arms = runs.into_iter().map(|(cs, t)| (Cond::Or(cs), body_of(t))).collect();
                return vec![Op::If { arms, els: (!none_empty).then(|| none.clone()) }];
            }
        }
        const NONE_ARM: usize = usize::MAX;
        let leaf = |t: usize| -> Block { if numbered { vec![line(format!("arm = {};", if t == NONE_ARM { "usize::MAX".to_owned() } else { t.to_string() }))] } else if t == NONE_ARM { none.clone() } else { body_of(t) } };
        let kind_arms = || -> Vec<(Cond, Block)> { kg.iter().map(|(cs, t)| (Cond::Or(cs.clone()), leaf(*t))).collect() };
        // The word's match and the kinds' tests.
        let rest = || -> Block {
            if !wg.is_empty() {
                let mut arms: Vec<(Pat, Block)> = wg.iter().map(|(pat, t)| (pat.clone(), leaf(*t))).collect();
                arms.extend(kg.iter().map(|(cs, t)| (Pat::guard(Cond::Or(cs.clone())), leaf(*t))));
                vec![Op::Match { subject: "self.word()".into(), arms, default: leaf(NONE_ARM) }]
            } else if !kg.is_empty() {
                vec![Op::If { arms: kind_arms(), els: Some(leaf(NONE_ARM)) }]
            } else {
                leaf(NONE_ARM)
            }
        };
        let sel = if mg.is_empty() {
            rest()
        } else {
            let mut arms: Vec<(Cond, Block)> = mg.iter().map(|(cs, t)| (Cond::Or(cs.clone()), leaf(*t))).collect();
            // The kinds' tests after the marks', one `if` chain: `else if`, as the lean parser writes it.
            if wg.is_empty() && !kg.is_empty() {
                arms.extend(kind_arms());
                vec![Op::If { arms, els: Some(leaf(NONE_ARM)) }]
            } else {
                vec![Op::If { arms, els: Some(rest()) }]
            }
        };
        if !numbered {
            return sel;
        }
        let mut out = vec![line("let arm: usize;")];
        out.extend(sel);
        let arms = uses.iter().enumerate().filter(|&(_, &u)| u > 0).map(|(t, _)| (Pat::words(vec![t.to_string()]), body_of(t))).collect();
        out.push(Op::Match { subject: "arm".into(), arms, default: none.clone() });
        out
    }

    /// Where test `c` may hold, as `decide_tests` joins runs: at a token that is one of `.0`'s words or of one of `.1`'s
    /// kinds; None, anywhere. A word's test its words, a kind's its kind, a mark's its definitions' first tokens; a test
    /// under others (a flag, `marked`) where its last does.
    /// Whether no token is where both `a` and `b` may hold: apart words, a word none of a kind's tokens is, or kinds of
    /// apart tokens (a token is of one kind).
    fn apart(&self, a: &Holds, b: &Holds) -> bool {
        let tokens_of = |k: &str| -> Vec<&str> { self.x.tokens.iter().filter(|(_, n)| n == k || n.strip_suffix("_close") == Some(k)).map(|(_, n)| n.as_str()).collect() };
        let (Some((aw, ak)), Some((bw, bk))) = (a, b) else { return false };
        let word_kind = |ws: &[String], ks: &[String]| {
            ks.iter().all(|k| {
                let kw = self.kind_words(k);
                ws.iter().all(|w| !kw.contains(w))
            })
        };
        aw.iter().all(|w| !bw.contains(w)) && word_kind(aw, bk) && word_kind(bw, ak) && ak.iter().all(|k| bk.iter().all(|o| k != o && tokens_of(o).iter().all(|x| !tokens_of(k).contains(x))))
    }

    /// Where a first token's test may hold (`holds`).
    fn first_holds(&self, t: &Test) -> Holds {
        match t {
            Test::Word(w) => Some((vec![format!("w::{}", self.word_const(*w))], vec![])),
            Test::Kind(k) => Some((vec![], vec![k.clone()])),
            Test::Mark(m) => self.holds(&Cond::Mark { mark: *m, test: Box::new(Cond::text("")) }),
        }
    }

    fn holds(&self, c: &Cond) -> Holds {
        match c.core() {
            Cond::Word { at: 0, ws } => Some((ws.clone(), vec![])),
            Cond::Kind { kind, .. } => Some((vec![], vec![kind.clone()])),
            Cond::And(cs) if cs.last().is_some_and(|l| matches!(l.core(), Cond::Kind { .. } | Cond::Mark { .. } | Cond::And(_))) => self.holds(cs.last().expect("a test")),
            Cond::Mark { mark, .. } => {
                // A mark: each definition's first token, a choice of words and kinds.
                let (mut ws, mut ks) = (Vec::new(), Vec::new());
                for d in self.x.marks.iter().filter(|x| x.name == self.x.marks[*mark].name) {
                    for x in d.tokens.and_then(|(_, seq)| seq.first())? {
                        match x {
                            TokenElem::Kind(k) => ks.push(k.clone()),
                            TokenElem::Word(w) => ws.push(format!("w::{}", self.word_const(self.word_of(&String::from_utf8_lossy(w))?))),
                            TokenElem::With(_) | TokenElem::Without(_) => {}
                            _ => return None,
                        }
                    }
                }
                Some((ws, ks))
            }
            _ => None,
        }
    }

    /// Whether an arm's tests are words and kinds alone, no flag's: a mark only where it holds at one kind's tokens
    /// alone, which the arm takes (no test of its own, `decide`).
    fn plain_tests(&self, fs: &[First]) -> bool {
        let implied = |m: usize| self.implied_kind(m).is_some_and(|k| fs.iter().any(|x| matches!(&x.0, Test::Kind(j) if *j == k)));
        fs.iter().all(|(t, f)| f.is_none() && (matches!(t, Test::Word(_) | Test::Kind(_)) || matches!(t, Test::Mark(m) if implied(*m))))
    }

    /// Whether `g` begins with a call of a level (a nested rule's method), which tests its own first token.
    fn leads_with_call(&self, g: &Gram) -> bool {
        match g {
            Gram::Sym(n) => self.token_of(g).is_none() && self.mark_of(n).is_none() && !self.remember.contains(n) && self.rules.contains_key(n.as_str()) && self.nested(n),
            // A wrap and a relabel take no token: the call after them tests it.
            Gram::Field(_, a) | Gram::Wrap { body: a, .. } | Gram::Relabel { body: a, .. } => self.leads_with_call(a),
            Gram::Seq(gs) => gs.first().is_some_and(|a| self.leads_with_call(a)),
            _ => false,
        }
    }

    /// The alternatives of `g`: an alternation's, flattened; else `g`.
    fn alts<'g>(g: &'g Gram, out: &mut Vec<&'g Gram>) {
        match g {
            Gram::Alt(gs) => gs.iter().for_each(|a| Self::alts(a, out)),
            _ => out.push(g),
        }
    }

    /// The arms of a decision among `g`'s alternatives, each before `rest`: its tests, its code
    /// (an `unless F` alternative's guard is its tests'), and whether it may match nothing.
    fn arms(&self, g: &Gram, rest: &[&Gram], rule: &str) -> Result<Vec<(Vec<First>, Block, bool)>, String> {
        let mut alts = Vec::new();
        Self::alts(g, &mut alts);
        let mut out = Vec::new();
        for a in alts {
            let (f, e) = self.first(a)?;
            let body = match a {
                Gram::Unless { body, .. } => self.code(body, rest, rule)?,
                _ => self.code(a, rest, rule)?,
            };
            out.push((f, body, e));
        }
        Ok(out)
    }

    /// The marks after a decision, before `rest` in `rule`: the rest's first, and where the
    /// rule may end there, its follow (its level's, a rule inlined in it), or the marks that end
    /// a level (`, ends`) where the rule is a level of its own.
    /// Each with whether the level ends there (a mark that ends a level, `, ends`, at a
    /// rule that is one) rather than taking it.
    fn exits_ending(&self, rest: &[&Gram], rule: &str) -> Result<Vec<(usize, bool)>, String> {
        let (fr, may_end) = self.rest_first(rest)?;
        let mut out: Vec<(usize, bool)> = fr.iter().filter_map(|t| if let Test::Mark(m) = t.0 { Some((m, false)) } else { None }).collect();
        if may_end {
            let end = self.nested(rule);
            out.extend(self.follow(rule)?.into_iter().map(|m| (m, end)));
        }
        out.sort_unstable();
        out.dedup_by_key(|e| e.0);
        Ok(out)
    }

    /// The exits where the level ends at a hidden mark that ends levels (`, ends`): not tested
    /// right after a mark is taken (`marked`), as the reference's state entered by mark arms
    /// alone (`after_mark_only`) does not end there.
    fn after_mark(&self, rest: &[&Gram], rule: &str) -> Result<Vec<usize>, String> {
        let (fr, may_end) = self.rest_first(rest)?;
        if !may_end {
            return Ok(Vec::new());
        }
        let first: Vec<usize> = fr.iter().filter_map(|t| if let Test::Mark(m) = t.0 { Some(m) } else { None }).collect();
        let ending = self.ending();
        Ok(self.follow(rule)?.into_iter().filter(|m| !first.contains(m) && ending.contains(m) && self.x.marks[*m].hidden).collect())
    }

    /// The exits of `ex` where the level ends at a hidden mark that ends levels and has
    /// questions: those `decide` tests only where `asks` has the mark (`gated`).
    fn gated(&self, ex: &[(usize, bool)]) -> Vec<usize> {
        let ending = self.ending();
        ex.iter().filter(|e| e.1 && ending.contains(&e.0) && self.x.marks[e.0].hidden && self.asks(e.0)).map(|e| e.0).collect()
    }

    /// Whether mark `m`'s name has questions a decision asks (`refuses`, `loses`).
    fn asks(&self, m: usize) -> bool {
        let name = &self.x.marks[m].name;
        self.x.marks.iter().any(|x| &x.name == name && (!x.refuses.is_empty() || !x.loses.is_empty()))
    }

    /// The questions a decision asks before its tests: each mark it tests that has some, at an
    /// arm, or where the level may end at it (`end`).
    fn asking(&self, arms: &[(Vec<First>, Block)], exits: &[(usize, bool)]) -> Block {
        let mut asked: Vec<(usize, bool)> = Vec::new();
        for (fs, _) in arms {
            for (t, _) in fs {
                if let Test::Mark(m) = t
                    && !asked.iter().any(|a| a.0 == *m)
                {
                    asked.push((*m, false));
                }
            }
        }
        for &(m, end) in exits {
            if !asked.iter().any(|a| a.0 == m) {
                asked.push((m, end));
            }
        }
        asked.sort_unstable();
        let mut out = Block::new();
        for (m, end) in asked.into_iter().filter(|a| self.asks(a.0)) {
            out.push(line(format!("self.ask_{}({end})?;", ident(&self.x.marks[m].name))));
        }
        out
    }

    /// Whether level `rule` counts the depth (`deeper`): it is one of the levels that cut every cycle of the rules' calls
    /// (`depth_cuts`). A call off every cycle nests no deeper than the grammar is long, so only a cycle's call can
    /// nest without bound, as the lean parser's recursion is only where its grammar's is.
    fn counts_depth(&self, rule: &str) -> bool {
        self.cuts.borrow_mut().get_or_insert_with(|| self.depth_cuts()).contains(rule)
    }

    /// The levels that count the depth: in the grammar's order, each level still on a cycle of calls once the levels
    /// before it that count are taken out; every level where a cycle has none but rules that are no level.
    fn depth_cuts(&self) -> HashSet<String> {
        let mut calls: HashMap<&str, Vec<String>> = HashMap::new();
        for (n, st) in &self.rules {
            if let Stmt::Node { body, .. } | Stmt::Choice { body, .. } = st {
                let mut out = Vec::new();
                called(body, &mut out);
                out.retain(|c| self.rules.contains_key(c.as_str()));
                calls.insert(n, out);
            }
        }
        // Whether `from` reaches `to` by calls, through no rule of `cut`.
        let reaches = |from: &str, to: &str, cut: &HashSet<String>| {
            let mut seen: HashSet<&str> = HashSet::new();
            let mut todo: Vec<&str> = calls.get(from).map(|v| v.iter().map(String::as_str).collect()).unwrap_or_default();
            while let Some(c) = todo.pop() {
                if c == to {
                    return true;
                }
                if cut.contains(c) || !seen.insert(c) {
                    continue;
                }
                todo.extend(calls.get(c).into_iter().flatten().map(String::as_str));
            }
            false
        };
        let mut cut = HashSet::new();
        for n in &self.order {
            if self.nested(n) && reaches(n, n, &cut) {
                cut.insert((*n).to_owned());
            }
        }
        if self.order.iter().any(|n| !cut.contains(*n) && reaches(n, n, &cut)) {
            return self.order.iter().filter(|n| self.nested(n)).map(|n| (*n).to_owned()).collect();
        }
        cut
    }

    fn nested(&self, rule: &str) -> bool {
        matches!(self.rules.get(rule), Some(Stmt::Node { nested: true, .. } | Stmt::Choice { nested: true, .. })) || self.order.first() == Some(&rule)
    }

    /// The marks that end a level (`, ends`), by their order.
    fn ending(&self) -> Vec<usize> {
        (0..self.x.marks.len()).filter(|&m| self.x.marks[m].ends && self.mark_of(&self.x.marks[m].name) == Some(m)).collect()
    }

    /// The marks after `rule` where it ends: those of its level, after each call of it.
    fn follow(&self, rule: &str) -> Result<Vec<usize>, String> {
        if self.nested(rule) {
            return Ok(if self.order.first() == Some(&rule) { Vec::new() } else { self.ending() });
        }
        if self.follows.borrow().is_none() {
            let mut sites = Vec::new();
            for name in &self.order {
                let body = match self.rules[name] {
                    Stmt::Node { body, .. } | Stmt::Choice { body, .. } => body,
                    _ => unreachable!(),
                };
                self.sites(body, &[], name, &mut sites)?;
            }
            let mut follows: HashMap<String, Vec<usize>> = HashMap::new();
            loop {
                let mut changed = false;
                for (callee, marks, may_end, caller) in &sites {
                    let mut add = marks.clone();
                    if *may_end {
                        if self.nested(caller) {
                            if self.order.first() != Some(&caller.as_str()) {
                                add.extend(self.ending());
                            }
                        } else {
                            add.extend(follows.get(caller).cloned().unwrap_or_default());
                        }
                    }
                    let f = follows.entry(callee.clone()).or_default();
                    for m in add {
                        if !f.contains(&m) {
                            f.push(m);
                            changed = true;
                        }
                    }
                }
                if !changed {
                    break;
                }
            }
            *self.follows.borrow_mut() = Some(follows);
        }
        let mut f = self.follows.borrow().as_ref().and_then(|f| f.get(rule).cloned()).unwrap_or_default();
        f.sort_unstable();
        Ok(f)
    }

    /// Each call of a rule in `g` before `rest`: the callee, the marks first after it, whether
    /// the caller may end there, the caller.
    fn sites(&self, g: &Gram, rest: &[&Gram], caller: &str, out: &mut Vec<(String, Vec<usize>, bool, String)>) -> Result<(), String> {
        match g {
            Gram::Sym(n) if self.mark_of(n).is_none() && self.rules.contains_key(n.as_str()) => {
                let (fr, may_end) = self.rest_first(rest)?;
                let marks = fr.iter().filter_map(|t| if let Test::Mark(m) = t.0 { Some(m) } else { None }).collect();
                out.push((n.clone(), marks, may_end, caller.to_owned()));
            }
            Gram::Seq(gs) => {
                for (i, x) in gs.iter().enumerate() {
                    let mut r: Vec<&Gram> = gs[i + 1..].iter().collect();
                    r.extend_from_slice(rest);
                    self.sites(x, &r, caller, out)?;
                }
            }
            Gram::Alt(gs) => {
                for x in gs {
                    self.sites(x, rest, caller, out)?;
                }
            }
            Gram::Star(a) | Gram::Plus(a) => {
                let mut r: Vec<&Gram> = vec![g];
                r.extend_from_slice(rest);
                self.sites(a, &r, caller, out)?;
            }
            Gram::Opt(a) | Gram::Field(_, a) | Gram::Retag(_, a) | Gram::Merge(_, a) | Gram::As(_, a) => self.sites(a, rest, caller, out)?,
            Gram::Wrap { body, .. } | Gram::Reread { body, .. } | Gram::Flag { body, .. } | Gram::Unless { body, .. } => self.sites(body, rest, caller, out)?,
            Gram::Leaf { rule, .. } => self.sites(&Gram::Sym(rule.clone()), rest, caller, out)?,
            _ => {}
        }
        Ok(())
    }

    /// `g`'s code, `rest` the rule's elements after it.
    fn code(&self, g: &Gram, rest: &[&Gram], rule: &str) -> Result<Block, String> {
        Ok(match g {
            // A token a `leaf`'s rule reads: into the leaf, no leaf of its own.
            Gram::Lit(_) | Gram::Sym(_) | Gram::As(_, _) | Gram::Merge(_, _) if self.recognizers.contains(rule) && self.token_of(g).or_else(|| if let Gram::As(_, a) | Gram::Merge(_, a) = g { self.token_of(a) } else { None }).is_some() => {
                let t = self.token_of(g).or_else(|| if let Gram::As(_, a) | Gram::Merge(_, a) = g { self.token_of(a) } else { None }).expect("a token");
                let take = match t.0 {
                    Test::Word(w) => Take::WordPass { word: format!("w::{}", self.word_const(w)) },
                    test => Take::Pass { test: self.token_cond(&test) },
                };
                vec![moves::take(take)]
            }
            Gram::Field(_, a) if self.recognizers.contains(rule) => self.code(a, rest, rule)?,
            // `leaf K R`: `R` reads the tokens, then one leaf of `K` over them, as the lean parser's `json` reads the
            // braces and makes its leaf.
            Gram::Leaf { kind, rule: r } => {
                self.leaf_call.set(true);
                let mut out = vec![line("self.leaf_open();")];
                out.extend(self.code(&Gram::Sym(r.clone()), rest, rule)?);
                out.extend(self.leaf_end(r, kind)?);
                out
            }
            Gram::Lit(_) | Gram::Sym(_) if self.token_of(g).is_some() => {
                let (test, kind) = self.token_of(g).expect("a token");
                self.take_token(&test, Some(&kind))?
            }
            Gram::As(k, a) if k == ::bitlex::syntax::IMPLIED => {
                let (test, _) = self.token_of(a).ok_or_else(|| format!("rule {rule}: an implied token that is no token"))?;
                self.take_token(&test, None)?
            }
            Gram::As(k, a) if self.token_of(a).is_none() && matches!(&**a, Gram::Sym(n) if self.rules.contains_key(n.as_str()) && !self.nested(n)) => {
                let Gram::Sym(n) = &**a else { unreachable!() };
                self.token_choice(n, Some(k), rule)?
            }
            Gram::As(k, a) => {
                let (test, _) = self.token_of(a).ok_or_else(|| format!("rule {rule}: `as {k}` of no token"))?;
                self.take_token(&test, Some(k))?
            }
            Gram::Sym(n) if self.remember.contains(n) => self.token_choice(n, None, rule)?,
            Gram::Sym(n) if self.mark_of(n).is_some() => {
                let o = self.mark_of(n).expect("a mark");
                if self.peeks() >> o & 1 == 1 {
                    // A peek where the rule takes it: it must hold (none where the decision before tested it, `no_retests`).
                    vec![Op::If { arms: vec![(Cond::Not(Box::new(Cond::Mark { mark: o, test: Box::new(self.peek(o)?) })), vec![Op::Refuse])], els: None }]
                } else {
                    vec![line(format!("self.mark_{}()?;", ident(&self.x.marks[o].name)))]
                }
            }
            // A rule under a `leaf` writes nothing: no rule outside one calls it.
            Gram::Sym(n) if self.recognizers.contains(n) && !self.recognizers.contains(rule) && !self.leaf_call.replace(false) => return Err(format!("rule {rule}: calls `{n}`, which a `leaf` reads, outside a leaf")),
            Gram::Sym(n) if self.rules.contains_key(n.as_str()) && self.nested(n) => {
                // A level's call: the questions it asks where it may end at a mark, those of the
                // level it returns to (set where that takes the mark, kept where it ends there too).
                let (set, keep, all) = self.call_asks(rest, rule)?;
                let mut call = if all == 0 {
                    vec![Op::Call(rule_fn(n))]
                } else {
                    vec![line(format!("let asks = self.asks;\nself.asks = asks & {keep:#x} | {set:#x};\nself.{}()?;\nself.asks = asks;", rule_fn(n)))]
                };
                let saves = self.call_saves.borrow();
                // Where only some levels set the word (`call_saves`), each of those keeps its caller's itself (`rule`).
                let (refuses, words) = match saves.as_ref() {
                    Some((r, _)) => (r.contains(n.as_str()), false),
                    None => (true, true),
                };
                // The level's word is the caller's again where the call returns (the machine's `PUSH`).
                if !self.remember.is_empty() && words {
                    call.insert(0, line("let word = self.word;"));
                    call.push(line("self.word = word;"));
                }
                // The marks the callee refuses (`refuse M in L`), the caller's again where it returns.
                if !self.refusing.is_empty() && refuses {
                    call.insert(0, line(format!("let refusing = self.refusing;\nself.refusing = {:#x};", self.refusing.get(n.as_str()).copied().unwrap_or(0))));
                    call.push(line("self.refusing = refusing;"));
                }
                call
            }
            // A choice of tokens, each its own word's leaf: one take where it is used, as the lean parser's `anon()`.
            Gram::Sym(n) if let Some((ws, Some((k, x)))) = self.anon_choice(n) => {
                let words: Vec<String> = ws.iter().map(|(w, _)| format!("w::{}", self.word_const(*w))).collect();
                let kind = if let [(_, t)] = ws.as_slice() { self.kind_const(t)? } else { self.anon_table(Some(n), &ws)? };
                let test = Cond::Word { at: 0, ws: words };
                let take = |test: Cond, kind: String| vec![moves::take(Take::Leaf { test, kind, field: "0".into() })];
                vec![Op::If { arms: vec![(test.clone(), take(test, kind))], els: Some(take(self.kind_cond(&k), self.kind_const(&x)?)) }]
            }
            Gram::Sym(n) if let Some((ws, None)) = self.anon_choice(n) => match ws.as_slice() {
                [(w, t)] => self.take_token(&Test::Word(*w), Some(t))?,
                _ => {
                    let words: Vec<String> = ws.iter().map(|(w, _)| format!("w::{}", self.word_const(*w))).collect();
                    vec![moves::take(Take::Leaf { test: Cond::Word { at: 0, ws: words }, kind: self.anon_table(Some(n), &ws)?, field: "0".into() })]
                }
            },
            Gram::Sym(n) if self.rules.contains_key(n.as_str()) => vec![Op::Call(rule_fn(n))],
            Gram::Sym(n) => return Err(format!("rule {rule}: `{n}`, no rule, mark or token")),
            Gram::Seq(gs) => {
                let mut out = Block::new();
                for (i, g) in gs.iter().enumerate() {
                    let mut r: Vec<&Gram> = gs[i + 1..].iter().collect();
                    r.extend_from_slice(rest);
                    out.extend(self.code(g, &r, rule)?);
                }
                out
            }
            Gram::Opt(a) => {
                let arms = self.arms(a, rest, rule)?;
                let ex = self.exits_ending(rest, rule)?;
                let arms: Vec<(Vec<First>, Block)> = arms.into_iter().map(|(f, b, _)| (f, b)).collect();
                // An optional part that must take a token, tested by words and kinds alone, before a token of one word
                // it cannot begin with: the part, unless that token follows, as the lean parser's `list` asks
                // `is_p(close)` and simdjson `*peek() == ']'`. The part refuses a token it does not begin with. Not a
                // part of one test (`['sep' …]`), which asks as much as the token after it, as the lean parser's.
                let follow = match rest.first().and_then(|r| self.token_of(r)) {
                    Some((Test::Word(w), _))
                        if ex.is_empty()
                            && arms.len() == 1
                            && !self.first(a)?.1
                            && (self.leads_with_call(a) || arms[0].0.len() > 1)
                            && self.plain_tests(&arms[0].0)
                            && !arms[0].0.iter().any(|(t, _)| matches!(t, Test::Word(x) if *x == w)) =>
                    {
                        Some(w)
                    }
                    _ => None,
                };
                if let Some(w) = follow {
                    return Ok(vec![Op::If { arms: vec![(Cond::text(format!("self.word() != w::{}", self.word_const(w))), arms[0].1.clone())], els: None }]);
                }
                let unmarked = self.after_mark(rest, rule)?;
                let mut out = self.asking(&arms, &ex);
                out.extend(self.decide(&arms, &ex, &unmarked, &Block::new(), &Block::new()));
                out
            }
            Gram::Star(a) | Gram::Plus(a) => {
                let mut out = Block::new();
                if let Gram::Plus(_) = g {
                    let r: Vec<&Gram> = std::iter::once(g).chain(rest.iter().copied()).collect();
                    out.extend(self.code(a, &r, rule)?);
                }
                let r: Vec<&Gram> = std::iter::once(g).chain(rest.iter().copied()).collect();
                let arms = self.arms(a, &r, rule)?;
                let ex = self.exits_ending(rest, rule)?;
                let arms: Vec<(Vec<First>, Block)> = arms.into_iter().map(|(f, b, _)| (f, b)).collect();
                let unmarked = self.after_mark(rest, rule)?;
                let mut d = self.asking(&arms, &ex);
                // A loop only its exit ends (the code after it begins with the exit's marks), of a part that must take a
                // token: the part until the exit, as the lean parser's `while !end { statement }`. A token neither
                // begins refuses in the part, as it did after the loop.
                let (fr, may_end) = self.rest_first(rest)?;
                if arms.len() == 1
                    && d.is_empty()
                    && unmarked.is_empty()
                    && !ex.is_empty()
                    && !may_end
                    && !fr.is_empty()
                    && fr.iter().all(|(t, f)| f.is_none() && matches!(t, Test::Mark(m) if ex.iter().any(|e| e.0 == *m && !e.1)))
                    && !self.first(a)?.1
                {
                    let exit = Cond::Or(ex.iter().map(|e| self.mark_test(e.0)).collect());
                    d.push(Op::If { arms: vec![(exit, vec![Op::Break])], els: None });
                    d.extend(arms[0].1.clone());
                    out.push(Op::Loop(d));
                    return Ok(out);
                }
                d.extend(self.decide(&arms, &ex, &unmarked, &vec![Op::Break], &vec![Op::Break]));
                out.push(Op::Loop(d));
                out
            }
            // A choice of words, each a leaf of its own kind or of one it is taken as (`'w' as K`): one take, as the lean
            // parser's `anon()`.
            Gram::Alt(gs) if let Some(ws) = self.anon_words(gs) => {
                let words: Vec<String> = ws.iter().map(|(w, _)| format!("w::{}", self.word_const(*w))).collect();
                vec![moves::take(Take::Leaf { test: Cond::Word { at: 0, ws: words }, kind: self.anon_table(None, &ws)?, field: "0".into() })]
            }
            Gram::Alt(_) => {
                let mut arms = self.arms(g, rest, rule)?;
                let empty = arms.iter().position(|a| a.2);
                // A choice that must take a token, whose last alternative is a call of a level that tests its own
                // first token: that call is the default, as the lean parser's `else { self.term()?; }`, and it refuses
                // a token it does not begin with. No test leads to it. Its words and kinds are asked after the other
                // arms' (whose words it does not have); a mark of its, asked before them, only where none of theirs
                // can hold at its token, as the lean parser's `term` asks its own peeks (decision 0685 rock 4).
                let mut alts = Vec::new();
                Self::alts(g, &mut alts);
                // A rule's call, a level's or not, whose code asks its own first token, its marks too: its arm asked by
                // the tokens its tests may hold at alone, where no other arm's test may hold at one of them, as the
                // lean parser's `primary` switches on the kind and asks a word's peeks in the word's arm.
                let rule_call = |a: &Gram| matches!(a, Gram::Sym(n) if self.mark_of(n).is_none() && self.token_of(a).is_none() && self.rules.contains_key(n.as_str()) && self.rule_first(n).is_ok_and(|f| !f.1));
                if alts.len() == arms.len() && empty.is_none() {
                    let plain: Vec<Option<Vec<First>>> = alts
                        .iter()
                        .zip(&arms)
                        .map(|(a, arm)| {
                            if !rule_call(a) || arm.0.iter().all(|(t, f)| f.is_none() && !matches!(t, Test::Mark(_))) {
                                return None;
                            }
                            let mut out: Vec<First> = Vec::new();
                            for (t, _) in &arm.0 {
                                let ts = match t {
                                    Test::Mark(m) => self.mark_tests(*m)?,
                                    t => vec![t.clone()],
                                };
                                for t in ts {
                                    if !out.iter().any(|x| x.0 == t) {
                                        out.push((t, None));
                                    }
                                }
                            }
                            Some(out)
                        })
                        .collect();
                    for i in 0..arms.len() {
                        let Some(p) = &plain[i] else { continue };
                        let apart = p.iter().all(|(t, _)| {
                            (0..arms.len()).filter(|&j| j != i).all(|j| plain[j].as_ref().unwrap_or(&arms[j].0).iter().all(|(o, _)| self.apart(&self.first_holds(t), &self.first_holds(o))))
                        });
                        if apart {
                            arms[i].0 = p.clone();
                        }
                    }
                }
                // A word of its was asked before the other arms' kinds: none of their kinds' tokens is one (a soft word
                // the lexer gives another kind too is). A test under a flag is asked where it is.
                let others: Vec<(&Test, Holds)> = arms[..arms.len().saturating_sub(1)].iter().flat_map(|a| a.0.iter().map(|(t, _)| (t, self.first_holds(t)))).collect();
                let marks_apart = |fs: &[First]| {
                    // A test another arm has is that arm's (`decide` gives it to the first).
                    fs.iter().filter(|(t, _)| !others.iter().any(|(o, _)| *o == t)).all(|(t, _)| match t {
                        // A mark that holds only at one kind's tokens, which the arm tests: that kind's test.
                        Test::Mark(m) if self.implied_kind(*m).is_some_and(|k| fs.iter().any(|x| matches!(&x.0, Test::Kind(j) if *j == k))) => true,
                        // Marks are asked in their order: another arm's asked first (or the same, its) took its token
                        // before as now.
                        Test::Mark(m) => others.iter().all(|(o, h)| matches!(o, Test::Mark(n) if n <= m) || self.apart(&self.first_holds(t), h)),
                        Test::Word(_) => others.iter().filter(|o| matches!(o.0, Test::Kind(_))).all(|(_, h)| self.apart(&self.first_holds(t), h)),
                        Test::Kind(_) => true,
                    })
                };
                // A rule's code, a level's or not, asks its own first token (`rule_call`): a rule that must take one
                // refuses where none of its tests holds.
                let default = empty.is_none()
                    && alts.last().is_some_and(|a| self.leads_with_call(a) || rule_call(a))
                    && arms.last().is_some_and(|a| a.0.iter().all(|f| f.1.is_none()) && marks_apart(&a.0));
                let none = match empty {
                    Some(i) => arms[i].1.clone(),
                    None if default => {
                        let last = arms.last_mut().expect("an alternative");
                        last.0.clear();
                        last.1.clone()
                    }
                    None => vec![Op::Refuse],
                };
                let ex = if empty.is_some() { self.exits_ending(rest, rule)? } else { Vec::new() };
                let arms: Vec<(Vec<First>, Block)> = arms.into_iter().map(|(f, b, _)| (f, b)).collect();
                let unmarked = if empty.is_some() { self.after_mark(rest, rule)? } else { Vec::new() };
                let mut out = self.asking(&arms, &ex);
                out.extend(self.decide(&arms, &ex, &unmarked, &none, &none));
                out
            }
            Gram::Field(f, a) => {
                let mut out = self.code(a, rest, rule)?;
                out.push(Op::Field(self.field_const(f)?));
                out
            }
            Gram::Retag(k, a) => {
                let mut out = vec![line(format!("self.retag({});", self.kind_const(k)?))];
                out.extend(self.code(a, rest, rule)?);
                out
            }
            Gram::Wrap { kind, field, moved, body } => {
                let f = match field {
                    Some(f) if f != "_" => self.field_const(f)?,
                    _ => "0".to_owned(),
                };
                let mut out = vec![line(format!("self.wrap({}, {f})?;", self.kind_const(kind)?))];
                if let Some(m) = moved {
                    out.push(line(format!("self.retype_moved({});", self.moved_kind(m)?)));
                }
                out.extend(self.code(body, rest, rule)?);
                out.push(line("self.close()?;"));
                out
            }
            // The node made last at this level relabelled, as the machine's `NODE_FIELD` and `RETAG` on it (modes 2 and
            // 3): the field of the node its wrap holds first, then its own field, then its kind; `drop` (kind `GONE`) takes
            // the node out, the nodes inside it in its place.
            Gram::Relabel { field, moved, inner, body } => {
                let mut out = Block::new();
                if let Some(g) = inner.as_deref().filter(|g| *g != "_") {
                    out.push(line(format!("self.relabel_inner({})?;", self.field_const(g)?)));
                }
                if let Some(f) = field.as_deref().filter(|f| *f != "_") {
                    out.push(line(format!("self.relabel_field({})?;", self.field_const(f)?)));
                }
                match moved.as_deref() {
                    Some(::bitlex::syntax::GONE) => out.push(line("self.drop_made()?;")),
                    Some(m) => out.push(line(format!("self.relabel_kind({})?;", self.moved_kind(m)?))),
                    None => {}
                }
                out.extend(self.code(body, rest, rule)?);
                out
            }
            Gram::Merge(k, a) => {
                let (test, _) = self.token_of(a).ok_or_else(|| format!("rule {rule}: a merge of no token"))?;
                vec![moves::take(Take::Merge { test: self.token_cond(&test), kind: self.kind_const(k)? })]
            }
            Gram::Flag { flag, set, body } => {
                let f = format!("FLAG_{}", upper(flag));
                let op = if *set { format!("self.flags | {f}") } else { format!("self.flags & !{f}") };
                let mut out = vec![line(format!("let flags = self.flags;\nself.flags = {op};"))];
                out.extend(self.code(body, rest, rule)?);
                out.push(line("self.flags = flags;"));
                out
            }
            Gram::Unless { flag, body } => vec![Op::If { arms: vec![(Cond::Flag(format!("FLAG_{}", upper(flag))), self.code(body, rest, rule)?)], els: None }],
            Gram::Reread { reading, body } => {
                if !self.readings.contains_key(reading.as_str()) {
                    return Err(format!("rule {rule}: no reading `{reading}`"));
                }
                let mut out = vec![line(format!("self.reread_{}()?;", ident(reading)))];
                out.extend(self.code(body, rest, rule)?);
                out
            }
            other => return Err(format!("rule {rule}: not written yet: {other:?}")),
        })
    }

    /// A level's call before `rest` in `rule`: the questions it asks where it may end at a mark, set where its caller
    /// takes the mark and kept where the caller ends there too; and every mark's that has questions (none: no call
    /// saves them).
    fn call_asks(&self, rest: &[&Gram], rule: &str) -> Result<(u64, u64, u64), String> {
        let ex = self.exits_ending(rest, rule)?;
        let (mut set, mut keep) = (0u64, 0u64);
        for (m, mark) in self.x.marks.iter().enumerate() {
            if !self.asks(m) {
                continue;
            }
            let first = self.mark_of(&mark.name).unwrap_or(m);
            match ex.iter().find(|e| e.0 == first) {
                Some((_, false)) => set |= 1 << first,
                Some((_, true)) => keep |= 1 << first,
                None => {}
            }
        }
        let all: u64 = (0..self.x.marks.len()).filter(|&m| self.asks(m)).fold(0, |a, m| a | 1 << self.mark_of(&self.x.marks[m].name).unwrap_or(m));
        Ok((set, keep, all))
    }

    /// A kind a wrap or relabel gives its node: a `remember` choice's name is its level's word.
    fn moved_kind(&self, name: &str) -> Result<String, String> {
        if self.remember.iter().any(|r| r == name) { Ok("self.word".to_owned()) } else { self.kind_const(name) }
    }

    /// A choice of tokens `n` taken: one decision, an arm a token, each a leaf of kind `kind` (none: its own); in a
    /// `remember` choice each also its level's word, as the machine's `REMEMBER`.
    fn token_choice(&self, n: &str, kind: Option<&str>, rule: &str) -> Result<Block, String> {
        let Some(Stmt::Choice { body, .. }) = self.rules.get(n) else { return Err(format!("rule {rule}: `{n}`, no choice of tokens")) };
        let mut alts = Vec::new();
        Self::alts(body, &mut alts);
        let remembered = self.remember.iter().any(|r| r == n);
        // Taken as one kind (`aggregate_word as variable`): every way's leaf is that kind, so one take of any of its
        // tokens, as the lean parser's `anon()`.
        if let (Some(k), false) = (kind, remembered) {
            let tests: Option<Vec<Test>> = alts.iter().map(|a| self.token_of(if let Gram::As(_, t) = a { t } else { a }).map(|t| t.0)).collect();
            if let Some(tests) = tests {
                let words: Option<Vec<String>> = tests.iter().map(|t| if let Test::Word(w) = t { Some(format!("w::{}", self.word_const(*w))) } else { None }).collect();
                let cond = match words {
                    Some(ws) if ws.len() > 1 => Cond::Word { at: 0, ws },
                    _ if tests.len() == 1 => self.token_cond(&tests[0]),
                    _ => Cond::Or(tests.iter().map(|t| self.token_cond(t)).collect()),
                };
                return Ok(vec![moves::take(Take::Leaf { test: cond, kind: self.kind_const(k)?, field: "0".into() })]);
            }
        }
        let mut arms = Vec::new();
        for a in alts {
            // A token taken as a kind (`'product' as identifier`): its leaf that kind, and the word it remembers.
            let (t, written) = match a {
                Gram::As(k, t) => (&**t, Some(k.as_str())),
                _ => (a, None),
            };
            let (test, own) = self.token_of(t).ok_or_else(|| format!("rule {rule}: choice `{n}` holds more than tokens"))?;
            let own = written.unwrap_or(&own);
            let mut body = self.take_token(&test, Some(kind.unwrap_or(own)))?;
            if remembered {
                body.push(line(format!("self.word = {};", self.kind_const(own)?)));
            }
            arms.push((self.first(a)?.0, body));
        }
        let none = vec![Op::Refuse];
        let mut out = self.asking(&arms, &[]);
        out.extend(self.decide(&arms, &[], &[], &none, &none));
        Ok(out)
    }

    /// A choice of tokens alone, no level, each a word whose leaf is its own kind (`additive = '+' | '-'`): its words and
    /// their tokens. Taken where it is used, by one take whose leaf's kind is the word's (`ANON`), as the lean parser's
    /// `anon()`.
    fn anon_choice(&self, n: &str) -> Option<(Vec<(u8, String)>, Option<(String, String)>)> {
        if self.remember.iter().any(|r| r == n) || self.order.first() == Some(&n) {
            return None;
        }
        let Some(Stmt::Choice { body, nested: false, .. }) = self.rules.get(n) else { return None };
        let mut alts = Vec::new();
        Self::alts(body, &mut alts);
        // Its words, each its own leaf; and at most one token of a kind, a leaf of the kind it is taken as (`K as X`), as
        // the lean parser's `anon()` takes any other token as an identifier.
        let (mut words, mut other) = (Vec::new(), None);
        for a in alts {
            match a {
                Gram::Lit(t) if (self.x.kind)(t).is_some() => words.push((self.word_of(t)?, t.clone())),
                Gram::As(x, k) if other.is_none() && matches!(self.token_of(k), Some((Test::Kind(_), _))) => {
                    let Some((Test::Kind(k), _)) = self.token_of(k) else { return None };
                    other = Some((k, x.clone()));
                }
                _ => return None,
            }
        }
        (!words.is_empty()).then_some((words, other))
    }

    /// A choice written in place whose ways are each a word, a leaf of its own kind or of the one it is taken as: the words
    /// and their leaves' kinds, where there are two or more.
    fn anon_words(&self, gs: &[Gram]) -> Option<Vec<(u8, String)>> {
        let ws: Vec<(u8, String)> = gs
            .iter()
            .map(|g| match g {
                Gram::Lit(t) if (self.x.kind)(t).is_some() => Some((self.word_of(t)?, t.clone())),
                Gram::As(x, l) if x != ::bitlex::syntax::IMPLIED && matches!(**l, Gram::Lit(_)) => match &**l {
                    Gram::Lit(t) => Some((self.word_of(t)?, x.clone())),
                    _ => None,
                },
                _ => None,
            })
            .collect::<Option<_>>()?;
        (ws.len() > 1).then_some(ws)
    }

    /// A token element: its test and its leaf's kind.
    fn token_of(&self, g: &Gram) -> Option<(Test, String)> {
        match g {
            Gram::Lit(t) => match self.word_of(t) {
                Some(w) => Some((Test::Word(w), t.clone())),
                None if self.is_token_kind(t) => Some((Test::Kind(t.clone()), t.clone())),
                None => None,
            },
            Gram::Sym(n) if self.mark_of(n).is_none() && !self.rules.contains_key(n.as_str()) && self.is_token_kind(n) => Some((Test::Kind(n.clone()), n.clone())),
            Gram::Sym(n) if self.mark_of(n).is_none() && !self.rules.contains_key(n.as_str()) => self.keyword_word(n).or_else(|| self.word_of(n)).map(|w| (Test::Word(w), n.clone())),
            _ => None,
        }
    }

    /// The token `t` taken: a leaf of `kind`, or none (a token the grammar implies), as the
    /// lean parser's `expect`: by its word, else by its kind's test.
    fn take_token(&self, t: &Test, kind: Option<&str>) -> Result<Block, String> {
        let take = match (t, kind) {
            (Test::Word(w), Some(k)) => Take::WordLeaf { word: format!("w::{}", self.word_const(*w)), kind: self.kind_const(k)? },
            (Test::Word(w), None) => Take::WordSkip { word: format!("w::{}", self.word_const(*w)) },
            (_, Some(k)) => Take::Leaf { test: self.token_cond(t), kind: self.kind_const(k)?, field: "0".into() },
            (_, None) => Take::Skip { test: self.token_cond(t) },
        };
        Ok(vec![moves::take(take)])
    }

    fn token_cond(&self, t: &Test) -> Cond {
        self.cond(&(t.clone(), None))
    }

    /// Where rule `name` ends, after its body and before its node's close: its endings (`ending`), each a test of the
    /// kind of the node made last at its level, as the lean parser's `literal` asks its term's after it (`retag_atom`).
    fn ending_code(&self, name: &str) -> Result<Block, String> {
        let mut out = Block::new();
        // The kinds it refuses, one test of them all, as the lean parser's `if !call { return Err(()) }`.
        let refused: Vec<&str> = self.endings.iter().filter(|e| e.0 == name && matches!(e.2, ::bitlex::syntax::Ending::Refused)).map(|e| e.1).collect();
        let refuse = |c: String| Op::If { arms: vec![(Cond::text(c), vec![Op::Refuse])], els: None };
        match refused.as_slice() {
            [] => {}
            [k] => out.push(refuse(format!("self.made_is({})", self.kind_const(k)?))),
            ks => {
                let ks = ks.iter().map(|k| self.kind_const(k)).collect::<Result<Vec<_>, _>>()?.join(" | ");
                out.push(refuse(format!("matches!(self.made_kind(), {ks})")));
            }
        }
        for &(_, kind, ending) in self.endings.iter().filter(|e| e.0 == name && !matches!(e.2, ::bitlex::syntax::Ending::Refused)) {
            let mut body = Block::new();
            match ending {
                ::bitlex::syntax::Ending::Refused => unreachable!("written above"),
                ::bitlex::syntax::Ending::Relabel { moved, inner } => {
                    if let Some(g) = inner {
                        body.push(line(format!("self.relabel_inner({})?;", self.field_const(g)?)));
                    }
                    body.push(line("self.relabel_field(0)?;"));
                    body.push(line(format!("self.relabel_kind({})?;", self.moved_kind(moved)?)));
                }
            }
            out.push(Op::If { arms: vec![(Cond::text(format!("self.made_is({})", self.kind_const(kind)?)), body)], els: None });
        }
        Ok(out)
    }

    /// Rule `name`'s method, as moves: its code, and the method's lines around it. Printed once every rule's moves are
    /// made (`printed`), as the rewrites read what all of them hold (`marked_read`).
    fn rule(&self, name: &str, nf: bool) -> Result<Block, String> {
        // `nf`: a rule that runs with no mark refused, its takes without the refused marks' test (`records::refusers`).
        let mut blk = Block::new();
        let mut code = |out: &mut String, mut b: Block| {
            if nf {
                moves::no_refusing(&mut b);
            }
            blk.push(Op::Line(std::mem::take(out)));
            blk.push(Op::At("        ".into(), b));
        };
        // Taken where it is used (`anon_choice`): no function.
        if self.anon_choice(name).is_some() {
            return Ok(Block::new());
        }
        let (body, node, nested, whole, padded) = match self.rules[name] {
            Stmt::Node { body, nested, whole, padded, .. } => (body, true, *nested, *whole, *padded),
            Stmt::Choice { body, nested, .. } => (body, false, *nested, false, false),
            _ => unreachable!(),
        };
        let root = self.order.first() == Some(&name);
        let mut out = format!("    /// `{name}`.\n    fn {}(&mut self) -> R {{\n", rule_fn(name));
        if nested && self.counts_depth(name) {
            out.push_str("        self.deeper()?;\n");
        }
        // A level that may set the level's word (`remember`) gives its caller's back where it ends, as the machine's
        // `PUSH` keeps it: one save, here, not one at each call.
        let keeps_word = nested && !self.remember.is_empty() && self.call_saves.borrow().as_ref().is_some_and(|(_, w)| w.contains(name));
        if keeps_word {
            out.push_str("        let word = self.word;\n");
        }
        if let Some(one) = self.statements(name, body)? {
            let _ = writeln!(out, "{}    }}\n", self.statement_loop(name, "self.statement()", "")?);
            let _ = write!(out, "    /// A statement of `{name}`: one of its repetition, as the lean parser's `statement`.\n    fn statement(&mut self) -> R {{\n");
            code(&mut out, self.code(one, &[], name)?);
            out.push_str("        Ok(())\n");
        } else if root {
            let _ = writeln!(out, "        self.open_root({});", self.kind_const(name)?);
            code(&mut out, self.code(body, &[], name)?);
            let _ = writeln!(out, "        self.close_root({whole}, {padded})");
        } else {
            if node {
                // A kind of the grammar's own (no tape kind, always retagged inside): a placeholder until its retag.
                let id = (self.x.kind)(name).ok_or_else(|| format!("no kind `{name}`"))?;
                if id > 0x3ff {
                    out.push_str("        self.open(UNSET);\n");
                } else {
                    let _ = writeln!(out, "        self.open({});", self.kind_const(name)?);
                }
            }
            let mut c = self.code(body, &[], name)?;
            c.extend(self.ending_code(name)?);
            // Its code where its one caller calls it (`inlined`): the node opened and closed there, as the lean
            // parser's `statement` opens `Include` in place. Not a level's that counts the depth or keeps the caller's word:
            // that state is its method's. (A call that saves the caller's marks asked is no plain call: none is written in.)
            if !(nested && (self.counts_depth(name) || keeps_word)) {
                let open = if node { Some(if (self.x.kind)(name).is_some_and(|id| id > 0x3ff) { "UNSET".to_owned() } else { self.kind_const(name)? }) } else { None };
                let mut inl: Block = open.iter().map(|k| Op::Open(k.clone())).collect();
                inl.extend(c.iter().cloned());
                if node {
                    inl.push(Op::Close);
                }
                inl.push(line("self.marked = false;"));
                if nf {
                    moves::no_refusing(&mut inl);
                }
                self.inline.borrow_mut().insert(rule_fn(name), inl);
            }
            code(&mut out, c);
            if node {
                out.push_str("        self.close()?;\n");
            }
            if keeps_word {
                out.push_str("        self.word = word;\n");
            }
            if nested && self.counts_depth(name) {
                out.push_str("        self.depth -= 1;\n");
            }
            // `marked` cleared at the rule's end: its own move, gone where no decision reads it (`moves::rewrite`).
            blk.push(Op::Line(std::mem::take(&mut out)));
            blk.push(line("        self.marked = false;\n"));
            out.push_str("        Ok(())\n");
        }
        out.push_str("    }\n\n");
        blk.push(Op::Line(out));
        Ok(blk)
    }

    /// The root `name`'s one statement where it recovers (`recover`, decision 0650): its body is one repetition, the
    /// statements, as the lean parser's file.
    fn statements<'g>(&self, name: &str, body: &'g Gram) -> Result<Option<&'g Gram>, String> {
        if self.order.first() != Some(&name) || !self.recover.is_some_and(|r| r.0 == name) {
            return Ok(None);
        }
        match body {
            Gram::Star(a) => Ok(Some(&**a)),
            Gram::Seq(v) if v.len() == 1 => self.statements(name, &v[0]),
            _ => Err(format!("recover {name}: its body is one repetition, the statements")),
        }
    }

    /// The root `name`'s statement loop (decision 0650), as the lean parser's: a mark before each statement (`call`);
    /// one that fails is cut back to it (`cut`, then `recover`) and made one
    /// ERROR leaf. Then the root's close.
    fn statement_loop(&self, name: &str, call: &str, cut: &str) -> Result<String, String> {
        let (whole, padded) = match self.rules[name] {
            Stmt::Node { whole, padded, .. } => (*whole, *padded),
            _ => return Err(format!("recover {name}: no node")),
        };
        Ok(format!(
            "        self.open_root({});\n        while self.i < self.n {{\n            let save = self.mark();\n            if {call}.is_err() {{\n{cut}                self.recover(save)?;\n            }}\n        }}\n        self.close_root({whole}, {padded})\n",
            self.kind_const(name)?
        ))
    }

    /// `recover`'s runtime (decision 0650), added to `out`: the statement's `Mark` (what a statement changes of the
    /// parser's state, as the lean parser's), `mark` and `reset`, `recover` (the ERROR leaf to the resume point), and `limited` (a limit refuses the file).
    fn recovery(&self, mut out: String, clauses: &[::bitlex::syntax::RecoverClause]) -> Result<String, String> {
        if !crate::bitlex_lex_rust::readers(self.x.lexer).is_empty() || out.contains("    lost: Option<") && out.contains("self.lose_pending()") {
            return Err("tier 2 as Rust: `recover` with regions read in parts or extras lost: not yet".into());
        }
        for (a, b) in [
            ("    depth: u32,\n", "    depth: u32,\n    /// A limit refused the file (the depth), not its syntax: no statement recovers (`recover`).\n    limited: bool,\n"),
            ("depth: 0,", "depth: 0, limited: false,"),
            ("        if self.depth > DEPTH {\n            return self.refuse();", "        if self.depth > DEPTH {\n            self.limited = true;\n            return self.refuse();"),
        ] {
            if out.matches(a).count() != 1 {
                return Err(format!("tier 2 as Rust: `recover`'s place in the runtime: {a:?}"));
            }
            out = out.replace(a, b);
        }
        // The level state a statement may leave changed, where the parser has it.
        let has = |f: &str| out.contains(&format!("    {f}: "));
        let mut fields = vec![("i", "usize"), ("pend", "usize"), ("xi", "usize"), ("next_x", "u32"), ("w", "usize"), ("levels", "usize"), ("top", "(u32, u32)"), ("last_end", "u32"), ("flags", "u32"), ("depth", "u32"), ("marked", "bool")];
        fields.retain(|(f, _)| *f != "marked" || has(f));
        for (f, t) in [("word", "u16"), ("asks", "u64"), ("refusing", "u32"), ("strings", "usize")] {
            if has(f) {
                fields.push((f, t));
            }
        }
        let mut code = String::from("\n/// Where a statement starts (the lean parser's `Mark`): its token, the comments, the tape's length, the levels and the\n/// level's state, which a statement that fails is cut back to.\n#[derive(Clone, Copy)]\nstruct Mark {\n");
        for (f, t) in &fields {
            let _ = writeln!(code, "    {f}: {t},");
        }
        code.push_str("}\n\n");
        let terminators: Vec<String> = self.x.terminators.iter().map(|b| format!("{b:#04x}")).collect();
        let _ = writeln!(code, "/// The last bytes of the language's line terminators (`recover`'s `first`).\nconst TERMINATORS: &[u8] = &[{}];\n", terminators.join(", "));
        code.push_str("#[allow(clippy::all, dead_code)]\nimpl<L: Layout> P<'_, L> {\n    /// The statement's mark (the lean parser's `mark`).\n    fn mark(&self) -> Mark {\n        Mark { ");
        let saved: Vec<String> = fields.iter().map(|(f, _)| match *f {
            "w" => "w: self.w.len()".to_owned(),
            "levels" => "levels: self.levels.len()".to_owned(),
            f => format!("{f}: self.{f}"),
        }).collect();
        code.push_str(&saved.join(", "));
        code.push_str(" }\n    }\n\n    /// Back at mark `m` (the lean parser's `reset`): its token and comments, the tape and the levels as they were.\n    fn reset(&mut self, m: Mark) {\n");
        for (f, _) in &fields {
            let _ = match *f {
                "w" => writeln!(code, "        self.w.truncate(m.w);"),
                "levels" => writeln!(code, "        self.levels.truncate(m.levels);"),
                f => writeln!(code, "        self.{f} = m.{f};"),
            };
        }
        if has("dead") {
            code.push_str("        self.dead.retain(|&o| o != NONE && (o as usize) < m.w);\n");
        }
        code.push_str("        self.taken = 0;\n        let _ = self.arrive();\n    }\n\n");
        // The resume point's tests at record `k`: each clause's tokens from `k` on, its first first on its line.
        let mut at = Vec::new();
        let mut past = Vec::new();
        for c in clauses {
            let mut test = Vec::new();
            if c.first {
                test.push("self.first_on_line(k)".to_owned());
            }
            for (j, e) in c.seq.iter().enumerate() {
                let place = if j == 0 { "k".to_owned() } else { format!("k + {j}") };
                test.push(self.elem(e, &place)?);
            }
            if c.past { past.push(test.join(" && ")) } else { at.push(test.join(" && ")) }
        }
        let or = |v: &[String]| if v.is_empty() { "false".to_owned() } else { v.join(" || ") };
        let _ = write!(code, "    /// A statement that failed, from mark `m` (the lean parser's `recover`): cut back to it, one ERROR leaf from its\n    /// first token to the resume point, the comments inside it its text; then on from there. A limit refuses the file.\n    fn recover(&mut self, m: Mark) -> R {{\n        if self.limited {{\n            return self.refuse();\n        }}\n        let first = m.i;\n        self.reset(m);\n        let mut k = first + 1;\n        while k < self.n {{\n            if {} {{\n                break;\n            }}\n            if {} {{\n                k += 1;\n                break;\n            }}\n            k += 1;\n        }}\n        let k = k.min(self.n);\n        self.flush();\n        let end = self.recs[k - 1].last + 1;\n        let at = self.w.push_leaf(ERROR, self.recs[first].start, end);\n        self.set_made(at as u32);\n        while self.xi < self.xs.len() && self.xs[self.xi].start < end {{\n            self.xi += 1;\n        }}\n        self.pend = self.xi;\n        self.next_x = self.xs.get(self.xi).map_or(u32::MAX, |x| x.start);\n        self.i = k;\n        self.last_end = end;\n        self.pass_comments();\n        self.arrive()\n    }}\n\n", or(&at), or(&past));
        code.push_str("    /// Whether token `j` is first on its line (the lean parser's `line_start`: only blanks before it back to a line's\n    /// terminator or the file's start). Tier 1 leaves only blanks and line ends between records, so: no record before\n    /// it, or a terminator among the bytes since the record before (the token before, or a comment after it), read a\n    /// word at a time.\n    fn first_on_line(&self, j: usize) -> bool {\n        let at = self.recs[j].start as usize;\n        let c = self.xs.partition_point(|x| (x.start as usize) < at);\n        let before = [j.checked_sub(1).map(|i| self.recs[i].last), c.checked_sub(1).map(|i| self.xs[i].last)];\n        let Some(last) = before.into_iter().flatten().max() else { return true };\n        let mut i = last as usize + 1;\n        while i < at {\n            let n = (at - i).min(8);\n            let mut w = [0u8; 8];\n            w[..n].copy_from_slice(&self.src[i..i + n]);\n            let x = u64::from_le_bytes(w);\n            if TERMINATORS.iter().any(|&t| {\n                let y = x ^ 0x0101_0101_0101_0101 * u64::from(t);\n                y.wrapping_sub(0x0101_0101_0101_0101) & !y & 0x8080_8080_8080_8080 != 0\n            }) {\n                return true;\n            }\n            i += 8;\n        }\n        false\n    }\n}\n\n/// An ERROR leaf's key: the tape's error code (`rerail_tape_format::CODE_ERROR`), of no kind or field.\nconst ERROR: u32 = (rerail_tape_format::CODE_ERROR as u32) << 16;\n");
        out.push_str(&code);
        Ok(out)
    }

    /// A token element's test of the token at `j` (a token mark's).
    fn elem(&self, e: &[TokenElem], j: &str) -> Result<String, String> {
        let mut pos = Vec::new();
        let mut and = Vec::new();
        for x in e {
            match x {
                TokenElem::Kind(k) => pos.push(self.kind_test(k, &format!("self.kind_at({j})"), &format!("self.word_at({j})"))),
                TokenElem::Word(w) => match self.word_of(&String::from_utf8_lossy(w)) {
                    Some(id) => pos.push(format!("self.word_at({j}) == w::{}", self.word_const(id))),
                    None => pos.push("false".into()),
                },
                // A token there, not one of these: a word's test negated (`!=`), as the lean parser's `!self.is_p_at(k, P)`.
                TokenElem::Not(inner) => {
                    let t = self.elem(inner, j)?;
                    let not = match t.split_once(" == ") {
                        Some((a, b)) if !t.contains("||") && !t.contains("&&") => format!("{a} != {b}"),
                        _ if t.starts_with("matches!(") && !t.contains("||") && !t.contains("&&") => format!("!{t}"),
                        _ => format!("!({t})"),
                    };
                    pos.push(format!("({j} < self.n && {not})"));
                }
                TokenElem::End => pos.push(format!("{j} >= self.n")),
                TokenElem::Without(m) => and.push(format!("!self.bit_at({j}, {})", self.bit_of(m)?)),
                TokenElem::With(m) => and.push(format!("self.bit_at({j}, {})", self.bit_of(m)?)),
                TokenElem::Starting(bytes) => and.push(format!("({j} < self.n && matches!(self.src[self.recs[{j}].start as usize], {}))", bytes.iter().map(|b| format!("{b:#04x}")).collect::<Vec<_>>().join(" | "))),
                TokenElem::Many(..) => return Err("a run inside an element".into()),
            }
        }
        // Words alone at one token: one `matches!`, as the lean parser's `matches!(k - P0, EQ | NE | …)`.
        let word = format!("self.word_at({j}) == ");
        let mut c = match pos.len() {
            0 => "true".to_owned(),
            1 => pos.remove(0),
            _ if pos.iter().all(|p| p.starts_with(&word) && !p[word.len()..].contains(' ')) => {
                format!("matches!(self.word_at({j}), {})", pos.iter().map(|p| &p[word.len()..]).collect::<Vec<_>>().join(" | "))
            }
            _ => format!("({})", pos.join(" || ")),
        };
        for a in and {
            c = format!("{c} && {a}");
        }
        Ok(c)
    }

    fn bit_of(&self, m: &str) -> Result<String, String> {
        Ok(self.bit_num(m)?.to_string())
    }

    /// Mark bit `b` read of a record's code (`reads`).
    fn read_bit(&self, b: u32) {
        self.reads.set(self.reads.get() | 1 << b);
    }

    /// The words a run's first token must be, where its first element tests only words (`.` or `@`): else none.
    fn first_words(&self, seq: &[Vec<TokenElem>]) -> Option<Vec<String>> {
        let mut ws = Vec::new();
        for x in seq.first()? {
            match x {
                TokenElem::Word(w) => ws.push(format!("w::{}", self.word_const(self.word_of(&String::from_utf8_lossy(w))?))),
                // A further test of the token only narrows the words.
                TokenElem::Without(_) | TokenElem::With(_) | TokenElem::Starting(_) => {}
                _ => return None,
            }
        }
        (!ws.is_empty()).then_some(ws)
    }

    /// The token before the token a run starts at, as its first function's `prev`: found
    /// only where the run's first element reads it, a `(C)*` with an ending; else none (`self.n`), as no element reads it.
    /// A run that scans a bracket: tokens at known places, then an opening token, the tokens between it and its closing
    /// one (`(C)* nest (O to C)`), the closing one, and tokens at known places after it. The opening token's place in the
    /// run; one scan finds where the bracket closes (`scan_fn`), as the lean parser scans to the `)` once and asks what
    /// follows.
    fn bracket(seq: &[Vec<TokenElem>]) -> Option<usize> {
        let k = seq.iter().position(|e| matches!(e.as_slice(), [TokenElem::Many(..)]))?;
        let [TokenElem::Many(_, nests, ending)] = seq[k].as_slice() else { return None };
        let fixed = |es: &[Vec<TokenElem>]| Self::fixed(es);
        // The closing token: the nest's, and it may ask a mark of it (`"}" without line`).
        let closes = |n: &::bitlex::syntax::Nest| n.close == seq[k + 1] || (seq[k + 1].starts_with(&n.close) && seq[k + 1][n.close.len()..].iter().all(|e| matches!(e, TokenElem::Without(_) | TokenElem::With(_))));
        (k >= 1 && ending.is_empty() && k + 1 < seq.len() && nests.iter().any(|n| n.open == seq[k - 1] && closes(n)) && fixed(&seq[..k]) && fixed(&seq[k + 2..])).then_some(k - 1)
    }

    /// The scan of a bracket run (`bracket`) whose opening token is at `h`: its function's name, and its code. One
    /// function a bracket and the tokens it may hold, however many marks ask it.
    fn scan_fn(&self, seq: &[Vec<TokenElem>], h: usize) -> Result<(String, String), String> {
        let [TokenElem::Many(choices, nests, _)] = seq[h + 1].as_slice() else { return Err("a scan of no repetition".into()) };
        let key = format!("{:?}", &seq[h..h + 3]);
        let mut scans = self.scans.borrow_mut();
        let base = match seq[h].as_slice() {
            [TokenElem::Word(w)] => self.word_of(&String::from_utf8_lossy(w)).map(|id| self.word_const(id).trim_end_matches(|c: char| c.is_ascii_digit() || c == '_').to_ascii_lowercase()),
            _ => None,
        }
        .unwrap_or_else(|| "bracket".to_owned());
        let name = match scans.iter().find(|x| x.0 == key) {
            Some(x) => x.1.clone(),
            None => {
                let same = scans.iter().filter(|x| x.1.starts_with(&format!("past_{base}"))).count();
                let n = if same == 0 { format!("past_{base}") } else { format!("past_{base}_{same}") };
                scans.push((key, n.clone()));
                n
            }
        };
        let mut b = String::new();
        let _ = writeln!(b, "    /// Where the bracket that opens at token `j` closes: the token after its close; none where no bracket opens there");
        let _ = writeln!(b, "    /// or a token it may not hold comes first. As the lean parser scans to the `)`, its pairs counted.");
        let _ = writeln!(b, "    fn {name}(&self, j: usize) -> Option<usize> {{");
        let _ = writeln!(b, "        if !({}) {{
            return None;
        }}", self.elem(&seq[h], "j")?);
        let _ = writeln!(b, "        let mut depth = [0u32; {}];
        let mut j = j + 1;
        loop {{", nests.len().max(1));
        let _ = writeln!(b, "            if depth.iter().all(|&d| d == 0) && {} {{
                return Some(j + 1);
            }}", self.elem(&seq[h + 2], "j")?);
        let _ = writeln!(b, "            if j >= self.n || !({}) {{
                return None;
            }}", self.elem(choices, "j")?);
        for (d, n) in nests.iter().enumerate() {
            let _ = writeln!(b, "            if {} {{
                depth[{d}] += 1;
            }} else if {} {{
                if depth[{d}] == 0 {{
                    return None;
                }}
                depth[{d}] -= 1;
            }}", moves::bare(&self.elem(&n.open, "j")?), moves::bare(&self.elem(&n.close, "j")?));
        }
        b.push_str("            j += 1;
        }
    }

");
        Ok((name, b))
    }

    /// Whether a run's tokens are each one token at a place it knows (no repetition, no mark asked of a token): a peek
    /// tests it inline.
    fn fixed(seq: &[Vec<TokenElem>]) -> bool {
        seq.iter().all(|e| e.iter().all(|x| !matches!(x, TokenElem::Many(..) | TokenElem::With(_) | TokenElem::Without(_) | TokenElem::Starting(_))))
    }

    fn run_prev(seq: &[Vec<TokenElem>]) -> &'static str {
        match seq.first().map(Vec::as_slice) {
            Some([TokenElem::Many(_, _, ending)]) if !ending.is_empty() => "self.prev_tok(self.i)",
            _ => "self.n",
        }
    }

    /// A run of token elements from `j` (the token before: `prev`), as a function's body: whether
    /// they hold.
    fn run(&self, seq: &[Vec<TokenElem>], name: &str, fns: &mut String) -> Result<(), String> {
        for (k, e) in seq.iter().enumerate() {
            let next = if k + 1 == seq.len() { "true".to_owned() } else { format!("self.{name}_{}(j + 1, j)", k + 1) };
            let body = if let [TokenElem::Many(choices, nests, ending)] = e.as_slice() {
                // `(C)*`: the elements after it tried at each token from here, where every depth is
                // 0 (and, with `ending`, after one of its tokens); else one more of the run's.
                let after = if k + 1 == seq.len() { "true".to_owned() } else { format!("self.{name}_{}(j, prev)", k + 1) };
                let mut b = String::new();
                let _ = writeln!(b, "        let (mut j, mut prev) = (j, prev);\n        let mut depth = [0u32; {}];", nests.len().max(1));
                b.push_str("        loop {\n");
                let ok_end = if ending.is_empty() { "true".to_owned() } else { format!("prev < self.n && {}", self.elem(ending, "prev")?) };
                let _ = writeln!(b, "            if depth.iter().all(|&d| d == 0) && {ok_end} && {after} {{\n                return true;\n            }}");
                let c = self.elem(choices, "j")?;
                let _ = writeln!(b, "            if j >= self.n || !({c}) {{\n                return false;\n            }}");
                for (d, n) in nests.iter().enumerate() {
                    let _ = writeln!(b, "            if {} {{\n                depth[{d}] += 1;\n            }} else if {} {{\n                if depth[{d}] == 0 {{\n                    return false;\n                }}\n                depth[{d}] -= 1;\n            }}", moves::bare(&self.elem(&n.open, "j")?), moves::bare(&self.elem(&n.close, "j")?));
                }
                b.push_str("            prev = j;\n            j += 1;\n        }\n");
                b
            } else {
                format!("        let _ = prev;\n        {} && {next}\n", self.elem(e, "j")?)
            };
            let _ = write!(fns, "    #[allow(unused_variables)]\n    fn {name}_{k}(&self, j: usize, prev: usize) -> bool {{\n{body}    }}\n\n");
        }
        Ok(())
    }

    /// A gap question's element at a record (`GapPattern`, decision 0451): comment `j` where `x`, else token `j`;
    /// `none` what it gives where there is no record there.
    fn gap_elem(&self, e: &[TokenElem], j: &str) -> Result<(String, bool), String> {
        let (not, choices) = match e {
            [TokenElem::Not(inner)] => (true, inner.as_slice()),
            c => (false, c),
        };
        let mut pos = Vec::new();
        let mut and = Vec::new();
        for c in choices {
            match c {
                TokenElem::Kind(k) if self.x.extras.contains(k) => pos.push(format!("(x && extra_kind(self.xs[{j}].code) == {})", self.kind_const(k)?)),
                TokenElem::Kind(k) => {
                    pos.push(format!("(!x && {})", self.kind_test(k, &format!("(self.recs[{j}].code & 0xff)"), &format!("(self.recs[{j}].code >> 8 & 0xff)"))));
                }
                TokenElem::Word(w) => match self.word_of(&String::from_utf8_lossy(w)) {
                    Some(id) => pos.push(format!("(!x && self.recs[{j}].code >> 8 & 0xff == w::{})", self.word_const(id))),
                    None => pos.push("false".into()),
                },
                TokenElem::With(m) => and.push(format!("self.rec_marks_hold(x, {j}, 1 << {})", self.bit_num(m)?)),
                TokenElem::Without(m) => and.push(format!("!self.rec_marks_hold(x, {j}, 1 << {})", self.bit_num(m)?)),
                TokenElem::Starting(bytes) => and.push(format!("matches!(self.src[self.record(x, {j}).start as usize], {})", bytes.iter().map(|b| format!("{b:#04x}")).collect::<Vec<_>>().join(" | "))),
                TokenElem::End | TokenElem::Not(_) | TokenElem::Many(..) => return Err(format!("a gap question's element {c:?}")),
            }
        }
        // No kinds or words: any record.
        let takes = if pos.is_empty() { "true".to_owned() } else { format!("({})", pos.join(" || ")) };
        let mut c = if not { format!("!{takes}") } else { takes };
        for a in and {
            c = format!("{c} && {a}");
        }
        Ok((c, not))
    }

    fn bit_num(&self, m: &str) -> Result<u32, String> {
        let b = u32::from(self.x.marks.iter().find(|x| x.name == m && x.bit.is_some()).and_then(|x| x.bit).ok_or_else(|| format!("mark {m}: no bit in the code"))?);
        self.read_bit(b);
        Ok(b)
    }

    /// A gap question as a function of the token's record (`gap_pattern_holds`): `after`'s
    /// records before it, the last just before; `since`'s the grammar token before, then the comments since it;
    /// `before`'s from it on. The records are the tokens and the comments, by where they start (`back`, `ahead`).
    fn gap_fn(&self, q: &::bitlex::syntax::GapPattern, name: &str, fns: &mut String) -> Result<(), String> {
        let mut conds = Vec::new();
        let a = q.after.len() as isize;
        for (i, e) in q.after.iter().enumerate() {
            let (c, not) = self.gap_elem(e, "j")?;
            if q.since && i == 0 {
                // The grammar token before (none before the file's first).
                let none = if not { "true" } else { "false" };
                conds.push(format!("{{ if here == 0 {{ {none} }} else {{ let (x, j) = (false, here - 1); {c} }} }}"));
            } else if q.since {
                // The comments since it, before the token.
                conds.push(format!("{{ let (x, j) = (true, x0 + {i} - 1); j < self.xi && {c} }}"));
            } else {
                let none = if not { "true" } else { "false" };
                conds.push(format!("{{ match self.back(here, {}) {{ Some((x, j)) => {c}, None => {none} }} }}", a - i as isize));
            }
        }
        for (i, e) in q.before.iter().enumerate() {
            if matches!(e.as_slice(), [TokenElem::End]) {
                conds.push(format!("self.ahead(here, {i}).is_none()"));
                continue;
            }
            let (c, _) = self.gap_elem(e, "j")?;
            conds.push(format!("{{ match self.ahead(here, {i}) {{ Some((x, j)) => {c}, None => false }} }}"));
        }
        let g = if q.since { "        let x0 = self.since_comments(here);\n" } else { "" };
        let _ = write!(fns, "    #[allow(unused_variables)]\n    fn {name}(&self, here: usize) -> bool {{\n{g}        {}\n    }}\n\n", if conds.is_empty() { "true".to_owned() } else { conds.iter().map(|c| format!("({c})")).collect::<Vec<_>>().join("\n            && ") });
        Ok(())
    }

    /// A mark's questions of one way (`drops`, `loses`, …) as one test at the token: none of the
    /// `unless` ones holds, and one of the others does where it has any (`patterns_hold`).
    fn gap_test(&self, qs: &[::bitlex::syntax::GapPattern], name: &str, at_end: bool, fns: &mut String) -> Result<String, String> {
        let (mut yes, mut no) = (Vec::new(), Vec::new());
        for (e, q) in qs.iter().enumerate() {
            let f = format!("{name}_{e}");
            self.gap_fn(q, &f, fns)?;
            let call = format!("self.{f}(self.i)");
            if q.unless { no.push(call) } else { yes.push(call) }
        }
        let mut c: Vec<String> = no.iter().map(|x| format!("!{x}")).collect();
        if !yes.is_empty() {
            c.push(format!("({})", yes.join(" || ")));
        }
        if !at_end {
            c.insert(0, "self.i < self.n".to_owned());
        }
        let _ = write!(fns, "    fn {name}(&self) -> bool {{\n        {}\n    }}\n\n", if c.is_empty() { "true".to_owned() } else { c.join(" && ") });
        Ok(format!("self.{name}()"))
    }

    /// A mark's test as a condition on the token: a peek's tokens (`peek`), else its function's (`m_NAME`).
    fn mark_test(&self, m: usize) -> Cond {
        let o = self.mark_of(&self.x.marks[m].name).unwrap_or(m);
        let test = match self.peek(o) {
            Ok(c) if self.peeks() >> o & 1 == 1 => c,
            _ => Cond::text(format!("self.m_{}()", ident(&self.x.marks[m].name))),
        };
        Cond::Mark { mark: o, test: Box::new(test) }
    }

    /// The marks the code tests by looking at the tokens, as the lean parser's `is_p_at` and `kind_at`, a bit each by
    /// order: a hidden mark of tokens alone, with no question and nothing its taking does, whose taken bit no test can
    /// read. A mark taken at a token sets its own bit and the bits before it (and its `passes`), and the bits go when
    /// the token is taken. So a peek's bit is read only by a test of the peek at a token where a mark at or after it
    /// was taken, before the token is. Where no such test can come (`set_then_asked`), the peek holds wherever the mark
    /// did, and needs no bit, no cache (`held`) and no take.
    fn peeks(&self) -> u64 {
        if let Some(p) = self.peek_marks.get() {
            return p;
        }
        let n = self.x.marks.len();
        let first = |m: usize| self.mark_of(&self.x.marks[m].name).unwrap_or(m);
        let mut peeks = 0u64;
        for o in 0..n {
            if first(o) != o {
                continue;
            }
            let defs: Vec<&Mark> = self.x.marks.iter().filter(|x| x.name == self.x.marks[o].name).collect();
            // A mark of tokens, or of a gap's bit (a line break before the token, as the lean parser's `on_line`).
            let plain = defs.iter().all(|d| {
                d.next.is_empty()
                    && !d.ends
                    && !d.settles
                    && d.drops.is_empty()
                    && d.loses.is_empty()
                    && d.refuses.is_empty()
                    && d.refuses_taken.is_empty()
                    && d.passes == 0
                    && d.hidden
                    && match (d.tokens, d.bit) {
                        (Some((_, seq)), None) => !seq.is_empty() && d.unless_after.is_empty(),
                        (None, Some(_)) => true,
                        _ => false,
                    }
            });
            if plain {
                peeks |= 1 << o;
            }
        }
        // Less each mark whose taken bit a test reads, where only the marks that are no peeks are taken (a peek sets no
        // bit): to a fixed point.
        loop {
            let fewer = peeks & !self.set_then_asked(peeks);
            if fewer == peeks {
                break;
            }
            peeks = fewer;
        }
        self.peek_marks.set(Some(peeks));
        peeks
    }

    /// The marks whose taken bit a test can read: each a mark's take sets (it and those before it, and its `passes`)
    /// that a test may ask at the same token after the take, before a token is taken. What a rule asks after it ends
    /// is its callers' (`follow`, to a fixed point); the root's is nothing, the file's end.
    fn set_then_asked(&self, peeks: u64) -> u64 {
        let mut follow: HashMap<&str, u64> = HashMap::new();
        loop {
            let mut next = follow.clone();
            let mut read = 0u64;
            for (name, st) in &self.rules {
                if let Stmt::Node { body, .. } | Stmt::Choice { body, .. } = st {
                    let after = follow.get(name).copied().unwrap_or(0);
                    self.walk_asks(body, after, peeks, &mut next, &mut read);
                }
            }
            if next == follow {
                return read;
            }
            follow = next;
        }
    }

    /// `g`, with `after` the marks that may be asked after it before a token is taken: each call's follow grows by
    /// what may be asked after it (`follow`), and each mark's take adds the bits it sets that may be asked (`read`).
    fn walk_asks<'b>(&'b self, g: &'b Gram, after: u64, peeks: u64, follow: &mut HashMap<&'b str, u64>, read: &mut u64) {
        match g {
            Gram::Sym(n) => match self.mark_of(n) {
                Some(m) if peeks >> m & 1 == 1 => {}
                Some(m) => {
                    let set = (0..self.x.marks.len()).filter(|&o| o <= m || self.x.marks[m].passes >> o & 1 == 1).fold(0u64, |a, o| a | 1 << o);
                    *read |= set & after;
                }
                None if self.rules.contains_key(n.as_str()) => *follow.entry(n.as_str()).or_default() |= after,
                None => {}
            },
            Gram::Seq(v) => {
                let mut cur = after;
                for x in v.iter().rev() {
                    self.walk_asks(x, cur, peeks, follow, read);
                    let (a, empty) = self.first_asks(x, &mut HashSet::new());
                    cur = if empty { a | cur } else { a };
                }
            }
            Gram::Alt(v) => v.iter().for_each(|x| self.walk_asks(x, after, peeks, follow, read)),
            Gram::Opt(a) => self.walk_asks(a, after, peeks, follow, read),
            Gram::Star(a) | Gram::Plus(a) => {
                let again = self.first_asks(a, &mut HashSet::new()).0 | after;
                self.walk_asks(a, again, peeks, follow, read);
            }
            Gram::Field(_, a) | Gram::Retag(_, a) | Gram::Merge(_, a) | Gram::As(_, a) => self.walk_asks(a, after, peeks, follow, read),
            Gram::Wrap { body, .. } | Gram::Relabel { body, .. } | Gram::Fold { body, .. } | Gram::Reread { body, .. } | Gram::Flag { body, .. } | Gram::Unless { body, .. } => {
                self.walk_asks(body, after, peeks, follow, read)
            }
            Gram::Leaf { rule, .. } => *follow.entry(rule.as_str()).or_default() |= after,
            _ => {}
        }
    }

    /// The marks `g` may ask before it takes a token (a decision tests the marks its ways begin with), and whether it
    /// may take none.
    fn first_asks(&self, g: &Gram, seen: &mut HashSet<String>) -> (u64, bool) {
        match g {
            Gram::Sym(n) => match self.mark_of(n) {
                Some(m) => (1 << m, true),
                None if self.rules.contains_key(n.as_str()) => {
                    // A rule reached again before a token: left recursion, which no machine has.
                    if !seen.insert(n.clone()) {
                        return (0, false);
                    }
                    let r = match self.rules[n.as_str()] {
                        Stmt::Node { body, .. } | Stmt::Choice { body, .. } => self.first_asks(body, seen),
                        _ => (u64::MAX, true),
                    };
                    seen.remove(n);
                    r
                }
                None if self.token_of(g).is_some() => (0, false),
                None => (u64::MAX, true),
            },
            Gram::Seq(v) => {
                let mut acc = 0;
                for x in v {
                    let (a, empty) = self.first_asks(x, seen);
                    acc |= a;
                    if !empty {
                        return (acc, false);
                    }
                }
                (acc, true)
            }
            Gram::Alt(v) => v.iter().fold((0, false), |(acc, e), x| {
                let (a, empty) = self.first_asks(x, seen);
                (acc | a, e || empty)
            }),
            Gram::Opt(a) | Gram::Star(a) => (self.first_asks(a, seen).0, true),
            Gram::Plus(a) | Gram::Field(_, a) | Gram::Retag(_, a) | Gram::Merge(_, a) | Gram::As(_, a) => self.first_asks(a, seen),
            Gram::Wrap { body, .. } | Gram::Relabel { body, .. } | Gram::Fold { body, .. } | Gram::Reread { body, .. } | Gram::Flag { body, .. } | Gram::Unless { body, .. } => self.first_asks(body, seen),
            Gram::Leaf { rule, .. } => self.first_asks(&Gram::Sym(rule.clone()), seen),
            Gram::Lit(_) | Gram::Balanced { .. } | Gram::Any(_) => (0, false),
        }
    }

    /// A peek mark's test of the tokens from the token, as the lean parser's: `self.word() == w::A && self.at_K_at(self.i + 1)`.
    fn peek(&self, o: usize) -> Result<Cond, String> {
        let name = &self.x.marks[o].name;
        let mut ors: Vec<Cond> = Vec::new();
        let all = |c: Vec<Cond>| if c.len() == 1 { c.into_iter().next().expect("a test") } else { Cond::And(c) };
        for (d, def) in self.x.marks.iter().filter(|x| &x.name == name).enumerate() {
            // A gap's bit: the lean parser's `on_line`, asked of the token.
            if def.tokens.is_none()
                && let Some(b) = def.bit
            {
                self.read_bit(u32::from(b));
                let mut c = Vec::new();
                if def.word != 0 {
                    c.push(Cond::word(format!("w::{}", self.word_const(def.word as u8))));
                }
                let bit = Cond::text(format!("self.bit_at(self.i, {b})"));
                c.push(if def.end { Cond::Or(vec![Cond::text("self.i >= self.n"), bit]) } else { bit });
                for w in def.unless_after {
                    if let Some(id) = self.word_of(&String::from_utf8_lossy(w)) {
                        c.push(Cond::text(format!("{{ let p = self.prev_tok(self.i); p >= self.n || self.word_at(p) != w::{} }}", self.word_const(id))));
                    }
                }
                ors.push(all(c));
                continue;
            }
            let Some((before, seq)) = def.tokens else { return Err(format!("mark {name}: a peek of no tokens")) };
            let mut c = Vec::new();
            if !before.is_empty() {
                let p = self.elem(before, "self.i - 1")?;
                c.push(if matches!(before, [TokenElem::Not(_)]) { Cond::Or(vec![Cond::text("self.i == 0"), Cond::text(p)]) } else { Cond::And(vec![Cond::text("self.i > 0"), Cond::text(p)]) });
            }
            // A run that scans (a repetition, a token with or without a mark): its function (`marks` writes it), as the
            // lean parser's `literal` scans to the `)` where it decides.
            if let Some(h) = Self::bracket(seq) {
                for (k, e) in seq[..h].iter().enumerate() {
                    c.push(self.elem_cond(e, k)?);
                }
                let (f, _) = self.scan_fn(seq, h)?;
                let at = if h == 0 { "self.i".to_owned() } else { format!("self.i + {h}") };
                let tail: Vec<String> = seq[h + 3..].iter().enumerate().map(|(k, e)| self.elem(e, &if k == 0 { "e".to_owned() } else { format!("e + {k}") })).collect::<Result<_, _>>()?;
                c.push(Cond::Scan { f, at, tail: tail.join(" && "), held: false });
                ors.push(all(c));
                continue;
            }
            if !Self::fixed(seq) {
                c.push(Cond::text(format!("self.pat_{}_{d}_0(self.i, {})", ident(name), Self::run_prev(seq))));
                ors.push(all(c));
                continue;
            }
            for (k, e) in seq.iter().enumerate() {
                c.push(self.elem_cond(e, k)?);
            }
            ors.push(all(c));
        }
        Ok(if ors.len() == 1 { ors.remove(0) } else { Cond::Or(ors.into_iter().map(|x| Cond::Group(Box::new(x))).collect()) })
    }

    /// A peek's test of the token `k` places on: the words it is one of (the lean parser's `is_p_at`), else its
    /// element's test (`elem`).
    fn elem_cond(&self, e: &[TokenElem], k: usize) -> Result<Cond, String> {
        let ws: Option<Vec<String>> = e.iter().map(|x| match x {
            TokenElem::Word(w) => self.word_of(&String::from_utf8_lossy(w)).map(|id| format!("w::{}", self.word_const(id))),
            _ => None,
        }).collect();
        if let Some(ws) = ws.filter(|ws| !ws.is_empty()) {
            return Ok(Cond::Word { at: k, ws });
        }
        let t = self.elem(e, &if k == 0 { "self.i".to_owned() } else { format!("self.i + {k}") })?;
        Ok(Cond::text(if k == 0 { t.replace("self.word_at(self.i)", "self.word()") } else { t }))
    }

    /// Each mark's test, `m_NAME`: it holds at the token and is not taken there.
    /// Where a mark no rule takes (`open_mark`) may hold: at its marks' words alone, where each such mark starts with
    /// words (its guard, `first_words`); then the token kinds none of whose tokens is such a word (`word_in`'s, the lean
    /// lexer's). None where one may hold at any token.
    fn open_mark_words(&self) -> Option<(HashSet<String>, HashSet<String>)> {
        let peeks = self.peeks();
        let mut guards: HashMap<String, Option<Vec<String>>> = HashMap::new();
        for (order, m) in self.x.marks.iter().enumerate() {
            if peeks >> order & 1 == 1 || guards.contains_key(&ident(&m.name)) {
                continue;
            }
            let mut firsts: Option<Vec<String>> = Some(Vec::new());
            for def in self.x.marks.iter().filter(|x| x.name == m.name) {
                match (def.tokens.and_then(|(_, seq)| self.first_words(seq)), firsts.as_mut()) {
                    (Some(ws), Some(all)) => all.extend(ws),
                    _ => firsts = None,
                }
            }
            guards.insert(ident(&m.name), firsts);
        }
        let open: HashSet<String> = self.x.marks.iter().filter(|m| !m.hidden).map(|m| ident(&m.name)).collect();
        if !open.iter().all(|n| matches!(guards.get(n), Some(Some(_)))) {
            return None;
        }
        let words: HashSet<String> = open.iter().filter_map(|n| guards.get(n).cloned().flatten()).flatten().collect();
        let kinds_of = |w: &str| -> Vec<u8> {
            self.x.tokens.iter().filter(|(_, name)| {
                let mut ws = self.x.keywords.iter().filter(|r| r.0 == *name).flat_map(|r| r.1.iter().map(|(t, _)| t)).chain(self.x.soft.iter().filter(|r| r.0 == *name).flat_map(|r| r.1.iter()));
                ws.any(|t| self.word_of(&String::from_utf8_lossy(t)).is_some_and(|id| format!("w::{}", self.word_const(id)) == w))
            }).map(|(b, _)| *b).collect()
        };
        let marked: HashSet<u8> = words.iter().flat_map(|w| kinds_of(w)).collect();
        let free: HashSet<String> = self.x.tokens.iter().map(|(_, n)| n.strip_suffix("_close").unwrap_or(n)).filter(|k| self.x.tokens.iter().filter(|(_, n)| n == k || n.strip_suffix("_close") == Some(*k)).all(|(b, _)| !marked.contains(b))).map(ident).collect();
        Some((words, free))
    }

    /// The table of a choice of tokens (`ws`, each its word and its leaf's kind): rule `rule`'s, named for it, or the one
    /// of the choices written in rules (`ANON`). The take's leaf's kind, the word's in it.
    fn anon_table(&self, rule: Option<&str>, ws: &[(u8, String)]) -> Result<String, String> {
        let table = match rule.map(upper) {
            Some(t) if !["ANON", "NONE", "UNSET", "DEPTH", "TERMINATORS", "REMEMBERED", "REFUSED"].contains(&t.as_str()) && !t.starts_with("FLAG_") => t,
            Some(t) => format!("WORDS_{t}"),
            None => "ANON".to_owned(),
        };
        for (w, t) in ws {
            let k = self.kind_const(t)?;
            self.anon.borrow_mut().entry(table.clone()).or_default().insert(*w, k);
        }
        Ok(format!("{table}[self.word() as usize]"))
    }

    /// The rules' methods (`made`, each its method's name and moves), each rule called from one place written there
    /// (`rule`'s inline form): its method gone, as the lean parser writes one function where the grammar has a rule
    /// for each part (decision 0685 step 1). Not one that calls itself, nor one the root is, nor one longer than
    /// `INLINE_LINES`: an idea of its own, as the lean parser's `violation` and `raw_line` are functions of one caller.
    pub(crate) fn inlined(&self, made: Vec<(String, Block)>) -> Vec<Block> {
        let mut made: Vec<(String, Block, Option<Block>)> = made.into_iter().map(|(f, b)| {
            let inl = self.inline.borrow().get(&f).cloned();
            (f, b, inl)
        }).collect();
        self.repeated(&mut made);
        self.lists(&mut made);
        let mut kept: HashSet<String> = HashSet::new();
        loop {
            let text: String = made.iter().map(|(_, b, _)| moves::print(b, "")).collect();
            // The rules' own order is the caller's first: the last first, so a rule's size is that of what is written in it.
            let Some(i) = made.iter().rposition(|(f, _, inl)| inl.as_ref().is_some_and(|b| moves::print(b, "").lines().count() <= INLINE_LINES) && !kept.contains(f) && text.matches(&format!("self.{f}()")).count() + text.matches(&format!("Self::{f})")).count() == 1) else { break };
            let (f, b, inl) = made.remove(i);
            let body = inl.expect("an inline form");
            let mut found = false;
            for (_, m, mi) in made.iter_mut() {
                found |= moves::inline_call(m, &f, &body);
                if let Some(mi) = mi {
                    moves::inline_call(mi, &f, &body);
                }
            }
            if !found {
                kept.insert(f.clone());
                made.insert(i, (f, b, Some(body)));
            }
        }
        // The rules' code as written where they are called, for `opened_by_caller`.
        let mut inline = self.inline.borrow_mut();
        inline.clear();
        for (f, _, inl) in &made {
            if let Some(i) = inl {
                inline.insert(f.clone(), i.clone());
            }
        }
        *self.methods.borrow_mut() = made.iter().map(|(f, _, _)| f.clone()).collect();
        made.into_iter().map(|(_, b, _)| b).collect()
    }

    /// Where a decision's arms each open a node, an arm that only calls a node's rule of one caller opens and closes
    /// that node itself, and the rule's method reads what is inside it, as the lean parser's `violation`: so the
    /// decision can open its node once (`moves::open_once`, decision 0685 step 2).
    /// A method called from one place once the rewrites made it short (a decision's arms joined, `moves::joined`): its
    /// code written there, as `inlined` writes one from the start; of no parameters, no locals, no depth counted, no
    /// return but a refusal, up to `INLINE_LINES`. The method is gone (its block empty).
    fn late_inline(made: &mut [(String, Block)]) {
        loop {
            let texts: Vec<String> = made.iter().map(|(_, b)| moves::print(b, "")).collect();
            let one = (1..made.len()).find(|&i| {
                let (f, b) = &made[i];
                let Some((at, _)) = node_method(b) else { return false };
                let Op::At(_, inner) = &b[at] else { return false };
                let text = moves::print(inner, "");
                let header = moves::print(&b[..at], "");
                header.contains(&format!("fn {f}(&mut self) -> R {{"))
                    && !header.contains("deeper")
                    && text.lines().count() <= INLINE_LINES
                    && !text.contains("let ")
                    && !text.contains("return Ok")
                    && !moves::one_leaf_only(inner)
                    && !text.contains(&format!("self.{f}()"))
                    && texts.iter().map(|t| t.matches(&format!("self.{f}()")).count()).sum::<usize>() == 1
                    && !texts.iter().any(|t| t.contains(&format!("Self::{f}")))
            });
            let Some(i) = one else { break };
            let f = made[i].0.clone();
            let (at, node) = node_method(&made[i].1).expect("a method");
            let Op::At(_, mut inner) = made[i].1[at].clone() else { unreachable!() };
            // A node's method: its node opened and closed where it is written, as the lean parser's `violation` opens
            // `SuppressLine`.
            if let Some(k) = node {
                inner.insert(0, Op::Open(k));
                inner.push(Op::Close);
            }
            for (_, b) in made.iter_mut() {
                moves::inline_call(b, &f, &inner);
            }
            made[i].1.clear();
        }
    }

    /// A leaf of one or more tokens a loop reads (`leaf K R`, `R = X+`, its first `X` the loop's but that it refuses where
    /// the loop ends): the loop alone, and the leaf refused where it read none, as the lean parser's scan to the `)`.
    fn leaf_loops(made: &mut [(String, Block)]) {
        // Each method that is one loop of one match whose default ends it: that match's arms, printed.
        let loops: HashMap<String, String> = made
            .iter()
            .filter_map(|(f, b)| {
                let Op::At(_, inner) = &b[method_body(b)?] else { return None };
                let [Op::Loop(body)] = inner.as_slice() else { return None };
                let [Op::Match { subject, arms, default }] = body.as_slice() else { return None };
                matches!(default.as_slice(), [Op::Break]).then(|| (f.clone(), moves::print(&[Op::Match { subject: subject.clone(), arms: arms.clone(), default: vec![Op::Refuse] }], "")))
            })
            .collect();
        fn walk(b: &mut Block, loops: &HashMap<String, String>) {
            let mut i = 0;
            while i + 2 < b.len() {
                let hit = matches!(&b[i], Op::Line(l) if l.trim() == "self.leaf_open();")
                    && matches!(&b[i + 1], Op::Match { default, .. } if matches!(default.as_slice(), [Op::Refuse]))
                    && matches!(&b[i + 2], Op::Call(f) if loops.get(f).is_some_and(|m| *m == moves::print(&b[i + 1..i + 2], "")));
                if hit {
                    b[i + 1] = Op::If { arms: vec![(Cond::Text("self.i == self.leaf_from".into()), vec![Op::Refuse])], els: None };
                    b.swap(i + 1, i + 2);
                    i += 3;
                    continue;
                }
                i += 1;
            }
            for o in b.iter_mut() {
                match o {
                    Op::If { arms, els } => {
                        arms.iter_mut().for_each(|(_, x)| walk(x, loops));
                        if let Some(e) = els {
                            walk(e, loops);
                        }
                    }
                    Op::Match { arms, default, .. } => {
                        arms.iter_mut().for_each(|(_, x)| walk(x, loops));
                        walk(default, loops);
                    }
                    Op::Loop(x) | Op::While(_, x) | Op::At(_, x) => walk(x, loops),
                    _ => {}
                }
            }
        }
        for (_, b) in made.iter_mut() {
            walk(b, &loops);
        }
    }

    /// Methods whose code is the same but for their names (the inside of a brace pair read for a raw text and for a JSON
    /// object, alike once each is one leaf, `leaves`): one, the first, called for each, as the lean parser's one
    /// `braces`.
    fn same_methods(made: &mut [(String, Block)]) {
        for j in 1..made.len() {
            let g = made[j].0.clone();
            if made[j].1.is_empty() {
                continue;
            }
            // Its code, its doc and its name aside.
            let code = |b: &Block, name: &str| -> String {
                moves::print(b, "").lines().filter(|l| !l.trim_start().starts_with("///")).collect::<Vec<_>>().join("\n").replace(&format!("fn {name}("), "fn _(").replace(&format!("self.{name}()"), "self._()")
            };
            let Some(i) = (0..j).find(|&i| {
                let f = &made[i].0;
                !made[i].1.is_empty() && method_body(&made[i].1).is_some() && code(&made[j].1, &g) == code(&made[i].1, f)
            }) else {
                continue;
            };
            let f = made[i].0.clone();
            made[j].1.clear();
            for (_, b) in made.iter_mut() {
                moves::rename_calls(b, &g, &f);
            }
        }
    }

    /// A leaf the rules grow a token at a time (a take of the token as a leaf of kind `K`, then merges into it, here
    /// and in the functions they call that only merge) is one leaf over the tokens: `leaf_open`, the tokens passed,
    /// `leaf_close(K)`, as the lean parser scans and then writes one `leaf_over` (rock 3). For a kind each of whose
    /// merges, and each call of whose merging functions, is in such a run; else as it was. Then a merge of one token
    /// into the one before is a leaf over the two (`moves::two_token_leaves`).
    fn leaves(made: &mut [(String, Block)]) {
        let mut kinds = Vec::new();
        for (_, b) in made.iter_mut() {
            moves::merge_kinds(b, &mut kinds);
        }
        let body = method_body;
        for k in kinds {
            // The functions that only merge into `k`: those whose code merges, calling only such (the most there are:
            // they may call each other).
            let mut merging: HashSet<String> = made.iter().filter(|(_, b)| body(b).is_some()).map(|(f, _)| f.clone()).collect();
            loop {
                let out: Vec<String> = made
                    .iter()
                    .filter(|(f, b)| merging.contains(f) && !body(b).is_some_and(|at| matches!(&b[at], Op::At(_, x) if moves::merges_only(x, &k, &merging))))
                    .map(|(f, _)| f.clone())
                    .collect();
                if out.is_empty() {
                    break;
                }
                for f in out {
                    merging.remove(&f);
                }
            }
            let mut new: Vec<(String, Block)> = made.to_vec();
            let mut calls: HashMap<String, usize> = HashMap::new();
            let mut any = false;
            for (f, b) in new.iter_mut() {
                if merging.contains(f) {
                    moves::passes_in(b, &k, &merging, &mut calls);
                } else {
                    any |= moves::one_leaf(b, &k, &merging, &mut calls);
                }
            }
            let mut left = Vec::new();
            for (_, b) in new.iter_mut() {
                moves::merge_kinds(b, &mut left);
            }
            let all_calls = merging.iter().all(|f| calls.get(f).copied().unwrap_or(0) == made.iter_mut().map(|(_, b)| moves::calls_of(b, f)).sum::<usize>());
            if any && !left.contains(&k) && all_calls {
                made.clone_from_slice(&new);
            }
        }
        // A merge left joins one token to the one before: one leaf over the two, its kind chosen first.
        for (_, b) in made.iter_mut() {
            moves::two_token_leaves(b);
        }
    }

    /// `made`: the rules' methods (`inlined`'s, each its name's), their moves rewritten. A method rewritten again is
    /// `rewrite`'s.
    fn opened_by_caller(&self, made: &mut [(String, Block)], rewrite: &dyn Fn(&mut Block)) {
        let text: String = made.iter().map(|(_, b)| moves::print(b, "")).collect();
        let inline = self.inline.borrow();
        // A rule's method, of one call, its inline form a node's: open, its code, close, `marked` cleared.
        let split: HashMap<String, Block> = made
            .iter()
            .filter_map(|(f, _)| inline.get(f).filter(|b| text.matches(&format!("self.{f}()")).count() + text.matches(&format!("Self::{f})")).count() == 1 && matches!(b.first(), Some(Op::Open(_))) && b.len() >= 3 && matches!(b[b.len() - 2], Op::Close)).map(|b| (f.clone(), b.clone())))
            .collect();
        let mut opened: Vec<String> = Vec::new();
        for (_, b) in made.iter_mut() {
            moves::open_arms(b, &split, self.marked_read.get(), &mut opened);
        }
        for (f, b) in made.iter_mut() {
            if !opened.contains(f) {
                continue;
            }
            let inl = &split[f.as_str()];
            let doc = match b.first() {
                Some(Op::Line(l)) => l.lines().next().unwrap_or_default().to_owned(),
                _ => String::new(),
            };
            let mut body = inl[1..inl.len() - 2].to_vec();
            rewrite(&mut body);
            *b = vec![line(format!("{doc}\n    fn {f}(&mut self) -> R {{\n")), Op::At("        ".into(), body), line("        Ok(())\n    }\n\n")];
        }
        for (_, b) in made.iter_mut() {
            moves::open_once(b);
        }
    }

    /// The loops the rules repeat (the same moves in more than one place, `REPEATED_LINES` or more): each one function,
    /// called where it was, as the lean parser's `rest_of_line` (decision 0685 step 4). Named for the one field it
    /// sets (`modifiers`). Not a loop that reads a local of the code around it.
    fn repeated(&self, made: &mut Vec<(String, Block, Option<Block>)>) {
        let mut n = 0;
        loop {
            let mut all = Vec::new();
            for (_, b, _) in made.iter() {
                moves::loops(b, &mut all);
            }
            let mut counts: Vec<(Block, usize)> = Vec::new();
            for seq in all {
                match counts.iter_mut().find(|(o, _)| *o == seq) {
                    Some(c) => c.1 += 1,
                    None => counts.push((seq, 1)),
                }
            }
            let own = |seq: &Block| {
                let text = moves::print(seq, "");
                let locals = ["asks", "flags", "word", "refusing", "first", "at", "kind"];
                text.lines().count() >= REPEATED_LINES
                    && !locals.iter().any(|l| {
                        let uses = text.match_indices(l).any(|(i, _)| {
                            let before = text[..i].chars().next_back();
                            let after = text[i + l.len()..].chars().next();
                            !before.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '.' || c == ':') && !after.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '(' || c == ':' || c == '!')
                        });
                        uses && !text.contains(&format!("let {l} "))
                    })
                    && !text.contains("past_")
            };
            // The longest first: a loop inside it goes with it, and the move after it where each place has the same.
            let Some((seq, _)) = counts.into_iter().filter(|(o, c)| *c > 1 && own(o)).max_by_key(|(o, _)| moves::print(o, "").len()) else { break };
            let name = moves::what_of(&seq).map_or_else(|| format!("part_{n}"), |w| format!("{w}s"));
            let name = if made.iter().any(|(f, _, _)| *f == name) || rule_fn(&name) != name { format!("{name}_{n}") } else { name };
            n += 1;
            for (_, b, inl) in made.iter_mut() {
                moves::call_for(b, &seq, &name);
                if let Some(i) = inl {
                    moves::call_for(i, &seq, &name);
                }
            }
            let block = vec![line(format!("    /// The loop the parser reads in more than one place (decision 0685 step 4).\n    fn {name}(&mut self) -> R {{\n")), Op::At("        ".into(), seq), line("        Ok(())\n    }\n\n")];
            made.push((name, block, None));
        }
    }

    /// The end of `leaf K R`: the marks `R` refuses checked over its tokens where the grammar says so
    /// (`leaf_refuses`), as the lean parser checks a policy document is JSON once it has read it, then the leaf
    /// (`leaf_close`, the lean parser's `leaf_over`).
    fn leaf_end(&self, r: &str, kind: &str) -> Result<Vec<Op>, String> {
        let mut out = Vec::new();
        if let Some(m) = self.leaf_refuses.get(r).filter(|m| **m != 0) {
            out.push(line(format!("self.leaf_refuses({m:#x})?;")));
        }
        out.push(line(format!("self.leaf_close({})?;", self.kind_const(kind)?)));
        Ok(out)
    }

    /// The runs around a loop the rules repeat but for the rules they call and the fields they set: each one function
    /// whose parameters are those (`field: u16`, `item: fn(&mut Self) -> R`), called with each place's, as the lean
    /// parser's `list(close, field, item)` (decision 0685 step 4). Of 4 lines or more, not reading a local.
    fn lists(&self, made: &mut Vec<(String, Block, Option<Block>)>) {
        let mut n = 0;
        loop {
            let mut all = Vec::new();
            for (_, b, _) in made.iter() {
                moves::runs(b, &mut all);
            }
            // Each abstracted run, and each place's slots.
            let mut groups: Vec<(Block, Vec<(Vec<String>, Vec<String>)>)> = Vec::new();
            for seq in all {
                let (a, c, f) = moves::abstracted(&seq);
                if c.is_empty() && f.is_empty() {
                    continue;
                }
                match groups.iter_mut().find(|(x, _)| *x == a) {
                    Some(g) => g.1.push((c, f)),
                    None => groups.push((a, vec![(c, f)])),
                }
            }
            let fits = |a: &Block, places: &[(Vec<String>, Vec<String>)]| {
                let text = moves::print(a, "");
                let varies = |pick: &dyn Fn(&(Vec<String>, Vec<String>)) -> &Vec<String>| (0..pick(&places[0]).len()).filter(|&k| places.iter().any(|p| pick(p)[k] != pick(&places[0])[k])).count();
                let (vc, vf) = (varies(&|p| &p.0), varies(&|p| &p.1));
                places.len() > 1
                    && vc + vf > 0
                    && vc <= 1
                    && vf <= 1
                    && text.lines().count() >= LIST_LINES
                    && !["asks", "flags", "refusing", "past_", "word;", "let "].iter().any(|l| text.contains(l))
            };
            let Some((norm, places)) = groups.into_iter().filter(|(a, p)| fits(a, p)).max_by_key(|(a, p)| (moves::print(a, "").len(), p.len())) else { break };
            let first = &places[0];
            let call_varies = (0..first.0.len()).find(|&k| places.iter().any(|p| p.0[k] != first.0[k]));
            let field_varies = (0..first.1.len()).find(|&k| places.iter().any(|p| p.1[k] != first.1[k]));
            let name = if n == 0 { "list".to_owned() } else { format!("list_{n}") };
            let name = if made.iter().any(|(f, _, _)| *f == name) || rule_fn(&name) != name { format!("{name}_{n}") } else { name };
            n += 1;
            // Its code: a varying slot its parameter's, a slot all places fill alike that filling.
            let body = moves::filled(&norm, &|i| if Some(i) == call_varies { line("item(self)?;") } else { Op::Call(first.0[i].clone()) }, &|i| if Some(i) == field_varies { "field".to_owned() } else { first.1[i].clone() });
            let mut params = String::new();
            if field_varies.is_some() {
                params.push_str(", field: u16");
            }
            if call_varies.is_some() {
                params.push_str(", item: fn(&mut Self) -> R");
            }
            let call = |c: &[String], f: &[String]| {
                let mut args = Vec::new();
                if let Some(k) = field_varies {
                    args.push(f[k].clone());
                }
                if let Some(k) = call_varies {
                    args.push(format!("Self::{}", c[k]));
                }
                line(format!("self.{name}({})?;", args.join(", ")))
            };
            for (_, b, inl) in made.iter_mut() {
                moves::call_for_run(b, &norm, &call);
                if let Some(i) = inl {
                    moves::call_for_run(i, &norm, &call);
                }
            }
            let block = vec![line(format!("    /// A list the parser reads in more than one place, its field and its item each place's (decision 0685 step\n    /// 4), as the lean parser's `list`.\n    fn {name}(&mut self{params}) -> R {{\n")), Op::At("        ".into(), body), line("        Ok(())\n    }\n\n")];
            made.push((name, block, None));
        }
    }

    /// Each block's moves in the lean parser's forms (`moves::rewrite`), then printed, once all of them are rewritten:
    /// their takes named as all of them ask (`moves::Names`).
    pub(crate) fn printed(&self, mut bs: Vec<Block>) -> Vec<String> {
        let marks = self.mark_words.get_or_init(|| self.open_mark_words());
        let anon: Vec<(String, HashSet<String>)> = self.anon.borrow().iter().filter(|(t, _)| *t != "ANON").map(|(t, ws)| (t.clone(), ws.keys().map(|w| format!("w::{}", self.word_const(*w))).collect())).collect();
        let rewrite = |b: &mut Block| {
            moves::rewrite(b, &moves::Cx { marks: marks.as_ref(), marked: self.marked_read.get() });
            moves::classes(b, &anon);
        };
        bs.iter_mut().for_each(&rewrite);
        // The rules' methods (`inlined`, in order): each decision whose arms open a node opens it once.
        let methods = self.methods.borrow().clone();
        if methods.len() == bs.len() {
            let mut made: Vec<(String, Block)> = methods.into_iter().zip(std::mem::take(&mut bs)).collect();
            self.opened_by_caller(&mut made, &rewrite);
            Self::leaves(&mut made);
            bs = made.into_iter().map(|(_, b)| b).collect();
        }
        let holds = |c: &Cond| self.holds(c);
        let apart = |a: &Holds, b: &Holds| self.apart(a, b);
        bs.iter_mut().for_each(|b| {
            moves::open_first(b);
            moves::known_takes(b);
            moves::joined(b);
            moves::into_default(b, &holds, &apart);
        });
        // A method those made short and of one caller: written there, and its caller's arms joined again.
        let methods = self.methods.borrow().clone();
        if methods.len() == bs.len() {
            let mut made: Vec<(String, Block)> = methods.into_iter().zip(std::mem::take(&mut bs)).collect();
            Self::late_inline(&mut made);
            Self::leaf_loops(&mut made);
            Self::same_methods(&mut made);
            bs = made.into_iter().map(|(_, b)| b).collect();
            bs.iter_mut().for_each(|b| {
                moves::known_takes(b);
                moves::joined(b);
                moves::into_default(b, &holds, &apart);
            });
        }
        let names = self.names.get_or_init(|| moves::Names::of(&mut bs));
        bs.iter().map(|b| moves::print_named(b, "", names)).collect()
    }

    /// The takes the rules' code calls (`names`), in the parser `out`: `take`, which takes the token, and the lean
    /// parser's helpers over it (`moves::Take::call`). Where the takes ask one pair of tests (`M`, `F`), `take` asks
    /// that pair and no helper has const parameters; else each has both, as each take asks them. `take` holds what
    /// the parser has: an extra lost (`lost`), the marks taken (`taken`), `marked`, a record split (`sub`).
    fn take_helpers(&self, out: &str) -> (String, String) {
        let names = self.names.get().expect("the rules printed");
        let has = |f: &str| out.contains(&format!("    {f}: "));
        let generic = names.pairs.len() > 1;
        let (g, gm) = if generic { ("<const M: bool, const F: bool>", "::<M, F>") } else { ("", "") };
        let m = names.pairs.iter().any(|p| p.0);
        let f = has("refusing") && names.pairs.iter().any(|p| p.1);
        let mut take = String::new();
        if generic {
            take.push_str("    /// The token taken: its span. Its tests those the take needs (`takes`): `M`, whether a mark no rule takes may\n    /// hold at the token (its word may be one of theirs); `F`, whether the level open may refuse a mark there (a rule a\n    /// refusing level reaches). The lean parser asks neither at a token it knows.\n");
        } else {
            take.push_str("    /// The token taken: its span; the next token's marks not yet taken.\n");
        }
        let _ = writeln!(take, "    #[inline(always)]\n    fn take{g}(&mut self) -> R<(u32, u32)> {{");
        if m && generic {
            take.push_str("        // Not past the last record: a take of `M` false passed a word's or a kind's test, which fails there.\n        if M && (self.i >= self.n || self.open_mark()) {\n            return self.refuse();\n        }\n");
        } else if m {
            take.push_str("        // Not past the last record, and no mark no rule takes at the token (`open_mark`).\n        if self.i >= self.n || self.open_mark() {\n            return self.refuse();\n        }\n");
        }
        if f {
            let _ = writeln!(take, "        // A mark the level refuses, at the token it takes (`refuse M in L`).\n        if {}{}self.refusing != 0 && self.marks_hold(self.i, self.refusing) {{\n            return self.refuse();\n        }}", if generic { "F && " } else { "" }, if has("sub") { "self.k == 0 && " } else { "" });
        }
        if has("lost") {
            take.push_str("        if let Some((kind, None)) = self.lost {\n            self.lost = Some((kind, Some(self.start())));\n        }\n");
        }
        if has("taken") {
            take.push_str("        self.taken = 0;\n");
        }
        if has("marked") {
            take.push_str("        self.marked = false;\n");
        }
        if has("sub") {
            take.push_str("        if self.k < self.sub.len() {\n            let (s, l, _) = self.sub[self.k];\n            self.k += 1;\n            if self.k < self.sub.len() {\n                self.known = 0;\n                self.last_end = l + 1;\n                return Ok((s, l + 1));\n            }\n            self.sub.clear();\n            self.k = 0;\n            self.advance(l as usize + 1)?;\n            self.last_end = l + 1;\n            return Ok((s, l + 1));\n        }\n");
        }
        take.push_str("        let (s, e) = (self.recs[self.i].start, self.recs[self.i].last + 1);\n        self.advance(e as usize)?;\n        self.last_end = e;\n        Ok((s, e))\n    }\n\n");
        // Each helper: its name, its doc, its parameters and its body; written where a take calls it or a helper
        // written calls it.
        let calls = |n: &str| names.calls.contains(n);
        let tk = ["id", "bump_as", "tk_as", "p_stored"].iter().any(|n| calls(n));
        let implied = ["p", "implied", "implied_if"].iter().any(|n| calls(n));
        let pass = ["pass_as", "word_pass_as"].iter().any(|n| calls(n));
        let doc_tk = if generic { " (`take`'s `M` and `F`)" } else { "" };
        let helpers: [(&str, bool, String, &str, String); 10] = [
            ("id", calls("id"), "An identifier, a leaf given `field` (the lean parser's `id` and `field`).".into(), "field: u16", format!("        self.tk_as{gm}(self.kind() == t::IDENTIFIER, k::IDENTIFIER, field)\n")),
            ("bump_as", calls("bump_as"), "The token, a leaf of `kind` (the lean parser's `bump_as`).".into(), "kind: u16", format!("        self.tk_as{gm}(true, kind, 0)\n")),
            ("tk_as", tk, format!("The token, which must pass `ok`, a leaf of `kind` given `field`{doc_tk}."), "ok: bool, kind: u16, field: u16", format!("        if !ok {{\n            return self.refuse();\n        }}\n        self.flush();\n        let (s, e) = self.take{gm}()?;\n        let at = self.w.push_leaf(u32::from(kind) | u32::from(field) << 10, s, e);\n        self.set_made(at as u32);\n        Ok(())\n")),
            ("p_stored", calls("p_stored"), "The token, which must be word `w`, a leaf of `kind`.".into(), "w: u32, kind: u16", format!("        self.tk_as{gm}(self.word() == w, kind, 0)\n")),
            ("p", calls("p"), "The token, which must be word `w`: one the grammar implies, no leaf.".into(), "w: u32", format!("        self.implied_if{gm}(self.word() == w)\n")),
            ("implied", calls("implied"), "A token the grammar implies, no leaf (the lean parser's `implied`).".into(), "", format!("        self.implied_if{gm}(true)\n")),
            ("implied_if", implied, "The token, which must pass `ok`: one the grammar implies, no leaf.".into(), "ok: bool", format!("        if !ok {{\n            return self.refuse();\n        }}\n        self.flush();\n        self.take{gm}()?;\n        Ok(())\n")),
            ("merge_as", calls("merge_as"), "The token, which must pass `ok`, joined to the leaf made last: one leaf of `kind`.".into(), "ok: bool, kind: u16", format!("        let at = self.top.1;\n        if !ok || at == NONE || at as usize + 1 != self.w.len() {{\n            return self.refuse();\n        }}\n        let (s, key) = (self.w.start(at as usize), self.w.key(at as usize));\n        // The comments between are its text.\n        self.pend = self.xi;\n        let (_, e) = self.take{gm}()?;\n        self.w.truncate(at as usize);\n        let at = self.w.push_leaf(key & !0x3ff | u32::from(kind), s, e);\n        self.set_made(at as u32);\n        Ok(())\n")),
            ("word_pass_as", calls("word_pass_as"), "The token, which must be word `w`: read into a `leaf`, no leaf of its own.".into(), "w: u32", format!("        self.pass_as{gm}(self.word() == w)\n")),
            ("pass_as", pass, "The token, which must pass `ok`: read into a `leaf`, no leaf of its own; the comments before it stay pending,\n    /// the leaf's text (`leaf_close`).".into(), "ok: bool", format!("        if !ok {{\n            return self.refuse();\n        }}\n        self.take{gm}()?;\n        Ok(())\n")),
        ];
        let mut takes = String::new();
        for (name, used, doc, params, body) in helpers {
            if !used {
                continue;
            }
            let inline = if name == "merge_as" { "" } else { "    #[inline(always)]\n" };
            let sep = if params.is_empty() { "" } else { ", " };
            let _ = write!(takes, "    /// {doc}\n{inline}    fn {name}{g}(&mut self{sep}{params}) -> R {{\n{body}    }}\n\n");
        }
        (take, takes)
    }

    fn marks(&self, out: &mut String) -> Result<(), String> {
        if self.x.marks.len() > 64 {
            return Err(format!("{} marks: a token's marks are one u64", self.x.marks.len()));
        }
        let mut done: HashSet<&str> = HashSet::new();
        let mut fns = String::new();
        // Each mark's guard words (`first_words`), by its name: none where a definition starts otherwise.
        let mut guards: std::collections::BTreeMap<String, Option<Vec<String>>> = std::collections::BTreeMap::new();
        // Each mark's bit where its test reads only the tokens (`open_mark` asks it with no cache).
        let mut direct: std::collections::BTreeMap<String, Option<usize>> = std::collections::BTreeMap::new();
        let peeks = self.peeks();
        for (order, m) in self.x.marks.iter().enumerate() {
            if !done.insert(m.name.as_str()) {
                continue;
            }
            let n = ident(&m.name);
            // A peek: no functions of its own but the scans its runs that are not fixed need (`peek`).
            if peeks >> order & 1 == 1 {
                for (d, def) in self.x.marks.iter().filter(|x| x.name == m.name).enumerate() {
                    if let Some((_, seq)) = def.tokens
                        && let Some(h) = Self::bracket(seq)
                    {
                        let (f, code) = self.scan_fn(seq, h)?;
                        if !fns.contains(&format!("    fn {f}(")) {
                            fns.push_str(&code);
                        }
                    } else if let Some((_, seq)) = def.tokens
                        && !Self::fixed(seq)
                    {
                        self.run(seq, &format!("pat_{n}_{d}"), &mut fns)?;
                    }
                }
                continue;
            }
            let defs: Vec<(usize, &Mark)> = self.x.marks.iter().enumerate().filter(|(_, x)| x.name == m.name).collect();
            let mut holds = Vec::new();
            // The words the token must be for any definition to hold: a guard before the mark's cache (`held`).
            let mut firsts: Option<Vec<String>> = Some(Vec::new());
            for (d, (_, def)) in defs.iter().enumerate() {
                let mut c = Vec::new();
                match (def.tokens.and_then(|(_, seq)| self.first_words(seq)), firsts.as_mut()) {
                    (Some(ws), Some(all)) => all.extend(ws),
                    _ => firsts = None,
                }
                if let Some((before, seq)) = def.tokens {
                    let pat = format!("pat_{n}_{d}");
                    self.run(seq, &pat, &mut fns)?;
                    c.push(format!("self.{pat}_0(self.i, {})", Self::run_prev(seq)));
                    if !before.is_empty() {
                        let p = self.elem(before, "p")?;
                        let negated = matches!(before, [TokenElem::Not(_)]);
                        c.push(if negated { format!("{{ let p = self.prev_tok(self.i); p >= self.n || {p} }}") } else { format!("{{ let p = self.prev_tok(self.i); p < self.n && {p} }}") });
                    }
                } else if let Some(b) = def.bit {
                    self.read_bit(u32::from(b));
                    if def.word != 0 {
                        c.push(format!("self.word() == {}", def.word));
                    }
                    c.push(if def.end { format!("(if self.i < self.n {{ self.bit_at(self.i, {b}) }} else {{ true }})") } else { format!("self.bit_at(self.i, {b})") });
                } else {
                    c.push("false".into());
                }
                // `unless after W`: none after the word.
                for w in def.unless_after {
                    if let Some(id) = self.word_of(&String::from_utf8_lossy(w)) {
                        c.push(format!("{{ let p = self.prev_tok(self.i); p >= self.n || self.word_at(p) != w::{} }}", self.word_const(id)));
                    }
                }
                // `before` and `unless before`: the runs from the token.
                let (mut yes, mut no) = (Vec::new(), Vec::new());
                for (e, (unless, seq)) in def.next.iter().enumerate() {
                    let pat = format!("next_{n}_{d}_{e}");
                    self.run(seq, &pat, &mut fns)?;
                    let call = format!("self.{pat}_0(self.i, {})", Self::run_prev(seq));
                    if *unless { no.push(call) } else { yes.push(call) }
                }
                for x in no {
                    c.push(format!("!{x}"));
                }
                if !yes.is_empty() {
                    c.push(format!("({})", yes.join(" || ")));
                }
                holds.push(format!("({})", c.join(" && ")));
            }
            guards.insert(n.clone(), firsts.clone());
            // A test of the tokens alone (no state the rules set): `open_mark` asks it afresh, not through the cache.
            let body = holds.join(" || ");
            let pure = ["self.flags", "self.asks", "self.word ", "self.word)", "self.refusing", "self.taken", "self.marked"].iter().all(|s| !body.contains(s));
            direct.insert(n.clone(), pure.then_some(order));
            let guard = match firsts {
                Some(mut ws) if !ws.is_empty() => {
                    ws.sort();
                    ws.dedup();
                    format!("matches!(self.word(), {}) && ", ws.join(" | "))
                }
                _ => String::new(),
            };
            let _ = writeln!(out, "    /// Mark `{}` holds at the token, not taken there; once a token (`held`).\n    #[inline]\n    fn m_{n}(&mut self) -> bool {{\n        {guard}self.taken >> {order} & 1 == 0 && self.held({order}, Self::h_{n})\n    }}\n\n    #[inline]\n    fn h_{n}(&self) -> bool {{\n        {}\n    }}\n", m.name, holds.join(" || "));
            // Taken: `refuses taken` asked; the extras before its bit settled; `drops` asked.
            let qs = |f: fn(&Mark<'a>) -> &'a [::bitlex::syntax::GapPattern]| -> Vec<::bitlex::syntax::GapPattern> { defs.iter().flat_map(|(_, d)| f(d).iter().cloned()).collect() };
            let mut take = String::new();
            let refuses_taken = qs(|d| d.refuses_taken);
            if !refuses_taken.is_empty() {
                let t = self.gap_test(&refuses_taken, &format!("refuses_taken_{n}"), true, &mut fns)?;
                let _ = write!(take, "        if {t} {{\n            return self.refuse();\n        }}\n");
            }
            if m.settles
                && let Some(b) = m.bit
            {
                self.read_bit(u32::from(b));
                let _ = writeln!(take, "        self.settle({b});");
            }
            let drops = qs(|d| d.drops);
            if !drops.is_empty() {
                let t = self.gap_test(&drops, &format!("drops_{n}"), false, &mut fns)?;
                let _ = write!(take, "        if {t} {{\n            self.drop_pending();\n        }}\n");
            }
            let _ = writeln!(out, "    /// Mark `{}` taken: it and the marks before it passed at this token.\n    fn take_{n}(&mut self) -> R {{\n{take}        self.taken |= {:#x};\n        Ok(())\n    }}\n", m.name, (u64::MAX >> (63 - order)) | m.passes);
            let _ = writeln!(out, "    /// Mark `{}` where the rule takes it: it must hold.\n    fn mark_{n}(&mut self) -> R {{\n        if !self.m_{n}() {{\n            return self.refuse();\n        }}\n{}        self.take_{n}()\n    }}\n", m.name, if self.marked_read.get() { "        self.marked = true;\n" } else { "" });
            // Asked wherever a decision tests it (`refuses`, then `loses`): at a level that takes
            // it, always; where the level may end at it (`end`), where the caller's level is the
            // mark's (`asks`, the lean parser's `stmt`).
            // `loses since`: the first extra pending moved (`move_first`), apart from the others.
            let (refuses, all) = (qs(|d| d.refuses), qs(|d| d.loses));
            let (moves, loses): (Vec<_>, Vec<_>) = all.into_iter().partition(|q| q.since);
            if !refuses.is_empty() || !loses.is_empty() || !moves.is_empty() {
                let mut ask = String::new();
                if !refuses.is_empty() {
                    let t = self.gap_test(&refuses, &format!("refuses_{n}"), true, &mut fns)?;
                    let _ = write!(ask, "        if {t} {{\n            return self.refuse();\n        }}\n");
                }
                if !loses.is_empty() {
                    let t = self.gap_test(&loses, &format!("loses_{n}"), false, &mut fns)?;
                    let _ = write!(ask, "        if {t} {{\n            self.lose_pending();\n        }}\n");
                }
                if !moves.is_empty() {
                    let t = self.gap_test(&moves, &format!("moves_{n}"), false, &mut fns)?;
                    let _ = write!(ask, "        if {t} {{\n            self.move_first();\n        }}\n");
                }
                let _ = writeln!(out, "    /// Mark `{}`'s questions, where a decision tests it (`end`: where the level may end at it).\n    fn ask_{n}(&mut self, end: bool) -> R {{\n        if end && self.asks >> {order} & 1 == 0 {{\n            return Ok(());\n        }}\n{ask}        Ok(())\n    }}\n", m.name);
            }
        }
        out.push_str(&fns);
        // The marks no rule may pass: where one holds at a token taken, the file is refused.
        // Asked by the token's word, as the lean parser's: a mark whose runs start with words only at those words; the
        // others at every token.
        let mut open: Vec<String> = self.x.marks.iter().filter(|m| !m.hidden).map(|m| ident(&m.name)).collect::<HashSet<_>>().into_iter().collect();
        open.sort();
        // A mark whose test reads only the tokens, asked once a token taken: its test, as its cache would give it.
        let call = |n: &String| match direct.get(n) {
            Some(Some(o)) => format!("(self.taken >> {o} & 1 == 0 && self.h_{n}())"),
            _ => format!("self.m_{n}()"),
        };
        let always: Vec<String> = open.iter().filter(|n| !matches!(guards.get(*n), Some(Some(_)))).map(call).collect();
        let mut by_word: std::collections::BTreeMap<String, Vec<String>> = std::collections::BTreeMap::new();
        for n in &open {
            if let Some(Some(ws)) = guards.get(n) {
                for w in ws {
                    by_word.entry(w.clone()).or_default().push(call(n));
                }
            }
        }
        let any = |cs: Vec<String>| if cs.is_empty() { "false".to_owned() } else { cs.join(" || ") };
        let body = if by_word.is_empty() {
            any(always.clone())
        } else {
            let mut arms: String = by_word.into_iter().map(|(w, cs)| format!("            {w} => {},\n", any(cs.into_iter().chain(always.iter().cloned()).collect()))).collect();
            let _ = writeln!(arms, "            _ => {},", any(always.clone()));
            format!("match self.word() {{\n{arms}        }}")
        };
        let _ = writeln!(out, "    /// A mark no rule takes holds at the token (a mark not hidden).\n    fn open_mark(&mut self) -> bool {{\n        {body}\n    }}\n");
        Ok(())
    }

    /// The split of each region tier 2 reads in parts (`Split::Tier2`, decision 0323's lowering B),
    /// as `records::read_parts` reads one: `split` at a record whose first byte opens one, a
    /// `split_R` a reader; `word_in`, a piece's word; `KINDS`, a byte's record kind.
    fn splits(&self, out: &mut String, free: &mut String) -> Result<(), String> {
        let mut at = String::new();
        // A string's region a template's `}` resumes (decision 0600): the reader that resumes
        // it, entered at its opener where that comes before the next record.
        let mut resume = String::new();
        let readers = crate::bitlex_lex_rust::readers(self.x.lexer);
        let lexer_regions: Vec<&::bitlex::syntax::Region> = readers.iter().flat_map(|rd| &rd.regions).collect();
        let kind_or = |k: Option<u8>, at: &str| match k {
            Some(k) => format!("{k}"),
            None => format!("u32::from(KINDS[usize::from(self.src[{at}])])"),
        };
        for (r, rd) in readers.iter().enumerate() {
            let piece = |name: &str, closes: bool| crate::bitlex_lex_rust::piece_kind(self.x.lexer, self.x.row_kinds, &rd.kind, name, closes);
            // A reader that resumes has no record of its own to split (`advance` enters it).
            if !rd.regions.iter().all(|g| g.resumes) {
                let _ = write!(at, "        if bit(&self.m.{}, s) {{\n            return self.split_{r}(j);\n        }}\n", rd.open);
            }
            let content = kind_or(piece(&rd.content, false), "at");
            let mut parts = String::new();
            let mut lasts = String::new();
            for (i, (name, start, len, last)) in rd.parts.iter().enumerate() {
                let bytes = match len {
                    ::bitlex::syntax::PartLen::Bytes(b) => *b,
                    ::bitlex::syntax::PartLen::Last(_) => 0,
                };
                let _ = write!(parts, "            // `{name}`\n            if let Some(e) = first_bit(&m.{start}, at).filter(|&e| e < c) {{\n                if part.is_none_or(|q| (e, {bytes}, {i}) < q) {{\n                    part = Some((e, {bytes}, {i}));\n                }}\n            }}\n");
                let l = match last {
                    Some(f) => format!("match first_bit(&m.{f}, e) {{\n                            Some(l) => l,\n                            None => return self.refuse(),\n                        }}"),
                    None => format!("e + {}", usize::from(bytes).saturating_sub(1)),
                };
                let _ = writeln!(lasts, "                        {i} => ({l}, {}),", kind_or(piece(name, false), "e"));
            }
            // The closer: one of a kind of its own (`closes`), else its region's own, its bytes.
            let own = kind_or(piece(&rd.kind, true), "c");
            let region_last = |q: usize| {
                let g = &rd.regions[q];
                match (&rd.region_or[q], &g.or) {
                    (Some(f), Some((_, m2))) => format!("if bit(&m.{f}, c) {{ c + {} }} else {{ c + {} }}", m2 - 1, g.bytes - 1),
                    _ => format!("c + {}", g.bytes - 1),
                }
            };
            let mut closer = String::new();
            for (k, f, b) in &rd.closes {
                let _ = write!(closer, "if bit(&m.{f}, c) {{\n                        (c + {}, {})\n                    }} else ", b - 1, kind_or(piece(k, true), "c"));
            }
            if rd.regions.len() == 1 {
                let _ = write!(closer, "{{\n                        ({}, {own})\n                    }}", region_last(0));
            } else {
                for q in 0..rd.regions.len() {
                    let test = if q + 1 == rd.regions.len() { String::new() } else { format!("if bit(&m.{}, s) ", rd.region_open[q]) };
                    let _ = write!(closer, "{test}{{\n                        // `{}`\n                        ({}, {own})\n                    }}{}", rd.regions[q].kind, region_last(q), if test.is_empty() { "" } else { " else " });
                }
            }
            let regions: Vec<&str> = rd.regions.iter().map(|g| g.kind.as_str()).collect();
            // Its closer at a template (decision 0600): a string open over the template's code;
            // a read that resumes one and closes at its quote, that string's end.
            let resumes = rd.regions.iter().all(|g| g.resumes);
            let resumed = |g: &::bitlex::syntax::Region| g.resumes || lexer_regions.iter().any(|q| q.resumes && q.after.contains(&g.kind));
            let mut templates: Vec<&String> = rd.regions.iter().zip(&rd.region_or).filter(|(g, _)| resumed(g)).filter_map(|(_, f)| f.as_ref()).collect();
            templates.dedup();
            let strings = if templates.is_empty() {
                String::new()
            } else {
                let t = templates.iter().map(|f| format!("bit(&m.{f}, c)")).collect::<Vec<_>>().join(" || ");
                if resumes {
                    format!("                    if !({t}) {{\n                        self.strings = self.strings.saturating_sub(1);\n                    }}\n")
                } else {
                    format!("                    if {t} {{\n                        self.strings += 1;\n                    }}\n")
                }
            };
            let (name, doc, head) = if resumes {
                // Its opener's kind: its own row's.
                let rows = self.x.lexer.iter().filter(|s| matches!(s, ::bitlex::syntax::Stmt::Token { .. }));
                let own = rows.enumerate().find(|(_, s)| matches!(s, ::bitlex::syntax::Stmt::Token { kind, .. } if *kind == rd.kind)).and_then(|(i, _)| self.x.row_kinds.get(i).copied().flatten());
                let _ = write!(resume, "        if self.strings > 0 && let Some(p) = first_bit(&self.m.{}, from).filter(|&p| p < (if self.i + 1 < self.n {{ self.recs[self.i + 1].start }} else {{ u32::MAX }}).min(self.next_x) as usize) {{\n            self.known = 0;\n            return self.resume_{r}(p);\n        }}\n", rd.open);
                (format!("resume_{r}"), "at byte `s`, no record of its own: the string's parts after its template's `}`", format!("s: usize) -> R {{\n        self.sub.clear();\n        self.k = 0;\n        self.piece(s, s, {});", kind_or(own, "s")))
            } else {
                (format!("split_{r}"), "record `j`", "j: usize) -> R {\n        let s = self.recs[j].start as usize;\n        self.sub.clear();\n        self.k = 0;\n        self.sub.push((s as u32, s as u32, self.recs[j].code));".to_owned())
            };
            let _ = write!(
                out,
                "    /// `{kind}`'s region (`read {regions} parts`), {doc}: its opener's byte, each part, each run of the bytes between, its closer.\n    fn {name}(&mut self, {head}\n        let m = self.m;\n        let mut at = s + 1;\n        loop {{\n            let Some(c) = first_bit(&m.{close}, at) else {{\n                return self.refuse();\n            }};\n            // The part first after `at`, before the closer: the earliest, the shortest at one byte.\n            let mut part: Option<(usize, u8, usize)> = None;\n{parts}            let to = part.map_or(c, |q| q.0);\n            if to > at {{\n                self.piece(at, to - 1, {content});\n            }}\n            match part {{\n                Some((e, _, i)) => {{\n                    let (l, kind) = match i {{\n{lasts}                        _ => unreachable!(),\n                    }};\n                    self.piece(e, l, kind);\n                    at = l + 1;\n                }}\n                None => {{\n                    let (l, kind) = {closer};\n                    self.piece(c, l, kind);\n{strings}                    return Ok(());\n                }}\n            }}\n        }}\n    }}\n\n",
                kind = rd.kind,
                regions = regions.join(" | "),
                close = rd.close
            );
        }
        let _ = write!(out, "    /// Past the token, to the next grammar token's record{}.\n    #[inline(always)]\n    fn advance(&mut self, from: usize) -> R {{\n{resume}        // The next token, the comments before it passed.\n        self.i += 1;\n        self.pass_comments();\n        self.arrive()\n    }}\n\n", if resume.is_empty() { "" } else { ", or the string's parts a template's `}` resumes, at the record before the next (decision 0600)" });
        let _ = write!(out, "    /// Record `j` split where its first byte opens a region tier 2 reads in parts.\n    #[inline(always)]\n    fn split(&mut self, j: usize) -> R {{\n        let s = self.recs[j].start as usize;\n{at}        let _ = s;\n        Ok(())\n    }}\n\n");
        // A piece's word: its kind's keywords', as the lean lexer finds a word.
        let mut arms = String::new();
        for (b, name) in self.x.tokens {
            let mut words: Vec<&Vec<u8>> = self.x.keywords.iter().filter(|r| r.0 == *name).flat_map(|r| r.1.iter().map(|(w, _)| w)).collect();
            words.extend(self.x.soft.iter().filter(|r| r.0 == *name).flat_map(|r| r.1.iter()));
            let mut pats = String::new();
            for w in words {
                if let Some(id) = self.word_of(&String::from_utf8_lossy(w)) {
                    let lit: String = w.iter().map(|&c| if c.is_ascii_alphanumeric() || c == b'_' { (c as char).to_string() } else { format!("\\x{c:02x}") }).collect();
                    let _ = writeln!(pats, "            b\"{lit}\" => w::{},", self.word_const(id));
                }
            }
            if !pats.is_empty() {
                let _ = write!(arms, "        {b} => match b {{\n{pats}            _ => 0,\n        }},\n");
            }
        }
        let _ = write!(free, "/// The word of a piece of record kind `kind`, its bytes `b`: its kind's keywords'.\nfn word_in(kind: u32, b: &[u8]) -> u32 {{\n    match kind {{\n{arms}        _ => 0,\n    }}\n}}\n\n");
        let kinds: Vec<String> = self.x.kinds.iter().map(|k| k.to_string()).collect();
        let _ = write!(free, "/// Each byte's record kind, as a token's first byte (`Records::kinds`).\nconst KINDS: [u8; 256] = [{}];\n\n", kinds.join(", "));
        Ok(())
    }

    /// A token kind's test, as the lean parser's `self.kind() == STR`: the record's kind (`kind`, of the code's low
    /// byte) one of its records' kinds (`t`), and its word (`word`) none of its row's hard words.
    /// The words a token of kind `k` can be (`w::…`): its keywords rows' words, soft ones too.
    fn kind_words(&self, k: &str) -> Vec<String> {
        let hard = self.x.keywords.iter().filter(|r| r.0 == k).flat_map(|r| r.1.iter().map(|(w, _)| w));
        let soft = self.x.soft.iter().filter(|r| r.0 == k).flat_map(|r| r.1.iter());
        hard.chain(soft).filter_map(|w| self.word_of(&String::from_utf8_lossy(w))).map(|w| format!("w::{}", self.word_const(w))).collect()
    }

    fn kind_test(&self, k: &str, kind: &str, word: &str) -> String {
        let kinds: Vec<String> = self.x.tokens.iter().filter(|(_, n)| *n == k || n.strip_suffix("_close") == Some(k)).map(|(_, n)| format!("t::{}", upper(n))).collect();
        // A row of keywords only (`required`) names them all: its kind holds at each, as the interpreter's `kind_set`.
        let hard: Vec<String> = self.x.keywords.iter().filter(|r| r.0 == k && !r.2).flat_map(|r| r.1.iter().filter_map(|(w, _)| self.word_of(&String::from_utf8_lossy(w)))).map(|w| format!("w::{}", self.word_const(w))).collect();
        let is = match kinds.as_slice() {
            [] => "false".to_owned(),
            [one] => format!("{kind} == {one}"),
            all => format!("matches!({kind}, {})", all.join(" | ")),
        };
        match hard.as_slice() {
            [] => is,
            [one] => format!("{is} && {word} != {one}"),
            all => format!("{is} && !matches!({word}, {})", all.join(" | ")),
        }
    }

    /// Each reading's function, `reread_R`: the node just made and the nodes inside it read again
    /// (`reread R`), the tape from it written again.
    fn readings(&self, out: &mut String) -> Result<(), String> {
        let mut names: Vec<&&str> = self.readings.keys().collect();
        names.sort();
        for name in names {
            let r = self.readings[*name];
            let mut arms = String::new();
            for (k, what) in &r.kinds {
                let id = (self.x.kind)(k).ok_or_else(|| format!("reading {name}: no kind `{k}`"))?;
                let to = match what {
                    // A `remember` choice's name: its level's word (`REMEMBERED`, `reread_node`).
                    ::bitlex::syntax::ReadKind::As(m) if self.remember.contains(m) => "Some(Some(REMEMBERED))".to_owned(),
                    ::bitlex::syntax::ReadKind::As(m) => format!("Some(Some({}))", (self.x.kind)(m).ok_or_else(|| format!("reading {name}: no kind `{m}`"))?),
                    ::bitlex::syntax::ReadKind::Gone => "Some(None)".to_owned(),
                    ::bitlex::syntax::ReadKind::Keep => "None".to_owned(),
                };
                let _ = writeln!(arms, "            {id} => {to},");
            }
            for e in self.x.extras {
                let id = (self.x.kind)(e).ok_or_else(|| format!("no kind `{e}`"))?;
                let _ = writeln!(arms, "            {id} => None,");
            }
            let mut fields = String::new();
            for (a, b) in &r.fields {
                let (fa, fb) = (self.field_id(a).map_err(|e| format!("reading {name}: {e}"))?, self.field_id(b).map_err(|e| format!("reading {name}: {e}"))?);
                let _ = writeln!(fields, "            {fa} => {fb},");
            }
            let root = match &r.root {
                Some(::bitlex::syntax::ReadKind::As(m)) => format!("Some(Some({}))", (self.x.kind)(m).ok_or_else(|| format!("no kind `{m}`"))?),
                Some(::bitlex::syntax::ReadKind::Gone) => "Some(None)".to_owned(),
                _ => "Some(Some(u16::MAX))".to_owned(),
            };
            let _ = writeln!(
                out,
                "    /// Reading `{name}`: each kind's way (`Some(Some(K))` read as `K`, `Some(None)` gone, `None` kept with what is inside it).\n    fn reading_{n}(kind: u16) -> Option<Option<u16>> {{\n        match kind {{\n{arms}            _ => Some(Some(u16::MAX - 1)),\n        }}\n    }}\n\n    fn reading_field_{n}(field: u16) -> u16 {{\n        match field {{\n{fields}            f => f,\n        }}\n    }}\n\n    fn reread_{n}(&mut self) -> R {{\n        let root: Option<Option<u16>> = {root};\n        self.reread(root, Self::reading_{n}, Self::reading_field_{n})\n    }}\n",
                n = ident(name)
            );
        }
        Ok(())
    }

    fn source(&self) -> Result<String, String> {
        // Every rule's moves, then each printed (`rule`).
        let made = self.order.iter().map(|name| Ok((rule_fn(name), self.rule(name, false)?))).collect::<Result<Vec<_>, String>>()?;
        let made = self.inlined(made);
        let rules: String = self.printed(made).concat();
        let entry = rule_fn(self.order.first().ok_or("no rule")?);
        self.assemble(&rules, &entry, "// Generated by native/crates/compiler/src/bitlex_rust.rs; do not edit. The Bitlex compiler's Rust from the grammar's tree rules.\n\n", false)
    }

    /// The parser around `rules` (its rules' code, `impl P`'s), entered at `entry` (a method of `P`): the runtime, the
    /// marks, the kinds' tests, the readings, the regions' reads, and `parse`, each part written where the rules use
    /// it. `stack`: `P` holds the one loop's stack (`one_loop`'s records form: a level's caller's place and state).
    fn assemble(&self, rules: &str, entry: &str, header: &str, stack: bool) -> Result<String, String> {
        let mut marks = String::new();
        self.marks(&mut marks)?;
        let mut readings = String::new();
        self.readings(&mut readings)?;
        let mut extras = String::new();
        let mut split = String::new();
        self.splits(&mut split, &mut extras)?;
        // An extra's kind: the first extra's, or a later one's by its own bit.
        extras.push('\n');
        let mut kind = String::new();
        for (b, k) in self.x.extra_bits.iter().skip(1) {
            let _ = write!(kind, "    if code >> {} & 1 == 1 {{\n        return k::{};\n    }}\n", 16 + u32::from(*b), self.kind_const(k)?.trim_start_matches("k::"));
        }
        let first = match self.x.extra_bits.first() {
            Some((_, k)) => self.kind_const(k)?,
            None => "0".to_owned(),
        };
        let _ = writeln!(extras, "/// An extra's node kind: a later extra kind's by its bit, else the first's.\nfn extra_kind(code: u32) -> u16 {{\n{kind}    {first}\n}}\n");
        let mut out = String::new();
        out.push_str(header);
        out.push_str("use rerail_tape_format::{Layout, Narrow, Widths, Writer};\n\n");
        out.push_str("/// The tape's kinds, by the grammar's names.\n#[allow(dead_code)]\npub mod k {\n");
        for (c, id) in self.kinds.borrow().iter() {
            let _ = writeln!(out, "    pub const {c}: u16 = {id};");
        }
        out.push_str("}\n\n/// The fields.\n#[allow(dead_code)]\npub mod f {\n");
        for (c, id) in self.fields.borrow().iter() {
            let _ = writeln!(out, "    pub const {c}: u16 = {id};");
        }
        out.push_str("}\n\n/// The records' kinds (a code's low byte), by their tokens' names.\n#[allow(dead_code)]\npub mod t {\n");
        for (b, name) in self.x.tokens {
            let _ = writeln!(out, "    pub const {}: u32 = {b};", upper(name));
        }
        out.push_str("}\n\n/// The words, as a record's code names them.\n#[allow(dead_code)]\npub mod w {\n");
        for i in 0..self.x.words.len() {
            let _ = writeln!(out, "    pub const {}: u32 = {};", self.word_const((i + 1) as u8), i + 1);
        }
        out.push_str("}\n\n");
        // Each choice of tokens' table (`anon_table`): each word's leaf's kind, as the lean parser's `anon()`.
        for (table, words) in self.anon.borrow().iter() {
            let kinds: Vec<String> = (0..=self.x.words.len()).map(|w| words.get(&(w as u8)).cloned().unwrap_or_else(|| "0".to_owned())).collect();
            let what = if table == "ANON" { "a choice of tokens written in a rule".to_owned() } else { format!("`{}`", table.to_ascii_lowercase()) };
            let _ = writeln!(out, "/// A word's leaf's kind where {what} takes it (the lean parser's `anon()`); 0: none of its words.\nconst {table}: [u16; {}] = [{}];\n", kinds.len(), kinds.join(", "));
        }
        for (i, f) in self.flags.iter().enumerate() {
            let _ = writeln!(out, "const FLAG_{}: u32 = 1 << {i};", upper(f));
        }
        out.push_str(&extras);
        out.push_str(RUNTIME);
        // `marked` where a decision reads it (`!self.marked`, `after_mark`).
        if self.marked_read.get() {
            for (a, b) in [
                ("    /// The marks whose questions a level", "    /// A mark taken last, no token or rule's end since (`G::after_mark`).\n    marked: bool,\n    /// The marks whose questions a level"),
                ("asks: 0,", "marked: false, asks: 0,"),
            ] {
                if out.matches(a).count() != 1 {
                    return Err(format!("tier 2 as Rust: `marked`'s place in the runtime: {a:?}"));
                }
                out = out.replace(a, b);
            }
        }
        out.push_str("\n#[allow(clippy::all, unused_parens, dead_code)]\nimpl<L: Layout> P<'_, L> {\n");
        out.push_str(rules);
        out.push_str(&marks);
        out.push_str(&readings);
        out.push_str(&split);
        out.push_str("}\n");
        // The one loop's stack: by each level open, the place its call returns to and the caller's state it saved.
        if stack {
            for (a, b) in [
                ("    strings: usize,\n}\n", "    strings: usize,\n    /// The one loop's stack (`run`): by each level open, its caller's state it saved, then the place it returns to; a\n    /// flag's value before its rule set it.\n    stack: Vec<u64>,\n}\n"),
                ("moved: None, strings: 0 };", "moved: None, strings: 0, stack: Vec::with_capacity(64) };"),
            ] {
                if out.matches(a).count() != 1 {
                    return Err(format!("tier 2 as one loop: the stack's place in the runtime: {a:?}"));
                }
                out = out.replace(a, b);
            }
        }
        // A field's slot past the grammar's ids (`field_id`) must be no field's id the tape names.
        let own = self.own_fields.borrow().len() as u16;
        if own > 0 && self.most_field.get() >= 64 - own {
            return Err(format!("tier 2 as Rust: {own} fields past a key's 6 bits, and field {} among their slots", self.most_field.get()));
        }
        // A grammar's `remember`: each level's word (`REMEMBER`), the kind a reading's `as C` reads its node as.
        if !self.remember.is_empty() {
            for (a, b) in [
                ("    flags: u32,\n    depth: u32,\n", "    flags: u32,\n    /// The level's word (`remember`): the kind of the token of a `remember` choice it took last; a call's is its caller's\n    /// again where it returns.\n    word: u16,\n    depth: u32,\n"),
                ("flags: 0, depth: 0,", "flags: 0, word: 0, depth: 0,"),
                ("            Some(Some(k)) => {\n                let at = self.w.push(key & !0x3ff | u32::from(k), s);", "            Some(Some(k)) => {\n                let k = if k == REMEMBERED { self.word } else { k };\n                let at = self.w.push(key & !0x3ff | u32::from(k), s);"),
            ] {
                if out.matches(a).count() != 1 {
                    return Err(format!("tier 2 as Rust: `remember`'s place in the runtime: {a:?}"));
                }
                out = out.replace(a, b);
            }
            out.push_str("\n/// A reading's way to the level's word (`remember`).\nconst REMEMBERED: u16 = u16::MAX - 2;\n");
        }
        // A grammar's `refuse M in L`: the marks the level open refuses, tested at each token it takes.
        if !self.refusing.is_empty() {
            if self.refusing.values().any(|&b| b == u32::MAX) {
                return Err("tier 2 as Rust: `refuse` of a mark tier 1 gives no bit".into());
            }
            for (a, b) in [
                ("    asks: u64,\n", "    asks: u64,\n    /// The marks the level open refuses (`refuse M in L`), their bits in a record's code: a token it takes with one\n    /// refuses the file. A call's are its own, the caller's again where it returns.\n    refusing: u32,\n"),
                ("asks: 0,", "asks: 0, refusing: 0,"),
            ] {
                if out.matches(a).count() != 1 {
                    return Err(format!("tier 2 as Rust: `refuse`'s place in the runtime: {a:?}"));
                }
                out = out.replace(a, b);
            }
        }
        // A grammar's `drop`: the nodes it took out, held as dead where a reading may reach them (`drop_made`).
        if rules.contains("self.drop_made(") {
            for (a, b) in [
                ("    gone: Vec<usize>,\n", "    gone: Vec<usize>,\n    /// The nodes a `drop` took out, each by the node it was in: a reading that reaches one refuses (`read_dead`).\n    dead: Vec<u32>,\n"),
                ("gone: Vec::new(), lost: None,", "gone: Vec::new(), dead: Vec::new(), lost: None,"),
                ("    /// The node a wrap moved (the wrap's first child) a `kind` now.\n", "    /// The nodes a `drop` took out past a wrap's place `at`, one later: the wrap went in before them.\n    #[cold]\n    #[inline(never)]\n    fn shift_dead(&mut self, at: usize) {\n        // A dropped node no reading can reach again goes: a reading starts at a level's node made last, which only\n        // moves on, so one before every open level's is passed for good (and one a reading took out, `NONE`).\n        let bound = self.levels.iter().chain(std::iter::once(&self.top)).filter(|l| l.1 != NONE).fold(self.w.len() as u32, |b, l| b.min(l.1));\n        self.dead.retain(|&o| o != NONE && o >= bound);\n        for o in &mut self.dead {\n            if *o != NONE && *o as usize >= at {\n                *o += 1;\n            }\n        }\n    }\n\n    /// The node a wrap moved (the wrap's first child) a `kind` now.\n"),
                ("        self.w.wrap(at, 0x3f << 10, u32::from(field) << 10, u32::from(kind));\n", "        self.w.wrap(at, 0x3f << 10, u32::from(field) << 10, u32::from(kind));\n        if !self.dead.is_empty() {\n            self.shift_dead(at);\n        }\n"),
                ("        if n == 1 && root == Some(None) && self.w.key(at) >> 16 & 1 == 0 {\n", "        if n == 1 && root == Some(None) && self.w.key(at) >> 16 & 1 == 0 && !self.dead.contains(&(at as u32)) {\n"),
                ("        let nodes: Vec<(u32, u32, u32, usize)> = (at..at + n).map(|i| (self.w.key(i), self.w.start(i), self.w.end(i), self.w.size(i) as usize)).collect();\n", "        let nodes: Vec<(u32, u32, u32, usize)> = (at..at + n).map(|i| (self.w.key(i), self.w.start(i), self.w.end(i), self.w.size(i) as usize)).collect();\n        if self.dead.iter().any(|&o| (o as usize).wrapping_sub(at) < n) {\n            self.read_dead(&nodes, at, root, way)?;\n        }\n"),
            ] {
                if out.matches(a).count() != 1 {
                    return Err(format!("tier 2 as Rust: `drop`'s place in the runtime: {a:?}"));
                }
                out = out.replace(a, b);
            }
        }
        // A grammar's `recover`: the statement's mark, the cut back, and the resume point (decision 0650).
        if let Some((_, clauses)) = self.recover.filter(|_| rules.contains("self.recover(")) {
            out = self.recovery(out, clauses)?;
        }
        // `leaf K R`: the record its first token is, held while `R` reads (a leaf holds no leaf).
        if rules.contains("self.leaf_open(") {
            for (a, b) in [("    /// The last token's end.\n    last_end: u32,\n", "    /// The last token's end.\n    last_end: u32,\n    /// A `leaf`'s first token's record (`leaf_open`).\n    leaf_from: usize,\n"), ("last_end: 0, ", "last_end: 0, leaf_from: 0, ")] {
                if out.matches(a).count() != 1 {
                    return Err(format!("tier 2 as Rust: `leaf`'s place in the runtime: {a:?}"));
                }
                out = out.replace(a, b);
            }
        }
        // The helpers only some grammars' rules call, each written where it is called.
        // A leaf's refused marks (`leaf_refuses`): one set, the grammar's, else no check.
        let used: Vec<String> = MORE
            .iter()
            .filter(|(name, _)| rules.contains(&format!("self.{name}(")))
            .map(|(_, code)| code.to_string())
            .collect();
        if !used.is_empty() {
            out.push_str("\n#[allow(clippy::all, dead_code)]\nimpl<L: Layout> P<'_, L> {\n");
            out.push_str(&used.join("\n"));
            out.push_str("}\n");
        }
        // No mark drops or loses an extra (`drops`, `loses`, decision 0623 step 7): tier 2 keeps no extra gone or
        // lost, and a taken token checks none.
        if ["self.drop_pending()", "self.lose_pending()", "self.move_first()"].iter().all(|c| !out.contains(c)) {
            for (a, b) in [
                ("    /// The extras a mark's question dropped or lost (`drops`, `loses`), by comment; the one lost,\n    /// its kind and, once its token is taken, that token's start: a pending extra of no width.\n    gone: Vec<usize>,\n", ""),
                ("    lost: Option<(u16, Option<u32>)>,\n    /// The extra a mark's `loses since` moved, by comment, and where: of no width there.\n    moved: Option<(usize, u32)>,\n", ""),
                ("gone: Vec::new(), ", ""),
                ("lost: None, moved: None, ", ""),
                ("        if let Some((kind, Some(s))) = self.lost {\n            self.lost = None;\n            let at = self.w.push(u32::from(kind) | 1 << 16, s);\n            self.w.close(at, s);\n        }\n", ""),
                ("        // An extra a `loses since` moved and the one after it go with those before: the lean\n        // lexer's semicolon is past that one.\n        let moved = self.moved.map(|(m, _)| m).filter(|&m| m >= self.pend && m < self.xi);\n        let through = moved.and_then(|m| (m + 1..self.xi).find(|k| !self.gone.contains(k)));\n", ""),
                (" && Some(j) != moved && Some(j) != through {", " {"),
                ("            if !self.gone.contains(&j) {\n                let (s, e) = self.extra_span(j);\n                self.w.push_leaf(u32::from(extra_kind(self.xs[j].code)) | 1 << 16, s, e);\n            }\n", "            let (s, e) = (self.xs[j].start, self.xs[j].last + 1);\n            self.w.push_leaf(u32::from(extra_kind(self.xs[j].code)) | 1 << 16, s, e);\n"),
                ("            if !self.gone.contains(&j) {\n                let (s, e) = self.extra_span(j);\n                self.w.push_leaf(u32::from(extra_kind(c)) | 1 << 16, s, e);\n                self.last_end = self.last_end.max(e);\n            }\n", "            let (s, e) = (self.xs[j].start, self.xs[j].last + 1);\n            self.w.push_leaf(u32::from(extra_kind(c)) | 1 << 16, s, e);\n            self.last_end = self.last_end.max(e);\n"),
                ("    /// Extra `j`'s span: its bytes, or of no width where a mark's `loses since` moved it.\n    fn extra_span(&self, j: usize) -> (u32, u32) {\n        match self.moved {\n            Some((m, s)) if m == j => (s, s),\n            _ => (self.xs[j].start, self.xs[j].last + 1),\n        }\n    }\n\n", ""),
                ("    /// `loses since`: the first extra pending of no width at the next one's start (the lean\n    /// lexer's `asi` reads a block comment through to another comment and loses its bytes there).\n    fn move_first(&mut self) {\n        let mut pending = (self.pend..self.xi).filter(|j| !self.gone.contains(j));\n        if let (Some(a), Some(b)) = (pending.next(), pending.next()) {\n            self.moved = Some((a, self.xs[b].start));\n        }\n    }\n\n", ""),
                ("    /// The extra pending last before the token (not yet written, not dropped or lost).\n    fn pending_last(&self) -> Option<usize> {\n        (self.pend..self.xi).rev().find(|j| !self.gone.contains(j))\n    }\n\n    /// `drops`: the extra pending last gone; with none left pending, the nodes closed next end\n    /// at the token, where the semicolon of no width is (the lean lexer's `Asi::Swallow`).\n    fn drop_pending(&mut self) {\n        if let Some(j) = self.pending_last() {\n            self.gone.push(j);\n            if self.pending_last().is_none() {\n                self.last_end = self.start();\n            }\n        }\n    }\n\n    /// `loses`: the extra pending last made again after the token is taken, of no width at its\n    /// start (the lean lexer's `asi` loses it); once a token.\n    fn lose_pending(&mut self) {\n        if self.lost.is_some() {\n            return;\n        }\n        if let Some(j) = self.pending_last() {\n            self.gone.push(j);\n            self.lost = Some((extra_kind(self.xs[j].code), None));\n        }\n    }\n\n", ""),
            ] {
                if !out.contains(a) {
                    return Err(format!("tier 2: the runtime's lost extras, not where they were: {a:?}"));
                }
                out = out.replace(a, b);
            }
        }
        let _ = stack;
        // No region read in parts (decision 0619, as 0586's): tier 1 gives no mask, and tier 2 holds none, splits no
        // record, and writes only the helpers its code calls.
        if crate::bitlex_lex_rust::readers(self.x.lexer).is_empty() {
            for (a, b) in [
                ("    /// The pieces of record `i` where tier 2 splits it (a string, decision 0323's lowering B), as\n    /// the lean parser's string arm reads its pieces: each one's first and last byte and code;\n    /// the token is piece `k`.\n    sub: Vec<(u32, u32, u32)>,\n    k: usize,\n", ""),
                ("sub: Vec::new(), k: 0, ", ""),
                ("    /// A piece of the record split, `s..=l`, of record kind `kind` and the word its bytes are.\n    fn piece(&mut self, s: usize, l: usize, kind: u32) {\n        let word = word_in(kind, &self.src[s..=l]);\n        self.sub.push((s as u32, l as u32, kind | word << 8));\n    }\n\n", ""),
                ("    /// Record `j`'s code, its first and its last byte: the piece's at the token.\n    #[inline(always)]\n    fn code_at(&self, j: usize) -> u32 {\n        if j == self.i && self.k < self.sub.len() { self.sub[self.k].2 } else { self.recs[j].code }\n    }\n", "    /// Record `j`'s code.\n    #[inline(always)]\n    fn code_at(&self, j: usize) -> u32 {\n        self.recs[j].code\n    }\n"),
                ("        } else if self.k < self.sub.len() {\n            self.sub[self.k].0\n", ""),
                ("    /// Tier 1's masks, a bit a byte: those a region's parts are found by (`split`).\n    m: &'a crate::tier1::Masks,\n", ""),
                ("let (recs, xs, m) = (&lx.recs[..], &lx.extras[..], &lx.masks);", "let (recs, xs) = (&lx.recs[..], &lx.extras[..]);"),
                ("P { src, recs, xs, m, n,", "P { src, recs, xs, n,"),
                ("    /// At record `i`: split it where tier 2 reads its region in parts.\n", "    /// At record `i`: its code read, no mark known there yet.\n"),
                ("        if self.i < self.n {\n            self.split(self.i)?;\n        }\n", "        self.cur = if self.i < self.n { self.recs[self.i].code } else { 0 };\n"),
                // The token's code held as the token is reached (a token is its record), as the lean parser holds its
                // token: its word read of it.
                ("    known: u64,\n    holds: u64,\n", "    known: u64,\n    holds: u64,\n    /// The token's code (none past the last record), read as the token is reached.\n    cur: u32,\n"),
                ("known: 0, holds: 0, ", "known: 0, holds: 0, cur: 0, "),
                ("    fn word(&self) -> u32 {\n        self.word_at(self.i)\n    }\n", "    fn word(&self) -> u32 {\n        self.cur >> 8 & 0xff\n    }\n"),
                ("    /// Record `j` split where its first byte opens a region tier 2 reads in parts.\n    #[inline(always)]\n    fn split(&mut self, j: usize) -> R {\n        let s = self.recs[j].start as usize;\n        let _ = s;\n        Ok(())\n    }\n\n", ""),
            ] {
                if !out.contains(a) {
                    return Err(format!("tier 2: the runtime's masks, not where they were: {a:?}"));
                }
                out = out.replacen(a, b, 1);
            }
            // A record's code where it is in the file: one test of the records' own length, not `n`'s and then the
            // slice's (a token's code is its record's here).
            out = out.replace("        if j >= self.n {\n            return false;\n        }\n        let c = self.code_at(j);\n", "        let Some(c) = self.recs.get(j).map(|r| r.code) else {\n            return false;\n        };\n");
            out = out.replace("        if j < self.n { self.code_at(j) >> 8 & 0xff } else { 0 }\n", "        self.recs.get(j).map_or(0, |r| r.code >> 8 & 0xff)\n");
            out = out.replace("        if j < self.n { self.code_at(j) & 0xff } else { NO_KIND }\n", "        self.recs.get(j).map_or(NO_KIND, |r| r.code & 0xff)\n");
            let parts: Vec<String> = RUNTIME.split("\n\n").map(|p| p.trim_matches('\n').to_owned()).filter(|p| !p.is_empty() && p.matches('{').count() == p.matches('}').count()).collect();
            out = one_loop::drop_uncalled(out, &parts);
        }
        // The marks' bits where no mark sets or reads them (every mark a peek, as the lean parser's): no field.
        for f in ["taken", "known", "holds"] {
            let reset = format!("self.{f} = 0;");
            if out.matches(&format!("self.{f}")).count() == out.lines().filter(|l| l.trim() == reset).count() {
                out = no_field(&out, f);
                out = out.replace("; the next token's marks not yet taken.", ".").replace(", no mark known there yet.", ".");
            }
        }
        let masks = if crate::bitlex_lex_rust::readers(self.x.lexer).is_empty() { "" } else { " and masks (those a region's parts are found by)" };
        let _ = writeln!(out, "\n/// The tape of `src` from tier 1's records (each token's first and last bytes and code){masks}, finished by `finish` (the\n/// image, or the columns, as the grammar's lean parser finishes), or the byte it was refused at.\npub fn parse<F: rerail_tape_format::Finish>(src: &[u8], lx: &crate::tier1::Lexed, finish: &mut F) -> Result<F::Out, u32> {{\n    // Where tier 1 sends the file to tree-sitter.\n    if lx.refused {{\n        return Err(0);\n    }}\n    let mut widths = Widths::guess(src.len());\n    loop {{\n        // Nearly every file is parsed at the narrow widths, fixed in the parser's code, as the lean parser's.\n        let at = if widths == Narrow::WIDTHS {{ parse_at(src, lx, Narrow, finish)? }} else {{ parse_at(src, lx, widths, finish)? }};\n        match at {{\n            Ok(c) => return Ok(c),\n            Err(rerail_tape_format::Redo(w)) => {{\n                if w == widths {{\n                    return Err(u32::MAX);\n                }}\n                widths = w;\n            }}\n        }}\n    }}\n}}\n\n/// `parse` at `layout`'s widths: the tape `finish` makes of the writer, or the widths to parse at again; or the byte\n/// it was refused at.\nfn parse_at<L: Layout, F: rerail_tape_format::Finish>(src: &[u8], lx: &crate::tier1::Lexed, layout: L, finish: &mut F) -> R<Result<F::Out, rerail_tape_format::Redo>> {{\n    let mut p = P::new(src, lx, layout)?;\n    p.{entry}()?;\n    Ok(finish.finish(p.w))\n}}");
        let (take, takes) = self.take_helpers(&out);
        if out.matches("@TAKE\n").count() != 1 || out.matches("@TAKES\n").count() != 1 {
            return Err("tier 2 as Rust: the takes' place in the runtime".into());
        }
        out = out.replace("@TAKE\n", &take).replace("@TAKES\n", &takes);
        Ok(plain_writer(&no_dead_methods(&out)))
    }
}

/// The rules `g` calls: its symbols, and the rule each `leaf` reads (marks and tokens too; the caller keeps the rules).
fn called(g: &Gram, out: &mut Vec<String>) {
    match g {
        Gram::Sym(n) | Gram::Leaf { rule: n, .. } => out.push(n.clone()),
        Gram::Seq(v) | Gram::Alt(v) => v.iter().for_each(|x| called(x, out)),
        Gram::Opt(a) | Gram::Star(a) | Gram::Plus(a) | Gram::Field(_, a) | Gram::Retag(_, a) | Gram::Merge(_, a) | Gram::As(_, a) => called(a, out),
        Gram::Wrap { body, .. } | Gram::Relabel { body, .. } | Gram::Fold { body, .. } | Gram::Reread { body, .. } | Gram::Flag { body, .. } | Gram::Unless { body, .. } => called(body, out),
        _ => {}
    }
}

/// `code` with no method nothing calls: those the runtime gives every grammar (`settle`, `open_mark`, …) where its code
/// no longer calls them, whatever replacement wrote them, as the lean parser has no helper it does not use.
fn no_dead_methods(code: &str) -> String {
    let mut out = code.to_owned();
    loop {
        let chunks: Vec<String> = out.split("\n\n").map(str::to_owned).collect();
        let mut dropped = false;
        for c in &chunks {
            // One method, its docs and attributes before it, braces balanced.
            let Some(name) = c.lines().find_map(|l| l.strip_prefix("    fn ")).and_then(|l| l.split(['(', '<']).next()) else { continue };
            if c.lines().filter(|l| l.starts_with("    fn ")).count() != 1 || c.matches('{').count() != c.matches('}').count() || !c.lines().all(|l| l.starts_with("    ")) {
                continue;
            }
            let Some(at) = out.find(&format!("\n\n{c}\n")) else { continue };
            let without = format!("{}{}", &out[..at + 1], &out[at + 2 + c.len()..]);
            let called = without.lines().filter(|l| !l.trim_start().starts_with("//")).any(|l| [format!("{name}("), format!("{name}::<"), format!("{name},"), format!("{name})"), format!("= {name}")].iter().any(|x| l.contains(x.as_str())));
            if !called {
                out = without;
                dropped = true;
                break;
            }
        }
        if !dropped {
            return out;
        }
    }
}

/// `code` with its writer's calls the plain ones (`Writer::plain_key` and the rest): tier 2 opens no chain and writes an
/// image or the columns, never format 16, so no call need check for a chain held or a node passed on (the writer
/// asserts both in a test build).
fn plain_writer(code: &str) -> String {
    [
        ("self.w.key(", "self.w.plain_key("),
        ("self.w.start(", "self.w.plain_start("),
        ("self.w.set_key(", "self.w.plain_set_key("),
        ("self.w.set_start(", "self.w.plain_set_start("),
        ("self.w.close(", "self.w.plain_close("),
        ("self.w.push_leaf(", "self.w.plain_leaf("),
        ("self.w.set_field(", "self.w.plain_field("),
        ("self.w.close_from(", "self.w.plain_close_from("),
        ("self.w.wrap(", "self.w.plain_wrap("),
        ("self.w.size(", "self.w.plain_size("),
        ("self.w.end(", "self.w.plain_end("),
        ("self.w.truncate(", "self.w.plain_truncate("),
    ]
    .iter()
    .fold(code.to_owned(), |c, (a, b)| c.replace(a, b))
}

/// `code` without field `f` of the parser (a `u64` no code reads): its declaration and the comment above it, its
/// start, and the lines that clear it.
fn no_field(code: &str, f: &str) -> String {
    let (decl, reset) = (format!("{f}: u64,"), format!("self.{f} = 0;"));
    let mut out: Vec<&str> = Vec::new();
    for l in code.lines() {
        if l.trim() == decl {
            while out.last().is_some_and(|x| x.trim_start().starts_with("///")) {
                out.pop();
            }
            continue;
        }
        if l.trim() != reset {
            out.push(l);
        }
    }
    (out.join("\n") + "\n").replace(&format!("{f}: 0, "), "")
}

/// The fewest lines of a loop the rules repeat that is a function of its own (`G::repeated`).
const REPEATED_LINES: usize = 6;

/// A method's block: its lines around one block of moves (`Op::At`), none of them a move of the tree's or a take; where
/// that block is.
/// A method's body (`method_body`), or a node's: its header opens the node (`self.open(K);`) and its footer closes it;
/// where its body is, and the node's kind.
fn node_method(b: &Block) -> Option<(usize, Option<String>)> {
    if let Some(at) = method_body(b) {
        return Some((at, None));
    }
    let at = b.iter().position(|o| matches!(o, Op::At(..)))?;
    let [Op::Line(head), Op::At(..), Op::Line(close), Op::Line(tail)] = b.as_slice() else { return None };
    let mut lines = head.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with("///"));
    let (Some(fn_line), Some(open), None) = (lines.next(), lines.next(), lines.next()) else { return None };
    let kind = open.strip_prefix("self.open(")?.strip_suffix(");")?;
    let plain_tail = tail.lines().map(str::trim).all(|l| l.is_empty() || matches!(l, "Ok(())" | "}"));
    (fn_line.starts_with("fn ") && close.trim() == "self.close()?;" && plain_tail && kind.starts_with("k::")).then(|| (at, Some(kind.to_owned())))
}

fn method_body(b: &Block) -> Option<usize> {
    let at = b.iter().position(|o| matches!(o, Op::At(..)))?;
    let plain = b.iter().enumerate().all(|(i, o)| match o {
        Op::Line(l) => l.lines().all(|l| {
            let t = l.trim();
            t.is_empty() || t.starts_with("///") || t.starts_with("fn ") || t.starts_with("#[") || matches!(t, "self.deeper()?;" | "self.depth -= 1;" | "Ok(())" | "}")
        }),
        _ => i == at,
    });
    plain.then_some(at)
}

/// The fewest lines of a run the rules repeat but for its calls and fields that is a function of its own (`G::lists`).
const LIST_LINES: usize = 4;

/// The most lines a rule's code written in its one caller has (`G::inlined`).
const INLINE_LINES: usize = 20;

/// The parser's helpers that only some grammars' rules call (`relabel`, `drop`), by name: each written into a parser
/// whose rules call it.
const MORE: &[(&str, &str)] = &[
    (
        "leaf_open",
        r#"    /// `leaf K R` starts: the comments before it written, its first token's record kept (`leaf_from`).
    fn leaf_open(&mut self) {
        self.flush();
        self.leaf_from = self.i;
    }

    /// `leaf K R`'s leaf, after `R` read the tokens: one leaf of `kind` from the first token's start to the last's end,
    /// the comments between them its text (as a merge's), those after it still pending.
    fn leaf_close(&mut self, kind: u16) -> R {
        while self.pend < self.xi && self.xs[self.pend].start < self.last_end {
            self.pend += 1;
        }
        let at = self.w.plain_leaf(u32::from(kind), self.recs[self.leaf_from].start, self.last_end);
        self.set_made(at as u32);
        Ok(())
    }
"#,
    ),
    (
        "leaf_refuses",
        r#"    /// The marks a `leaf`'s rule refuses (`refuse M in L`), at any of the tokens it read: its text checked, as the
    /// lean parser checks a policy document is JSON.
    fn leaf_refuses(&self, marks: u32) -> R {
        if (self.leaf_from..self.i).any(|j| self.marks_hold(j, marks)) {
            return self.refuse();
        }
        Ok(())
    }
"#,
    ),
    (
        "made_kind",
        r#"    /// `ending … K refused`: the kind of the node made last at this level, `NONE` where it made none.
    fn made_kind(&self) -> u16 {
        let at = self.top.1;
        if at == NONE { NONE as u16 } else { (self.w.key(at as usize) & 0x3ff) as u16 }
    }
"#,
    ),
    (
        "made_is",
        r#"    /// `ending … K`: the node made last at this level is a `kind`.
    fn made_is(&self, kind: u16) -> bool {
        let at = self.top.1;
        at != NONE && self.w.key(at as usize) & 0x3ff == u32::from(kind)
    }
"#,
    ),
    (
        "relabel_inner",
        r#"    /// `relabel … inner G`: the node the node made last at this level, a wrap, holds first takes field `G`.
    fn relabel_inner(&mut self, field: u16) -> R {
        let at = self.top.1;
        if at == NONE || self.w.size(at as usize) < 2 {
            return self.refuse();
        }
        let at = at as usize + 1;
        let k = self.w.key(at);
        self.w.set_key(at, k & !(0x3f << 10) | u32::from(field) << 10);
        Ok(())
    }
"#,
    ),
    (
        "relabel_field",
        r#"    /// `relabel F`: the node made last at this level takes field `F`.
    fn relabel_field(&mut self, field: u16) -> R {
        let at = self.top.1;
        if at == NONE {
            return self.refuse();
        }
        let k = self.w.key(at as usize);
        self.w.set_key(at as usize, k & !(0x3f << 10) | u32::from(field) << 10);
        Ok(())
    }
"#,
    ),
    (
        "relabel_kind",
        r#"    /// `relabel … as M`: the node made last at this level a `kind` now.
    fn relabel_kind(&mut self, kind: u16) -> R {
        let at = self.top.1;
        if at == NONE {
            return self.refuse();
        }
        let k = self.w.key(at as usize);
        self.w.set_key(at as usize, k & !0x3ff | u32::from(kind));
        Ok(())
    }
"#,
    ),
    (
        "drop_made",
        r#"    /// `drop`: the node made last at this level goes, the nodes inside it in its place; none is made last at this
    /// level until the next.
    ///
    /// As the machine's `drop` leaves its record of kind `DEAD` in the table (left out at `ACCEPT`), the node is held
    /// as dead in the node open (`dead`), so that a reading that reaches it refuses. A dead node in it goes to that
    /// node too.
    fn drop_made(&mut self) -> R {
        let at = self.top.1;
        let open = self.top.0;
        for o in &mut self.dead {
            if *o == at {
                *o = open;
            }
        }
        self.reread(Some(None), |_| None, |f| f)?;
        // Held once: a node open held dead again is held as before (a drop at each of a list's separators).
        if !self.dead.contains(&open) {
            self.dead.push(open);
        }
        self.set_made(NONE);
        Ok(())
    }

    /// The dead nodes (`drop`) in the node read again from `at` (`nodes`, as `reread` has them): one in a node the
    /// reading reaches, not one it keeps whole, refuses, as the machine's reading refuses the `DEAD` record it
    /// reaches (it names no such kind). Each other one is then in its node where the reading writes it.
    fn read_dead(&mut self, nodes: &[(u32, u32, u32, usize)], at: usize, root: Option<Option<u16>>, way: fn(u16) -> Option<Option<u16>>) -> R {
        let (mut to, mut reached) = (vec![NONE; nodes.len()], vec![false; nodes.len()]);
        let (mut next, mut kept) = (at as u32, 0);
        for (i, &(key, _, _, size)) in nodes.iter().enumerate() {
            let way = if i < kept || key >> 16 & 1 == 1 {
                None
            } else if i == 0 {
                match root {
                    Some(Some(u16::MAX)) => way((key & 0x3ff) as u16),
                    r => r,
                }
            } else {
                way((key & 0x3ff) as u16)
            };
            match way {
                None => kept = kept.max(i + size),
                Some(_) => reached[i] = true,
            }
            if way != Some(None) {
                to[i] = next;
                next += 1;
            }
        }
        for o in &mut self.dead {
            let j = (*o as usize).wrapping_sub(at);
            if j < nodes.len() {
                if reached[j] {
                    return self.refuse();
                }
                *o = to[j];
            }
        }
        Ok(())
    }
"#,
    ),
];

/// The parser's own code: the tokens, the tree, the comments.
const RUNTIME: &str = r#"
type R<X = ()> = Result<X, u32>;

/// Mask `x`'s first bit at or after byte `from`, a word at a time.
fn first_bit(x: &[u64], from: usize) -> Option<usize> {
    let mut q = from / 64;
    let mut w = x.get(q)? & (!0u64 << (from % 64));
    while w == 0 {
        q += 1;
        w = *x.get(q)?;
    }
    Some(64 * q + w.trailing_zeros() as usize)
}

/// Whether mask `x` holds at byte `i`.
fn bit(x: &[u64], i: usize) -> bool {
    x.get(i / 64).is_some_and(|w| w >> (i % 64) & 1 == 1)
}

/// No node made yet at a level.
const NONE: u32 = u32::MAX;
/// No record's kind: past the last record (`kind_at`).
const NO_KIND: u32 = u32::MAX;
/// The kind of a node open whose kind its rule gives later (`retag`).
const UNSET: u16 = 0x3ff;
/// The deepest the calls go.
const DEPTH: u32 = 400;

/// The parser: tier 1's records (the grammar tokens), its comments apart, the token read next, the tree.
pub struct P<'a, L: Layout> {
    src: &'a [u8],
    /// Tier 1's records, one a token: its first and last byte and its code (the lean parser's `Tok`).
    recs: &'a [crate::tier1::Rec],
    /// The comments (the extras), apart from the tokens, as the lean parser's `comments`.
    xs: &'a [crate::tier1::Rec],
    /// Tier 1's masks, a bit a byte: those a region's parts are found by (`split`).
    m: &'a crate::tier1::Masks,
    n: usize,
    /// The next grammar token's record.
    i: usize,
    /// The pieces of record `i` where tier 2 splits it (a string, decision 0323's lowering B), as
    /// the lean parser's string arm reads its pieces: each one's first and last byte and code;
    /// the token is piece `k`.
    sub: Vec<(u32, u32, u32)>,
    k: usize,
    /// The comments: the first not yet written (`pend`), the first after the token's start (`xi`), and where that one
    /// starts (`next_x`, `u32::MAX` past the last), as the lean parser's `comment` and `next_comment`.
    pend: usize,
    xi: usize,
    next_x: u32,
    w: Writer<L>,
    /// The level open: its node, and the node made last in it, held as the lean parser holds its `top`; the levels
    /// around it on `levels` (one push and one pop a level).
    top: (u32, u32),
    levels: Vec<(u32, u32)>,
    /// The last token's end.
    last_end: u32,
    /// The marks taken or passed at this token, by their order.
    taken: u64,
    /// The marks found at this token (`held`): which, and whether each holds.
    known: u64,
    holds: u64,
    flags: u32,
    depth: u32,
    /// The marks whose questions a level that may end at them asks (`ask_M`), by order: those
    /// of the caller's level, as the lean parser's `stmt`.
    asks: u64,
    /// The extras a mark's question dropped or lost (`drops`, `loses`), by comment; the one lost,
    /// its kind and, once its token is taken, that token's start: a pending extra of no width.
    gone: Vec<usize>,
    lost: Option<(u16, Option<u32>)>,
    /// The extra a mark's `loses since` moved, by comment, and where: of no width there.
    moved: Option<(usize, u32)>,
    /// The strings open over a template's code (decision 0600): their parts go on at the next
    /// opener of a reader that resumes them (`advance`).
    strings: usize,
}

#[allow(dead_code)]
impl<'a, L: Layout> P<'a, L> {
    fn new(src: &'a [u8], lx: &'a crate::tier1::Lexed, widths: L) -> R<Self> {
        let (recs, xs, m) = (&lx.recs[..], &lx.extras[..], &lx.masks);
        let n = recs.len();
        // Room for each token's node and half as many again, and each comment's, as the lean parser's writer.
        let mut p = P { src, recs, xs, m, n, i: 0, sub: Vec::new(), k: 0, pend: 0, xi: 0, next_x: xs.first().map_or(u32::MAX, |x| x.start), w: Writer::new(0, widths, n + n / 2 + xs.len() + 8), top: (NONE, NONE), levels: Vec::with_capacity(64), last_end: 0, taken: 0, known: 0, holds: 0, flags: 0, depth: 0, asks: 0, gone: Vec::new(), lost: None, moved: None, strings: 0 };
        p.pass_comments();
        p.arrive()?;
        Ok(p)
    }

    /// At record `i`: split it where tier 2 reads its region in parts.
    #[inline(always)]
    fn arrive(&mut self) -> R {
        self.known = 0;
        if self.i < self.n {
            self.split(self.i)?;
        }
        Ok(())
    }

    /// A piece of the record split, `s..=l`, of record kind `kind` and the word its bytes are.
    fn piece(&mut self, s: usize, l: usize, kind: u32) {
        let word = word_in(kind, &self.src[s..=l]);
        self.sub.push((s as u32, l as u32, kind | word << 8));
    }

    /// Record `j`'s code, its first and its last byte: the piece's at the token.
    #[inline(always)]
    fn code_at(&self, j: usize) -> u32 {
        if j == self.i && self.k < self.sub.len() { self.sub[self.k].2 } else { self.recs[j].code }
    }

    /// Whether mark `order` holds at the token (`h`), found once a token, as the lean parser
    /// keeps its `asi`.
    #[inline(always)]
    fn held(&mut self, order: u32, h: fn(&Self) -> bool) -> bool {
        if self.known >> order & 1 == 0 {
            self.known |= 1 << order;
            let v = h(self);
            self.holds = self.holds & !(1 << order) | u64::from(v) << order;
        }
        self.holds >> order & 1 == 1
    }

    /// The comments before the token passed (`xi`), as the lean parser's comment cursor moves on.
    #[inline(always)]
    fn pass_comments(&mut self) {
        let to = if self.i < self.n { self.recs[self.i].start } else { u32::MAX };
        while self.next_x < to {
            self.xi += 1;
            self.next_x = self.xs.get(self.xi).map_or(u32::MAX, |x| x.start);
        }
    }

    /// The grammar token before token `j` (`n`: none).
    #[inline(always)]
    fn prev_tok(&self, j: usize) -> usize {
        if j == 0 { self.n } else { j - 1 }
    }

    /// A record: comment `j` where `x`, else token `j`.
    #[inline(always)]
    fn record(&self, x: bool, j: usize) -> crate::tier1::Rec {
        if x { self.xs[j] } else { self.recs[j] }
    }

    /// The `k`th record before the token (`here`, the token `i`), the comments among them by where they start: whether a
    /// comment, and its index; none before the file's first.
    fn back(&self, here: usize, k: usize) -> Option<(bool, usize)> {
        debug_assert_eq!(here, self.i);
        let (mut t, mut x) = (here, self.xi);
        let mut got = None;
        for _ in 0..k {
            got = if x > 0 && (t == 0 || self.xs[x - 1].start > self.recs[t - 1].start) {
                x -= 1;
                Some((true, x))
            } else if t > 0 {
                t -= 1;
                Some((false, t))
            } else {
                return None;
            };
        }
        got
    }

    /// The `k`th record from the token on (`here`, the token `i`; 0 the token itself), the comments among them by where
    /// they start; none past the last.
    fn ahead(&self, here: usize, k: usize) -> Option<(bool, usize)> {
        debug_assert_eq!(here, self.i);
        if here >= self.n {
            return None;
        }
        let (mut t, mut x) = (here, self.xi);
        let mut got = (false, here);
        for _ in 0..k {
            let next = if t + 1 < self.n { self.recs[t + 1].start } else { u32::MAX };
            got = if x < self.xs.len() && self.xs[x].start < next {
                x += 1;
                (true, x - 1)
            } else if t + 1 < self.n {
                t += 1;
                (false, t)
            } else {
                return None;
            };
        }
        Some(got)
    }

    /// The first comment since the grammar token before the token (`here`, the token `i`): the comments between.
    fn since_comments(&self, here: usize) -> usize {
        debug_assert_eq!(here, self.i);
        if here == 0 {
            return 0;
        }
        let mut x = self.xi;
        while x > 0 && self.xs[x - 1].start > self.recs[here - 1].start {
            x -= 1;
        }
        x
    }

    /// Whether one of the marks `bits` holds at a record (`rec`): a comment's are the marks it keeps of its own gap.
    fn rec_marks_hold(&self, x: bool, j: usize, bits: u32) -> bool {
        if x { self.xs[j].code >> 16 & bits != 0 } else { self.marks_hold(j, bits) }
    }

    #[inline(always)]
    fn word(&self) -> u32 {
        self.word_at(self.i)
    }

    #[inline(always)]
    fn word_at(&self, j: usize) -> u32 {
        if j < self.n { self.code_at(j) >> 8 & 0xff } else { 0 }
    }

    /// The token's record kind (`t`), as the lean parser's `kind()`; none past the last record.
    #[inline(always)]
    fn kind(&self) -> u32 {
        self.kind_at(self.i)
    }

    #[inline(always)]
    fn kind_at(&self, j: usize) -> u32 {
        if j < self.n { self.code_at(j) & 0xff } else { NO_KIND }
    }

    /// Mark `b` at record `j`: its code's bit, else, for a gap mark tier 1 leaves to tier 2 (`tier1::ASKED`), the bytes;
    /// for a mark whose token tests it leaves (`tier1::TESTED`, decision 0655 step 6), the tokens.
    #[inline(always)]
    fn bit_at(&self, j: usize, b: u32) -> bool {
        j < self.n && (self.code_at(j) >> (16 + b) & 1 == 1 || crate::tier1::ASKED >> b & 1 == 1 && self.gap(j, b) || crate::tier1::TESTED >> b & 1 == 1 && crate::tier1::tested(self.src, self.recs, self.xs, j, b))
    }

    /// Gap mark `b` at token `j`, asked of the bytes since the grammar token before's first byte, as tier 1 reads a
    /// gap and the lean parser asks whether a token is on a line.
    fn gap(&self, j: usize, b: u32) -> bool {
        let p = self.prev_tok(j);
        let (from, before) = if p < self.n { (self.recs[p].start as usize + 1, self.word_at(p)) } else { (0, 0) };
        crate::tier1::gap(self.src, from, self.recs[j].start as usize, before, b)
    }

    /// Whether one of the marks `bits` holds at record `j`: its code's, the gap marks tier 2 asks of the bytes, and the
    /// marks whose token tests it asks.
    fn marks_hold(&self, j: usize, bits: u32) -> bool {
        if self.recs[j].code >> 16 & bits != 0 {
            return true;
        }
        let mut ask = (crate::tier1::ASKED | crate::tier1::TESTED) & bits;
        while ask != 0 {
            if self.bit_at(j, ask.trailing_zeros()) {
                return true;
            }
            ask &= ask - 1;
        }
        false
    }

    #[inline(always)]
    fn start(&self) -> u32 {
        if self.i >= self.n {
            self.src.len() as u32
        } else if self.k < self.sub.len() {
            self.sub[self.k].0
        } else {
            self.recs[self.i].start
        }
    }

    #[inline(always)]
    fn at_end(&self) -> bool {
        self.i >= self.n
    }

    #[cold]
    fn refuse<X>(&self) -> R<X> {
        // `RERAIL_TIER2_TRACE`: where, as the lean parser's `RERAIL_KOTLIN_TRACE` says it.
        #[cfg(debug_assertions)]
        if std::env::var_os("RERAIL_TIER2_TRACE").is_some() {
            eprintln!("refused at {}:\n{}", self.start(), std::backtrace::Backtrace::force_capture());
        }
        Err(self.start())
    }

    fn deeper(&mut self) -> R {
        self.depth += 1;
        if self.depth > DEPTH {
            return self.refuse();
        }
        Ok(())
    }

    /// The comments before the token, written into the node open: the one a mark lost first.
    #[inline(always)]
    fn flush(&mut self) {
        if let Some((kind, Some(s))) = self.lost {
            self.lost = None;
            let at = self.w.push(u32::from(kind) | 1 << 16, s);
            self.w.close(at, s);
        }
        while self.pend < self.xi {
            let j = self.pend;
            self.pend += 1;
            if !self.gone.contains(&j) {
                let (s, e) = self.extra_span(j);
                self.w.push_leaf(u32::from(extra_kind(self.xs[j].code)) | 1 << 16, s, e);
            }
        }
    }

    /// Extra `j`'s span: its bytes, or of no width where a mark's `loses since` moved it.
    fn extra_span(&self, j: usize) -> (u32, u32) {
        match self.moved {
            Some((m, s)) if m == j => (s, s),
            _ => (self.xs[j].start, self.xs[j].last + 1),
        }
    }

    /// `loses since`: the first extra pending of no width at the next one's start (the lean
    /// lexer's `asi` reads a block comment through to another comment and loses its bytes there).
    fn move_first(&mut self) {
        let mut pending = (self.pend..self.xi).filter(|j| !self.gone.contains(j));
        if let (Some(a), Some(b)) = (pending.next(), pending.next()) {
            self.moved = Some((a, self.xs[b].start));
        }
    }

    /// The comments pending before the line break of settling mark `bit` (an extra's code holds
    /// the bits of its own gap): written now, into the node open, the node's end past them.
    fn settle(&mut self, bit: u32) {
        // An extra a `loses since` moved and the one after it go with those before: the lean
        // lexer's semicolon is past that one.
        let moved = self.moved.map(|(m, _)| m).filter(|&m| m >= self.pend && m < self.xi);
        let through = moved.and_then(|m| (m + 1..self.xi).find(|k| !self.gone.contains(k)));
        while self.pend < self.xi {
            let j = self.pend;
            let c = self.xs[j].code;
            if c >> (16 + bit) & 1 == 1 && Some(j) != moved && Some(j) != through {
                break;
            }
            self.pend += 1;
            if !self.gone.contains(&j) {
                let (s, e) = self.extra_span(j);
                self.w.push_leaf(u32::from(extra_kind(c)) | 1 << 16, s, e);
                self.last_end = self.last_end.max(e);
            }
        }
    }

    /// The extra pending last before the token (not yet written, not dropped or lost).
    fn pending_last(&self) -> Option<usize> {
        (self.pend..self.xi).rev().find(|j| !self.gone.contains(j))
    }

    /// `drops`: the extra pending last gone; with none left pending, the nodes closed next end
    /// at the token, where the semicolon of no width is (the lean lexer's `Asi::Swallow`).
    fn drop_pending(&mut self) {
        if let Some(j) = self.pending_last() {
            self.gone.push(j);
            if self.pending_last().is_none() {
                self.last_end = self.start();
            }
        }
    }

    /// `loses`: the extra pending last made again after the token is taken, of no width at its
    /// start (the lean lexer's `asi` loses it); once a token.
    fn lose_pending(&mut self) {
        if self.lost.is_some() {
            return;
        }
        if let Some(j) = self.pending_last() {
            self.gone.push(j);
            self.lost = Some((extra_kind(self.xs[j].code), None));
        }
    }

@TAKE
    #[inline(always)]
    fn set_made(&mut self, at: u32) {
        self.top.1 = at;
    }

@TAKES
    fn open_root(&mut self, kind: u16) {
        let at = self.w.push(u32::from(kind), 0);
        self.levels.push(std::mem::replace(&mut self.top, (at as u32, NONE)));
    }

    /// The root closed at the end of the file: from the file's start (`whole`), else (or `whole padded`) from its
    /// first token or comment, as tree-sitter's root starts past the file's leading blanks.
    fn close_root(&mut self, whole: bool, padded: bool) -> R {
        if self.i < self.n {
            return self.refuse();
        }
        self.flush();
        // From its first token or comment (none: the file's end), to the file's end (`whole`) or its last token's.
        let first = if whole && !padded {
            0
        } else if self.w.len() > 1 {
            self.w.start(1)
        } else {
            self.src.len() as u32
        };
        let end = if whole { self.src.len() as u32 } else { self.last_end };
        self.w.set_start(0, first);
        self.w.close(0, end);
        Ok(())
    }

    /// A node of `kind`, open at the token.
    #[inline(always)]
    fn open(&mut self, kind: u16) {
        self.flush();
        let at = self.w.push(u32::from(kind), self.start());
        self.levels.push(std::mem::replace(&mut self.top, (at as u32, NONE)));
    }

    /// The node open closed at the last token's end; the node made last at its parent's level.
    /// One still of no kind (`UNSET`, never retagged) refuses the file.
    #[inline(always)]
    fn close(&mut self) -> R {
        let at = std::mem::replace(&mut self.top, self.levels.pop().unwrap_or((NONE, NONE))).0 as usize;
        // Closed before the refusal: a refused file's tape is not read.
        if self.w.close_from(at, self.last_end) & 0x3ff == u32::from(UNSET) {
            return self.refuse();
        }
        self.set_made(at as u32);
        Ok(())
    }

    /// The node made last at this level given `field`, where it has none.
    fn field_made(&mut self, field: u16) {
        let at = self.top.1;
        if at == NONE {
            return;
        }
        self.w.set_field(at as usize, 0x3f << 10, u32::from(field) << 10);
    }

    /// The node open, a `kind` now.
    fn retag(&mut self, kind: u16) {
        let at = self.top.0 as usize;
        let k = self.w.key(at);
        self.w.set_key(at, k & !0x3ff | u32::from(kind));
    }

    /// A node of `kind` put over the node made last at this level, open: it takes that node's
    /// field, and that node takes `field`.
    fn wrap(&mut self, kind: u16, field: u16) -> R {
        let at = self.top.1;
        if at == NONE {
            return self.refuse();
        }
        let at = at as usize;
        self.w.wrap(at, 0x3f << 10, u32::from(field) << 10, u32::from(kind));
        self.levels.push(std::mem::replace(&mut self.top, (at as u32, at as u32 + 1)));
        Ok(())
    }

    /// The node a wrap moved (the wrap's first child) a `kind` now.
    fn retype_moved(&mut self, kind: u16) {
        let at = self.top.0 as usize + 1;
        let k = self.w.key(at);
        self.w.set_key(at, k & !0x3ff | u32::from(kind));
    }

    /// The node made last at this level and the nodes inside it read again: `way` gives each
    /// kind's (`Some(Some(K))` read as `K`, `Some(None)` gone and its children in its place,
    /// `None` kept as it is with every node inside it; `u16::MAX - 1` refuses), `field` each
    /// field's new one, and `root` the node's own way (`u16::MAX`: its kind's).
    fn reread(&mut self, root: Option<Option<u16>>, way: fn(u16) -> Option<Option<u16>>, field: fn(u16) -> u16) -> R {
        let at = self.top.1;
        if at == NONE {
            return self.refuse();
        }
        let at = at as usize;
        let n = self.w.size(at) as usize;
        if at + n != self.w.len() {
            return self.refuse();
        }
        // A leaf gone (no extra, kept as it is; no dead node held in it): nothing to read again, the tape one node
        // shorter (`drop`'s, at a list's separator).
        if n == 1 && root == Some(None) && self.w.key(at) >> 16 & 1 == 0 {
            self.w.truncate(at);
            return Ok(());
        }
        // The nodes as they are, then written again in preorder.
        let nodes: Vec<(u32, u32, u32, usize)> = (at..at + n).map(|i| (self.w.key(i), self.w.start(i), self.w.end(i), self.w.size(i) as usize)).collect();
        self.w.truncate(at);
        let mut i = 0;
        while i < nodes.len() {
            i = self.reread_node(&nodes, i, root, way, field)?;
        }
        if self.w.len() == at + 1 || at < self.w.len() {
            self.set_made(at as u32);
        }
        Ok(())
    }

    /// `reread`'s node `i` and the nodes inside it: the node after them.
    fn reread_node(&mut self, nodes: &[(u32, u32, u32, usize)], i: usize, root: Option<Option<u16>>, way: fn(u16) -> Option<Option<u16>>, field: fn(u16) -> u16) -> R<usize> {
        let (key, s, e, size) = nodes[i];
        let kind = (key & 0x3ff) as u16;
        let to = if key >> 16 & 1 == 1 {
            None
        } else if i == 0 {
            match root {
                Some(Some(u16::MAX)) => way(kind),
                r => r,
            }
        } else {
            way(kind)
        };
        let key = if i == 0 { key } else { key & !(0x3f << 10) | u32::from(field((key >> 10 & 0x3f) as u16)) << 10 };
        match to {
            Some(Some(k)) if k == u16::MAX - 1 => self.refuse(),
            None => {
                let at = self.w.push(key, s);
                let mut j = i + 1;
                while j < i + size {
                    j = self.reread_node(nodes, j, Some(None), |_| None, |f| f)?;
                }
                self.w.close(at, e);
                Ok(j)
            }
            Some(None) => {
                let mut j = i + 1;
                while j < i + size {
                    j = self.reread_node(nodes, j, root, way, field)?;
                }
                Ok(j)
            }
            Some(Some(k)) => {
                let at = self.w.push(key & !0x3ff | u32::from(k), s);
                let mut j = i + 1;
                while j < i + size {
                    j = self.reread_node(nodes, j, root, way, field)?;
                }
                self.w.close(at, e);
                Ok(j)
            }
        }
    }
}
"#;

/// Tier 2's Rust, written from the tree rules.
pub fn tier2(x: &Input<'_>) -> Result<String, String> {
    // A level's indent and a mark's column (decision 0698): the JIT's and the interpreter's only, yet.
    if x.tree.iter().any(|s| matches!(s, Stmt::Indented(_))) {
        return Err("an `indented` level: not yet in Rust (decision 0698)".into());
    }
    G::new(x).source()
}
