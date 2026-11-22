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
//! rewrites them all with plain stores. The body of a block is the same UKL the lanes kernel
//! uses (`emit`), at 64 bits; a carried instruction reads the thread's state.
use rerail_blocks::vector::Vector;
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
    /// Segments listed to run again but not run: their file refused by a total in a segment
    /// before them, whose start was final (`serial_check`). The file goes to the fallback.
    pub dropped: usize,
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

    /// The UKL that writes the state's words to `to[0..]`.
    fn pack(&self, native: &BlockNative, to: &str) -> String {
        let mut words = vec![Vec::new(); self.words];
        let _ = native;
        for &(at, bits, w, first, scan) in &self.parts {
            let value = if scan { format!("U64(cy{at})") } else { format!("(s{at} >> {}u)", 64 - bits) };
            words[w].push(format!("({value} << {first}u)"));
        }
        words.iter().enumerate().map(|(w, parts)| format!("{to}[{w}] = {};", if parts.is_empty() { "0ul".to_owned() } else { parts.join(" | ") })).collect::<Vec<_>>().join(" ")
    }

    /// The UKL that sets the state from the words at `from[0..]`.
    fn unpack(&self, native: &BlockNative, from: &str) -> String {
        self.parts
            .iter()
            .map(|&(at, bits, w, first, scan)| {
                let _ = native;
                let field = format!("(({from}[{w}] >> {first}u) & {:#x}ul)", if bits == 64 { u64::MAX } else { (1u64 << bits) - 1 });
                if scan {
                    format!("cy{at} = U32({field});")
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
        "UKL_INLINE void serial_{p}(Bytes bytes, FileRow f, uint32_t first, uint32_t last, bool again, uint32_t warm,\n\
         \x20                          UKL_GLOBAL uint64_t* out, UKL_GLOBAL uint64_t* rec, UKL_PRIVATE const uint64_t* given, uint64_t cin, uint64_t cw) {{\n"
    );
    for &(at, .., scan) in &state.parts {
        if scan {
            out.push_str(&format!("  uint32_t cy{at} = 0u;\n"));
        } else {
            out.push_str(&format!("  uint64_t s{at} = 0ul;\n"));
        }
    }
    let delayed: Vector<usize> = (0..native.outputs.len()).filter(|&k| native.output_delays[k] > 0).collect();
    for &k in &delayed {
        assert!(native.output_delays[k] < 64, "an output's delay is below a block");
        out.push_str(&format!("  uint64_t q{k} = 0ul;\n"));
    }
    let start = state.unpack(native, "given");
    // The scans' carries are exact (`Levels`): from `cw` at the warm-up's first block, set to
    // `cin` at its second part's first, `a`.
    let carries = |x: &str| match levels {
        Some(levels) => levels.scans.iter().enumerate().filter(|&(i, _)| levels.level[i] > 0).map(|(i, at)| format!(" cy{at} = U32(({x} >> {i}u) & 1ul);")).collect::<String>(),
        None => String::new(),
    };
    out.push_str(&format!(
        "  const uint32_t a = first - ukl_min(first, warm);\n\
         \x20 uint32_t b0 = a - ukl_min(a, warm);\n\
         \x20 if (again) {{ b0 = first; {start} }} else {{{} }}\n",
        carries("(b0 < a ? cw : cin)")
    ));
    out.push_str(&format!(
        "  const uint32_t len = f.len;\n\
         \x20 // A block's bytes are loaded a block ahead: the loads of the next overlap this one's work.\n\
         {load_ahead}\
         \x20 // The block after the segment only finishes its delayed words.\n\
         \x20 for (uint32_t b = b0; b <= last; b++) {{\n\
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
                "      {{ const uint64_t w = stream_cut(v{v}, b, len, {d}u); if (b > first) out[f.out_base + {k}u * f.groups + b - 1u] = (q{k} >> {d}u) | (w << {}u); q{k} = w; }}\n",
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
            BlockInst::Toggle(_) if scan => tail.push_str(&format!("      cy{at} = U32(v{at} >> 63u);\n")),
            BlockInst::Escaped(_) | BlockInst::Fill(..) if scan => tail.push_str(&format!("      cy{at} = o{at};\n")),
            _ => tail.push_str(&format!("      s{at} = v{at};\n")),
        }
    }
    let body = |values: &BlockNative| {
        let all: Vector<usize> = (0..values.insts.len()).collect();
        let mut text = emit(p, values, LaneWidth::W64, &all, Word { base: "", offset: "b * 64u", lanes: false }, carried_serial);
        text.push_str(&tail);
        text.lines().map(|l| format!("    {l}\n")).collect::<String>()
    };
    let (full, ascii) = (full_form(native), ascii_form(native));
    out.push_str(&format!(
        "    if (b > 0u && (b + 1u) * 64u <= len) {{\n\
         \x20     const B4 high = b4_or(b4_or(b4_or(pl, c0), b4_or(c1, c2)), c3);\n\
         \x20     // The thread's own body: its threads run segments of their own, each its own count of\n\
         \x20     // blocks, so no SIMD-group operation is reached by all of them (0440 step C).\n\
         \x20     if ((b4_any(high) & 0x80808080u) == 0u) {{\n{}      }} else {{\n{}      }}\n\
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
        "UKL_INLINE void lanes_{p}(Bytes bytes, FileRow f, uint32_t first, uint32_t last, bool again, uint32_t warm, uint32_t l,\n\
         \x20                          UKL_GLOBAL uint64_t* out, UKL_GLOBAL uint64_t* rec, UKL_PRIVATE const uint64_t* given, uint64_t cin, uint64_t cw, UKL_GLOBAL uint32_t* counts __UKL_PARAMS) {{\n\
         \x20 // `aw` the levels' segment start, where `cin` holds; `a` a warm-up before it, where `cw` holds.\n\
         \x20 const uint32_t aw = first - ukl_min(first, warm);\n\
         \x20 const uint32_t a = again ? first : aw - ukl_min(aw, warm);\n\
         \x20 const uint32_t b = a + l;\n\
         \x20 const uint32_t len = f.len;\n"
    );
    // A run again starts at the segment's first block from `given`: lane 0's halos and the scans'
    // carries; its start is written as given.
    for &(at, .., scan) in &state.parts {
        out.push_str(&if scan { format!("  uint32_t cy{at} = 0u;\n") } else { format!("  uint64_t s{at} = 0ul;\n") });
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
            (format!("(again ? cy{at} : U32((cw >> {i}u) & 1ul))"), Some(("!again && b >= aw".to_owned(), format!("U32((cin >> {i}u) & 1ul)"))))
        }
        _ => (format!("(again ? cy{at} : 0u)"), None),
    };
    let carried = |at: usize, inst: &BlockInst, out: &mut String| match *inst {
        BlockInst::Prev(a, k) => out.push_str(&format!("  const uint64_t v{at} = apply_prev(v{a}, {k}u, halo_before(v{a}, l, s{a} __UKL_ARGS));\n")),
        _ => {
            let (from, reset) = exact(at);
            lane_scan(at, inst, &from, reset.as_ref().map(|(c, v)| (c.as_str(), v.as_str())), out)
        }
    };
    let mut tail = String::from("  const bool mine = b >= first && b < last;\n");
    for (k, &v) in native.outputs.iter().enumerate() {
        match native.output_delays[k] {
            // Output words are `ow`: `o` is a scan's carry out.
            0 => tail.push_str(&format!("  const uint64_t ow{k} = SEL(0ul, stream_cut(v{v}, b, len, 0u), mine);\n")),
            d => tail.push_str(&format!(
                "  const uint64_t ww{k} = stream_cut(v{v}, b, len, {d}u), wn{k} = sg_down64(ww{k}, 1u __UKL_ARGS);\n  const uint64_t ow{k} = SEL(0ul, (ww{k} >> {d}u) | (wn{k} << {}u), mine);\n",
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
    // The totals' one count is every total's words OR-ed (`block_native::counted`).
    for (c, k) in counted_outputs(native).into_iter().enumerate() {
        let w = if c < native.extracts.len() { format!("ow{k}") } else { native.totals.iter().map(|t| format!("ow{t}")).collect::<Vec<_>>().join(" | ") };
        tail.push_str(&format!("  {{ const uint32_t n = ukl_sg_sum(ukl_popcount64({w}) __UKL_ARGS); if (l == 0u) counts[{c}u] = n; }}\n"));
    }
    // The state after the lane, under the serial names (`State::pack`).
    let mut names = String::new();
    for &(at, .., scan) in &state.parts {
        if scan {
            names.push_str(&format!(" const uint32_t cy{at} = co{at};"));
        } else {
            names.push_str(&format!(" const uint64_t s{at} = v{at};"));
        }
    }
    tail.push_str(&format!(
        "  {{{names}\n    if (l + 1u == first - a) {{ {} }}\n    if (l + 1u == last - a) {{ {} }} }}\n",
        state.pack(native, "rec"),
        state.pack(native, &format!("(rec + {}u)", state.words))
    ));
    let all: Vector<usize> = (0..native.insts.len()).collect();
    out.push_str(&lane_bodies(p, native, &all, &carried, &tail));
    out.push_str("}\n\n");
    out
}

/// The bodies of lane block `b` for the instructions `chosen`, then `tail`: the general one
/// at the file's edges, the full one inside, and the ASCII one where the group's blocks and the
/// 16 bytes before each are ASCII. The group takes one body: the choice is `simd_all`.
fn lane_bodies(p: usize, native: &BlockNative, chosen: &[usize], carried: &dyn Fn(usize, &BlockInst, &mut String), tail: &str) -> String {
    let body = |values: &BlockNative| {
        let mut text = emit(p, values, LaneWidth::W64, chosen, Word { base: "b * 4u", offset: "b * 64u", lanes: true }, carried);
        text.push_str(tail);
        text.lines().map(|l| format!("    {l}\n")).collect::<String>()
    };
    let (full, ascii) = (full_form(native), ascii_form(native));
    format!(
        "  const bool inside = b > 0u && (b + 1u) * 64u <= len;\n\
         \x20 if (sg_all(inside __UKL_ARGS)) {{\n\
         \x20   const B4 h = b4_or(b4_or(b4_or(load16(bytes, f, b * 4u - 1u), load16(bytes, f, b * 4u)), b4_or(load16(bytes, f, b * 4u + 1u), load16(bytes, f, b * 4u + 2u))), load16(bytes, f, b * 4u + 3u));\n\
         \x20   if (sg_all((b4_any(h) & 0x80808080u) == 0u __UKL_ARGS)) {{\n{}    }} else {{\n{}    }}\n\
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
    // The carry's statements (`ci{at}`).
    let carry = |at: usize| match reset {
        None => format!("  const uint32_t ci{at} = tapply1(texclusive1(tt{at}, l, &agg{at} __UKL_ARGS), {from});\n"),
        // Both prefixes in every lane, each a statement: a SIMD-group function under a branch is not
        // reached by every lane, nor is one in an operand of `SEL` (a C conditional).
        Some((cond, cin)) => format!(
            "  const uint32_t ca{at} = tapply1(texclusive1(tt{at}, l, &agg{at} __UKL_ARGS), {from});\n  const uint32_t cb{at} = tapply1(texclusive1(SEL(2u, tt{at}, {cond}), l, &agr{at} __UKL_ARGS), {cin});\n  const uint32_t ci{at} = SEL(ca{at}, cb{at}, {cond});\n"
        ),
    };
    let agr = if reset.is_some() { format!("  uint32_t agr{at} = 0u;\n") } else { String::new() };
    out.push_str(&agr);
    match *inst {
        BlockInst::Toggle(a) => out.push_str(&format!(
            "  const uint32_t sm{at} = summary_toggle(v{a});\n  const uint32_t tt{at} = sm{at} | ((1u ^ sm{at}) << 1u);\n  uint32_t agg{at} = 0u;\n{}  const uint64_t v{at} = apply_toggle(v{a}, ci{at});\n  const uint32_t co{at} = U32(v{at} >> 63u);\n",
            carry(at)
        )),
        BlockInst::Escaped(_) | BlockInst::Fill(..) => {
            let call = |c: &str, o: &str| match *inst {
                BlockInst::Escaped(a) => format!("escaped(v{a}, {c}, &{o})"),
                BlockInst::Fill(a, e) => format!("fill_word(v{a}, v{e}, {c}, &{o})"),
                _ => unreachable!(),
            };
            out.push_str(&format!(
                "  uint32_t o{at}_0 = 0u, o{at}_1 = 0u;\n  const uint64_t f{at}_0 = {}, f{at}_1 = {};\n  const uint32_t tt{at} = (o{at}_0 & 1u) | ((o{at}_1 & 1u) << 1u);\n  uint32_t agg{at} = 0u;\n{}  const uint64_t v{at} = SEL(f{at}_0, f{at}_1, ci{at} != 0u);\n  const uint32_t co{at} = ci{at} != 0u ? o{at}_1 : o{at}_0;\n",
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
        "UKL_INLINE void lcone_{p}_{lv}(Bytes bytes, UKL_GLOBAL const uint64_t* out, FileRow f, uint32_t a, uint32_t a_next, uint32_t warm, uint32_t l, uint64_t cin, uint64_t cw, UKL_GLOBAL uint32_t* tab __UKL_PARAMS) {{\n\
         \x20 const uint32_t start = a - ukl_min(a, warm);\n\
         \x20 const uint64_t from = start < a ? cw : cin;\n\
         \x20 const uint32_t b = start + l;\n\
         \x20 const uint32_t len = f.len;\n"
    );
    let mut out = out;
    let h = halos(native, levels, "from", lv, &[]);
    out.push_str(&h.decls);
    let carried = |at: usize, inst: &BlockInst, out: &mut String| match *inst {
        BlockInst::Prev(a, k) => out.push_str(&format!("  const uint64_t v{at} = apply_prev(v{a}, {k}u, halo_before(v{a}, l, {} __UKL_ARGS));\n", h.word(usize::from(a)))),
        _ => {
            let i = levels.bit(at);
            let from = if levels.level[i] > 0 { format!("U32((from >> {i}u) & 1ul)") } else { "0u".to_owned() };
            lane_scan(at, inst, &from, None, out);
        }
    };
    let mut tail = String::from("  const bool span = b >= a && b < a_next, span_w = b >= a && b + warm < a_next;\n");
    for &(k, at) in &own {
        let mut scan = String::new();
        lane_scan(at, &native.insts[at], "0u", None, &mut scan);
        tail.push_str(&scan);
        tail.push_str(&format!(
            "  {{ uint32_t g0 = 0u, g1 = 0u; texclusive1(SEL(2u, tt{at}, span), l, &g0 __UKL_ARGS); texclusive1(SEL(2u, tt{at}, span_w), l, &g1 __UKL_ARGS);\n    if (l == 0u) {{ tab[{}u] = g0; tab[{}u] = g1; }} }}\n",
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
/// a pair of UKL booleans, known and value (`hk`, `hv`), folded where constant. Without it a
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
    /// What every step of the walk reads.
    struct Walk<'a> {
        native: &'a BlockNative,
        levels: &'a Levels,
        from: &'a str,
        limit: usize,
        own: &'a [usize],
    }
    fn go(w: &Walk<'_>, at: usize, memo: &mut Vec<Option<(HBit, HBit)>>, decls: &mut String) -> (HBit, HBit) {
        let &Walk { native, levels, from, limit, own } = w;
        if let Some(x) = &memo[at] {
            return x.clone();
        }
        // A way back to itself (through a carry) is unknown.
        memo[at] = Some((HBit::F, HBit::F));
        let e = |x: u16, memo: &mut Vec<Option<(HBit, HBit)>>, decls: &mut String| go(w, usize::from(x), memo, decls);
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
                Some(&(_, x)) => go(w, x, memo, decls),
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
            let (k, v) = go(&Walk { native, levels, from, limit, own }, a, &mut memo, &mut decls);
            let word = match h_and(&k, &v) {
                HBit::F => "0ul".to_owned(),
                HBit::T => "~0ul".to_owned(),
                HBit::E(x) => format!("(0ul - U64({x}))"),
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
fn cone_of(native: &BlockNative, mut inside: Vec<bool>) -> Vector<usize> {
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
        "UKL_INLINE void lgroup_{p}_{lv}(Bytes bytes, UKL_GLOBAL const uint64_t* out, FileRow f, uint32_t a, uint32_t a_next, uint32_t warm, uint32_t l, uint32_t c, uint64_t cin, uint64_t cw, UKL_GLOBAL uint32_t* tab __UKL_PARAMS) {{\n\
         \x20 const uint32_t start = a - ukl_min(a, warm);\n\
         \x20 const uint64_t from = start < a ? cw : cin;\n\
         \x20 const uint32_t b = start + l;\n\
         \x20 const uint32_t len = f.len;\n"
    );
    let own_ids: Vector<usize> = own.iter().map(|(gi, _)| *gi).collect();
    let h = halos(native, levels, "from", lv, &own_ids);
    out.push_str(&h.decls);
    let carried = |at: usize, inst: &BlockInst, out: &mut String| match *inst {
        BlockInst::Prev(a, k) => out.push_str(&format!("  const uint64_t v{at} = apply_prev(v{a}, {k}u, halo_before(v{a}, l, {} __UKL_ARGS));\n", h.word(usize::from(a)))),
        _ => {
            let from = match own.iter().find_map(|(_, g)| g.scans.iter().position(|&x| x == at)) {
                Some(k) => format!("U32(c == {}u)", k + 1),
                None => {
                    let i = levels.bit(at);
                    if levels.level[i] > 0 { format!("U32((from >> {i}u) & 1ul)") } else { "0u".to_owned() }
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
            "  {{ const uint32_t n = {count}, st = c > {n}u ? {NONE}u : n == 0u ? 0u : n == 1u ? {which} : {NONE}u;\n\
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
        "UKL_INLINE uint64_t gbits_{p}_{gi}(uint32_t c) {{ return {} : 0ul; }}\n\
         UKL_INLINE uint32_t gtable_{p}_{gi}(UKL_GLOBAL const uint32_t* gtabs, uint32_t j, uint32_t which) {{\n\
         \x20 UKL_GLOBAL const uint32_t* t = gtabs + j * {stride}u + {base}u + which * {GROUP_STATES}u;\n\
         \x20 uint32_t x = 0u;\n\
         \x20 for (uint32_t c = 0u; c < {GROUP_STATES}u; c++) x |= (c == {NONE}u ? {NONE}u : (t[c] & 7u)) << (3u * c);\n\
         \x20 return x;\n}}\n\
         UKL_INLINE void gprefix_{p}_{gi}(FileRow f, uint32_t l, uint32_t sg, uint32_t id, UKL_LOCAL uint32_t* tg, UKL_GLOBAL const uint32_t* gtabs, UKL_GLOBAL uint64_t* cin, UKL_GLOBAL uint64_t* cw __UKL_PARAMS) {{\n\
         \x20 const uint32_t n = f.halo_base, run = (n + {pt}u - 1u) / {pt}u;\n\
         \x20 const uint32_t lo = ukl_min(id * run, n), hi = ukl_min(lo + run, n);\n\
         \x20 uint32_t mine = TID3;\n\
         \x20 for (uint32_t s = lo; s < hi; s++) mine = tcompose3(mine, gtable_{p}_{gi}(gtabs, f.group0 + s, 1u));\n\
         \x20 uint32_t total = 0u;\n\
         \x20 const uint32_t e = texclusive3(mine, l, &total __UKL_ARGS);\n\
         \x20 ukl_tg_barrier();\n\
         \x20 if (l == 0u) tg[sg] = total;\n\
         \x20 ukl_tg_barrier();\n\
         \x20 uint32_t before = TID3;\n\
         \x20 for (uint32_t k = 0u; k < sg; k++) before = tcompose3(before, tg[k]);\n\
         \x20 uint32_t c = tapply3(tcompose3(before, e), 0u);\n\
         \x20 for (uint32_t s = lo; s < hi; s++) {{\n\
         \x20   const uint32_t j = f.group0 + s;\n\
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
    scans: Vector<usize>,
    /// Each scan's level, from 1; 0 for a scan left to the warm-ups (`exact_scans`).
    level: Vector<usize>,
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
    scans: Vector<usize>,
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
fn exact_scans(native: &BlockNative) -> Vector<usize> {
    let scans = native.insts.iter().enumerate().filter(|(_, i)| matches!(i, BlockInst::Toggle(_) | BlockInst::Escaped(_) | BlockInst::Fill(..)));
    match std::env::var("RERAIL_SERIAL_EXACT").ok().as_deref() {
        Some("all") => scans.map(|(at, _)| at).collect(),
        Some(v) => v.split(',').filter_map(|x| x.trim().parse::<usize>().ok()).collect(),
        None => {
            let st_ = kept_programs();
            let learned = st_.learned.lock().ok().and_then(|m| m.get(&fingerprint(native)).cloned()).unwrap_or_default();
            let mut exact: Vector<usize> = scans.filter(|(_, i)| matches!(i, BlockInst::Toggle(_))).map(|(at, _)| at).collect();
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
            let scans: Vector<usize> = group.members.iter().copied().filter(|&m| is_scan(&native.insts[m])).collect();
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
        let mut levels = Levels { scans: rerail_blocks::vector::Vector::new(), level: rerail_blocks::vector::Vector::new(), depth: 0, groups: Vec::new() };
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
        "UKL_INLINE void cone_{p}_{l}(Bytes bytes, UKL_GLOBAL const uint64_t* out, FileRow f, uint32_t a, uint32_t a_next, uint32_t warm, uint64_t cin, uint64_t cw, UKL_GLOBAL uint32_t* tab) {{\n\
         \x20 const uint32_t start = a - ukl_min(a, warm);\n\
         \x20 const uint64_t from = start < a ? cw : cin;\n"
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
                    out.push_str(&format!("  uint32_t cy{at} = U32((from >> {i}u) & 1ul);\n"));
                    set.push_str(&format!(" cy{at} = U32((cin >> {i}u) & 1ul);"));
                } else {
                    out.push_str(&format!("  uint32_t cy{at} = 0u;\n"));
                }
                tail.push_str(&match native.insts[at] {
                    BlockInst::Toggle(_) => format!("      cy{at} = U32(v{at} >> 63u);\n"),
                    _ => format!("      cy{at} = o{at};\n"),
                });
            }
            _ => {}
        }
    }
    for a in &prevs {
        out.push_str(&format!("  uint64_t s{a} = 0ul;\n"));
        tail.push_str(&format!("      s{a} = v{a};\n"));
    }
    // The level's scans, each from both carries: `x` from 0, `y` from 1.
    let mut twice = String::new();
    let mut record = String::new();
    let mut finish = String::new();
    for &(k, at) in &own {
        out.push_str(&format!("  uint32_t x{at} = 0u, y{at} = 1u, tw{at} = 2u;\n"));
        set.push_str(&format!(" x{at} = 0u; y{at} = 1u;"));
        twice.push_str(&match native.insts[at] {
            BlockInst::Toggle(a) => format!("      {{ const uint32_t n = U32(ukl_popcount64(v{a})) & 1u; x{at} ^= n; y{at} ^= n; }}\n"),
            BlockInst::Escaped(a) => format!("      {{ uint32_t o = 0u, r = 0u; escaped(v{a}, U64(x{at}), &o); escaped(v{a}, U64(y{at}), &r); x{at} = o; y{at} = r; }}\n"),
            BlockInst::Fill(a, e) => format!("      {{ uint32_t o = 0u, r = 0u; fill_word(v{a}, v{e}, U64(x{at}), &o); fill_word(v{a}, v{e}, U64(y{at}), &r); x{at} = o; y{at} = r; }}\n"),
            _ => unreachable!("a scan"),
        });
        record.push_str(&format!(" tw{at} = x{at} | (y{at} << 1u);"));
        finish.push_str(&format!("  tab[{}u] = x{at} | (y{at} << 1u); tab[{}u] = tw{at};\n", 2 * k, 2 * k + 1));
    }
    let mut body = emit(p, native, LaneWidth::W64, &cone, Word { base: "", offset: "b * 64u", lanes: false }, carried_serial);
    body.push_str(&tail);
    body.push_str(&twice);
    let body: String = body.lines().map(|line| format!("    {line}\n")).collect();
    out.push_str(&format!(
        "  const uint32_t len = f.len;\n{}\
         \x20 for (uint32_t b = start; b < a_next; b++) {{\n{NEXT_BLOCK}\
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
        "  B4 n0 = load16(bytes, f, {b0} * 4u), n1 = load16(bytes, f, {b0} * 4u + 1u), n2 = load16(bytes, f, {b0} * 4u + 2u), n3 = load16(bytes, f, {b0} * 4u + 3u);\n\
         \x20 B4 pl = {b0} > 0u ? load16(bytes, f, {b0} * 4u - 1u) : b4(0u, 0u, 0u, 0u);\n"
    )
}

/// A serial loop's first statements: the block's chunks, the byte before them, and the next
/// block's loads.
const NEXT_BLOCK: &str = "    const B4 c0 = n0, c1 = n1, c2 = n2, c3 = n3;\n\
    const uint32_t before = pl.w >> 24u;\n\
    n0 = load16(bytes, f, b * 4u + 4u); n1 = load16(bytes, f, b * 4u + 5u); n2 = load16(bytes, f, b * 4u + 6u); n3 = load16(bytes, f, b * 4u + 7u);\n";

/// A carried instruction in serial form: the thread's state, `s` a `prev` operand's last word and
/// `c` a scan's carry.
fn carried_serial(at: usize, inst: &BlockInst, out: &mut String) {
    match *inst {
        BlockInst::Prev(a, k) => out.push_str(&format!("  const uint64_t v{at} = (v{a} << {k}u) | (s{a} >> {}u);\n", 64 - u32::from(k))),
        BlockInst::Toggle(a) => out.push_str(&format!("  const uint64_t v{at} = prefix_xor(v{a}) ^ (0ul - U64(cy{at}));\n")),
        BlockInst::Escaped(a) => out.push_str(&format!("  uint32_t o{at} = 0u;\n  const uint64_t v{at} = escaped(v{a}, U64(cy{at}), &o{at});\n")),
        BlockInst::Fill(a, e) => out.push_str(&format!("  uint32_t o{at} = 0u;\n  const uint64_t v{at} = fill_word(v{a}, v{e}, U64(cy{at}), &o{at});\n")),
        _ => unreachable!("a carried instruction"),
    }
}

/// A generated source, with `generate_serial`'s two counts.
type Source = (std::sync::Arc<String>, usize, usize);

/// What the serial programs' runs keep of callers' programs (decision 0442: the caller's, `rerail_context::Shared`):
/// each program's generated sources, what its runs learned, the sources that learn nothing more, its shapes and passes.
#[derive(Default)]
struct Programs {
    learned: std::sync::Mutex<std::collections::HashMap<u64, Vector<usize>>>,
    sources: std::sync::Mutex<std::collections::HashMap<u64, Source>>,
    settled: std::sync::Mutex<std::collections::HashSet<u64>>,
    shapes: std::sync::Mutex<std::collections::HashMap<u64, std::sync::Arc<Shape>>>,
    passes: std::sync::Mutex<std::collections::HashMap<u64, std::sync::Arc<rerail_compiler::block_native::Passes>>>,
}

fn kept_programs() -> std::sync::Arc<Programs> {
    rerail_context::shared::<Programs>()
}

/// A source's kernels by its key (`serial_source`) and name, found without hashing the source again, as
/// `lane::kernel` does (twelve kernels a pass, its source near a megabyte): the device's, on this machine (decision
/// 0442).
#[derive(Default)]
pub(crate) struct Kernels(pub(crate) std::sync::Mutex<std::collections::HashMap<(u64, &'static str), lane::Kernel>>);

pub(crate) fn kernels() -> std::sync::Arc<Kernels> {
    rerail_context::machine::<Kernels>()
}

/// The run's count of sources generated and kernels compiled (a benchmark reads it around its timed runs, which must
/// make none).
#[derive(Default)]
struct Compiles(std::sync::atomic::AtomicUsize);

/// The sources generated and kernels compiled so far in the run under way.
pub fn compiles() -> usize {
    rerail_context::local::<Compiles>().0.load(std::sync::atomic::Ordering::Relaxed)
}

fn kernel(key: u64, source: &str, name: &'static str) -> Result<lane::Kernel, Error> {
    let st_ = kernels();
    if let Some(hit) = st_.0.lock().ok().and_then(|m| m.get(&(key, name)).cloned()) {
        return Ok(hit);
    }
    rerail_context::with_local::<Compiles, _>(|c| c.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
    let made = lane::kernel(source, name, None);
    if let Ok(mut m) = st_.0.lock() {
        m.insert((key, name), made.clone());
    }
    Ok(made)
}

/// The serial kernels' source for `programs` (each in its planes form): the segments' runs, the check, and the
/// positions and totals from the finished words. `generate_serial`'s sources are kept by what they are made from:
/// each program, the pass's stores and slots, what runs have learned of each, and the switches the generator reads.
/// A batch's source is made once, not at every run: made in a debug build, a pass of `.rerail`'s took 26 s.
fn serial_source(programs: &[&BlockNative], io: Option<(&[u16], u16)>) -> (u64, std::sync::Arc<String>, usize, usize) {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for p in programs {
        format!("{p:?}").hash(&mut h);
        kept_programs().learned.lock().ok().and_then(|m| m.get(&fingerprint(p)).cloned()).hash(&mut h);
    }
    io.hash(&mut h);
    for k in ["RERAIL_SERIAL_EXACT", "RERAIL_SERIAL_LANES", "RERAIL_SERIAL_CONE"] {
        std::env::var(k).ok().hash(&mut h);
    }
    let key = h.finish();
    let st_ = kept_programs();
    if let Some((source, sw, counted)) = st_.sources.lock().ok().and_then(|m| m.get(&key).cloned()) {
        return (key, source, sw, counted);
    }
    // `RERAIL_SERIAL_CACHE=dir`: sources kept on disk by key, for runs by hand (a profile, a
    // trace) that would otherwise make them again in each process.
    let disk = std::env::var_os("RERAIL_SERIAL_CACHE").map(|d| std::path::PathBuf::from(d).join(format!("{key:016x}.msl")));
    if let Some(Ok(text)) = disk.as_ref().map(std::fs::read_to_string)
        && let Some((head, source)) = text.split_once('\n')
    {
        let mut n = head.split(' ').filter_map(|x| x.parse::<usize>().ok());
        if let (Some(sw), Some(counted)) = (n.next(), n.next()) {
            let made = (std::sync::Arc::new(source.to_owned()), sw, counted);
            let st_ = kept_programs();
            if let Ok(mut m) = st_.sources.lock() {
                m.insert(key, made.clone());
            }
            return (key, made.0, made.1, made.2);
        }
    }
    rerail_context::with_local::<Compiles, _>(|c| c.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
    let (source, sw, counted) = generate_serial(programs, io);
    if let Some(path) = &disk {
        let _ = std::fs::create_dir_all(path.parent().unwrap_or(path));
        let _ = std::fs::write(path, format!("{sw} {counted}\n{source}"));
    }
    let made = (std::sync::Arc::new(source), sw, counted);
    let st_ = kept_programs();
    if let Ok(mut m) = st_.sources.lock() {
        m.insert(key, made.clone());
    }
    (key, made.0, made.1, made.2)
}

/// The serial kernels for `programs` (each in its planes form): the segments' runs, the check, and
/// the positions and totals from the finished words.
fn generate_serial(programs: &[&BlockNative], io: Option<(&[u16], u16)>) -> (String, usize, usize) {
    let mut out = format!("{}{}", head(LaneWidth::W64), table_helpers(3));
    // A program in passes (`native_ir::Passes`): its slots a block, and the pass's stores.
    let (stores, slots) = io.unwrap_or((&[], 0));
    for p in 0..programs.len() {
        out.push_str(&format!("#define SLOTS{p} {slots}u\n"));
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
                group.push_str(&format!("        case {l}u: lgroup_{p}_{l}(bytes, out, f, a, a_next, q.z, l, c, cin[s], cw[s], gtabs + s * {gstride}u __UKL_ARGS); break;\n"));
            }
            out.push_str(&cone_function(p, native, lv, l));
            out.push_str(&lanes_cone_function(p, native, lv, l));
            lcone.push_str(&format!("        case {l}u: lcone_{p}_{l}(bytes, out, f, a, a_next, q.z, l, cin[s], cw[s], tables + s * {}u __UKL_ARGS); break;\n", 2 * most));
            cone.push_str(&format!("        case {l}u: cone_{p}_{l}(bytes, out, f, a, a_next, q.z, cin[t], cw[t], tables + t * {}u); break;\n", 2 * most));
            let mut scans: String = lv.at(l).iter().map(|&(i, _)| format!(" scan_prefix(f, {i}u, l, sg, id, tg, tables, cin, cw __UKL_ARGS);")).collect();
            scans.extend(lv.groups_at(l).into_iter().map(|(gi, _)| format!(" gprefix_{p}_{gi}(f, l, sg, id, tg, gtabs, cin, cw __UKL_ARGS);")));
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
         UKL_INLINE void scan_prefix(FileRow f, uint32_t i, uint32_t l, uint32_t sg, uint32_t id, UKL_LOCAL uint32_t* tg, UKL_GLOBAL const uint32_t* tables, UKL_GLOBAL uint64_t* cin, UKL_GLOBAL uint64_t* cw __UKL_PARAMS) {{\n\
         \x20 const uint32_t n = f.halo_base, run = (n + {pt}u - 1u) / {pt}u;\n\
         \x20 const uint32_t lo = ukl_min(id * run, n), hi = ukl_min(lo + run, n);\n\
         \x20 uint32_t mine = TID1;\n\
         \x20 for (uint32_t s = lo; s < hi; s++) mine = tcompose1(mine, tables[(f.group0 + s) * {r}u + 2u * i]);\n\
         \x20 uint32_t total = 0u;\n\
         \x20 const uint32_t e = texclusive1(mine, l, &total __UKL_ARGS);\n\
         \x20 ukl_tg_barrier();\n\
         \x20 if (l == 0u) tg[sg] = total;\n\
         \x20 ukl_tg_barrier();\n\
         \x20 uint32_t before = TID1;\n\
         \x20 for (uint32_t k = 0u; k < sg; k++) before = tcompose1(before, tg[k]);\n\
         \x20 uint32_t c = tapply1(tcompose1(before, e), 0u);\n\
         \x20 // Each segment gives the next its carry at the next's warm-up start.\n\
         \x20 for (uint32_t s = lo; s < hi; s++) {{\n\
         \x20   const uint32_t j = f.group0 + s;\n\
         \x20   cin[j] |= U64(c) << i;\n\
         \x20   if (s + 1u < n) cw[j + 1u] |= U64(tapply1(tables[j * {r}u + 2u * i + 1u], c)) << i;\n\
         \x20   c = tapply1(tables[j * {r}u + 2u * i], c);\n\
         \x20 }}\n\
         }}\n\n",
        r = 2 * most,
        pt = PREFIX_THREADS,
    ));
    let lane_cases: String = (0..programs.len()).map(|p| format!("      case {p}u: lanes_{p}(bytes, f, seg.y, seg.z, false, q.z, l, out, records + s * {}u, st, cin[s], cw[s], counts + s * {counted}u __UKL_ARGS); break;\n", 2 * sw)).collect();
    let again_cases: String = (0..programs.len()).map(|p| format!("      case {p}u: lanes_{p}(bytes, f, seg.y, seg.z, true, q.z, l, out, records + s * {}u, st, cin[s], cw[s], counts + s * {counted}u __UKL_ARGS); break;\n", 2 * sw)).collect();
    let cases: String = (0..programs.len()).map(|p| format!("      case {p}u: serial_{p}(bytes, f, seg.y, seg.z, again, q.z, out, records + s * {}u, st, cin[s], cw[s]); break;\n", 2 * sw)).collect();
    // The positions and totals of each program, from its output words.
    let mut count_cases = String::new();
    let mut write_cases = String::new();
    for (p, native) in programs.iter().enumerate() {
        let mut count = String::new();
        let mut write = String::new();
        for (e, &k) in native.extracts.iter().enumerate() {
            count.push_str(&format!(
                "      {{ uint32_t n = 0u; for (uint32_t j = seg.y; j < seg.z; j++) n += U32(ukl_popcount64(out[f.out_base + {k}u * f.groups + j])); counts[t * {counted}u + {e}u] = n; }}\n"
            ));
            write.push_str(&format!(
                "      {{ uint32_t at = bases[t * {counted}u + {e}u]; for (uint32_t j = seg.y; j < seg.z; j++) {{ uint64_t w = out[f.out_base + {k}u * f.groups + j];\n\
                 \x20       for (uint32_t i = 0u; i < 64u && w != 0ul; i++) {{ positions[f.pos_base + {e}u * ukl_max(len, 1u) + at] = j * 64u + U32(ukl_ctz64(w)); at++; w &= w - 1ul; }} }} }}\n"
            ));
        }
        // All the totals into one flag, the count after the extracts' (`counted_outputs`).
        if !native.totals.is_empty() {
            let words: String = native.totals.iter().map(|&k| format!(" | out[f.out_base + {k}u * f.groups + j]")).collect();
            count.push_str(&format!(
                "      {{ uint64_t any = 0ul; for (uint32_t j = seg.y; j < seg.z; j++) any = any{words}; counts[t * {counted}u + {}u] = U32(any != 0ul); }}\n",
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
                    "      {{ uint64_t w = j < seg.z ? out[f.out_base + {k}u * f.groups + j] : 0ul; const uint32_t n = U32(ukl_popcount64(w));\n\
                     \x20       uint32_t at = bases[s * {counted}u + {e}u] + ukl_sg_exclusive_sum(n, l __UKL_ARGS);\n\
                     \x20       for (uint32_t i = 0u; i < 64u && w != 0ul; i++) {{ positions[f.pos_base + {e}u * ukl_max(len, 1u) + at] = j * 64u + U32(ukl_ctz64(w)); at++; w &= w - 1ul; }} }}\n"
                )
            })
            .collect();
        lwrite_cases.push_str(&format!("    case {p}u: {{\n{write}      break; }}\n"));
    }
    // A segment's totals' flag, its count after the extracts' (`counted_outputs`), in lanes form
    // made by its run: else zero, and no file is refused before the records.
    let total_cases: String = (0..programs.len())
        .filter(|&p| !programs[p].totals.is_empty())
        .map(|p| format!("      case {p}u: total = counts[t * {counted}u + {}u] != 0u; break;\n", programs[p].extracts.len()))
        .collect();
    let extracts: String = (0..programs.len()).map(|p| format!("    case {p}u: e_count = {}u; x_count = {}u; break;\n", counted_outputs(programs[p]).len(), programs[p].extracts.len())).collect();
    out.push_str(&format!(
        r#"// A launch's parameters: `x` a count (segments, files), `y` a level or a run again, `z` the warm-up blocks, `w` the
// check's round.
typedef struct SerialQ {{ uint32_t x, y, z, w; }} SerialQ;
UKL_REFLECT_STRUCT(SerialQ, x, y, z, w)
#define SLAB Bytes bytes = f.slab == 0u ? s0 : f.slab == 1u ? s1 : f.slab == 2u ? s2 : s3
// A segment's run: `q.x` segments, `q.y` 1 for runs again (segment `list[t]`, its start `given`), `q.z` the warm-up
// blocks, `q.w` the check that listed them. A segment: (file, first block, last block, index in the file). A run
// again goes on into the next segment while its end differs from the end it had, unless the next is listed too: a
// stale state (a fill no byte reads until it resets) can carry through many segments. A thread a segment.
DEFINE_KERNEL2(serial, serial_run,
               ROBUFFER, uint32_t, s0,      "[slab]",
               RWBUFFER, uint64_t, out,     "[outputs]",
               ROBUFFER, FileRow,  files,   "[files]",
               ROBUFFER, B4,       segs,    "[segments]",
               RWBUFFER, uint64_t, records, "[2 * sw * segments]",
               ROBUFFER, uint32_t, list,    "[segments]",
               ROBUFFER, uint64_t, given,   "[sw * segments]",
               ARGS,     SerialQ,  q,       ,
               ROBUFFER, uint64_t, cin,     "[segments]",
               ROBUFFER, uint64_t, cw,      "[segments]",
               ROBUFFER, uint32_t, marks,   "[segments]",
               RWBUFFER, uint32_t, hopped,  "[3]",
               ROBUFFER, uint32_t, s1,      "[slab]",
               ROBUFFER, uint32_t, s2,      "[slab]",
               ROBUFFER, uint32_t, s3,      "[slab]",
               GID0,     uint32_t, t,       )
{{
  if (t >= q.x) return;
  const bool again = q.y != 0u;
  uint32_t s = again ? list[t] : t;
  uint64_t st[{sw}];
  for (uint32_t i = 0u; i < {sw}u; i++) st[i] = again ? given[t * {sw}u + i] : 0ul;
  const uint32_t hops = again ? files[segs[s].x].halo_base : 1u;
  uint32_t hop = 0u;
  for (; hop < hops; hop++) {{
    const B4 seg = segs[s];
    const FileRow f = files[seg.x];
    SLAB;
    uint64_t old[{sw}];
    for (uint32_t i = 0u; i < {sw}u; i++) old[i] = records[s * {r}u + {sw}u + i];
    switch (f.program) {{
{cases}      default: break;
    }}
    bool same = true;
    for (uint32_t i = 0u; i < {sw}u; i++) same = same && records[s * {r}u + {sw}u + i] == old[i];
    if (!again || same || seg.w + 1u >= f.halo_base || marks[s + 1u] == q.w) break;
    for (uint32_t i = 0u; i < {sw}u; i++) st[i] = records[s * {r}u + {sw}u + i];
    s++;
  }}
  if (again) {{ ukl_atomic_add(&hopped[0], hop + 1u); ukl_atomic_max(&hopped[1], hop + 1u); }}
}}

// The first run in lanes form: one SIMD group a segment (`lanes_function`), a threadgroup each.
DEFINE_KERNEL2(serial, lanes_run,
               ROBUFFER, uint32_t, s0,      "[slab]",
               RWBUFFER, uint64_t, out,     "[outputs]",
               ROBUFFER, FileRow,  files,   "[files]",
               ROBUFFER, B4,       segs,    "[segments]",
               RWBUFFER, uint64_t, records, "[2 * sw * segments]",
               ARGS,     SerialQ,  q,       ,
               ROBUFFER, uint64_t, cin,     "[segments]",
               ROBUFFER, uint64_t, cw,      "[segments]",
               RWBUFFER, uint32_t, counts,  "[counted * segments]",
               ROBUFFER, uint32_t, s1,      "[slab]",
               ROBUFFER, uint32_t, s2,      "[slab]",
               ROBUFFER, uint32_t, s3,      "[slab]",
               TGID0,    uint32_t, s,       ,
               LANE,     uint32_t, l,       )
{{
  if (s >= q.x) return;
  const B4 seg = segs[s];
  const FileRow f = files[seg.x];
  SLAB;
  const uint64_t st[{sw}] = {{0ul}};
  switch (f.program) {{
{lane_cases}    default: break;
  }}
}}

// The runs again in lanes form: `serial_run`'s, one SIMD group a listed segment.
DEFINE_KERNEL2(serial, lanes_again,
               ROBUFFER, uint32_t, s0,      "[slab]",
               RWBUFFER, uint64_t, out,     "[outputs]",
               ROBUFFER, FileRow,  files,   "[files]",
               ROBUFFER, B4,       segs,    "[segments]",
               RWBUFFER, uint64_t, records, "[2 * sw * segments]",
               ROBUFFER, uint32_t, list,    "[segments]",
               ROBUFFER, uint64_t, given,   "[sw * segments]",
               ARGS,     SerialQ,  q,       ,
               ROBUFFER, uint64_t, cin,     "[segments]",
               ROBUFFER, uint64_t, cw,      "[segments]",
               ROBUFFER, uint32_t, marks,   "[segments]",
               RWBUFFER, uint32_t, hopped,  "[3]",
               RWBUFFER, uint32_t, counts,  "[counted * segments]",
               ROBUFFER, uint32_t, s1,      "[slab]",
               ROBUFFER, uint32_t, s2,      "[slab]",
               ROBUFFER, uint32_t, s3,      "[slab]",
               RWBUFFER, uint32_t, refusal, "[6 * files]",
               TGID0,    uint32_t, i,       ,
               LANE,     uint32_t, l,       )
{{
  if (i >= q.x) return;
  uint32_t s = list[i];
  // A file with a total in a segment before its first listed (the check's, `serial_check`) is refused: its
  // segments are not run again, its later checks list none.
  {{
    UKL_GLOBAL uint32_t* fate = refusal + segs[s].x * 6u;
    const uint32_t par = 1u + 2u * (q.w & 1u);
    if (ukl_atomic_load(fate + par + 1u) > ukl_atomic_load(fate + par)) {{
      if (l == 0u) {{ ukl_atomic_store(fate, 1u); ukl_atomic_add(&hopped[2], 1u); }}
      return;
    }}
  }}
  uint64_t st[{sw}];
  for (uint32_t k = 0u; k < {sw}u; k++) st[k] = given[i * {sw}u + k];
  const uint32_t hops = files[segs[s].x].halo_base;
  uint32_t hop = 0u;
  for (; hop < hops; hop++) {{
    const B4 seg = segs[s];
    const FileRow f = files[seg.x];
    SLAB;
    uint64_t old[{sw}];
    for (uint32_t k = 0u; k < {sw}u; k++) old[k] = records[s * {r}u + {sw}u + k];
    ukl_sg_barrier();
    switch (f.program) {{
{again_cases}      default: break;
    }}
    ukl_sg_barrier();
    bool same = true;
    for (uint32_t k = 0u; k < {sw}u; k++) same = same && records[s * {r}u + {sw}u + k] == old[k];
    if (same || seg.w + 1u >= f.halo_base || marks[s + 1u] == q.w) break;
    for (uint32_t k = 0u; k < {sw}u; k++) st[k] = records[s * {r}u + {sw}u + k];
    s++;
  }}
  if (l == 0u) {{ ukl_atomic_add(&hopped[0], hop + 1u); ukl_atomic_max(&hopped[1], hop + 1u); }}
}}

// Level `q.y`'s pass: each segment's tables of the level's scans (`cone_function`). A thread a segment.
DEFINE_KERNEL2(serial, serial_cone,
               ROBUFFER, uint32_t, s0,     "[slab]",
               ROBUFFER, uint64_t, out,    "[outputs]",
               ROBUFFER, FileRow,  files,  "[files]",
               ROBUFFER, B4,       segs,   "[segments]",
               ARGS,     SerialQ,  q,      ,
               ROBUFFER, uint64_t, cin,    "[segments]",
               RWBUFFER, uint32_t, tables, "[2 * most * segments]",
               ROBUFFER, uint64_t, cw,     "[segments]",
               ROBUFFER, uint32_t, s1,     "[slab]",
               ROBUFFER, uint32_t, s2,     "[slab]",
               ROBUFFER, uint32_t, s3,     "[slab]",
               GID0,     uint32_t, t,      )
{{
  if (t >= q.x) return;
  const B4 seg = segs[t];
  const FileRow f = files[seg.x];
  SLAB;
  const uint32_t a = seg.y - ukl_min(seg.y, q.z), a_next = seg.z - ukl_min(seg.z, q.z);
  switch (f.program) {{
{cone_cases}    default: break;
  }}
}}

// Level `q.y`'s pass in lanes form (`lanes_cone_function`): one SIMD group a segment.
DEFINE_KERNEL2(serial, lanes_cone,
               ROBUFFER, uint32_t, s0,     "[slab]",
               ROBUFFER, uint64_t, out,    "[outputs]",
               ROBUFFER, FileRow,  files,  "[files]",
               ROBUFFER, B4,       segs,   "[segments]",
               ARGS,     SerialQ,  q,      ,
               ROBUFFER, uint64_t, cin,    "[segments]",
               RWBUFFER, uint32_t, tables, "[2 * most * segments]",
               ROBUFFER, uint64_t, cw,     "[segments]",
               ROBUFFER, uint32_t, s1,     "[slab]",
               ROBUFFER, uint32_t, s2,     "[slab]",
               ROBUFFER, uint32_t, s3,     "[slab]",
               TGID0,    uint32_t, s,      ,
               LANE,     uint32_t, l,      )
{{
  if (s >= q.x) return;
  const B4 seg = segs[s];
  const FileRow f = files[seg.x];
  SLAB;
  const uint32_t a = seg.y - ukl_min(seg.y, q.z), a_next = seg.z - ukl_min(seg.z, q.z);
  switch (f.program) {{
{lcone_cases}    default: break;
  }}
}}

// Level `q.y`'s fixed points (`lanes_group_function`): one SIMD group a segment and a state.
DEFINE_KERNEL2(serial, lanes_group,
               ROBUFFER, uint32_t, s0,    "[slab]",
               ROBUFFER, uint64_t, out,   "[outputs]",
               ROBUFFER, FileRow,  files, "[files]",
               ROBUFFER, B4,       segs,  "[segments]",
               ARGS,     SerialQ,  q,     ,
               ROBUFFER, uint64_t, cin,   "[segments]",
               ROBUFFER, uint64_t, cw,    "[segments]",
               ROBUFFER, uint32_t, s1,    "[slab]",
               ROBUFFER, uint32_t, s2,    "[slab]",
               ROBUFFER, uint32_t, s3,    "[slab]",
               RWBUFFER, uint32_t, gtabs, "[stride * segments]",
               TGID0,    uint32_t, w,     ,
               LANE,     uint32_t, l,     )
{{
  const uint32_t s = w / {states}u, c = w % {states}u;
  if (s >= q.x) return;
  const B4 seg = segs[s];
  const FileRow f = files[seg.x];
  SLAB;
  const uint32_t a = seg.y - ukl_min(seg.y, q.z), a_next = seg.z - ukl_min(seg.z, q.z);
  switch (f.program) {{
{group_cases}    default: break;
  }}
}}

// Level `q.y`'s prefix: one threadgroup a file.
DEFINE_KERNEL2(serial, serial_cone_prefix,
               ROBUFFER, FileRow,  files,  "[files]",
               ARGS,     SerialQ,  q,      ,
               RWBUFFER, uint64_t, cin,    "[segments]",
               ROBUFFER, uint32_t, tables, "[2 * most * segments]",
               RWBUFFER, uint64_t, cw,     "[segments]",
               ROBUFFER, uint32_t, gtabs,  "[stride * segments]",
               SHARED,   uint32_t, tg,     {tgs},
               TGID0,    uint32_t, g,      ,
               LID0,     uint32_t, id,     ,
               LANE,     uint32_t, l,      )
{{
  const uint32_t sg = id / GROUP_LANES;
  const FileRow f = files[g];
  switch (f.program) {{
{prefix_cases}    default: break;
  }}
}}

// The check: a segment whose start is not its predecessor's end is listed, with that end. A file refused
// (`refusal`, its first word) lists none: it goes to the fallback. Each file's first segment listed and first
// segment agreeing with a total set, this check's (`q.w`'s parity: words 1 and 2, or 3 and 4), as `~t` (zeroed:
// none); its first segment clears the other parity's for the next check. A total in a segment before the first
// listed is final: the runs again find the file refused (`lanes_again`).
DEFINE_KERNEL2(serial, serial_check,
               ROBUFFER, B4,       segs,    "[segments]",
               ROBUFFER, uint64_t, records, "[2 * sw * segments]",
               RWBUFFER, uint32_t, list,    "[segments]",
               RWBUFFER, uint64_t, given,   "[sw * segments]",
               ARGS,     SerialQ,  q,       ,
               RWBUFFER, uint32_t, listed,  "[1]",
               RWBUFFER, uint32_t, marks,   "[segments]",
               ROBUFFER, uint32_t, counts,  "[counted * segments]",
               RWBUFFER, uint32_t, refusal, "[6 * files]",
               ROBUFFER, FileRow,  files,   "[files]",
               GID0,     uint32_t, t,       )
{{
  if (t >= q.x) return;
  const B4 seg = segs[t];
  UKL_GLOBAL uint32_t* fate = refusal + seg.x * 6u;
  if (ukl_atomic_load(fate) != 0u) return;
  const uint32_t par = 1u + 2u * (q.w & 1u);
  if (seg.w == 0u) {{ ukl_atomic_store(fate + 4u - par, 0u); ukl_atomic_store(fate + 5u - par, 0u); }}
  bool same = seg.w == 0u;
  UKL_GLOBAL const uint64_t* end = records + (t - ukl_min(t, 1u)) * {r}u + {sw}u;
  if (!same) {{
    UKL_GLOBAL const uint64_t* mine = records + t * {r}u;
    same = true;
    for (uint32_t i = 0u; i < {sw}u; i++) same = same && mine[i] == end[i];
  }}
  if (same) {{
    bool total = false;
    switch (files[seg.x].program) {{
{total_cases}      default: break;
    }}
    if (total) ukl_atomic_max(fate + par + 1u, ~t);
    return;
  }}
  ukl_atomic_max(fate + par, ~t);
  const uint32_t at = ukl_atomic_add(listed, 1u);
  list[at] = t;
  marks[t] = q.w;
  for (uint32_t i = 0u; i < {sw}u; i++) given[at * {sw}u + i] = end[i];
}}

// Each segment's count of each extract's bits, and the totals' flags.
DEFINE_KERNEL2(serial, serial_count,
               ROBUFFER, uint64_t, out,    "[outputs]",
               ROBUFFER, FileRow,  files,  "[files]",
               ROBUFFER, B4,       segs,   "[segments]",
               RWBUFFER, uint32_t, counts, "[counted * segments]",
               ARGS,     SerialQ,  q,      ,
               GID0,     uint32_t, t,      )
{{
  if (t >= q.x) return;
  const B4 seg = segs[t];
  const FileRow f = files[seg.x];
  switch (f.program) {{
{count_cases}    default: break;
  }}
}}

// A file's segments' bases: one SIMD group a file, 32 segments at a time.
DEFINE_KERNEL2(serial, serial_prefix,
               ROBUFFER, FileRow,  files,  "[files]",
               ROBUFFER, uint32_t, counts, "[counted * segments]",
               RWBUFFER, uint32_t, bases,  "[counted * segments]",
               RWBUFFER, uint32_t, tot,    "[totals]",
               TGID0,    uint32_t, g,      ,
               LANE,     uint32_t, l,      )
{{
  const FileRow f = files[g];
  uint32_t e_count = 0u, x_count = 0u;
  switch (f.program) {{
{extracts}    default: break;
  }}
  for (uint32_t e = 0u; e < e_count; e++) {{
    uint32_t run = 0u;
    for (uint32_t s = 0u; s < f.halo_base; s += 32u) {{
      const uint32_t j = f.group0 + s + l;
      const uint32_t c = s + l < f.halo_base ? counts[j * {counted}u + e] : 0u;
      const uint32_t before = ukl_sg_exclusive_sum(c, l __UKL_ARGS);
      if (s + l < f.halo_base) bases[j * {counted}u + e] = run + before;
      run += ukl_sg_sum(c __UKL_ARGS);
    }}
    // An extract's count; a total's flag.
    if (l == 0u) tot[f.tot_base + e] = e < x_count ? run : U32(run != 0u);
  }}
}}

// Each segment's positions in lanes form: one SIMD group a segment, a word a lane.
DEFINE_KERNEL2(serial, lanes_write,
               ROBUFFER, uint64_t, out,       "[outputs]",
               ROBUFFER, FileRow,  files,     "[files]",
               ROBUFFER, B4,       segs,      "[segments]",
               ROBUFFER, uint32_t, bases,     "[counted * segments]",
               RWBUFFER, uint32_t, positions, "[positions]",
               ARGS,     SerialQ,  q,         ,
               TGID0,    uint32_t, s,         ,
               LANE,     uint32_t, l,         )
{{
  if (s >= q.x) return;
  const B4 seg = segs[s];
  const FileRow f = files[seg.x];
  const uint32_t len = f.len, j = seg.y + l;
  switch (f.program) {{
{lwrite_cases}    default: break;
  }}
}}

// Each segment's positions, from its base.
DEFINE_KERNEL2(serial, serial_write,
               ROBUFFER, uint64_t, out,       "[outputs]",
               ROBUFFER, FileRow,  files,     "[files]",
               ROBUFFER, B4,       segs,      "[segments]",
               ROBUFFER, uint32_t, bases,     "[counted * segments]",
               RWBUFFER, uint32_t, positions, "[positions]",
               ARGS,     SerialQ,  q,         ,
               GID0,     uint32_t, t,         )
{{
  if (t >= q.x) return;
  const B4 seg = segs[t];
  const FileRow f = files[seg.x];
  const uint32_t len = f.len;
  switch (f.program) {{
{write_cases}    default: break;
  }}
}}
"#,
        r = 2 * sw,
        tgs = PREFIX_THREADS / GROUP,
        states = GROUP_STATES,
    ));
    out.push_str(&records_source(programs));
    (out, sw, counted)
}

/// Threads a file of the record pass: each takes every `RECORD_LANES`th token. A file's lanes are
/// one threadgroup (the pass's barrier is theirs).
const RECORD_LANES: usize = 64;
const _: () = assert!(RECORD_LANES == THREADS, "a file's record lanes are one threadgroup");

/// The record pass (`rerail_compiler::records`) in UKL: per program with records, its tables
/// as constants and its function; the kernel `serial_records`, a file's tokens over
/// `RECORD_LANES` threads. A token's work is straight-line word operations: its first byte's
/// kind, its first eight bytes (two 16-byte loads, shifted), a slot, a compare, and for a word
/// past eight bytes its other words. Each token checks it starts after the token before; a
/// file whose streams disagree gets its totals' flag set.
fn records_source(programs: &[&BlockNative]) -> String {
    let list = |v: &mut dyn Iterator<Item = String>| v.collect::<Vec<_>>().join(", ");
    let mut out = String::from(
        "// Bytes `p..p + 8` of a file, zeros past its end.\n\
         UKL_INLINE uint64_t record_load8(Bytes bytes, FileRow f, uint32_t p) {\n\
         \x20 const B4 a = load16(bytes, f, p / 16u), b = load16(bytes, f, p / 16u + 1u);\n\
         \x20 const uint64_t w0 = U64(a.x) | U64(a.y) << 32, w1 = U64(a.z) | U64(a.w) << 32, w2 = U64(b.x) | U64(b.y) << 32;\n\
         \x20 const uint32_t o = p % 16u, r = (o % 8u) * 8u;\n\
         \x20 const uint64_t lo = o < 8u ? w0 : w1, hi = o < 8u ? w1 : w2;\n\
         \x20 return r == 0u ? lo : (lo >> r) | (hi << (64u - r));\n\
         }\n\
         UKL_INLINE uint64_t record_cut(uint64_t w, uint32_t n) { return n >= 8u ? w : w & ((1ul << (8u * n)) - 1ul); }\n\
         // Whether output `o` holds in bytes `a..=b` (`records::any_in`).\n\
         UKL_INLINE bool record_any(UKL_GLOBAL const uint64_t* out, FileRow f, uint32_t o, uint32_t a, uint32_t b) {\n\
         \x20 if (a > b) return false;\n\
         \x20 for (uint32_t q = a / 64u; q <= b / 64u; q++) {\n\
         \x20   uint64_t x = out[f.out_base + o * f.groups + q];\n\
         \x20   if (q == a / 64u) x &= ~0ul << (a % 64u);\n\
         \x20   if (q == b / 64u && b % 64u != 63u) x &= (1ul << (b % 64u + 1u)) - 1ul;\n\
         \x20   if (x != 0ul) return true;\n\
         \x20 }\n\
         \x20 return false;\n\
         }\n\
         UKL_INLINE bool record_bit(UKL_GLOBAL const uint64_t* out, FileRow f, uint32_t o, uint32_t p) { return ((out[f.out_base + o * f.groups + p / 64u] >> (p % 64u)) & 1ul) != 0ul; }\n",
    );
    let mut cases = String::new();
    // A token test's code (`Records::checks`) over the token `s..=l` (`len` bytes): `bad` set
    // where it holds. A count over its words; a run from a window of one word or two.
    let test_code = |r: &rerail_compiler::records::Records, i: usize, c: &rerail_compiler::records::TokenCheck| -> String {
        let o = c.o;
        assert!(i < 8, "at most 8 token tests");
        if c.always() {
            // At every token its rule reads (`records::test_holds`): a reader's, where an
            // arm's opener holds at its first byte.
            match r.rules.get(usize::from(c.rule)) {
                Some(&rerail_compiler::records::EndRule::Read(k)) => {
                    let any = r.readers[usize::from(k)].arms.iter().map(|a| format!("record_bit(out, f, {}u, s)", a.opener)).collect::<Vec<_>>().join(" || ");
                    format!("bad = bad || {any};")
                }
                _ => "bad = true;".to_string(),
            }
        } else if c.all {
            // The mask at every byte of a token `least` long at least (`records::check_holds`).
            format!("if (len >= {n}u) {{ bool every = true; for (uint32_t q = s / 64u; q <= l / 64u; q++) {{ uint64_t want = ~0ul; if (q == s / 64u) want &= ~0ul << (s % 64u); if (q == l / 64u) want &= ~0ul >> (63u - l % 64u); every = every && (out[f.out_base + {o}u * f.groups + q] & want) == want; }} bad = bad || every; }}", n = c.least)
        } else if c.count {
            format!("uint32_t got = 0u; for (uint32_t q = s / 64u; q <= l / 64u; q++) {{ uint64_t x = out[f.out_base + {o}u * f.groups + q]; if (q == s / 64u) x &= ~0ul << (s % 64u); if (q == l / 64u) x &= ~0ul >> (63u - l % 64u); got += ukl_popcount64(x); }} bad = bad || got >= {n}u;", n = c.least)
        } else {
            let need = format!("{:#x}ul", !0u64 >> (64 - c.least));
            format!("if (len >= {n}u) {{ const uint32_t sh = s % 64u; uint64_t win = out[f.out_base + {o}u * f.groups + s / 64u] >> sh; if (sh + {n}u > 64u) win |= out[f.out_base + {o}u * f.groups + s / 64u + 1u] << (64u - sh); bad = bad || (win & {need}) == {need}; }}", n = c.least)
        }
    };
    for (p, n) in programs.iter().enumerate() {
        let Some(r) = &n.records else { continue };
        let t = rerail_compiler::records::WordTable::new(&r.words).expect("verified: the words fit a table");
        let tails: Vector<u64> = if t.tails.is_empty() { rerail_blocks::vector![0; 7] } else { t.tails.iter().flatten().copied().collect() };
        let (se, le, e, x) = (r.starts, r.lasts, n.extracts.len(), n.extracts.len());
        let tested = r.checks.iter().any(|c| c.then.marks());
        // The tests, each once whatever rules it is at (a bit of `record_tests`).
        let mut distinct: Vec<rerail_compiler::records::TokenCheck> = Vec::new();
        for c in &r.checks {
            // A test that always holds keeps its rule: where it holds is the rule's reads.
            let c = rerail_compiler::records::TokenCheck { rule: if c.always() { c.rule } else { 0 }, ..*c };
            if !distinct.contains(&c) {
                distinct.push(c);
            }
        }
        let bit_of = |c: &rerail_compiler::records::TokenCheck| distinct.iter().position(|d| *d == rerail_compiler::records::TokenCheck { rule: if c.always() { c.rule } else { 0 }, ..*c }).expect("listed");
        out.push_str(&format!(
            "UKL_TABLE uint8_t record_kinds_{p}[256] = {{{kinds}}};\n\
             UKL_TABLE uint64_t record_keys_{p}[{slots}] = {{{keys}}};\n\
             UKL_TABLE uint32_t record_meta_{p}[{slots}] = {{{meta}}};\n\
             UKL_TABLE uint64_t record_tails_{p}[{nt}] = {{{tails}}};\n\
             UKL_TABLE uint8_t record_tests_{p}[256] = {{{tests}}};\n\
             // The word of the token at `s` of `len` bytes: its first eight bytes and its length\n\
             // to a slot, compared, and a word past eight bytes its other words.\n\
             UKL_INLINE uint32_t record_word_{p}(Bytes bytes, FileRow f, uint32_t s, uint32_t len) {{\n\
             \x20 const uint64_t key = record_cut(record_load8(bytes, f, s), len);\n\
             \x20 const uint32_t slot = U32(((key ^ U64(len)) * {mult}ul) >> {shift}u);\n\
             \x20 const uint32_t meta = record_meta_{p}[slot];\n\
             \x20 uint32_t word = (record_keys_{p}[slot] == key && (meta & 0xffu) == len) ? meta >> 8u : 0u;\n\
             \x20 if (word != 0u && len > 8u) {{\n\
             \x20   bool all = true;\n\
             \x20   for (uint32_t j = 1u; j < 8u; j++) all = all && (len <= 8u * j || record_cut(record_load8(bytes, f, s + 8u * j), len - 8u * j) == record_tails_{p}[(word - 1u) * 7u + j - 1u]);\n\
             \x20   word = all ? word : 0u;\n\
             \x20 }}\n\
             \x20 return word;\n\
             }}\n\
             // The marks' tests' hits at the token `s..=l` (`records::check_hits`).\n\
             UKL_INLINE uint32_t record_hits_{p}(Bytes bytes, FileRow f, UKL_GLOBAL const uint64_t* out, uint32_t s, uint32_t l) {{\n\
             \x20 const uint32_t len = l + 1u - s;\n\
             \x20 const uint32_t on = record_tests_{p}[U32(record_load8(bytes, f, s) & 0xfful)];\n\
             \x20 uint32_t hits = 0u;{hits}\n\
             \x20 (void)len; (void)on;\n\
             \x20 return hits;\n\
             }}\n\
             UKL_INLINE void records_{p}(Bytes bytes, FileRow f, uint32_t lane, UKL_GLOBAL const uint64_t* out, UKL_GLOBAL uint32_t* positions, UKL_GLOBAL const uint32_t* tot, UKL_GLOBAL uint32_t* flag, UKL_GLOBAL uint32_t* stop) {{\n\
             \x20 const uint32_t cap = ukl_max(f.len, 1u);{refused}\n\
             \x20 UKL_GLOBAL const uint32_t* starts = positions + f.pos_base + {se}u * cap;\n\
             \x20 UKL_GLOBAL const uint32_t* lasts = positions + f.pos_base + {le}u * cap;\n\
             \x20 UKL_GLOBAL uint32_t* codes = positions + f.pos_base + {e}u * cap;\n\
             \x20 const uint32_t ns = tot[f.tot_base + {se}u], nl = tot[f.tot_base + {le}u], n = ukl_min(ns, nl);\n\
             \x20 bool bad = lane == 0u && ns != nl, told = bad;\n\
             \x20 for (uint32_t k = lane; k < n; k += {lanes}u) {{\n\
             \x20   // Past the first token refused (`stop`: `~n`, the tokens coded): the file goes to the fallback.\n\
             \x20   if (ukl_atomic_load(stop) >= ~k) break;\n\
             \x20   const uint32_t s = starts[k], l = lasts[k], after = k > 0u ? lasts[k - 1u] + 1u : 0u;\n\
             \x20   bad = bad || l < s || s < after;\n\
             \x20   const uint32_t len = l + 1u - s;\n\
             \x20   uint32_t kind = record_kinds_{p}[U32(record_load8(bytes, f, s) & 0xfful)];\n\
             \x20   uint32_t word = record_word_{p}(bytes, f, s, len);{unword}\n\
             \x20   uint32_t marks = 0u;{marks}{retags}{gaps}{checks}\n\
             \x20   codes[k] = kind | (word << {kb}u) | (marks << {ms}u);\n\
             \x20   if (bad && !told) {{ told = true; ukl_atomic_max(stop, ~(k + 1u)); }}\n\
             \x20 }}\n\
             {unplaced}\
             \x20 if (bad) ukl_atomic_or(flag + f.tot_base + {x}u, 1u);\n\
             }}\n",
            // A file its totals refused (`serial_prefix`'s flag) goes to the fallback: no token
            // of it is coded (`stop` at all of them). The pass's own refusal sets the same word
            // (`flag` is `tot`), so every lane reads it before any lane can: a file's lanes are
            // one threadgroup, and its barrier orders the reads first. Read as each lane started,
            // a lane started after another refused returned uncoded: a 64th of the tokens lost
            // each, the CPU's lanes running in turn (the rr marks test, roll-up 22).
            refused = if n.totals.is_empty() {
                String::new()
            } else {
                format!("\n  const bool before = tot[f.tot_base + {x}u] != 0u;\n  ukl_tg_barrier();\n  if (before) {{ if (lane == 0u) ukl_atomic_store(stop, ~0u); return; }}")
            },
            kinds = list(&mut r.kinds.iter().map(|k| k.to_string())),
            // The token tests (`Records::checks`): by its first byte, the tests of a token's
            // rule, each a bit; a count over its words, a run from a window of one word or two.
            tests = list(&mut (0..256usize).map(|b| {
                let rule = r.ends.get(b).copied().unwrap_or(rerail_compiler::records::NO_RULE);
                r.checks.iter().filter(|c| c.rule == rule).fold(0u32, |a, c| a | 1 << bit_of(c)).to_string()
            })),
            // A reader's kind and marks by its start mask (`Records::retags`).
            // A token a reader says is no word (`records::pass`).
            unword = r.retags.iter().filter(|t| !t.part).map(|t| format!(" if (record_bit(out, f, {}u, s)) word = 0u;", t.o)).collect::<String>(),
            retags = r
                .retags
                .iter()
                .map(|t| format!(" if (record_bit(out, f, {}u, s)) {{ kind = {}u; marks |= {}u; }}", t.o, t.kind, t.marks))
                .collect::<String>(),
            checks = distinct
                .iter()
                .enumerate()
                .filter(|(_, c)| c.then == rerail_compiler::records::CheckThen::Escape)
                .map(|(i, c)| format!(" if ((record_tests_{p}[U32(record_load8(bytes, f, s) & 0xfful)] >> {i}u) & 1u) {{ {} }}", test_code(r, i, c)))
                .collect::<String>(),
            // The marks' tests (`CheckThen::At`, `After`): a token's hits, `records::check_hits`.
            hits = distinct
                .iter()
                .enumerate()
                .filter_map(|(i, c)| {
                    let bit = match c.then {
                        // A cut is in the lasts' mask already (`TokenRule::last`).
                        rerail_compiler::records::CheckThen::Escape | rerail_compiler::records::CheckThen::Cut => return None,
                        rerail_compiler::records::CheckThen::At(k) => 1u32 << k,
                        rerail_compiler::records::CheckThen::After(k) => 1u32 << (rerail_compiler::records::AFTER_SHIFT + u32::from(k)),
                    };
                    Some(format!(" if ((on >> {i}u) & 1u) {{ bool bad = false; {} if (bad) hits |= {bit}u; }}", test_code(r, i, c)))
                })
                .collect::<String>(),
            slots = t.keys.len(),
            keys = list(&mut t.keys.iter().map(|k| format!("{k:#x}ul"))),
            meta = list(&mut t.meta.iter().map(|m| format!("{m}u"))),
            nt = tails.len(),
            tails = list(&mut tails.iter().map(|k| format!("{k:#x}ul"))),
            lanes = RECORD_LANES,
            mult = t.mult,
            shift = 64 - t.bits,
            kb = rerail_compiler::records::KIND_BITS,
            ms = rerail_compiler::records::MARK_SHIFT,
            marks = r
                .marks
                .iter()
                .enumerate()
                .map(|(k, &(o, w))| {
                    let word = if w == 0 { String::new() } else { format!(" && word == {w}u") };
                    if o == rerail_compiler::records::WORD_ALONE {
                        return format!(" if (word == {w}u) marks |= {}u;", 1u32 << k);
                    }
                    format!(" if (((out[f.out_base + {o}u * f.groups + s / 64u] >> (s % 64u)) & 1ul) != 0ul{word}) marks |= {}u;", 1u32 << k)
                })
                .collect::<String>(),
            // `else escape` (`Records::unplaced`): lane 0 reads the bytes past the last grammar
            // token's start (the extras' passed over).
            unplaced = if r.unplaced == 0 {
                String::new()
            } else {
                let before = match r.extra.map(|e| r.marks[usize::from(e)].0) {
                    Some(o) => format!("record_bit(out, f, {o}u, starts[j])"),
                    None => "false".into(),
                };
                let tests: String = r
                    .marks
                    .iter()
                    .enumerate()
                    .filter(|&(m, _)| r.unplaced >> m & 1 == 1)
                    .map(|(m, &(o, _))| {
                        let any = if r.tested_only >> m & 1 == 1 { String::new() } else { format!(" || record_any(out, f, {o}u, from, f.len - 1u)") };
                        format!(" bad = bad || ((pend >> {m}u) & 1u) != 0u{any};")
                    })
                    .collect();
                // The marks' tests' bits after the last grammar token: its `After` hits, and the
                // extras' after it, both.
                let pend = if tested {
                    format!(" uint32_t pend = 0u; for (int32_t u = ukl_max(j, 0); u < I32(n); u++) {{ const uint32_t h = record_hits_{p}(bytes, f, out, starts[u], lasts[u]); pend |= (h >> {a}u) | (u > j ? h & {low}u : 0u); }}", low = (1u32 << rerail_compiler::records::AFTER_SHIFT) - 1, a = rerail_compiler::records::AFTER_SHIFT)
                } else {
                    " const uint32_t pend = 0u;".to_string()
                };
                format!("\x20 if (lane == 0u && f.len > 0u) {{ int32_t j = I32(n) - 1; while (j >= 0 && {before}) j--; const uint32_t from = j >= 0 ? starts[j] + 1u : 0u;{pend}{tests} }}\n")
            },
            // The gap marks (`records::gap_marks`): a grammar token's from the bytes after the
            // grammar token before it, the extras' tokens before it passed over by their mark.
            gaps = if r.gaps == 0 {
                String::new()
            } else {
                // The marks' tests' bits (`records::gap_marks`): the token's own `At` hits; the
                // grammar token before's `After` hits; the extras' between, both.
                let pend = || {
                    if !tested {
                        return " const uint32_t pend = 0u;".to_string();
                    }
                    format!(" uint32_t pend = record_hits_{p}(bytes, f, out, s, l) & {low}u; for (int32_t u = ukl_max(j, 0); u < I32(k); u++) {{ const uint32_t h = record_hits_{p}(bytes, f, out, starts[u], lasts[u]); pend |= (h >> {a}u) | (u > j ? h & {low}u : 0u); }}", low = (1u32 << rerail_compiler::records::AFTER_SHIFT) - 1, a = rerail_compiler::records::AFTER_SHIFT)
                };
                let extra = r.extra.map(|e| (e, r.marks[usize::from(e)].0));
                let is_extra = |at: &str| match extra {
                    Some((_, o)) => format!("record_bit(out, f, {o}u, {at})"),
                    None => "false".into(),
                };
                let tests: String = r
                    .marks
                    .iter()
                    .enumerate()
                    .filter(|&(m, _)| r.gaps >> m & 1 == 1)
                    .map(|(m, &(o, _))| {
                        // `unless after`: none after a grammar token of these words.
                        let none: String = r.unless.iter().filter(|&&(u, _)| usize::from(u) == m).map(|&(_, w)| format!(" && wb != {w}u")).collect();
                        let any = if r.tested_only >> m & 1 == 1 { "false".to_string() } else { format!("record_any(out, f, {o}u, from, s)") };
                        format!(" if (((pend >> {m}u) & 1u || {any}){none}) marks |= {}u;", 1u32 << m)
                    })
                    .collect();
                let wb = if r.unless.is_empty() { String::new() } else { format!(" const uint32_t wb = j >= 0 ? record_word_{p}(bytes, f, starts[j], lasts[j] + 1u - starts[j]) : 0u;") };
                // An extra keeps the marks of its own gap (`Records::own`), from the record
                // before.
                let own: String = (0..r.marks.len())
                    .filter(|&m| r.own >> m & 1 == 1 && r.gaps >> m & 1 == 1 && r.tested_only >> m & 1 == 0)
                    .map(|m| format!(" if (record_any(out, f, {}u, from, s)) marks |= {}u;", r.marks[m].0, 1u32 << m))
                    .collect();
                let own = if own.is_empty() { String::new() } else { format!(" else {{ const uint32_t from = k > 0u ? starts[k - 1u] + 1u : 0u;{own} }}") };
                format!(
                    " marks &= ~{gaps}u; if (!{here}) {{ int32_t j = I32(k) - 1; while (j >= 0 && {before}) j--; const uint32_t from = j >= 0 ? starts[j] + 1u : 0u;{pend}{wb}{tests} }}{own}",
                    pend = pend(),
                    gaps = r.gaps,
                    here = is_extra("s"),
                    before = is_extra("starts[j]"),
                )
            },
        ));
        cases.push_str(&format!("    case {p}u: records_{p}(bytes, f, lane, out, positions, tot, flag, refusal + fi * 6u + 5u); break;\n"));
    }
    out.push_str(&format!(
        "// The record pass: thread `t` is file `t / {lanes}`'s lane `t % {lanes}`.\n\
         DEFINE_KERNEL2(serial, serial_records, ROBUFFER, uint32_t, s0, \"[slab]\", ROBUFFER, uint64_t, out, \"[outputs]\", ROBUFFER, FileRow, files, \"[files]\",\n\
         \x20   RWBUFFER, uint32_t, positions, \"[positions]\", ARGS, SerialQ, q, , ROBUFFER, uint32_t, tot, \"[totals]\", RWBUFFER, uint32_t, flag, \"[totals]\",\n\
         \x20   ROBUFFER, uint32_t, s1, \"[slab]\", ROBUFFER, uint32_t, s2, \"[slab]\", ROBUFFER, uint32_t, s3, \"[slab]\", RWBUFFER, uint32_t, refusal, \"[6 * files]\",\n\
         \x20   GID0, uint32_t, t, ) {{\n\
         \x20 const uint32_t fi = t / {lanes}u, lane = t % {lanes}u;\n\
         \x20 if (fi >= q.x) return;\n\
         \x20 const FileRow f = files[fi];\n\
         \x20 SLAB;\n\
         \x20 switch (f.program) {{\n{cases}    default: break;\n  }}\n\
         }}\n",
        lanes = RECORD_LANES,
    ));
    out
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
fn counted_outputs(native: &BlockNative) -> Vector<usize> {
    rerail_compiler::block_native::counted(native)
}

/// Whether the exact scans are left to the runs to learn: no `RERAIL_SERIAL_EXACT`.
fn exact_unset() -> bool {
    std::env::var("RERAIL_SERIAL_EXACT").is_err()
}

/// After a first pass with wrong starts: each program's scans wrong in more than one segment in
/// `PROMOTE` (their carry bit in the start differs from the end before) join its `LEARNED`.
///
/// A source whose runs promoted nothing new is settled: its runs learn no more (what they would
/// find is what it holds), and their records are not read back.
fn learn(key: u64, forms: &[BlockNative], rows: &[FileRow], segs: &[[u32; 4]], records: &[u64], sw: usize) {
    let mut grew = false;
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
        let promoted: Vector<usize> = state.parts.iter().zip(&wrong).filter(|&(_, &n)| n * PROMOTE > seen).map(|(&(at, ..), _)| at).collect();
        if !promoted.is_empty() {
            let st_ = kept_programs();
            if let Ok(mut m) = st_.learned.lock() {
                let known = m.entry(fingerprint(native)).or_default();
                for at in promoted {
                    if !known.contains(&at) {
                        known.push(at);
                        grew = true;
                    }
                }
            }
        }
    }
    if !grew {
        let st_ = kept_programs();
        if let Ok(mut m) = st_.settled.lock() {
            m.insert(key);
        }
    }
}

/// The sources (`serial_source`'s keys) whose runs learn nothing more (`learn`).
fn settled(key: u64) -> bool {
    kept_programs().settled.lock().is_ok_and(|m| m.contains(&key))
}

/// What a run's buffers and launches take from its programs' levels, made once a source.
struct Shape {
    /// The most scans of a program (the tables' words a segment).
    most: usize,
    /// The deepest level (`cones_on`: else none).
    depth: usize,
    /// The fixed-point tables' words a segment (`group_stride`).
    stride: usize,
    /// The levels with a fixed point (`group_levels`).
    with_groups: std::collections::BTreeSet<usize>,
}

fn shape_of(key: u64, programs: &[&BlockNative]) -> std::sync::Arc<Shape> {
    let st_ = kept_programs();
    if let Some(hit) = st_.shapes.lock().ok().and_then(|m| m.get(&key).cloned()) {
        return hit;
    }
    let levels: Vec<Levels> = programs.iter().map(|n| Levels::of(n)).collect();
    let shape = std::sync::Arc::new(Shape {
        most: levels.iter().map(|l| l.scans.len()).max().unwrap_or(0).max(1),
        depth: if cones_on() { levels.iter().map(|l| l.depth).max().unwrap_or(0) } else { 0 },
        stride: group_stride(programs),
        with_groups: group_levels(programs),
    });
    let st_ = kept_programs();
    if let Ok(mut m) = st_.shapes.lock() {
        m.insert(key, shape.clone());
    }
    shape
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

/// The forward carry's source for `programs` (each in its planes form), as a batch of them runs it: what the
/// tests read of its lane code.
pub fn source(programs: &[&BlockNative]) -> String {
    let forms: Vec<BlockNative> = programs.iter().map(|p| p.planes_form()).collect();
    let refs: Vec<&BlockNative> = forms.iter().collect();
    serial_source(&refs, None).1.to_string()
}

/// The cone pass's kernel: in lanes form or one thread a segment (`lanes_on`).
fn cone_kernel() -> &'static str {
    if lanes_on() { "lanes_cone" } else { "serial_cone" }
}

/// Every kernel of a serial source that `run_serial_io` compiles.
fn serial_kernels() -> [&'static str; 12] {
    [
        cone_kernel(),
        "serial_cone_prefix",
        "lanes_group",
        "lanes_run",
        "lanes_again",
        "serial_run",
        "serial_check",
        "serial_count",
        "serial_prefix",
        "serial_write",
        "lanes_write",
        "serial_records",
    ]
}

/// Whether `programs`' kernels in the forward carry (their source as runs have learned it) are compiled. When one is
/// not, its compile starts on a thread of its own (`kernels::ready_behind_function`) and this is false at once: the
/// caller runs the batch another way meanwhile, and no launch waits on the compiler (decision 0012). A batch with back
/// fills or loads runs in passes (`run_passes`), one program at a time; its kernels are compiled at its run.
pub fn compiled(programs: &[&BlockNative]) -> bool {
    if super::ready().is_err() {
        return false;
    }
    let forms: Vec<BlockNative> = programs.iter().map(|p| p.planes_form()).collect();
    if forms.iter().any(|p| p.insts.iter().any(|i| matches!(i, BlockInst::BackFill(..) | BlockInst::Load(_)))) {
        return true;
    }
    let refs: Vec<&BlockNative> = forms.iter().collect();
    let (_, source, _, _) = serial_source(&refs, None);
    // Every kernel asked, so that each one not compiled starts compiling now.
    serial_kernels().iter().fold(true, |all, name| rerail_accel::gpu::kernels::ready_behind_function(&source, name, None).is_some() && all)
}

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
            first.dropped += second.dropped;
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
    Ok(run_kept(programs, files, None, segments, true)?.0)
}

/// `run_serial` with tier 1's buffers left where the device wrote them (`Resident`): no copy of
/// them back. The run's `files` are empty.
pub fn run_resident(programs: &[&BlockNative], files: &[BatchFile<'_>], segments: Segments) -> Result<(SerialRun, Resident), Error> {
    run_resident_digested(programs, files, None, segments)
}

/// `run_resident` with each file's SHA-256 given (`digests`, in the files' order), as the
/// residency's documents take theirs (decision 0032 §4): the engine knows them from a `stat` or
/// a Git blob (decision 0142 §3), and the run hashes no file's bytes.
pub fn run_resident_digested(programs: &[&BlockNative], files: &[BatchFile<'_>], digests: Option<&[[u8; 32]]>, segments: Segments) -> Result<(SerialRun, Resident), Error> {
    let (run, kept) = run_kept(programs, files, digests, segments, false)?;
    Ok((run, kept.ok_or(Error::BackendFailure)?))
}

/// Compiles `programs`' kernels ahead of their first batch, and what a run learns of them
/// (`learn`): runs over `sample` until one makes no source or kernel, each compiling what the
/// one before learned. Nothing is kept of the runs. A process calls it once a program, off the parse's path (a
/// thread of its own), so no batch waits on Metal's compiler.
pub fn prewarm(programs: &[&BlockNative], sample: &[BatchFile<'_>], segments: Segments) -> Result<(), Error> {
    // Until a run makes nothing: each run may learn, and its next compiles what it learned.
    for _ in 0..6 {
        let before = compiles();
        run_split_kept(programs, sample, segments)?;
        if compiles() == before {
            break;
        }
    }
    Ok(())
}

/// `run_kept` over `files`, halved where the device refuses it, its results dropped.
fn run_split_kept(programs: &[&BlockNative], files: &[BatchFile<'_>], segments: Segments) -> Result<(), Error> {
    match run_kept(programs, files, None, segments, false) {
        Err(Error::BackendCapacity) if files.len() > 1 => {
            let (a, b) = files.split_at(files.len() / 2);
            run_split_kept(programs, a, segments)?;
            run_split_kept(programs, b, segments)
        }
        other => other.map(|_| ()),
    }
}

/// The files' bytes admitted once a run, every pass of it reading them where they are.
fn run_kept(programs: &[&BlockNative], files: &[BatchFile<'_>], digests: Option<&[[u8; 32]]>, segments: Segments, read: bool) -> Result<(SerialRun, Option<Resident>), Error> {
    let bytes: Vec<&[u8]> = files.iter().map(|f| f.bytes).collect();
    let sources = match digests {
        Some(d) if d.len() == files.len() => crate::slabs::admit_sources_digested(&bytes, d)?,
        Some(_) => return Err(Error::InvalidOperand(0)),
        None => crate::slabs::admit_sources(&bytes)?,
    };
    if programs.iter().any(|p| p.insts.iter().any(|i| matches!(i, BlockInst::BackFill(..) | BlockInst::Load(_)))) {
        return run_passes(programs, files, &sources, segments, read);
    }
    let (run, kept) = run_serial_io(programs, files, &sources, segments, None, read)?;
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
    /// Where the records are in the positions (`records::RecordLayout`): the device's block
    /// passes write a token a thread, so the compiler chooses columns for them.
    pub layout: rerail_compiler::records::RecordLayout,
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
        Resident {
            out,
            tot: self.tot,
            positions: self.positions,
            files: self.files,
            sources: self.sources,
            layout: rerail_compiler::records::RecordLayout::choose(rerail_compiler::records::RecordWriter::Lanes, false),
        }
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
/// A program's passes (`BlockNative::passes`), made once a program: in a debug build `.rerail`'s
/// took most of a second, at every run.
fn passes_of(program: &BlockNative) -> std::sync::Arc<rerail_compiler::block_native::Passes> {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    format!("{program:?}").hash(&mut h);
    let key = h.finish();
    let st_ = kept_programs();
    if let Some(hit) = st_.passes.lock().ok().and_then(|m| m.get(&key).cloned()) {
        return hit;
    }
    let made = std::sync::Arc::new(program.passes());
    let st_ = kept_programs();
    if let Ok(mut m) = st_.passes.lock() {
        m.insert(key, made.clone());
    }
    made
}

fn run_passes(programs: &[&BlockNative], files: &[BatchFile<'_>], sources: &[std::sync::Arc<crate::slabs::ResidentSource>], segments: Segments, read: bool) -> Result<(SerialRun, Option<Resident>), Error> {
    let [program] = programs else { return Err(Error::UnsupportedBackend) };
    ready()?;
    let passes = passes_of(program);
    let passes: &rerail_compiler::block_native::Passes = &passes;
    let fit = |n: usize| u32::try_from(n).map_err(|_| Error::BackendCapacity);
    let blocks: Vector<usize> = files.iter().map(|f| blocks_of(f.bytes.len(), program.delay)).collect();
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
    let out = crate::pool::zeroed_on_device::<u64>((offset + widest * total_blocks).max(1))?;
    fit(offset + widest * total_blocks)?;
    // The segments, as `run_serial_io` cuts them: (file, first block, end, index in the file).
    let mut rows = Vec::with_capacity(files.len());
    let mut segs: Vector<[u32; 4]> = rerail_blocks::vector::Vector::new();
    for (i, &b) in blocks.iter().enumerate() {
        let count = b.div_ceil(segments.blocks);
        rows.push(FileRow { group0: fit(segs.len())?, groups: fit(b)?, halo_base: fit(count)?, slot_base: slot_base[i], len: fit(files[i].bytes.len())?, ..FileRow::default() });
        for s in 0..count {
            segs.push([fit(i)?, fit(s * segments.blocks)?, fit(((s + 1) * segments.blocks).min(b))?, fit(s)?]);
        }
    }
    let (mut seconds, mut launches) = (0.0, 0usize);
    let (mut reruns, mut dropped, mut rounds, mut first_seconds, mut rerun_seconds, mut sink_seconds) = (0, 0, 0, 0.0, 0.0, 0.0);
    let last = passes.passes.len() - 1;
    for (i, pass) in passes.passes.iter().enumerate() {
        match pass {
            rerail_compiler::block_native::Pass::Forward { body, stores } => {
                let io = Io { out: &out, offset, slot_base: &slot_base, slots: passes.slots, stores };
                let (run, kept) = run_serial_io(&[body], files, sources, segments, Some(&io), read && i == last)?;
                seconds += run.run.seconds;
                launches += run.run.launches;
                reruns += run.reruns;
                dropped += run.dropped;
                rounds += run.rounds;
                first_seconds += run.first_seconds;
                rerun_seconds += run.rerun_seconds;
                sink_seconds += run.sink_seconds;
                if i == last {
                    let run = SerialRun { run: BatchRun { files: run.run.files, launches, seconds }, segments: segs.len(), reruns, dropped, rounds, first_seconds, rerun_seconds, sink_seconds };
                    return Ok((run, if read { None } else { kept.map(|k| k.with(Some(out))) }));
                }
            }
            rerail_compiler::block_native::Pass::Back(fills) => {
                let key = {
                    use std::hash::{Hash, Hasher};
                    let mut h = std::collections::hash_map::DefaultHasher::new();
                    ("back", fills, passes.slots).hash(&mut h);
                    h.finish()
                };
                let made = |name| match kernels().0.lock().ok().and_then(|m| m.get(&(key, name)).cloned()) {
                    Some(hit) => Ok(hit),
                    None => kernel(key, &back_kernels(fills, passes.slots), name),
                };
                let (tables_k, prefix_k, apply_k) = (made("back_tables")?, made("back_prefix")?, made("back_apply")?);
                let table = crate::pool::upload(&rows)?;
                let seg_table = crate::pool::upload(&segs)?;
                let tables = crate::pool::zeroed::<u32>((segs.len() * fills.len()).max(1))?;
                let cins = crate::pool::zeroed::<u32>((segs.len() * fills.len()).max(1))?;
                let buffers: Vec<(&'static str, Buffer)> =
                    [("out", &out), ("files", &table), ("segs", &seg_table), ("tables", &tables), ("cins", &cins)].map(|(name, b)| (name, b.device_buffer().clone())).to_vec();
                let n = |x: usize| vec![("q", [fit(x).unwrap_or(0), 0u32, 0, 0])];
                let groups = |x: usize| x.div_ceil(THREADS).max(1);
                let kernels = [
                    Pass { name: "back_tables", kernel: tables_k, buffers: buffers.clone(), args: n(segs.len()), groups: groups(segs.len()), threads: THREADS },
                    Pass { name: "back_prefix", kernel: prefix_k, buffers: buffers.clone(), args: n(files.len()), groups: groups(files.len()), threads: THREADS },
                    Pass { name: "back_apply", kernel: apply_k, buffers, args: n(segs.len()), groups: groups(segs.len()), threads: THREADS },
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
        let words = format!("const uint64_t s{j} = out[f.slot_base + b * {slots}u + {s}u], e{j} = out[f.slot_base + b * {slots}u + {e}u];");
        tables.push_str(&format!("    {{ {words} uint32_t o = 0u; back_fill_word(s{j}, e{j}, c{j}_0, &o); c{j}_0 = o; back_fill_word(s{j}, e{j}, c{j}_1, &o); c{j}_1 = o; }}\n"));
        apply.push_str(&format!("    {{ {words} uint32_t o = 0u; out[f.slot_base + b * {slots}u + {own}u] = back_fill_word(s{j}, e{j}, c{j}, &o); c{j} = o; }}\n"));
        prefix.push_str(&format!("    {{ cins[g * {n}u + {j}u] = c{j}; c{j} = (tables[g * {n}u + {j}u] >> c{j}) & 1u; }}\n"));
    }
    let init = |f: &dyn Fn(usize) -> String| (0..n).map(f).collect::<String>();
    format!(
        "{}\n\
         typedef struct SerialQ {{ uint32_t x, y, z, w; }} SerialQ;\n\
         UKL_REFLECT_STRUCT(SerialQ, x, y, z, w)\n\
         // `back_fill` on a word: `fill` on the bits reversed; the carry out is bit 0.\n\
         UKL_INLINE uint64_t back_fill_word(uint64_t s, uint64_t e, uint32_t cin, UKL_PRIVATE uint32_t* cout) {{\n\
         \x20 uint32_t o = 0u;\n\
         \x20 const uint64_t b = reverse64(fill_word(reverse64(s), reverse64(e), U64(cin), &o));\n\
         \x20 *cout = U32(b & 1ul);\n\
         \x20 return b;\n}}\n\
         DEFINE_KERNEL2(serial, back_tables, ROBUFFER, uint64_t, out, \"[slots]\", ROBUFFER, FileRow, files, \"[files]\", ROBUFFER, B4, segs, \"[segments]\",\n\
         \x20              ARGS, SerialQ, q, , RWBUFFER, uint32_t, tables, \"[fills * segments]\", GID0, uint32_t, t, ) {{\n\
         \x20 if (t >= q.x) return;\n\
         \x20 const B4 seg = segs[t];\n\
         \x20 const FileRow f = files[seg.x];\n\
         {}\
         \x20 for (uint32_t b = seg.z; b > seg.y;) {{\n    b--;\n{tables}  }}\n\
         {}}}\n\
         DEFINE_KERNEL2(serial, back_prefix, ROBUFFER, FileRow, files, \"[files]\", ARGS, SerialQ, q, , ROBUFFER, uint32_t, tables, \"[fills * segments]\",\n\
         \x20              RWBUFFER, uint32_t, cins, \"[fills * segments]\", GID0, uint32_t, t, ) {{\n\
         \x20 if (t >= q.x) return;\n\
         \x20 const FileRow f = files[t];\n\
         {}\
         \x20 for (uint32_t s = f.halo_base; s > 0u;) {{\n    s--;\n    const uint32_t g = f.group0 + s;\n{prefix}  }}\n}}\n\
         DEFINE_KERNEL2(serial, back_apply, RWBUFFER, uint64_t, out, \"[slots]\", ROBUFFER, FileRow, files, \"[files]\", ROBUFFER, B4, segs, \"[segments]\",\n\
         \x20              ARGS, SerialQ, q, , ROBUFFER, uint32_t, cins, \"[fills * segments]\", GID0, uint32_t, t, ) {{\n\
         \x20 if (t >= q.x) return;\n\
         \x20 const B4 seg = segs[t];\n\
         \x20 const FileRow f = files[seg.x];\n\
         {}\
         \x20 for (uint32_t b = seg.z; b > seg.y;) {{\n    b--;\n{apply}  }}\n}}\n",
        head(LaneWidth::W64),
        init(&|j| format!("  uint32_t c{j}_0 = 0u, c{j}_1 = 1u;\n")),
        init(&|j| format!("  tables[t * {n}u + {j}u] = c{j}_0 | (c{j}_1 << 1u);\n")),
        init(&|j| format!("  uint32_t c{j} = 0u;\n")),
        init(&|j| format!("  uint32_t c{j} = cins[t * {n}u + {j}u];\n")),
    )
}

/// `run_serial` over programs without passes, or one forward pass of a program in passes (`io`).
fn run_serial_io(programs: &[&BlockNative], files: &[BatchFile<'_>], sources: &[std::sync::Arc<crate::slabs::ResidentSource>], segments: Segments, io: Option<&Io<'_>>, read: bool) -> Result<(SerialRun, Option<Kept>), Error> {
    ready()?;
    let forms: Vec<BlockNative> = programs.iter().map(|p| p.planes_form()).collect();
    let refs: Vec<&BlockNative> = forms.iter().collect();
    let (source_key, source, sw, counted) = serial_source(&refs, io.map(|io| (io.stores, io.slots)));
    let source: &str = &source;
    let fit = |n: usize| u32::try_from(n).map_err(|_| Error::BackendCapacity);
    let (slabs, slab_of) = crate::slabs::source_slabs(sources).ok_or(Error::BackendCapacity)?;
    if files.is_empty() {
        return Ok((SerialRun { run: BatchRun { files: Vec::new(), launches: 0, seconds: 0.0 }, segments: 0, reruns: 0, dropped: 0, rounds: 0, first_seconds: 0.0, rerun_seconds: 0.0, sink_seconds: 0.0 }, None));
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
        // The extracts' positions, and with records a code a token after them.
        pos_base += (n.extracts.len() + usize::from(n.records.is_some())) * len.max(1);
        tot_base += rerail_compiler::block_native::counted(n).len();
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
    let shape = shape_of(source_key, &refs);
    let (most, depth, stride, with_groups) = (shape.most, shape.depth, shape.stride, &shape.with_groups);
    let tables = crate::pool::zeroed::<u32>(total * 2 * most)?;
    let cins = crate::pool::zeroed::<u64>(total)?;
    let cws = crate::pool::zeroed::<u64>(total)?;
    let gtabs = crate::pool::zeroed::<u32>(total * stride)?;
    let marks = crate::pool::zeroed::<u32>(total)?;
    // Segments run again in all, and the most one thread ran.
    let hopped = crate::pool::zeroed::<u32>(3)?;
    // Each file's refusal while its starts are found (`serial_check`): refused, then each
    // parity's first listed and first total; and its tokens coded before the first refused, as
    // `~n` (`records_source`).
    let refusal = crate::pool::zeroed::<u32>(rows.len() * 6)?;
    assert!(2 * segments.warm < segments.blocks, "a segment is longer than its two warm-ups");
    let cone = kernel(source_key, source, cone_kernel())?;
    let cone_prefix = kernel(source_key, source, "serial_cone_prefix")?;
    let group_pass = kernel(source_key, source, "lanes_group")?;
    let lanes = lanes_on();
    if lanes {
        // The lanes: the warm-up, the segment, and the block after it for the delayed outputs.
        assert!(segments.warm >= 1 && segments.blocks + 2 * segments.warm < GROUP, "a segment and its two warm-ups fit a SIMD group");
    }
    let lanes_run = kernel(source_key, source, "lanes_run").inspect_err(|e| eprintln!("lanes: pipeline: {e:?}"))?;
    let lanes_again = kernel(source_key, source, "lanes_again").inspect_err(|e| eprintln!("lanes: again: {e:?}"))?;
    let run = kernel(source_key, source, "serial_run").inspect_err(|e| eprintln!("serial: pipeline: {e:?}"))?;
    let check = kernel(source_key, source, "serial_check").inspect_err(|e| eprintln!("serial: check: {e:?}"))?;
    let count = kernel(source_key, source, "serial_count")?;
    let prefix = kernel(source_key, source, "serial_prefix")?;
    let write = kernel(source_key, source, "serial_write")?;
    let lanes_write = kernel(source_key, source, "lanes_write")?;
    let records_pass = kernel(source_key, source, "serial_records")?;
    // Every buffer of the run by its row name: each pass's kernel binds those of its rows (`Launch::buffer_if`).
    let mut all: Vec<(&'static str, Buffer)> = vec![("out", out.device_buffer().clone()), ("files", table.device_buffer().clone()), ("segs", seg_table.device_buffer().clone())];
    for (k, name) in SLAB_BUFFERS.iter().enumerate() {
        all.push((name, slabs.get(k).unwrap_or(&slabs[0]).device_buffer().clone()));
    }
    for (name, block) in [
        ("records", &records),
        ("list", &list),
        ("given", &given),
        ("listed", &listed),
        ("counts", &counts),
        ("bases", &bases),
        ("tot", &tot),
        ("flag", &tot),
        ("tables", &tables),
        ("cin", &cins),
        ("cw", &cws),
        ("marks", &marks),
        ("gtabs", &gtabs),
        ("hopped", &hopped),
        ("refusal", &refusal),
        ("positions", &positions),
    ] {
        all.push((name, block.device_buffer().clone()));
    }
    let groups = |n: usize| n.div_ceil(THREADS).max(1);
    let warm = fit(segments.warm).unwrap_or(0);
    let args = |a: u32, b: u32| vec![("q", [a, b, warm, 0u32])];
    let at_round = |a: u32, b: u32, round: usize| vec![("q", [a, b, warm, round as u32])];
    // A pass: `name`'s kernel over every buffer. A lanes kernel runs a SIMD group a threadgroup: its groups loop by
    // their data, and where SIMD groups are emulated a group waits for every thread of its threadgroup (0440 step C).
    let pass = |name: &'static str, kernel: &lane::Kernel, args: Vec<(&'static str, [u32; 4])>, groups: usize, threads: usize| Pass { name, kernel: kernel.clone(), buffers: all.clone(), args, groups, threads };
    let mut first = Vec::new();
    for l in 1..=depth {
        if lanes_on() {
            first.push(pass("lanes_cone", &cone, args(fit(total)?, fit(l)?), total, GROUP));
        } else {
            first.push(pass("serial_cone", &cone, args(fit(total)?, fit(l)?), groups(total), THREADS));
        }
        if with_groups.contains(&l) {
            first.push(pass("lanes_group", &group_pass, args(fit(total)?, fit(l)?), total * GROUP_STATES, GROUP));
        }
        first.push(pass("serial_cone_prefix", &cone_prefix, args(0, fit(l)?), files.len(), PREFIX_THREADS));
    }
    if lanes {
        first.push(pass("lanes_run", &lanes_run, args(fit(total)?, 0), total, GROUP));
    } else {
        first.push(pass("serial_run", &run, args(fit(total)?, 0), groups(total), THREADS));
    }
    first.push(pass("serial_check", &check, at_round(fit(total)?, 0, 1), groups(total), THREADS));
    if std::env::var("RERAIL_SERIAL_TIMES").is_ok() {
        // Each pass alone, for its time; then the run proper.
        // Each kernel's GPU time, and the threads a threadgroup its registers allow (1,024 at
        // most: fewer when its registers are many).
        let times: Vec<String> = first
            .iter()
            .map(|p| {
                let ms = encode_passes(std::slice::from_ref(p)).map(|s| s * 1e3).unwrap_or(f64::NAN);
                let most = crate::lane::limits(&p.kernel).map_or(0, |l| l.threads);
                format!("{} {ms:.3} ms ({most} threads)", p.name)
            })
            .collect();
        eprintln!("serial: {} segments; {}", total, times.join(", "));
        unsafe { std::ptr::write_bytes(listed.host().cast::<u32>(), 0, 1) };
        unsafe { std::ptr::write_bytes(cins.host().cast::<u64>(), 0, total) };
        unsafe { std::ptr::write_bytes(cws.host().cast::<u64>(), 0, total) };
    }
    let first_seconds = encode_passes(&first)?;
    if listed.read::<u32>(1)[0] > 0 && exact_unset() && !settled(source_key) {
        learn(source_key, &forms, &rows, &segs, &records.read::<u64>(total * 2 * sw), sw);
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
        if let Ok(at) = std::env::var("RERAIL_SERIAL_DEBUG").map(|v| v.parse::<usize>().unwrap_or(0))
            && at > 0
        {
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
            if lanes { pass("lanes_again", &lanes_again, at_round(fit(n)?, 1, rounds - 1), n, GROUP) } else { pass("serial_run", &run, at_round(fit(n)?, 1, rounds - 1), groups(n), THREADS) },
            pass("serial_check", &check, at_round(fit(total)?, 0, rounds), groups(total), THREADS),
        ];
        rerun_seconds += encode_passes(&again)?;
        launches += 1;
    }
    // In lanes form the runs counted their segments: the positions are a prefix and a write.
    let mut sinks = Vec::new();
    if !lanes {
        sinks.push(pass("serial_count", &count, args(fit(total)?, 0), groups(total), THREADS));
    }
    sinks.push(pass("serial_prefix", &prefix, args(0, 0), files.len(), GROUP));
    if lanes {
        sinks.push(pass("lanes_write", &lanes_write, args(fit(total)?, 0), total, GROUP));
    } else {
        sinks.push(pass("serial_write", &write, args(fit(total)?, 0), groups(total), THREADS));
    }
    if forms.iter().any(|n| n.records.is_some()) {
        sinks.push(pass("serial_records", &records_pass, args(fit(files.len())?, 0), groups(files.len() * RECORD_LANES), THREADS));
    }
    let sink_seconds = encode_passes(&sinks)?;
    launches += 1;
    let mut results = Vec::with_capacity(files.len());
    let times = (first_seconds, rerun_seconds, sink_seconds);
    let dropped = hopped.read::<u32>(3)[2] as usize;
    let serial = |files: Vec<BlockRun>| SerialRun { run: BatchRun { files, launches, seconds: times.0 + times.1 + times.2 }, segments: total, reruns, dropped, rounds, first_seconds, rerun_seconds, sink_seconds };
    if !read {
        let placed = rows
            .iter()
            .map(|r| ResidentFile { out_base: r.out_base as usize, blocks: r.groups as usize, pos_base: r.pos_base as usize, capacity: (r.len as usize).max(1), tot_base: r.tot_base as usize })
            .collect();
        return Ok((serial(Vec::new()), Some(Kept { out: owned, tot, positions, files: placed, sources: sources.to_vec() })));
    }
    let words = out.read::<u64>(out_base.max(1));
    let tot = tot.read::<u32>(tot_base.max(1));
    let positions = positions.read::<u32>(pos_base.max(1));
    let refused = refusal.read::<u32>(rows.len() * 6);
    for (i, row) in rows.iter().enumerate() {
        let n = &forms[row.program as usize];
        let len = row.len as usize;
        let blocks = row.groups as usize;
        let outputs: Vec<Vector<u64>> = (0..n.outputs.len())
            .map(|k| {
                let at = row.out_base as usize + k * blocks;
                Vector::from(&words[at..at + len.div_ceil(64)])
            })
            .collect();
        let capacity = len.max(1);
        let count = |c: usize| tot[row.tot_base as usize + c];
        let file_positions = (0..n.extracts.len())
            .map(|e| {
                let from = row.pos_base as usize + e * capacity;
                Vector::from(&positions[from..from + (count(e) as usize).min(capacity)])
            })
            .collect();
        let totals = if n.totals.is_empty() { rerail_blocks::vector::Vector::new() } else { rerail_blocks::vector![u64::from(count(n.extracts.len()) != 0)] };
        let codes = match &n.records {
            Some(r) => {
                let from = row.pos_base as usize + n.extracts.len() * capacity;
                // A refused file's codes to its first token refused (the record pass stops there).
                let stop = refused[i * 6 + 5];
                let k = count(usize::from(r.starts)).min(count(usize::from(r.lasts))).min(if stop == 0 { u32::MAX } else { !stop }) as usize;
                rerail_blocks::vector::Vector::from(&positions[from..from + k.min(capacity)])
            }
            None => rerail_blocks::vector::Vector::new(),
        };
        results.push(BlockRun { outputs, positions: file_positions, totals, codes });
    }
    Ok((serial(results), None))
}
