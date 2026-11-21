//! Tier 2's program as structured Rust, a backend of the native IR (`Structure`) beside
//! Cranelift's (`native_cpu_stream`): rustc and LLVM compile it ahead of time.
//!
//! The code has the lean parser's shape (`rr_syntax::lean`):
//!
//! - **A function a rule.** A grammar call (`PUSH`, then a `GOTO` to its rule) is a Rust call;
//!   its `RETURN` a Rust return, `Ok(again)`: whether the caller switches on the same token
//!   again. The calls open are the machine's stack (`native_cpu_stream::calls_on_stack`).
//! - **The level's state as locals.** The level's word (`REMEMBER`) and flags (`FLAG`) are a
//!   rule's parameters, given to its calls and its own again when they return, as the
//!   interpreter keeps them beside each return.
//! - **The run's state in one struct**, `S`, which each rule takes as `&mut S`, as the lean
//!   parser's methods take `&mut self`: the token's start, end and code, the node table's
//!   counts, the buffers. (Each kept apart costs more, measured over `.rerail`'s corpus: there is
//!   about one call a token. The state in each rule's locals, copied out and in at each call,
//!   cost 28% more instructions; its hot fields alone in locals, 17%; the buffers behind a
//!   read-only reference of their own, 11%, in the registers each call saves.)
//! - **Structured control.** A rule's words are a graph (each word a node); it is written as
//!   loops, labelled blocks and `match`es (`clif_rust::Shape`), each `SWITCH` a `match` on the
//!   token's kind or byte.
//! - **One failure exit.** A word that refuses returns `refuse(why::…)`, its reason (`native_cpu_stream::refusal`),
//!   a cold call, passed up by `?`, as the lean parser returns its `Code`s. The run returns `refusal::MARK | why`: no
//!   word's place, as a refused file goes to the fallback and the interpreter names the word where that is wanted
//!   (`stream::run`). Out of the tape's room (`why::ROOM`), the harness runs it again on a bigger tape.
//! - **Nodes written inline**, a row at a time, as the lean parser's push.
//! - **A local check** (`CHECK`) is word-parallel: at a token whose bytes have bits of the
//!   check's mask, each 64-byte word of the token's with bits is classified by nibble tables
//!   (tier 1's own byte classes), the check's other operations are word operations on the
//!   bitmasks, and the test is one AND (`A_word & L_word != 0`). No loop over bits or bytes.
//!
//! The code reads and writes the harness's `Ctx` and tape as the JIT's does
//! (`native_cpu_stream::CompiledStream::set_entry`), so the tapes compare row for row.

use crate::clif_rust::Shape;
use crate::native_ir::{FlagOp, On, Structure, StructuralOperation as Op};
use crate::program::{LocalCheck, LocalOp, TokenMark};
use crate::stream_ir::{ByteSwitch, DEAD, GONE, KEEP, REFUSE, REMEMBERED, WORD_KIND};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt::Write;

/// A direct format 16 run's word whose node the source's language has no number for (decision 0516): the file is
/// refused there, as the JIT's run refuses it at `fail::NUMBER`.
pub const UNNUMBERED: &str = "// No number in the language for this node: the file refused (`fail::NUMBER`).\nreturn refuse(why::NUMBER);";

/// The program's facts the words read.
struct Facts<'a> {
    native: &'a Structure,
    /// A mark loses an extra (`SETTLE m=2`): the token taken makes it again.
    loses: bool,
    /// The marks the records hold, and the live bits at a token and at the stream's end
    /// (`native_cpu_stream`'s `in_code`, `rest_bits`, `end_bits`).
    in_code: usize,
    rest_bits: u64,
    end_bits: u64,
    /// The gap marks tier 2 tests itself (`native_cpu_block::lazy_marks`): `(mark, output)`.
    lazy: Vec<(u8, u16)>,
    /// Each kind's least byte, the stream's end as a kind past them, each word's kind.
    kind_reps: Option<Vec<u8>>,
    end_kind: Option<u8>,
    word_kind_of: Vec<u8>,
    /// The parts' own kinds (`records::Retag::part`): a token of one may be any word.
    part_kinds: Vec<usize>,
    framed: bool,
    context: bool,
    /// Each function's entry place.
    entries: Vec<usize>,
    /// Each word's phase (`native_cpu_stream::phases`), and whether a `TOKEN` has both (then a
    /// flag, `need`, says whether it takes a token, and `end` is the token's end): else each
    /// word knows both.
    phase: Vec<u8>,
    flagged: bool,
    /// The tape's widths and the language's tag (`native_cpu_stream::compile_at`'s).
    widths: rerail_tape_format::Widths,
    tag: u8,
    /// Where tier 2 makes each token's record from its start (page 40), as simdjson's stage 2
    /// takes a structural index: the rules and the region read as `parse_string`.
    fetch: Option<Fetched>,
    /// The supertype classes the tape's codes carry (`native_cpu_stream::supertype_classes`).
    classes: Vec<u16>,
    /// The tape sized from the input once (`Bounds::unchecked`): no room check a token.
    unchecked: bool,
    /// Where the program is one function, as `walk_document` (`OneFn`): its calls and returns.
    one: Option<OneFn>,
    /// Each node's code written as it is made (`coded`), no commit pass after.
    coded: bool,
    /// Format 16 (decision 0434): each node passed to the writer's ops at its word (`OPEN`, `LEAF`, `CLOSE`, the
    /// root's at `ACCEPT`), its number by its key, which `coded` knows there; no format 15 image.
    sixteen: bool,
    /// The stream's end as a start: past the last token, `TOKEN` gives the file's length as its
    /// start, with no branch (`sentinel`), as simdjson's index ends with one past the last.
    sentinel: bool,
    /// The grammar's names (`Words::names`), as Rust: each function's (by its entry), each node
    /// kind's constant (`k::…`), each record kind's (`t::…`) and each word's (`w::…`).
    fn_names: HashMap<usize, String>,
    kind_names: HashMap<u16, String>,
    token_names: HashMap<u8, String>,
    word_names: Vec<String>,
    /// A format 16 source's language's numbers by key (`native_cpu_stream::numbers_by_key`): a direct run writes
    /// each node's as a literal at its word. Empty elsewhere.
    numbers: Vec<u32>,
    /// The regions read as a method each (`string_methods`), as the lean parser's `string`: each part's `TOKEN` the
    /// name of the method its loop is, one method a loop of the same code.
    strings: HashMap<usize, String>,
    /// The opener's place is passed to the region's method (`s.string(s.start + 1)`), as the lean parser's
    /// `string(v)`, where every way into each method comes from an opener's arm with the token's start unchanged
    /// (`opener_place_at_call`); else the arm sets `s.part_at`, which the method reads.
    part_arg: bool,
    /// The methods the switches' arms call (`Facts::method`): each its name and its signature and body, one a code.
    methods: std::cell::RefCell<Vec<(String, String)>>,
    /// The dispatches' values the rules use (`L`, `rule`): each the name of the place it goes on to.
    labels: std::cell::RefCell<Vec<String>>,
}

impl Facts<'_> {
    /// The method of signature and body `code`, named for `base` (`base`, else `base_N` where another method has that
    /// name): one method a code, however many arms call it, as the lean parser's `primitive` from each place.
    fn method(&self, base: &str, code: String) -> String {
        let mut ms = self.methods.borrow_mut();
        if let Some((name, _)) = ms.iter().find(|(_, c)| *c == code) {
            return name.clone();
        }
        let same = ms.iter().filter(|(n, _)| n == base || n.strip_prefix(base).is_some_and(|r| r.starts_with('_') && r[1..].bytes().all(|b| b.is_ascii_digit()))).count();
        let name = if same == 0 { base.to_string() } else { format!("{base}_{same}") };
        ms.push((name.clone(), code));
        name
    }

    /// The number of the node of key `kf` (kind, field at bit 16) and supertype class bits `classes` in the source's
    /// language, `None` where its table has none (the run refuses there, as the JIT's at `fail::NUMBER`).
    fn number(&self, kf: u32, classes: u32) -> Option<u32> {
        let k = (kf & 0x3ff) | ((kf >> 16) & 0x3f) << rerail_tape_format::KIND_BITS | classes;
        self.numbers.get(k as usize).copied().filter(|&n| n != u32::MAX)
    }
}

impl Facts<'_> {
    /// The test that the stream has ended, at the run's state `who` (`s`, `self`).
    fn at_end(&self, who: &str) -> String {
        if self.sentinel { format!("({who}.start == {who}.len)") } else { format!("{who}.at_end") }
    }
}

/// Whether the end can be a start (`Facts::sentinel`): records made in tier 2, no marks (their
/// `live` at the end is its own), and no `TOKEN` after a switch takes the end (a token taken
/// past the end refuses, where the end as a start would be the end again).
fn sentinel(x: &Facts<'_>) -> bool {
    let words = &x.native.structural;
    if x.fetch.is_none() || x.flagged || words.iter().any(|w| matches!(w.operation, Op::Mark { .. })) {
        return false;
    }
    let mut seen = vec![false; words.len()];
    let mut work: Vec<usize> = x.native.switches.iter().map(|sw| usize::from(sw.end)).filter(|&t| t < words.len()).collect();
    while let Some(pc) = work.pop() {
        if std::mem::replace(&mut seen[pc], true) {
            continue;
        }
        if matches!(words[pc].operation, Op::Token { .. }) {
            return false;
        }
        work.extend(plain_succ(x, pc));
    }
    true
}

/// A program written as one function, as simdjson's `walk_document`: a call (`PUSH`) stores its
/// site's number on a stack of its own (`S::ret`, as `is_array[depth]`) and goes on at the rule's
/// first word; a `RETURN` takes the number off and goes on at that site's place, as `scope_end`
/// goes back to the state of the scope it returns to.
struct OneFn {
    /// Each call's place, and the number it stores: calls that every `RETURN` sends to the same
    /// place store one number, as `is_array` is one bit for a scope's two states.
    sites: Vec<(usize, usize)>,
    /// Each `RETURN`'s numbers (those of the calls whose rule holds it), each with its place.
    returns: HashMap<usize, Vec<(usize, usize)>>,
    /// Each call's places after it: where its rule's returns go back to (its target, and the one
    /// after only where a `RETURN` gives `again`).
    after: HashMap<usize, Vec<usize>>,
    /// The calls nested as the containers (`stacked`): a call's number kept in its container's
    /// row, and no `CLOSE` that can find no node open.
    stacked: bool,
    /// Where they nest so: the most nodes a rule holds open at a word, counted as `stacked` counts them (a call's
    /// container its callee's).
    most: i64,
}

/// Whether each node's code can be written as it is made (`Facts::coded`): no word edits a
/// node made (`stacked`'s words; a field only on the node made last), and every `NODE_SUPER` and
/// `NODE_FIELD` is reached only from a node's `OPEN` or `LEAF` by words that make no node
/// (`GOTO`, `SPAN`, another of them), so the node's key is its own and the words after it
/// (`key_after`), known when it is made.
fn coded(x: &Facts<'_>) -> bool {
    let words = &x.native.structural;
    if !x.one.as_ref().is_some_and(|o| o.stacked) || words.iter().any(|w| matches!(w.operation, Op::NodeField { on, .. } if on != On::Here)) {
        return false;
    }
    let mut preds: Vec<Vec<usize>> = vec![Vec::new(); words.len()];
    for pc in 0..words.len() {
        for t in plain_succ(x, pc) {
            preds[t].push(pc);
        }
    }
    // A straight word's way back reaches a node's word on every path.
    let mut ok: Vec<Option<bool>> = vec![None; words.len()];
    fn back(x: &Facts<'_>, preds: &[Vec<usize>], ok: &mut Vec<Option<bool>>, q: usize) -> bool {
        if let Some(v) = ok[q] {
            return v;
        }
        ok[q] = Some(false);
        let words = &x.native.structural;
        let v = !preds[q].is_empty()
            && preds[q].iter().all(|&p| match words[p].operation {
                Op::Open { .. } | Op::Leaf { .. } => plain_succ(x, p).len() == 1,
                Op::Goto { .. } | Op::Span { .. } | Op::NodeSuper { .. } | Op::NodeField { on: On::Here, .. } => plain_succ(x, p).len() == 1 && back(x, preds, ok, p),
                _ => false,
            });
        ok[q] = Some(v);
        v
    }
    (0..words.len()).filter(|&q| matches!(words[q].operation, Op::NodeSuper { .. } | Op::NodeField { .. })).all(|q| back(x, &preds, &mut ok, q))
}

/// The key bits node word `pc`'s key gets from the words after it that make no node (`coded`):
/// each supertype's class (`native_cpu_tape::commit`'s), and a field set there (in `kf`'s
/// place, above its kind). Of a node with its field in the next word (`field_after`), that one
/// is its own already.
fn key_after(x: &Facts<'_>, pc: usize) -> (u32, u32) {
    let words = &x.native.structural;
    let (mut bits, mut field) = (0u32, 0u32);
    let mut q = pc;
    for _ in 0..words.len() {
        let &[t] = plain_succ(x, q).as_slice() else { break };
        match words[t].operation {
            Op::NodeSuper { kind } => {
                if let Some(j) = x.classes.iter().position(|&c| c == kind) {
                    bits |= 1 << (16 + j);
                }
            }
            Op::NodeField { field: f, on: On::Here } => field = u32::from(f) << 16,
            Op::Goto { .. } | Op::Span { .. } => {}
            _ => break,
        }
        q = t;
    }
    (field, bits)
}

/// Whether the calls nest as the nodes (`OneFn::stacked`): each `PUSH` right after its
/// container's `OPEN` (a supertype or field word between), each `RETURN` right after the `CLOSE`
/// of that container, and each rule's opens and closes counted alike on every way, a call the
/// close of the container before it: 0 at a `RETURN`'s `CLOSE`, and at least 1 at every other
/// `CLOSE` and every `PUSH`. Then that `CLOSE` closes the node the call's `OPEN` made, the
/// container's row holds the call's number until it (its end is not read before), and no
/// `CLOSE` finds no node open. Only words whose nodes this counts: no wrap, fold or drop. Where they do: the most
/// nodes a rule holds open at a word (`OneFn::most`, the stack's bound's).
fn stacked(x: &Facts<'_>) -> Option<i64> {
    let words = &x.native.structural;
    let one = x.one.as_ref()?;
    let mut most = 0;
    let mut preds: Vec<Vec<usize>> = vec![Vec::new(); words.len()];
    for pc in 0..words.len() {
        for t in plain_succ(x, pc) {
            preds[t].push(pc);
        }
    }
    let only_from = |pc: usize, q: usize| preds[pc].as_slice() == [q];
    for &(pc, _) in &one.sites {
        let mut q = pc;
        loop {
            if q == 0 || !only_from(q, q - 1) {
                return None;
            }
            q -= 1;
            match words[q].operation {
                Op::Open { .. } => break,
                Op::NodeSuper { .. } | Op::NodeField { .. } => {}
                _ => return None,
            }
        }
    }
    for &r in one.returns.keys() {
        if r == 0 || !only_from(r, r - 1) || !matches!(words[r - 1].operation, Op::Close { .. }) {
            return None;
        }
    }
    // Each rule from its first word (the document's from word 0): a count a word, the same on
    // every way.
    let mut entries: Vec<usize> = vec![0];
    entries.extend(one.sites.iter().map(|&(pc, _)| crate::native_cpu_stream::callee(x.native, pc)));
    entries.sort_unstable();
    entries.dedup();
    for e in entries {
        let mut at: HashMap<usize, i64> = HashMap::from([(e, 0)]);
        let mut work = vec![e];
        while let Some(pc) = work.pop() {
            let c = at[&pc];
            most = most.max(c);
            let after = match words[pc].operation {
                Op::Open { .. } => c + 1,
                Op::Close { .. } => {
                    let last = plain_succ(x, pc).iter().any(|&t| matches!(words[t].operation, Op::Return { .. }));
                    if (last && c != 0 && e != 0) || (!last && c < 1) {
                        return None;
                    }
                    c - 1
                }
                Op::Push { .. } if c < 1 => return None,
                Op::Push { .. } => c - 1,
                Op::Token { .. } | Op::Switch { .. } | Op::Leaf { .. } | Op::NodeSuper { .. } | Op::NodeField { .. } | Op::Span { .. } | Op::Check { .. } | Op::Goto { .. } | Op::Return { .. } | Op::Accept | Op::Stop | Op::Empty { .. } => c,
                _ => return None,
            };
            most = most.max(after);
            for t in plain_succ(x, pc) {
                match at.get(&t) {
                    Some(&v) if v != after => return None,
                    Some(_) => {}
                    None => {
                        at.insert(t, after);
                        work.push(t);
                    }
                }
            }
        }
    }
    Some(most)
}

impl OneFn {
    fn of(native: &Structure, x: &dyn Fn(usize) -> Vec<usize>) -> OneFn {
        let words = &native.structural;
        let sites: Vec<usize> = (0..words.len()).filter(|&pc| matches!(words[pc].operation, Op::Push { .. })).collect();
        let mut returns: HashMap<usize, Vec<(usize, usize)>> = HashMap::new();
        for (k, &pc) in sites.iter().enumerate() {
            let Op::Push { target, .. } = words[pc].operation else { unreachable!() };
            // The rule's words: from its first, its calls not followed (`plain_succ`).
            let mut seen = vec![false; words.len()];
            let mut work = vec![crate::native_cpu_stream::callee(native, pc)];
            while let Some(w) = work.pop() {
                if std::mem::replace(&mut seen[w], true) {
                    continue;
                }
                if let Op::Return { again } = words[w].operation {
                    returns.entry(w).or_default().push((k, usize::from(target) + usize::from(again)));
                }
                work.extend(x(w));
            }
        }
        let mut after: HashMap<usize, Vec<usize>> = HashMap::new();
        for arms in returns.values() {
            for &(k, t) in arms {
                let v = after.entry(sites[k]).or_default();
                if !v.contains(&t) {
                    v.push(t);
                }
            }
        }
        // A call's number: the places every return sends it to, numbered as they first come.
        let mut rets: Vec<usize> = returns.keys().copied().collect();
        rets.sort_unstable();
        let place = |k: usize| -> Vec<Option<usize>> { rets.iter().map(|r| returns[r].iter().find(|&&(s, _)| s == k).map(|&(_, t)| t)).collect() };
        let mut kinds: Vec<Vec<Option<usize>>> = Vec::new();
        let number: Vec<usize> = (0..sites.len())
            .map(|k| {
                let p = place(k);
                kinds.iter().position(|q| *q == p).unwrap_or_else(|| {
                    kinds.push(p);
                    kinds.len() - 1
                })
            })
            .collect();
        for arms in returns.values_mut() {
            for a in arms.iter_mut() {
                a.0 = number[a.0];
            }
            arms.sort_unstable();
            arms.dedup();
        }
        OneFn { sites: sites.into_iter().zip(number).collect(), returns, after, stacked: false, most: 0 }
    }
}

/// The facts of a program whose tier 2 makes the records (`Records::fetched_in_tier2`).
struct Fetched {
    ft: Box<crate::native_cpu_stream::Fetch>,
    /// The region tier 2 splits, read as `parse_string` (`native_cpu_stream::Seq`), and its
    /// part's first byte.
    seq: Option<(crate::native_cpu_stream::Seq, u8)>,
    /// Each word reached with no part pending, and with one (`parts_pending_seen`).
    seen: Vec<[bool; 2]>,
}

impl Fetched {
    /// A `TOKEN` where a part of the region is always pending: the string's next part.
    fn part_at(&self, x: &Facts<'_>, pc: usize) -> bool {
        matches!(x.native.structural[pc].operation, Op::Token { .. }) && self.seen[pc] == [false, true]
    }
}

impl Facts<'_> {
    /// The function of the rule entered at `pc`.
    fn function(&self, pc: usize) -> String {
        self.fn_names.get(&pc).cloned().unwrap_or_else(|| format!("r{pc}"))
    }

    /// Node kind `k` (as a `u16`).
    fn kind(&self, k: u16) -> String {
        self.kind_names.get(&k).map_or_else(|| format!("{k}"), |n| format!("k::{n}"))
    }

    /// Node kind `k` and field bits `f` (`kind_field`, a `u32`).
    fn kind_field(&self, k: u16, f: u32) -> String {
        match (self.kind_names.get(&k), f) {
            (Some(n), 0) => format!("k::{n} as u32"),
            (Some(n), f) => format!("k::{n} as u32 | {f:#x}"),
            (None, f) => format!("{}", u32::from(k) | f),
        }
    }

    /// String `s`'s word, as a record names it (`S::word`).
    fn word(&self, s: u16) -> String {
        self.word_names.get(usize::from(s)).map_or_else(|| format!("{}", u32::from(s) + 1), |n| format!("w::{n}"))
    }

    /// Record kind `k`, the stream's end past them (`end_kind`).
    fn token(&self, k: usize) -> String {
        if self.end_kind.is_some_and(|e| usize::from(e) == k) {
            return "t::END".into();
        }
        u8::try_from(k).ok().and_then(|k| self.token_names.get(&k)).map_or_else(|| format!("{k}"), |n| format!("t::{n}"))
    }

    /// The named constants: `k`, `t` and `w`.
    fn constants(&self, out: &mut String) {
        let mut k: Vec<(&u16, &String)> = self.kind_names.iter().collect();
        k.sort();
        out.push_str("/// The node kinds, by the grammar's names.\nmod k {\n");
        for (v, n) in k {
            writeln!(out, "    pub const {n}: u16 = {v};").unwrap();
        }
        out.push_str("}\n\n/// The record kinds (a token's, by its first byte), by the grammar's names.\nmod t {\n");
        let mut t: Vec<(&u8, &String)> = self.token_names.iter().collect();
        t.sort();
        for (v, n) in t {
            writeln!(out, "    pub const {n}: u32 = {v};").unwrap();
        }
        if let Some(e) = self.end_kind {
            writeln!(out, "    /// The stream's end, a kind past the records'.\n    pub const END: u32 = {e};").unwrap();
        }
        out.push_str("}\n\n/// The words, as a record names them (a string's index, plus one).\nmod w {\n");
        for (i, n) in self.word_names.iter().enumerate() {
            writeln!(out, "    pub const {n}: u32 = {};", i + 1).unwrap();
        }
        out.push_str("}\n\n");
    }
}

/// The Rust keywords, which a grammar's name may be.
const KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern", "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return", "self", "Self", "static", "struct", "super", "trait", "true", "type", "unsafe", "use", "where", "while", "abstract", "become", "box", "do", "final", "macro", "override", "priv", "try", "typeof", "unsized", "virtual", "yield", "gen",
];

/// A grammar's name (a rule's, a kind's, a word's: `import_header`, `"`, `?.`) as a Rust name:
/// its letters, digits and `_`, each other character by its name; upper case for a constant.
fn rust_name(name: &str, upper: bool) -> String {
    rust_name_of(name, upper)
}

/// `rust_name`, for the other generators (`rust_block`).
pub(crate) fn rust_name_of(name: &str, upper: bool) -> String {
    let mut out = String::new();
    for c in name.chars() {
        let part = match c {
            c if c.is_ascii_alphanumeric() || c == '_' => {
                out.push(if upper { c.to_ascii_uppercase() } else { c.to_ascii_lowercase() });
                continue;
            }
            '"' => "quote", '\'' => "tick", '`' => "backtick", '.' => "dot", ',' => "comma", ':' => "colon", ';' => "semi",
            '(' => "lparen", ')' => "rparen", '[' => "lbrack", ']' => "rbrack", '{' => "lbrace", '}' => "rbrace",
            '<' => "lt", '>' => "gt", '=' => "eq", '!' => "bang", '?' => "question", '+' => "plus", '-' => "minus",
            '*' => "star", '/' => "slash", '%' => "percent", '&' => "amp", '|' => "bar", '^' => "caret", '~' => "tilde",
            '@' => "at", '#' => "hash", '$' => "dollar", '\\' => "backslash", ' ' => "space",
            _ => "x",
        };
        if !out.is_empty() && !out.ends_with('_') {
            out.push('_');
        }
        out.push_str(&if upper { part.to_ascii_uppercase() } else { part.to_owned() });
        out.push('_');
    }
    let mut out = out.trim_end_matches('_').to_owned();
    if out.is_empty() || out.starts_with(|c: char| c.is_ascii_digit()) || KEYWORDS.contains(&out.as_str()) {
        out.insert(0, if upper { 'N' } else { 'r' });
        out.insert(1, '_');
    }
    out
}

/// Each number's name, distinct: the second of one name gets its number after it.
fn distinct<K: Copy + std::fmt::Display + std::hash::Hash + Eq>(names: impl Iterator<Item = (K, String)>) -> HashMap<K, String> {
    let mut seen: HashMap<String, usize> = HashMap::default();
    let mut out = HashMap::default();
    for (k, n) in names {
        let c = seen.entry(n.clone()).or_default();
        *c += 1;
        out.insert(k, if *c == 1 { n } else { format!("{n}_{k}") });
    }
    out
}

/// Tier 2's program as Rust source: `tier2_entry(ctx)`, the harness's entry
/// (`native_cpu_stream::Entry`), with its fingerprint, `SOURCE`.
pub fn rust_source(native: &Structure) -> Result<String, String> {
    rust_source_at(native, rerail_tape_format::Narrow::WIDTHS, native.tag)
}

/// As `rust_source`, the tape written at `widths` with the language's `tag`.
pub fn rust_source_at(native: &Structure, widths: rerail_tape_format::Widths, tag: u8) -> Result<String, String> {
    source(native, widths, tag, None)
}

/// As `rust_source_at`, writing format 16 by the writer's ops as it parses (decision 0434 §6, R17), for
/// `CompiledStream::write16`'s harness: the entry takes `Ctx::writer` and `Ctx::numbers`. Where each node's key is
/// known at its word (`Facts::coded`), each node goes to the ops at its word, as it is made, with no row; elsewhere
/// the rows are the run's window, and the nodes go to the ops at its commit points, one behind (`pass16`), as the
/// JIT's run passes them.
///
/// The source is `lang`'s (decision 0516): a direct run's node numbers are literals at their words, as the lean parser
/// writes a kind's constant, and its runs take `lang` (its fingerprint is the source's `LANG`). A window run reads
/// them by key at run time, as the JIT's (`Ctx::numbers`).
pub fn rust_source_16(native: &Structure, widths: rerail_tape_format::Widths, tag: u8, lang: &rerail_tape_format::f16::Lang) -> Result<String, String> {
    source(native, widths, tag, Some(lang))
}

/// Where every node's key is known at its word (`Facts::coded`) and no leaf is deferred or an extra: each node word's
/// key bits from the words after it (`key_after`: its field, its supertypes' classes), by its place. A format 16 run
/// of such a program passes each node to the writer's ops at its word, with no row (`native_cpu_stream`'s, as
/// `rust_source_16`'s). `None` elsewhere.
pub fn direct_keys(native: &Structure, widths: rerail_tape_format::Widths, tag: u8) -> Option<Vec<(u32, u32)>> {
    let x = facts(native, widths, tag, true).ok()?;
    let plain = native.extras.is_empty() && native.readings.provisional.is_empty() && !native.structural.iter().any(|w| matches!(w.operation, Op::Leaf { deferred: true, .. } | Op::Leaf { extra: true, .. }));
    (x.coded && plain).then(|| (0..native.structural.len()).map(|pc| key_after(&x, pc)).collect())
}

/// Every key a direct run of the program writes (`direct_keys`' programs), as the language's entries: each node word's
/// kind (a keyword's leaf: each keyword's), its field and its supertypes' classes, keyed as the run numbers them
/// (`Facts::number`). A language that holds them all leaves no node word of the grammar without its number, whatever
/// the files a test reads. `None` where the program is not one a direct run takes.
pub fn direct_entries(native: &Structure, widths: rerail_tape_format::Widths, tag: u8) -> Option<Vec<rerail_tape_format::tape::Entry>> {
    direct_keys(native, widths, tag)?;
    let x = facts(native, widths, tag, true).ok()?;
    let mut out: Vec<rerail_tape_format::tape::Entry> = Vec::new();
    let mut add = |kf: u32, classes: u32| {
        let e = rerail_tape_format::tape::Entry {
            kind: (kf & 0x3ff) as u16,
            field: ((kf >> 16) & 0x3f) as u16,
            flags: 0,
            alias: None,
            classes: (0..x.classes.len()).filter(|&j| classes >> (16 + j) & 1 != 0).map(|j| x.classes[j]).collect(),
        };
        if !out.contains(&e) {
            out.push(e);
        }
    };
    for (pc, w) in native.structural.iter().enumerate() {
        let (field, classes) = key_after(&x, pc);
        match w.operation {
            Op::Open { kind } => {
                let kf = u32::from(kind) | u32::from(field_after(&x, pc).unwrap_or(0)) << 16;
                add(if field != 0 { kf & 0xffff | field } else { kf }, classes);
            }
            Op::Leaf { kind, .. } if kind != REMEMBERED && kind != DEAD => {
                let f = u32::from(field_after(&x, pc).unwrap_or(0)) << 16;
                let kinds: Vec<u32> = if kind == WORD_KIND { native.word_kinds.iter().map(|&(_, k)| u32::from(k)).collect() } else { vec![u32::from(kind)] };
                for k in kinds {
                    add(if field != 0 { (k | f) & 0xffff | field } else { k | f }, classes);
                }
            }
            _ => {}
        }
    }
    Some(out)
}

/// The most nodes a direct run of the program holds open (`f16::Writer::reserve_run`'s depth): a rule holds at most
/// `OneFn::most` at a word, a call nests at most `native_cpu_stream::DEPTH` deep and keeps its container open. With
/// the stack reserved so up front, a direct run checks its room once, where it starts, as the lean parser sizes its
/// stacks once (`DEFAULT_MAX_DEPTH`), and no node checks it. `None` where the program is not one a direct run takes.
pub fn open_bound(native: &Structure, widths: rerail_tape_format::Widths, tag: u8) -> Option<u64> {
    open_bound_of(&facts(native, widths, tag, true).ok()?)
}

fn open_bound_of(x: &Facts<'_>) -> Option<u64> {
    let one = x.one.as_ref().filter(|o| o.stacked && x.coded && x.sixteen)?;
    let depth = crate::native_cpu_stream::DEPTH as u64;
    Some((depth + 1) * one.most.max(0) as u64 + depth)
}

