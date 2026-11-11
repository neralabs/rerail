//! Block programs on Metal with the carry going forward only: a file is cut into **segments** of
//! 64-byte blocks, and no segment waits on another. A segment runs in one of two forms:
//!
//! - in **lanes** (the default): one SIMD group, a block a lane; a `prev` takes the lane
//!   before's word, a scan its lanes' prefix (`lanes_function`);
//! - in the CPU's shape (`RERAIL_SERIAL_LANES=0`): one thread runs the blocks in order, the
//!   carried state in its registers, as the Cranelift loop does on one core
//!   (`native_cpu_block`).
//!
//! The carry goes forward only, by speculation:
//!
//! 0. The scans' carries are made exact first (`Levels`): level by level, each segment's table of
//!    each scan (its carry out for each carry in), then a prefix over each file's segments.
//! 1. Every segment starts a few blocks early (its **warm-up**): its scans from their exact
//!    carries, its `prev`s from zero. It writes its state at its first block and at its end.
//! 2. A check compares each segment's first state with its predecessor's end. A segment whose
//!    start differs is run again from its predecessor's end; the check repeats until every
//!    start agrees. A file's first segment starts exact.
//! 3. Last, the extracts' positions and the totals are made from the finished output words: each
//!    segment counts its bits, each file's segments take their bases by one SIMD-group prefix,
//!    and each segment writes its positions.
//!
//! A segment's words of the outputs depend only on its start and its bytes, so a run again
//! rewrites them all with plain stores. The body of a block is the same MSL the lanes kernel
//! uses (`emit`), at 64 bits; a carried instruction reads the thread's state.
use super::*;

/// How a batch runs in segments: the blocks a segment holds, and the blocks before it it warms
/// up on.
#[derive(Clone, Copy, Debug)]
pub struct Segments {
    pub blocks: usize,
    pub warm: usize,
}

/// A serial batch's results: the files', and how the speculation went.
pub struct SerialRun {
    pub run: BatchRun,
    /// Segments in the batch, and segments run again in all.
    pub segments: usize,
    pub reruns: usize,
    /// Checks until every start agreed (1 when the speculation was right everywhere).
    pub rounds: usize,
    /// GPU seconds of the first pass, of the runs again with their checks, and of the positions.
    pub first_seconds: f64,
    pub rerun_seconds: f64,
    pub sink_seconds: f64,
}

/// The carried state of a program in serial form, as packed words: each `prev`'s operand by its
/// newest bits (the most any `prev` of it reads), then each scan's carry bit. A component never
/// spans two words.
struct State {
    /// `(instruction, bits, word, first bit, scan)`: an operand of `prev`s (its last word `s`),
    /// or a scan (its carry `c`). A scan read by a `prev` has both.
    parts: Vec<(usize, u32, usize, u32, bool)>,
    words: usize,
}

impl State {
    fn of(native: &BlockNative) -> Self {
        let mut reach: std::collections::BTreeMap<usize, u32> = std::collections::BTreeMap::new();
        for inst in &native.insts {
            if let BlockInst::Prev(a, k) = *inst {
                let r = reach.entry(usize::from(a)).or_default();
                *r = (*r).max(u32::from(k));
            }
        }
        let mut list: Vec<(usize, u32, bool)> = reach.into_iter().map(|(at, bits)| (at, bits, false)).collect();
        list.extend(native.insts.iter().enumerate().filter(|(_, i)| matches!(i, BlockInst::Toggle(_) | BlockInst::Escaped(_) | BlockInst::Fill(..))).map(|(at, _)| (at, 1, true)));
        let (mut parts, mut word, mut used) = (Vec::new(), 0usize, 0u32);
        for (at, bits, scan) in list {
            if used + bits > 64 {
                word += 1;
                used = 0;
            }
            parts.push((at, bits, word, used, scan));
            used += bits;
        }
        State { words: word + usize::from(used > 0 || parts.is_empty()), parts }
    }

    /// The MSL that writes the state's words to `to[0..]`.
    fn pack(&self, native: &BlockNative, to: &str) -> String {
        let mut words = vec![Vec::new(); self.words];
        let _ = native;
        for &(at, bits, w, first, scan) in &self.parts {
            let value = if scan { format!("ulong(cy{at})") } else { format!("(s{at} >> {}u)", 64 - bits) };
            words[w].push(format!("({value} << {first}u)"));
        }
        words.iter().enumerate().map(|(w, parts)| format!("{to}[{w}] = {};", if parts.is_empty() { "0ul".to_owned() } else { parts.join(" | ") })).collect::<Vec<_>>().join(" ")
    }

