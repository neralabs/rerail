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
//!   counts, the buffers. (Each kept apart costs more, measured over `.rr`'s corpus: there is
//!   about one call a token. The state in each rule's locals, copied out and in at each call,
//!   cost 28% more instructions; its hot fields alone in locals, 17%; the buffers behind a
//!   read-only reference of their own, 11%, in the registers each call saves.)
//! - **Structured control.** A rule's words are a graph (each word a node); it is written as
//!   loops, labelled blocks and `match`es (`clif_rust::Shape`), each `SWITCH` a `match` on the
//!   token's kind or byte.
//! - **One failure exit.** A word that refuses returns `Err(false)`, passed up by `?`, as the lean
//!   parser's `Err(false)`. No word's place is returned: a refused file goes to the fallback, and
//!   the interpreter names the word where that is wanted (`stream::run`). Out of the tape's
//!   room, the run sets `overflow` first, and the harness runs it again on a bigger tape.
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
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write;

/// The program's facts the words read.
struct Facts<'a> {
    native: &'a Structure,
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
    /// The stream's end as a start: past the last token, `TOKEN` gives the file's length as its
    /// start, with no branch (`sentinel`), as simdjson's index ends with one past the last.
    sentinel: bool,
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
}

/// Whether the calls nest as the nodes (`OneFn::stacked`): each `PUSH` right after its
/// container's `OPEN` (a supertype or field word between), each `RETURN` right after the `CLOSE`
/// of that container, and each rule's opens and closes counted alike on every way, a call the
/// close of the container before it: 0 at a `RETURN`'s `CLOSE`, and at least 1 at every other
/// `CLOSE` and every `PUSH`. Then that `CLOSE` closes the node the call's `OPEN` made, the
/// container's row holds the call's number until it (its end is not read before), and no
/// `CLOSE` finds no node open. Only words whose nodes this counts: no wrap, fold or drop.
fn stacked(x: &Facts<'_>) -> bool {
    let words = &x.native.structural;
    let Some(one) = &x.one else { return false };
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
                return false;
            }
            q -= 1;
            match words[q].operation {
                Op::Open { .. } => break,
                Op::NodeSuper { .. } | Op::NodeField { .. } => {}
                _ => return false,
            }
        }
    }
    for &r in one.returns.keys() {
        if r == 0 || !only_from(r, r - 1) || !matches!(words[r - 1].operation, Op::Close { .. }) {
            return false;
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
            let after = match words[pc].operation {
                Op::Open { .. } => c + 1,
                Op::Close { .. } => {
                    let last = plain_succ(x, pc).iter().any(|&t| matches!(words[t].operation, Op::Return { .. }));
                    if (last && c != 0 && e != 0) || (!last && c < 1) {
                        return false;
                    }
                    c - 1
                }
                Op::Push { .. } if c < 1 => return false,
                Op::Push { .. } => c - 1,
                Op::Token { .. } | Op::Switch { .. } | Op::Leaf { .. } | Op::NodeSuper { .. } | Op::NodeField { .. } | Op::Span { .. } | Op::Check { .. } | Op::Goto { .. } | Op::Return { .. } | Op::Accept | Op::Stop | Op::Empty { .. } => c,
                _ => return false,
            };
            for t in plain_succ(x, pc) {
                match at.get(&t) {
                    Some(&v) if v != after => return false,
                    Some(_) => {}
                    None => {
                        at.insert(t, after);
                        work.push(t);
                    }
                }
            }
        }
    }
    true
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
        OneFn { sites: sites.into_iter().zip(number).collect(), returns, after, stacked: false }
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

/// Tier 2's program as Rust source: `tier2_entry(ctx)`, the harness's entry
/// (`native_cpu_stream::Entry`), with its fingerprint, `SOURCE`.
pub fn rust_source(native: &Structure) -> Result<String, String> {
    rust_source_at(native, rerail_tape_format::Narrow::WIDTHS, native.tag)
}