/// The program's facts (`Facts`), or why the Rust backend does not take it.
fn facts(native: &Structure, widths: rerail_tape_format::Widths, tag: u8, sixteen: bool) -> Result<Facts<'_>, String> {
    if !crate::native_cpu_stream::calls_on_stack(native) {
        return Err("calls are not on the machine's stack (`calls_on_stack`)".into());
    }
    let fetch = fetched(native)?;
    let late = native.block_programs.iter().filter_map(|b| b.records.as_ref()).any(|r| r.readers.iter().any(|rd| !rd.parts.is_empty() && rd.split == crate::records::Split::Tier2));
    if late && fetch.as_ref().is_none_or(|f| f.seq.is_none()) {
        return Err("a region tier 2 splits (decision 0323), not read as `parse_string`: not yet ahead of time".into());
    }
    let classes = crate::native_cpu_stream::supertype_classes(native).map_err(|e| format!("supertype classes: {e:?}"))?;
    let records = native.block_programs.first().and_then(|b| b.records.as_ref());
    let starts = native.structural.iter().any(|b| matches!(b.operation, Op::Token { .. }));
    let in_code = records.map_or(0, |r| r.mark_bits());
    let lazy: Vec<(u8, u16)> = native.block_programs.first().and_then(|b| b.records.as_ref().map(|r| (b, r))).map_or(Vec::new(), |(b, r)| {
        let bw = crate::native_cpu_bytes::Bytewise::of(b);
        crate::native_cpu_block::lazy_marks(r, &|o| bw.can(o))
    });
    let lazy_bits: u64 = lazy.iter().fold(0, |a, &(k, _)| a | 1 << k);
    // The code's token marks with no bit (`Records::alone`): live at every token, as past it.
    let rest_bits: u64 = (if in_code >= 64 { 0 } else { u64::MAX << in_code }) | lazy_bits | records.map_or(0, |r| u64::from(r.alone));
    let mut end_bits = rest_bits;
    for b in &native.structural {
        if let Op::Mark { order, end: true, .. } = b.operation
            && usize::from(order) < in_code
        {
            end_bits |= 1 << order;
        }
    }
    let kind_reps: Option<Vec<u8>> = records.filter(|_| starts).map(|r| {
        let n = r.kinds.iter().copied().max().map_or(0, |m| usize::from(m) + 1);
        (0..n).map(|k| r.kinds.iter().position(|&x| usize::from(x) == k).unwrap_or(0) as u8).collect()
    });
    let end_kind = kind_reps.as_ref().and_then(|r| u8::try_from(r.len()).ok());
    let word_kind_of: Vec<u8> = records.filter(|_| starts).map_or_else(Vec::new, |r| r.words.iter().map(|w| r.kinds[usize::from(w[0])]).collect());
    let part_kinds: Vec<usize> = records.map_or(Vec::new(), |r| r.retags.iter().filter(|t| t.part).map(|t| usize::from(t.kind)).collect());
    let words = &native.structural;
    let framed = words.iter().any(|b| matches!(b.operation, Op::Wrap { .. } | Op::Reread { .. } | Op::Retag { on: On::Made, .. } | Op::NodeField { on: On::Made | On::MadeUnset | On::Inner, .. }));
    let context = words.iter().any(|b| matches!(b.operation, Op::Remember { .. } | Op::Flag { .. } | Op::Push { flag: Some(_), .. }));
    let mut entries: BTreeSet<usize> = BTreeSet::from([0]);
    for (pc, b) in words.iter().enumerate() {
        if let Op::Push { .. } = b.operation {
            entries.insert(crate::native_cpu_stream::callee(native, pc));
        }
    }
    let phase = crate::native_cpu_stream::phases(native, &crate::native_cpu_stream::push_targets(native));
    let flagged = phase.contains(&(crate::stream::TAKEN | crate::stream::UNTAKEN));
    let names = &native.names;
    let fn_names = distinct(names.rules.iter().filter_map(|(i, n)| native.rules.get(usize::from(*i)).map(|&pc| (usize::from(pc), rust_name(n, false)))));
    // A rule's function and the harness's own share no name.
    let fn_names = fn_names.into_iter().map(|(pc, n)| (pc, if n == "tier2_entry" || n.starts_with('r') && n[1..].bytes().all(|b| b.is_ascii_digit()) { format!("rule_{n}") } else { n })).collect();
    let kind_names = distinct(names.kinds.iter().map(|(k, n)| (*k, rust_name(n, true))));
    let token_names = distinct(names.tokens.iter().map(|(k, n)| (*k, rust_name(n, true))));
    let word_names: Vec<String> = {
        let d = distinct(native.strings.iter().enumerate().map(|(i, s)| (i, rust_name(&String::from_utf8_lossy(s), true))));
        (0..native.strings.len()).map(|i| d[&i].clone()).collect()
    };
    let loses = native.structural.iter().any(|b| matches!(b.operation, Op::LoseLast { .. }));
    if fetch.is_some() && (flagged || !native.extras.is_empty()) {
        return Err("records made in tier 2, with a token's taking flagged or extras: not yet ahead of time".into());
    }
    let unchecked = crate::native_cpu_stream::bounds(native).unchecked(crate::native_cpu_stream::marks_of(words));
    let mut facts = Facts { native, loses, in_code, rest_bits, end_bits, lazy, kind_reps, end_kind, word_kind_of, part_kinds, framed, context, entries: entries.into_iter().collect(), phase, flagged, widths, tag, fetch, classes, unchecked, one: None, sentinel: false, coded: false, sixteen, fn_names, kind_names, token_names, word_names, numbers: Vec::new(), strings: HashMap::new(), part_arg: false, methods: Default::default(), labels: Default::default() };
    facts.sentinel = sentinel(&facts);
    // Where tier 2 makes the records, the program is one function, as `walk_document`.
    if facts.fetch.is_some() && !facts.context {
        let one = OneFn::of(native, &|w| plain_succ(&facts, w));
        facts.one = Some(one);
        facts.entries = vec![0];
        let st = stacked(&facts);
        if let Some(one) = &mut facts.one {
            one.stacked = st.is_some();
            one.most = st.unwrap_or(0);
        }
        facts.coded = coded(&facts);
    }
    if sixteen && facts.coded && !native.readings.provisional.is_empty() {
        return Err("format 16 ahead of time: provisional fields, which `ACCEPT` reads from the rows".into());
    }
    if let Some(fx) = &facts.fetch {
        for (pc, w) in words.iter().enumerate() {
            if matches!(w.operation, Op::Token { .. }) && fx.seen[pc] == [true, true] {
                return Err(format!("word {pc}: a `TOKEN` where a part may be pending or not"));
            }
            if fx.part_at(&facts, pc) {
                part_facts(&facts, fx, pc)?;
            }
        }
    }
    Ok(facts)
}

fn source(native: &Structure, widths: rerail_tape_format::Widths, tag: u8, lang: Option<&rerail_tape_format::f16::Lang>) -> Result<String, String> {
    let mut facts = facts(native, widths, tag, lang.is_some())?;
    if let Some(lang) = lang {
        facts.numbers = crate::native_cpu_stream::numbers_by_key(lang, &facts.classes, &native.readings.extras);
    }
    let mut out = String::new();
    out.push_str("// Generated by rerail_compiler::rust_stream from tier 2's native IR. Do not edit.\n");
    out.push_str("#![allow(unused_mut, unused_variables, unused_assignments, unreachable_code, unused_labels, unused_unsafe, unused_parens, non_snake_case, dead_code, clippy::all)]\n\n");
    out.push_str("use core::arch::aarch64::*;\n\n");
    facts.constants(&mut out);
    let mut strings = string_methods(&facts);
    facts.strings = strings.iter().flat_map(|(name, pcs, _)| pcs.iter().map(move |&pc| (pc, name.clone()))).collect();
    // The opener's place an argument where every call has it as the token's start: the methods written again, with it.
    facts.part_arg = opener_place_at_call(&facts);
    if facts.part_arg {
        strings = string_methods(&facts);
    }
    prelude(&mut out, &facts)?;
    for &e in &facts.entries {
        rule(&mut out, &facts, e)?;
    }
    if !strings.is_empty() {
        out.push_str("impl S {\n");
        for (name, pcs, body) in &strings {
            let at: Vec<String> = pcs.iter().map(usize::to_string).collect();
            out.push_str(&indented(&format!("/// A region read to its closer, as the lean parser's `string`: each stop's content, each escape and its checks, then the closer's token, which the caller goes on from (the parts' `TOKEN`s at words {}).\n#[inline(always)]\nfn {name}(&mut self{}) -> R {{\nlet s = self;\n'part: loop {{\n{body}}}\n}}\n", at.join(", "), if facts.part_arg { ", mut at: u32" } else { "" }), 1));
            out.push('\n');
        }
        out.push_str("}\n");
    }
    let labels = facts.labels.borrow();
    if !labels.is_empty() {
        out.push_str("/// Where a dispatch goes on to (`d`), each named for its place as its loop's label is, as the lean parser's `Label`.\n#[derive(Clone, Copy, PartialEq, Eq)]\nenum L {\n");
        for v in labels.iter() {
            writeln!(out, "    {v},").unwrap();
        }
        out.push_str("}\n\n");
    }
    drop(labels);
    let methods = facts.methods.borrow();
    if !methods.is_empty() {
        out.push_str("impl S {\n");
        for (name, code) in methods.iter() {
            let (doc, rest) = code.split_once('\u{0}').unwrap_or(("", code));
            out.push_str(&indented(&format!("{doc}#[inline(always)]\nfn {name}{rest}\n"), 1));
            out.push('\n');
        }
        out.push_str("}\n");
    }
    drop(methods);
    if facts.sixteen {
        // The writer's ops, the host's own (`f16::Writer`'s), compiled with the run.
        out.push_str("\n/// The tape's writer's ops (decision 0434 §6), as `rerail_tape_format` compiles them\n/// (`ops`: `w` is the words', as the lean lexers name them).\nmod ops {\n");
        out.push_str(rerail_tape_format::WRITER_RUST);
        out.push_str("}\n");
    }
    if let Some(lang) = lang {
        let l = crate::clif_rust::fingerprint(&lang.to_text(""));
        writeln!(out, "\n/// The language whose numbers a direct run writes (`f16::Lang::to_text`'s fingerprint): its runs take it.\npub const LANG: u64 = {l:#x};").unwrap();
    }
    let h = crate::clif_rust::fingerprint(&out);
    writeln!(out, "\n/// This source's fingerprint (`clif_rust::fingerprint` of what comes before).\npub const SOURCE: u64 = {h:#x};").unwrap();
    Ok(out)
}

/// A direct format 16 run's writer (page 40, step 38): the writer's per-node work written here, in the lean parser's
/// `Walk` shape, its state the walk's fields, not the ops' CLIF translated. The rules are the reference writer's
/// (`f16/reference.rs`): `open`, `close`, `leaf`, a cell, a code. The cold paths (a block's room, a value past its
/// cell's width, a code past the cut, a block made final, a failure) call the ops with the state stored first.
fn w16(out: &mut String, x: &Facts<'_>) {
    if !(x.sixteen && x.coded) {
        out.push_str("/// No direct format 16 run: no writer.\nstruct W16;\n\n");
        return;
    }
    // The stack's room: once, where the run starts, where the program has a bound of open nodes (`open_bound`); else
    // at each node.
    let room = if open_bound_of(x).is_some() {
        ""
    } else {
        "\n        if (self.open_len as u64 + 1) * ops::OPEN > self.open_cap {\n            unsafe { self.fail(ops::fail::FULL + ops::part::OPEN as u32, ops::NONE) };\n            return None;\n        }"
    };
    out.push_str(&W16_SRC.replace("$OPEN_ROOM", room));
}

/// `W16`, as `w16` writes it.
const W16_SRC: &str = r#"
/// The format 16 writer of a direct run, held by the walk as the lean parser's `Walk` holds its columns: the nodes
/// so far, the last start, the open nodes' frames (the run's one stack, as simdjson's `open_containers`: the
/// writer's node, children, last child and start, and the walk's slot at byte 12), the current block's room, and
/// the document's layout, read once from the writer's state. Each cell is a store at its column's offset from its
/// block.
struct W16 {
    state: *mut u8,
    n: u32,
    prev: u64,
    len: u64,
    open: *mut u8,
    open_len: u32,
    open_cap: u64,
    /// The block of the node made last: its index, room and side area.
    block: u32,
    room: *mut u8,
    rooms: *mut u8,
    table: *mut u8,
    cut: u32,
    /// Each array's (slots, gaps, lengths) cell width and escape (its widest value), each column's offset in a
    /// room; the checkpoints' width and its mask.
    w: [usize; 3],
    esc: [u64; 3],
    at: [usize; 5],
    cw: usize,
    cmask: u64,
}

#[inline(always)]
unsafe fn rd32(p: *const u8, off: usize) -> u32 {
    unsafe { (p.add(off) as *const u32).read_unaligned() }
}
#[inline(always)]
unsafe fn rd64(p: *const u8, off: usize) -> u64 {
    unsafe { (p.add(off) as *const u64).read_unaligned() }
}
#[inline(always)]
unsafe fn wr32(p: *mut u8, off: usize, v: u32) {
    unsafe { (p.add(off) as *mut u32).write_unaligned(v) }
}
#[inline(always)]
unsafe fn wr64(p: *mut u8, off: usize, v: u64) {
    unsafe { (p.add(off) as *mut u64).write_unaligned(v) }
}
/// The cell at `p`, its bytes `m` (all ones in them), written as a word: `v`'s bytes, the bytes past it kept.
#[inline(always)]
unsafe fn cell(p: *mut u8, m: u64, v: u64) {
    unsafe {
        let old = (p as *const u64).read_unaligned();
        (p as *mut u64).write_unaligned(old & !m | v & m);
    }
}

impl W16 {
    /// The writer's state at `state`, read once for the run.
    unsafe fn begin(state: *mut u8) -> W16 {
        unsafe {
            let s = &*(state as *const ops::State);
            let part = |k: usize| state.offset(s.parts[k].at as isize);
            let cw = s.cw as usize;
            let mut wr = W16 {
                state,
                n: s.n,
                prev: s.prev_start,
                len: s.source_len,
                open: part(ops::part::OPEN),
                open_len: s.open_len,
                open_cap: s.parts[ops::part::OPEN].cap,
                block: 0,
                room: core::ptr::null_mut(),
                rooms: if s.flags & 2 != 0 { part(ops::part::ROOMS) } else { part(ops::part::OUT) },
                table: part(ops::part::TABLE),
                cut: s.cut,
                w: s.w.map(|w| w as usize),
                esc: s.esc,
                at: s.at.map(|a| a as usize),
                cw,
                cmask: if cw >= 8 { u64::MAX } else { (1u64 << (8 * cw)) - 1 },
            };
            if wr.n != 0 {
                wr.block = (wr.n - 1) / ops::Z;
                wr.room = wr.room_of(wr.block);
            }
            wr
        }
    }

    /// Whether the stack holds `bound` open nodes (`rust_stream::open_bound`), the run's most: its room checked once,
    /// where the run starts, and by no node. Else the writer's failure is kept, as a full stack's.
    #[inline(always)]
    unsafe fn holds(&self, bound: u64) -> bool {
        if (bound + 1) * ops::OPEN > self.open_cap {
            unsafe { self.fail(ops::fail::FULL + ops::part::OPEN as u32, ops::NONE) };
            return false;
        }
        true
    }

    /// The state the ops read stored: the nodes, the last start, the open nodes, the frontier (every node made is
    /// final but for the open ones: a direct run commits each as it is made).
    #[inline(always)]
    unsafe fn store(&self, frontier: u32) {
        unsafe {
            let s = &mut *(self.state as *mut ops::State);
            s.n = self.n;
            s.prev_start = self.prev;
            s.open_len = self.open_len;
            s.frontier = frontier;
        }
    }

    /// Block `b`'s room, from its table entry.
    #[inline(always)]
    unsafe fn room_of(&self, b: u32) -> *mut u8 {
        unsafe { self.rooms.add(rd32(self.table, b as usize * ops::TABLE as usize) as usize) }
    }

    /// The first failure kept (`w_fail`).
    #[cold]
    #[inline(never)]
    unsafe fn fail(&self, code: u32, node: u32) {
        unsafe { ops::w_fail(self.state as *const u8, code, node) };
    }

    /// Block `b`'s room made (`w_new_block`): once every `ops::Z` nodes. Whether it was.
    #[cold]
    #[inline(never)]
    unsafe fn new_block(&mut self, b: u32) -> bool {
        unsafe {
            if ops::w_new_block(self.state as *const u8, b) != 0 {
                return false;
            }
            self.block = b;
            self.room = self.room_of(b);
        }
        true
    }

    /// Node `i`'s value `v` in array `a` (0 slots, 1 gaps, 2 lengths), at row `j` of the room at `room`; a value
    /// past its cell's width to its block's side list (`w_put`). In a direct run a cell is written once, so no
    /// side-list entry of it is ever dropped.
    #[inline(always)]
    unsafe fn put(&self, room: *mut u8, a: usize, i: u32, j: u32, v: u64) -> bool {
        if v >= self.esc[a] {
            return unsafe { self.put_side(a, i, v) };
        }
        unsafe { cell(room.add(self.at[1 + a] + j as usize * self.w[a]), self.esc[a], v) };
        true
    }

    #[cold]
    #[inline(never)]
    unsafe fn put_side(&self, a: usize, i: u32, v: u64) -> bool {
        unsafe { ops::w_put(self.state as *const u8, a as u32, i, v) == 0 }
    }

    /// Node `i`'s code past the cut: its byte and its side-list entry (`w_put_code`).
    #[cold]
    #[inline(never)]
    unsafe fn code_side(&self, i: u32, number: u32) -> bool {
        unsafe { ops::w_put_code(self.state as *const u8, i, number) == 0 }
    }

    /// Block `b` made final where it is (`w_settle`), the state stored first.
    #[cold]
    #[inline(never)]
    unsafe fn settle(&self, b: u32, frontier: u32) -> bool {
        unsafe {
            self.store(frontier);
            ops::w_settle(self.state as *const u8, b) == 0
        }
    }

    /// Node `n`, of `number`, opened at `start` (the reference's `open`): its code, its gap, its checkpoint, the
    /// slot of the node before it; pushed where `push`. Its row in its block, or `None` where it failed.
    #[inline(always)]
    unsafe fn open_node(&mut self, number: u32, start: u64, push: bool) -> Option<u32> {
        let i = self.n;
        if start < self.prev || start > self.len || i + 1 >= ops::MAX_NODES {
            unsafe { self.fail(ops::fail::BOUNDS, i) };
            return None;
        }
        if i != 0 && self.open_len == 0 {
            unsafe { self.fail(ops::fail::OUTSIDE, i) };
            return None;
        }$OPEN_ROOM
        let (b, j) = (i / ops::Z, i % ops::Z);
        unsafe {
            if j == 0 && !self.new_block(b) {
                return None;
            }
            let room = self.room;
            // The node before is a leaf unless it is the parent: its slot describes this node.
            if self.open_len != 0 {
                let t = self.open.add((self.open_len as usize - 1) * ops::OPEN as usize);
                let (p, idx, last) = (rd32(t, 0), rd32(t, 4), rd32(t, 8));
                wr32(t, 4, idx + 1);
                wr32(t, 8, i);
                if i - 1 != p {
                    let v = if idx % ops::K == 0 { u64::from(i - p) << 2 | 3 } else { u64::from(i - last) << 2 | 1 };
                    let ok = if j == 0 { self.put_side(0, i - 1, v) } else { self.put(room, 0, i - 1, j - 1, v) };
                    if !ok {
                        return None;
                    }
                }
            }
            if number < self.cut {
                *room.add(j as usize) = number as u8;
            } else if !self.code_side(i, number) {
                return None;
            }
            let gap = if i == 0 { 0 } else { start - self.prev };
            if !self.put(room, 1, i, j, gap) {
                return None;
            }
            if j % ops::K == 0 {
                cell(room.add(self.at[4] + (j / ops::K) as usize * self.cw), self.cmask, start);
            }
            self.prev = start;
            self.n = i + 1;
            if push {
                let t = self.open.add(self.open_len as usize * ops::OPEN as usize);
                wr32(t, 0, i);
                wr32(t, 4, 0);
                wr32(t, 8, ops::NONE);
                wr32(t, 12, 0);
                wr64(t, 16, start);
                self.open_len += 1;
            }
            // The block before has its last slot now.
            if j == 0 && b != 0 && !self.settle(b - 1, i) {
                return None;
            }
        }
        Some(j)
    }

    /// A node at the end, open (`w_open`). Whether it was (else the writer's failure is kept).
    #[inline(always)]
    unsafe fn open(&mut self, number: u32, start: u64) -> bool {
        unsafe { self.open_node(number, start, true).is_some() }
    }

    /// A closed node with no children (`w_leaf`): opened, then its length; never on the open nodes.
    #[inline(always)]
    unsafe fn leaf(&mut self, number: u32, start: u64, end: u64) -> bool {
        unsafe {
            let Some(j) = self.open_node(number, start, false) else { return false };
            let i = self.n - 1;
            if end < start || end > self.len {
                self.fail(ops::fail::BOUNDS, i);
                return false;
            }
            self.put(self.room, 2, i, j, end - start)
        }
    }

    /// Whether the writer failed: the walk stops there, the host reads its failure.
    fn failed(&self) -> bool {
        unsafe { (*(self.state as *const ops::State)).bad != 0 }
    }

    /// The innermost open node's slot: the walk's, in its frame.
    #[inline(always)]
    unsafe fn slot(&self) -> u32 {
        unsafe { rd32(self.open.add((self.open_len as usize - 1) * ops::OPEN as usize), 12) }
    }
    #[inline(always)]
    unsafe fn set_slot(&mut self, k: u32) {
        unsafe { wr32(self.open.add((self.open_len as usize - 1) * ops::OPEN as usize), 12, k) }
    }

    /// The innermost open node's end (`w_close`): its length, and an inner node's slot; its block made final
    /// where a node past its end is made.
    #[inline(always)]
    unsafe fn close(&mut self, end: u64) {
        unsafe {
            if self.open_len == 0 {
                self.fail(ops::fail::NO_OPEN, self.n);
                return;
            }
            self.open_len -= 1;
            let t = self.open.add(self.open_len as usize * ops::OPEN as usize);
            let (i, start) = (rd32(t, 0), rd64(t, 16));
            if end < start || end > self.len {
                self.fail(ops::fail::BOUNDS, i);
                return;
            }
            let (b, j) = (i / ops::Z, i % ops::Z);
            let room = if b == self.block { self.room } else { self.room_of(b) };
            if !self.put(room, 2, i, j, end - start) {
                return;
            }
            let size = self.n - i;
            if size > 1 && !self.put(room, 0, i, j, u64::from(size) << 1) {
                return;
            }
            if self.n > (b + 1) * ops::Z {
                self.settle(b, self.n);
            }
        }
    }
}
"#;

/// The harness's `Ctx` and node row, the run's state `S`, and the words' own functions.
fn prelude(out: &mut String, x: &Facts<'_>) -> Result<(), String> {
    let n = x.native;
    let b = crate::native_cpu_stream::bounds(n);
    out.push_str(&CTX.replace("$STATE", &crate::native_cpu_stream::STATE_SLOTS.to_string()).replace("$REFUSAL", &format!("{:#x}", crate::native_cpu_stream::refusal::MARK)));
    // Each refusal's reason (`native_cpu_stream::refusal`), by name.
    out.push_str("/// Why the run refused the file (`native_cpu_stream::refusal`).\nmod why {\n");
    for (k, (name, text)) in crate::native_cpu_stream::refusal::NAMES.iter().enumerate() {
        writeln!(out, "    /// {}.\n    pub const {name}: u32 = {k};", text[..1].to_uppercase() + &text[1..]).unwrap();
    }
    out.push_str("}\n\n");
    tape_consts(out, x);
    let counts = n.structural.iter().filter_map(|w| if let Op::Count { count, .. } = w.operation { Some(usize::from(count) + 1) } else { None }).max().unwrap_or(0);
    writeln!(out, "const COUNTS: usize = {};", counts.max(1)).unwrap();
    writeln!(out, "const DEPTH: u32 = {};", crate::native_cpu_stream::DEPTH).unwrap();
    if let Some(bound) = open_bound_of(x) {
        writeln!(out, "/// The most nodes the run holds open (`rust_stream::open_bound`): the writer's stack is reserved for them.\nconst OPEN_BOUND: u64 = {bound};").unwrap();
    }
    if x.sixteen {
        // The failures of a run's own (the writer's `fail`).
        out.push_str("const NUMBER: u32 = ops::fail::NUMBER;\nconst PASSED: u32 = ops::fail::PASSED;\n");
    }
    w16(out, x);
    out.push_str(&STATE.replace("$PART_AT", if x.part_arg { "" } else { "\n    /// Where tier 2 makes the records: the place of a region's next part (0: none pending).\n    part_at: u32," }));
    // The entry.
    let begin = if x.sixteen && x.coded { "unsafe { W16::begin(c.writer) }" } else { "W16" };
    out.push_str(
        &"/// The entry, on the harness's `Ctx` (`native_cpu_stream::Entry`): 0 where the file is taken,
/// `u64::MAX` out of the tape's room, else a refusal (`REFUSAL`, its reason marked).
///
/// # Safety
/// As the JIT's entry: `ctx`'s pointers as `CompiledStream::call` sets them.
pub unsafe extern \"C\" fn tier2_entry(ctx: u64) -> u64 {
    let c = unsafe { &*(ctx as *const Ctx) };
    let mut s = S {
        bytes: c.bytes, len: c.len as u32, starts: c.starts, lasts: c.lasts, codes: c.codes, n: c.nstarts as usize,
        rs: c.record_stride as usize / 4,
        outputs: c.outputs as *const u8, ostride: c.output_stride as usize, lstride: c.lasts_stride as usize,
        totals: c.totals as *const u8, tstride: c.totals_stride as usize, tmask: c.totals_mask,
        rows: c.rows, snode: c.super_node, skind: c.super_kind, open: c.open, counts_out: c.counts,
        fbase: c.frame_base, head: c.head, link: c.link, place: c.place,
        size_c: c.size_c as *mut SizeT, off_c: c.off_c as *mut OffT, tail_c: c.tail_c as *mut TailT, base_c: c.base_c,
        image: c.image, id_of: c.id_of, code_kf: c.code_kf, code_first: c.code_first,
        writer: c.writer, numbers: c.numbers, passed: 0,
        block_base: 0, coded: 0, ncodes: 0, wide: 0,
        room_nodes: c.room_nodes as usize, room_supers: c.room_supers as usize,
        next: 0, start: 0, end: 0, rec_end: 0, code: 0, last_end: 0, at_end: false, need: true,$PART_AT_INIT live: 0,
        nodes: 0, nopen: 0, pending: 0, keep: 0, supers: 0, lost: 0, last_made: 0, moved: 0,
        counts: [0; COUNTS], depth: 0, ret: [0; DEPTH as usize], wr: $W16_BEGIN,
    };
"
        .replace("$W16_BEGIN", begin)
        .replace("$PART_AT_INIT", if x.part_arg { "" } else { " part_at: 0," }),
    );
    let args = if x.context { ", 0, 0" } else { "" };
    // A direct run stops at the writer's failure (its stack is the walk's, D2), which the host reads, as the JIT's
    // run, which goes on to the end, leaves it (`write16`: a full open part is a run again, deeper).
    let failed = if x.sixteen && x.coded { "        _ if s.wr.failed() => 0,\n" } else { "" };
    if open_bound_of(x).is_some() {
        writeln!(out, "    // The stack's room for the run's most open nodes, once (`rust_stream::open_bound`): no node checks it.\n    if !unsafe {{ s.wr.holds(OPEN_BOUND) }} {{\n        return 0;\n    }}").unwrap();
    }
    writeln!(out, "    match {}(&mut s{args}) {{\n{failed}        Ok(_) => 0,\n        Err(why::ROOM) => u64::MAX,\n        Err(why) => REFUSAL | u64::from(why),\n    }}\n}}\n", x.function(0)).unwrap();

    out.push_str("impl S {\n");
    // The room: the most a token's path writes (`Bounds`), a call open's pops where returns chain.
    let pops = |per: u64| if per > 0 { format!(" + self.depth as usize * {per}") } else { String::new() };
    writeln!(
        out,
        "    /// A call one level deeper, as the lean parser's `deeper()`: refused past `DEPTH`.
    #[inline(always)]
    fn deeper(&mut self, rule: impl FnOnce(&mut S) -> Result<bool, u32>) -> Result<bool, u32> {{
        if self.depth == DEPTH {{
            return refuse(why::DEPTH);
        }}
        self.depth += 1;
        let again = rule(self)?;
        self.depth -= 1;
        Ok(again)
    }}

    /// The tape's room for the path to the next token (`native_cpu_stream::Bounds`).
    #[inline(always)]
    fn room(&mut self) -> R {{
        if self.nodes as usize + {}{} > self.room_nodes || self.supers as usize + {}{} > self.room_supers {{
            return refuse(why::ROOM);
        }}
        Ok(())
    }}
",
        b.nodes,
        pops(b.pop_nodes),
        b.supers,
        pops(b.pop_supers)
    )
    .unwrap();
    // The token steps, one for each `TOKEN` form.
    let mut forms: BTreeSet<(Option<u8>, bool)> = BTreeSet::new();
    for w in &n.structural {
        if let Op::Token { any, extras, .. } = w.operation {
            forms.insert((any, extras));
        }
    }
    for (any, extras) in forms {
        if x.fetch.is_some() {
            fetched_token_step(out, x, any)?;
        } else {
            token_step(out, x, any, extras, x.flagged)?;
        }
    }
    if let Some(fx) = &x.fetch {
        fetched_helpers(out, x, fx)?;
    }
    // The vector reads a fetched token step and a token's check share.
    if x.fetch.is_some() || n.local_checks.iter().any(|c| c.token) {
        out.push_str(VECTORS);
    }
    node_words(out, x);
    // The keyword's leaf kind (`WORD_KIND`): a `match` on the token's word.
    out.push_str("    /// A keyword's leaf kind, by its token's word (`Words::word_kinds`).\n    #[inline(always)]\n    fn word_kind(&self) -> Result<u32, u32> {\n        Ok(match self.word() {\n");
    for &(s, k) in &n.word_kinds {
        writeln!(out, "            {} => {k},", u32::from(s) + 1).unwrap();
    }
    out.push_str("            _ => return refuse(why::WORD),\n        })\n    }\n\n");
    // Accept: the table.
    accept(out, x);
    for (i, r) in n.readings.maps.iter().enumerate() {
        reread(out, i, r);
    }
    for (i, m) in n.token_marks.iter().enumerate() {
        pattern(out, i, m, n.extras.first().map(|e| e.0));
        if i == 0 || n.token_marks[i - 1].order != m.order {
            patterns(out, i, &n.token_marks);
        }
    }
    for (i, m) in n.gap_patterns.iter().enumerate() {
        gap_pattern(out, i, m);
        if i == 0 || n.gap_patterns[i - 1].order != m.order {
            gap_patterns(out, i, &n.gap_patterns);
        }
    }
    // The refusing token marks' tests, at a token whose code has a bit of theirs.
    let refusing: Vec<String> = n
        .token_marks
        .iter()
        .enumerate()
        .filter(|(_, m)| m.refuses)
        .map(|(i, m)| format!("(self.code >> {} & 1 != 0 && self.pattern_{i}())", crate::records::MARK_SHIFT + u32::from(m.order)))
        .collect();
    if !refusing.is_empty() {
        writeln!(out, "    /// Whether a token mark that refuses holds at this token (`TokenMark::refuses`).\n    #[inline(never)]\n    #[cold]\n    fn refusals(&self) -> bool {{\n        {}\n    }}\n", refusing.join(" || ")).unwrap();
    }
    let mut on_p = String::new();
    // A check's mask from the bytes, where they make it (`native_cpu_bytes`): its window
    // function through `clif_rust`, a module of its own; the CPU's tier 1 makes none of it.
    let bytewise = n.block_programs.first().map(crate::native_cpu_bytes::Bytewise::of);
    let mut windows = String::new();
    for (i, c) in n.local_checks.iter().enumerate() {
        // A token's check (decision 0398) is the CPU's and the interpreter's: no grammar that
        // goes through Rust has one yet (the `.rerail` grammar checks none).
        if c.token {
            // Reading back, the records' last bytes (`Ctx::lasts`): tier 1's, not a region tier 2 makes or splits.
            if c.back != 0 && x.fetch.is_some() {
                return Err(format!("check {i}: a token's check that reads back, where tier 2 makes the records"));
            }
            token_check(&mut on_p, i, c)?;
            continue;
        }
        let o = mask_output(n, c.mask)?;
        let word = match bytewise.as_ref().filter(|b| b.can(o as u16)) {
            Some(b) => {
                let func = b.window_function(o as u16, 0);
                let src = crate::clif_rust::translate(&[(0, func)], &[(0, "mask")], &[]).map_err(|e| format!("check {i}'s mask: {e}"))?;
                writeln!(windows, "/// Check {i}'s mask from the bytes, a window (`native_cpu_bytes`).\nmod window_{i} {{\n{src}}}\n").unwrap();
                format!("unsafe {{ window_{i}::mask(self.bytes as u64, self.len as u64, (w as u64) << 6) }}")
            }
            None => format!("self.mask_word({o}, w)"),
        };
        check(out, &mut on_p, i, c, &word)?;
    }
    // A gap mark tier 2 tests itself: the grammar token before, past the extras (none where its
    // word is an `unless after` one), then the mask's words from the byte after its first to
    // the token's (`records::gap_marks`), each a window from the bytes.
    if let (Some(b), Some(r)) = (bytewise.as_ref(), n.block_programs.first().and_then(|b| b.records.as_ref())) {
        let shift = crate::records::MARK_SHIFT + u32::from(r.extra.unwrap_or(0));
        let skip = if r.extra.is_some() { format!("            if c >> {shift} & 1 == 0 {{ break (st + 1, c >> 8 & 0xff); }}\n") } else { "            break (st + 1, c >> 8 & 0xff);\n".into() };
        for &(k, o) in &x.lazy {
            let func = b.window_function(o, 0);
            let src = crate::clif_rust::translate(&[(0, func)], &[(0, "mask")], &[]).map_err(|e| format!("gap mark {k}'s mask: {e}"))?;
            writeln!(windows, "/// Gap mark {k}'s mask from the bytes, a window (`native_cpu_bytes`).\nmod lazy_window_{k} {{\n{src}}}\n").unwrap();
            let unless: Vec<String> = r.unless.iter().filter(|&&(m, _)| m == k).map(|&(_, w)| w.to_string()).collect();
            let unless = if unless.is_empty() { String::new() } else { format!("        if matches!(word, {}) {{ return false; }}\n", unless.join(" | ")) };
            // A mark with token tests (`lazy_coded`): its tests' hit, in the token's code, first.
            let coded = if crate::native_cpu_block::lazy_coded(r) >> k & 1 == 1 {
                format!("        if unsafe {{ *self.codes.add((self.next as usize - 1) * self.rs) }} >> {} & 1 != 0 {{ return true; }}\n", crate::records::MARK_SHIFT + u32::from(k))
            } else {
                String::new()
            };
            writeln!(
                out,
                "    /// Gap mark {k} at the token, from the bytes (`native_cpu_block::lazy_marks`).\n    #[inline(never)]\n    fn lazy_{k}(&self) -> bool {{\n\
                 {coded}\
                 \x20       let mut j = self.next as usize - 1;\n\
                 \x20       let (from, word) = loop {{\n\
                 \x20           if j == 0 {{ break (0, 0); }}\n\
                 \x20           j -= 1;\n\
                 \x20           let (c, st) = unsafe {{ (*self.codes.add(j * self.rs), *self.starts.add(j * self.rs)) }};\n\
                 {skip}\
                 \x20       }};\n\
                 {unless}\
                 \x20       let s = self.start;\n\
                 \x20       if from > s {{ return false; }}\n\
                 \x20       for q in from >> 6..=s >> 6 {{\n\
                 \x20           let w = unsafe {{ lazy_window_{k}::mask(self.bytes as u64, self.len as u64, (q as u64) << 6) }};\n\
                 \x20           if w & S::within(q as i64, from as i64, s as i64 + 1) != 0 {{ return true; }}\n\
                 \x20       }}\n\
                 \x20       false\n    }}\n"
            )
            .unwrap();
        }
    }
    out.push_str("}\n\n");
    writeln!(out, "impl S {{\n{on_p}}}\n{windows}").unwrap();
    Ok(())
}