    /// The MSL that sets the state from the words at `from[0..]`.
    fn unpack(&self, native: &BlockNative, from: &str) -> String {
        self.parts
            .iter()
            .map(|&(at, bits, w, first, scan)| {
                let _ = native;
                let field = format!("(({from}[{w}] >> {first}u) & {:#x}ul)", if bits == 64 { u64::MAX } else { (1u64 << bits) - 1 });
                if scan {
                    format!("cy{at} = uint({field});")
                } else {
                    format!("s{at} = {field} << {}u;", 64 - bits)
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// Program `p` in serial form: the blocks `first..=last` of a segment after its warm-up (or from
/// a given state), one body a block: the general one at the file's edges, the full one inside,
/// and the ASCII one inside where the block and the 16 bytes before it are ASCII.
fn serial_function(p: usize, native: &BlockNative, state: &State, levels: Option<&Levels>, stores: &[u16]) -> String {
    let mut out = format!(
        "static inline void serial_{p}(device const uint4* bytes, FileRow f, uint first, uint last, bool again, uint warm,\n\
         \x20                          device ulong* out, device ulong* rec, thread const ulong* given, ulong cin, ulong cw) {{\n"
    );
    for &(at, .., scan) in &state.parts {
        if scan {
            out.push_str(&format!("  uint cy{at} = 0u;\n"));
        } else {
            out.push_str(&format!("  ulong s{at} = 0ul;\n"));
        }
    }
    let delayed: Vec<usize> = (0..native.outputs.len()).filter(|&k| native.output_delays[k] > 0).collect();
    for &k in &delayed {
        assert!(native.output_delays[k] < 64, "an output's delay is below a block");
        out.push_str(&format!("  ulong q{k} = 0ul;\n"));
    }
    let start = state.unpack(native, "given");
    // The scans' carries are exact (`Levels`): from `cw` at the warm-up's first block, set to
    // `cin` at its second part's first, `a`.
    let carries = |x: &str| match levels {
        Some(levels) => levels.scans.iter().enumerate().filter(|&(i, _)| levels.level[i] > 0).map(|(i, at)| format!(" cy{at} = uint(({x} >> {i}u) & 1ul);")).collect::<String>(),
        None => String::new(),
    };
    out.push_str(&format!(
        "  const uint a = first - min(first, warm);\n\
         \x20 uint b0 = a - min(a, warm);\n\
         \x20 if (again) {{ b0 = first; {start} }} else {{{} }}\n",
        carries("(b0 < a ? cw : cin)")
    ));
    out.push_str(&format!(
        "  const uint len = f.len;\n\
         \x20 // A block's bytes are loaded a block ahead: the loads of the next overlap this one's work.\n\
         {load_ahead}\
         \x20 // The block after the segment only finishes its delayed words.\n\
         \x20 for (uint b = b0; b <= last; b++) {{\n\
         {next_block}\
         \x20   if (!again && b == a) {{{} }}\n\
         \x20   if (b == first) {{ {} }}\n\
         \x20   if (b == last) {{ {} }}\n",
        carries("cin"),
        state.pack(native, "rec"),
        state.pack(native, &format!("(rec + {}u)", state.words)),
        load_ahead = load_ahead("b0"),
        next_block = NEXT_BLOCK,
    ));
    if delayed.is_empty() {
        out.push_str("    if (b == last) break;\n");
    }
    // The tail of every body: the sinks, then the state for the next block.
    let mut tail = String::new();
    for (k, &v) in native.outputs.iter().enumerate() {
        match native.output_delays[k] {
            0 => tail.push_str(&format!("      if (b >= first && b < last) out[f.out_base + {k}u * f.groups + b] = stream_cut(v{v}, b, len, 0u);\n")),
            d => tail.push_str(&format!(
                "      {{ const ulong w = stream_cut(v{v}, b, len, {d}u); if (b > first) out[f.out_base + {k}u * f.groups + b - 1u] = (q{k} >> {d}u) | (w << {}u); q{k} = w; }}\n",
                64 - u32::from(d)
            )),
        }
    }
    // A pass before the last (`native_ir::Passes`): each output to its slot, as made.
    for (k, &slot) in stores.iter().enumerate() {
        tail.push_str(&format!("      if (b >= first && b < last) out[f.slot_base + b * SLOTS{p} + {slot}u] = v{};\n", native.outputs[k]));
    }
    for &(at, .., scan) in &state.parts {
        match native.insts[at] {
            BlockInst::Toggle(_) if scan => tail.push_str(&format!("      cy{at} = uint(v{at} >> 63u);\n")),
            BlockInst::Escaped(_) | BlockInst::Fill(..) if scan => tail.push_str(&format!("      cy{at} = o{at};\n")),
            _ => tail.push_str(&format!("      s{at} = v{at};\n")),
        }
    }
    let body = |values: &BlockNative| {
        let all: Vec<usize> = (0..values.insts.len()).collect();
        let mut text = emit(p, values, LaneWidth::W64, &all, "", "b * 64u", carried_serial);
        text.push_str(&tail);
        text.lines().map(|l| format!("    {l}\n")).collect::<String>()
    };
    let (full, ascii) = (full_form(native), ascii_form(native));
    out.push_str(&format!(
        "    if (b > 0u && (b + 1u) * 64u <= len) {{\n\
         \x20     const uint4 high = pl | c0 | c1 | c2 | c3;\n\
         \x20     // One body for the SIMD group's threads at once: a thread with a byte above 127 takes the\n\
         \x20     // others to the full body.\n\
         \x20     if (simd_all(((high.x | high.y | high.z | high.w) & 0x80808080u) == 0u)) {{\n{}      }} else {{\n{}      }}\n\
         \x20   }} else {{\n{}    }}\n    pl = c3;\n  }}\n}}\n\n",
        body(&ascii),
        body(&full),
        body(native)
    ));
    out
}

/// Program `p` in lanes form: a segment's blocks run across one SIMD group, block `a + l` in
/// lane `l`, from `a`, its warm-up's first block. The carry goes forward only: a `prev` takes
/// the lane before's word (zero in lane 0), a scan its lanes' tables' prefix from the segment's
/// carry, exact (`cin`, from the levels) or 0. No group waits on another. The lane before the
/// segment's first block and its last block's lane write the state, as `serial_function` does,
/// so the check and the runs again are the serial ones. A delayed output's word takes the next
/// lane's bits: the block after the segment is run too.
fn lanes_function(p: usize, native: &BlockNative, state: &State, levels: Option<&Levels>, stores: &[u16]) -> String {
    let mut out = format!(
        "static inline void lanes_{p}(device const uint4* bytes, FileRow f, uint first, uint last, bool again, uint warm, uint l,\n\
         \x20                          device ulong* out, device ulong* rec, thread const ulong* given, ulong cin, ulong cw, device uint* counts) {{\n\
         \x20 // `aw` the levels' segment start, where `cin` holds; `a` a warm-up before it, where `cw` holds.\n\
         \x20 const uint aw = first - min(first, warm);\n\
         \x20 const uint a = again ? first : aw - min(aw, warm);\n\
         \x20 const uint b = a + l;\n\
         \x20 const uint len = f.len;\n"
    );
    // A run again starts at the segment's first block from `given`: lane 0's halos and the scans'
    // carries; its start is written as given.
    for &(at, .., scan) in &state.parts {
        out.push_str(&if scan { format!("  uint cy{at} = 0u;\n") } else { format!("  ulong s{at} = 0ul;\n") });
    }
    // A first run's `prev`s before lane 0 read what its exact carries there give (`halos`).
    if let Some(lv) = levels {
        let h = halos(native, lv, "cw", usize::MAX, &[]);
        let mut set = String::new();
        for &(at, _, _, _, scan) in &state.parts {
            if !scan && h.word(at) != "0ul" {
                set.push_str(&format!("  s{at} = {};\n", h.word(at)));
            }
        }
        if !set.is_empty() {
            out.push_str(&format!("  if (!again) {{\n{}{set}  }}\n", h.decls));
        }
    }
    out.push_str(&format!("  if (again) {{ {} if (l == 0u) {{ {} }} }}\n", state.unpack(native, "given"), state.pack(native, "rec")));
    // An exact scan is set to `cin` at `aw`: its prefix starts again there, the lanes before
    // it from `cw` (their `prev`s from zero would make it wrong at `aw`, as the levels' passes
    // never are).
    let exact = |at: usize| match levels {
        Some(lv) if lv.level[lv.bit(at)] > 0 => {
            let i = lv.bit(at);
            (format!("(again ? cy{at} : uint((cw >> {i}u) & 1ul))"), Some(("!again && b >= aw".to_owned(), format!("uint((cin >> {i}u) & 1ul)"))))
        }
        _ => (format!("(again ? cy{at} : 0u)"), None),
    };
    let carried = |at: usize, inst: &BlockInst, out: &mut String| match *inst {
        BlockInst::Prev(a, k) => out.push_str(&format!("  const ulong v{at} = apply_prev(v{a}, {k}u, halo_before(v{a}, l, s{a}));\n")),
        _ => {
            let (from, reset) = exact(at);
            lane_scan(at, inst, &from, reset.as_ref().map(|(c, v)| (c.as_str(), v.as_str())), out)
        }
    };
    let mut tail = String::from("  const bool mine = b >= first && b < last;\n");
    for (k, &v) in native.outputs.iter().enumerate() {
        match native.output_delays[k] {
            // Output words are `ow`: `o` is a scan's carry out.
            0 => tail.push_str(&format!("  const ulong ow{k} = select(0ul, stream_cut(v{v}, b, len, 0u), mine);\n")),
            d => tail.push_str(&format!(
                "  const ulong ww{k} = stream_cut(v{v}, b, len, {d}u), wn{k} = ulong(simd_shuffle_down(uint(ww{k}), 1u)) | (ulong(simd_shuffle_down(uint(ww{k} >> 32u), 1u)) << 32u);\n  const ulong ow{k} = select(0ul, (ww{k} >> {d}u) | (wn{k} << {}u), mine);\n",
                64 - u32::from(d)
            )),
        }
        tail.push_str(&format!("  if (mine) out[f.out_base + {k}u * f.groups + b] = ow{k};\n"));
    }
    // A pass before the last (`native_ir::Passes`): each output to its slot, as made.
    for (k, &slot) in stores.iter().enumerate() {
        tail.push_str(&format!("  if (mine) out[f.slot_base + b * SLOTS{p} + {slot}u] = v{};\n", native.outputs[k]));
    }
    // The segment's count of each counted output (`counted_outputs`), for the positions' bases
    // and the totals.
    // The totals' one count is every total's words OR-ed (`native_cpu_block::counted`).
    for (c, k) in counted_outputs(native).into_iter().enumerate() {
        let w = if c < native.extracts.len() { format!("ow{k}") } else { native.totals.iter().map(|t| format!("ow{t}")).collect::<Vec<_>>().join(" | ") };
        tail.push_str(&format!("  {{ const uint n = simd_sum(uint(popcount({w}))); if (l == 0u) counts[{c}u] = n; }}\n"));
    }
    // The state after the lane, under the serial names (`State::pack`).
    let mut names = String::new();
    for &(at, .., scan) in &state.parts {
        if scan {
            names.push_str(&format!(" const uint cy{at} = co{at};"));
        } else {
            names.push_str(&format!(" const ulong s{at} = v{at};"));
        }
    }
    tail.push_str(&format!(
        "  {{{names}\n    if (l + 1u == first - a) {{ {} }}\n    if (l + 1u == last - a) {{ {} }} }}\n",
        state.pack(native, "rec"),
        state.pack(native, &format!("(rec + {}u)", state.words))
    ));
    let all: Vec<usize> = (0..native.insts.len()).collect();
    out.push_str(&lane_bodies(p, native, &all, &carried, &tail));
    out.push_str("}\n\n");
    out
}

/// The bodies of lane block `b` for the instructions `chosen`, then `tail`: the general one
/// at the file's edges, the full one inside, and the ASCII one where the group's blocks and the
/// 16 bytes before each are ASCII. The group takes one body: the choice is `simd_all`.
fn lane_bodies(p: usize, native: &BlockNative, chosen: &[usize], carried: &dyn Fn(usize, &BlockInst, &mut String), tail: &str) -> String {
    let body = |values: &BlockNative| {
        let mut text = emit(p, values, LaneWidth::W64, chosen, "b * 4u", "b * 64u", carried);
        text.push_str(tail);
        text.lines().map(|l| format!("    {l}\n")).collect::<String>()
    };
    let (full, ascii) = (full_form(native), ascii_form(native));
    format!(
        "  const bool inside = b > 0u && (b + 1u) * 64u <= len;\n\
         \x20 if (simd_all(inside)) {{\n\
         \x20   const uint4 h = load16(bytes, f, b * 4u - 1u) | load16(bytes, f, b * 4u) | load16(bytes, f, b * 4u + 1u) | load16(bytes, f, b * 4u + 2u) | load16(bytes, f, b * 4u + 3u);\n\
         \x20   if (simd_all(((h.x | h.y | h.z | h.w) & 0x80808080u) == 0u)) {{\n{}    }} else {{\n{}    }}\n\
         \x20 }} else {{\n{}  }}\n",
        body(&ascii),
        body(&full),
        body(native)
    )
}

/// Scan `at` in lanes form, its carry into the group's lane 0 `from`: the lanes' tables'
/// prefix. Defines `v{at}`, its carry out of the lane `co{at}` and the lane's table `tt{at}`.
fn lane_scan(at: usize, inst: &BlockInst, from: &str, reset: Option<(&str, &str)>, out: &mut String) {
    // With `reset` (a lane condition, a carry): the lanes where it holds take their own prefix,
    // from the carry, instead.
    let carry = |at: usize| match reset {
        None => format!("tapply1(texclusive1(tt{at}, l, agg{at}), {from})"),
        // Both prefixes in every lane: a SIMD-group function under a branch is not.
        Some((cond, cin)) => format!(
            "select(tapply1(texclusive1(tt{at}, l, agg{at}), {from}), tapply1(texclusive1(select(2u, tt{at}, {cond}), l, agr{at}), {cin}), {cond})"
        ),
    };
    let agr = if reset.is_some() { format!("  uint agr{at} = 0u;\n") } else { String::new() };
    out.push_str(&agr);
    match *inst {
        BlockInst::Toggle(a) => out.push_str(&format!(
            "  const uint sm{at} = summary_toggle(v{a});\n  const uint tt{at} = sm{at} | ((1u ^ sm{at}) << 1u);\n  uint agg{at} = 0u;\n  const uint ci{at} = {};\n  const ulong v{at} = apply_toggle(v{a}, ci{at});\n  const uint co{at} = uint(v{at} >> 63u);\n",
            carry(at)
        )),
        BlockInst::Escaped(_) | BlockInst::Fill(..) => {
            let call = |c: &str, o: &str| match *inst {
                BlockInst::Escaped(a) => format!("escaped(v{a}, {c}, {o})"),
                BlockInst::Fill(a, e) => format!("fill_word(v{a}, v{e}, {c}, {o})"),
                _ => unreachable!(),
            };
            out.push_str(&format!(
                "  uint o{at}_0 = 0u, o{at}_1 = 0u;\n  const ulong f{at}_0 = {}, f{at}_1 = {};\n  const uint tt{at} = (o{at}_0 & 1u) | ((o{at}_1 & 1u) << 1u);\n  uint agg{at} = 0u;\n  const uint ci{at} = {};\n  const ulong v{at} = select(f{at}_0, f{at}_1, ci{at} != 0u);\n  const uint co{at} = ci{at} != 0u ? o{at}_1 : o{at}_0;\n",
                call("0ul", &format!("o{at}_0")),
                call("1ul", &format!("o{at}_1")),
                carry(at),
            ));
        }
        _ => unreachable!("a scan"),
    }
}

/// Level `l`'s pass in lanes form (`cone_function`'s): one SIMD group a segment, block
/// `start + l` in lane `l` from the warm-up's first block `start`. The scans below start from
/// their exact carries `cw` there and are exact through the group; the level's own tables are
/// composed over the lanes of `a..a_next` and of `a..a_next - warm`.
fn lanes_cone_function(p: usize, native: &BlockNative, levels: &Levels, lv: usize) -> String {
    let own = levels.at(lv);
    let mut inside = vec![false; native.insts.len()];
    for &(_, at) in &own {
        for x in native.insts[at].operands() {
            inside[x] = true;
        }
    }
    let cone = cone_of(native, inside);
    let out = format!(
        "static inline void lcone_{p}_{lv}(device const uint4* bytes, device const ulong* out, FileRow f, uint a, uint a_next, uint warm, uint l, ulong cin, ulong cw, device uint* tab) {{\n\
         \x20 const uint start = a - min(a, warm);\n\
         \x20 const ulong from = start < a ? cw : cin;\n\
         \x20 const uint b = start + l;\n\
         \x20 const uint len = f.len;\n"
    );
    let mut out = out;
    let h = halos(native, levels, "from", lv, &[]);
    out.push_str(&h.decls);
    let carried = |at: usize, inst: &BlockInst, out: &mut String| match *inst {
        BlockInst::Prev(a, k) => out.push_str(&format!("  const ulong v{at} = apply_prev(v{a}, {k}u, halo_before(v{a}, l, {}));\n", h.word(usize::from(a)))),
        _ => {
            let i = levels.bit(at);
            let from = if levels.level[i] > 0 { format!("uint((from >> {i}u) & 1ul)") } else { "0u".to_owned() };
            lane_scan(at, inst, &from, None, out);
        }
    };
    let mut tail = String::from("  const bool span = b >= a && b < a_next, span_w = b >= a && b + warm < a_next;\n");
    for &(k, at) in &own {
        let mut scan = String::new();
        lane_scan(at, &native.insts[at], "0u", None, &mut scan);
        tail.push_str(&scan);
        tail.push_str(&format!(
            "  {{ uint g0 = 0u, g1 = 0u; texclusive1(select(2u, tt{at}, span), l, g0); texclusive1(select(2u, tt{at}, span_w), l, g1);\n    if (l == 0u) {{ tab[{}u] = g0; tab[{}u] = g1; }} }}\n",
            2 * k,
            2 * k + 1
        ));
    }
    out.push_str(&lane_bodies(p, native, &cone, &carried, &tail));
    out.push_str("}\n\n");
    out
}

/// What each `prev` reads before a segment's first lane, from the carries known there: a scan's
/// word ends in its carry out (a toggle's parity, a fill's open region), a group's scan in its
/// state, a carry in its next, a `prev` in its operand's end (the block before is taken as the
/// same to its end: a multi-byte `prev` reads that bit `k` times); what reads the bytes is
/// unknown, and an unknown word reads as zero, as a warm-up's `prev`s do. Each value's end is
/// a pair of MSL booleans, known and value (`hk`, `hv`), folded where constant. Without it a
/// segment inside a string reads its first byte as code. `from`: the carries at the first lane;
/// the scans known there are those of a level below `limit`, and a group's of `own` from its
/// state `c`.
struct Halos {
    decls: String,
    words: std::collections::HashMap<usize, String>,
}

#[derive(Clone, PartialEq)]
enum HBit {
    F,
    T,
    E(String),
}

fn h_not(x: &HBit) -> HBit {
    match x {
        HBit::F => HBit::T,
        HBit::T => HBit::F,
        HBit::E(e) => HBit::E(format!("!{e}")),
    }
}

fn h_and(x: &HBit, y: &HBit) -> HBit {
    match (x, y) {
        (HBit::F, _) | (_, HBit::F) => HBit::F,
        (HBit::T, o) | (o, HBit::T) => o.clone(),
        (HBit::E(a), HBit::E(b)) => HBit::E(format!("({a} && {b})")),
    }
}

fn h_or(x: &HBit, y: &HBit) -> HBit {
    match (x, y) {
        (HBit::T, _) | (_, HBit::T) => HBit::T,
        (HBit::F, o) | (o, HBit::F) => o.clone(),
        (HBit::E(a), HBit::E(b)) => HBit::E(format!("({a} || {b})")),
    }
}

fn halos(native: &BlockNative, levels: &Levels, from: &str, limit: usize, own: &[usize]) -> Halos {
    let n = native.insts.len();
    let mut memo: Vec<Option<(HBit, HBit)>> = vec![None; n];
    let mut decls = String::new();
    fn go(native: &BlockNative, levels: &Levels, from: &str, limit: usize, own: &[usize], at: usize, memo: &mut Vec<Option<(HBit, HBit)>>, decls: &mut String) -> (HBit, HBit) {
        if let Some(x) = &memo[at] {
            return x.clone();
        }
        // A way back to itself (through a carry) is unknown.
        memo[at] = Some((HBit::F, HBit::F));
        let e = |x: u16, memo: &mut Vec<Option<(HBit, HBit)>>, decls: &mut String| go(native, levels, from, limit, own, usize::from(x), memo, decls);
        let unknown = (HBit::F, HBit::F);
        let owner = levels.groups.iter().enumerate().find(|(_, g)| g.scans.contains(&at));
        let (k, v) = match native.insts[at] {
            BlockInst::Toggle(_) | BlockInst::Fill(..) => match owner {
                Some((gi, g)) if own.contains(&gi) => (HBit::T, HBit::E(format!("(c == {}u)", g.scans.iter().position(|&x| x == at).expect("a member") + 1))),
                _ => {
                    let i = levels.bit(at);
                    if levels.level[i] > 0 && levels.level[i] < limit { (HBit::T, HBit::E(format!("((({from} >> {i}u) & 1ul) != 0ul)"))) } else { unknown }
                }
            },
            BlockInst::Carry(_) => match native.groups.iter().flat_map(|g| g.carries.iter()).find(|&&(c, _)| c == at) {
                Some(&(_, x)) => go(native, levels, from, limit, own, x, memo, decls),
                None => unknown,
            },
            BlockInst::Prev(a, _) => e(a, memo, decls),
            BlockInst::Const(0) => (HBit::T, HBit::F),
            BlockInst::Const(u64::MAX) => (HBit::T, HBit::T),
            BlockInst::Not(a) => {
                let (k, v) = e(a, memo, decls);
                (k, h_not(&v))
            }
            BlockInst::And(a, b) | BlockInst::AndNot(a, b) | BlockInst::Or(a, b) | BlockInst::Xor(a, b) => {
                let (ka, va) = e(a, memo, decls);
                let (kb, mut vb) = e(b, memo, decls);
                if matches!(native.insts[at], BlockInst::AndNot(..)) {
                    vb = h_not(&vb);
                }
                match native.insts[at] {
                    BlockInst::Or(..) => (h_or(&h_or(&h_and(&ka, &kb), &h_and(&ka, &va)), &h_and(&kb, &vb)), h_or(&va, &vb)),
                    BlockInst::Xor(..) => (h_and(&ka, &kb), match (&va, &vb) {
                        (HBit::E(x), HBit::E(y)) => HBit::E(format!("({x} != {y})")),
                        (HBit::E(x), HBit::T) | (HBit::T, HBit::E(x)) => HBit::E(format!("!{x}")),
                        (HBit::E(x), HBit::F) | (HBit::F, HBit::E(x)) => HBit::E(x.clone()),
                        (x, y) => if x == y { HBit::F } else { HBit::T },
                    }),
                    _ => (h_or(&h_or(&h_and(&ka, &kb), &h_and(&ka, &h_not(&va))), &h_and(&kb, &h_not(&vb))), h_and(&va, &vb)),
                }
            }
            _ => unknown,
        };
        // A named pair: the expressions are used again by what reads the value.
        let mut name = |b: HBit, p: &str| match b {
            HBit::E(x) => {
                decls.push_str(&format!("  const bool {p}{at} = {x};\n"));
                HBit::E(format!("{p}{at}"))
            }
            other => other,
        };
        let k = name(k, "hk");
        let v = if k == HBit::F { HBit::F } else { name(v, "hv") };
        memo[at] = Some((k.clone(), v.clone()));
        (k, v)
    }
    let mut words = std::collections::HashMap::new();
    for inst in &native.insts {
        if let BlockInst::Prev(a, _) = *inst {
            let a = usize::from(a);
            if words.contains_key(&a) {
                continue;
            }
            let (k, v) = go(native, levels, from, limit, own, a, &mut memo, &mut decls);
            let word = match h_and(&k, &v) {
                HBit::F => "0ul".to_owned(),
                HBit::T => "~0ul".to_owned(),
                HBit::E(x) => format!("(0ul - ulong({x}))"),
            };
            words.insert(a, word);
        }
    }
    Halos { decls, words }
}

impl Halos {
    fn word(&self, a: usize) -> String {
        self.words.get(&a).cloned().unwrap_or_else(|| "0ul".to_owned())
    }
}

/// The values `seeds` read, through anything, themselves included: a fixed point's members all
/// once any is (it runs as one step, `schedule`).
fn cone_of(native: &BlockNative, mut inside: Vec<bool>) -> Vec<usize> {
    loop {
        for at in (0..native.insts.len()).rev() {
            if inside[at] {
                for x in native.insts[at].operands() {
                    inside[x] = true;
                }
            }
        }
        let mut grew = false;
        for g in &native.groups {
            if g.members.iter().any(|&m| inside[m]) {
                for &m in &g.members {
                    grew |= !std::mem::replace(&mut inside[m], true);
                }
            }
        }
        if !grew {
            return (0..native.insts.len()).filter(|&at| inside[at]).collect();
        }
    }
}

/// Level `lv`'s fixed points in lanes form, from state `c` (`Leveled`): one SIMD group a
/// segment and a state, block `start + l` in lane `l` from the warm-up's first block `start`,
/// the group's scans' carries there one-hot from `c`, everything below from its exact carries
/// (`cw`), the `prev`s from zero. Each group writes the state it reaches at `a` (`tab[0..8]`)
/// and at the next segment's warm-up start, `a_next - warm` (`tab[8..16]`), by `c`.
fn lanes_group_function(p: usize, native: &BlockNative, levels: &Levels, lv: usize) -> String {
    let own = levels.groups_at(lv);
    let mut inside = vec![false; native.insts.len()];
    for (_, g) in &own {
        for &m in &native.groups[g.group].members {
            inside[m] = true;
        }
    }
    let cone = cone_of(native, inside);
    let mut out = format!(
        "static inline void lgroup_{p}_{lv}(device const uint4* bytes, device const ulong* out, FileRow f, uint a, uint a_next, uint warm, uint l, uint c, ulong cin, ulong cw, device uint* tab) {{\n\
         \x20 const uint start = a - min(a, warm);\n\
         \x20 const ulong from = start < a ? cw : cin;\n\
         \x20 const uint b = start + l;\n\
         \x20 const uint len = f.len;\n"
    );
    let own_ids: Vec<usize> = own.iter().map(|(gi, _)| *gi).collect();
    let h = halos(native, levels, "from", lv, &own_ids);
    out.push_str(&h.decls);
    let carried = |at: usize, inst: &BlockInst, out: &mut String| match *inst {
        BlockInst::Prev(a, k) => out.push_str(&format!("  const ulong v{at} = apply_prev(v{a}, {k}u, halo_before(v{a}, l, {}));\n", h.word(usize::from(a)))),
        _ => {
            let from = match own.iter().find_map(|(_, g)| g.scans.iter().position(|&x| x == at)) {
                Some(k) => format!("uint(c == {}u)", k + 1),
                None => {
                    let i = levels.bit(at);
                    if levels.level[i] > 0 { format!("uint((from >> {i}u) & 1ul)") } else { "0u".to_owned() }
                }
            };
            lane_scan(at, inst, &from, None, out);
        }
    };
    // A file's first segment starts at its own start: its state there is `c`.
    let mut tail = String::new();
    for (gi, g) in &own {
        let n = g.scans.len();
        let count: Vec<String> = g.scans.iter().map(|&x| format!("co{x}")).collect();
        let which: Vec<String> = g.scans.iter().enumerate().map(|(k, &x)| format!("co{x} * {}u", k + 1)).collect();
        tail.push_str(&format!(
            "  {{ const uint n = {count}, st = c > {n}u ? {NONE}u : n == 0u ? 0u : n == 1u ? {which} : {NONE}u;\n\
             \x20   if (l == 0u && start == a) tab[{t0}u + c] = c > {n}u ? {NONE}u : c;\n\
             \x20   if (b + 1u == a && b >= start) tab[{t0}u + c] = st;\n\
             \x20   if (b + 1u + warm == a_next && b >= start) tab[{t1}u + c] = st; }}\n",
            count = count.join(" + "),
            which = which.join(" + "),
            t0 = gi * 2 * GROUP_STATES,
            t1 = gi * 2 * GROUP_STATES + GROUP_STATES,
        ));
    }
    out.push_str(&lane_bodies(p, native, &cone, &carried, &tail));
    out.push_str("}\n\n");
    out
}

/// Group `gi`'s states over a file's segments, from their tables (`lanes_group_function`), as
/// `scan_prefix` does a scan's: each segment's state at its warm-up start sets its scans' bits
/// of `cw`, its state at `a` theirs of `cin`. `NONE` sets none.
fn group_prefix(p: usize, gi: usize, g: &Leveled, levels: &Levels, stride: usize) -> String {
    let bits: Vec<String> = g.scans.iter().enumerate().map(|(k, &x)| format!("c == {}u ? {:#x}ul", k + 1, 1u64 << levels.bit(x))).collect();
    format!(
        "static inline ulong gbits_{p}_{gi}(uint c) {{ return {} : 0ul; }}\n\
         static inline uint gtable_{p}_{gi}(device const uint* gtabs, uint j, uint which) {{\n\
         \x20 device const uint* t = gtabs + j * {stride}u + {base}u + which * {GROUP_STATES}u;\n\
         \x20 uint x = 0u;\n\
         \x20 for (uint c = 0u; c < {GROUP_STATES}u; c++) x |= (c == {NONE}u ? {NONE}u : (t[c] & 7u)) << (3u * c);\n\
         \x20 return x;\n}}\n\
         static inline void gprefix_{p}_{gi}(FileRow f, uint l, uint sg, uint id, threadgroup uint* tg, device const uint* gtabs, device ulong* cin, device ulong* cw) {{\n\
         \x20 const uint n = f.halo_base, run = (n + {pt}u - 1u) / {pt}u;\n\
         \x20 const uint lo = min(id * run, n), hi = min(lo + run, n);\n\
         \x20 uint mine = TID3;\n\
         \x20 for (uint s = lo; s < hi; s++) mine = tcompose3(mine, gtable_{p}_{gi}(gtabs, f.group0 + s, 1u));\n\
         \x20 uint total = 0u;\n\
         \x20 const uint e = texclusive3(mine, l, total);\n\
         \x20 threadgroup_barrier(mem_flags::mem_threadgroup);\n\
         \x20 if (l == 0u) tg[sg] = total;\n\
         \x20 threadgroup_barrier(mem_flags::mem_threadgroup);\n\
         \x20 uint before = TID3;\n\
         \x20 for (uint k = 0u; k < sg; k++) before = tcompose3(before, tg[k]);\n\
         \x20 uint c = tapply3(tcompose3(before, e), 0u);\n\
         \x20 for (uint s = lo; s < hi; s++) {{\n\
         \x20   const uint j = f.group0 + s;\n\
         \x20   cw[j] |= gbits_{p}_{gi}(c);\n\
         \x20   cin[j] |= gbits_{p}_{gi}(tapply3(gtable_{p}_{gi}(gtabs, j, 0u), c));\n\
         \x20   c = tapply3(gtable_{p}_{gi}(gtabs, j, 1u), c);\n\
         \x20 }}\n\
         }}\n\n",
        bits.join(" : "),
        base = gi * 2 * GROUP_STATES,
        pt = PREFIX_THREADS,
    )
}

/// A program's scans by level: each scan's level is one more than the highest of the scans it
/// reads, through anything (`prev`s included). Level by level, a pass over every segment makes
/// each scan's table (its carry out for each carry in), and a prefix over each file's segments
/// gives every segment its exact carry in. A scan's table has two entries whatever it reads, so
/// the levels need no other state: only the `prev`s, which forget in k bytes, are left to the
/// warm-ups.
struct Levels {
    /// The scans in order: scan `i` is bit `i` of a segment's carries.
    scans: Vec<usize>,
    /// Each scan's level, from 1; 0 for a scan left to the warm-ups (`exact_scans`).
    level: Vec<usize>,
    depth: usize,
    /// The fixed points (`rounds until settled`) with a level of their own (`Leveled`).
    groups: Vec<Leveled>,
}

/// A fixed point taken as one carry of a few states: none of its scans' carries set, or one of
/// them (a lexical state: code, or inside one kind of region). Its level's pass runs each
/// segment from every state at the segment's warm-up start and maps it to the state at the next
/// one's, and at its own start; a prefix of those maps over the file's segments gives each
/// segment its state. An end with two carries set is no state (`NONE`): the segments after
/// it are speculated, as before. The check after the run stays: a state that misses a `prev`'s
/// bits only costs a run again.
struct Leveled {
    group: usize,
    level: usize,
    /// The group's scans, in order: state `k + 1` is scan `k`'s carry set.
    scans: Vec<usize>,
}

/// The states a fixed point's table maps (`table_helpers(3)`: 3-bit entries): its scans' one-hot
/// carries and none set, and `NONE`.
const GROUP_STATES: usize = 8;
const NONE: usize = GROUP_STATES - 1;

/// The scans made exact by levels (instruction numbers of the planes form): `RERAIL_SERIAL_EXACT`
/// (`all` for every scan), or what runs of the program have learned (`LEARNED`), or its
/// toggles. The others start from zero at each warm-up, as the `prev`s do: right when they
/// forget their start within a warm-up. A toggle never forgets (its carry is a parity), so it
/// is exact from the first run.
fn exact_scans(native: &BlockNative) -> Vec<usize> {
    let scans = native.insts.iter().enumerate().filter(|(_, i)| matches!(i, BlockInst::Toggle(_) | BlockInst::Escaped(_) | BlockInst::Fill(..)));
    match std::env::var("RERAIL_SERIAL_EXACT").ok().as_deref() {
        Some("all") => scans.map(|(at, _)| at).collect(),
        Some(v) => v.split(',').filter_map(|x| x.trim().parse().ok()).collect(),
        None => {
            let learned = LEARNED.lock().ok().and_then(|m| m.get(&fingerprint(native)).cloned()).unwrap_or_default();
            let mut exact: Vec<usize> = scans.filter(|(_, i)| matches!(i, BlockInst::Toggle(_))).map(|(at, _)| at).collect();
            exact.extend(learned);
            exact.sort_unstable();
            exact.dedup();
            exact
        }
    }
}

/// The scans each program's runs found speculation wrong for, by the program's fingerprint: a
/// scan whose speculated start was wrong in more than one segment in `PROMOTE` is made exact in
/// the program's later runs. A wrong start costs a run again of its segment (and of the ones
/// after, while its end differs); an exact scan costs a level's pass over every segment.
static LEARNED: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<u64, Vec<usize>>>> = std::sync::LazyLock::new(Default::default);
const PROMOTE: usize = 1000;

fn fingerprint(native: &BlockNative) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    format!("{:?}", native.insts).hash(&mut h);
    h.finish()
}

impl Levels {
    fn of(native: &BlockNative) -> Self {
        let exact_set = exact_scans(native);
        // A fixed point's values that has no level, and every value that reads one: a level's
        // pass makes none of them, so a scan among them is speculated, as the `prev`s are. A pass's slot words
        // (`Load`) are known: the passes before wrote them, and a level's pass reads them.
        let mut rounds: Vec<bool> = vec![false; native.insts.len()];
        let is_scan = |i: &BlockInst| matches!(i, BlockInst::Toggle(_) | BlockInst::Escaped(_) | BlockInst::Fill(..));
        // A fixed point with one to six scans has a level (in lanes form, `lanes_group_function`).
        let mut group_of: Vec<Option<usize>> = vec![None; native.insts.len()];
        let mut leveled = Vec::new();
        for (g, group) in native.groups.iter().enumerate() {
            let scans: Vec<usize> = group.members.iter().copied().filter(|&m| is_scan(&native.insts[m])).collect();
            if lanes_on() && (1..GROUP_STATES - 1).contains(&scans.len()) {
                for &m in &group.members {
                    group_of[m] = Some(leveled.len());
                }
                leveled.push(Leveled { group: g, level: 0, scans });
            } else {
                for &m in &group.members {
                    rounds[m] = true;
                }
            }
        }
        for at in 0..native.insts.len() {
            rounds[at] |= native.insts[at].operands().iter().any(|&o| rounds[o]);
        }
        // Each value's level: the most of its operands'; a scan's one more when it is exact; a
        // leveled group's members all one more than the most of what they read outside it. A
        // member may read a value made after the group's first member: to a fixed point.
        let mut lv = vec![0usize; native.insts.len()];
        let mut exact = vec![false; native.insts.len()];
        loop {
            let mut changed = false;
            for (at, inst) in native.insts.iter().enumerate() {
                let new = match group_of[at] {
                    Some(gi) => {
                        let group = &native.groups[leveled[gi].group];
                        1 + group.members.iter().flat_map(|&m| native.insts[m].operands()).filter(|&o| group_of[o] != Some(gi)).map(|o| lv[o]).max().unwrap_or(0)
                    }
                    None => {
                        let below = inst.operands().iter().map(|&x| lv[x]).max().unwrap_or(0);
                        exact[at] = is_scan(inst) && exact_set.contains(&at) && !rounds[at];
                        below + usize::from(exact[at])
                    }
                };
                if new != lv[at] {
                    lv[at] = new;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        let mut levels = Levels { scans: Vec::new(), level: Vec::new(), depth: 0, groups: Vec::new() };
        for (at, inst) in native.insts.iter().enumerate() {
            if is_scan(inst) {
                let level = if group_of[at].is_some() || exact[at] { lv[at] } else { 0 };
                levels.scans.push(at);
                levels.level.push(level);
                levels.depth = levels.depth.max(level);
            }
        }
        for mut g in leveled {
            g.level = lv[g.scans[0]];
            levels.groups.push(g);
        }
        assert!(levels.scans.len() <= 64, "a segment's carries fit a word");
        levels
    }

    /// The scans of level `l` outside the leveled groups, as (bit, instruction).
    fn at(&self, l: usize) -> Vec<(usize, usize)> {
        self.scans.iter().enumerate().filter(|&(i, &at)| self.level[i] == l && !self.groups.iter().any(|g| g.scans.contains(&at))).map(|(i, &at)| (i, at)).collect()
    }

    /// The leveled groups of level `l`, as (index in `groups`, group).
    fn groups_at(&self, l: usize) -> Vec<(usize, &Leveled)> {
        self.groups.iter().enumerate().filter(|(_, g)| g.level == l).collect()
    }

    fn bit(&self, at: usize) -> usize {
        self.scans.iter().position(|&x| x == at).expect("a scan")
    }
}

/// Level `l`'s pass for program `p` over blocks `a..a_next` (one segment's, a warm-up before
/// them): each scan of the level's table, at `a_next - warm` and at `a_next`. The scans below
/// start from their exact carries `cw` at the warm-up's first block and are set to `cin` at `a`;
/// the level's own run from 0 and from 1. Tables: bit c the carry out for carry in c.
fn cone_function(p: usize, native: &BlockNative, levels: &Levels, l: usize) -> String {
    let own = levels.at(l);
    let mut inside = vec![false; native.insts.len()];
    for &(_, at) in &own {
        for x in native.insts[at].operands() {
            inside[x] = true;
        }
    }
    let cone = cone_of(native, inside);
    let mut out = format!(
        "static inline void cone_{p}_{l}(device const uint4* bytes, device const ulong* out, FileRow f, uint a, uint a_next, uint warm, ulong cin, ulong cw, device uint* tab) {{\n\
         \x20 const uint start = a - min(a, warm);\n\
         \x20 const ulong from = start < a ? cw : cin;\n"
    );
    let (mut set, mut tail) = (String::new(), String::new());
    let mut prevs: std::collections::BTreeSet<usize> = std::collections::BTreeSet::new();
    for &at in &cone {
        match native.insts[at] {
            BlockInst::Prev(a, _) => {
                prevs.insert(usize::from(a));
            }
            BlockInst::Toggle(_) | BlockInst::Escaped(_) | BlockInst::Fill(..) => {
                let i = levels.bit(at);
                if levels.level[i] > 0 {
                    out.push_str(&format!("  uint cy{at} = uint((from >> {i}u) & 1ul);\n"));
                    set.push_str(&format!(" cy{at} = uint((cin >> {i}u) & 1ul);"));
                } else {
                    out.push_str(&format!("  uint cy{at} = 0u;\n"));
                }
                tail.push_str(&match native.insts[at] {
                    BlockInst::Toggle(_) => format!("      cy{at} = uint(v{at} >> 63u);\n"),
                    _ => format!("      cy{at} = o{at};\n"),
                });
            }
            _ => {}
        }
    }
    for a in &prevs {
        out.push_str(&format!("  ulong s{a} = 0ul;\n"));
        tail.push_str(&format!("      s{a} = v{a};\n"));
    }
    // The level's scans, each from both carries: `x` from 0, `y` from 1.
    let mut twice = String::new();
    let mut record = String::new();
    let mut finish = String::new();
    for &(k, at) in &own {
        out.push_str(&format!("  uint x{at} = 0u, y{at} = 1u, tw{at} = 2u;\n"));
        set.push_str(&format!(" x{at} = 0u; y{at} = 1u;"));
        twice.push_str(&match native.insts[at] {
            BlockInst::Toggle(a) => format!("      {{ const uint n = uint(popcount(v{a})) & 1u; x{at} ^= n; y{at} ^= n; }}\n"),
            BlockInst::Escaped(a) => format!("      {{ uint o = 0u, r = 0u; escaped(v{a}, ulong(x{at}), o); escaped(v{a}, ulong(y{at}), r); x{at} = o; y{at} = r; }}\n"),
            BlockInst::Fill(a, e) => format!("      {{ uint o = 0u, r = 0u; fill_word(v{a}, v{e}, ulong(x{at}), o); fill_word(v{a}, v{e}, ulong(y{at}), r); x{at} = o; y{at} = r; }}\n"),
            _ => unreachable!("a scan"),
        });
        record.push_str(&format!(" tw{at} = x{at} | (y{at} << 1u);"));
        finish.push_str(&format!("  tab[{}u] = x{at} | (y{at} << 1u); tab[{}u] = tw{at};\n", 2 * k, 2 * k + 1));
    }
    let mut body = emit(p, native, LaneWidth::W64, &cone, "", "b * 64u", carried_serial);
    body.push_str(&tail);
    body.push_str(&twice);
    let body: String = body.lines().map(|line| format!("    {line}\n")).collect();
    out.push_str(&format!(
        "  const uint len = f.len;\n{}\
         \x20 for (uint b = start; b < a_next; b++) {{\n{NEXT_BLOCK}\
         \x20   if (b == a) {{{set} }}\n\
         \x20   if (b + warm == a_next && b >= a) {{{record} }}\n\
         \x20   {{\n{body}    }}\n\
         \x20   pl = c3;\n\
         \x20 }}\n{finish}}}\n\n",
        load_ahead("start")
    ));
    out
}

/// The loads before a serial loop from block `b0`: its first block's chunks, and the chunk
/// before them (zero before the file).
fn load_ahead(b0: &str) -> String {
    format!(
        "  uint4 n0 = load16(bytes, f, {b0} * 4u), n1 = load16(bytes, f, {b0} * 4u + 1u), n2 = load16(bytes, f, {b0} * 4u + 2u), n3 = load16(bytes, f, {b0} * 4u + 3u);\n\
         \x20 uint4 pl = {b0} > 0u ? load16(bytes, f, {b0} * 4u - 1u) : uint4(0u);\n"
    )
}

/// A serial loop's first statements: the block's chunks, the byte before them, and the next
/// block's loads.
const NEXT_BLOCK: &str = "    const uint4 c0 = n0, c1 = n1, c2 = n2, c3 = n3;\n\
    const uint before = pl.w >> 24u;\n\
    n0 = load16(bytes, f, b * 4u + 4u); n1 = load16(bytes, f, b * 4u + 5u); n2 = load16(bytes, f, b * 4u + 6u); n3 = load16(bytes, f, b * 4u + 7u);\n";

/// A carried instruction in serial form: the thread's state, `s` a `prev` operand's last word and
/// `c` a scan's carry.
fn carried_serial(at: usize, inst: &BlockInst, out: &mut String) {
    match *inst {
        BlockInst::Prev(a, k) => out.push_str(&format!("  const ulong v{at} = (v{a} << {k}u) | (s{a} >> {}u);\n", 64 - u32::from(k))),
        BlockInst::Toggle(a) => out.push_str(&format!("  const ulong v{at} = prefix_xor(v{a}) ^ (0ul - ulong(cy{at}));\n")),
        BlockInst::Escaped(a) => out.push_str(&format!("  uint o{at} = 0u;\n  const ulong v{at} = escaped(v{a}, ulong(cy{at}), o{at});\n")),
        BlockInst::Fill(a, e) => out.push_str(&format!("  uint o{at} = 0u;\n  const ulong v{at} = fill_word(v{a}, v{e}, ulong(cy{at}), o{at});\n")),
        _ => unreachable!("a carried instruction"),
    }
}

/// The serial kernels for `programs` (each in its planes form): the segments' runs, the check, and
/// the positions and totals from the finished words.
/// `generate_serial`'s sources by what they are made from: each program, the pass's stores and
/// slots, what runs have learned of each, and the switches the generator reads. A batch's
/// source is made once, not at every run: made in a debug build, a pass of `.rr`'s took 26 s.
static SOURCES: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<u64, (std::sync::Arc<String>, usize, usize)>>> = std::sync::LazyLock::new(Default::default);

/// A source's kernels by its key (`serial_source`) and name: found without hashing the source
/// again, as `pipeline_named` does (twelve kernels a pass, its source near a megabyte).
pub(crate) static KERNELS: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<(u64, &'static str), metal::ComputePipelineState>>> = std::sync::LazyLock::new(Default::default);

fn kernel(key: u64, source: &str, name: &'static str) -> Result<metal::ComputePipelineState, Error> {
    if let Some(hit) = KERNELS.lock().ok().and_then(|m| m.get(&(key, name)).cloned()) {
        return Ok(hit);
    }
    let made = pipeline_named(source, name)?;
    if let Ok(mut m) = KERNELS.lock() {
        m.insert((key, name), made.clone());
    }
    Ok(made)
}

fn serial_source(programs: &[&BlockNative], io: Option<(&[u16], u16)>) -> (u64, std::sync::Arc<String>, usize, usize) {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for p in programs {
        format!("{p:?}").hash(&mut h);
        LEARNED.lock().ok().and_then(|m| m.get(&fingerprint(p)).cloned()).hash(&mut h);
    }
    io.hash(&mut h);
    for k in ["RERAIL_SERIAL_EXACT", "RERAIL_SERIAL_LANES", "RERAIL_SERIAL_CONE"] {
        std::env::var(k).ok().hash(&mut h);
    }
    let key = h.finish();
    if let Some((source, sw, counted)) = SOURCES.lock().ok().and_then(|m| m.get(&key).cloned()) {
        return (key, source, sw, counted);
    }
    let (source, sw, counted) = generate_serial(programs, io);
    let made = (std::sync::Arc::new(source), sw, counted);
    if let Ok(mut m) = SOURCES.lock() {
        m.insert(key, made.clone());
    }
    (key, made.0, made.1, made.2)
}

fn generate_serial(programs: &[&BlockNative], io: Option<(&[u16], u16)>) -> (String, usize, usize) {
    let mut out = format!("{CARRY}\n{}\n{}{EQ16}\n{TRANSPOSE8}\n", batch_helpers(), table_helpers(3));
    // A program in passes (`native_ir::Passes`): its slots a block, and the pass's stores.
    let (stores, slots) = io.unwrap_or((&[], 0));
    for p in 0..programs.len() {
        out.push_str(&format!("constant uint SLOTS{p} = {slots}u;\n"));
    }
    let states: Vec<State> = programs.iter().map(|n| State::of(n)).collect();
    let sw = states.iter().map(|s| s.words).max().unwrap_or(1);
    let counted = programs.iter().map(|n| counted_outputs(n).len()).max().unwrap_or(0).max(1);
    let levels: Vec<Option<Levels>> = programs.iter().map(|n| cones_on().then(|| Levels::of(n))).collect();
    let most = levels.iter().flatten().map(|l| l.scans.len()).max().unwrap_or(0).max(1);
    let gstride = group_stride(programs);
    // Each level's pass and prefix, by program.
    let (mut cone_cases, mut prefix_cases, mut lcone_cases, mut group_cases) = (String::new(), String::new(), String::new(), String::new());
    for (p, native) in programs.iter().enumerate() {
        out.push_str(&serial_function(p, native, &states[p], levels[p].as_ref(), stores));
        out.push_str(&lanes_function(p, native, &states[p], levels[p].as_ref(), stores));
        let Some(lv) = &levels[p] else { continue };
        let (mut cone, mut prefix, mut lcone, mut group) = (String::new(), String::new(), String::new(), String::new());
        for (gi, g) in lv.groups.iter().enumerate() {
            out.push_str(&group_prefix(p, gi, g, lv, gstride));
        }
        for l in 1..=lv.depth {
            if !lv.groups_at(l).is_empty() {
                out.push_str(&lanes_group_function(p, native, lv, l));
                group.push_str(&format!("        case {l}u: lgroup_{p}_{l}(bytes, out, f, a, a_next, q.z, l, c, cin[s], cw[s], gtabs + s * {gstride}u); break;\n"));
            }
            out.push_str(&cone_function(p, native, lv, l));
            out.push_str(&lanes_cone_function(p, native, lv, l));
            lcone.push_str(&format!("        case {l}u: lcone_{p}_{l}(bytes, out, f, a, a_next, q.z, l, cin[s], cw[s], tables + s * {}u); break;\n", 2 * most));
            cone.push_str(&format!("        case {l}u: cone_{p}_{l}(bytes, out, f, a, a_next, q.z, cin[t], cw[t], tables + t * {}u); break;\n", 2 * most));
            let mut scans: String = lv.at(l).iter().map(|&(i, _)| format!(" scan_prefix(f, {i}u, l, sg, id, tg, tables, cin, cw);")).collect();
            scans.extend(lv.groups_at(l).into_iter().map(|(gi, _)| format!(" gprefix_{p}_{gi}(f, l, sg, id, tg, gtabs, cin, cw);")));
            prefix.push_str(&format!("        case {l}u:{scans} break;\n"));
        }
        cone_cases.push_str(&format!("    case {p}u: switch (q.y) {{\n{cone}        default: break;\n      }} break;\n"));
        lcone_cases.push_str(&format!("    case {p}u: switch (q.y) {{\n{lcone}        default: break;\n      }} break;\n"));
        group_cases.push_str(&format!("    case {p}u: switch (q.y) {{\n{group}        default: break;\n      }} break;\n"));
        prefix_cases.push_str(&format!("    case {p}u: switch (q.y) {{\n{prefix}        default: break;\n      }} break;\n"));
    }
    out.push_str(&format!(
        "// Scan `i`'s carries over a file's segments, from their tables: `cin` at each segment's `a`, and\n\
         // `cw` at its warm-up's first block (the table to there of the segment before, applied to\n\
         // that one's carry in). Each thread of the file's threadgroup composes a run of its segments,\n\
         // the runs' tables are scanned across the threadgroup (each SIMD group's total through `tg`),\n\
         // and each thread walks its run again with its carry.\n\
         static inline void scan_prefix(FileRow f, uint i, uint l, uint sg, uint id, threadgroup uint* tg, device const uint* tables, device ulong* cin, device ulong* cw) {{\n\
         \x20 const uint n = f.halo_base, run = (n + {pt}u - 1u) / {pt}u;\n\
         \x20 const uint lo = min(id * run, n), hi = min(lo + run, n);\n\
         \x20 uint mine = TID1;\n\
         \x20 for (uint s = lo; s < hi; s++) mine = tcompose1(mine, tables[(f.group0 + s) * {r}u + 2u * i]);\n\
         \x20 uint total = 0u;\n\
         \x20 const uint e = texclusive1(mine, l, total);\n\
         \x20 threadgroup_barrier(mem_flags::mem_threadgroup);\n\
         \x20 if (l == 0u) tg[sg] = total;\n\
         \x20 threadgroup_barrier(mem_flags::mem_threadgroup);\n\
         \x20 uint before = TID1;\n\
         \x20 for (uint k = 0u; k < sg; k++) before = tcompose1(before, tg[k]);\n\
         \x20 uint c = tapply1(tcompose1(before, e), 0u);\n\
         \x20 // Each segment gives the next its carry at the next's warm-up start.\n\
         \x20 for (uint s = lo; s < hi; s++) {{\n\
         \x20   const uint j = f.group0 + s;\n\
         \x20   cin[j] |= ulong(c) << i;\n\
         \x20   if (s + 1u < n) cw[j + 1u] |= ulong(tapply1(tables[j * {r}u + 2u * i + 1u], c)) << i;\n\
         \x20   c = tapply1(tables[j * {r}u + 2u * i], c);\n\
         \x20 }}\n\
         }}\n\n",
        r = 2 * most,
        pt = PREFIX_THREADS,
    ));
    let lane_cases: String = (0..programs.len()).map(|p| format!("      case {p}u: lanes_{p}(bytes, f, seg.y, seg.z, false, q.z, l, out, records + s * {}u, st, cin[s], cw[s], counts + s * {counted}u); break;\n", 2 * sw)).collect();
    let again_cases: String = (0..programs.len()).map(|p| format!("      case {p}u: lanes_{p}(bytes, f, seg.y, seg.z, true, q.z, l, out, records + s * {}u, st, cin[s], cw[s], counts + s * {counted}u); break;\n", 2 * sw)).collect();
    let cases: String = (0..programs.len()).map(|p| format!("      case {p}u: serial_{p}(bytes, f, seg.y, seg.z, again, q.z, out, records + s * {}u, st, cin[s], cw[s]); break;\n", 2 * sw)).collect();
    // The positions and totals of each program, from its output words.
    let mut count_cases = String::new();
    let mut write_cases = String::new();
    for (p, native) in programs.iter().enumerate() {
        let mut count = String::new();
        let mut write = String::new();
        for (e, &k) in native.extracts.iter().enumerate() {
            count.push_str(&format!(
                "      {{ uint n = 0u; for (uint j = seg.y; j < seg.z; j++) n += uint(popcount(out[f.out_base + {k}u * f.groups + j])); counts[t * {counted}u + {e}u] = n; }}\n"
            ));
            write.push_str(&format!(
                "      {{ uint at = bases[t * {counted}u + {e}u]; for (uint j = seg.y; j < seg.z; j++) {{ ulong w = out[f.out_base + {k}u * f.groups + j];\n\
                 \x20       for (uint i = 0u; i < 64u && w != 0ul; i++) {{ positions[f.pos_base + {e}u * max(len, 1u) + at] = j * 64u + uint(ctz(w)); at++; w &= w - 1ul; }} }} }}\n"
            ));
        }
        // All the totals into one flag, the count after the extracts' (`counted_outputs`).
        if !native.totals.is_empty() {
            let words: String = native.totals.iter().map(|&k| format!(" | out[f.out_base + {k}u * f.groups + j]")).collect();
            count.push_str(&format!(
                "      {{ ulong any = 0ul; for (uint j = seg.y; j < seg.z; j++) any = any{words}; counts[t * {counted}u + {}u] = uint(any != 0ul); }}\n",
                native.extracts.len()
            ));
        }
        count_cases.push_str(&format!("    case {p}u: {{\n{count}      break; }}\n"));
        write_cases.push_str(&format!("    case {p}u: {{\n{write}      break; }}\n"));
    }
    let mut lwrite_cases = String::new();
    for (p, native) in programs.iter().enumerate() {
        let write: String = native
            .extracts
            .iter()
            .enumerate()
            .map(|(e, &k)| {
                format!(
                    "      {{ ulong w = j < seg.z ? out[f.out_base + {k}u * f.groups + j] : 0ul; const uint n = uint(popcount(w));\n\
                     \x20       uint at = bases[s * {counted}u + {e}u] + simd_prefix_exclusive_sum(n);\n\
                     \x20       for (uint i = 0u; i < 64u && w != 0ul; i++) {{ positions[f.pos_base + {e}u * max(len, 1u) + at] = j * 64u + uint(ctz(w)); at++; w &= w - 1ul; }} }}\n"
                )
            })
            .collect();
        lwrite_cases.push_str(&format!("    case {p}u: {{\n{write}      break; }}\n"));
    }
    let extracts: String = (0..programs.len()).map(|p| format!("    case {p}u: e_count = {}u; x_count = {}u; break;\n", counted_outputs(programs[p]).len(), programs[p].extracts.len())).collect();
    out.push_str(&format!(
        "#define SLAB device const uint4* bytes = f.slab == 0u ? s0 : f.slab == 1u ? s1 : f.slab == 2u ? s2 : s3\n\
         // A segment's run: `q.x` segments, `q.y` 1 for runs again (segment `list[t]`, its start\n\
         // `given`), `q.z` the warm-up blocks, `q.w` the check that listed them. A segment: (file,\n\
         // first block, last block, index in the file). A run again goes on into the next segment\n\
         // while its end differs from the end it had, unless the next is listed too: a stale state\n\
         // (a fill no byte reads until it resets) can carry through many segments.\n\
         kernel void serial_run(device const uint4* s0 [[buffer(0)]], device ulong* out [[buffer(1)]], device const FileRow* files [[buffer(2)]],\n\
         \x20   device const uint4* segs [[buffer(3)]], device ulong* records [[buffer(4)]], device const uint* list [[buffer(5)]],\n\
         \x20   device const ulong* given [[buffer(6)]], constant uint4& q [[buffer(7)]], device const ulong* cin [[buffer(10)]],\n\
         \x20   device const ulong* cw [[buffer(14)]],\n\
         \x20   device const uint* marks [[buffer(12)]], device atomic_uint* hopped [[buffer(13)]], device const uint4* s1 [[buffer(16)]], device const uint4* s2 [[buffer(17)]],\n\
         \x20   device const uint4* s3 [[buffer(18)]], uint t [[thread_position_in_grid]]) {{\n\
         \x20 if (t >= q.x) return;\n\
         \x20 const bool again = q.y != 0u;\n\
         \x20 uint s = again ? list[t] : t;\n\
         \x20 ulong st[{sw}];\n\
         \x20 for (uint i = 0u; i < {sw}u; i++) st[i] = again ? given[t * {sw}u + i] : 0ul;\n\
         \x20 const uint hops = again ? files[segs[s].x].halo_base : 1u;\n\
         \x20 uint hop = 0u;\n\
         \x20 for (; hop < hops; hop++) {{\n\
         \x20   const uint4 seg = segs[s];\n\
         \x20   const FileRow f = files[seg.x];\n\
         \x20   SLAB;\n\
         \x20   ulong old[{sw}];\n\
         \x20   for (uint i = 0u; i < {sw}u; i++) old[i] = records[s * {r}u + {sw}u + i];\n\
         \x20   switch (f.program) {{\n{cases}      default: break;\n    }}\n\
         \x20   bool same = true;\n\
         \x20   for (uint i = 0u; i < {sw}u; i++) same = same && records[s * {r}u + {sw}u + i] == old[i];\n\
         \x20   if (!again || same || seg.w + 1u >= f.halo_base || marks[s + 1u] == q.w) break;\n\
         \x20   for (uint i = 0u; i < {sw}u; i++) st[i] = records[s * {r}u + {sw}u + i];\n\
         \x20   s++;\n\
         \x20 }}\n\
         \x20 if (again) {{ atomic_fetch_add_explicit(&hopped[0], hop + 1u, memory_order_relaxed); atomic_fetch_max_explicit(&hopped[1], hop + 1u, memory_order_relaxed); }}\n\
         }}\n\n\
         // The first run in lanes form: one SIMD group a segment (`lanes_function`).\n\
         kernel void lanes_run(device const uint4* s0 [[buffer(0)]], device ulong* out [[buffer(1)]], device const FileRow* files [[buffer(2)]],\n\
         \x20   device const uint4* segs [[buffer(3)]], device ulong* records [[buffer(4)]], constant uint4& q [[buffer(7)]],\n\
         \x20   device const ulong* cin [[buffer(10)]], device const ulong* cw [[buffer(14)]], device uint* counts [[buffer(15)]], device const uint4* s1 [[buffer(16)]], device const uint4* s2 [[buffer(17)]],\n\
         \x20   device const uint4* s3 [[buffer(18)]], uint t [[thread_position_in_grid]], uint l [[thread_index_in_simdgroup]]) {{\n\
         \x20 const uint s = t / {GROUP}u;\n\
         \x20 if (s >= q.x) return;\n\
         \x20 const uint4 seg = segs[s];\n\
         \x20 const FileRow f = files[seg.x];\n\
         \x20 SLAB;\n\
         \x20 const ulong st[{sw}] = {{}};\n\
         \x20 switch (f.program) {{\n{lane_cases}    default: break;\n  }}\n\
         }}\n\n\
         // The runs again in lanes form: `serial_run`'s, one SIMD group a listed segment.\n\
         kernel void lanes_again(device const uint4* s0 [[buffer(0)]], device ulong* out [[buffer(1)]], device const FileRow* files [[buffer(2)]],\n\
         \x20   device const uint4* segs [[buffer(3)]], device ulong* records [[buffer(4)]], device const uint* list [[buffer(5)]],\n\
         \x20   device const ulong* given [[buffer(6)]], constant uint4& q [[buffer(7)]], device const ulong* cin [[buffer(10)]],\n\
         \x20   device const ulong* cw [[buffer(14)]], device const uint* marks [[buffer(12)]], device atomic_uint* hopped [[buffer(13)]], device uint* counts [[buffer(15)]], device const uint4* s1 [[buffer(16)]], device const uint4* s2 [[buffer(17)]],\n\
         \x20   device const uint4* s3 [[buffer(18)]], uint t [[thread_position_in_grid]], uint l [[thread_index_in_simdgroup]]) {{\n\
         \x20 const uint i = t / {GROUP}u;\n\
         \x20 if (i >= q.x) return;\n\
         \x20 uint s = list[i];\n\
         \x20 ulong st[{sw}];\n\
         \x20 for (uint k = 0u; k < {sw}u; k++) st[k] = given[i * {sw}u + k];\n\
         \x20 const uint hops = files[segs[s].x].halo_base;\n\
         \x20 uint hop = 0u;\n\
         \x20 for (; hop < hops; hop++) {{\n\
         \x20   const uint4 seg = segs[s];\n\
         \x20   const FileRow f = files[seg.x];\n\
         \x20   SLAB;\n\
         \x20   ulong old[{sw}];\n\
         \x20   for (uint k = 0u; k < {sw}u; k++) old[k] = records[s * {r}u + {sw}u + k];\n\
         \x20   simdgroup_barrier(mem_flags::mem_device);\n\
         \x20   switch (f.program) {{\n{again_cases}      default: break;\n    }}\n\
         \x20   simdgroup_barrier(mem_flags::mem_device);\n\
         \x20   bool same = true;\n\
         \x20   for (uint k = 0u; k < {sw}u; k++) same = same && records[s * {r}u + {sw}u + k] == old[k];\n\
         \x20   if (same || seg.w + 1u >= f.halo_base || marks[s + 1u] == q.w) break;\n\
         \x20   for (uint k = 0u; k < {sw}u; k++) st[k] = records[s * {r}u + {sw}u + k];\n\
         \x20   s++;\n\
         \x20 }}\n\
         \x20 if (l == 0u) {{ atomic_fetch_add_explicit(&hopped[0], hop + 1u, memory_order_relaxed); atomic_fetch_max_explicit(&hopped[1], hop + 1u, memory_order_relaxed); }}\n\
         }}\n\n\
         // Level `q.y`'s pass: each segment's tables of the level's scans (`cone_function`).\n\
         kernel void serial_cone(device const uint4* s0 [[buffer(0)]], device const ulong* out [[buffer(1)]], device const FileRow* files [[buffer(2)]], device const uint4* segs [[buffer(3)]],\n\
         \x20   constant uint4& q [[buffer(7)]], device const ulong* cin [[buffer(10)]], device uint* tables [[buffer(11)]],\n\
         \x20   device const ulong* cw [[buffer(14)]], device const uint4* s1 [[buffer(16)]], device const uint4* s2 [[buffer(17)]],\n\
         \x20   device const uint4* s3 [[buffer(18)]], uint t [[thread_position_in_grid]]) {{\n\
         \x20 if (t >= q.x) return;\n\
         \x20 const uint4 seg = segs[t];\n\
         \x20 const FileRow f = files[seg.x];\n\
         \x20 SLAB;\n\
         \x20 const uint a = seg.y - min(seg.y, q.z), a_next = seg.z - min(seg.z, q.z);\n\
         \x20 switch (f.program) {{\n{cone_cases}    default: break;\n  }}\n\
         }}\n\n\
         // Level `q.y`'s pass in lanes form (`lanes_cone_function`): one SIMD group a segment.\n\
         kernel void lanes_cone(device const uint4* s0 [[buffer(0)]], device const ulong* out [[buffer(1)]], device const FileRow* files [[buffer(2)]], device const uint4* segs [[buffer(3)]],\n\
         \x20   constant uint4& q [[buffer(7)]], device const ulong* cin [[buffer(10)]], device uint* tables [[buffer(11)]],\n\
         \x20   device const ulong* cw [[buffer(14)]], device const uint4* s1 [[buffer(16)]], device const uint4* s2 [[buffer(17)]],\n\
         \x20   device const uint4* s3 [[buffer(18)]], uint t [[thread_position_in_grid]], uint l [[thread_index_in_simdgroup]]) {{\n\
         \x20 const uint s = t / {GROUP}u;\n\
         \x20 if (s >= q.x) return;\n\
         \x20 const uint4 seg = segs[s];\n\
         \x20 const FileRow f = files[seg.x];\n\
         \x20 SLAB;\n\
         \x20 const uint a = seg.y - min(seg.y, q.z), a_next = seg.z - min(seg.z, q.z);\n\
         \x20 switch (f.program) {{\n{lcone_cases}    default: break;\n  }}\n\
         }}\n\n\
         // Level `q.y`'s fixed points (`lanes_group_function`): one SIMD group a segment and a state.\n\
         kernel void lanes_group(device const uint4* s0 [[buffer(0)]], device const ulong* out [[buffer(1)]], device const FileRow* files [[buffer(2)]], device const uint4* segs [[buffer(3)]],\n\
         \x20   constant uint4& q [[buffer(7)]], device const ulong* cin [[buffer(10)]], device const ulong* cw [[buffer(14)]], device const uint4* s1 [[buffer(16)]],\n\
         \x20   device const uint4* s2 [[buffer(17)]], device const uint4* s3 [[buffer(18)]], device uint* gtabs [[buffer(19)]],\n\
         \x20   uint t [[thread_position_in_grid]], uint l [[thread_index_in_simdgroup]]) {{\n\
         \x20 const uint s = t / {GROUP}u / {GROUP_STATES}u, c = t / {GROUP}u % {GROUP_STATES}u;\n\
         \x20 if (s >= q.x) return;\n\
         \x20 const uint4 seg = segs[s];\n\
         \x20 const FileRow f = files[seg.x];\n\
         \x20 SLAB;\n\
         \x20 const uint a = seg.y - min(seg.y, q.z), a_next = seg.z - min(seg.z, q.z);\n\
         \x20 switch (f.program) {{\n{group_cases}    default: break;\n  }}\n\
         }}\n\n\
         // Level `q.y`'s prefix: one SIMD group a file.\n\
         kernel void serial_cone_prefix(device const FileRow* files [[buffer(2)]], constant uint4& q [[buffer(7)]], device ulong* cin [[buffer(10)]],\n\
         \x20   device const uint* tables [[buffer(11)]], device ulong* cw [[buffer(14)]], device const uint* gtabs [[buffer(19)]], uint l [[thread_index_in_simdgroup]],\n\
         \x20   uint sg [[simdgroup_index_in_threadgroup]], uint id [[thread_index_in_threadgroup]], uint g [[threadgroup_position_in_grid]]) {{\n\
         \x20 threadgroup uint tg[{tgs}];\n\
         \x20 const FileRow f = files[g];\n\
         \x20 switch (f.program) {{\n{prefix_cases}    default: break;\n  }}\n\
         }}\n\n\
         // The check: a segment whose start is not its predecessor's end is listed, with that end.\n\
         kernel void serial_check(device const uint4* segs [[buffer(3)]], device const ulong* records [[buffer(4)]], device uint* list [[buffer(5)]],\n\
         \x20   device ulong* given [[buffer(6)]], constant uint4& q [[buffer(7)]], device atomic_uint* listed [[buffer(9)]],\n\
         \x20   device uint* marks [[buffer(12)]], uint t [[thread_position_in_grid]]) {{\n\
         \x20 if (t >= q.x || segs[t].w == 0u) return;\n\
         \x20 device const ulong* mine = records + t * {r}u;\n\
         \x20 device const ulong* end = records + (t - 1u) * {r}u + {sw}u;\n\
         \x20 bool same = true;\n\
         \x20 for (uint i = 0u; i < {sw}u; i++) same = same && mine[i] == end[i];\n\
         \x20 if (same) return;\n\
         \x20 const uint at = atomic_fetch_add_explicit(listed, 1u, memory_order_relaxed);\n\
         \x20 list[at] = t;\n\
         \x20 marks[t] = q.w;\n\
         \x20 for (uint i = 0u; i < {sw}u; i++) given[at * {sw}u + i] = end[i];\n\
         }}\n\n\
         // Each segment's count of each extract's bits, and the totals' flags.\n\
         kernel void serial_count(device const ulong* out [[buffer(1)]], device const FileRow* files [[buffer(2)]], device const uint4* segs [[buffer(3)]],\n\
         \x20   device uint* counts [[buffer(4)]], device uint* tot [[buffer(8)]], constant uint4& q [[buffer(7)]], uint t [[thread_position_in_grid]]) {{\n\
         \x20 if (t >= q.x) return;\n\
         \x20 const uint4 seg = segs[t];\n\
         \x20 const FileRow f = files[seg.x];\n\
         \x20 switch (f.program) {{\n{count_cases}    default: break;\n  }}\n}}\n\n\
         // A file's segments' bases: one SIMD group a file, 32 segments at a time.\n\
         kernel void serial_prefix(device const FileRow* files [[buffer(2)]], device const uint* counts [[buffer(4)]], device uint* bases [[buffer(5)]],\n\
         \x20   device uint* tot [[buffer(8)]], uint l [[thread_index_in_simdgroup]], uint g [[threadgroup_position_in_grid]]) {{\n\
         \x20 const FileRow f = files[g];\n\
         \x20 uint e_count = 0u, x_count = 0u;\n\
         \x20 switch (f.program) {{\n{extracts}    default: break;\n  }}\n\
         \x20 for (uint e = 0u; e < e_count; e++) {{\n\
         \x20   uint run = 0u;\n\
         \x20   for (uint s = 0u; s < f.halo_base; s += 32u) {{\n\
         \x20     const uint j = f.group0 + s + l;\n\
         \x20     const uint c = s + l < f.halo_base ? counts[j * {counted}u + e] : 0u;\n\
         \x20     if (s + l < f.halo_base) bases[j * {counted}u + e] = run + simd_prefix_exclusive_sum(c);\n\
         \x20     run += simd_sum(c);\n\
         \x20   }}\n\
         \x20   // An extract's count; a total's flag.\n\
         \x20   if (l == 0u) tot[f.tot_base + e] = e < x_count ? run : uint(run != 0u);\n\
         \x20 }}\n\
         }}\n\n\
         // Each segment's positions in lanes form: one SIMD group a segment, a word a lane.\n\
         kernel void lanes_write(device const ulong* out [[buffer(1)]], device const FileRow* files [[buffer(2)]], device const uint4* segs [[buffer(3)]],\n\
         \x20   device const uint* bases [[buffer(5)]], device uint* positions [[buffer(6)]], constant uint4& q [[buffer(7)]], uint t [[thread_position_in_grid]],\n\
         \x20   uint l [[thread_index_in_simdgroup]]) {{\n\
         \x20 const uint s = t / {GROUP}u;\n\
         \x20 if (s >= q.x) return;\n\
         \x20 const uint4 seg = segs[s];\n\
         \x20 const FileRow f = files[seg.x];\n\
         \x20 const uint len = f.len, j = seg.y + l;\n\
         \x20 switch (f.program) {{\n{lwrite_cases}    default: break;\n  }}\n}}\n\n\
         // Each segment's positions, from its base.\n\
         kernel void serial_write(device const ulong* out [[buffer(1)]], device const FileRow* files [[buffer(2)]], device const uint4* segs [[buffer(3)]],\n\
         \x20   device const uint* bases [[buffer(5)]], device uint* positions [[buffer(6)]], constant uint4& q [[buffer(7)]], uint t [[thread_position_in_grid]]) {{\n\
         \x20 if (t >= q.x) return;\n\
         \x20 const uint4 seg = segs[t];\n\
         \x20 const FileRow f = files[seg.x];\n\
         \x20 const uint len = f.len;\n\
         \x20 switch (f.program) {{\n{write_cases}    default: break;\n  }}\n}}\n",
        r = 2 * sw,
        tgs = PREFIX_THREADS / GROUP,
    ));
    (out, sw, counted)
}

/// Whether the scans' carries are made exact first, level by level (`Levels`);
/// `RERAIL_SERIAL_CONE=0` speculates them like the `prev`s, to see what that costs.
fn cones_on() -> bool {
    std::env::var("RERAIL_SERIAL_CONE").map_or(true, |v| v != "0")
}

/// Each segment's words of fixed-point tables (`lanes_group_function`): two of `GROUP_STATES`
/// a leveled group, for the program with the most.
fn group_stride(programs: &[&BlockNative]) -> usize {
    programs.iter().map(|n| if cones_on() { Levels::of(n).groups.len() } else { 0 }).max().unwrap_or(0).max(1) * 2 * GROUP_STATES
}

/// The levels with a leveled group, of any of `programs`.
fn group_levels(programs: &[&BlockNative]) -> std::collections::BTreeSet<usize> {
    if !cones_on() {
        return Default::default();
    }
    programs.iter().flat_map(|n| Levels::of(n).groups.into_iter().map(|g| g.level)).collect()
}

/// The outputs counted per segment: the extracts', then one for all the totals (one flag).
fn counted_outputs(native: &BlockNative) -> Vec<usize> {
    rerail_compiler::native_cpu_block::counted(native)
}

/// Whether the exact scans are left to the runs to learn: no `RERAIL_SERIAL_EXACT`.
fn exact_unset() -> bool {
    std::env::var("RERAIL_SERIAL_EXACT").is_err()
}

/// After a first pass with wrong starts: each program's scans wrong in more than one segment in
/// `PROMOTE` (their carry bit in the start differs from the end before) join its `LEARNED`.
fn learn(forms: &[BlockNative], rows: &[FileRow], segs: &[[u32; 4]], records: &[u64], sw: usize) {
    for (p, native) in forms.iter().enumerate() {
        let state = State::of(native);
        let mut wrong = vec![0usize; state.parts.len()];
        let mut seen = 0usize;
        for (t, seg) in segs.iter().enumerate() {
            if seg[3] == 0 || rows[seg[0] as usize].program as usize != p {
                continue;
            }
            seen += 1;
            let (mine, end) = (&records[t * 2 * sw..], &records[(t - 1) * 2 * sw + sw..]);
            for (i, &(_, _, w, first, scan)) in state.parts.iter().enumerate() {
                if scan && (mine[w] >> first) & 1 != (end[w] >> first) & 1 {
                    wrong[i] += 1;
                }
            }
        }
        let promoted: Vec<usize> = state.parts.iter().zip(&wrong).filter(|&(_, &n)| n * PROMOTE > seen).map(|(&(at, ..), _)| at).collect();
        if !promoted.is_empty() {
            if let Ok(mut m) = LEARNED.lock() {
                m.entry(fingerprint(native)).or_default().extend(promoted);
            }
        }
    }
}

/// Whether the runs are in lanes form (`lanes_function`); `RERAIL_SERIAL_LANES=0` runs each
/// segment in one thread instead.
fn lanes_on() -> bool {
    std::env::var("RERAIL_SERIAL_LANES").map_or(true, |v| v != "0")
}

/// Threads of each file's prefix of a level (`scan_prefix`): eight SIMD groups.
const PREFIX_THREADS: usize = 256;

/// Threads a threadgroup of the segment kernels.
const THREADS: usize = 64;

/// `run_serial`, split while the device refuses it for capacity, as `run_split` does: halves in
/// the files' order, their counts and seconds summed.
pub fn run_serial_split(programs: &[&BlockNative], files: &[BatchFile<'_>], segments: Segments) -> Result<SerialRun, Error> {
    match run_serial(programs, files, segments) {
        Err(Error::BackendCapacity) if files.len() > 1 => {
            let (a, b) = files.split_at(files.len() / 2);
            let mut first = run_serial_split(programs, a, segments)?;
            let second = run_serial_split(programs, b, segments)?;
            first.run.files.extend(second.run.files);
            first.run.launches += second.run.launches;
            first.run.seconds += second.run.seconds;
            first.segments += second.segments;
            first.reruns += second.reruns;
            first.rounds = first.rounds.max(second.rounds);
            first.first_seconds += second.first_seconds;
            first.rerun_seconds += second.rerun_seconds;
            first.sink_seconds += second.sink_seconds;
            Ok(first)
        }
        other => other,
    }
}

/// Run `files` in segments (each under `programs[file.program]`, in its planes form): one first
/// pass, the checks and runs again until every start agrees, then the positions and totals.
pub fn run_serial(programs: &[&BlockNative], files: &[BatchFile<'_>], segments: Segments) -> Result<SerialRun, Error> {
    Ok(run_kept(programs, files, segments, true)?.0)
}

/// `run_serial` with tier 1's buffers left where the device wrote them (`Resident`): no copy of
/// them back. The run's `files` are empty.
pub fn run_resident(programs: &[&BlockNative], files: &[BatchFile<'_>], segments: Segments) -> Result<(SerialRun, Resident), Error> {
    let (run, kept) = run_kept(programs, files, segments, false)?;
    Ok((run, kept.ok_or(Error::BackendFailure)?))
}

fn run_kept(programs: &[&BlockNative], files: &[BatchFile<'_>], segments: Segments, read: bool) -> Result<(SerialRun, Option<Resident>), Error> {
    if programs.iter().any(|p| p.insts.iter().any(|i| matches!(i, BlockInst::BackFill(..) | BlockInst::Load(_)))) {
        return run_passes(programs, files, segments, read);
    }
    let (run, kept) = run_serial_io(programs, files, segments, None, read)?;
    Ok((run, kept.map(|k| k.with(None))))
}

/// A file of a `Resident` run: its output words (output `k`'s from `out_base + k * blocks`),
/// its streams' positions (stream `e`'s from `pos_base + e * capacity`), and its counts (from
/// `tot_base`: a stream's count, then the totals).
#[derive(Clone, Copy, Debug)]
pub struct ResidentFile {
    pub out_base: usize,
    pub blocks: usize,
    pub pos_base: usize,
    pub capacity: usize,
    pub tot_base: usize,
}

/// Tier 1's buffers after a run, in the device's shared memory, where tier 2 reads them: the
/// output words, the positions (two readable slots past the last), the counts (32-bit, one
/// readable past the last), each file's place in them, and the files' bytes as the device read
/// them (`ResidentSource`: 64 readable bytes past each).
pub struct Resident {
    pub out: crate::pool::Block,
    pub tot: crate::pool::Block,
    pub positions: crate::pool::Block,
    pub files: Vec<ResidentFile>,
    pub sources: Vec<std::sync::Arc<crate::slabs::ResidentSource>>,
}

/// `Resident` before its output buffer is known (a pass of a program in passes writes to the
/// buffer `run_passes` owns).
struct Kept {
    out: Option<crate::pool::Block>,
    tot: crate::pool::Block,
    positions: crate::pool::Block,
    files: Vec<ResidentFile>,
    sources: Vec<std::sync::Arc<crate::slabs::ResidentSource>>,
}

impl Kept {
    fn with(self, out: Option<crate::pool::Block>) -> Resident {
        let out = self.out.or(out).expect("a run's output buffer");
        Resident { out, tot: self.tot, positions: self.positions, files: self.files, sources: self.sources }
    }
}

/// A pass of a program in passes, as `run_serial_io` runs it: the output buffer every pass
/// shares (each file's slot words first, then the outputs of the pass that runs, from
/// `offset`), each file's slot base, the slots a block, and the pass's stores (none: the last).
struct Io<'a> {
    out: &'a crate::pool::Block,
    offset: usize,
    slot_base: &'a [u32],
    slots: u16,
    stores: &'a [u16],
}

/// A file's blocks in the forward carry: its bytes and the program's delay, one at least.
fn blocks_of(len: usize, delay: u8) -> usize {
    (len + usize::from(delay)).div_ceil(64).max(1)
}

/// A program with `back_fill`s (`native_ir::Passes`), as Cranelift runs it: each forward pass in
/// the forward carry over the batch, a pass before the last storing its outputs to the slots
/// as made; each backward pass's fills from the files' last blocks to their first
/// (`back_kernels`). The slots stay in the device's buffer from pass to pass. One program a
/// batch.
/// A program's passes (`BlockNative::passes`), made once a program: in a debug build `.rr`'s
/// took most of a second, at every run.
fn passes_of(program: &BlockNative) -> std::sync::Arc<rerail_compiler::block_native::Passes> {
    use std::hash::{Hash, Hasher};
    static PASSES: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<u64, std::sync::Arc<rerail_compiler::block_native::Passes>>>> = std::sync::LazyLock::new(Default::default);
    let mut h = std::collections::hash_map::DefaultHasher::new();
    format!("{program:?}").hash(&mut h);
    let key = h.finish();
    if let Some(hit) = PASSES.lock().ok().and_then(|m| m.get(&key).cloned()) {
        return hit;
    }
    let made = std::sync::Arc::new(program.passes());
    if let Ok(mut m) = PASSES.lock() {
        m.insert(key, made.clone());
    }
    made
}

fn run_passes(programs: &[&BlockNative], files: &[BatchFile<'_>], segments: Segments, read: bool) -> Result<(SerialRun, Option<Resident>), Error> {
    let [program] = programs else { return Err(Error::UnsupportedBackend) };
    METAL_DEVICE.as_ref().ok_or(Error::UnsupportedBackend)?;
    let passes = passes_of(program);
    let passes: &rerail_compiler::block_native::Passes = &passes;
    let fit = |n: usize| u32::try_from(n).map_err(|_| Error::BackendCapacity);
    let blocks: Vec<usize> = files.iter().map(|f| blocks_of(f.bytes.len(), program.delay)).collect();
    let total_blocks: usize = blocks.iter().sum();
    let mut slot_base = Vec::with_capacity(files.len());
    let mut at = 0usize;
    for &b in &blocks {
        slot_base.push(fit(at)?);
        at += b * usize::from(passes.slots);
    }
    let offset = at;
    let widest = passes
        .passes
        .iter()
        .map(|p| match p {
            rerail_compiler::block_native::Pass::Forward { body, .. } => body.outputs.len(),
            rerail_compiler::block_native::Pass::Back(_) => 0,
        })
        .max()
        .unwrap_or(0);
    let out = crate::pool::zeroed::<u64>((offset + widest * total_blocks).max(1))?;
    fit(offset + widest * total_blocks)?;
    // The segments, as `run_serial_io` cuts them: (file, first block, end, index in the file).
    let mut rows = Vec::with_capacity(files.len());
    let mut segs: Vec<[u32; 4]> = Vec::new();
    for (i, &b) in blocks.iter().enumerate() {
        let count = b.div_ceil(segments.blocks);
        rows.push(FileRow { group0: fit(segs.len())?, groups: fit(b)?, halo_base: fit(count)?, slot_base: slot_base[i], len: fit(files[i].bytes.len())?, ..FileRow::default() });
        for s in 0..count {
            segs.push([fit(i)?, fit(s * segments.blocks)?, fit(((s + 1) * segments.blocks).min(b))?, fit(s)?]);
        }
    }
    let (mut seconds, mut launches) = (0.0, 0usize);
    let (mut reruns, mut rounds, mut first_seconds, mut rerun_seconds, mut sink_seconds) = (0, 0, 0.0, 0.0, 0.0);
    let last = passes.passes.len() - 1;
    for (i, pass) in passes.passes.iter().enumerate() {
        match pass {
            rerail_compiler::block_native::Pass::Forward { body, stores } => {
                let io = Io { out: &out, offset, slot_base: &slot_base, slots: passes.slots, stores };
                let (run, kept) = run_serial_io(&[body], files, segments, Some(&io), read && i == last)?;
                seconds += run.run.seconds;
                launches += run.run.launches;
                reruns += run.reruns;
                rounds += run.rounds;
                first_seconds += run.first_seconds;
                rerun_seconds += run.rerun_seconds;
                sink_seconds += run.sink_seconds;
                if i == last {
                    let run = SerialRun { run: BatchRun { files: run.run.files, launches, seconds }, segments: segs.len(), reruns, rounds, first_seconds, rerun_seconds, sink_seconds };
                    return Ok((run, if read { None } else { kept.map(|k| k.with(Some(out))) }));
                }
            }
            rerail_compiler::block_native::Pass::Back(fills) => {
                let source = back_kernels(fills, passes.slots);
                let (tables_k, prefix_k, apply_k) = (pipeline_named(&source, "back_tables")?, pipeline_named(&source, "back_prefix")?, pipeline_named(&source, "back_apply")?);
                let table = crate::pool::upload(&rows)?;
                let seg_table = crate::pool::upload(&segs)?;
                let tables = crate::pool::zeroed::<u32>((segs.len() * fills.len()).max(1))?;
                let cins = crate::pool::zeroed::<u32>((segs.len() * fills.len()).max(1))?;
                fn bufs<'b>(list: &[(u64, &'b crate::pool::Block)]) -> Vec<(u64, &'b metal::BufferRef, u64)> {
                    list.iter().map(|&(at, b)| (at, b.buffer(), 0)).collect()
                }
                let n = |x: usize| vec![(7u64, bytes_of(&[fit(x).unwrap_or(0), 0u32, 0, 0]))];
                let groups = |x: usize| x.div_ceil(THREADS).max(1);
                let kernels = [
                    Pass { pipeline: &tables_k, buffers: bufs(&[(1, &out), (2, &table), (3, &seg_table), (11, &tables)]), args: n(segs.len()), groups: groups(segs.len()), threads: THREADS },
                    Pass { pipeline: &prefix_k, buffers: bufs(&[(2, &table), (11, &tables), (12, &cins)]), args: n(files.len()), groups: groups(files.len()), threads: THREADS },
                    Pass { pipeline: &apply_k, buffers: bufs(&[(1, &out), (2, &table), (3, &seg_table), (12, &cins)]), args: n(segs.len()), groups: groups(segs.len()), threads: THREADS },
                ];
                seconds += encode_passes(&kernels)?;
                launches += 1;
            }
        }
    }
    unreachable!("a program's last pass is a forward one")
}

/// A backward pass's kernels (`native_ir::Pass::Back`): each fill `(s, e, own)` of slots, from
/// a file's last block to its first, the carry zero at its last block (`block::carry::back_fill`).
/// As a scan backwards over the segments: each segment's table (its carry out at its first
/// block for each carry in at its last), each file's carries into its segments from its last
/// segment back, then each segment's words from its carry in. A thread walks its segment's
/// blocks, or its file's segments: steps of a block or a segment, never of a byte.
fn back_kernels(fills: &[(u16, u16, u16)], slots: u16) -> String {
    let n = fills.len();
    let mut tables = String::new();
    let mut apply = String::new();
    let mut prefix = String::new();
    for (j, &(s, e, own)) in fills.iter().enumerate() {
        let words = format!("const ulong s{j} = out[f.slot_base + b * {slots}u + {s}u], e{j} = out[f.slot_base + b * {slots}u + {e}u];");
        tables.push_str(&format!("    {{ {words} uint o = 0u; back_fill_word(s{j}, e{j}, c{j}_0, o); c{j}_0 = o; back_fill_word(s{j}, e{j}, c{j}_1, o); c{j}_1 = o; }}\n"));
        apply.push_str(&format!("    {{ {words} uint o = 0u; out[f.slot_base + b * {slots}u + {own}u] = back_fill_word(s{j}, e{j}, c{j}, o); c{j} = o; }}\n"));
        prefix.push_str(&format!("    {{ cins[g * {n}u + {j}u] = c{j}; c{j} = (tables[g * {n}u + {j}u] >> c{j}) & 1u; }}\n"));
    }
    let init = |f: &dyn Fn(usize) -> String| (0..n).map(f).collect::<String>();
    format!(
        "{CARRY}\n{FILE_ROW}\n\
         // `back_fill` on a word: `fill` on the bits reversed; the carry out is bit 0.\n\
         static inline ulong back_fill_word(ulong s, ulong e, uint cin, thread uint& cout) {{\n\
         \x20 uint o = 0u;\n\
         \x20 const ulong b = reverse_bits(fill_word(reverse_bits(s), reverse_bits(e), ulong(cin), o));\n\
         \x20 cout = uint(b & 1ul);\n\
         \x20 return b;\n}}\n\
         kernel void back_tables(device const ulong* out [[buffer(1)]], device const FileRow* files [[buffer(2)]], device const uint4* segs [[buffer(3)]],\n\
         \x20                       constant uint4& q [[buffer(7)]], device uint* tables [[buffer(11)]], uint t [[thread_position_in_grid]]) {{\n\
         \x20 if (t >= q.x) return;\n\
         \x20 const uint4 seg = segs[t];\n\
         \x20 const FileRow f = files[seg.x];\n\
         {}\
         \x20 for (uint b = seg.z; b > seg.y;) {{\n    b--;\n{tables}  }}\n\
         {}}}\n\
         kernel void back_prefix(device const FileRow* files [[buffer(2)]], constant uint4& q [[buffer(7)]], device const uint* tables [[buffer(11)]],\n\
         \x20                       device uint* cins [[buffer(12)]], uint t [[thread_position_in_grid]]) {{\n\
         \x20 if (t >= q.x) return;\n\
         \x20 const FileRow f = files[t];\n\
         {}\
         \x20 for (uint s = f.halo_base; s > 0u;) {{\n    s--;\n    const uint g = f.group0 + s;\n{prefix}  }}\n}}\n\
         kernel void back_apply(device ulong* out [[buffer(1)]], device const FileRow* files [[buffer(2)]], device const uint4* segs [[buffer(3)]],\n\
         \x20                      constant uint4& q [[buffer(7)]], device const uint* cins [[buffer(12)]], uint t [[thread_position_in_grid]]) {{\n\
         \x20 if (t >= q.x) return;\n\
         \x20 const uint4 seg = segs[t];\n\
         \x20 const FileRow f = files[seg.x];\n\
         {}\
         \x20 for (uint b = seg.z; b > seg.y;) {{\n    b--;\n{apply}  }}\n}}\n",
        init(&|j| format!("  uint c{j}_0 = 0u, c{j}_1 = 1u;\n")),
        init(&|j| format!("  tables[t * {n}u + {j}u] = c{j}_0 | (c{j}_1 << 1u);\n")),
        init(&|j| format!("  uint c{j} = 0u;\n")),
        init(&|j| format!("  uint c{j} = cins[t * {n}u + {j}u];\n")),
    )
}

/// `run_serial` over programs without passes, or one forward pass of a program in passes (`io`).
fn run_serial_io(programs: &[&BlockNative], files: &[BatchFile<'_>], segments: Segments, io: Option<&Io<'_>>, read: bool) -> Result<(SerialRun, Option<Kept>), Error> {
    METAL_DEVICE.as_ref().ok_or(Error::UnsupportedBackend)?;
    let forms: Vec<BlockNative> = programs.iter().map(|p| p.planes_form()).collect();
    let refs: Vec<&BlockNative> = forms.iter().collect();
    let (source_key, source, sw, counted) = serial_source(&refs, io.map(|io| (io.stores, io.slots)));
    let source: &str = &source;
    let fit = |n: usize| u32::try_from(n).map_err(|_| Error::BackendCapacity);
    let bytes: Vec<&[u8]> = files.iter().map(|f| f.bytes).collect();
    let sources = crate::slabs::admit_sources(&bytes)?;
    let (slabs, slab_of) = crate::slabs::source_slabs(&sources).ok_or(Error::BackendCapacity)?;
    if files.is_empty() {
        return Ok((SerialRun { run: BatchRun { files: Vec::new(), launches: 0, seconds: 0.0 }, segments: 0, reruns: 0, rounds: 0, first_seconds: 0.0, rerun_seconds: 0.0, sink_seconds: 0.0 }, None));
    }
    // The file table: `groups` is the file's words an output (its blocks), `group0` its first
    // segment, `halo_base` its segments.
    let (mut rows, mut segs) = (Vec::with_capacity(files.len()), Vec::<[u32; 4]>::new());
    let (mut out_base, mut pos_base, mut tot_base) = (io.map_or(0, |io| io.offset), 0usize, 0usize);
    for (i, file) in files.iter().enumerate() {
        let n = forms.get(file.program).ok_or(Error::BackendFailure)?;
        let len = file.bytes.len();
        let blocks = blocks_of(len, n.delay);
        let count = blocks.div_ceil(segments.blocks);
        rows.push(FileRow {
            src16: fit(sources[i].offset / 16)?,
            len: fit(len)?,
            program: fit(file.program)?,
            group0: fit(segs.len())?,
            groups: fit(blocks)?,
            out_base: fit(out_base)?,
            state_base: 0,
            halo_base: fit(count)?,
            ext_base: 0,
            pos_base: fit(pos_base)?,
            slab: slab_of[i],
            tot_base: fit(tot_base)?,
            slot_base: io.map_or(0, |io| io.slot_base[i]),
        });
        for s in 0..count {
            segs.push([fit(i)?, fit(s * segments.blocks)?, fit(((s + 1) * segments.blocks).min(blocks))?, fit(s)?]);
        }
        out_base += n.outputs.len() * blocks;
        pos_base += n.extracts.len() * len.max(1);
        tot_base += rerail_compiler::native_cpu_block::counted(n).len();
    }
    let total = segs.len();
    let table = crate::pool::upload(&rows)?;
    let seg_table = crate::pool::upload(&segs)?;
    let owned = match io {
        Some(_) => None,
        None => Some(crate::pool::zeroed::<u64>(out_base.max(1))?),
    };
    let out = match (io, &owned) {
        (Some(io), _) => io.out,
        (None, Some(o)) => o,
        (None, None) => unreachable!("an output buffer, given or owned"),
    };
    let records = crate::pool::zeroed::<u64>(total * 2 * sw)?;
    let list = crate::pool::zeroed::<u32>(total)?;
    let given = crate::pool::zeroed::<u64>(total * sw)?;
    let listed = crate::pool::zeroed::<u32>(1)?;
    let counts = crate::pool::zeroed::<u32>(total * counted)?;
    let bases = crate::pool::zeroed::<u32>(total * counted)?;
    // Slack for tier 2's reads in place: an 8-byte load at the last count, two slots past the
    // last position.
    let tot = crate::pool::zeroed::<u32>(tot_base + 2)?;
    let positions = crate::pool::zeroed::<u32>(pos_base + 2)?;
    let most = forms.iter().map(|n| Levels::of(n).scans.len()).max().unwrap_or(0).max(1);
    let depth = if cones_on() { forms.iter().map(|n| Levels::of(n).depth).max().unwrap_or(0) } else { 0 };
    let tables = crate::pool::zeroed::<u32>(total * 2 * most)?;
    let cins = crate::pool::zeroed::<u64>(total)?;
    let cws = crate::pool::zeroed::<u64>(total)?;
    let gtabs = crate::pool::zeroed::<u32>(total * group_stride(&refs))?;
    let with_groups = group_levels(&refs);
    let marks = crate::pool::zeroed::<u32>(total)?;
    // Segments run again in all, and the most one thread ran.
    let hopped = crate::pool::zeroed::<u32>(2)?;
    assert!(2 * segments.warm < segments.blocks, "a segment is longer than its two warm-ups");
    let cone = kernel(source_key, source, if lanes_on() { "lanes_cone" } else { "serial_cone" })?;
    let cone_prefix = kernel(source_key, source, "serial_cone_prefix")?;
    let group_pass = kernel(source_key, source, "lanes_group")?;
    let lanes = lanes_on();
    if lanes {
        // The lanes: the warm-up, the segment, and the block after it for the delayed outputs.
        assert!(segments.warm >= 1 && segments.blocks + 2 * segments.warm < GROUP, "a segment and its two warm-ups fit a SIMD group");
    }
    let lanes_run = kernel(source_key, source, "lanes_run").inspect_err(|e| eprintln!("lanes: pipeline: {e:?}"))?;
    let lanes_again = kernel(source_key, source, "lanes_again")?;
    let run = kernel(source_key, source, "serial_run").inspect_err(|e| eprintln!("serial: pipeline: {e:?}"))?;
    let check = kernel(source_key, source, "serial_check")?;
    let count = kernel(source_key, source, "serial_count")?;
    let prefix = kernel(source_key, source, "serial_prefix")?;
    let write = kernel(source_key, source, "serial_write")?;
    let lanes_write = kernel(source_key, source, "lanes_write")?;
    let mut common: Vec<(u64, &metal::BufferRef, u64)> = vec![(1, out.buffer(), 0), (2, table.buffer(), 0), (3, seg_table.buffer(), 0)];
    for (k, &at) in SLAB_BUFFERS.iter().enumerate() {
        common.push((at, &slabs.get(k).unwrap_or(&slabs[0]).buffer.buffer, 0));
    }
    let with = |extra: &[(u64, &'static str)]| -> Vec<(u64, &metal::BufferRef, u64)> {
        let mut b = common.clone();
        for &(at, name) in extra {
            let buffer = match name {
                "records" => records.buffer(),
                "list" => list.buffer(),
                "given" => given.buffer(),
                "listed" => listed.buffer(),
                "counts" => counts.buffer(),
                "bases" => bases.buffer(),
                "tot" => tot.buffer(),
                "tables" => tables.buffer(),
                "cins" => cins.buffer(),
                "cws" => cws.buffer(),
                "marks" => marks.buffer(),
                "gtabs" => gtabs.buffer(),
                "hopped" => hopped.buffer(),
                _ => positions.buffer(),
            };
            b.push((at, buffer, 0));
        }
        b
    };
    let groups = |n: usize| n.div_ceil(THREADS).max(1);
    let args = |a: u32, b: u32| vec![(7u64, bytes_of(&[a, b, fit(segments.warm).unwrap_or(0), 0u32]))];
    let at_round = |a: u32, b: u32, round: usize| vec![(7u64, bytes_of(&[a, b, fit(segments.warm).unwrap_or(0), round as u32]))];
    let run_buffers = with(&[(4, "records"), (5, "list"), (6, "given"), (10, "cins"), (12, "marks"), (13, "hopped"), (14, "cws")]);
    let check_buffers = with(&[(4, "records"), (5, "list"), (6, "given"), (9, "listed"), (12, "marks")]);
    let mut first = Vec::new();
    for l in 1..=depth {
        let cone_groups = if lanes_on() { groups(total * GROUP) } else { groups(total) };
        first.push(Pass { pipeline: &cone, buffers: with(&[(10, "cins"), (11, "tables"), (14, "cws")]), args: args(fit(total)?, fit(l)?), groups: cone_groups, threads: THREADS });
        if with_groups.contains(&l) {
            first.push(Pass { pipeline: &group_pass, buffers: with(&[(10, "cins"), (14, "cws"), (19, "gtabs")]), args: args(fit(total)?, fit(l)?), groups: groups(total * GROUP_STATES * GROUP), threads: THREADS });
        }
        first.push(Pass { pipeline: &cone_prefix, buffers: with(&[(10, "cins"), (11, "tables"), (14, "cws"), (19, "gtabs")]), args: args(0, fit(l)?), groups: files.len(), threads: PREFIX_THREADS });
    }
    if lanes {
        first.push(Pass { pipeline: &lanes_run, buffers: with(&[(4, "records"), (10, "cins"), (14, "cws"), (15, "counts")]), args: args(fit(total)?, 0), groups: groups(total * GROUP), threads: THREADS });
    } else {
        first.push(Pass { pipeline: &run, buffers: run_buffers.clone(), args: args(fit(total)?, 0), groups: groups(total), threads: THREADS });
    }
    first.push(Pass { pipeline: &check, buffers: check_buffers.clone(), args: at_round(fit(total)?, 0, 1), groups: groups(total), threads: THREADS });
    if std::env::var("RERAIL_SERIAL_TIMES").is_ok() {
        // Each pass alone, for its time; then the run proper.
        let times: Vec<String> = first.iter().map(|p| encode_passes(std::slice::from_ref(p)).map(|s| format!("{:.3}", s * 1e3)).unwrap_or_default()).collect();
        eprintln!("serial: passes ms {}", times.join(" "));
        unsafe { std::ptr::write_bytes(listed.host().cast::<u32>(), 0, 1) };
        unsafe { std::ptr::write_bytes(cins.host().cast::<u64>(), 0, total) };
        unsafe { std::ptr::write_bytes(cws.host().cast::<u64>(), 0, total) };
    }
    let first_seconds = encode_passes(&first)?;
    if listed.read::<u32>(1)[0] > 0 && exact_unset() {
        learn(&forms, &rows, &segs, &records.read::<u64>(total * 2 * sw), sw);
    }
    if std::env::var("RERAIL_SERIAL_DEBUG").is_ok() {
        let lv = Levels::of(&forms[0]);
        let sizes: Vec<String> = (1..=lv.depth).map(|l| format!("L{l}: {:?}", lv.at(l).iter().map(|&(_, at)| format!("{:?}", forms[0].insts[at])).collect::<Vec<_>>())).collect();
        eprintln!("serial: {} scans, depth {}; {}", lv.scans.len(), lv.depth, sizes.join("; "));
    }
    if std::env::var("RERAIL_SERIAL_DEBUG").is_ok() {
        // Which parts of the state the first pass's starts got wrong, over the segments.
        let rec = records.read::<u64>(total * 2 * sw);
        let state = State::of(&forms[0]);
        let mut wrong = vec![0usize; state.parts.len()];
        let mut any = 0usize;
        for t in 1..total {
            if segs[t][3] == 0 {
                continue;
            }
            let (mine, end) = (&rec[t * 2 * sw..], &rec[(t - 1) * 2 * sw + sw..]);
            let mut differs = false;
            for (i, &(_, bits, w, first, _)) in state.parts.iter().enumerate() {
                let m = if bits == 64 { u64::MAX } else { (1u64 << bits) - 1 };
                if (mine[w] >> first) & m != (end[w] >> first) & m {
                    wrong[i] += 1;
                    differs = true;
                }
            }
            any += usize::from(differs);
            if differs && any <= 6 {
                let row = &rows[segs[t][0] as usize];
                let at = segs[t][1] as usize * 64;
                let bytes = files[segs[t][0] as usize].bytes;
                let lo = at.saturating_sub(48);
                eprintln!("serial: segment {t} (file {}, byte {at}): {:?} | {:?}", segs[t][0], String::from_utf8_lossy(&bytes[lo..at.min(bytes.len())]), String::from_utf8_lossy(&bytes[at.min(bytes.len())..(at + 16).min(bytes.len())]));
                let _ = row;
            }
        }
        let parts: Vec<String> = state.parts.iter().zip(&wrong).filter(|(_, n)| **n > 0).map(|(&(at, bits, _, _, scan), n)| format!("%{at}{}/{bits}={:?}:{n}", if scan { "c" } else { "s" }, forms[0].insts[at])).collect();
        eprintln!("serial: {any} of {total} starts wrong; {}", parts.join(" "));
        if let Ok(at) = std::env::var("RERAIL_SERIAL_DEBUG").map(|v| v.parse::<usize>().unwrap_or(0)) {
            if at > 0 {
                let mut inside = vec![false; forms[0].insts.len()];
                inside[at] = true;
                for x in (0..=at).rev() {
                    if inside[x] {
                        for o in forms[0].insts[x].operands() {
                            inside[o] = true;
                        }
                    }
                }
                for (x, i) in forms[0].insts.iter().enumerate().filter(|(x, _)| inside[*x]) {
                    eprintln!("  %{x} = {i:?}");
                }
            }
        }
    }
    let (mut rerun_seconds, mut reruns, mut rounds, mut launches) = (0.0, 0usize, 1usize, 1usize);
    loop {
        let n = listed.read::<u32>(1)[0] as usize;
        if n == 0 {
            break;
        }
        if rounds > total {
            return Err(Error::BackendFailure);
        }
        reruns += n;
        rounds += 1;
        if std::env::var("RERAIL_SERIAL_DEBUG").is_ok() && rounds < 8 {
            let l = list.read::<u32>(n.min(8));
            eprintln!("serial: round {rounds}: {n} listed, first {l:?}; hops so far {:?}", hopped.read::<u32>(2));
        }
        // SAFETY: one word, written between launches, when no kernel runs.
        unsafe { std::ptr::write_bytes(listed.host().cast::<u32>(), 0, 1) };
        // The list and the starts it gives are read by the run before the check writes them.
        let again = [
            if lanes {
                Pass { pipeline: &lanes_again, buffers: [run_buffers.clone(), with(&[(15, "counts")])].concat(), args: at_round(fit(n)?, 1, rounds - 1), groups: groups(n * GROUP), threads: THREADS }
            } else {
                Pass { pipeline: &run, buffers: run_buffers.clone(), args: at_round(fit(n)?, 1, rounds - 1), groups: groups(n), threads: THREADS }
            },
            Pass { pipeline: &check, buffers: check_buffers.clone(), args: at_round(fit(total)?, 0, rounds), groups: groups(total), threads: THREADS },
        ];
        rerun_seconds += encode_passes(&again)?;
        launches += 1;
    }
    // In lanes form the runs counted their segments: the positions are a prefix and a write.
    let mut sinks = Vec::new();
    if !lanes {
        sinks.push(Pass { pipeline: &count, buffers: with(&[(4, "counts"), (8, "tot")]), args: args(fit(total)?, 0), groups: groups(total), threads: THREADS });
    }
    sinks.push(Pass { pipeline: &prefix, buffers: with(&[(4, "counts"), (5, "bases"), (8, "tot")]), args: args(0, 0), groups: files.len(), threads: GROUP });
    if lanes {
        sinks.push(Pass { pipeline: &lanes_write, buffers: with(&[(5, "bases"), (6, "positions")]), args: args(fit(total)?, 0), groups: groups(total * GROUP), threads: THREADS });
    } else {
        sinks.push(Pass { pipeline: &write, buffers: with(&[(5, "bases"), (6, "positions")]), args: args(fit(total)?, 0), groups: groups(total), threads: THREADS });
    }
    let sink_seconds = encode_passes(&sinks)?;
    launches += 1;
    let mut results = Vec::with_capacity(files.len());
    let times = (first_seconds, rerun_seconds, sink_seconds);
    let serial = |files: Vec<BlockRun>| SerialRun { run: BatchRun { files, launches, seconds: times.0 + times.1 + times.2 }, segments: total, reruns, rounds, first_seconds, rerun_seconds, sink_seconds };
    if !read {
        let placed = rows
            .iter()
            .map(|r| ResidentFile { out_base: r.out_base as usize, blocks: r.groups as usize, pos_base: r.pos_base as usize, capacity: (r.len as usize).max(1), tot_base: r.tot_base as usize })
            .collect();
        return Ok((serial(Vec::new()), Some(Kept { out: owned, tot, positions, files: placed, sources })));
    }
    let words = out.read::<u64>(out_base.max(1));
    let tot = tot.read::<u32>(tot_base.max(1));
    let positions = positions.read::<u32>(pos_base.max(1));
    for row in &rows {
        let n = &forms[row.program as usize];
        let len = row.len as usize;
        let blocks = row.groups as usize;
        let outputs: Vec<Vec<u64>> = (0..n.outputs.len())
            .map(|k| {
                let at = row.out_base as usize + k * blocks;
                words[at..at + len.div_ceil(64)].to_vec()
            })
            .collect();
        let capacity = len.max(1);
        let count = |c: usize| tot[row.tot_base as usize + c];
        let positions = (0..n.extracts.len())
            .map(|e| {
                let from = row.pos_base as usize + e * capacity;
                positions[from..from + (count(e) as usize).min(capacity)].to_vec()
            })
            .collect();
        let totals = if n.totals.is_empty() { Vec::new() } else { vec![u64::from(count(n.extracts.len()) != 0)] };
        results.push(BlockRun { outputs, positions, totals });
    }
    Ok((serial(results), None))
}
