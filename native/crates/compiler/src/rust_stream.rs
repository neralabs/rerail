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
    let records = native.block_programs.first().and_then(|b| b.records.as_ref());
    let starts = native.structural.iter().any(|b| matches!(b.operation, Op::Token { .. }));
    let in_code = records.map_or(0, |r| r.marks.len());
    let rest_bits: u64 = if in_code >= 64 { 0 } else { u64::MAX << in_code };
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
    let facts = Facts { native, in_code, rest_bits, end_bits, kind_reps, end_kind, word_kind_of, framed, context, entries: entries.into_iter().collect(), phase, flagged, widths, tag };
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
        outputs: c.outputs as *const u8, ostride: c.output_stride as usize, lstride: c.lasts_stride as usize,
        totals: c.totals as *const u8, tstride: c.totals_stride as usize, tmask: c.totals_mask,
        rows: c.rows, snode: c.super_node, skind: c.super_kind, open: c.open, counts_out: c.counts,
        fbase: c.frame_base, head: c.head, link: c.link, place: c.place,
        size_c: c.size_c as *mut SizeT, off_c: c.off_c as *mut OffT, tail_c: c.tail_c as *mut TailT, base_c: c.base_c,
        image: c.image, id_of: c.id_of, code_kf: c.code_kf, code_first: c.code_first,
        block_base: 0, coded: 0, ncodes: 0, wide: 0,
        room_nodes: c.room_nodes as usize, room_supers: c.room_supers as usize,
        next: 0, start: 0, end: 0, rec_end: 0, code: 0, last_end: 0, at_end: false, need: true, live: 0,
        nodes: 0, nopen: 0, pending: 0, keep: 0, supers: 0, last_made: 0, moved: 0,
        counts: [0; COUNTS], depth: 0,
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
        token_step(out, x, any, extras, x.flagged)?;
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
    let mut on_p = String::new();
    for (i, c) in n.local_checks.iter().enumerate() {
        let o = mask_output(n, c.mask)?;
        check(out, &mut on_p, i, c, o)?;
    }
    out.push_str("}\n\n");
    writeln!(out, "impl S {{\n{on_p}}}\n").unwrap();
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
         const CODE_EXTRA: u8 = {};\nconst SENTINEL: u16 = {:#x};\nconst MAGIC: u64 = {:#x};\nconst HEAD: u32 = {:#x};\nconst FORMAT_VERSION: u32 = {};\n\
         const HEAD_AT: usize = {};\nconst VERSION_AT: usize = {};\nconst SOURCE_LEN_AT: usize = {};\nconst NODES_AT: usize = {};\n",
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
        t::CODE_EXTRA,
        t::SENTINEL,
        u64::from_le_bytes(*t::MAGIC),
        head,
        t::FORMAT_VERSION,
        t::header::LANGUAGE,
        t::header::VERSION,
        t::header::SOURCE_LEN,
        t::header::NODES,
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
    image: *mut u8, id_of: *mut u16, code_kf: *mut u16, code_first: *mut u32,
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
    outputs: *const u8, ostride: usize, lstride: usize, totals: *const u8, tstride: usize, tmask: u64,
    rows: *mut Row, snode: *mut u32, skind: *mut u32, open: *mut u32, counts_out: *mut u64,
    /// Scratch for a drop (`drop_dead`); which rows are wraps (`inner_field`).
    fbase: *mut u32, head: *mut u32, link: *mut u32, place: *mut u32,
    /// The tape (docs/parser-generators/37): its columns at their widths, the bases, the image
    /// (the header's room, then the code column in place), each key's code id and each code's
    /// key and first node.
    size_c: *mut SizeT, off_c: *mut OffT, tail_c: *mut TailT, base_c: *mut u32,
    image: *mut u8, id_of: *mut u16, code_kf: *mut u16, code_first: *mut u32,
    /// The last block's base; the nodes committed and the codes; the values past their width.
    block_base: u32, coded: u32, ncodes: u32, wide: u32,
    room_nodes: usize, room_supers: usize,
    /// The next record; the token's start, end (its start until its bytes are taken), its
    /// record's end and code; the end of the token before; the stream's end; the bytes taken.
    next: u32, start: u32, end: u32, rec_end: u32, code: u32, last_end: u32, at_end: bool, need: bool,
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
            let (st, c) = unsafe {{ (*self.starts.add(i), *self.codes.add(i)) }};
            self.start = st;
{set_end}            self.rec_end = unsafe {{ *self.lasts.add(i) }} + 1;
            self.code = c;
            self.next = i as u32 + 1;
{extra}            self.live = {live};
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
            let mut i = from;
            while i < to {
                let key = *(self.row(i) as *const u32);
                let k = (key & 0x3ff) | ((key >> 16) & 0x3f) << KIND_BITS;
                let slot = self.id_of.add(k as usize);
                let mut id = u32::from(*slot);
                if id == NO_CODE {
                    *slot = c as u16;
                    *self.code_kf.add(c as usize) = k as u16;
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

    /// `CLOSE`: its end, size and tail; the pending extras after it are its parent's. A node
    /// closed at the top level: the nodes before it are final, their codes committed.
    #[inline(always)]
    fn close<const THIS: bool>(&mut self, this_end: u32) -> R {
        if self.nopen == 0 {
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
    out.push_str(&s);
}

/// `ACCEPT`: the root closed at the file's end; no provisional field left; no value past its
/// width; the rest committed; the image's end (`native_cpu_tape::finish`): the columns moved
/// into place after the code column, the code table, the header (format 15: no stored
/// digest).
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
        if !self.at_end || self.nopen != 1 {
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
            for c in 0..codes {
                let k = *self.code_kf.add(c);
                let e = at.add(6 * c);
                (e as *mut u16).write_unaligned(k);
                *e.add(2) = if $IS_EXTRA { CODE_EXTRA } else { 0 };
                (e.add(3) as *mut u16).write_unaligned(SENTINEL);
                *e.add(5) = 0;
                *self.id_of.add(k as usize) = NO_CODE as u16;
            }
            let table = 6 * codes;
            let total = at.add(table) as usize - image as usize;
            (image as *mut u64).write_unaligned(MAGIC);
            (image.add(HEAD_AT) as *mut u32).write_unaligned(HEAD);
            (image.add(VERSION_AT) as *mut u32).write_unaligned(FORMAT_VERSION);
            (image.add(SOURCE_LEN_AT) as *mut u64).write_unaligned(self.len as u64);
            for (k, v) in [n, codes, table, 0, 0].into_iter().enumerate() {
                (image.add(NODES_AT + 8 * k) as *mut u64).write_unaligned(v as u64);
            }
            total
        }
    }
"#;
    out.push_str(&src.replace("$PROVISIONAL", &provisional).replace("$FEW", few).replace("$IS_EXTRA", &is_extra));
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
        if t.is_empty() { "false".into() } else { t.join(" || ") }
    };
    let mut ahead = 0i8;
    for el in &m.elems {
        match el.offset {
            0 => writeln!(body, "        let c = unsafe {{ *self.codes.add(here) }};\n        if !({}) {{ return false; }}", test(el)).unwrap(),
            o if o < 0 => {
                // The record before, past the extras (an offset of -1).
                writeln!(body, "        let mut j = here;\n        let c = loop {{\n            if j == 0 {{ return false; }}\n            j -= 1;\n            let c = unsafe {{ *self.codes.add(j) }};\n            if c >> {shift} & 1 == 0 {{ break c; }}\n        }};\n        if !({}) {{ return false; }}", test(el)).unwrap();
            }
            o => {
                for _ in ahead..o {
                    writeln!(body, "        let c = loop {{\n            at += 1;\n            if at >= self.n {{ return false; }}\n            let c = unsafe {{ *self.codes.add(at) }};\n            if c >> {shift} & 1 == 0 {{ break c; }}\n        }};").unwrap();
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
fn check(out: &mut String, on_p: &mut String, i: usize, c: &LocalCheck, o: usize) -> Result<(), String> {
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
            let a = self.mask_word({o}, w) & S::within(w as i64, st as i64, lim as i64);
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
        if self.mask_word({o}, w) >> (st & 63) == 0 && (lim - 1) >> 6 == w {{
            return Ok(());
        }}
        self.check_{i}_words(st, lim)
    }}

"
    )
    .unwrap();
    Ok(())
}

/// A word's successors in its function: its next, its targets, a call's two returns.
fn succ(x: &Facts<'_>, pc: usize) -> Vec<usize> {
    let n = x.native.structural.len();
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
    if let (true, Some(reps)) = (on_kinds(x, sw), &x.kind_reps) {
        let mut t: Vec<u16> = reps.iter().map(|&b| sw.targets[usize::from(b)]).collect();
        if x.end_kind.is_some() {
            t.push(sw.end);
        }
        let head = if x.end_kind.is_none() {
            let j = p.to(sw.end);
            format!("if s.at_end {{ {j} }} else ")
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
        p.code = format!("{head}{{ let kind = s.code & 0xff;\n{body}}}");
        return p;
    }
    let j = p.to(sw.end);
    let mut out = format!("if s.at_end {{ {j} }} else {{\n");
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
    let n = x.native;
    let kind_of = |k: u16| -> String { if k == REMEMBERED { "{ if word == 0 { return Err(false) } word as u16 }".into() } else { format!("{k}") } };
    // The token's end, as this word reads it: its phase's (`phases`), or the run's field.
    let taken = x.phase[pc] == crate::stream::TAKEN;
    let end = if x.flagged {
        "s.end"
    } else if taken {
        "s.rec_end"
    } else {
        "(if s.at_end { s.rec_end } else { s.start })"
    };
    Ok(match n.structural[pc].operation {
        Op::Token { any, extras, .. } if x.flagged || taken => format!("s.{}()?;\n{}", token_name(any, extras), jump(0)?),
        // No token to take here: the next word switches on the one untaken.
        Op::Token { .. } => format!("s.room()?;\n{}", jump(0)?),
        Op::Switch { table } => switch_plan(x, &n.switches[usize::from(table)]).fill(jump)?,
        Op::Span { last: false, len } => format!("if s.start + {len} != s.rec_end {{ return Err(false); }}\n{}{}", if x.flagged { "s.end = s.rec_end;\ns.need = true;\n" } else { "" }, jump(0)?),
        Op::Span { last: true, .. } => format!("{}{}", if x.flagged { "s.end = s.rec_end;\ns.need = true;\n" } else { "" }, jump(0)?),
        Op::Empty { total } => format!("if s.total({total}) != 0 {{ return Err(false); }}\n{}", jump(0)?),
        Op::Word { string, test: true } => format!("if s.word() == {} {{ {} }} else {{ {} }}", u32::from(string) + 1, jump(0)?, jump(1)?),
        Op::Word { string, test: false } => format!("if s.word() != {} {{ return Err(false); }}\n{}", u32::from(string) + 1, jump(0)?),
        Op::Open { kind } => format!("s.open({});\n{}", u32::from(kind) | u32::from(field_after(x, pc).unwrap_or(0)) << 16, jump(0)?),
        Op::Close { this } => format!("s.close::<{this}>({})?;\n{}", if this { end } else { "0" }, jump(0)?),
        Op::Leaf { kind, deferred, extra } => {
            let f = u32::from(field_after(x, pc).unwrap_or(0)) << 16;
            let k = match (kind == WORD_KIND, f) {
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
    let shape = Shape::of(first, &sizes).ok_or_else(|| format!("rule at {entry}: an irreducible graph"))?;
    let emit = |node: usize, jump: &dyn Fn(usize) -> Result<String, String>| -> Result<String, String> { word(x, nodes[node], jump) };
    let body = shape.tree(&emit, 0)?;
    let params = if x.context { ", mut word: u32, mut flags: u32" } else { "" };
    writeln!(out, "/// The rule entered at word {entry}.\n#[inline(never)]\nfn r{entry}(s: &mut S{params}) -> Result<bool, bool> {{\n    let mut d: u32 = 0;\n{body}}}\n").unwrap();
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