/// The tape's constants: the widths (`SizeT`, `OffT`, `TailT`), the format's.
fn tape_consts(out: &mut String, x: &Facts<'_>) {
    use rerail_tape_format as t;
    let w = x.widths;
    let ty = |b: u8| if b == 2 { "u16" } else { "u32" };
    let head = u32::from(x.tag) | u32::from(w.size) << 8 | u32::from(w.off) << 16 | u32::from(w.tail | if w.from_start { t::TAIL_FROM_START } else { 0 }) << 24;
    writeln!(
        out,
        "type SizeT = {};\ntype OffT = {};\ntype TailT = {};\nconst SIXTEEN: bool = {};\nconst FROM_START: bool = {};\nconst TAIL_MAX: u32 = {:#x};\n\
         const STRIDE: u32 = {};\nconst STRIDE_LOG: u32 = {};\nconst WIDE_OFF: u32 = {};\nconst WIDE_TAIL: u32 = {};\n\
         const NO_CODE: u32 = {:#x};\nconst KIND_BITS: u32 = {};\nconst HEADER_BYTES: usize = {};\nconst COLUMN_ALIGN: usize = {};\n\
         const NODES_AT: usize = {};\nconst CODE_EXTRA: u8 = {};\nconst SENTINEL: u16 = {:#x};\nconst MAGIC: u64 = {:#x};\nconst HEAD: u32 = {:#x};\nconst FORMAT_VERSION: u32 = {};\n",
        ty(w.size),
        ty(w.off),
        ty(w.tail),
        x.sixteen,
        w.from_start,
        if w.tail == 2 { 0xffffu32 } else { 0x7fff_ffff },
        t::CHECKPOINT_STRIDE,
        t::CHECKPOINT_STRIDE.trailing_zeros(),
        crate::native_cpu_tape::WIDE_OFF,
        crate::native_cpu_tape::WIDE_TAIL,
        crate::native_cpu_tape::NO_CODE,
        t::KIND_BITS,
        t::HEADER_BYTES,
        t::COLUMN_ALIGN,
        t::header::NODES,
        t::CODE_EXTRA,
        t::SENTINEL,
        u64::from_le_bytes(*t::MAGIC),
        head,
        t::FORMAT_VERSION,
    )
    .unwrap();
}

const CTX: &str = "/// The harness's argument (`native_cpu_stream::Ctx`), field for field.
#[repr(C)]
pub struct Ctx {
    bytes: *const u8, len: u64, starts: *const u32, nstarts: u64, lasts: *const u32, codes: *const u32,
    totals: *const u64, rows: *mut Row, super_node: *mut u32, super_kind: *mut u32, open: *mut u32,
    returns: *mut u32, counts: *mut u64, lasts_stride: u64, outputs: *const u64, frame_base: *mut u32,
    frame_last: *mut u32, frame_last_base: *mut u32, head: *mut u32, link: *mut u32, place: *mut u32,
    out_rows: *mut u8, output_stride: u64, totals_stride: u64, totals_mask: u64, room_nodes: u64,
    room_supers: u64, room_pushes: u64, stack_base: u64, stack_limit: u64, frame_shift: u64,
    state: [u64; $STATE], size_c: *mut u8, off_c: *mut u8, tail_c: *mut u8, base_c: *mut u32,
    image: *mut u8, id_of: *mut u16, code_kf: *mut u32, code_first: *mut u32,
    record_stride: u64, writer: *mut u8, numbers: *const u32,
}

/// A node's working row (`native_cpu_tape::ROW`): its key (kind and field), its bytes. Not
/// part of the tape.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Row { kind: u16, field: u16, start: u32, end: u32 }

/// A refusal's value: its reason (`why`) marked (`native_cpu_stream::refusal::MARK`), where no word's place is.
pub const REFUSAL: u64 = $REFUSAL;
/// A row's marks (`head`): a wrap; a dropped row taken out from among its children.
const WRAPPED: u32 = 1;
const DROPPED_IN: u32 = 2;
const DEAD: u16 = u16::MAX;

type R = Result<(), u32>;

/// A refusal, its reason `why`: cold, so every way to one is a branch out of the hot code, as the lean parser's
/// `cold_path` before each error it returns.
#[cold]
#[inline(never)]
fn refuse<T>(why: u32) -> Result<T, u32> {
    Err(why)
}

";

const STATE: &str = "/// The run's buffers and state, as the lean parser's `Parser`: each rule takes it as `&mut`,
/// as the lean parser's methods take `&mut self`.
struct S {
    bytes: *const u8, len: u32, starts: *const u32, lasts: *const u32, codes: *const u32, n: usize,
    /// The words from a token's record field to the next token's (`Ctx::record_stride`).
    rs: usize,
    outputs: *const u8, ostride: usize, lstride: usize, totals: *const u8, tstride: usize, tmask: u64,
    rows: *mut Row, snode: *mut u32, skind: *mut u32, open: *mut u32, counts_out: *mut u64,
    /// Scratch for a drop (`drop_dead`); which rows are wraps (`inner_field`).
    fbase: *mut u32, head: *mut u32, link: *mut u32, place: *mut u32,
    /// The tape (docs/parser-generators/37): its columns at their widths, the bases, the image
    /// (the header's room, then the code column in place), each key's code id and each code's
    /// key and first node.
    size_c: *mut SizeT, off_c: *mut OffT, tail_c: *mut TailT, base_c: *mut u32,
    image: *mut u8, id_of: *mut u16, code_kf: *mut u32, code_first: *mut u32,
    /// A format 16 run's (`Facts::sixteen`): the writer's state, each key's number in the language's table; the
    /// nodes passed to its ops (`pass16`).
    writer: *mut u8, numbers: *const u32, passed: u32,
    /// The last block's base; the nodes committed and the codes; the values past their width.
    block_base: u32, coded: u32, ncodes: u32, wide: u32,
    room_nodes: usize, room_supers: usize,
    /// The next record; the token's start, end (its start until its bytes are taken), its
    /// record's end and code; the end of the token before; the stream's end; the bytes taken.
    next: u32, start: u32, end: u32, rec_end: u32, code: u32, last_end: u32, at_end: bool, need: bool,$PART_AT
    /// The marks not yet taken or passed at this token.
    live: u64,
    /// The nodes made, open; the deferred extras' leaves, and the end before them; the
    /// supertypes.
    nodes: u32, nopen: u32, pending: u32, keep: u32, supers: u32,
    /// An extra a mark lost (`SETTLE m=2`): its kind and field, plus one (0: none), made again
    /// by the next `TOKEN` that takes a token.
    lost: u32,
    /// The node made last at the innermost level (one more than its row); the node the last
    /// wrap moved.
    last_made: u32, moved: u32,
    counts: [u32; COUNTS],
    /// The calls open.
    depth: u32,
    /// Where the program is one function (`OneFn`): each open call's site.
    ret: [u16; DEPTH as usize],
    /// A direct format 16 run's writer (`W16`), held by the walk.
    wr: W16,
}

impl S {
    #[inline(always)]
    fn word(&self) -> u32 { (self.code >> 8) & 0xff }
    #[inline(always)]
    fn row(&self, i: u32) -> *mut Row { unsafe { self.rows.add(i as usize) } }
    #[inline(always)]
    fn col(c: *mut u32, i: u32) -> *mut u32 { unsafe { c.add(i as usize) } }
}

impl S {
    /// Output `o`'s mask word `w`.
    #[inline(always)]
    fn mask_word(&self, o: usize, w: u32) -> u64 {
        unsafe { (self.outputs.add(o * self.ostride + w as usize * self.lstride) as *const u64).read_unaligned() }
    }
    #[inline(always)]
    fn bit(&self, o: usize, at: u32) -> bool { self.mask_word(o, at >> 6) >> (at & 63) & 1 != 0 }
    #[inline(always)]
    fn total(&self, t: usize) -> u64 { unsafe { (self.totals.add(t * self.tstride) as *const u64).read_unaligned() & self.tmask } }
    /// The 64 bytes of word `q` of the file (none past it: zeros).
    #[inline(always)]
    fn block(&self, q: i64) -> [uint8x16_t; 4] {
        unsafe {
            if q < 0 || q * 64 >= self.len as i64 {
                let z = vdupq_n_u8(0);
                return [z, z, z, z];
            }
            let p = self.bytes.add(q as usize * 64);
            [vld1q_u8(p), vld1q_u8(p.add(16)), vld1q_u8(p.add(32)), vld1q_u8(p.add(48))]
        }
    }
    /// The bits of word `q` whose places are in `lo..hi`.
    #[inline(always)]
    fn within(q: i64, lo: i64, hi: i64) -> u64 {
        let (a, b) = ((lo - q * 64).clamp(0, 64), (hi - q * 64).clamp(0, 64));
        if a >= b { 0 } else { (if b == 64 { u64::MAX } else { (1u64 << b) - 1 }) & (u64::MAX << a) }
    }
}

/// A vector's lanes that are all ones, a bit each (simdjson's `to_bitmask`), four vectors.
#[inline(always)]
fn bits(v: [uint8x16_t; 4]) -> u64 {
    unsafe {
        let m = vld1q_u8([1u8, 2, 4, 8, 16, 32, 64, 128, 1, 2, 4, 8, 16, 32, 64, 128].as_ptr());
        let a = vpaddq_u8(vandq_u8(v[0], m), vandq_u8(v[1], m));
        let b = vpaddq_u8(vandq_u8(v[2], m), vandq_u8(v[3], m));
        let c = vpaddq_u8(a, b);
        let c = vpaddq_u8(c, c);
        vgetq_lane_u64(vreinterpretq_u64_u8(c), 0)
    }
}

";

fn mask_output(n: &Structure, mask: u16) -> Result<usize, String> {
    n.block_programs.first().and_then(|b| b.masks.get(usize::from(mask)).copied()).ok_or_else(|| format!("mask {mask}: no output"))
}