/// As `rust_source`, the tape written at `widths` with the language's `tag`.
pub fn rust_source_at(native: &Structure, widths: rerail_tape_format::Widths, tag: u8) -> Result<String, String> {
    if !crate::native_cpu_stream::calls_on_stack(native) {
        return Err("calls are not on the machine's stack (`calls_on_stack`)".into());
    }
    if native.extras.len() > 1 {
        return Err("more than one kind of extra".into());
    }
    let fetch = fetched(native)?;
    let late = native.block_programs.iter().filter_map(|b| b.records.as_ref()).any(|r| r.readers.iter().any(|rd| !rd.parts.is_empty() && rd.split == crate::records::Split::Tier2));
    if late && fetch.as_ref().is_none_or(|f| f.seq.is_none()) {
        return Err("a region tier 2 splits (decision 0323), not read as `parse_string`: not yet ahead of time".into());
    }
    let classes = crate::native_cpu_stream::supertype_classes(native).map_err(|e| format!("supertype classes: {e:?}"))?;
    let records = native.block_programs.first().and_then(|b| b.records.as_ref());
    let starts = native.structural.iter().any(|b| matches!(b.operation, Op::Token { .. }));
    let in_code = records.map_or(0, |r| r.marks.len());
    let lazy: Vec<(u8, u16)> = native.block_programs.first().and_then(|b| b.records.as_ref().map(|r| (b, r))).map_or(Vec::new(), |(b, r)| {
        let bw = crate::native_cpu_bytes::Bytewise::of(b);
        crate::native_cpu_block::lazy_marks(r, &|o| bw.can(o))
    });
    let lazy_bits: u64 = lazy.iter().fold(0, |a, &(k, _)| a | 1 << k);
    let rest_bits: u64 = (if in_code >= 64 { 0 } else { u64::MAX << in_code }) | lazy_bits;
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
    let flagged = phase.iter().any(|&p| p == crate::stream::TAKEN | crate::stream::UNTAKEN);
    if fetch.is_some() && (flagged || native.extras.len() > 0) {
        return Err("records made in tier 2, with a token's taking flagged or extras: not yet ahead of time".into());
    }
    let unchecked = crate::native_cpu_stream::bounds(native).unchecked(crate::native_cpu_stream::marks_of(words));
    let mut facts = Facts { native, in_code, rest_bits, end_bits, lazy, kind_reps, end_kind, word_kind_of, framed, context, entries: entries.into_iter().collect(), phase, flagged, widths, tag, fetch, classes, unchecked, one: None, sentinel: false };
    facts.sentinel = sentinel(&facts);
    // Where tier 2 makes the records, the program is one function, as `walk_document`.
    if facts.fetch.is_some() && !facts.context {
        let one = OneFn::of(native, &|w| plain_succ(&facts, w));
        facts.one = Some(one);
        facts.entries = vec![0];
        let st = stacked(&facts);
        if let Some(one) = &mut facts.one {
            one.stacked = st;
        }
    }
    if let Some(fx) = &facts.fetch {
        for pc in 0..words.len() {
            if matches!(words[pc].operation, Op::Token { .. }) && fx.seen[pc] == [true, true] {
                return Err(format!("word {pc}: a `TOKEN` where a part may be pending or not"));
            }
            if fx.part_at(&facts, pc) {
                part_facts(&facts, fx, pc)?;
            }
        }
    }
    let mut out = String::new();
    out.push_str("// Generated by rerail_compiler::rust_stream from tier 2's native IR. Do not edit.\n");
    out.push_str("#![allow(unused_mut, unused_variables, unused_assignments, unreachable_code, unused_labels, unused_unsafe, unused_parens, non_snake_case, dead_code, clippy::all)]\n\n");
    out.push_str("use core::arch::aarch64::*;\n\n");
    prelude(&mut out, &facts)?;
    for &e in &facts.entries {
        rule(&mut out, &facts, e)?;
    }
    let h = crate::clif_rust::fingerprint(&out);
    writeln!(out, "\n/// This source's fingerprint (`clif_rust::fingerprint` of what comes before).\npub const SOURCE: u64 = {h:#x};").unwrap();
    Ok(out)
}