/// `TOKEN`'s step: the room, then (where the bytes were taken) the next record, its extras
/// placed as deferred leaves, the marks live at it.
fn token_step(out: &mut String, x: &Facts<'_>, any: Option<u8>, extras: bool, flagged: bool) -> Result<(), String> {
    let n = x.native;
    let name = token_name(any, extras);
    let live = if x.in_code > 0 {
        format!("(self.code >> 16) as u64 | {:#x}", x.rest_bits)
    } else if let Some(k) = any {
        let o = mask_output(n, u16::from(k))?;
        format!("if self.bit({o}, st) {{ u64::MAX }} else {{ 0 }}")
    } else {
        "u64::MAX".into()
    };
    let end_code = x.end_kind.map_or(String::new(), |k| format!(" self.code = {k};"));
    // With phases known (`flagged` false), a `TOKEN` that runs this takes a token: the one
    // before was taken, its end its record's.
    let (test, before, set_end, extra_end) = if flagged {
        ("        if !self.need {\n            return Ok(());\n        }\n", "            self.last_end = self.end;\n            self.need = false;\n", "            self.end = st;\n", "                self.end = self.rec_end;\n")
    } else {
        ("", "            self.last_end = self.rec_end;\n", "", "")
    };
    let extra = match (extras, n.extras.split_first()) {
        (true, Some((&(bit, kind), more))) => {
            // The leaf's kind: the first's, or the last later kind whose bit the code has
            // (`stream::extra_kind`).
            let kind = more.iter().fold(kind.to_string(), |k, &(b, later)| format!("if c >> {} & 1 != 0 {{ {later} }} else {{ {k} }}", crate::records::MARK_SHIFT + u32::from(b)));
            format!(
                "            if c >> {} & 1 != 0 {{\n{extra_end}                self.leaf::<true, true>({kind}, self.rec_end);\n                self.room()?;\n                continue;\n            }}\n",
                crate::records::MARK_SHIFT + u32::from(bit)
            )
        }
        _ => String::new(),
    };
    // The token marks that refuse (`TokenMark::refuses`): their bits in the code, one test; where
    // one is set, `refusals` (cold) tests their tokens.
    let bits: u64 = n.token_marks.iter().filter(|m| m.refuses).fold(0, |b, m| b | 1u64 << (crate::records::MARK_SHIFT + u32::from(m.order)));
    let refusals = if bits == 0 { String::new() } else { format!("            if u64::from(c) & {bits:#x} != 0 && self.refusals() {{\n                return refuse(why::TOKEN);\n            }}\n") };
    writeln!(
        out,
        "    /// `TOKEN`: the next record{}.
    #[inline(always)]
    fn {name}(&mut self) -> R {{
        self.room()?;
{test}        if self.at_end {{
            return refuse(why::END);
        }}
        loop {{
{before}{lost}            let i = self.next as usize;
            if i == self.n {{
                self.at_end = true;
                self.live = {:#x};{end_code}
                return Ok(());
            }}
            let (st, c) = unsafe {{ (*self.starts.add(i * self.rs), *self.codes.add(i * self.rs)) }};
            self.start = st;
{set_end}            self.rec_end = unsafe {{ *self.lasts.add(i * self.rs) }} + 1;
            self.code = c;
            self.next = i as u32 + 1;
{extra}{refusals}            self.live = {live};
            return Ok(());
        }}
    }}
",
        if flagged { ", where this one's bytes are taken" } else { "" },
        x.end_bits,
        lost = if x.loses { "            if self.lost != 0 {\n                // A lost extra: after the token's leaf, of no width at its start.\n                let k = self.lost - 1;\n                self.lost = 0;\n                let at = self.start;\n                self.leaf::<true, true>(k, at);\n            }\n" } else { "" }
    )
    .unwrap();
    Ok(())
}

fn token_name(any: Option<u8>, extras: bool) -> String {
    format!("token_{}{}", any.map_or("all".to_string(), |k| format!("m{k}")), if extras { "_x" } else { "" })
}

/// The node words: open, close, leaf, wrap, fold, merge, the relabels of the node made last
/// at a level, and the tape's rare steps, as the JIT's (`native_cpu_stream`'s words and
/// `native_cpu_tape`'s functions).
///
/// Each node has a working row (`Row`: its key, start and end); the tape's columns are written
/// at their place (docs/parser-generators/37): `off` at its push from its block's base, `size`
/// and `tail` at its close, `code` at a commit point (a node closed at the top level: the
/// nodes before it are final). The widths are the source's constants (`SizeT`, `OffT`,
/// `TailT`). A wrap moves the table's tail up a cell, one row where it wraps a leaf; a drop
/// compacts it; each then gives the blocks from there their bases again. `ACCEPT` closes the
/// root, commits the rest and writes the image's end.
fn node_words(out: &mut String, x: &Facts<'_>) {
    let n = x.native;
    let framed = x.framed;
    let f = |s: &str| if framed { s.to_string() } else { String::new() };
    // Each row's marks (`head`, moved with the rows; `marked`): which rows are wraps, where
    // `NODE_FIELD m=3` asks; which had a dropped row taken out from among their children, where
    // a reread reads the table.
    let marked = marks(n);
    let inner = n.structural.iter().any(|b| matches!(b.operation, Op::NodeField { on: On::Inner, .. }));
    let w = |s: &str| if marked { s.to_string() } else { String::new() };
    let dropped_in = if marked { "unsafe { *Self::col(self.head, i) & DROPPED_IN != 0 }" } else { "false" };
    let early = if x.widths.from_start { "|| end < unsafe { (*self.row(last)).start }" } else { "" };
    let src = r#"
    /// Whether a dropped row was taken out from among row `i`'s children: a reread that reads
    /// on into them refuses, as it would meet the dropped one (`stream::run` keeps it there).
    #[inline(always)]
    fn dropped_in(&self, i: u32) -> bool {
        $DROPPED_IN
    }

    /// Node `node`'s offset, at its push with start `start`: from its block's base, which it
    /// sets where it starts a block; where it starts before the base or 0x10000 past it, the
    /// block's base and offsets again (`blocks`). `n`: the nodes, it counted.
    #[inline(always)]
    fn push_off(&mut self, node: u32, start: u32, n: u32) {
        // A format 16 run's writer makes its own blocks (decision 0434): no format 15 offsets or bases.
        if SIXTEEN {
            return;
        }
        unsafe {
            if node & (STRIDE - 1) == 0 {
                *self.base_c.add((node >> STRIDE_LOG) as usize) = start;
                self.block_base = start;
                *self.off_c.add(node as usize) = 0;
            } else {
                let off = start.wrapping_sub(self.block_base);
                if off > 0xffff {
                    self.blocks(node >> STRIDE_LOG, n);
                } else {
                    *self.off_c.add(node as usize) = off as OffT;
                }
            }
        }
    }

    /// Blocks `b` on, of `n` nodes: each block's base (its nodes' least start) and its nodes'
    /// offsets from it; the last base kept, an offset that needs 4 bytes flagged. A step a node.
    #[inline(never)]
    #[cold]
    fn blocks(&mut self, b: u32, n: u32) {
        // A format 16 run's writer makes its own blocks (decision 0434).
        if SIXTEEN {
            return;
        }
        let (mut k, mut last, mut wide) = (b, 0u32, 0u32);
        unsafe {
            while k * STRIDE < n {
                let from = k * STRIDE;
                let to = (from + STRIDE).min(n);
                let mut m = (*self.row(from)).start;
                for i in from..to {
                    m = m.min((*self.row(i)).start);
                }
                *self.base_c.add(k as usize) = m;
                for i in from..to {
                    let o = (*self.row(i)).start.wrapping_sub(m);
                    *self.off_c.add(i as usize) = o as OffT;
                    // Past its width (`OffT`'s): at 4 bytes an offset always fits.
                    wide |= u32::from(o > OffT::MAX as u32) * WIDE_OFF;
                }
                last = m;
                k += 1;
            }
        }
        self.block_base = last;
        self.wide |= wide;
    }

    /// The table cut back to `n` nodes (a fold, a merge): its last block's base again, where a
    /// block is left part full.
    #[inline(always)]
    fn cut_to(&mut self, n: u32) {
        if !SIXTEEN && n & (STRIDE - 1) != 0 {
            self.blocks((n - 1) >> STRIDE_LOG, n);
        }
    }

    /// Node `at` closed: its size, and its tail from its last descendant (row `last`) to `end`;
    /// a tail that does not fit its width (or wraps) flagged.
    #[inline(always)]
    fn close_cells(&mut self, at: u32, size: u32, last: u32, end: u32) {
        unsafe {
            let tail = end.wrapping_sub((*self.row(if FROM_START { at } else { last })).start);
            *self.size_c.add(at as usize) = size as SizeT;
            // A format 16 run's pass reads only the sizes: no tails.
            if SIXTEEN {
                return;
            }
            *self.tail_c.add(at as usize) = tail as TailT;
            if tail > TAIL_MAX $EARLY {
                self.wide |= WIDE_TAIL;
            }
        }
    }

    /// Nodes from the last commit to `to` final: their codes written, each key's id numbered by
    /// first use.
    #[inline(always)]
    fn commit(&mut self, to: u32) {$COMMIT_BODY
    }

    /// Node `at`'s kind changed (`native_cpu_stream::Table::retag`): where it is committed, its codes taken back;
    /// in a format 16 run, where the ops already have it (the root, which they hold open), retagged there.
    #[inline(always)]
    fn retag_at(&mut self, at: u32) {$RETAG_BODY
    }

    /// `native_cpu_tape::commit`: a step a node.
    #[inline(never)]
    fn commit_from(&mut self, from: u32, to: u32, codes: u32) -> u32 {
        let mut c = codes;
        unsafe {
            let code = self.image.add(HEADER_BYTES) as *mut u16;
            let mut i = from;$CLASS_FIRST
            while i < to {
                let key = *(self.row(i) as *const u32);
                let k = (key & 0x3ff) | ((key >> 16) & 0x3f) << KIND_BITS;$CLASS_BITS
                let slot = self.id_of.add(k as usize);
                let mut id = u32::from(*slot);
                if id == NO_CODE {
                    *slot = c as u16;
                    *self.code_kf.add(c as usize) = k;
                    *self.code_first.add(c as usize) = i;
                    id = c;
                    c += 1;
                }
                *code.add(i as usize) = id as u16;
                i += 1;
            }
        }
        c
    }

    /// An edit of node `node`: where it is committed, the codes first used at it or after taken
    /// back, numbered again at the next commit. A step a code.
    #[inline(always)]
    fn uncode(&mut self, node: u32) {$UNCODE_BODY
    }

    #[inline(never)]
    #[cold]
    fn uncode_from(&mut self, node: u32) {
        let mut c = self.ncodes;
        unsafe {
            while c > 0 && *self.code_first.add(c as usize - 1) >= node {
                let k = *self.code_kf.add(c as usize - 1);
                *self.id_of.add(k as usize) = NO_CODE as u16;
                c -= 1;
            }
        }
        self.ncodes = c;
        self.coded = node;
    }

    /// `OPEN`: its key (its kind, with its field above it or none), its start, its offset.
    #[inline(always)]
    fn open(&mut self, kind_field: u32) {
        let n = self.nodes;
        self.pending = 0;
        let start = self.start;
        unsafe {
            let r = self.row(n);
            *(r as *mut u32) = kind_field;
            (*r).start = start;
            *Self::col(self.open, self.nopen) = n;$HEAD_N
        }
        self.push_off(n, start, n + 1);
        $OPEN_MADE
        self.nopen += 1;
        self.nodes = n + 1;
    }

    /// A call's number, kept in its container's place (the node open last): its row, whose end the
    /// `CLOSE` writes (`OneFn::stacked`), or in a format 16 run its slot on the stack (`open16`).
    #[inline(always)]
    fn keep_ret(&mut self, k: u32) {
        unsafe { (*self.row(*Self::col(self.open, self.nopen - 1))).end = k };
    }

    /// The number `keep_ret` kept in the node open last, before its `CLOSE`.
    #[inline(always)]
    fn taken_ret(&self) -> u32 {
        unsafe { (*self.row(*Self::col(self.open, self.nopen - 1))).end }
    }

    /// `SETTLE`: the pending extras, from the first, while their codes have no bit `k` (the
    /// marks of an extra's own gap: a line end before it), settled: the nodes closed next keep
    /// them. They are the records before the token's (the one lost last left out).
    #[inline(always)]
    fn settle_before(&mut self, k: u32) {
        let token = $TOKEN_AT;
        let from = token - self.pending as usize - usize::from(self.lost != 0);
        let mut i = 0;
        while i < self.pending as usize && unsafe { *self.codes.add((from + i) * self.rs) } >> (16 + k) & 1 == 0 {
            i += 1;
        }
        if i > 0 {
            let first = self.nodes - self.pending;
            self.keep = unsafe { (*self.row(first + i as u32 - 1)).end };
            self.pending -= i as u32;
        }
    }

    /// `SETTLE m=2` (a mark's `loses`), where its question holds at the token: the extra
    /// pending last taken out, its kind kept (`lost`): the `TOKEN` that takes the token makes
    /// it again after it, of no width. With none left pending, the nodes closed next end at the
    /// token before.
    #[inline(always)]
    fn lose(&mut self) {
        self.pending -= 1;
        self.nodes -= 1;
        self.lost = unsafe { *(self.row(self.nodes) as *const u32) } + 1;
        if self.pending == 0 {
            self.last_end = self.keep;
        }
    }

    /// `CLOSE`: its end, size and tail; the pending extras after it are its parent's. A node
    /// closed at the top level: the nodes before it are final, their codes committed.
    #[inline(always)]
    fn close<const THIS: bool, const SURE: bool>(&mut self, this_end: u32) -> R {
        if !SURE && self.nopen == 0 {
            return refuse(why::NODES);
        }
        self.nopen -= 1;
        let at = unsafe { *Self::col(self.open, self.nopen) };
        let end = if THIS { this_end } else if self.pending != 0 { self.keep } else { self.last_end };
        unsafe { (*self.row(at)).end = end };
        $CLOSE_MADE
        let kept = self.nodes - self.pending;
        self.close_cells(at, kept - at, kept - 1, end);
$COMMIT_CLOSE        Ok(())
    }

    /// `LEAF`: its key (its kind, with its field above it or none), its bytes; its offset, its
    /// size and tail.
    #[inline(always)]
    fn leaf<const DEFERRED: bool, const EXTRA: bool>(&mut self, kind_field: u32, end: u32) {
        if DEFERRED {
            if self.pending == 0 {
                self.keep = self.last_end;
            }
            self.pending += 1;
        } else {
            self.pending = 0;
        }
        let n = self.nodes;
        let start = self.start;
        unsafe {
            let r = self.row(n);
            *(r as *mut u32) = kind_field;
            (*r).start = start;
            (*r).end = end;$HEAD_N
        }
        self.push_off(n, start, n + 1);
        self.close_cells(n, 1, n, end);
        $LEAF_MADE
        self.nodes = n + 1;
    }

    /// The node made last at the innermost level: its row.
    #[inline(always)]
    fn made(&self) -> Result<u32, u32> {
        if self.nopen == 0 || self.last_made == 0 {
            return refuse(why::NODES);
        }
        Ok(self.last_made - 1)
    }

    /// `WRAP`: a node before the one made last (its field the wrap's), open around it. That
    /// node and the rest of the table (the pending extras) move up a cell: one row where it is
    /// a leaf no supertype names, else `shift`, then the blocks from it again.
    #[inline(always)]
    fn wrap(&mut self, kind: u16) -> R {
        let r = self.made()?;
        self.uncode(r);
        let n = self.nodes;
        let named = self.supers != 0 && unsafe { *Self::col(self.snode, self.supers - 1) } >= r;
        unsafe {
            if n - r == 1 && !named {
                *self.row(n) = *self.row(r);
                *self.size_c.add(n as usize) = *self.size_c.add(r as usize);
                if !SIXTEEN {
                    *self.tail_c.add(n as usize) = *self.tail_c.add(r as usize);
                }$HEAD_MOVE
                self.push_off(n, (*self.row(n)).start, n + 1);
            } else {
                self.shift(r, n);
                self.blocks(r >> STRIDE_LOG, n + 1);
            }
            let (to, from) = (self.row(r), self.row(r + 1));
            *(to as *mut u32) = u32::from(kind) | u32::from((*from).field) << 16;
            (*from).field = 0;$HEAD_WRAP
            *Self::col(self.open, self.nopen) = r;
        }
        self.last_made = r + 2;
        self.moved = r + 1;
        self.pending = 0;
        self.nopen += 1;
        self.nodes = n + 1;
        Ok(())
    }

    /// A wrap at row `r` of a table of `n`: rows `r..n` up a row, latest first, with their size
    /// and tail cells (and marks); the supertypes' nodes from `r` on, one more. A step a row.
    #[inline(never)]
    fn shift(&mut self, r: u32, n: u32) {
        unsafe {
            let mut i = n;
            while i > r {
                let j = (i - 1) as usize;
                *self.row(i) = *self.row(i - 1);
                *self.size_c.add(i as usize) = *self.size_c.add(j);
                if !SIXTEEN {
                    *self.tail_c.add(i as usize) = *self.tail_c.add(j);
                }$HEAD_SHIFT
                i -= 1;
            }
            let mut k = self.supers;
            while k > 0 && *Self::col(self.snode, k - 1) >= r {
                *Self::col(self.snode, k - 1) += 1;
                k -= 1;
            }
        }
    }

    /// The dropped rows (`DEAD`) from row `from` on taken out (`native_cpu_stream::drop_rows`),
    /// then the blocks from there again.
    #[inline(always)]
    fn drop_from(&mut self, from: u32) {
        self.uncode(from);
        let outer = unsafe { *Self::col(self.open, self.nopen - 1) };
        self.drop_dead(from, outer);
        self.blocks(from >> STRIDE_LOG, self.nodes);
    }

    /// The dropped rows of rows `from..n` taken out, the rest moved down: first each row's
    /// count of dropped before it (`fbase`); then each kept row to its place, its size less the
    /// dropped in its range (`link`: a kept row's place, a dropped one's parent's). A dropped
    /// row's parent is the innermost row before it whose range holds it (a stack of them,
    /// `place`: each its row and its range's end), or else `outer`; it is marked (`DROPPED_IN`).
    /// Then each kept row's tail again, from its last descendant now; then the supertypes of
    /// the tail's nodes on to their rows, a dropped node's gone. A step a row.
    #[inline(never)]
    fn drop_dead(&mut self, from: u32, outer: u32) {
        let n = self.nodes;
        let (before, to, stack) = (self.fbase, self.link, self.place);
        unsafe {
            let mut d = 0u32;
            for i in from..n {
                *Self::col(before, i) = d;
                d += u32::from((*self.row(i)).kind == DEAD);
            }
            *Self::col(before, n) = d;
            let mut depth = 0u32;
            for i in from..n {
                while depth > 0 && *Self::col(stack, 2 * (depth - 1) + 1) <= i {
                    depth -= 1;
                }
                let p = if depth > 0 { *Self::col(to, *Self::col(stack, 2 * (depth - 1))) } else { outer };
                let mut size = u32::from(*self.size_c.add(i as usize));
                let e = (i + size).min(n);
                *Self::col(stack, 2 * depth) = i;
                *Self::col(stack, 2 * depth + 1) = e;
                depth += 1;
                if (*self.row(i)).kind == DEAD {
                    *Self::col(to, i) = p;$HEAD_DROPPED
                    continue;
                }
                let bi = *Self::col(before, i);
                let j = i - bi;
                *Self::col(to, i) = j;
                size -= *Self::col(before, e) - bi;
                *self.row(j) = *self.row(i);
                *self.size_c.add(j as usize) = size as SizeT;$HEAD_KEPT
            }
            let kept_n = n - d;
            let mut wide = 0u32;
            for j in (from..kept_n).filter(|_| !SIXTEEN) {
                let size = u32::from(*self.size_c.add(j as usize));
                let base = if FROM_START { j } else { j + size - 1 };
                let tl = (*self.row(j)).end.wrapping_sub((*self.row(base)).start);
                *self.tail_c.add(j as usize) = tl as TailT;
                wide |= u32::from(tl > TAIL_MAX) * WIDE_TAIL;
            }
            let mut k = self.supers;
            while k > 0 && *Self::col(self.snode, k - 1) >= from {
                k -= 1;
            }
            let mut kept = k;
            for s in k..self.supers {
                let at = *Self::col(self.snode, s);
                if *Self::col(before, at + 1) != *Self::col(before, at) {
                    continue;
                }
                *Self::col(self.snode, kept) = *Self::col(to, at);
                *Self::col(self.skind, kept) = *Self::col(self.skind, s);
                kept += 1;
            }
            self.supers = kept;
            if self.moved >= from && self.moved < n {
                self.moved = *Self::col(to, self.moved);
            }
            self.wide |= wide;
            self.nodes = kept_n;
        }
    }

    /// `RETAG m=2`: the node made last takes a kind; `DEAD` drops it.
    #[inline(always)]
    fn made_kind<const DROP: bool>(&mut self, kind: u16) -> R {
        let r = self.made()?;
        unsafe { (*self.row(r)).kind = kind };
        self.retag_at(r);
        if DROP {
            self.last_made = 0;
            self.drop_from(r);
        }
        Ok(())
    }

    #[inline(always)]
    fn made_field<const UNSET: bool>(&mut self, field: u16) -> R {
        let r = self.made()?;
        unsafe {
            let row = self.row(r);
            if !UNSET || (*row).field == 0 {
                (*row).field = field;
            }
        }
        Ok(())
    }

    /// The node a wrap made last holds first (`NODE_FIELD m=3`): the row after it, unless that
    /// is a wrap too.
    #[inline(always)]
    fn inner_field(&mut self, field: u16) -> R {
        let r = self.made()?;
        unsafe {
            if $INNER {
                return refuse(why::NODES);
            }
            (*self.row(r + 1)).field = field;
        }
        Ok(())
    }

    #[inline(always)]
    fn here_kind(&mut self, kind: u16) -> R {
        if self.nopen == 0 {
            return refuse(why::NODES);
        }
        let at = unsafe { *Self::col(self.open, self.nopen - 1) };
        unsafe { (*self.row(at)).kind = kind };
        self.retag_at(at);
        Ok(())
    }

    /// `RETAG m=1`: the node the last wrap moved takes a kind.
    #[inline(always)]
    fn moved_kind(&mut self, kind: u16) {
        unsafe { (*self.row(self.moved)).kind = kind };
        self.retag_at(self.moved);
    }

    /// `MERGE m=1`: the innermost open node's two leaves, all it holds, one; closed again.
    #[inline(always)]
    fn fold(&mut self, kind: u16) -> R {
        if self.nopen == 0 {
            return refuse(why::NODES);
        }
        let o = unsafe { *Self::col(self.open, self.nopen - 1) };
        if self.nodes != o + 3 || self.pending != 0 {
            return refuse(why::NODES);
        }
        let (a, b) = (o + 1, o + 2);
        unsafe {
            if *self.size_c.add(a as usize) != 1 || *self.size_c.add(b as usize) != 1 {
                return refuse(why::NODES);
            }
            let end = (*self.row(b)).end;
            (*self.row(a)).kind = kind;
            // The first may be committed (the root's two leaves, each closed at the top level).
            self.retag_at(a);
            (*self.row(a)).end = end;
            self.close_cells(a, 1, a, end);
        }
        self.nodes = b;
        self.cut_to(b);
        $FOLD_MADE
        Ok(())
    }

    /// `MERGE`: the leaf before the pending extras takes them as its text; closed again.
    #[inline(always)]
    fn merge(&mut self, kind: u16, end: u32) -> R {
        let kept = self.nodes - self.pending;
        if kept == 0 {
            return refuse(why::NODES);
        }
        let leaf = kept - 1;
        unsafe {
            if *self.size_c.add(leaf as usize) != 1 || (self.nopen != 0 && *Self::col(self.open, self.nopen - 1) >= leaf) {
                return refuse(why::NODES);
            }
            (*self.row(leaf)).kind = kind;
            self.retag_at(leaf);
            (*self.row(leaf)).end = end;
        }
        self.close_cells(leaf, 1, leaf, end);
        self.nodes = kept;
        if self.pending != 0 {
            self.pending = 0;
            self.cut_to(kept);
        }
        Ok(())
    }

    #[inline(always)]
    fn node_super(&mut self, kind: u32) {
        unsafe {
            *Self::col(self.snode, self.supers) = self.nodes - 1;
            *Self::col(self.skind, self.supers) = kind;
        }
        self.supers += 1;
    }
"#;
    let inner_test = if inner { "*Self::col(self.head, r) & WRAPPED == 0 || *Self::col(self.head, r + 1) & WRAPPED != 0" } else { "true" };
    // The current token's record: `next - 1`, but at the end, where `next` stays at the count;
    // with the end as a start (`sentinel`), `next` is one past it there too.
    let token_at = if x.sentinel { "self.next as usize - 1".to_owned() } else { format!("self.next as usize - usize::from(!{})", x.at_end("self")) };
    let s = src
        .replace("$TOKEN_AT", &token_at)
        .replace("$DROPPED_IN", dropped_in)
        .replace("$EARLY", early)
        .replace("$HEAD_N", &w("\n            *Self::col(self.head, n) = 0;"))
        .replace("$OPEN_MADE", &f("self.last_made = 0;"))
        .replace("$CLOSE_MADE", &f("self.last_made = at + 1;"))
        .replace("$LEAF_MADE", &f("if !DEFERRED && !EXTRA {\n            self.last_made = n + 1;\n        }"))
        .replace("$HEAD_MOVE", &w("\n                *Self::col(self.head, n) = *Self::col(self.head, r);"))
        .replace("$HEAD_WRAP", &w("\n            *Self::col(self.head, r) = WRAPPED;"))
        .replace("$HEAD_SHIFT", &w("\n                *Self::col(self.head, i) = *Self::col(self.head, i - 1);"))
        .replace("$HEAD_DROPPED", &w("\n                    *Self::col(self.head, p) |= DROPPED_IN;"))
        .replace("$HEAD_KEPT", &w("\n                *Self::col(self.head, j) = *Self::col(self.head, i);"))
        .replace("$INNER", inner_test)
        .replace("$FOLD_MADE", &f("self.last_made = b;"))
        .replace("$COMMIT_BODY", if x.sixteen { COMMIT16 } else { COMMIT15 })
        .replace("$RETAG_BODY", if x.sixteen { "\n        if at < self.passed {\n            self.retag16(at);\n        } else {\n            self.uncode(at);\n        }" } else { "\n        self.uncode(at);" })
        .replace("$UNCODE_BODY", if x.sixteen { UNCODE16 } else { UNCODE15 });
    // A node's supertypes (its `NODE_SUPER` records, in node order): a bit each of its key,
    // the class's place in `classes` (`native_cpu_tape::commit`).
    let (first, bits, all) = if x.classes.is_empty() {
        (String::new(), String::new(), String::new())
    } else {
        let arms: Vec<String> = x.classes.iter().enumerate().map(|(j, cl)| format!("{cl} => 1 << {}", 16 + j)).collect();
        // Node `at`'s records, among all (`retag16`, a cold path).
        let all = format!(
            "\n            let mut k = k;\n            while sk < self.supers {{\n                if *Self::col(self.snode, sk) == at {{\n                    k |= match *Self::col(self.skind, sk) {{ {}, _ => 0 }};\n                }}\n                sk += 1;\n            }}",
            arms.join(", ")
        );
        (
            "\n            let mut sk = self.supers;\n            while sk > 0 && *Self::col(self.snode, sk - 1) >= from {\n                sk -= 1;\n            }".to_string(),
            format!("\n                let mut k = k;\n                while sk < self.supers && *Self::col(self.snode, sk) == i {{\n                    k |= match *Self::col(self.skind, sk) {{ {}, _ => 0 }};\n                    sk += 1;\n                }}", arms.join(", ")),
            all,
        )
    };
    let s = cells16(x, &s);
    out.push_str(&s.replace("$CLASS_FIRST", &first).replace("$CLASS_BITS", &bits).replace(
        "$COMMIT_CLOSE",
        if x.sixteen && x.coded {
            "        // SAFETY: the writer's state, its room reserved for the run (`f16::Writer::reserve_run`).\n        unsafe { self.wr.close(u64::from(end)) };\n"
        } else if x.coded {
            ""
        } else {
            "        if self.nopen == 1 {\n            self.commit(at);\n        }\n"
        },
    ));
    if x.sixteen {
        out.push_str(&PASS16.replace("$CLASS_FIRST", &first).replace("$CLASS_BITS", &bits).replace("$CLASS_ALL", &all));
        let (open16, leaf16) = if x.coded {
            (
                "if !self.wr.open(number, u64::from(self.start)) {\n                return refuse(why::WRITER);\n            }",
                "if !self.wr.leaf(number, u64::from(self.start), u64::from(end)) {\n                return refuse(why::WRITER);\n            }",
            )
        } else {
            ("ops::w_open(self.writer, number, u64::from(self.start));\n            ops::w_commit(self.writer);", "ops::w_leaf(self.writer, number, u64::from(self.start), u64::from(end));\n            ops::w_commit(self.writer);")
        };
        out.push_str(&
            "    /// `OPEN` in a format 16 run (`Facts::sixteen`): the node passed to the writer's ops, opened by its
    /// number (the language's, a literal at the word: decision 0516) at the token's start, then committed,
    /// as `native_cpu_tape::pass` passes a node. Tier 2 keeps no row of it: its slot on the stack of open
    /// nodes holds the call number `keep_ret` puts there, as simdjson's stack holds a container's kind alone.
    #[inline(always)]
    fn open16(&mut self, number: u32) -> R {
        self.pending = 0;
        // SAFETY: the writer's state, its room reserved for the run (`f16::Writer::reserve_run`); the
        // stack's room is the bound's.
        unsafe {
            $OPEN16$PUSH16
        }$COUNT16
        Ok(())
    }

    /// `LEAF` in a format 16 run: the node passed to the writer's ops, its number and its bytes, then
    /// committed. No row.
    #[inline(always)]
    fn leaf16(&mut self, number: u32, end: u32) -> R {
        self.pending = 0;
        // SAFETY: as `open16`'s.
        unsafe {
            $LEAF16
        }$NODES16
        Ok(())
    }
"
            .replace("$OPEN16", open16)
            .replace("$LEAF16", leaf16)
            // A direct run's one stack is the writer's (D2): its frames hold the walk's slot, and its count the nodes.
            .replace("$PUSH16", if x.coded { "" } else { "\n            *Self::col(self.open, self.nopen) = 0;" })
            .replace("$COUNT16", if x.coded { "" } else { "\n        self.nopen += 1;\n        self.nodes += 1;" })
            .replace("$NODES16", if x.coded { "" } else { "\n        self.nodes += 1;" }),
        );
    } else if x.coded {
        out.push_str(
            "    /// A node's code as it is made (`Facts::coded`): its key's id (its kind, its field, its
    /// supertypes' classes), numbered by first use, as `commit_from` numbers them, in one pass.
    #[inline(always)]
    fn code_now(&mut self, kf: u32, classes: u32) {
        let i = self.nodes - 1;
        let k = (kf & 0x3ff) | ((kf >> 16) & 0x3f) << KIND_BITS | classes;
        unsafe {
            let slot = self.id_of.add(k as usize);
            let mut id = u32::from(*slot);
            if id == NO_CODE {
                id = self.ncodes;
                *slot = id as u16;
                *self.code_kf.add(id as usize) = k;
                *self.code_first.add(id as usize) = i;
                self.ncodes = id + 1;
            }
            *(self.image.add(HEADER_BYTES) as *mut u16).add(i as usize) = id as u16;
        }
    }
",
        );
    }
}

/// `ACCEPT`: the root closed at the file's end; no provisional field left; no value past its
/// width; the rest committed; the image's end (`native_cpu_tape::finish`): the columns moved
/// into place after the code column, the code table, the header.
fn accept(out: &mut String, x: &Facts<'_>) {
    let n = x.native;
    let provisional = if n.readings.provisional.is_empty() {
        String::new()
    } else {
        let fields: Vec<String> = n.readings.provisional.iter().map(u16::to_string).collect();
        format!(
            "        for i in 0..nodes {{\n            if matches!(unsafe {{ (*self.row(i)).field }}, {}) {{\n                return refuse(why::FIELD);\n            }}\n        }}\n",
            fields.join(" | ")
        )
    };
    let extras: Vec<String> = n.readings.extras.iter().map(u16::to_string).collect();
    let is_extra = if extras.is_empty() { "false".to_string() } else { format!("matches!(k & 0x3ff, {})", extras.join(" | ")) };
    let few = if x.widths.size == 2 { "\n        if nodes > 0xffff {\n            return refuse(why::WIDTH);\n        }" } else { "" };
    let src = r#"    /// `ACCEPT`.
    #[inline(never)]
    fn accept(&mut self) -> R {
        if !$AT_END || self.nopen != 1 {
            return refuse(why::NODES);
        }
        let nodes = self.nodes;
        let root = unsafe { *self.open };
        unsafe { (*self.row(root)).end = self.len };
        self.close_cells(root, nodes - root, nodes - 1, self.len);
$PROVISIONAL        if self.wide != 0 {
            return refuse(why::WIDTH);
        }$FEW
$COMMIT_ALL        let total = self.finish(nodes);
        unsafe {
            *self.counts_out = nodes as u64;
            *self.counts_out.add(1) = self.supers as u64;
            *self.counts_out.add(2) = total as u64;
        }
        Ok(())
    }

    /// `n` bytes from `src` to `dst`, 8 at a time (each column has bytes past its cells), then
    /// 16 zero bytes from `dst + n`: the column's padding. Gives back `dst + n`.
    #[inline(always)]
    unsafe fn put_column(dst: *mut u8, src: *const u8, n: usize) -> *mut u8 {
        unsafe {
            let mut i = 0;
            while i < n {
                (dst.add(i) as *mut u64).write_unaligned((src.add(i) as *const u64).read_unaligned());
                i += 8;
            }
            let at = dst.add(n);
            (at as *mut u64).write_unaligned(0);
            (at.add(8) as *mut u64).write_unaligned(0);
            at
        }
    }

    /// `at` rounded up to the columns' alignment, from the image's start.
    #[inline(always)]
    fn padded(&self, at: *mut u8) -> *mut u8 {
        let rel = at as usize - self.image as usize;
        unsafe { self.image.add((rel + COLUMN_ALIGN - 1) & !(COLUMN_ALIGN - 1)) }
    }

    /// The image's end; its length. The id table is cleared for the next file. A step a code.
    #[inline(never)]
    fn finish(&mut self, n: u32) -> usize {
        let n = n as usize;
        unsafe {
            let image = self.image;
            let code_end = image.add(HEADER_BYTES + 2 * n);
            (code_end as *mut u64).write_unaligned(0);
            (code_end.add(8) as *mut u64).write_unaligned(0);
            let mut at = self.padded(code_end);
            at = self.padded(Self::put_column(at, self.size_c as *const u8, n * core::mem::size_of::<SizeT>()));
            at = self.padded(Self::put_column(at, self.base_c as *const u8, 4 * n.div_ceil(STRIDE as usize)));
            at = self.padded(Self::put_column(at, self.off_c as *const u8, n * core::mem::size_of::<OffT>()));
            at = self.padded(Self::put_column(at, self.tail_c as *const u8, n * core::mem::size_of::<TailT>()));
            let codes = self.ncodes as usize;
            let mut e = at;
            for c in 0..codes {
                let key = *self.code_kf.add(c);
                let k = key as u16;
                (e as *mut u16).write_unaligned(k);
                *e.add(2) = if $IS_EXTRA { CODE_EXTRA } else { 0 };
                (e.add(3) as *mut u16).write_unaligned(SENTINEL);
                // Its classes, each at the next place, which moves on where the code has it.
                let mut place = e.add(6);
                let mut count = 0u8;$CLASS_PUT
                *e.add(5) = count;
                *self.id_of.add(key as usize) = NO_CODE as u16;
                e = place;
            }
            let table = e as usize - at as usize;
            let total = at.add(table) as usize - image as usize;
            (image as *mut u64).write_unaligned(MAGIC);
            (image.add(8) as *mut u32).write_unaligned(HEAD);
            (image.add(12) as *mut u32).write_unaligned(FORMAT_VERSION);
            (image.add(16) as *mut u64).write_unaligned(self.len as u64);
            for (k, v) in [n, codes, table, 0, 0].into_iter().enumerate() {
                (image.add(NODES_AT + 8 * k) as *mut u64).write_unaligned(v as u64);
            }
            total
        }
    }
"#;
    let put: String = x
        .classes
        .iter()
        .enumerate()
        .map(|(j, cl)| format!("\n                (place as *mut u16).write_unaligned({cl});\n                if key >> {} & 1 != 0 {{\n                    place = place.add(2);\n                    count += 1;\n                }}", 16 + j))
        .collect();
    let src = cells16(x, src);
    let src = if x.sixteen {
        // The root closed in the ops, and no image: the host finishes the writer (`CompiledStream::write16`).
        // Values past format 15's widths do not reach format 16.
        let end = if x.coded {
            "        // SAFETY: as `close`'s; the writer's state stored for the host's `finish`.\n        unsafe {\n            self.wr.close(u64::from(self.len));\n            self.wr.store(self.wr.n);\n        }\n        let total = 0;"
        } else {
            // The nodes not yet passed, and the root closed (`native_cpu_tape::pass`).
            "        self.pass16(self.passed, nodes, 0);\n        let total = 0;"
        };
        src.replace("$COMMIT_ALL        let total = self.finish(nodes);", end).replace("        if self.wide != 0 {\n            return refuse(why::WIDTH);\n        }", "")
    } else {
        src.to_string()
    };
    out.push_str(&src.replace("$COMMIT_ALL", if x.coded { "" } else { "        self.commit(nodes);\n" }).replace("$AT_END", &x.at_end("self")).replace("$PROVISIONAL", &provisional).replace("$FEW", few).replace("$IS_EXTRA", &is_extra).replace("$CLASS_PUT", &put));
}

/// A format 16 run's passes to the writer's ops where its words edit nodes made (decision 0434 §6, R17): the rows are
/// its window, and nodes reach the ops at its commit points, as `native_cpu_tape::pass` and `retag` pass them.
const PASS16: &str = "    /// Nodes `from..to` passed to the writer's ops in preorder (`native_cpu_tape::pass`): before each, and at `to`,
    /// the ops' open nodes whose subtrees end there closed at their ends (the bottom one, the root, kept where
    /// `keep`); each opened by its number (its key, as `commit_from` keys it) at its start, then committed. Nothing
    /// once an op failed; a key with no number is the ops' failure (`NUMBER`), which the host reads, as every op's.
    #[inline(never)]
    fn pass16(&mut self, from: u32, to: u32, keep: u32) {
        if self.bad16() {
            return;
        }
        // SAFETY: the writer's state (`ops::State`), its room reserved for the run; the rows and sizes of the nodes
        // made.
        unsafe {$CLASS_FIRST
            let mut i = from;
            loop {
                loop {
                    let st = self.writer as *const ops::State;
                    let len = (*st).open_len;
                    if len <= keep {
                        break;
                    }
                    let opens = self.writer.offset((*st).parts[ops::part::OPEN].at as isize);
                    let j = *(opens.add((len as usize - 1) * ops::OPEN as usize) as *const u32);
                    if j + *self.size_c.add(j as usize) as u32 > i {
                        break;
                    }
                    if ops::w_close(self.writer, u64::from((*self.row(j)).end)) != 0 {
                        return;
                    }
                }
                if i >= to {
                    break;
                }
                let key = *(self.row(i) as *const u32);
                let k = (key & 0x3ff) | ((key >> 16) & 0x3f) << KIND_BITS;$CLASS_BITS
                let number = *self.numbers.add(k as usize);
                if number == u32::MAX {
                    self.fail16(NUMBER, i);
                    return;
                }
                if ops::w_open(self.writer, number, u64::from((*self.row(i)).start)) != 0 || ops::w_commit(self.writer) != 0 {
                    return;
                }
                i += 1;
            }
        }
    }

    /// Node `at`, already passed to the ops and open there (the root), retagged as its row now is (`w_retag`), its
    /// number by its key as `pass16` takes it.
    #[inline(never)]
    #[cold]
    fn retag16(&mut self, at: u32) {
        if self.bad16() {
            return;
        }
        // SAFETY: as `pass16`'s.
        unsafe {
            let key = *(self.row(at) as *const u32);
            let k = (key & 0x3ff) | ((key >> 16) & 0x3f) << KIND_BITS;
            let mut sk = 0;$CLASS_ALL
            let number = *self.numbers.add(k as usize);
            if number == u32::MAX {
                self.fail16(NUMBER, at);
            } else {
                ops::w_retag(self.writer, at, number);
            }
        }
    }

    /// Whether an op failed (`ops::State::bad`).
    #[inline(always)]
    fn bad16(&self) -> bool {
        // SAFETY: the writer's state.
        unsafe { (*(self.writer as *const ops::State)).bad != 0 }
    }

    /// The run's failure where none came before (`w_fail`): its code (`ops::fail`) and node.
    #[cold]
    fn fail16(&mut self, code: u32, node: u32) {
        // SAFETY: the writer's state.
        unsafe { ops::w_fail(self.writer, code, node) };
    }

";

/// `commit`'s body in format 15: the codes numbered.
const COMMIT15: &str = "
        self.ncodes = self.commit_from(self.coded, to, self.ncodes);
        self.coded = to;";

/// In a format 16 run (decision 0434 §6, R17), as the JIT's (`Table::commit`): a commit point is not final (a fold
/// of the root's two closed leaves retags the one before), so the ops take nodes one commit point behind: those
/// from the last passed to the last commit point (`pass16`), the root and the kept child open.
const COMMIT16: &str = "
        self.pass16(self.passed, self.coded, 1);
        self.passed = self.passed.max(self.coded);
        self.coded = to;";

/// `uncode`'s body in format 15: the codes from `node` on taken back.
const UNCODE15: &str = "
        if node < self.coded {
            self.uncode_from(node);
        }";

/// In a format 16 run: the commit point moved back to the edit; an edit of a node the ops already have is the
/// run's failure (`fail::PASSED` in the JIT's), the file then the engine's.
const UNCODE16: &str = "
        if node < self.coded {
            if node < self.passed {
                self.fail16(PASSED, node);
            }
            self.coded = node;
        }";

/// The node words' templates of a format 16 run (`Facts::sixteen`): without format 15's cells (each node's offset
/// and its block's base, its size and tail), which only format 15's image and the edits read, and a format 16
/// run has neither (`coded` holds only where no word edits a node made). The ops write format 16's own.
fn cells16(x: &Facts<'_>, t: &str) -> String {
    if !(x.sixteen && x.coded) {
        return t.to_string();
    }
    let mut out = t.to_string();
    for line in [
        "        self.push_off(n, start, n + 1);\n",
        "        let kept = self.nodes - self.pending;\n        self.close_cells(at, kept - at, kept - 1, end);\n",
        "        self.close_cells(n, 1, n, end);\n",
        "        self.close_cells(root, nodes - root, nodes - 1, self.len);\n",
        // No rows (`open16`): a close and the root's end touch only the stack and the ops.
        "        let at = unsafe { *Self::col(self.open, self.nopen) };\n",
        "        unsafe { (*self.row(at)).end = end };\n",
        "        let root = unsafe { *self.open };\n        unsafe { (*self.row(root)).end = self.len };\n",
    ] {
        out = out.replace(line, "");
    }
    // One stack of open nodes, the writer's (D2): a call's number in its container's frame, not in a row; the
    // open nodes and the nodes made the writer's counts.
    for (from, to) in [
        ("unsafe { (*self.row(*Self::col(self.open, self.nopen - 1))).end = k };", "unsafe { self.wr.set_slot(k) };"),
        ("unsafe { (*self.row(*Self::col(self.open, self.nopen - 1))).end }", "unsafe { self.wr.slot() }"),
        ("        if !SURE && self.nopen == 0 {\n            return refuse(why::NODES);\n        }\n        self.nopen -= 1;\n", "        if !SURE && self.wr.open_len == 0 {\n            return refuse(why::NODES);\n        }\n"),
        ("|| self.nopen != 1 {\n            return refuse(why::NODES);\n        }\n        let nodes = self.nodes;\n", "|| self.wr.open_len != 1 {\n            return refuse(why::NODES);\n        }\n        let nodes = self.wr.n;\n"),
    ] {
        out = out.replace(from, to);
    }
    out
}

/// `RETAG m=3`: reading `i` over the node made last at the innermost level and the nodes inside
/// it, in preorder (its rows); a kind each, its field the reading's. The dropped are taken out
/// after (`drop_dead`).
fn reread(out: &mut String, i: usize, r: &crate::stream_ir::Reading) {
    let way = |w: u16| -> String {
        match w {
            KEEP => "{ i += u32::from(*self.size_c.add(i as usize)); continue; }".to_string(),
            GONE => "{ if self.dropped_in(i) { return refuse(why::NODES) } if i == root { self.last_made = 0 } (*row).kind = DEAD; gone = true; }".to_string(),
            REMEMBERED => "{ if self.dropped_in(i) || word == 0 { return refuse(why::NODES) } (*row).kind = word as u16; self.retag_at(i); }".to_string(),
            k => format!("{{ if self.dropped_in(i) {{ return refuse(why::NODES) }} (*row).kind = {k}; self.retag_at(i); }}"),
        }
    };
    let mut fields = String::new();
    for &(a, b) in &r.fields {
        write!(fields, "{a} => {b}, ").unwrap();
    }
    let mut kinds = String::new();
    let mut seen = BTreeSet::new();
    for &(k, w) in &r.kinds {
        if seen.insert(k) {
            writeln!(kinds, "                    {k} => {}", way(w)).unwrap();
        }
    }
    let root_way = match r.root {
        Some(w) => format!("if i == root {} else ", way(w)),
        None => String::new(),
    };
    writeln!(
        out,
        "    /// Reading {i} (`RETAG m=3`).
    #[inline(never)]
    fn reread_{i}(&mut self, word: u32) -> R {{
        let root = self.made()?;
        let end = self.nodes - self.pending;
        let mut gone = false;
        let mut i = root;
        while i < end {{
            unsafe {{
                let row = self.row(i);
                if i != root {{
                    (*row).field = match (*row).field {{ {fields}f => f }};
                }}
                {root_way}{{
                    match (*row).kind {{
{kinds}                        _ => return refuse(why::NODES),
                    }}
                }}
            }}
            i += 1;
        }}
        if gone {{
            self.drop_from(root);
        }}
        Ok(())
    }}
"
    )
    .unwrap();
}

/// A token mark's tokens (`Words::token_marks`): each element at its offset among the records
/// that are no extra (`extra_bit`: none for a grammar without extras). A step a record.
fn pattern(out: &mut String, i: usize, m: &TokenMark, extra_bit: Option<u8>) {
    // A record is an extra where its code has the extra's bit; with no extras, none is.
    let no_extra = |c: &str| match extra_bit {
        Some(b) => format!("{c} >> {} & 1 == 0", crate::records::MARK_SHIFT + u32::from(b)),
        None => "true".into(),
    };
    let free = no_extra("c");
    let mut body = String::new();
    let test = element_test;
    // The next record that is no extra, after `at`: past the last, `past`.
    let advance = |past: &str| format!("        let c = loop {{\n            at += 1;\n            if at >= self.n {{ return {past}; }}\n            let c = unsafe {{ *self.codes.add(at * self.rs) }};\n            if {free} {{ break c; }}\n        }};\n");
    pattern_body(out, i, m, &mut body, &free, &test, &advance);
}

/// A token element's test of a record's code `c` (`TokenElem::holds`): a kind of its set or a
/// word of its (a `not` element none), and its `with` and `without` marks.
fn element_test(el: &crate::program::TokenElem) -> String {
    let mut all = Vec::new();
    // Of no kinds and words, a `not` element is any record: no test of its choices.
    if el.kinds != 0 || el.rows != 0 || !el.words.is_empty() || !el.not {
        let mut t = Vec::new();
        if el.kinds != 0 || el.rows != 0 {
            // Not one of the kinds' hard keywords (`TokenElem::except`).
            let keyword = if el.except.is_empty() {
                String::new()
            } else {
                let x: Vec<String> = el.except.iter().map(|&s| (u32::from(s) + 1).to_string()).collect();
                format!(" && !matches!((c >> 8) & 0xff, {})", x.join(" | "))
            };
            // A kind of a shared first byte by its rows' start's mark bit (`TokenElem::rows`).
            let row = if el.rows != 0 { format!(" || c >> {} & {:#x} != 0", crate::records::MARK_SHIFT, el.rows) } else { String::new() };
            t.push(format!("(((c & 0xff) < 64 && {:#x}u64 >> (c & 0xff) & 1 != 0{row}){keyword})", el.kinds));
        }
        if !el.words.is_empty() {
            let w: Vec<String> = el.words.iter().map(|&s| (u32::from(s) + 1).to_string()).collect();
            t.push(format!("matches!((c >> 8) & 0xff, {})", w.join(" | ")));
        }
        let t = if t.is_empty() { "false".into() } else { t.join(" || ") };
        // A `not` element: none of its choices.
        // One `matches!` needs no parentheses to be negated.
        let one = t.starts_with("matches!(") && !t.contains(" || ");
        all.push(if el.not && one { format!("!{t}") } else if el.not { format!("!({t})") } else if all.is_empty() && el.without == 0 && el.with == 0 { t } else { format!("({t})") });
    }
    // `without M`: none of the marks' bits in the code (`TokenElem::without`).
    if el.without != 0 {
        all.push(format!("c >> {} & {:#x} == 0", crate::records::MARK_SHIFT, el.without));
    }
    // `with M`: all of the marks' bits in the code (`TokenElem::with`).
    if el.with != 0 {
        all.push(format!("c >> {} & {:#x} == {:#x}", crate::records::MARK_SHIFT, el.with, el.with));
    }
    if all.is_empty() { "true".into() } else { all.join(" && ") }
}

/// `pattern`'s body: the elements' tests in turn, and the run's.
fn pattern_body(out: &mut String, i: usize, m: &TokenMark, body: &mut String, free: &str, test: &dyn Fn(&crate::program::TokenElem) -> String, advance: &dyn Fn(&str) -> String) {
    // The token's own element, those before the run; the run (decision 0416) tries the rest
    // (`pattern_{i}_rest`) from each of its records on, as a forward scan.
    let run = m.elems.iter().position(|e| e.run);
    for el in m.elems.iter().take(run.unwrap_or(m.elems.len())) {
        match el.offset {
            0 => writeln!(body, "        let c = unsafe {{ *self.codes.add(here * self.rs) }};\n        if !({}) {{ return false; }}", test(el)).unwrap(),
            o if o < 0 => {
                // The record before, past the extras (an offset of -1). None before: an `end`
                // element (`after ~E`) holds there, any other fails.
                if el.end {
                    writeln!(body, "        let mut j = here;\n        let c = loop {{\n            if j == 0 {{ break None; }}\n            j -= 1;\n            let c = unsafe {{ *self.codes.add(j * self.rs) }};\n            if {free} {{ break Some(c); }}\n        }};\n        if let Some(c) = c {{ if !({}) {{ return false; }} }}", test(el)).unwrap();
                } else {
                    writeln!(body, "        let mut j = here;\n        let c = loop {{\n            if j == 0 {{ return false; }}\n            j -= 1;\n            let c = unsafe {{ *self.codes.add(j * self.rs) }};\n            if {free} {{ break c; }}\n        }};\n        if !({}) {{ return false; }}", test(el)).unwrap();
                }
            }
            _ => {
                // No record left: an `end` element holds there (it is the last), any other fails.
                body.push_str(&advance(if el.end { "true" } else { "false" }));
                writeln!(body, "        if !({}) {{ return false; }}", test(el)).unwrap();
            }
        }
    }
    let Some(r) = run else {
        writeln!(out, "    /// Token mark {i}'s tokens follow.\n    #[inline(never)]\n    fn pattern_{i}(&self) -> bool {{\n        let here = self.next as usize - 1;\n        let mut at = here;\n{body}        true\n    }}\n").unwrap();
        return;
    };
    let mut rest = String::new();
    for el in &m.elems[r + 1..] {
        rest.push_str(&advance(if el.end { "true" } else { "false" }));
        writeln!(rest, "        if !({}) {{ return false; }}", test(el)).unwrap();
    }
    // The run's depths (Q339): the rest only where each is 0; a closer at 0 ends the run.
    let run_el = &m.elems[r];
    let counters = run_el.nests.iter().map(|n| usize::from(n.counter) + 1).max().unwrap_or(0);
    let (depths, mut zero) = if counters == 0 { (String::new(), String::new()) } else { (format!("        let mut d = [0u32; {counters}];\n"), format!("d == [0; {counters}] && ")) };
    // Its `ending` (Q342): the record it took last, `at`, one the test takes.
    if let Some(x) = &run_el.ending {
        write!(zero, "{{ let c = unsafe {{ *self.codes.add(at * self.rs) }}; {} }} && ", test(x)).unwrap();
    }
    let mut nest = String::new();
    for n in &run_el.nests {
        let k = n.counter;
        if n.open {
            writeln!(nest, "            if {} {{ d[{k}] += 1; }}", test(&n.test)).unwrap();
        } else {
            writeln!(nest, "            if {} {{ if d[{k}] == 0 {{ return false; }} d[{k}] -= 1; }}", test(&n.test)).unwrap();
        }
    }
    writeln!(
        out,
        "    /// Token mark {i}'s tokens follow.\n    #[inline(never)]\n    fn pattern_{i}(&self) -> bool {{\n        let here = self.next as usize - 1;\n        let mut at = here;\n{body}{depths}        loop {{\n            if {zero}self.pattern_{i}_rest(at) {{ return true; }}\n    {}            if !({}) {{ return false; }}\n{nest}        }}\n    }}\n\n    /// Token mark {i}'s tokens after its run, from the record after `at`.\n    #[inline(always)]\n    fn pattern_{i}_rest(&self, mut at: usize) -> bool {{\n{rest}        true\n    }}\n",
        advance("false").replace("\n        ", "\n            "),
        test(run_el)
    )
    .unwrap();
}

/// A gap pattern's entry (`Words::gap_patterns`, decision 0451): each element the record at its
/// offset from the token's, as the lexer wrote them, extras included; its `starting`, the
/// record's first byte, as the lean lexer's `asi` reads the byte after a comment. At the
/// stream's end, no token: none holds.
fn gap_pattern(out: &mut String, i: usize, m: &TokenMark) {
    let mut body = String::new();
    for el in &m.elems {
        let o = el.offset;
        let none = if el.end { "true" } else { "false" };
        let at = if o < 0 { format!("here.checked_sub({})", -i32::from(o)) } else if o == 0 { "Some(here)".into() } else { format!("Some(here + {o}).filter(|&j| j < self.n)") };
        let j = if o == 0 { "        let j = here;\n".to_string() } else { format!("        let Some(j) = {at} else {{ return {none} }};\n") };
        body.push_str(&j);
        let test = element_test(el);
        if test != "true" {
            // A `~` of one `matches!`: its test, not negated twice.
            let fails = match test.strip_prefix('!').filter(|t| t.starts_with("matches!(") && !t.contains(" && ")) {
                Some(t) => t.to_string(),
                None => format!("!({test})"),
            };
            writeln!(body, "        let c = unsafe {{ *self.codes.add(j * self.rs) }};\n        if {fails} {{ return false; }}").unwrap();
        }
        // `starting`: the token's first byte, as the lean lexer's `asi` matches it.
        if el.first != [0; 4] {
            let bytes: Vec<String> = (0..=255u8).filter(|&b| el.first_holds(b)).map(|b| byte_literal(usize::from(b))).collect();
            writeln!(body, "        let b = unsafe {{ *self.bytes.add(*self.starts.add(j * self.rs) as usize) }};\n        if !matches!(b, {}) {{ return false; }}", bytes.join(" | ")).unwrap();
        }
    }
    writeln!(out, "    /// Gap pattern entry {i}'s records follow.\n    #[inline(never)]\n    fn gap_{i}(&self) -> bool {{\n        if self.at_end {{ return false; }}\n        let here = self.next as usize - 1;\n{body}        true\n    }}\n").unwrap();
}

/// A gap pattern's entries together, from `first`, as the token marks' (`patterns`).
fn gap_patterns(out: &mut String, first: usize, marks: &[TokenMark]) {
    let order = marks[first].order;
    let these: Vec<usize> = (first..marks.len()).take_while(|&i| marks[i].order == order).collect();
    let none: Vec<String> = these.iter().filter(|&&i| marks[i].unless).map(|i| format!("!self.gap_{i}()")).collect();
    let one: Vec<String> = these.iter().filter(|&&i| !marks[i].unless).map(|i| format!("self.gap_{i}()")).collect();
    let one = if one.is_empty() { "true".to_string() } else { format!("({})", one.join(" || ")) };
    let all: Vec<String> = none.into_iter().chain(std::iter::once(one)).collect();
    writeln!(out, "    /// Gap pattern {order}'s question: its entries together.\n    #[inline(always)]\n    fn gaps_{first}(&self) -> bool {{\n        {}\n    }}\n", all.join(" && ")).unwrap();
}

/// The token marks of one order together, from entry `first` (`program::patterns_hold`): none
/// of the `unless` ones and, where there are others, one of them.
fn patterns(out: &mut String, first: usize, marks: &[TokenMark]) {
    let order = marks[first].order;
    let these: Vec<usize> = (first..marks.len()).take_while(|&i| marks[i].order == order).collect();
    let none: Vec<String> = these.iter().filter(|&&i| marks[i].unless).map(|i| format!("!self.pattern_{i}()")).collect();
    let one: Vec<String> = these.iter().filter(|&&i| !marks[i].unless).map(|i| format!("self.pattern_{i}()")).collect();
    let one = if one.is_empty() { "true".to_string() } else { format!("({})", one.join(" || ")) };
    let all: Vec<String> = none.into_iter().chain(std::iter::once(one)).collect();
    writeln!(out, "    /// Mark {order}'s tokens follow: its entries together.\n    #[inline(always)]\n    fn patterns_{first}(&self) -> bool {{\n        {}\n    }}\n", all.join(" && ")).unwrap();
}

/// A local check's word form (`program::local_value` over the 64 places of a word, a bit
/// each): the values each op's needs, `(op, words back)`, a `Prev` reading the word before its
/// operand's too; and the byte sets' classes, tier 1's form (a class a bit of a byte, by two
/// nibble tables, `lo[b & 15] & hi[b >> 4]`, eight classes a pair of tables), each set's table
/// and class bits.
pub(crate) struct WordForm {
    pub need: BTreeSet<(usize, usize)>,
    pub tables: Vec<([u8; 16], [u8; 16])>,
    pub class_of: HashMap<usize, (usize, u8)>,
}

pub(crate) fn word_form(ops: &[LocalOp]) -> Result<WordForm, String> {
    let n = ops.len();
    let mut need: BTreeSet<(usize, usize)> = BTreeSet::new();
    let mut todo = vec![(n - 1, 0usize)];
    while let Some((k, t)) = todo.pop() {
        if !need.insert((k, t)) {
            continue;
        }
        match ops[k] {
            LocalOp::Not(a) => todo.push((usize::from(a), t)),
            LocalOp::And(a, b) | LocalOp::Or(a, b) | LocalOp::Xor(a, b) | LocalOp::AndNot(a, b) => todo.extend([(usize::from(a), t), (usize::from(b), t)]),
            LocalOp::Prev(a, _) => todo.extend([(usize::from(a), t), (usize::from(a), t + 1)]),
            _ => {}
        }
    }
    let mut tables: Vec<([u8; 16], [u8; 16])> = Vec::new();
    let mut class_of: HashMap<usize, (usize, u8)> = HashMap::new();
    let mut classes = 0usize;
    for (k, op) in ops.iter().enumerate() {
        let LocalOp::Bytes(set) = *op else { continue };
        let member = |b: usize| set[b / 64] >> (b % 64) & 1 == 1;
        // Each high nibble's low nibbles; each distinct nonempty set of them a class.
        let mut pats: Vec<u16> = Vec::new();
        let mut of_hi = [0u16; 16];
        for (h, slot) in of_hi.iter_mut().enumerate() {
            *slot = (0..16).filter(|&l| member(h * 16 + l)).fold(0u16, |m, l| m | 1 << l);
            if *slot != 0 && !pats.contains(slot) {
                pats.push(*slot);
            }
        }
        if pats.len() > 8 {
            return Err(format!("a byte set of {} classes", pats.len()));
        }
        if classes % 8 + pats.len() > 8 {
            classes = classes.div_ceil(8) * 8;
        }
        let mut bits = 0u8;
        for (pi, &pat) in pats.iter().enumerate() {
            let at = classes + pi;
            while tables.len() <= at / 8 {
                tables.push(([0; 16], [0; 16]));
            }
            let (lo, hi) = &mut tables[at / 8];
            let bit = 1u8 << (at % 8);
            for (l, cell) in lo.iter_mut().enumerate() {
                if pat >> l & 1 == 1 {
                    *cell |= bit;
                }
            }
            for h in 0..16 {
                if of_hi[h] == pat {
                    hi[h] |= bit;
                }
            }
            bits |= bit;
        }
        class_of.insert(k, (classes / 8, bits));
        classes += pats.len();
    }
    Ok(WordForm { need, tables, class_of })
}

/// A local check (`CHECK`): at the token, the check's mask's word at its start, one load; where
/// it has bits in the token's bytes, the check over each word of them, word-parallel.
fn check(out: &mut String, on_p: &mut String, i: usize, c: &LocalCheck, word: &str) -> Result<(), String> {
    let w = word_form(&c.ops).map_err(|e| format!("check {i}: {e}"))?;
    let (n, need, tables, class_of) = (c.ops.len(), w.need, w.tables, w.class_of);
    let backs: BTreeSet<usize> = need.iter().filter(|&&(k, _)| matches!(c.ops[k], LocalOp::Bytes(_))).map(|&(_, t)| t).collect();
    let mut body = String::new();
    for (ti, (lo, hi)) in tables.iter().enumerate() {
        writeln!(body, "        let lo{ti} = vld1q_u8({lo:?}.as_ptr());\n        let hi{ti} = vld1q_u8({hi:?}.as_ptr());").unwrap();
    }
    let used_tables: BTreeSet<usize> = class_of.values().filter(|&&(_, b)| b != 0).map(|&(t, _)| t).collect();
    for &t in &backs {
        writeln!(body, "        let b{t} = self.block(q - {t});").unwrap();
        for &ti in &used_tables {
            writeln!(body, "        let c{ti}_{t} = [0, 1, 2, 3].map(|v| vandq_u8(vqtbl1q_u8(lo{ti}, vandq_u8(b{t}[v], vdupq_n_u8(15))), vqtbl1q_u8(hi{ti}, vshrq_n_u8::<4>(b{t}[v]))));").unwrap();
        }
    }
    // Emit in dependency order: by op, then by word back, deepest first.
    let mut order: Vec<(usize, usize)> = need.iter().copied().collect();
    order.sort_by_key(|&(k, t)| (k, std::cmp::Reverse(t)));
    for (k, t) in order {
        let v = |a: u16, t: usize| format!("v{a}_{t}");
        let e = match c.ops[k] {
            LocalOp::Bytes(_) => match class_of[&k] {
                (_, 0) => "0".into(),
                (ti, bits) => format!("bits(c{ti}_{t}.map(|v| vtstq_u8(v, vdupq_n_u8({bits})))) & S::within(q - {t}, 0, self.len as i64)"),
            },
            LocalOp::Const(p) => format!("if q - {t} >= 0 {{ {p:#x}u64 }} else {{ 0 }}"),
            LocalOp::Within(d) => format!("S::within(q - {t}, {d}, self.len as i64 + {d})"),
            LocalOp::Not(a) => format!("!{}", v(a, t)),
            LocalOp::And(a, b) => format!("{} & {}", v(a, t), v(b, t)),
            LocalOp::Or(a, b) => format!("{} | {}", v(a, t), v(b, t)),
            LocalOp::Xor(a, b) => format!("{} ^ {}", v(a, t), v(b, t)),
            LocalOp::AndNot(a, b) => format!("{} & !{}", v(a, t), v(b, t)),
            LocalOp::Prev(a, s) => format!("({} << {s}) | (if q - {t} >= 1 {{ {} >> {} }} else {{ 0 }})", v(a, t), v(a, t + 1), 64 - u32::from(s)),
            LocalOp::Fill(..) | LocalOp::Start | LocalOp::Last | LocalOp::Ended => return Err(format!("check {i}: a token's check's op in a local check")),
        };
        writeln!(body, "        let v{k}_{t}: u64 = {e};").unwrap();
    }
    let (dq, dr) = (i64::from(c.delay) / 64, i64::from(c.delay) % 64);
    let at = if dr == 0 { format!("self.local_{i}(w as i64 + {dq})") } else { format!("(self.local_{i}(w as i64 + {dq}) >> {dr}) | (self.local_{i}(w as i64 + {}) << {})", dq + 1, 64 - dr) };
    writeln!(
        on_p,
        "    /// Local check {i}'s value over the 64 places of word `q`, a bit each.
    #[inline(always)]
    fn local_{i}(&self, q: i64) -> u64 {{
        unsafe {{
{body}            v{}_0
        }}
    }}

    /// Each word of the token's bytes with bits of the mask: the check at them, one AND.
    #[cold]
    #[inline(never)]
    fn check_{i}_words(&self, st: u32, lim: u32) -> R {{
        let (first, last) = (st >> 6, (lim - 1) >> 6);
        for w in first..=last {{
            let a = {word} & S::within(w as i64, st as i64, lim as i64);
            if a != 0 && a & ({at}) != 0 {{
                return refuse(why::CHECK);
            }}
        }}
        Ok(())
    }}
",
        n - 1
    )
    .unwrap();
    writeln!(
        out,
        "    /// `CHECK {i}`: most tokens have no bit of the mask from their start and end in its word.
    #[inline(always)]
    fn check_{i}(&self) -> R {{
        let (st, lim) = (self.start, self.rec_end);
        let w = st >> 6;
        if {word} >> (st & 63) == 0 && (lim - 1) >> 6 == w {{
            return Ok(());
        }}
        self.check_{i}_words(st, lim)
    }}

"
    )
    .unwrap();
    Ok(())
}

/// A word's successors as its operation names them (`succ` before the part's `TOKEN` and a
/// switch's sending its shared bytes on): its next, its targets, a call's two returns.
fn plain_succ(x: &Facts<'_>, pc: usize) -> Vec<usize> {
    let n = x.native.structural.len();
    let v = match x.native.structural[pc].operation {
        Op::Goto { target } => vec![usize::from(target)],
        Op::Switch { table } => x.native.switches[usize::from(table)].all_targets().filter(|&t| t != REFUSE).map(usize::from).collect(),
        Op::Word { test: true, .. } | Op::Mark { .. } | Op::Row { .. } | Op::Flag { op: FlagOp::Test, .. } => vec![pc + 1, pc + 2],
        // In one function (`OneFn`), only the places its rule's returns go back to.
        Op::Push { .. } if x.one.is_some() => x.one.as_ref().and_then(|o| o.after.get(&pc)).cloned().unwrap_or_default(),
        Op::Push { target, .. } => vec![usize::from(target), usize::from(target) + 1],
        Op::Return { .. } | Op::Accept | Op::Stop => vec![],
        _ => vec![pc + 1],
    };
    v.into_iter().filter(|&t| t < n).collect()
}

/// A word's successors in its function: its next, its targets, a call's two returns.
fn succ(x: &Facts<'_>, pc: usize) -> Vec<usize> {
    let n = x.native.structural.len();
    if let Some(fx) = x.fetch.as_ref().filter(|fx| fx.part_at(x, pc)) {
        // A region read by its method (`string_methods`): on at the closer's place alone.
        if x.strings.contains_key(&pc) {
            return vec![usize::from(past_span(x, part_facts(x, fx, pc).expect("checked").closer, 1))];
        }
        return part_plan(x, fx, pc).refs.into_iter().map(usize::from).filter(|&t| t < n).collect();
    }
    if let Some(one) = &x.one {
        match x.native.structural[pc].operation {
            Op::Push { .. } => return vec![crate::native_cpu_stream::callee(x.native, pc)],
            Op::Return { .. } => return one.returns.get(&pc).map_or(Vec::new(), |r| r.iter().map(|&(_, t)| t).collect()),
            _ => {}
        }
    }
    let v = match x.native.structural[pc].operation {
        // A node's field set by the next word, stored with its kind: on past that word.
        Op::Open { .. } | Op::Leaf { .. } if field_after(x, pc).is_some() => vec![pc + 2],
        Op::Goto { target } => vec![usize::from(target)],
        Op::Switch { table } => switch_plan(x, &x.native.switches[usize::from(table)]).refs.into_iter().map(usize::from).collect(),
        Op::Word { test: true, .. } | Op::Mark { .. } | Op::Row { .. } | Op::Flag { op: FlagOp::Test, .. } => vec![pc + 1, pc + 2],
        Op::Push { target, .. } => vec![usize::from(target), usize::from(target) + 1],
        Op::Return { .. } | Op::Accept | Op::Stop => vec![],
        _ => vec![pc + 1],
    };
    v.into_iter().filter(|&t| t < n).collect()
}

/// Whether rows carry marks (`head`): where a field is set on a wrap's inner node, or a reread
/// may meet a dropped row's place (`native_cpu_stream::TableFns` keeps the same).
pub(crate) fn marks(n: &Structure) -> bool {
    n.structural.iter().any(|b| matches!(b.operation, Op::NodeField { on: On::Inner, .. } | Op::Reread { .. }))
}

/// The field the word after a node's sets on it (`NODE_FIELD` on the node just made), stored
/// with the node's kind: one 32-bit store of both, as the JIT's (`with_field`).
fn field_after(x: &Facts<'_>, pc: usize) -> Option<u16> {
    match x.native.structural.get(pc + 1).map(|w| w.operation) {
        Some(Op::NodeField { field, on: On::Here }) => Some(field),
        _ => None,
    }
}

/// Whether a switch is on the record's kind (`records`): at offset 0, with records.
fn on_kinds(x: &Facts<'_>, sw: &ByteSwitch) -> bool {
    x.kind_reps.is_some() && sw.offset == 0
}

/// A switch's code with its branches as marks (`\u{1}k\u{1}`, its `k`th reference), and the
/// place each reference goes to: a word's successors, one an arm, so a place two arms share is
/// a merge of the graph, written once.
#[derive(Default)]
struct Plan {
    code: String,
    refs: Vec<u16>,
}

impl Plan {
    fn to(&mut self, t: u16) -> String {
        if t == REFUSE {
            return "return refuse(why::BYTE);".into();
        }
        self.refs.push(t);
        format!("\u{1}{}\u{1}", self.refs.len() - 1)
    }

    /// The code, each mark its branch's code.
    fn fill(&self, jump: &dyn Fn(usize) -> Result<String, String>) -> Result<String, String> {
        let mut out = String::with_capacity(self.code.len());
        for (i, part) in self.code.split('\u{1}').enumerate() {
            if i % 2 == 0 {
                out.push_str(part);
            } else {
                out.push_str(&jump(part.parse::<usize>().map_err(|e| e.to_string())?)?);
            }
        }
        Ok(out)
    }
}

/// A `match` over `values` (each value's place), `0..len`: the runs of a place one arm, the
/// place with the most runs the default where every value has one; else a value with no
/// place refuses.
fn arms(p: &mut Plan, scrutinee: &str, values: &[u16], name: &dyn Fn(usize) -> String) -> String {
    let mut by: BTreeMap<u16, Vec<(usize, usize)>> = BTreeMap::new();
    let mut i = 0;
    while i < values.len() {
        let mut j = i;
        while j + 1 < values.len() && values[j + 1] == values[i] {
            j += 1;
        }
        by.entry(values[i]).or_default().push((i, j));
        i = j + 1;
    }
    let mut s = format!("match {scrutinee} {{\n");
    let dflt = if by.contains_key(&REFUSE) { None } else { by.iter().max_by_key(|(_, r)| r.len()).map(|(&t, _)| t) };
    for (&t, runs) in &by {
        if t == REFUSE || Some(t) == dflt {
            continue;
        }
        let pats: Vec<String> = runs.iter().map(|&(a, b)| if a == b { name(a) } else { format!("{}..={}", name(a), name(b)) }).collect();
        let j = p.to(t);
        writeln!(s, "{} => {{\n{j}\n}}", pats.join(" | ")).unwrap();
    }
    match dflt {
        Some(t) => {
            let j = p.to(t);
            writeln!(s, "_ => {{\n{j}\n}}").unwrap();
        }
        None => s.push_str("_ => return refuse(why::BYTE),\n"),
    }
    s.push_str("}\n");
    s
}

/// A byte as a pattern: a printable ASCII byte as its literal (`b'.'`).
fn byte_literal(b: usize) -> String {
    match u8::try_from(b) {
        Ok(c @ (b'!'..=b'~')) if c != b'\'' && c != b'\\' => format!("b'{}'", c as char),
        Ok(b'\'') => "b'\\''".into(),
        Ok(b'\\') => "b'\\\\'".into(),
        _ => format!("{b:#04x}"),
    }
}

/// A switch's `match`: on the record's kind (a kind whose words the switch names, on its word
/// within its arm), or on a byte of the token.
fn switch_plan(x: &Facts<'_>, sw: &ByteSwitch) -> Plan {
    let mut p = Plan::default();
    // Where tier 2 makes the records, a token not made yet takes the switch on its first byte,
    // as simdjson's `switch (*value)`: each arm makes the token's record (its last byte, its
    // code) for its bytes, then goes on. A made one (a part, the stream's end, one switched on
    // again) takes the switch on its kind.
    let by_byte = match (&x.fetch, on_kinds(x, sw)) {
        (Some(fx), true) => Some(fetched_arms(x, fx, sw, &mut p)),
        _ => None,
    };
    // Reached only from the `TOKEN` before it, which takes a token: the token is made here, or
    // the stream has ended; no switch on its kind, as simdjson switches on a value once.
    if let Some(b) = &by_byte
        && fresh(x, sw)
    {
        // The end as a start refused by the byte switch (its byte past the file, 0, has no
        // arm): no test of the end, as simdjson's switch takes the index past the last.
        let refuses = sw.end == REFUSE || x.native.structural.get(usize::from(sw.end)).is_some_and(|w| matches!(w.operation, Op::Stop));
        if x.sentinel && refuses && x.fetch.as_ref().is_some_and(|fx| zero_refused(x, fx, sw)) {
            p.code = b.clone();
            return p;
        }
        let j = p.to(sw.end);
        p.code = format!("if {} {{\n{j}\n}} else {{\n{b}}}\n", x.at_end("s"));
        return p;
    }
    if let (true, Some(reps)) = (on_kinds(x, sw), &x.kind_reps) {
        let mut t: Vec<u16> = reps.iter().map(|&b| sw.targets[usize::from(b)]).collect();
        if x.end_kind.is_some() {
            t.push(sw.end);
        }
        let head = if x.end_kind.is_none() {
            let j = p.to(sw.end);
            format!("if {} {{\n{j}\n}} else ", x.at_end("s"))
        } else {
            String::new()
        };
        let mut by_word = String::new();
        let mut values = vec![REFUSE; 256];
        for (k, &tk) in t.iter().enumerate() {
            // A word whose target is its kind's needs no arm of its own. A part's own kind may be
            // any word, but only in a state that meets it (its byte's target no refusal): a state
            // outside its region never reads one (`bitlex_masks::row_modes`).
            let part = x.part_kinds.contains(&k) && tk != REFUSE;
            let named: Vec<(u16, u16)> = sw.words.iter().copied().filter(|&(w, to)| to != tk && k < reps.len() && (part || x.word_kind_of.get(usize::from(w)).is_some_and(|&y| usize::from(y) == k))).collect();
            if named.is_empty() {
                values[k] = tk;
                continue;
            }
            let mut inner = String::from("match s.word() {\n");
            for (w, to_w) in named {
                let j = p.to(to_w);
                writeln!(inner, "{} => {{ {j} }}", x.word(w)).unwrap();
            }
            let j = p.to(tk);
            writeln!(inner, "_ => {{ {j} }}\n}}").unwrap();
            writeln!(by_word, "{} => {{ {inner} }}", x.token(k)).unwrap();
        }
        let rest = arms(&mut p, "kind", &values, &|k| x.token(k));
        let body = if by_word.is_empty() { rest } else { format!("match kind {{\n{by_word}_ => {{ {rest} }}\n}}\n") };
        // The end's kind: the record's, or (the end as a start) the end's own.
        let kind = match x.end_kind {
            Some(k) if x.sentinel => format!("if {} {{ {k} }} else {{ s.code & 0xff }}", x.at_end("s")),
            _ => "s.code & 0xff".into(),
        };
        let by_kind = format!("{head}{{ let kind = {kind};\n{body}}}");
        p.code = match by_byte {
            Some(b) => format!("if s.rec_end == 0 && !{} {{\n{b}}} else {{\n{by_kind}}}\n", x.at_end("s")),
            None => by_kind,
        };
        return p;
    }
    let j = p.to(sw.end);
    let mut out = format!("if {} {{\n{j}\n}} else {{\n", x.at_end("s"));
    if sw.offset == 0 && !sw.words.is_empty() {
        out.push_str("match s.word() {\n");
        for &(w, t) in &sw.words {
            let j = p.to(t);
            writeln!(out, "{} => {{ {j} }}", x.word(w)).unwrap();
        }
        out.push_str("_ => {}\n}\n");
    }
    writeln!(out, "let at = s.start + {};\nif at >= s.rec_end {{ return refuse(why::SPAN); }}", sw.offset).unwrap();
    out.push_str(&arms(&mut p, "unsafe { *s.bytes.add(at as usize) }", &sw.targets, &byte_literal));
    out.push_str("}\n");
    p.code = out;
    p
}

/// A word's code, `jump(k)` its `k`th successor's.
fn word(x: &Facts<'_>, pc: usize, jump: &dyn Fn(usize) -> Result<String, String>) -> Result<String, String> {
    word_with(x, pc, jump, None)
}

/// `word`, the token's code known (`known`) where it is a constant: a keyword's leaf its kind,
/// as simdjson's `case 't'` writes its own tape word.
fn word_with(x: &Facts<'_>, pc: usize, jump: &dyn Fn(usize) -> Result<String, String>, known: Option<u32>) -> Result<String, String> {
    let n = x.native;
    let kind_of = |k: u16| -> String { if k == REMEMBERED { "{ if word == 0 { return refuse(why::WORD) } word as u16 }".into() } else { x.kind(k) } };
    // The token's end, as this word reads it: its phase's (`phases`), or the run's field.
    let taken = x.phase[pc] == crate::stream::TAKEN;
    let end = if x.flagged {
        "s.end"
    } else if taken {
        "s.rec_end"
    } else if x.sentinel {
        "(if s.start == s.len { s.rec_end } else { s.start })"
    } else {
        "(if s.at_end { s.rec_end } else { s.start })"
    };
    if let Some(name) = x.strings.get(&pc) {
        let at = if x.part_arg { "s.start + 1" } else { "" };
        return Ok(format!("s.{name}({at})?;\n{}", jump(0)?));
    }
    if let Some(fx) = x.fetch.as_ref().filter(|fx| fx.part_at(x, pc)) {
        return part_plan(x, fx, pc).fill(jump);
    }
    if let Some(one) = &x.one {
        match n.structural[pc].operation {
            // A call: its site on the stack, then the rule's first word (`object_begin`'s
            // `depth++`).
            Op::Push { .. } => {
                let k = one.sites.iter().find(|&&(p, _)| p == pc).expect("a site").1;
                if one.stacked {
                    // In its container's row, until the `CLOSE` before the return reads it.
                    return Ok(format!("if s.depth == DEPTH {{ return refuse(why::DEPTH); }}\ns.depth += 1;\ns.keep_ret({k});\n{}", jump(0)?));
                }
                return Ok(format!("if s.depth == DEPTH {{ return refuse(why::DEPTH); }}\ns.ret[s.depth as usize] = {k};\ns.depth += 1;\n{}", jump(0)?));
            }
            // A return: the site off the stack, then its place (`scope_end`).
            Op::Return { .. } => {
                let arms = one.returns.get(&pc).map_or(&[][..], Vec::as_slice);
                let mut m = String::from(if one.stacked { "s.depth -= 1;\nmatch ret {\n" } else { "s.depth -= 1;\nmatch s.ret[s.depth as usize] {\n" });
                for (i, &(k, _)) in arms.iter().enumerate() {
                    writeln!(m, "{k} => {{ {} }}", jump(i)?).unwrap();
                }
                // Every open call's site is one of these (`OneFn`).
                m.push_str("_ => return refuse(why::RETURN),\n}\n");
                return Ok(m);
            }
            _ => {}
        }
    }
    Ok(match n.structural[pc].operation {
        Op::Token { any, extras, .. } if x.flagged || taken => format!("s.{}()?;\n{}", token_name(any, extras), jump(0)?),
        // No token to take here: the next word switches on the one untaken.
        Op::Token { .. } => format!("{}{}", if x.unchecked { "" } else { "s.room()?;\n" }, jump(0)?),
        Op::Switch { table } => switch_plan(x, &n.switches[usize::from(table)]).fill(jump)?,
        Op::Span { last: false, len } => format!("if s.start + {len} != s.rec_end {{ return refuse(why::SPAN); }}\n{}{}", if x.flagged { "s.end = s.rec_end;\ns.need = true;\n" } else { "" }, jump(0)?),
        Op::Span { last: true, .. } => format!("{}{}", if x.flagged { "s.end = s.rec_end;\ns.need = true;\n" } else { "" }, jump(0)?),
        Op::Empty { total } => format!("if s.total({total}) != 0 {{ return refuse(why::EMPTY); }}\n{}", jump(0)?),
        Op::Word { string, test: true } => format!("if s.word() == {} {{\n{}\n}}{}", x.word(string), jump(0)?, or_else(&jump(1)?)),
        Op::Word { string, test: false } => format!("if s.word() != {} {{ return refuse(why::WORD); }}\n{}", x.word(string), jump(0)?),
        Op::Open { kind } => {
            let kf = u32::from(kind) | u32::from(field_after(x, pc).unwrap_or(0)) << 16;
            let code = if x.coded {
                let (field, classes) = key_after(x, pc);
                let kf = if field != 0 { kf & 0xffff | field } else { kf };
                if x.sixteen && x.coded {
                    // The node is the ops' alone: no row, its container's slot on the stack. Its number the
                    // language's, a literal; a key the table lacks refuses the file (the JIT's `fail::NUMBER`).
                    let Some(number) = x.number(kf, classes) else { return Ok(UNNUMBERED.into()) };
                    return Ok(format!("s.open16({number})?;\n{}", jump(0)?));
                }
                format!("s.code_now({kf}, {classes:#x});\n")
            } else {
                String::new()
            };
            // A node not coded where made: its kind and field by the grammar's names.
            let open = if code.is_empty() { x.kind_field(kind, u32::from(field_after(x, pc).unwrap_or(0)) << 16) } else { format!("{kf}") };
            format!("s.open({open});\n{code}{}", jump(0)?)
        }
        Op::Close { this } => {
            let sure = x.one.as_ref().is_some_and(|o| o.stacked);
            // The `CLOSE` before a `RETURN`: the call's number from its container's row first.
            let before = plain_succ(x, pc).iter().any(|&t| matches!(n.structural[t].operation, Op::Return { .. }));
            let take = if sure && before { "ret = s.taken_ret();\n" } else { "" };
            format!("{take}s.close::<{this}, {sure}>({})?;\n{}", if this { end } else { "0" }, jump(0)?)
        }
        Op::Leaf { kind, deferred, extra } => {
            let f = u32::from(field_after(x, pc).unwrap_or(0)) << 16;
            let constant = known.filter(|_| kind == WORD_KIND).map(|c| n.word_kinds.iter().find(|&&(w, _)| u32::from(w) + 1 == (c >> 8) & 0xff).map(|&(_, k)| u32::from(k)));
            let k = match (kind == WORD_KIND, f) {
                (true, _) if constant == Some(None) => return Ok("return refuse(why::WORD);".into()),
                (true, f) if constant.is_some() => format!("{}", constant.flatten().unwrap_or(0) | f),
                (true, 0) => "s.word_kind()?".to_string(),
                (true, f) => format!("s.word_kind()? | {f}"),
                (false, f) => x.kind_field(kind, f),
            };
            if x.coded {
                let (field, classes) = key_after(x, pc);
                let kf = if field != 0 { format!("kf & 0xffff | {field}") } else { "kf".into() };
                if x.sixteen && x.coded {
                    if deferred || extra {
                        return Err(format!("word {pc}: a deferred or extra leaf in a format 16 run"));
                    }
                    // The leaf's number the language's, a literal: its kind's, or a keyword's by its token's word, as
                    // the lean lexer's keyword `match` gives its kind.
                    let key = |kind: u32| if field != 0 { (kind | f) & 0xffff | field } else { kind | f };
                    if kind == WORD_KIND && constant.is_none() {
                        let mut m = String::from("let number = match s.word() {\n");
                        for &(w, k) in &n.word_kinds {
                            if let Some(number) = x.number(key(u32::from(k)), classes) {
                                writeln!(m, "{} => {number},", u32::from(w) + 1).unwrap();
                            }
                        }
                        m.push_str("_ => return refuse(why::WORD),\n};\n");
                        return Ok(format!("{m}s.leaf16(number, {end})?;\n{}", jump(0)?));
                    }
                    let kind = if kind == WORD_KIND { constant.flatten().unwrap_or(0) } else { u32::from(kind) };
                    let Some(number) = x.number(key(kind), classes) else { return Ok(UNNUMBERED.into()) };
                    return Ok(format!("s.leaf16({number}, {end})?;\n{}", jump(0)?));
                }
                let now = format!("s.code_now({kf}, {classes:#x});");
                return Ok(format!("let kf = {k};\ns.leaf::<{deferred}, {extra}>(kf, {end});\n{now}\n{}", jump(0)?));
            }
            format!("s.leaf::<{deferred}, {extra}>({k}, {end});\n{}", jump(0)?)
        }
        Op::Settle { .. } => format!("s.pending = 0;\n{}", jump(0)?),
        Op::SettleBefore { mark } => format!("s.settle_before({mark});\n{}", jump(0)?),
        // The next node made takes the dropped one's row and cells.
        // The extra pending last made again at the token, of no width: its row and cells.
        // Taken out, its kind kept: the `TOKEN` that takes the token makes it again after it.
        // The lean lexer's `asi`: a comment pending, then the question at the token.
        Op::LoseLast { pattern, flag } => match flag {
            Some(f) => format!("if flags & {} != 0 && s.pending != 0 && s.lost == 0 && s.gaps_{pattern}() {{\n    s.lose();\n}}\n{}", 1u32 << f, jump(0)?),
            None => format!("if s.pending != 0 && s.lost == 0 && s.gaps_{pattern}() {{\n    s.lose();\n}}\n{}", jump(0)?),
        },
        Op::DropLast { pattern } => format!("if s.pending != 0 && s.gaps_{pattern}() {{ s.pending -= 1; s.nodes -= 1; if s.pending == 0 {{ s.last_end = s.start; }} }}\n{}", jump(0)?),
        Op::Mark { mask, order, end, keep, word: mark_word, pattern } => {
            let k = u32::from(order);
            // A token mark past the code's (decision 0407): live at every token, its tokens
            // tested here; at the stream's end, held where the end has it.
            let here = if (k as usize) < x.in_code || pattern.is_some() {
                // In the record, folded into `live` at the token (the end's marks too).
                // A lazy gap mark with a pattern (`before`, decision 0416): its gap, then its token.
                let lazy = x.lazy.iter().any(|&(m, _)| u32::from(m) == k);
                match pattern {
                    Some(i) if lazy => format!("(if s.at_end {{ {end} }} else {{ s.lazy_{k}() && s.patterns_{i}() }})"),
                    Some(i) => format!("(if s.at_end {{ {end} }} else {{ s.patterns_{i}() }})"),
                    None if lazy => format!("(s.at_end || s.lazy_{k}())"),
                    None => "true".into(),
                }
            } else {
                let o = mask_output(n, mask)?;
                let w = mark_word.map_or(String::new(), |s| format!("s.word() == {} && ", x.word(s)));
                format!("(if s.at_end {{ {end} }} else {{ {w}s.bit({o}, s.start) }})")
            };
            let take = if keep { String::new() } else { format!("s.live &= !((2u64 << {k}) - 1);\n") };
            let here = if here == "true" { String::new() } else { format!(" && {here}") };
            format!("if s.live >> {k} & 1 != 0{here} {{\n{take}{}\n}}{}", jump(0)?, or_else(&jump(1)?))
        }
        Op::Row { mask } => format!("if s.bit({}, s.start) {{\n{}\n}}{}", mask_output(n, mask)?, jump(0)?, or_else(&jump(1)?)),
        Op::Retag { kind, on: On::Here } => format!("s.here_kind({})?;\n{}", x.kind(kind), jump(0)?),
        Op::Retag { kind, on: On::Moved } => format!("s.moved_kind({});\n{}", kind_of(kind), jump(0)?),
        Op::Retag { kind, on: On::Made } => format!("s.made_kind::<{}>({})?;\n{}", kind == DEAD, kind_of(kind), jump(0)?),
        Op::Reread { reading } => format!("s.reread_{reading}({})?;\n{}", if x.context { "word" } else { "0" }, jump(0)?),
        Op::Wrap { kind } => format!("s.wrap({})?;\n{}", x.kind(kind), jump(0)?),
        Op::Merge { kind, fold: true } => format!("s.fold({})?;\n{}", x.kind(kind), jump(0)?),
        Op::Merge { kind, fold: false } => format!("s.merge({}, {end})?;\n{}", x.kind(kind), jump(0)?),
        // In a format 16 run the node's field and supertypes are in its number, taken at its word (`key_after`).
        Op::NodeField { on: On::Here, .. } | Op::NodeSuper { .. } if x.sixteen && x.coded => jump(0)?,
        Op::NodeField { field, on: On::Here } => format!("unsafe {{ (*s.row(s.nodes - 1)).field = {field} }};\n{}", jump(0)?),
        Op::NodeField { field, on: On::Moved } => format!("unsafe {{ (*s.row(s.moved)).field = {field} }};\n{}", jump(0)?),
        Op::NodeField { field, on: On::Made } => format!("s.made_field::<false>({field})?;\n{}", jump(0)?),
        Op::NodeField { field, on: On::MadeUnset } => format!("s.made_field::<true>({field})?;\n{}", jump(0)?),
        Op::NodeField { field, on: On::Inner } => format!("s.inner_field({field})?;\n{}", jump(0)?),
        Op::NodeSuper { kind } => format!("s.node_super({kind});\n{}", jump(0)?),
        Op::Push { flag, .. } => {
            let callee = crate::native_cpu_stream::callee(n, pc);
            let args = if x.context {
                let f = match flag {
                    Some((k, true)) => format!("flags | {}", 1u32 << k),
                    Some((k, false)) => format!("flags & !{}", 1u32 << k),
                    None => "flags".into(),
                };
                format!(", word, {f}")
            } else {
                String::new()
            };
            format!("if s.deeper(move |s| {}(s{args}))? {{\n{}\n}}{}", x.function(callee), jump(1)?, or_else(&jump(0)?))
        }
        Op::Remember { kind: WORD_KIND } => format!("word = s.word_kind()?;\n{}", jump(0)?),
        Op::Remember { kind } => format!("word = {kind};\n{}", jump(0)?),
        Op::Flag { flag, op: FlagOp::Set } => format!("flags |= {};\n{}", 1u32 << flag, jump(0)?),
        Op::Flag { flag, op: FlagOp::Clear } => format!("flags &= !{};\n{}", 1u32 << flag, jump(0)?),
        Op::Flag { flag, op: FlagOp::Test } => format!("if flags & {} != 0 {{\n{}\n}}{}", 1u32 << flag, jump(0)?, or_else(&jump(1)?)),
        Op::Count { count, up: true, max } => format!("s.counts[{count}] += 1;\nif s.counts[{count}] > {max} {{ return refuse(why::COUNT); }}\n{}", jump(0)?),
        Op::Count { count, up: false, .. } => format!("s.counts[{count}] = s.counts[{count}].saturating_sub(1);\n{}", jump(0)?),
        Op::Check { check } => format!("s.check_{check}()?;\n{}", jump(0)?),
        Op::Return { again } => format!("return Ok({again});"),
        Op::Accept => "s.accept()?;\nreturn Ok(false);".into(),
        Op::Stop => "return refuse(why::STOP);".into(),
        Op::Goto { .. } => jump(0)?,
        ref op => return Err(format!("word {pc}: {op:?} is not tier 2's")),
    })
}

/// An `if`'s `else`: `else if …` where the branch is one `if` chain (as a hand-written chain of
/// tests reads), else `else { … }`.
fn or_else(e: &str) -> String {
    let t = e.trim();
    if t.starts_with("if ") && one_if(t) {
        format!(" else {t}")
    } else {
        format!(" else {{\n{e}\n}}")
    }
}

/// Whether `t`, an `if`, is one statement: every brace that closes at its outer level is followed
/// by `else` or the end.
fn one_if(t: &str) -> bool {
    let t = &no_literals(t);
    let mut depth = 0i32;
    for (i, c) in t.char_indices() {
        match c {
            '(' | '{' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    let rest = t[i + 1..].trim_start();
                    if !(rest.is_empty() || rest.starts_with("else")) {
                        return false;
                    }
                }
            }
            ';' if depth == 0 => return false,
            _ => {}
        }
    }
    depth == 0
}

/// `s` with each byte literal's contents left out (`b'{'` is `b''`): its braces are no code's.
fn no_literals(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut cs = s.chars().peekable();
    while let Some(c) = cs.next() {
        out.push(c);
        if c == 'b' && cs.peek() == Some(&'\'') {
            out.push(cs.next().expect("a quote"));
            while let Some(d) = cs.next() {
                if d == '\\' {
                    cs.next();
                } else if d == '\'' {
                    out.push(d);
                    break;
                }
            }
        }
    }
    out
}

/// The function of the rule entered at `entry`: its words reachable from there, structured.
fn rule(out: &mut String, x: &Facts<'_>, entry: usize) -> Result<(), String> {
    let mut nodes: Vec<usize> = vec![entry];
    let mut index: HashMap<usize, usize> = HashMap::from([(entry, 0)]);
    let mut k = 0;
    while k < nodes.len() {
        for t in succ(x, nodes[k]) {
            if let std::collections::hash_map::Entry::Vacant(v) = index.entry(t) {
                v.insert(nodes.len());
                nodes.push(t);
            }
        }
        k += 1;
    }
    let first: Vec<Vec<usize>> = nodes.iter().map(|&pc| succ(x, pc).iter().map(|t| index[t]).collect()).collect();
    let sizes = vec![1; nodes.len()];
    // The words whose code is their one jump and nothing else (a `GOTO`, a check decided when
    // tier 2 compiles): no dispatch's arm is only one (`Shape::threaded`).
    const MARK: &str = "\u{0}jump";
    let only_jump: Vec<bool> = (0..nodes.len()).map(|k| first[k].len() == 1 && word(x, nodes[k], &|j| Ok(format!("{MARK}{j}"))).is_ok_and(|c| c == format!("{MARK}0"))).collect();
    let shape = Shape::threaded(first, &sizes, &only_jump).ok_or_else(|| format!("rule at {entry}: an irreducible graph"))?;
    let emit = |node: usize, jump: &dyn Fn(usize) -> Result<String, String>| -> Result<String, String> { word(x, nodes[node], jump) };
    // Each label by its word: what the word does and its place (`'token_12: loop`, `break 'to_close_30;`), a
    // dispatch's by its node; a copy of a word's node its copy's number after.
    let name = |k: usize, looped: bool| -> String {
        let what = match shape.block_of(k) {
            Some(b) => {
                let pc = nodes[b];
                let copy = shape.copy_of(k);
                let role = label_role(x, pc);
                if copy == 0 { format!("{role}_{pc}") } else { format!("{role}_{pc}_{copy}") }
            }
            None => format!("entries_{k}"),
        };
        if looped { what } else { format!("to_{what}") }
    };
    // A dispatch's value by the place it goes on to, as the lean parser's `Label`s (`Label::ObjectContinue`): the
    // label's name in the type's case (`'close_153: loop` is `L::Close153`).
    let used = std::cell::RefCell::new(Vec::<String>::new());
    let label = |d: usize, k: usize| -> String {
        let place = name(shape.entry_of(d, k), true);
        let mut parts = place.split('_');
        let role = parts.next().unwrap_or_default();
        let mut variant: String = role.chars().take(1).flat_map(char::to_uppercase).chain(role.chars().skip(1)).collect();
        variant.push_str(&parts.collect::<Vec<_>>().join("_"));
        let mut u = used.borrow_mut();
        if !u.contains(&variant) {
            u.push(variant.clone());
        }
        format!("L::{variant}")
    };
    let body = shape.tree_labelled(&emit, 0, &name, &label)?;
    let used = used.into_inner();
    let declare = used.first().map_or(String::new(), |first| format!("let mut d = L::{first};\n"));
    {
        let mut all = x.labels.borrow_mut();
        for v in used {
            if !all.contains(&v) {
                all.push(v);
            }
        }
    }
    let params = if x.context { ", mut word: u32, mut flags: u32" } else { "" };
    // The call's number a `RETURN` matches on, from its container's row (`OneFn::stacked`).
    let ret = if x.one.as_ref().is_some_and(|o| o.stacked) { "let mut ret: u32 = 0;\n" } else { "" };
    out.push_str(&indented(&format!("/// The rule entered at word {entry}.\n#[inline(never)]\nfn {}(s: &mut S{params}) -> Result<bool, u32> {{\n{declare}{ret}{body}}}\n", x.function(entry)), 0));
    out.push('\n');
    Ok(())
}