/// The harness's `Ctx` and node row, the run's state `S`, and the words' own functions.
fn prelude(out: &mut String, x: &Facts<'_>) -> Result<(), String> {
    let n = x.native;
    let b = crate::native_cpu_stream::bounds(n);
    out.push_str(&CTX.replace("$STATE", &crate::native_cpu_stream::STATE_SLOTS.to_string()));
    tape_consts(out, x);
    let counts = n.structural.iter().filter_map(|w| if let Op::Count { count, .. } = w.operation { Some(usize::from(count) + 1) } else { None }).max().unwrap_or(0);
    writeln!(out, "const COUNTS: usize = {};", counts.max(1)).unwrap();
    writeln!(out, "const DEPTH: u32 = {};", crate::native_cpu_stream::DEPTH).unwrap();
    out.push_str(STATE);
    // The entry.
    out.push_str(
        "/// The entry, on the harness's `Ctx` (`native_cpu_stream::Entry`): 0 where the file is taken,
/// `u64::MAX` out of the tape's room, else a refusal (`REFUSED`).
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
        block_base: 0, coded: 0, ncodes: 0, wide: 0,
        room_nodes: c.room_nodes as usize, room_supers: c.room_supers as usize,
        next: 0, start: 0, end: 0, rec_end: 0, code: 0, last_end: 0, at_end: false, need: true, part_at: 0, live: 0,
        nodes: 0, nopen: 0, pending: 0, keep: 0, supers: 0, last_made: 0, moved: 0,
        counts: [0; COUNTS], depth: 0, ret: [0; DEPTH as usize],
    };
",
    );
    let args = if x.context { ", 0, 0" } else { "" };
    writeln!(out, "    match r0(&mut s{args}) {{\n        Ok(_) => 0,\n        Err(true) => u64::MAX,\n        Err(false) => REFUSED,\n    }}\n}}\n").unwrap();

    out.push_str("impl S {\n");
    // The room: the most a token's path writes (`Bounds`), a call open's pops where returns chain.
    let pops = |per: u64| if per > 0 { format!(" + self.depth as usize * {per}") } else { String::new() };
    writeln!(
        out,
        "    /// The tape's room for the path to the next token (`native_cpu_stream::Bounds`).
    #[inline(always)]
    fn room(&mut self) -> R {{
        if self.nodes as usize + {}{} > self.room_nodes || self.supers as usize + {}{} > self.room_supers {{
            return Err(true);
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
    node_words(out, x);
    // The keyword's leaf kind (`WORD_KIND`): a `match` on the token's word.
    out.push_str("    /// A keyword's leaf kind, by its token's word (`Words::word_kinds`).\n    #[inline(always)]\n    fn word_kind(&self) -> Result<u32, bool> {\n        Ok(match self.word() {\n");
    for &(s, k) in &n.word_kinds {
        writeln!(out, "            {} => {k},", u32::from(s) + 1).unwrap();
    }
    out.push_str("            _ => return Err(false),\n        })\n    }\n\n");
    // Accept: the table.
    accept(out, x);
    for (i, r) in n.readings.maps.iter().enumerate() {
        reread(out, i, r);
    }
    for (i, m) in n.token_marks.iter().enumerate() {
        pattern(out, i, m, n.extras.first().map_or(0, |e| e.0));
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
        // goes through Rust has one yet (the `.rr` grammar checks none).
        if c.token {
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
        "type SizeT = {};\ntype OffT = {};\ntype TailT = {};\nconst FROM_START: bool = {};\nconst TAIL_MAX: u32 = {:#x};\n\
         const STRIDE: u32 = {};\nconst STRIDE_LOG: u32 = {};\nconst WIDE_OFF: u32 = {};\nconst WIDE_TAIL: u32 = {};\n\
         const NO_CODE: u32 = {:#x};\nconst KIND_BITS: u32 = {};\nconst HEADER_BYTES: usize = {};\nconst COLUMN_ALIGN: usize = {};\n\
         const NODES_AT: usize = {};\nconst CODE_EXTRA: u8 = {};\nconst SENTINEL: u16 = {:#x};\nconst MAGIC: u64 = {:#x};\nconst HEAD: u32 = {:#x};\nconst FORMAT_VERSION: u32 = {};\n",
        ty(w.size),
        ty(w.off),
        ty(w.tail),
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
    record_stride: u64,
}

/// A node's working row (`native_cpu_tape::ROW`): its key (kind and field), its bytes. Not
/// part of the tape.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Row { kind: u16, field: u16, start: u32, end: u32 }

/// A refusal's value: one more than a place past every word's (no word's place is kept).
pub const REFUSED: u64 = u64::MAX - 1;
/// A row's marks (`head`): a wrap; a dropped row taken out from among its children.
const WRAPPED: u32 = 1;
const DROPPED_IN: u32 = 2;
const DEAD: u16 = u16::MAX;

type R = Result<(), bool>;

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
    /// The last block's base; the nodes committed and the codes; the values past their width.
    block_base: u32, coded: u32, ncodes: u32, wide: u32,
    room_nodes: usize, room_supers: usize,
    /// The next record; the token's start, end (its start until its bytes are taken), its
    /// record's end and code; the end of the token before; the stream's end; the bytes taken.
    next: u32, start: u32, end: u32, rec_end: u32, code: u32, last_end: u32, at_end: bool, need: bool,
    /// Where tier 2 makes the records: the place of a region's next part (0: none pending).
    part_at: u32,
    /// The marks not yet taken or passed at this token.
    live: u64,
    /// The nodes made, open; the deferred extras' leaves, and the end before them; the
    /// supertypes.
    nodes: u32, nopen: u32, pending: u32, keep: u32, supers: u32,
    /// The node made last at the innermost level (one more than its row); the node the last
    /// wrap moved.
    last_made: u32, moved: u32,
    counts: [u32; COUNTS],
    /// The calls open.
    depth: u32,
    /// Where the program is one function (`OneFn`): each open call's site.
    ret: [u16; DEPTH as usize],
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
    let extra = match (extras, n.extras.first()) {
        (true, Some(&(bit, kind))) => format!(
            "            if c >> {} & 1 != 0 {{\n{extra_end}                self.leaf::<true, true>({kind}, self.rec_end);\n                self.room()?;\n                continue;\n            }}\n",
            crate::records::MARK_SHIFT + u32::from(bit)
        ),
        _ => String::new(),
    };
    // The token marks that refuse (`TokenMark::refuses`): their bits in the code, one test; where
    // one is set, `refusals` (cold) tests their tokens.
    let bits: u64 = n.token_marks.iter().filter(|m| m.refuses).fold(0, |b, m| b | 1u64 << (crate::records::MARK_SHIFT + u32::from(m.order)));
    let refusals = if bits == 0 { String::new() } else { format!("            if u64::from(c) & {bits:#x} != 0 && self.refusals() {{\n                return Err(false);\n            }}\n") };
    writeln!(
        out,
        "    /// `TOKEN`: the next record{}.
    #[inline(always)]
    fn {name}(&mut self) -> R {{
        self.room()?;
{test}        if self.at_end {{
            return Err(false);
        }}
        loop {{
{before}            let i = self.next as usize;
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
        x.end_bits
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
                    wide |= u32::from(o > 0xffff) * WIDE_OFF;
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
        if n & (STRIDE - 1) != 0 {
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
            *self.tail_c.add(at as usize) = tail as TailT;
            if tail > TAIL_MAX $EARLY {
                self.wide |= WIDE_TAIL;
            }
        }
    }

    /// Nodes from the last commit to `to` final: their codes written, each key's id numbered by
    /// first use.
    #[inline(always)]
    fn commit(&mut self, to: u32) {
        self.ncodes = self.commit_from(self.coded, to, self.ncodes);
        self.coded = to;
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
    fn uncode(&mut self, node: u32) {
        if node < self.coded {
            self.uncode_from(node);
        }
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

    /// A call's number, kept in its container's row (the node open last), whose end the `CLOSE`
    /// writes (`OneFn::stacked`).
    #[inline(always)]
    fn keep_ret(&mut self, k: u32) {
        unsafe { (*self.row(*Self::col(self.open, self.nopen - 1))).end = k };
    }

    /// The number `keep_ret` kept in the node open last, before its `CLOSE`.
    #[inline(always)]
    fn taken_ret(&self) -> u32 {
        unsafe { (*self.row(*Self::col(self.open, self.nopen - 1))).end }
    }

    /// `CLOSE`: its end, size and tail; the pending extras after it are its parent's. A node
    /// closed at the top level: the nodes before it are final, their codes committed.
    #[inline(always)]
    fn close<const THIS: bool, const SURE: bool>(&mut self, this_end: u32) -> R {
        if !SURE && self.nopen == 0 {
            return Err(false);
        }
        self.nopen -= 1;
        let at = unsafe { *Self::col(self.open, self.nopen) };
        let end = if THIS { this_end } else if self.pending != 0 { self.keep } else { self.last_end };
        unsafe { (*self.row(at)).end = end };
        $CLOSE_MADE
        let kept = self.nodes - self.pending;
        self.close_cells(at, kept - at, kept - 1, end);
        if self.nopen == 1 {
            self.commit(at);
        }
        Ok(())
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
    fn made(&self) -> Result<u32, bool> {
        if self.nopen == 0 || self.last_made == 0 {
            return Err(false);
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
                *self.tail_c.add(n as usize) = *self.tail_c.add(r as usize);$HEAD_MOVE
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
                *self.tail_c.add(i as usize) = *self.tail_c.add(j);$HEAD_SHIFT
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
            for j in from..kept_n {
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
                return Err(false);
            }
            (*self.row(r + 1)).field = field;
        }
        Ok(())
    }

    #[inline(always)]
    fn here_kind(&mut self, kind: u16) -> R {
        if self.nopen == 0 {
            return Err(false);
        }
        unsafe { (*self.row(*Self::col(self.open, self.nopen - 1))).kind = kind };
        Ok(())
    }

    /// `RETAG m=1`: the node the last wrap moved takes a kind.
    #[inline(always)]
    fn moved_kind(&mut self, kind: u16) {
        unsafe { (*self.row(self.moved)).kind = kind };
    }

    /// `MERGE m=1`: the innermost open node's two leaves, all it holds, one; closed again.
    #[inline(always)]
    fn fold(&mut self, kind: u16) -> R {
        if self.nopen == 0 {
            return Err(false);
        }
        let o = unsafe { *Self::col(self.open, self.nopen - 1) };
        if self.nodes != o + 3 || self.pending != 0 {
            return Err(false);
        }
        let (a, b) = (o + 1, o + 2);
        unsafe {
            if *self.size_c.add(a as usize) != 1 || *self.size_c.add(b as usize) != 1 {
                return Err(false);
            }
            let end = (*self.row(b)).end;
            (*self.row(a)).kind = kind;
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
            return Err(false);
        }
        let leaf = kept - 1;
        unsafe {
            if *self.size_c.add(leaf as usize) != 1 || (self.nopen != 0 && *Self::col(self.open, self.nopen - 1) >= leaf) {
                return Err(false);
            }
            (*self.row(leaf)).kind = kind;
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
    let s = src
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
        .replace("$FOLD_MADE", &f("self.last_made = b;"));
    // A node's supertypes (its `NODE_SUPER` records, in node order): a bit each of its key,
    // the class's place in `classes` (`native_cpu_tape::commit`).
    let (first, bits) = if x.classes.is_empty() {
        (String::new(), String::new())
    } else {
        let arms: Vec<String> = x.classes.iter().enumerate().map(|(j, cl)| format!("{cl} => 1 << {}", 16 + j)).collect();
        (
            "\n            let mut sk = self.supers;\n            while sk > 0 && *Self::col(self.snode, sk - 1) >= from {\n                sk -= 1;\n            }".to_string(),
            format!("\n                let mut k = k;\n                while sk < self.supers && *Self::col(self.snode, sk) == i {{\n                    k |= match *Self::col(self.skind, sk) {{ {}, _ => 0 }};\n                    sk += 1;\n                }}", arms.join(", ")),
        )
    };
    out.push_str(&s.replace("$CLASS_FIRST", &first).replace("$CLASS_BITS", &bits));
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
            "        for i in 0..nodes {{\n            if matches!(unsafe {{ (*self.row(i)).field }}, {}) {{\n                return Err(false);\n            }}\n        }}\n",
            fields.join(" | ")
        )
    };
    let extras: Vec<String> = n.readings.extras.iter().map(u16::to_string).collect();
    let is_extra = if extras.is_empty() { "false".to_string() } else { format!("matches!(k & 0x3ff, {})", extras.join(" | ")) };
    let few = if x.widths.size == 2 { "\n        if nodes > 0xffff {\n            return Err(false);\n        }" } else { "" };
    let src = r#"    /// `ACCEPT`.
    #[inline(never)]
    fn accept(&mut self) -> R {
        if !$AT_END || self.nopen != 1 {
            return Err(false);
        }
        let nodes = self.nodes;
        let root = unsafe { *self.open };
        unsafe { (*self.row(root)).end = self.len };
        self.close_cells(root, nodes - root, nodes - 1, self.len);
$PROVISIONAL        if self.wide != 0 {
            return Err(false);
        }$FEW
        self.commit(nodes);
        let total = self.finish(nodes);
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
    out.push_str(&src.replace("$AT_END", &x.at_end("self")).replace("$PROVISIONAL", &provisional).replace("$FEW", few).replace("$IS_EXTRA", &is_extra).replace("$CLASS_PUT", &put));
}

/// `RETAG m=3`: reading `i` over the node made last at the innermost level and the nodes inside
/// it, in preorder (its rows); a kind each, its field the reading's. The dropped are taken out
/// after (`drop_dead`).
fn reread(out: &mut String, i: usize, r: &crate::stream_ir::Reading) {
    let way = |w: u16| -> String {
        match w {
            KEEP => "{ i += u32::from(*self.size_c.add(i as usize)); continue; }".to_string(),
            GONE => "{ if self.dropped_in(i) { return Err(false) } if i == root { self.last_made = 0 } (*row).kind = DEAD; gone = true; }".to_string(),
            REMEMBERED => "{ if self.dropped_in(i) || word == 0 { return Err(false) } (*row).kind = word as u16; }".to_string(),
            k => format!("{{ if self.dropped_in(i) {{ return Err(false) }} (*row).kind = {k}; }}"),
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
{kinds}                        _ => return Err(false),
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
/// that are no extra. A step a record.
fn pattern(out: &mut String, i: usize, m: &TokenMark, extra_bit: u8) {
    let shift = crate::records::MARK_SHIFT + u32::from(extra_bit);
    let mut body = String::new();
    let test = |el: &crate::program::TokenElem| -> String {
        let mut t = Vec::new();
        if el.kinds != 0 {
            t.push(format!("((c & 0xff) < 64 && {:#x}u64 >> (c & 0xff) & 1 != 0)", el.kinds));
        }
        if !el.words.is_empty() {
            let w: Vec<String> = el.words.iter().map(|&s| (u32::from(s) + 1).to_string()).collect();
            t.push(format!("matches!((c >> 8) & 0xff, {})", w.join(" | ")));
        }
        let t = if t.is_empty() { "false".into() } else { t.join(" || ") };
        // A `not` element: none of its choices.
        if el.not { format!("!({t})") } else { t }
    };
    let mut ahead = 0i8;
    for el in &m.elems {
        match el.offset {
            0 => writeln!(body, "        let c = unsafe {{ *self.codes.add(here * self.rs) }};\n        if !({}) {{ return false; }}", test(el)).unwrap(),
            o if o < 0 => {
                // The record before, past the extras (an offset of -1).
                writeln!(body, "        let mut j = here;\n        let c = loop {{\n            if j == 0 {{ return false; }}\n            j -= 1;\n            let c = unsafe {{ *self.codes.add(j * self.rs) }};\n            if c >> {shift} & 1 == 0 {{ break c; }}\n        }};\n        if !({}) {{ return false; }}", test(el)).unwrap();
            }
            o => {
                // No record left: an `end` element holds there (it is the last), any other fails.
                let past = if el.end { "true" } else { "false" };
                for _ in ahead..o {
                    writeln!(body, "        let c = loop {{\n            at += 1;\n            if at >= self.n {{ return {past}; }}\n            let c = unsafe {{ *self.codes.add(at * self.rs) }};\n            if c >> {shift} & 1 == 0 {{ break c; }}\n        }};").unwrap();
                }
                ahead = o;
                writeln!(body, "        if !({}) {{ return false; }}", test(el)).unwrap();
            }
        }
    }
    writeln!(out, "    /// Token mark {i}'s tokens follow.\n    #[inline(never)]\n    fn pattern_{i}(&self) -> bool {{\n        let here = self.next as usize - 1;\n        let mut at = here;\n{body}        true\n    }}\n").unwrap();
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
            for l in 0..16 {
                if pat >> l & 1 == 1 {
                    lo[l] |= bit;
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
                return Err(false);
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
            return "return Err(false);".into();
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
fn arms(p: &mut Plan, scrutinee: &str, values: &[u16]) -> String {
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
        let pats: Vec<String> = runs.iter().map(|&(a, b)| if a == b { format!("{a}") } else { format!("{a}..={b}") }).collect();
        let j = p.to(t);
        writeln!(s, "{} => {{ {j} }}", pats.join(" | ")).unwrap();
    }
    match dflt {
        Some(t) => {
            let j = p.to(t);
            writeln!(s, "_ => {{ {j} }}").unwrap();
        }
        None => s.push_str("_ => return Err(false),\n"),
    }
    s.push_str("}\n");
    s
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
        p.code = format!("if {} {{ {j} }} else {{\n{b}}}\n", x.at_end("s"));
        return p;
    }
    if let (true, Some(reps)) = (on_kinds(x, sw), &x.kind_reps) {
        let mut t: Vec<u16> = reps.iter().map(|&b| sw.targets[usize::from(b)]).collect();
        if x.end_kind.is_some() {
            t.push(sw.end);
        }
        let head = if x.end_kind.is_none() {
            let j = p.to(sw.end);
            format!("if {} {{ {j} }} else ", x.at_end("s"))
        } else {
            String::new()
        };
        let mut by_word = String::new();
        let mut values = vec![REFUSE; 256];
        for (k, &tk) in t.iter().enumerate() {
            let named: Vec<(u16, u16)> = sw.words.iter().copied().filter(|&(w, _)| k < reps.len() && x.word_kind_of.get(usize::from(w)).is_some_and(|&y| usize::from(y) == k)).collect();
            if named.is_empty() {
                values[k] = tk;
                continue;
            }
            let mut inner = String::from("match s.word() {\n");
            for (w, to_w) in named {
                let j = p.to(to_w);
                writeln!(inner, "{} => {{ {j} }}", u32::from(w) + 1).unwrap();
            }
            let j = p.to(tk);
            writeln!(inner, "_ => {{ {j} }}\n}}").unwrap();
            writeln!(by_word, "{k} => {{ {inner} }}").unwrap();
        }
        let rest = arms(&mut p, "kind", &values);
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
    let mut out = format!("if {} {{ {j} }} else {{\n", x.at_end("s"));
    if sw.offset == 0 && !sw.words.is_empty() {
        out.push_str("match s.word() {\n");
        for &(w, t) in &sw.words {
            let j = p.to(t);
            writeln!(out, "{} => {{ {j} }}", u32::from(w) + 1).unwrap();
        }
        out.push_str("_ => {}\n}\n");
    }
    writeln!(out, "let at = s.start + {};\nif at >= s.rec_end {{ return Err(false); }}", sw.offset).unwrap();
    out.push_str(&arms(&mut p, "unsafe { *s.bytes.add(at as usize) }", &sw.targets));
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
    let kind_of = |k: u16| -> String { if k == REMEMBERED { "{ if word == 0 { return Err(false) } word as u16 }".into() } else { format!("{k}") } };
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
                    return Ok(format!("if s.depth == DEPTH {{ return Err(false); }}\ns.depth += 1;\ns.keep_ret({k});\n{}", jump(0)?));
                }
                return Ok(format!("if s.depth == DEPTH {{ return Err(false); }}\ns.ret[s.depth as usize] = {k};\ns.depth += 1;\n{}", jump(0)?));
            }
            // A return: the site off the stack, then its place (`scope_end`).
            Op::Return { .. } => {
                let arms = one.returns.get(&pc).map_or(&[][..], Vec::as_slice);
                let mut m = String::from(if one.stacked { "s.depth -= 1;\nmatch ret {\n" } else { "s.depth -= 1;\nmatch s.ret[s.depth as usize] {\n" });
                for (i, &(k, _)) in arms.iter().enumerate() {
                    writeln!(m, "{k} => {{ {} }}", jump(i)?).unwrap();
                }
                // Every open call's site is one of these (`OneFn`).
                m.push_str("_ => return Err(false),\n}\n");
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
        Op::Span { last: false, len } => format!("if s.start + {len} != s.rec_end {{ return Err(false); }}\n{}{}", if x.flagged { "s.end = s.rec_end;\ns.need = true;\n" } else { "" }, jump(0)?),
        Op::Span { last: true, .. } => format!("{}{}", if x.flagged { "s.end = s.rec_end;\ns.need = true;\n" } else { "" }, jump(0)?),
        Op::Empty { total } => format!("if s.total({total}) != 0 {{ return Err(false); }}\n{}", jump(0)?),
        Op::Word { string, test: true } => format!("if s.word() == {} {{ {} }} else {{ {} }}", u32::from(string) + 1, jump(0)?, jump(1)?),
        Op::Word { string, test: false } => format!("if s.word() != {} {{ return Err(false); }}\n{}", u32::from(string) + 1, jump(0)?),
        Op::Open { kind } => format!("s.open({});\n{}", u32::from(kind) | u32::from(field_after(x, pc).unwrap_or(0)) << 16, jump(0)?),
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
                (true, _) if constant == Some(None) => return Ok("return Err(false);".into()),
                (true, f) if constant.is_some() => format!("{}", constant.flatten().unwrap_or(0) | f),
                (true, 0) => "s.word_kind()?".to_string(),
                (true, f) => format!("s.word_kind()? | {f}"),
                (false, f) => format!("{}", u32::from(kind) | f),
            };
            format!("s.leaf::<{deferred}, {extra}>({k}, {end});\n{}", jump(0)?)
        }
        Op::Settle { .. } => format!("s.pending = 0;\n{}", jump(0)?),
        Op::Mark { mask, order, end, keep, word: mark_word, pattern } => {
            let k = u32::from(order);
            let here = if (k as usize) < x.in_code {
                // In the record, folded into `live` at the token (the end's marks too).
                match pattern {
                    Some(i) => format!("(s.at_end || s.pattern_{i}())"),
                    None if x.lazy.iter().any(|&(m, _)| u32::from(m) == k) => format!("(s.at_end || s.lazy_{k}())"),
                    None => "true".into(),
                }
            } else {
                if pattern.is_some() {
                    return Err("a token mark not in the records".into());
                }
                let o = mask_output(n, mask)?;
                let w = mark_word.map_or(String::new(), |s| format!("s.word() == {} && ", u32::from(s) + 1));
                format!("(if s.at_end {{ {end} }} else {{ {w}s.bit({o}, s.start) }})")
            };
            let take = if keep { String::new() } else { format!("s.live &= !((2u64 << {k}) - 1);\n") };
            format!("if s.live >> {k} & 1 != 0 && {here} {{\n{take}{}\n}} else {{\n{}\n}}", jump(0)?, jump(1)?)
        }
        Op::Row { mask } => format!("if s.bit({}, s.start) {{ {} }} else {{ {} }}", mask_output(n, mask)?, jump(0)?, jump(1)?),
        Op::Retag { kind, on: On::Here } => format!("s.here_kind({kind})?;\n{}", jump(0)?),
        Op::Retag { kind, on: On::Moved } => format!("s.moved_kind({});\n{}", kind_of(kind), jump(0)?),
        Op::Retag { kind, on: On::Made } => format!("s.made_kind::<{}>({})?;\n{}", kind == DEAD, kind_of(kind), jump(0)?),
        Op::Reread { reading } => format!("s.reread_{reading}({})?;\n{}", if x.context { "word" } else { "0" }, jump(0)?),
        Op::Wrap { kind } => format!("s.wrap({kind})?;\n{}", jump(0)?),
        Op::Merge { kind, fold: true } => format!("s.fold({kind})?;\n{}", jump(0)?),
        Op::Merge { kind, fold: false } => format!("s.merge({kind}, {end})?;\n{}", jump(0)?),
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
            format!("if s.depth == DEPTH {{ return Err(false); }}\ns.depth += 1;\nlet again = r{callee}(s{args})?;\ns.depth -= 1;\nif again {{ {} }} else {{ {} }}", jump(1)?, jump(0)?)
        }
        Op::Remember { kind: WORD_KIND } => format!("word = s.word_kind()?;\n{}", jump(0)?),
        Op::Remember { kind } => format!("word = {kind};\n{}", jump(0)?),
        Op::Flag { flag, op: FlagOp::Set } => format!("flags |= {};\n{}", 1u32 << flag, jump(0)?),
        Op::Flag { flag, op: FlagOp::Clear } => format!("flags &= !{};\n{}", 1u32 << flag, jump(0)?),
        Op::Flag { flag, op: FlagOp::Test } => format!("if flags & {} != 0 {{ {} }} else {{ {} }}", 1u32 << flag, jump(0)?, jump(1)?),
        Op::Count { count, up: true, max } => format!("s.counts[{count}] += 1;\nif s.counts[{count}] > {max} {{ return Err(false); }}\n{}", jump(0)?),
        Op::Count { count, up: false, .. } => format!("s.counts[{count}] = s.counts[{count}].saturating_sub(1);\n{}", jump(0)?),
        Op::Check { check } => format!("s.check_{check}()?;\n{}", jump(0)?),
        Op::Return { again } => format!("return Ok({again});"),
        Op::Accept => "s.accept()?;\nreturn Ok(false);".into(),
        Op::Stop => "return Err(false);".into(),
        Op::Goto { .. } => jump(0)?,
        ref op => return Err(format!("word {pc}: {op:?} is not tier 2's")),
    })
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
    let body = shape.tree(&emit, 0)?;
    let params = if x.context { ", mut word: u32, mut flags: u32" } else { "" };
    // The call's number a `RETURN` matches on, from its container's row (`OneFn::stacked`).
    let ret = if x.one.as_ref().is_some_and(|o| o.stacked) { "    let mut ret: u32 = 0;\n" } else { "" };
    writeln!(out, "/// The rule entered at word {entry}.\n#[inline(never)]\nfn r{entry}(s: &mut S{params}) -> Result<bool, bool> {{\n    let mut d: u32 = 0;\n{ret}{body}}}\n").unwrap();
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
            return Err(false);
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
fn fetched_helpers(out: &mut String, x: &Facts<'_>, fx: &Fetched) -> Result<(), String> {
    let ft = &fx.ft;
    let mut consts = String::new();
    out.push_str(
        "    /// The lanes of a vector that are all ones, four bits each (`vshrn`, as simdjson's
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

    /// The 8 bytes from `at`, a word (the file has 64 readable bytes after it).
    #[inline(always)]
    fn word8(&self, at: u32) -> u64 {
        unsafe { (self.bytes.add(at as usize) as *const u64).read_unaligned() }
    }

    #[inline(always)]
    fn byte(&self, at: u32) -> u8 {
        unsafe { *self.bytes.add(at as usize) }
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
    fn part_stop(&self, at: u32) -> Result<(u32, bool), bool> {{
        let mut p = at;
        loop {{
            if p >= self.len {{
                return Err(false);
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
            0 => format!("s.rec_end = s.start + 1;\n{}{}", code_set(x, &[one], a), p.to(one)),
            1 => format!("s.rec_end = s.start + 1;\n{}s.part_at = s.start + 1;\n{}", code_set(x, &[one], a), p.to(one)),
            _ => {
                let r = a;
                let mut words = String::new();
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
                        Some((code_text, past)) => (format!("{code_text}{}", p.to(past)), code_set(x, &[past], *code)),
                        None => (p.to(at), code_set(x, &[at], *code)),
                    };
                    writeln!(
                        words,
                        "if st + {n} <= s.len && s.word8(st) & {mask:#x} == {want:#x} && (st + {n} == s.len || S::CLASS_{r}[usize::from(s.byte(st + {n}))] == 0) {{\n// {:?}\ns.rec_end = st + {n};\n{set}{to}\n}}",
                        String::from_utf8_lossy(w)
                    )
                    .unwrap();
                }
                match check_at(x, t).filter(|_| t != REFUSE) {
                    // The check at its place made in the pass that finds its end, then on past it.
                    Some((q, ci, _)) => {
                        let past = p.to(q as u16 + 1);
                        let long = p.to(t);
                        let (sp, sl) = (code_set(x, &[q as u16 + 1], kind), code_set(x, &[t], kind));
                        format!("let st = s.start;\n{words}match s.run_{r}_check_{ci}(st)? {{\nSome(l) => {{\ns.rec_end = l + 1;\n{sp}{past}\n}}\nNone => {{\ns.rec_end = s.run_{r}(st) + 1;\n{sl}{long}\n}}\n}}")
                    }
                    None => format!("let st = s.start;\n{words}s.rec_end = s.run_{r}(st) + 1;\n{}{}", code_set(x, &[t], kind), p.to(t)),
                }
            }
        };
        writeln!(s, "{} => {{\n{body}\n}}", pats.join(" | ")).unwrap();
    }
    match delegate {
        Some((at, _)) => {
            let j = p.to(at);
            writeln!(s, "_ => {{ {j} }}\n}}").unwrap();
        }
        None => s.push_str("_ => return Err(false),\n}\n"),
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
    let (content, then) = match straight_to(x, pf.content, pc) {
        Some(code) => (format!("{code}{room}s.last_end = s.rec_end;\n"), "}\n"),
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
    let by_word = arms(&mut p, "s.word()", &to);
    p.code = format!(
        "{}s.last_end = s.rec_end;
let at = s.part_at;
let (i, closer) = s.part_stop(at)?;
if i != at {{
// The bytes before the stop: content (its code read by no word after).
s.start = at;
s.rec_end = i;
s.part_at = i;
{content}
{then}if closer {{
// The closer: the region's last token.
s.start = i;
s.rec_end = i + 1;
{}s.part_at = 0;
{closer}
}} else {{
// A part of two bytes (an escape): its code by its second byte.
if i + 1 >= s.len {{ return Err(false); }}
s.start = i;
s.rec_end = i + 2;
s.code = S::CODES2[usize::from(s.byte(i + 1))];
s.part_at = i + 2;
{by_word}}}
",
        if x.unchecked { "" } else { "s.room()?;\n" },
        code_set(x, &[past_span(x, pf.closer, 1)], pf.closer_code)
    );
    p
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
        return if back == 0 { set } else if set == [0; 4] { set } else { ALL };
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

/// A token's check's ops as Rust, a `let v{k}` each, over one window of places (64 of them,
/// `wide`, else 16): the byte sets by the classes `c{t}` (four vectors, or one), a `prev`
/// carrying its operand's last window (`pv{a}`) and a `fill` its carry (`cin{k}`) where `wide`.
/// What carries to the next window goes to `after`.
fn check_ops(c: &LocalCheck, w: &WordForm, wide: bool, step: &mut String, after: &mut String) {
    let carried: BTreeSet<usize> = c.ops.iter().filter_map(|o| if let LocalOp::Prev(a, _) = *o { Some(usize::from(a)) } else { None }).collect();
    for (k, op) in c.ops.iter().enumerate() {
        let v = |a: u16| format!("v{a}");
        let e = match *op {
            LocalOp::Bytes(_) => match w.class_of[&k] {
                (_, 0) => "0".into(),
                (ti, bits) if wide => format!("bits(c{ti}.map(|v| vtstq_u8(v, vdupq_n_u8({bits}))))"),
                (ti, bits) => format!("Self::bits16(vtstq_u8(c{ti}, vdupq_n_u8({bits})))"),
            },
            LocalOp::Const(pat) => format!("{pat:#x}u64"),
            LocalOp::Within(d) => format!("S::within(q as i64, {d}, left + {d})"),
            LocalOp::Start => "if q == 0 { 1 } else { 0 }".into(),
            LocalOp::Last => "S::within(q as i64, n as i64 - 1, n as i64)".into(),
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
            LocalOp::Ended => unreachable!("refused by `token_check_ok`"),
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
    if !token_check_ok(c) {
        return Err(format!("check {i}: a token's check that reads back, or of a delay of 64 or more: not yet ahead of time"));
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
    let mut step = String::from("            let b = self.window(st + 64 * q);\n");
    for &ti in &used {
        writeln!(step, "            let c{ti} = [0, 1, 2, 3].map(|v| vandq_u8(vqtbl1q_u8(lo{ti}, vandq_u8(b[v], vdupq_n_u8(15))), vqtbl1q_u8(hi{ti}, vshrq_n_u8::<4>(b[v]))));").unwrap();
    }
    let mut after = String::new();
    check_ops(c, &w, true, &mut step, &mut after);
    let last = c.ops.len() - 1;
    let d = c.delay;
    writeln!(
        out,
        "    /// `CHECK {i}`, a token's check: over its bytes and {d} after them, 64 bytes a step.
    #[inline(always)]
    fn check_{i}(&self) -> R {{
        let st = self.start;
        let n = self.rec_end - st;
        let left = self.len as i64 - st as i64;
        unsafe {{
{body}{lets}        let mut q = 0u32;
        while 64 * q < n + {d} {{
{step}            if v{last} & S::within(q as i64, {d}, n as i64 + {d}) != 0 {{
                return Err(false);
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
    fn run_{r}_check_{ci}(&self, st: u32) -> Result<Option<u32>, bool> {{
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
                return Err(false);
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