/// Where tier 2 makes the records (`Records::fetched_in_tier2`): its facts; refused where a
/// first byte's rule is one this backend does not write (a pair of two bytes, a last-byte mask,
/// a region not read as `parse_string`, a run whose output is not a byte class), or a word
/// whose first byte begins a run is not one simdjson's atom test reads (eight bytes or fewer,
/// each of the run's class).
fn fetched(native: &Structure) -> Result<Option<Fetched>, String> {
    use crate::records::EndRule;
    let Some(ft) = crate::native_cpu_stream::fetch_of(native).map_err(|e| format!("records made in tier 2: {e:?}"))? else { return Ok(None) };
    let split = crate::native_cpu_stream::split_parts(native).map_err(|e| format!("a region tier 2 splits: {e:?}"))?;
    let seq = match split.and_then(|sp| sp.seq.clone()) {
        Some(q) => {
            let es: Vec<u8> = (0..=255u8).filter(|&b| q.stops[usize::from(b) / 64] >> (b % 64) & 1 == 1 && q.xs[usize::from(b) / 64] >> (b % 64) & 1 == 0).collect();
            let &[e0] = es.as_slice() else { return Err("a region read as `parse_string` whose part begins with more than one byte".into()) };
            if q.codes2.is_none() {
                return Err("a region read as `parse_string` with no part codes".into());
            }
            Some((q, e0))
        }
        None => None,
    };
    for b in 0..256usize {
        match ft.rules.get(usize::from(ft.ends[b])) {
            None => {}
            Some(r) if crate::native_cpu_stream::one_byte(&ft, r) => {}
            Some(&EndRule::Read(i)) if ft.seq[usize::from(i)] && seq.is_some() => {}
            Some(&EndRule::Run(_)) => {
                let class = ft.run_class[usize::from(ft.ends[b])].as_ref().ok_or_else(|| format!("byte {b:#04x}: a run whose output is not a byte class"))?;
                // A word with a byte out of the run's class is never the run's token.
                for (w, _) in ft.words.iter().filter(|(w, _)| w.first() == Some(&(b as u8)) && w.iter().all(|&y| class[usize::from(y)] == 1)) {
                    if w.len() > 8 {
                        return Err(format!("byte {b:#04x}: a word ({w:?}) that is not an atom of its run"));
                    }
                }
            }
            Some(r) => return Err(format!("byte {b:#04x}: records made in tier 2 by {r:?}: not yet ahead of time")),
        }
    }
    let seen = crate::native_cpu_stream::parts_pending_seen(native, &crate::native_cpu_stream::push_targets(native), &ft);
    Ok(Some(Fetched { ft, seq, seen }))
}

/// `TOKEN` where tier 2 makes the records: the next token's start (tier 1's structural index),
/// as simdjson's stage 2 takes the next index. Its record (its last byte, its code) is made by
/// the switch on its first byte (`fetched_arms`), so not made yet: `rec_end` 0.
fn fetched_token_step(out: &mut String, x: &Facts<'_>, any: Option<u8>) -> Result<(), String> {
    let name = token_name(any, false);
    let live = match any {
        Some(k) => format!("if self.bit({}, st) {{ u64::MAX }} else {{ 0 }}", mask_output(x.native, u16::from(k))?),
        None => "u64::MAX".into(),
    };
    let end_code = x.end_kind.map_or(String::new(), |k| format!("\n            self.code = {k};"));
    // The tape's room for the path to the next token, unless the tape is sized from the input.
    let room = if x.unchecked { "" } else { "        self.room()?;\n" };
    if x.sentinel {
        // `rec_end` 0 (not made yet) only where a switch has a kind path to tell it apart.
        // A part's switch is `part_plan`'s, which reads no `rec_end`.
        let part = |pc: usize| pc > 0 && x.fetch.as_ref().is_some_and(|fx| fx.part_at(x, pc - 1));
        let again = x.native.structural.iter().enumerate().any(|(pc, w)| matches!(w.operation, Op::Switch { table } if !part(pc) && on_kinds(x, &x.native.switches[usize::from(table)]) && !fresh(x, &x.native.switches[usize::from(table)])));
        let unmade = if again { "        self.rec_end = 0;\n" } else { "" };
        // Past the last token, the file's length as the start, the stream's two slots past its
        // count readable: a select, no branch; the byte there (0, padding) no switch takes.
        writeln!(
            out,
            "    /// `TOKEN`: the next token's start (past the last, the file's length, as simdjson's index
    /// past its last); its record is made where it is switched on.
    #[inline(always)]
    fn {name}(&mut self) -> R {{
{room}        self.last_end = self.rec_end;
        let i = self.next.min(self.n as u32);
        let st = unsafe {{ *self.starts.add(i as usize * self.rs) }};
        self.start = if i as usize == self.n {{ self.len }} else {{ st }};
{unmade}        self.next = i + 1;
        Ok(())
    }}
"
        )
        .unwrap();
        return Ok(());
    }
    writeln!(
        out,
        "    /// `TOKEN`: the next token's start; its record is made where it is switched on.
    #[inline(always)]
    fn {name}(&mut self) -> R {{
{room}        if self.at_end {{
            return refuse(why::END);
        }}
        self.last_end = self.rec_end;
        let i = self.next as usize;
        if i == self.n {{
            self.at_end = true;
            self.live = {:#x};{end_code}
            return Ok(());
        }}
        let st = unsafe {{ *self.starts.add(i * self.rs) }};
        self.start = st;
        self.rec_end = 0;
        self.next = i as u32 + 1;
        self.live = {live};
        Ok(())
    }}
",
        x.end_bits
    )
    .unwrap();
    Ok(())
}

/// A byte set's test on a vector, a lane each: one compare where it is one byte, else tier 1's
/// classes (two nibble tables, `word_form`), as simdjson's `classify`.
fn set_test(set: [u64; 4], v: &str, name: &str, consts: &mut String) -> Result<String, String> {
    let ones: Vec<u8> = (0..=255u8).filter(|&b| set[usize::from(b) / 64] >> (b % 64) & 1 == 1).collect();
    if let &[b] = ones.as_slice() {
        return Ok(format!("vceqq_u8({v}, vdupq_n_u8({b:#04x}))"));
    }
    let w = word_form(&[LocalOp::Bytes(set)])?;
    let (ti, bits) = w.class_of[&0];
    let (lo, hi) = w.tables[ti];
    writeln!(consts, "    const {name}_LO: [u8; 16] = {lo:?};\n    const {name}_HI: [u8; 16] = {hi:?};").unwrap();
    Ok(format!("vtstq_u8(vandq_u8(vqtbl1q_u8(vld1q_u8(Self::{name}_LO.as_ptr()), vandq_u8({v}, vdupq_n_u8(15))), vqtbl1q_u8(vld1q_u8(Self::{name}_HI.as_ptr()), vshrq_n_u8::<4>({v}))), vdupq_n_u8({bits}))"))
}

/// The helpers of the records tier 2 makes: a run's end, 16 bytes a vector (`run_R`); a
/// region's next stop, as `parse_string`'s `copy_and_find` (`part_stop`); the tables they and
/// the arms read.
/// A vector's lanes as bits, and the file's 64 bytes from a place as vectors.
const VECTORS: &str = "    /// The lanes of a vector that are all ones, four bits each (`vshrn`, as simdjson's
    /// NEON `to_bitmask` for one vector).
    #[inline(always)]
    fn nib(v: uint8x16_t) -> u64 {
        unsafe { vget_lane_u64::<0>(vreinterpret_u64_u8(vshrn_n_u16::<4>(vreinterpretq_u16_u8(v)))) }
    }

    /// The lanes of a vector that are all ones, a bit each (16 bits).
    #[inline(always)]
    fn bits16(v: uint8x16_t) -> u64 {
        unsafe {
            let m = vandq_u8(v, vld1q_u8([1u8, 2, 4, 8, 16, 32, 64, 128, 1, 2, 4, 8, 16, 32, 64, 128].as_ptr()));
            u64::from(vaddv_u8(vget_low_u8(m))) | u64::from(vaddv_u8(vget_high_u8(m))) << 8
        }
    }

    /// The 64 bytes from `at`, four vectors, zeros past the file (and before it none: `at` is in it).
    #[inline(always)]
    fn window(&self, at: u32) -> [uint8x16_t; 4] {
        unsafe {
            let left = self.len as i64 - at as i64;
            if left <= 0 {
                let z = vdupq_n_u8(0);
                return [z, z, z, z];
            }
            let p = self.bytes.add(at as usize);
            let v = [vld1q_u8(p), vld1q_u8(p.add(16)), vld1q_u8(p.add(32)), vld1q_u8(p.add(48))];
            if left >= 64 {
                return v;
            }
            let iota = vld1q_u8([0u8, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15].as_ptr());
            let l = vdupq_n_u8(left as u8);
            [0u8, 1, 2, 3].map(|j| vandq_u8(v[usize::from(j)], vcltq_u8(vaddq_u8(iota, vdupq_n_u8(16 * j)), l)))
        }
    }

";

fn fetched_helpers(out: &mut String, x: &Facts<'_>, fx: &Fetched) -> Result<(), String> {
    let ft = &fx.ft;
    let mut consts = String::new();
    out.push_str(
        "    /// The 8 bytes from `at`, a word (the file has 64 readable bytes after it).
    #[inline(always)]
    fn word8(&self, at: u32) -> u64 {
        unsafe { (self.bytes.add(at as usize) as *const u64).read_unaligned() }
    }

    #[inline(always)]
    fn byte(&self, at: u32) -> u8 {
        unsafe { *self.bytes.add(at as usize) }
    }


",
    );
    // Each run whose bytes some first byte begins: its end, and its class as a table of bytes.
    let mut runs: BTreeSet<u8> = BTreeSet::new();
    for b in 0..256usize {
        if let Some(crate::records::EndRule::Run(_)) = ft.rules.get(usize::from(ft.ends[b])) {
            runs.insert(ft.ends[b]);
        }
    }
    for &r in &runs {
        let class = ft.run_class[usize::from(r)].as_ref().expect("checked in `fetched`");
        let mut set = [0u64; 4];
        for b in 0..256usize {
            if class[b] == 0 {
                set[b / 64] |= 1 << (b % 64);
            }
        }
        let test = set_test(set, "v", &format!("RUN_{r}_END"), &mut consts)?;
        let table: Vec<String> = class.iter().map(u8::to_string).collect();
        writeln!(consts, "    /// Rule {r}'s run's bytes (1: in it), as simdjson's `structural_or_whitespace` table.\n    const CLASS_{r}: [u8; 256] = [{}];", table.join(", ")).unwrap();
        writeln!(
            out,
            "    /// Rule {r}'s token's last byte from its first at `s`: the run of its class after it
    /// (`EndRule::Run`), 16 bytes a vector; the file's end ends it.
    #[inline(always)]
    fn run_{r}(&self, s: u32) -> u32 {{
        let mut p = s + 1;
        loop {{
            let left = self.len - p;
            let v = unsafe {{ vld1q_u8(self.bytes.add(p as usize)) }};
            let mut stop = Self::nib(unsafe {{ {test} }});
            if left < 16 {{
                stop |= u64::MAX << (4 * left);
            }}
            if stop != 0 {{
                return p + stop.trailing_zeros() / 4 - 1;
            }}
            p += 16;
        }}
    }}
"
        )
        .unwrap();
    }
    // Each run's token and the check at its place, from one read (`run_check`).
    let mut pairs: BTreeSet<(u8, usize)> = BTreeSet::new();
    for sw in x.native.switches.iter().filter(|sw| on_kinds(x, sw)) {
        for b in 0..256usize {
            if let Some(crate::records::EndRule::Run(_)) = ft.rules.get(usize::from(ft.ends[b]))
                && let Some((_, ci, _)) = check_at(x, kind_target(x, fx, sw, u32::from(ft.kinds[b])))
            {
                pairs.insert((ft.ends[b], ci));
            }
        }
    }
    for (r, ci) in pairs {
        run_check(out, r, ft.run_class[usize::from(r)].as_ref().expect("checked in `fetched`"), ci, &x.native.local_checks[ci])?;
    }
    if let Some((q, _)) = &fx.seq {
        if q.stops[0] & 1 != 0 {
            return Err("a region whose stops hold 0, the padding past the file".into());
        }
        let parts: [u64; 4] = std::array::from_fn(|k| q.stops[k] & !q.xs[k]);
        let closers = set_test(q.xs, "v", "CLOSERS", &mut consts)?;
        let parts = set_test(parts, "v", "PARTS", &mut consts)?;
        let codes2 = q.codes2.as_ref().expect("checked in `fetched`");
        let t: Vec<String> = codes2.iter().map(|c| format!("{c:#x}")).collect();
        writeln!(consts, "    /// A part's code by its second byte (`records::pass` of its two bytes), as simdjson's `escape_map`.\n    const CODES2: [u32; 256] = [{}];", t.join(", ")).unwrap();
        writeln!(
            out,
            "    /// The closers' bytes of a vector, and the bytes that begin a part of two (`\\`).
    #[inline(always)]
    fn closers(v: uint8x16_t) -> uint8x16_t {{
        unsafe {{ {closers} }}
    }}

    #[inline(always)]
    fn parts(v: uint8x16_t) -> uint8x16_t {{
        unsafe {{ {parts} }}
    }}

    /// Two vectors' lanes that are all ones, four bits each (`vshrn`, then `vshrn2`).
    #[inline(always)]
    fn nib32(a: uint8x16_t, b: uint8x16_t) -> u128 {{
        unsafe {{
            let n = vreinterpretq_u64_u8(vshrn_high_n_u16::<4>(vshrn_n_u16::<4>(vreinterpretq_u16_u8(a)), vreinterpretq_u16_u8(b)));
            u128::from(vgetq_lane_u64::<0>(n)) | u128::from(vgetq_lane_u64::<1>(n)) << 64
        }}
    }}

    /// The region's next stop from `at`, a clean place: the first byte of a closer or a part,
    /// 32 bytes a step, and whether it is a closer, from the two masks, as `parse_string`'s
    /// `copy_and_find` and its `has_quote_first`. None before the file's end refuses the file
    /// (the region open there).
    #[inline(always)]
    fn part_stop(&self, at: u32) -> Result<(u32, bool), u32> {{
        let mut p = at;
        loop {{
            if p >= self.len {{
                return refuse(why::STRING);
            }}
            // Past the file the bytes are 0 (its padding), no stop's: no mask, as simdjson's.
            let (a, b) = unsafe {{ (vld1q_u8(self.bytes.add(p as usize)), vld1q_u8(self.bytes.add(p as usize + 16))) }};
            let c = Self::nib32(Self::closers(a), Self::closers(b));
            let m = c | Self::nib32(Self::parts(a), Self::parts(b));
            if m != 0 {{
                let first = m & m.wrapping_neg();
                return Ok((p + m.trailing_zeros() / 4, c & first != 0));
            }}
            p += 32;
        }}
    }}
"
        )
        .unwrap();
    }
    let _ = x;
    out.push_str(&consts);
    out.push('\n');
    Ok(())
}

/// A token's place by the switch's kind path (`switch_plan`): its kind's (its byte's first,
/// `kind_reps`), or its word's where the switch names it.
fn kind_target(x: &Facts<'_>, fx: &Fetched, sw: &ByteSwitch, code: u32) -> u16 {
    let w = (code >> 8) & 0xff;
    if w > 0
        && let Some(&(_, t)) = sw.words.iter().find(|&&(y, _)| u32::from(y) + 1 == w)
    {
        return t;
    }
    let kind = (code & 0xff) as usize;
    let rep = x.kind_reps.as_ref().and_then(|r| r.get(kind)).map_or_else(|| fx.ft.kinds.iter().position(|&k| usize::from(k) == kind).unwrap_or(0), |&b| usize::from(b));
    sw.targets[rep]
}

/// The arms of a switch on a token's first byte, where tier 2 makes the records: bytes alike in
/// rule, code and place one arm, each making the token's record for its bytes:
/// - a token of one byte (`{`, `,`): its code and place known, taken at once (simdjson's
///   `case ','`);
/// - a region's opener (`"`): the token is the opener alone, its parts pending after it;
/// - a run (a number's bytes): each word that begins with its bytes as simdjson's atom test
///   (`str4ncmp(value, "true")`, then the byte after not in the run's class), then the run's end.
fn fetched_arms(x: &Facts<'_>, fx: &Fetched, sw: &ByteSwitch, p: &mut Plan) -> String {
    use crate::records::EndRule;
    let ft = &fx.ft;
    // A switch whose arms are another's and some of its own (an array's first value, which also
    // takes `]`, and its value after a `,`): its own arms, then the other's switch for every other
    // byte, as `walk_document` peeks for `]` before the one value switch (`delegate_of`).
    let delegate = delegate_of(x, fx, sw);
    // (form, code or rule, kind, place): 0 one byte, 1 an opener, 2 a run.
    let mut groups: BTreeMap<(u8, u32, u32, u16), Vec<usize>> = BTreeMap::new();
    for b in 0..256usize {
        let named = sw.words.iter().any(|&(y, _)| ft.words.get(usize::from(y)).is_some_and(|(w, _)| w.first() == Some(&(b as u8))));
        let key = match ft.rules.get(usize::from(ft.ends[b])) {
            Some(&EndRule::Run(_)) => (2, u32::from(ft.ends[b]), u32::from(ft.kinds[b]), kind_target(x, fx, sw, u32::from(ft.kinds[b]))),
            Some(&EndRule::Read(_)) => (1, ft.codes1[b], 0, kind_target(x, fx, sw, ft.codes1[b])),
            _ => (0, ft.codes1[b], 0, kind_target(x, fx, sw, ft.codes1[b])),
        };
        if key.3 == REFUSE && !named {
            continue;
        }
        if let Some((_, other)) = delegate
            && byte_sig(x, fx, other, b) == byte_sig(x, fx, sw, b)
        {
            continue;
        }
        groups.entry(key).or_default().push(b);
    }
    let mut s = String::from("match s.byte(s.start) {\n");
    for (&(form, a, kind, t), bytes) in &groups {
        let mut pats: Vec<String> = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            let mut j = i;
            while j + 1 < bytes.len() && bytes[j + 1] == bytes[j] + 1 {
                j += 1;
            }
            pats.push(if i == j { format!("{:#04x}", bytes[i]) } else { format!("{:#04x}..={:#04x}", bytes[i], bytes[j]) });
            i = j + 1;
        }
        // A one-byte token (punctuation, an opener): its place's test of that (`SPAN`) holds,
        // and is not made again (`past_span`).
        let one = past_span(x, t, 1);
        let body = match form {
            // Its end only where a word reads it (`end_read`).
            0 => format!("{}{}{}", if end_read(x, one) { "s.rec_end = s.start + 1;\n" } else { "" }, code_set(x, &[one], a), p.to(one)),
            // The opener's place: passed at the method's call where `part_arg` holds, else kept for the part.
            1 => format!("{}{}{}{}", if end_read(x, one) { "s.rec_end = s.start + 1;\n" } else { "" }, code_set(x, &[one], a), if x.part_arg { "" } else { "s.part_at = s.start + 1;\n" }, p.to(one)),
            _ => {
                let r = a;
                let (mut tests, mut arms) = (String::new(), String::new());
                let class = ft.run_class[r as usize].as_ref().expect("checked in `fetched`");
                for (w, code) in ft.words.iter().filter(|(w, _)| w.first().is_some_and(|f| bytes.contains(&usize::from(*f))) && w.iter().all(|&y| class[usize::from(y)] == 1)) {
                    let n = w.len();
                    let mask = if n == 8 { u64::MAX } else { (1u64 << (8 * n)) - 1 };
                    let want = w.iter().rev().fold(0u64, |acc, &y| acc << 8 | u64::from(y));
                    let at = match check_at(x, kind_target(x, fx, sw, *code)) {
                        // A check of no delay at the word's place reads the word's bytes alone:
                        // decided here, as simdjson tests an atom by its bytes and nothing else.
                        Some((q, _, c)) if c.delay == 0 => {
                            if crate::program::token_value(c, w, 0, w.len(), &|_| false) {
                                REFUSE
                            } else {
                                q as u16 + 1
                            }
                        }
                        _ => kind_target(x, fx, sw, *code),
                    };
                    // Its leaf written in its arm, its kind a constant (`known_leaf`).
                    let (to, set) = match known_leaf(x, at, *code).ok().flatten() {
                        Some((code_text, _)) if code_text.ends_with(UNNUMBERED) => (code_text, String::new()),
                        Some((code_text, past)) => (format!("{code_text}{}", p.to(past)), code_set(x, &[past], *code)),
                        None => (p.to(at), code_set(x, &[at], *code)),
                    };
                    let k = arms.matches("\n// \"").count() + 1;
                    let name = format!("{:?}", String::from_utf8_lossy(w));
                    writeln!(tests, "// {name}\nif st + {n} <= s.len && s.word8(st) & {mask:#x} == {want:#x} && (st + {n} == s.len || S::CLASS_{r}[usize::from(s.byte(st + {n}))] == 0) {{\nreturn {k};\n}}").unwrap();
                    writeln!(arms, "{k} => {{\n// {name}\ns.rec_end = st + {n};\n{set}{to}\n}}").unwrap();
                }
                // The words, one method that names the one the token is (as simdjson's `is_valid_true_atom` and its
                // kin), its arms here.
                let atom = (!tests.is_empty()).then(|| {
                    x.method("atom", format!("/// Which of its run's words the token at `st` is, one load of its bytes each (as simdjson's atom tests): one more than its place, 0 for none.\n\u{0}(&self, st: u32) -> u32 {{\nlet s = self;\n{tests}0\n}}"))
                });
                let words = |rest: String| match &atom {
                    Some(m) => format!("match s.{m}(st) {{\n{arms}_ => {{\n{rest}\n}}\n}}"),
                    None => rest,
                };
                match check_at(x, t).filter(|_| t != REFUSE) {
                    // The check at its place made in the pass that finds its end, then on past it.
                    Some((q, ci, _)) => {
                        let (sp, sl) = (code_set(x, &[q as u16 + 1], kind), code_set(x, &[t], kind));
                        // The place's words on to past the check, each with one way (`straight_back`): the run, its
                        // end and its check one method, as simdjson's `parse_number`, then on past the check.
                        let check = straight_back(x, usize::from(t), q + 1).filter(|_| sp == sl).and_then(|ws| ws.iter().map(|&w| word(x, w, &|_| Ok(String::new())).ok()).collect::<Option<String>>());
                        match check {
                            Some(check) => {
                                let m = x.method("number", format!("/// The token at `st` read to its end and checked in one pass (`run_{r}_check_{ci}`), as simdjson's `parse_number`; where that pass cannot tell, its end by the run, then the check.\n\u{0}(&mut self, st: u32) -> R {{\nlet s = self;\nmatch s.run_{r}_check_{ci}(st)? {{\nSome(l) => s.rec_end = l + 1,\nNone => {{\ns.rec_end = s.run_{r}(st) + 1;\n{check}}}\n}}\nOk(())\n}}"));
                                let past = p.to(q as u16 + 1);
                                format!("let st = s.start;\n{}", words(format!("s.{m}(st)?;\n{sp}{past}")))
                            }
                            None => {
                                let past = p.to(q as u16 + 1);
                                let long = p.to(t);
                                format!("let st = s.start;\n{}", words(format!("match s.run_{r}_check_{ci}(st)? {{\nSome(l) => {{\ns.rec_end = l + 1;\n{sp}{past}\n}}\nNone => {{\ns.rec_end = s.run_{r}(st) + 1;\n{sl}{long}\n}}\n}}")))
                            }
                        }
                    }
                    // A token no word of it is refuses: its end is not made.
                    None if t == REFUSE => format!("let st = s.start;\n{}", words("return refuse(why::BYTE);".into())),
                    None => format!("let st = s.start;\n{}", words(format!("s.rec_end = s.run_{r}(st) + 1;\n{}{}", code_set(x, &[t], kind), p.to(t)))),
                }
            }
        };
        writeln!(s, "{} => {{\n{body}\n}}", pats.join(" | ")).unwrap();
    }
    // No byte of its own: the switch is its default alone, no `match`.
    if groups.is_empty() {
        return match delegate {
            Some((at, _)) => format!("{}\n", p.to(at)),
            None => "return refuse(why::BYTE);\n".into(),
        };
    }
    match delegate {
        Some((at, _)) => {
            let j = p.to(at);
            writeln!(s, "_ => {{\n{j}\n}}\n}}").unwrap();
        }
        None => s.push_str("_ => return refuse(why::BYTE),\n}\n"),
    }
    s
}

/// What a switch does with a token of first byte `b` (`fetched_arms`): its arm's form, code and
/// rule, and each place it can go to (a word's that begins with `b`, then the rest's). None: it
/// refuses it.
fn byte_sig(x: &Facts<'_>, fx: &Fetched, sw: &ByteSwitch, b: usize) -> Option<(u8, u32, Vec<u16>)> {
    use crate::records::EndRule;
    let ft = &fx.ft;
    let named = sw.words.iter().any(|&(y, _)| ft.words.get(usize::from(y)).is_some_and(|(w, _)| w.first() == Some(&(b as u8))));
    let (form, a, t) = match ft.rules.get(usize::from(ft.ends[b])) {
        Some(&EndRule::Run(_)) => (2, u32::from(ft.ends[b]), kind_target(x, fx, sw, u32::from(ft.kinds[b]))),
        Some(&EndRule::Read(_)) => (1, ft.codes1[b], kind_target(x, fx, sw, ft.codes1[b])),
        _ => (0, ft.codes1[b], kind_target(x, fx, sw, ft.codes1[b])),
    };
    if t == REFUSE && !named {
        return None;
    }
    let mut to: Vec<u16> = ft.words.iter().filter(|(w, _)| w.first() == Some(&(b as u8))).map(|(_, code)| kind_target(x, fx, sw, *code)).collect();
    to.push(t);
    if to.iter().all(|&t| t == REFUSE) {
        return None;
    }
    Some((form, a, to))
}

/// The switch `sw` sends the bytes it shares with another to (`fetched_arms`): another switch on a
/// token's first byte in the same function, which does with each byte what `sw` does, or refuses
/// it, and refuses some `sw` takes: its place and table. The smallest such, so a chain of them
/// ends.
fn delegate_of<'a>(x: &'a Facts<'_>, fx: &Fetched, sw: &ByteSwitch) -> Option<(u16, &'a ByteSwitch)> {
    let words = &x.native.structural;
    let mine = |o: &ByteSwitch| -> Option<usize> {
        let (mut own, mut shared) = (0, 0);
        for b in 0..256usize {
            match (byte_sig(x, fx, sw, b), byte_sig(x, fx, o, b)) {
                (a, c) if a == c => shared += usize::from(a.is_some()),
                (Some(_), None) => own += 1,
                _ => return None,
            }
        }
        (own > 0 && shared > 0).then_some(shared)
    };
    let here = words.iter().position(|w| matches!(w.operation, Op::Switch { table } if std::ptr::eq(&x.native.switches[usize::from(table)], sw)))?;
    let mut best: Option<(usize, u16, &'a ByteSwitch)> = None;
    for (pc, w) in words.iter().enumerate() {
        let Op::Switch { table } = w.operation else { continue };
        let o = &x.native.switches[usize::from(table)];
        if pc == here || !on_kinds(x, o) || o.end != sw.end || !reaches(x, here, pc) {
            continue;
        }
        if let Some(shared) = mine(o)
            && best.as_ref().is_none_or(|&(s, _, _)| shared > s)
        {
            best = Some((shared, pc as u16, o));
        }
    }
    best.map(|(_, pc, o)| (pc, o))
}

/// Whether switch `sw` is reached only from the `TOKEN` before it, where that takes a token: its
/// token not made yet, or the stream's end.
fn fresh(x: &Facts<'_>, sw: &ByteSwitch) -> bool {
    let words = &x.native.structural;
    let Some(here) = words.iter().position(|w| matches!(w.operation, Op::Switch { table } if std::ptr::eq(&x.native.switches[usize::from(table)], sw))) else { return false };
    if here == 0 || !matches!(words[here - 1].operation, Op::Token { .. }) || x.phase[here - 1] != crate::stream::TAKEN || x.fetch.as_ref().is_some_and(|fx| fx.part_at(x, here - 1)) {
        return false;
    }
    (0..words.len()).all(|pc| pc == here - 1 || !plain_succ(x, pc).contains(&here))
}

/// A keyword's way from `at` to its leaf (`LEAF` of the word's kind), each word on it one with
/// one successor that reads no code (an `OPEN`, a supertype, a field, a `GOTO`): their code, the
/// leaf's kind the constant of `code`'s word, and the place after the leaf. `None` where the way
/// is not such.
fn known_leaf(x: &Facts<'_>, at: u16, code: u32) -> Result<Option<(String, u16)>, String> {
    let words = &x.native.structural;
    let (mut q, mut out) = (usize::from(at), String::new());
    for _ in 0..16 {
        let Some(w) = words.get(q) else { return Ok(None) };
        let next = succ(x, q);
        let &[t] = next.as_slice() else { return Ok(None) };
        match w.operation {
            Op::Leaf { kind, .. } if kind == WORD_KIND => {
                out.push_str(&word_with(x, q, &|_| Ok(String::new()), Some(code))?);
                return Ok(Some((out, t as u16)));
            }
            Op::Open { .. } | Op::NodeSuper { .. } | Op::NodeField { .. } | Op::Goto { .. } => {
                out.push_str(&word(x, q, &|_| Ok(String::new()))?);
                q = t;
            }
            _ => return Ok(None),
        }
    }
    Ok(None)
}

/// Whether the end a switch's arm makes (`rec_end`) is read from word `t` on, in a direct format 16 run: as the lean
/// parser makes a node's end where it writes it, an arm makes its token's end only where a word reads it. Each way is
/// walked twice over: the end itself, and the end a `TOKEN` passes on as the last token's (`last_end`), until a word
/// makes it again. A word not known to leave both alone reads them; outside a direct run, every end is read.
fn end_read(x: &Facts<'_>, t: u16) -> bool {
    end_read_from(x, usize::from(t), false)
}

/// `end_read` from word `t`, of the last token's end where `last` (a string's copy of it: `part_plan`).
fn end_read_from(x: &Facts<'_>, t: usize, last: bool) -> bool {
    if !(x.sixteen && x.coded) || x.flagged {
        return true;
    }
    let words = &x.native.structural;
    // (word, the last token's end, not this one's)
    let mut seen: HashSet<(usize, bool)> = HashSet::new();
    let mut work = vec![(t, last)];
    while let Some((pc, last)) = work.pop() {
        if pc >= words.len() || !seen.insert((pc, last)) {
            continue;
        }
        // A string copies this end first, as the last token's (`s.last_end = s.rec_end;`), then makes both again
        // at its parts (a content's end, the closer's): this end on, as the last one, at the closer's place, where
        // no part came between. The last one in it is made again.
        if let Some(fx) = x.fetch.as_ref().filter(|fx| fx.part_at(x, pc)) {
            if !last {
                let Ok(pf) = part_facts(x, fx, pc) else { return true };
                work.push((usize::from(past_span(x, pf.closer, 1)), true));
            }
            continue;
        }
        // The words after, as they run: a call's callee, a return's places, else each word's own (not `succ`, which
        // writes each switch's arms, and so asks this again).
        let next: Vec<usize> = match (words[pc].operation, &x.one) {
            (Op::Push { .. }, Some(_)) => vec![crate::native_cpu_stream::callee(x.native, pc)],
            (Op::Return { .. }, Some(one)) => one.returns.get(&pc).map_or(Vec::new(), |r| r.iter().map(|&(_, t)| t).collect()),
            _ => plain_succ(x, pc),
        };
        let on = |work: &mut Vec<(usize, bool)>, l: bool| work.extend(next.iter().map(|&q| (q, l)));
        match words[pc].operation {
            // The end passed on as the last token's; this one's kept until an arm makes it again.
            Op::Token { .. } if !last => {
                on(&mut work, false);
                on(&mut work, true);
            }
            Op::Token { .. } => {}
            Op::Switch { table } => {
                let sw = &x.native.switches[usize::from(table)];
                if !(on_kinds(x, sw) && fresh(x, sw)) {
                    return true;
                }
                if last {
                    on(&mut work, true);
                } else if sw.end != REFUSE {
                    // Every arm makes its token's end; at the stream's end, none does.
                    work.push((usize::from(sw.end), false));
                }
            }
            // A close of its own token's end reads this one's; any other, the last one's.
            Op::Close { this } => {
                if this != last {
                    return true;
                }
                on(&mut work, last);
            }
            Op::Leaf { deferred, .. } => {
                if !last || deferred {
                    return true;
                }
                on(&mut work, last);
            }
            // A last-byte `SPAN` reads nothing here (`flagged` is out); a length's reads the end.
            Op::Check { .. } | Op::Span { last: false, .. } if !last => return true,
            Op::Check { .. } | Op::Span { .. } | Op::Open { .. } | Op::NodeSuper { .. } | Op::NodeField { .. } | Op::Goto { .. } | Op::Count { .. } | Op::Push { .. } | Op::Return { .. } | Op::Empty { .. } => on(&mut work, last),
            Op::Accept | Op::Stop => {}
            _ => return true,
        }
    }
    false
}

/// `s.code = code;`, where a word reached from `to` reads it (`code_read`); nothing where none
/// does, as simdjson's switch makes no record a token.
fn code_set(x: &Facts<'_>, to: &[u16], code: u32) -> String {
    if to.iter().any(|&t| reads_code(x, t)) { format!("s.code = {code:#x};\n") } else { String::new() }
}

/// Whether the byte past the file (0, padding) is refused by `sw`'s switch on the first byte, and
/// by the switch it gives its other bytes to (`delegate_of`).
fn zero_refused(x: &Facts<'_>, fx: &Fetched, sw: &ByteSwitch) -> bool {
    let ft = &fx.ft;
    let named = sw.words.iter().any(|&(y, _)| ft.words.get(usize::from(y)).is_some_and(|(w, _)| w.first() == Some(&0)));
    let code = match ft.rules.get(usize::from(ft.ends[0])) {
        Some(&crate::records::EndRule::Run(_)) => u32::from(ft.kinds[0]),
        _ => ft.codes1[0],
    };
    if named || kind_target(x, fx, sw, code) != REFUSE {
        return false;
    }
    delegate_of(x, fx, sw).is_none_or(|(_, other)| zero_refused(x, fx, other))
}

/// Whether word `to` is reached from word `from` in the same function (`succ`, the plain
/// successors: no switch's to another).
fn reaches(x: &Facts<'_>, from: usize, to: usize) -> bool {
    let mut seen = vec![false; x.native.structural.len()];
    let mut work = vec![from];
    while let Some(pc) = work.pop() {
        if pc == to {
            return true;
        }
        if std::mem::replace(&mut seen[pc], true) {
            continue;
        }
        work.extend(plain_succ(x, pc));
    }
    false
}

/// What a part's `TOKEN` (`Fetched::part_at`) and the switch after it go to: the closer's
/// place, the content's, the part's by its word (else its kind's); the closer's code.
struct PartFacts {
    closer: u16,
    content: u16,
    words: Vec<(u16, u16)>,
    part: u16,
    closer_code: u32,
}

/// Whether the token's code is read from word `t` on, before a word that makes the next
/// token's or this one's again: a `TOKEN`, a part (which writes its own) and a switch on the
/// first byte (`fresh`) end the walk; a switch with a kind path reads it. Of the other words,
/// those known to read no code are passed (each successor, a call's returns); any other reads it.
fn reads_code(x: &Facts<'_>, t: u16) -> bool {
    let words = &x.native.structural;
    let mut seen = vec![false; words.len()];
    let mut work = vec![usize::from(t)];
    while let Some(pc) = work.pop() {
        if pc >= words.len() || std::mem::replace(&mut seen[pc], true) || x.fetch.as_ref().is_some_and(|fx| fx.part_at(x, pc)) {
            continue;
        }
        match words[pc].operation {
            Op::Token { .. } | Op::Stop => continue,
            Op::Switch { table } if fresh(x, &x.native.switches[usize::from(table)]) => continue,
            Op::Goto { .. } | Op::Span { .. } | Op::Open { .. } | Op::Close { .. } | Op::NodeSuper { .. } | Op::NodeField { .. } | Op::Check { .. } | Op::Count { .. } => {}
            Op::Leaf { kind, .. } if kind != WORD_KIND && kind != REMEMBERED => {}
            Op::Push { .. } | Op::Return { .. } if x.one.is_some() => {}
            _ => return true,
        }
        work.extend(succ(x, pc));
    }
    false
}

fn part_facts(x: &Facts<'_>, fx: &Fetched, pc: usize) -> Result<PartFacts, String> {
    let (q, e0) = fx.seq.as_ref().ok_or_else(|| format!("word {pc}: a part pending with no region read as `parse_string`"))?;
    let Some(Op::Switch { table }) = x.native.structural.get(pc + 1).map(|w| w.operation) else {
        return Err(format!("word {pc}: a part's `TOKEN` not before a switch"));
    };
    if !matches!(x.native.structural[pc].operation, Op::Token { any: None, extras: false, .. }) || x.phase[pc] != crate::stream::TAKEN {
        return Err(format!("word {pc}: a part's `TOKEN` that tests a mark or takes no token"));
    }
    let sw = &x.native.switches[usize::from(table)];
    let ft = &fx.ft;
    let in_set = |set: &[u64; 4], b: usize| set[b / 64] >> (b % 64) & 1 == 1;
    let one = |ts: BTreeSet<u16>, what: &str| -> Result<u16, String> {
        match ts.into_iter().collect::<Vec<_>>().as_slice() {
            &[t] => Ok(t),
            ts => Err(format!("word {pc}: a part's {what} goes to {ts:?}")),
        }
    };
    let closer = one((0..256).filter(|&b| in_set(&q.xs, b)).map(|b| kind_target(x, fx, sw, ft.codes1[b])).collect(), "closer")?;
    let codes: BTreeSet<u32> = (0..256).filter(|&b| in_set(&q.xs, b)).map(|b| ft.codes1[b]).collect();
    let closer_code = match codes.into_iter().collect::<Vec<_>>().as_slice() {
        &[c] => c,
        _ => return Err(format!("word {pc}: a closer of more than one code")),
    };
    // The content: its place by its first byte's kind; no word the switch names begins with it,
    // and no word after reads its code (which is then not made).
    let content = one((0..256).filter(|&b| !in_set(&q.stops, b)).map(|b| kind_target(x, fx, sw, u32::from(ft.kinds[b]))).collect(), "content")?;
    if sw.words.iter().any(|&(y, _)| ft.words.get(usize::from(y)).is_some_and(|(w, _)| w.first().is_some_and(|&f| !in_set(&q.stops, usize::from(f))))) || reads_code(x, content) {
        return Err(format!("word {pc}: a content part whose code is read"));
    }
    let codes2 = q.codes2.as_ref().expect("checked in `fetched`");
    let kinds: BTreeSet<u32> = codes2.iter().map(|c| c & 0xff).collect();
    let &[kind] = kinds.iter().copied().collect::<Vec<_>>().as_slice() else { return Err(format!("word {pc}: a part of more than one kind")) };
    let part = kind_target(x, fx, sw, kind);
    let _ = e0;
    Ok(PartFacts { closer, content, words: sw.words.clone(), part, closer_code })
}

/// A part's `TOKEN` and the switch after it, as `parse_string`'s loop: the next stop from the
/// pending place, then one branch to the closer's place, the content's (the bytes before the
/// stop) or the part's (an escape: its code by its second byte, its place by its word).
fn part_plan(x: &Facts<'_>, fx: &Fetched, pc: usize) -> Plan {
    let pf = part_facts(x, fx, pc).expect("checked in `rust_source_at`");
    let (q, _) = fx.seq.as_ref().expect("checked");
    let mut p = Plan::default();
    let _ = q;
    // A content part whose words go straight back to this one: they are written here, and the
    // stop found is taken on at once, not found again, as `copy_and_find` handles its stop.
    let room = if x.unchecked { "" } else { "s.room()?;\n" };
    // The last token's end, kept at each part, only where a word after the closer reads it (`end_read_from`).
    let keep = if end_read_from(x, usize::from(past_span(x, pf.closer, 1)), true) { "s.last_end = s.rec_end;\n" } else { "" };
    let (content, then) = match straight_to(x, pf.content, pc) {
        Some(code) => (format!("{code}{room}{keep}"), "}\n"),
        None => (p.to(pf.content), "} else "),
    };
    // The closer is one byte: its place's test of that (`SPAN`) holds, and is not made again.
    let closer = p.to(past_span(x, pf.closer, 1));
    // Each escape's word to its place, past the checks its bytes cannot hold at (as simdjson's
    // `\u` work only after a `u`), the words of one place a range, as `escape_map`'s one load.
    let mut to: Vec<u16> = vec![pf.part; 256];
    for &(w, t) in &pf.words {
        let bytes = fx.ft.words.get(usize::from(w)).map(|(b, _)| b.as_slice()).unwrap_or(&[]);
        if let Some(slot) = to.get_mut(usize::from(w) + 1) {
            *slot = past_void_checks(x, past_span(x, t, 2), bytes);
        }
    }
    to[0] = pf.part;
    // Code 0 is no word; word `w`'s is `w + 1`, by its name.
    let by_word = arms(&mut p, "s.word()", &to, &|c| if c == 0 { "0".to_owned() } else { x.word(c as u16 - 1) });
    // The next part's place: the method's argument (`part_arg`), else the field the opener's arm set.
    let (take, place, done) = if x.part_arg { ("", "at = ", "") } else { ("let at = s.part_at;\n", "s.part_at = ", "s.part_at = 0;\n") };
    p.code = format!(
        "{}{keep}{take}let (i, closer) = s.part_stop(at)?;
if i != at {{
// The bytes before the stop: content (its code read by no word after).
s.start = at;
s.rec_end = i;
{place}i;
{content}
{then}if closer {{
// The closer: the region's last token.
s.start = i;
s.rec_end = i + 1;
{}{done}{closer}
}} else {{
// A part of two bytes (an escape): its code by its second byte.
if i + 1 >= s.len {{ return refuse(why::STRING); }}
s.start = i;
s.rec_end = i + 2;
s.code = S::CODES2[usize::from(s.byte(i + 1))];
{place}i + 2;
{by_word}}}
",
        if x.unchecked { "" } else { "s.room()?;\n" },
        code_set(x, &[past_span(x, pf.closer, 1)], pf.closer_code)
    );
    p
}

/// What word `pc` does, for a label's name (`rule`): a region's part (`string`), a token, a switch, a node's open or
/// close, a leaf, a call, a return, a check.
fn label_role(x: &Facts<'_>, pc: usize) -> &'static str {
    if x.fetch.as_ref().is_some_and(|fx| fx.part_at(x, pc)) {
        return "string";
    }
    match x.native.structural[pc].operation {
        Op::Token { .. } => "token",
        Op::Switch { .. } => "switch",
        Op::Open { .. } => "open",
        Op::Close { .. } => "close",
        Op::Leaf { .. } => "leaf",
        Op::Push { .. } => "call",
        Op::Return { .. } => "ret",
        Op::Check { .. } => "check",
        Op::Span { .. } => "span",
        Op::NodeSuper { .. } => "super",
        Op::NodeField { .. } => "field",
        Op::Goto { .. } => "goto",
        Op::Accept => "accept",
        _ => "word",
    }
}

/// `src`, generated code, each line indented four spaces a brace it is within (from `depth`), its blank lines
/// dropped: as a person writes it. Braces in comments and string literals are not counted.
fn indented(src: &str, depth: usize) -> String {
    let mut out = String::with_capacity(src.len() * 2);
    let mut depth = depth as i64;
    // A statement's closing braces after it on its line (`break 'a;}`), each on its own.
    let lines = src.lines().flat_map(|line| {
        let l = line.trim();
        let body = l.trim_end_matches('}');
        if body.len() < l.len() && body.ends_with(';') && !body.contains('{') && !body.contains("//") {
            std::iter::once(body).chain(std::iter::repeat_n("}", l.len() - body.len())).collect::<Vec<_>>()
        } else {
            vec![l]
        }
    });
    for l in lines {
        if l.is_empty() {
            continue;
        }
        let (mut opens, mut lead, mut closes, mut seen) = (0i64, 0i64, 0i64, false);
        let (mut quoted, mut prev) = (false, ' ');
        for c in l.chars() {
            match c {
                '"' if prev != '\\' => quoted = !quoted,
                '/' if !quoted && prev == '/' => break,
                '{' if !quoted => {
                    opens += 1;
                    seen = true;
                }
                '}' if !quoted => {
                    if !seen && opens == 0 {
                        lead += 1;
                    } else if opens > 0 {
                        opens -= 1;
                    } else {
                        closes += 1;
                    }
                    seen = true;
                }
                c if !c.is_whitespace() => seen = true,
                _ => {}
            }
            prev = c;
        }
        depth -= lead;
        for _ in 0..depth.max(0) {
            out.push_str("    ");
        }
        out.push_str(l);
        out.push('\n');
        depth += opens - closes;
    }
    out
}

/// The regions read by a method each, as the lean parser's `string` (its name, the parts' `TOKEN`s whose loop it is,
/// its loop's body): where every way of a part's loop but its closer's goes straight back to its `TOKEN`
/// (`straight_back`), the loop is the method's, which returns at the closer, and its `TOKEN` is the method's call,
/// gone on from at the closer's place. The ways' common last words are written once, after the escapes' `match`.
/// One method a loop of the same code: a string in every place it is read (a key, a value, the document) is one
/// method, called from each. No method where the program has context beside its returns.
fn string_methods(x: &Facts<'_>) -> Vec<(String, Vec<usize>, String)> {
    let Some(fx) = x.fetch.as_ref().filter(|_| !x.context) else { return Vec::new() };
    let mut out: Vec<(String, Vec<usize>, String)> = Vec::new();
    for pc in 0..x.native.structural.len() {
        if !fx.part_at(x, pc) {
            continue;
        }
        let Ok(pf) = part_facts(x, fx, pc) else { continue };
        let closer = past_span(x, pf.closer, 1);
        let p = part_plan(x, fx, pc);
        let ways: Option<Vec<Vec<usize>>> = p.refs.iter().filter(|&&t| t != closer).map(|&t| straight_back(x, usize::from(t), pc)).collect();
        let Some(ways) = ways else { continue };
        // The words every way ends with, written once after the branches.
        let mut common = 0;
        while ways.first().is_some_and(|w| w.len() > common) && ways.iter().all(|w| w.len() > common && w[w.len() - 1 - common] == ways[0][ways[0].len() - 1 - common]) {
            common += 1;
        }
        let code = |words: &[usize]| -> Option<String> { words.iter().map(|&q| word(x, q, &|_| Ok(String::new())).ok()).collect() };
        let Some(tail) = ways.first().map_or(Some(String::new()), |w| code(&w[w.len() - common..])) else { continue };
        let body = p.fill(&|i| {
            let t = p.refs[i];
            if t == closer {
                return Ok("return Ok(());".into());
            }
            let w = straight_back(x, usize::from(t), pc).ok_or("a way")?;
            code(&w[..w.len() - common]).ok_or_else(|| "a word".into())
        });
        let Ok(body) = body else { continue };
        let body = format!("{body}{tail}");
        match out.iter_mut().find(|(_, _, b)| *b == body) {
            Some((_, pcs, _)) => pcs.push(pc),
            None => out.push((String::new(), vec![pc], body)),
        }
    }
    let one = out.len() == 1;
    for (k, (name, _, _)) in out.iter_mut().enumerate() {
        *name = if one { "string".into() } else { format!("string_{k}") };
    }
    out
}

/// Whether the opener's place is the token's start's next byte at every region method's call, so it is passed there
/// (`s.string(s.start + 1)`, `Facts::part_arg`), as the lean parser's `string(v)`. Every part is read by a method, and
/// every way into each method's `TOKEN`, from the words the run reaches, comes from a switch on a fresh token's first
/// byte (the opener's arm: the token is the opener) through words that leave the token's start (`OPEN`, a supertype,
/// a field, a `GOTO`, a `SPAN`, a `LEAF` of a fixed kind). A way from a rule's entry, a call, a return or any other
/// word keeps the field.
fn opener_place_at_call(x: &Facts<'_>) -> bool {
    let Some(fx) = x.fetch.as_ref() else { return false };
    let words = &x.native.structural;
    let n = words.len();
    if x.strings.is_empty() || (0..n).any(|pc| fx.part_at(x, pc) && !x.strings.contains_key(&pc)) {
        return false;
    }
    // Each reached word's predecessors.
    let mut preds: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut seen = vec![false; n];
    let mut work: Vec<usize> = x.entries.clone();
    while let Some(pc) = work.pop() {
        if pc >= n || std::mem::replace(&mut seen[pc], true) {
            continue;
        }
        for t in succ(x, pc) {
            if t < n {
                preds[t].push(pc);
                work.push(t);
            }
        }
    }
    x.strings.keys().all(|&method| {
        let mut back = vec![false; n];
        let mut work = vec![method];
        while let Some(pc) = work.pop() {
            if std::mem::replace(&mut back[pc], true) {
                continue;
            }
            // A rule's entry: its start is its caller's.
            if x.entries.contains(&pc) {
                return false;
            }
            for &q in &preds[pc] {
                let passes = match words[q].operation {
                    Op::Switch { table } if fresh(x, &x.native.switches[usize::from(table)]) => continue,
                    Op::Open { .. } | Op::NodeSuper { .. } | Op::NodeField { .. } | Op::Goto { .. } | Op::Span { .. } => true,
                    Op::Leaf { kind, .. } => kind != WORD_KIND && kind != REMEMBERED,
                    _ => false,
                };
                if !passes {
                    return false;
                }
                work.push(q);
            }
        }
        true
    })
}

/// The words from `from` on to word `to`, where each has one successor and is a node word (`LEAF` of a fixed kind, a
/// supertype, a field), a `GOTO`, a last byte's `SPAN` or a `CHECK`: a part's way back to its `TOKEN`. `None` where
/// the way is other.
fn straight_back(x: &Facts<'_>, from: usize, to: usize) -> Option<Vec<usize>> {
    let words = &x.native.structural;
    let (mut q, mut out) = (from, Vec::new());
    for _ in 0..16 {
        if q == to {
            return Some(out);
        }
        let &[t] = succ(x, q).as_slice() else { return None };
        match words.get(q)?.operation {
            Op::Leaf { kind, .. } if kind != WORD_KIND && kind != REMEMBERED => {}
            Op::NodeSuper { .. } | Op::NodeField { .. } | Op::Goto { .. } | Op::Span { last: true, .. } | Op::Check { .. } => {}
            _ => return None,
        }
        out.push(q);
        q = t;
    }
    None
}

/// The code of the words from `from` to word `to`, where each has one successor and is a node
/// word (`LEAF` of a fixed kind, a supertype, a field), a `GOTO` or a last byte's `SPAN`; `None`
/// where the way is other.
fn straight_to(x: &Facts<'_>, from: u16, to: usize) -> Option<String> {
    let words = &x.native.structural;
    let (mut q, mut out) = (usize::from(from), String::new());
    for _ in 0..16 {
        if q == to {
            return Some(out);
        }
        let &[t] = succ(x, q).as_slice() else { return None };
        match words.get(q)?.operation {
            Op::Leaf { kind, .. } if kind != WORD_KIND && kind != REMEMBERED => {}
            Op::NodeSuper { .. } | Op::NodeField { .. } | Op::Goto { .. } | Op::Span { last: true, .. } => {}
            _ => return None,
        }
        out.push_str(&word(x, q, &|_| Ok(String::new())).ok()?);
        q = t;
    }
    None
}

/// Place `t` past its `SPAN` of `len` bytes, where the token is known to be that long.
fn past_span(x: &Facts<'_>, t: u16, len: u16) -> u16 {
    match x.native.structural.get(usize::from(t)).map(|w| w.operation) {
        Some(Op::Span { last: false, len: l }) if l == len && !x.flagged => t + 1,
        _ => t,
    }
}

/// Place `t` past the `SPAN`s and the mask checks at it that a token of bytes `bytes` cannot
/// hold at: each check's mask holds only at bytes none of them is (`may_bytes`), so it has no
/// bit in the token and the check holds. As simdjson does the `\u` work only after a `u`.
fn past_void_checks(x: &Facts<'_>, t: u16, bytes: &[u8]) -> u16 {
    let Some(block) = x.native.block_programs.first() else { return t };
    let (words, mut q) = (&x.native.structural, usize::from(t));
    let mut past = t;
    if bytes.is_empty() {
        return t;
    }
    loop {
        match words.get(q).map(|w| w.operation) {
            Some(Op::Span { last: true, .. }) if !x.flagged => q += 1,
            Some(Op::Check { check }) => {
                let c = &x.native.local_checks[usize::from(check)];
                let Some(&o) = block.masks.get(usize::from(c.mask)).filter(|_| !c.token && c.back == 0) else { return past };
                // The mask's bit at a place is its value's `delay` places on: the byte there is
                // `delay` back from the value's.
                let may = may_bytes(block, usize::from(block.outputs[o]), u32::from(block.output_delays[o]));
                if bytes.iter().any(|&b| may[usize::from(b) / 64] >> (b % 64) & 1 == 1) {
                    return past;
                }
                q += 1;
                past = q as u16;
            }
            _ => return past,
        }
    }
}

/// The bytes, `back` places before where value `x` of a block holds, that can be there: a
/// superset. A value of its own byte (`native_cpu_lanes::byte_set`) at its own place; a `prev` by
/// `k` the same `k` places further back; an `and` only where both can, an `or` where either; any
/// other value at any byte.
fn may_bytes(b: &crate::block_native::BlockNative, x: usize, back: u32) -> [u64; 4] {
    use crate::block_native::BlockInst as I;
    const ALL: [u64; 4] = [!0; 4];
    if let Some(set) = crate::native_cpu_lanes::byte_set(b, x) {
        return if back == 0 || set == [0; 4] { set } else { ALL };
    }
    match b.insts[x] {
        I::And(p, q) => {
            let (p, q) = (may_bytes(b, usize::from(p), back), may_bytes(b, usize::from(q), back));
            std::array::from_fn(|k| p[k] & q[k])
        }
        I::Or(p, q) => {
            let (p, q) = (may_bytes(b, usize::from(p), back), may_bytes(b, usize::from(q), back));
            std::array::from_fn(|k| p[k] | q[k])
        }
        I::AndNot(p, _) => may_bytes(b, usize::from(p), back),
        I::Prev(a, k) if back >= u32::from(k) => may_bytes(b, usize::from(a), back - u32::from(k)),
        I::Const(0) => [0; 4],
        _ => ALL,
    }
}

/// A token's check that this backend writes (`program::token_value`): none back, no `ended`.
fn token_check_ok(c: &LocalCheck) -> bool {
    c.token && c.back == 0 && !c.ops.iter().any(|o| matches!(o, LocalOp::Ended)) && c.delay < 64
}

/// A token's check the window loop takes (`token_check`): one that reads back (decision 0428) too, `back` under 64,
/// so its `Ended` bits are in the first window.
fn token_check_loop_ok(c: &LocalCheck) -> bool {
    c.token && c.back < 64 && c.delay < 64
}

/// A token's check's ops as Rust, a `let v{k}` each, over one window of places (64 of them,
/// `wide`, else 16): the byte sets by the classes `c{t}` (four vectors, or one), a `prev`
/// carrying its operand's last window (`pv{a}`) and a `fill` its carry (`cin{k}`) where `wide`.
/// What carries to the next window goes to `after`.
/// Where the check reads back (`LocalCheck::back`, decision 0428), the frame's places: `pre` before the file, the
/// first window's bytes loaded from the file's first and their bits moved up past those (`sh`), `ended` the tokens'
/// ends before the token, and `left` the frame's places from the file's start to its end.
fn check_ops(c: &LocalCheck, w: &WordForm, wide: bool, step: &mut String, after: &mut String) {
    let carried: BTreeSet<usize> = c.ops.iter().filter_map(|o| if let LocalOp::Prev(a, _) = *o { Some(usize::from(a)) } else { None }).collect();
    for (k, op) in c.ops.iter().enumerate() {
        let v = |a: u16| format!("v{a}");
        let back = u32::from(c.back);
        let e = match *op {
            LocalOp::Bytes(_) => match w.class_of[&k] {
                (_, 0) => "0".into(),
                (ti, bits) if wide && back != 0 => format!("bits(c{ti}.map(|v| vtstq_u8(v, vdupq_n_u8({bits})))) << sh"),
                (ti, bits) if wide => format!("bits(c{ti}.map(|v| vtstq_u8(v, vdupq_n_u8({bits}))))"),
                (ti, bits) => format!("Self::bits16(vtstq_u8(c{ti}, vdupq_n_u8({bits})))"),
            },
            LocalOp::Const(pat) => format!("{pat:#x}u64"),
            LocalOp::Within(d) if back != 0 => format!("S::within(q as i64, pre + {d}, left + {d})"),
            LocalOp::Within(d) => format!("S::within(q as i64, {d}, left + {d})"),
            LocalOp::Start if back != 0 => format!("S::within(q as i64, {back}, {})", back + 1),
            LocalOp::Start => "if q == 0 { 1 } else { 0 }".into(),
            LocalOp::Last if back != 0 => format!("S::within(q as i64, {back} + n as i64 - 1, {back} + n as i64)"),
            LocalOp::Last => "S::within(q as i64, n as i64 - 1, n as i64)".into(),
            LocalOp::Ended => "if q == 0 { ended } else { 0 }".into(),
            LocalOp::Not(a) => format!("!{}", v(a)),
            LocalOp::And(a, b) => format!("{} & {}", v(a), v(b)),
            LocalOp::Or(a, b) => format!("{} | {}", v(a), v(b)),
            LocalOp::Xor(a, b) => format!("{} ^ {}", v(a), v(b)),
            LocalOp::AndNot(a, b) => format!("{} & !{}", v(a), v(b)),
            LocalOp::Prev(a, sh) if wide => format!("({} << {sh}) | (pv{a} >> {})", v(a), 64 - u32::from(sh)),
            LocalOp::Prev(a, sh) => format!("{} << {sh}", v(a)),
            LocalOp::Fill(a, b) => {
                // `block::carry::fill`: one add with the carry in.
                let cin = if wide { format!("cin{k}") } else { "0".into() };
                writeln!(step, "            let ne{k} = !{};\n            let bb{k} = ne{k} | {};\n            let (sum{k}, o1_{k}) = {}.overflowing_add(bb{k});\n            let (sum{k}, o2_{k}) = sum{k}.overflowing_add({cin});", v(b), v(a), v(a)).unwrap();
                if wide {
                    writeln!(after, "            cin{k} = u64::from(o1_{k} | o2_{k});").unwrap();
                }
                format!("{} | (ne{k} & (sum{k} ^ {} ^ bb{k}))", v(a), v(a))
            }
        };
        writeln!(step, "            let v{k}: u64 = {e};").unwrap();
    }
    if wide {
        for &a in &carried {
            writeln!(after, "            pv{a} = v{a};").unwrap();
        }
    }
}

/// A token's check (decision 0398, `program::token_value`): over the token's bytes and `delay`
/// after them, a 64-byte window a step, word-parallel: tier 1's classes for the byte sets, the
/// other ops word operations, a `prev` carrying its operand's last word, a `fill` its carry.
/// It refuses where its last op holds at a place from `delay` to the token's end and `delay`.
fn token_check(out: &mut String, i: usize, c: &LocalCheck) -> Result<(), String> {
    if !token_check_loop_ok(c) {
        return Err(format!("check {i}: a token's check that reads back 64 bytes or more, or of a delay of 64 or more: not yet ahead of time"));
    }
    let w = word_form(&c.ops).map_err(|e| format!("check {i}: {e}"))?;
    let carried: BTreeSet<usize> = c.ops.iter().filter_map(|o| if let LocalOp::Prev(a, _) = *o { Some(usize::from(a)) } else { None }).collect();
    let fills: Vec<usize> = c.ops.iter().enumerate().filter(|(_, o)| matches!(o, LocalOp::Fill(..))).map(|(k, _)| k).collect();
    let mut body = String::new();
    for (ti, (lo, hi)) in w.tables.iter().enumerate() {
        writeln!(body, "        let lo{ti} = vld1q_u8({lo:?}.as_ptr());\n        let hi{ti} = vld1q_u8({hi:?}.as_ptr());").unwrap();
    }
    let mut lets = String::new();
    for &a in &carried {
        writeln!(lets, "        let mut pv{a} = 0u64;").unwrap();
    }
    for &k in &fills {
        writeln!(lets, "        let mut cin{k} = 0u64;").unwrap();
    }
    let used: BTreeSet<usize> = w.class_of.values().filter(|&&(_, b)| b != 0).map(|&(t, _)| t).collect();
    let back = u32::from(c.back);
    let mut step = if back == 0 {
        String::from("            let b = self.window(st + 64 * q);\n")
    } else {
        // The frame from `back` bytes before the token; a window that begins before the file loaded from its first
        // byte, its bytes' bits moved up past the places before it (`native_cpu_stream::token_window`).
        String::from("            let from = base + 64 * q as i64;\n            let sh = if from < 0 { (-from) as u32 } else { 0 };\n            let b = self.window(from.max(0) as u32);\n")
    };
    for &ti in &used {
        writeln!(step, "            let c{ti} = [0, 1, 2, 3].map(|v| vandq_u8(vqtbl1q_u8(lo{ti}, vandq_u8(b[v], vdupq_n_u8(15))), vqtbl1q_u8(hi{ti}, vshrq_n_u8::<4>(b[v]))));").unwrap();
    }
    let mut after = String::new();
    check_ops(c, &w, true, &mut step, &mut after);
    let last = c.ops.len() - 1;
    let d = c.delay;
    // The token's last bytes it can hold at (`native_cpu_stream::token_gate`): any other, none of it, as simdjson's
    // stage 2 switches on an escape's byte before its `\u` work.
    let gate = match crate::native_cpu_stream::token_gate(c) {
        Some(set) => format!(
            "        let g = unsafe {{ *self.bytes.add(self.rec_end as usize - 1) }};\n        if [{}][usize::from(g >> 6)] >> (g & 63) & 1 == 0 {{\n            return Ok(());\n        }}\n",
            set.map(|w| format!("{w:#x}u64")).join(", ")
        ),
        None => String::new(),
    };
    // Reading back: the frame's first place in the file (`base`, below 0 where it begins before it), the places
    // before the file, and the tokens' ends before this one from the records before its own (`ended_word`): at most
    // `back` end in the frame, each a byte or more.
    let frame = if back == 0 {
        "        let left = self.len as i64 - st as i64;\n".to_string()
    } else {
        let mut f = format!("        let base = st as i64 - {back};\n        let left = self.len as i64 - base;\n        let pre = (-base).max(0);\n        let mut ended = 0u64;\n");
        if c.ops.iter().any(|o| matches!(o, LocalOp::Ended)) {
            writeln!(
                f,
                "        for j in 1..={back}i64 {{\n            let at = self.next as i64 - 1 - j;\n            if at >= 0 {{\n                let p = unsafe {{ *self.lasts.add(at as usize * self.rs) }} as i64 - base;\n                if (0..{back}).contains(&p) {{\n                    ended |= 1 << p;\n                }}\n            }}\n        }}"
            )
            .unwrap();
        }
        f
    };
    writeln!(
        out,
        "    /// `CHECK {i}`, a token's check: over {back} bytes before it, its bytes and {d} after them, 64 bytes a step.
    #[inline(always)]
    fn check_{i}(&self) -> R {{
{gate}        let st = self.start;
        let n = self.rec_end - st;
{frame}        unsafe {{
{body}{lets}        let mut q = 0u32;
        while 64 * q < {back} + n + {d} {{
{step}            if v{last} & S::within(q as i64, {back} + {d}, {back} + n as i64 + {d}) != 0 {{
                return refuse(why::CHECK);
            }}
{after}            q += 1;
        }}
        }}
        Ok(())
    }}
"
    )
    .unwrap();
    Ok(())
}

/// The check at place `t`, where its words are a token's `SPAN`s then a `CHECK` over its bytes
/// alone (`token_check_ok`) of a delay under 15: the check's place, its index, the check.
fn check_at<'a>(x: &'a Facts<'a>, t: u16) -> Option<(usize, usize, &'a LocalCheck)> {
    let words = &x.native.structural;
    let mut q = usize::from(t);
    while matches!(words.get(q).map(|w| w.operation), Some(Op::Span { last: true, .. })) {
        q += 1;
    }
    let Some(Op::Check { check }) = words.get(q).map(|w| w.operation) else { return None };
    let c = &x.native.local_checks[usize::from(check)];
    (token_check_ok(c) && c.delay < 15 && q + 1 < words.len()).then_some((q, usize::from(check), c))
}

/// A run's token and the check at its place made from one read of its 16 bytes, as
/// `parse_number` validates the digits as it reads them (`run_R_check_C`): the check's sets and
/// the run's end (the bytes not of its class) classified together. Where the end is in them,
/// the token's last byte (the check passed), else none (the token is longer: the run's end and
/// the check as apart); the check failing refuses the file.
fn run_check(out: &mut String, r: u8, class: &[u8; 256], ci: usize, c: &LocalCheck) -> Result<(), String> {
    let mut not_c = [0u64; 4];
    for b in 0..256usize {
        if class[b] == 0 {
            not_c[b / 64] |= 1 << (b % 64);
        }
    }
    let mut both = c.clone();
    both.ops.push(LocalOp::Bytes(not_c));
    let w = word_form(&both.ops).map_err(|e| format!("check {ci} with rule {r}'s end: {e}"))?;
    let mut body = String::new();
    let used: BTreeSet<usize> = w.class_of.values().filter(|&&(_, b)| b != 0).map(|&(t, _)| t).collect();
    for &ti in &used {
        let (lo, hi) = w.tables[ti];
        writeln!(body, "            let c{ti} = vandq_u8(vqtbl1q_u8(vld1q_u8({lo:?}.as_ptr()), vandq_u8(b, vdupq_n_u8(15))), vqtbl1q_u8(vld1q_u8({hi:?}.as_ptr()), vshrq_n_u8::<4>(b)));").unwrap();
    }
    let end = match w.class_of[&c.ops.len()] {
        (_, 0) => "0".to_string(),
        (ti, bits) => format!("Self::bits16(vtstq_u8(c{ti}, vdupq_n_u8({bits})))"),
    };
    let mut step = String::new();
    let mut after = String::new();
    check_ops(c, &w, false, &mut step, &mut after);
    let (last, d) = (c.ops.len() - 1, u32::from(c.delay));
    // The end's lanes: 1 to 15 less the delay, the check's places all in the 16.
    let lanes = ((1u64 << (16 - d)) - 1) & !1;
    writeln!(
        out,
        "    /// Rule {r}'s token from `st` and `CHECK {ci}` over it, from one read of its 16 bytes (as
    /// `parse_number`): its last byte, the check passed; none where its end is past them.
    #[inline(always)]
    fn run_{r}_check_{ci}(&self, st: u32) -> Result<Option<u32>, u32> {{
        let left = self.len as i64 - st as i64;
        unsafe {{
            let b = vld1q_u8(self.bytes.add(st as usize));
            // Past the file, zeros (as the check reads it); the file's end ends the run.
            let inside = if left < 16 {{ vcltq_u8(vld1q_u8([0u8, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15].as_ptr()), vdupq_n_u8(left as u8)) }} else {{ vdupq_n_u8(0xff) }};
            let b = vandq_u8(b, inside);
{body}            let ends = ({end} | !Self::bits16(inside)) & {lanes:#x};
            if ends == 0 {{
                return Ok(None);
            }}
            let n = ends.trailing_zeros();
            let q = 0u32;
{step}            if v{last} & (((1u64 << n) - 1) << {d}) != 0 {{
                return refuse(why::CHECK);
            }}
            Ok(Some(st + n - 1))
        }}
    }}
"
    )
    .unwrap();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A local check's word form against `program::local_value`, place by place, over random
    /// bytes: the generated expression's semantics, evaluated here as the generated code does.
    #[test]
    fn a_check_word_is_its_places_values() {
        // The word form, evaluated with plain integers (the generated code's NEON classifies the
        // same sets).
        fn word(ops: &[LocalOp], bytes: &[u8], q: i64) -> u64 {
            fn v(ops: &[LocalOp], k: usize, bytes: &[u8], q: i64) -> u64 {
                let within = |lo: i64, hi: i64| S_within(q, lo, hi);
                match ops[k] {
                    LocalOp::Bytes(s) => {
                        let mut m = 0;
                        for b in 0..64 {
                            let p = q * 64 + b;
                            let x = if p >= 0 && (p as usize) < bytes.len() { bytes[p as usize] } else { 0 };
                            if s[usize::from(x) / 64] >> (x % 64) & 1 == 1 {
                                m |= 1 << b;
                            }
                        }
                        m & within(0, bytes.len() as i64)
                    }
                    LocalOp::Const(p) => if q >= 0 { p } else { 0 },
                    LocalOp::Within(d) => within(i64::from(d), bytes.len() as i64 + i64::from(d)),
                    LocalOp::Not(a) => !v(ops, usize::from(a), bytes, q),
                    LocalOp::And(a, b) => v(ops, usize::from(a), bytes, q) & v(ops, usize::from(b), bytes, q),
                    LocalOp::Or(a, b) => v(ops, usize::from(a), bytes, q) | v(ops, usize::from(b), bytes, q),
                    LocalOp::Xor(a, b) => v(ops, usize::from(a), bytes, q) ^ v(ops, usize::from(b), bytes, q),
                    LocalOp::AndNot(a, b) => v(ops, usize::from(a), bytes, q) & !v(ops, usize::from(b), bytes, q),
                    LocalOp::Prev(a, s) => (v(ops, usize::from(a), bytes, q) << s) | if q >= 1 { v(ops, usize::from(a), bytes, q - 1) >> (64 - u32::from(s)) } else { 0 },
                    LocalOp::Fill(..) | LocalOp::Start | LocalOp::Last | LocalOp::Ended => unreachable!(),
                }
            }
            v(ops, ops.len() - 1, bytes, q)
        }
        #[allow(non_snake_case)]
        fn S_within(q: i64, lo: i64, hi: i64) -> u64 {
            let (a, b) = ((lo - q * 64).clamp(0, 64), (hi - q * 64).clamp(0, 64));
            if a >= b { 0 } else { (if b == 64 { u64::MAX } else { (1u64 << b) - 1 }) & (u64::MAX << a) }
        }
        let hex = {
            let mut s = [0u64; 4];
            for b in b"0123456789abcdefABCDEF" {
                s[usize::from(*b) / 64] |= 1 << (b % 64);
            }
            s
        };
        let ops = vec![LocalOp::Bytes(hex), LocalOp::Not(0), LocalOp::Prev(1, 3), LocalOp::Bytes([0, 1 << (b'u' - 64), 0, 0]), LocalOp::Prev(3, 1), LocalOp::And(4, 1), LocalOp::Or(5, 2), LocalOp::Within(2), LocalOp::And(6, 7)];
        let mut seed = 7u64;
        let bytes: Vec<u8> = (0..300)
            .map(|_| {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                b"u0aF\\zx9"[(seed >> 60) as usize % 8]
            })
            .collect();
        for q in 0..6i64 {
            let w = word(&ops, &bytes, q);
            for b in 0..64 {
                let p = q * 64 + b;
                assert_eq!(w >> b & 1 == 1, crate::program::local_value(&ops, &bytes, p), "place {p}");
            }
        }
    }
}
