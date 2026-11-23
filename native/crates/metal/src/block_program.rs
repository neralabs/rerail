//! Block programs on the device (`block`): the code every executor of them there shares, in UKL
//! on the process's device (decision 0440 step C). The executor is the forward carry
//! (`block_serial.rs`, doc 23): a file in segments, the scans made exact level by level by
//! passes and a prefix, the `prev`s guessed and checked; no segment waits on another, in a
//! launch or across launches (decision 0540 step 7: there is no look-back between groups).
//!
//! Its kernels are compiled after `block_common.uglykl`. A group is a SIMD group of the device's
//! width, 16 or 32 lanes (`group_lanes`): elsewhere a batch is refused (`ready`), and the CPU's
//! executors run it. Files are read in place through a file table
//! (`FileRow`), as zero outside them. The instructions are written out straight-line (`emit`),
//! a word of a lane at a time.
#[allow(unused_imports)]
use crate::Error;
use crate::lane;
pub use rerail_compiler::block::BatchFile;
use rerail_compiler::block::BlockRun;
use rerail_compiler::block_native::{BlockInst, BlockNative};
use rerail_device::device::Buffer;

/// The same programs in the CPU's shape: a thread a segment of blocks, the carry forward by
/// speculation (`block_serial.rs`).
#[path = "block_serial.rs"]
pub mod serial;

/// The shared code (`block_common.uglykl`): the file table, the carries and the sinks, for the
/// lane word the source defines in front of it (`head`).
const COMMON: &str = include_str!("block_common.uglykl");
/// A source's head at lane width `width`: its word (`BW`, `BM`) and the shared code. Every source
/// starts with it.
pub fn head(width: LaneWidth) -> String {
    format!("#define BW {}\n#define BM {}\n{COMMON}\n", width.bytes(), width.word())
}

/// The device the batches run on, where they can: a group is one of its SIMD groups, so a device
/// whose groups are not 16 or 32 lanes refuses them (`UnsupportedBackend`), and the CPU runs them.
pub fn ready() -> Result<&'static rerail_device::device::Device, Error> {
    let device = lane::device()?;
    group_of(device).ok_or(Error::UnsupportedBackend)?;
    Ok(device)
}

/// A group's lanes on `device`: its SIMD group, where that is 16 or 32 lanes (`GROUP_LANES` in
/// `block_common.uglykl`: a ballot is a word of the group's lanes, and the borrow's
/// carry-lookahead adds two of them in 64 bits). A threadgroup is one group.
fn group_of(device: &rerail_device::device::Device) -> Option<usize> {
    match device.simd_width() {
        w @ (16 | 32) => Some(w as usize),
        _ => None,
    }
}

/// The lanes of a group on the process's device (`group_of`).
pub fn group_lanes() -> Result<usize, Error> {
    ready().map(|d| group_of(d).expect("a ready device's group"))
}

/// The bytes each lane owns, and so the width of its mask words.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LaneWidth {
    W16,
    W32,
    W64,
}

impl LaneWidth {
    pub const ALL: [LaneWidth; 3] = [LaneWidth::W16, LaneWidth::W32, LaneWidth::W64];

    pub fn bytes(self) -> usize {
        match self {
            LaneWidth::W16 => 16,
            LaneWidth::W32 => 32,
            LaneWidth::W64 => 64,
        }
    }

    /// The C type of a lane word (`BM` in the kernels).
    fn word(self) -> &'static str {
        match self {
            LaneWidth::W16 => "uint16_t",
            LaneWidth::W32 => "uint32_t",
            LaneWidth::W64 => "uint64_t",
        }
    }
}

/// A file of a batch, as the kernels read it (`FileRow` in `batch_helpers`).
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct FileRow {
    src16: u32,
    len: u32,
    program: u32,
    group0: u32,
    groups: u32,
    out_base: u32,
    state_base: u32,
    halo_base: u32,
    ext_base: u32,
    pos_base: u32,
    /// The batch's slab binding the file's bytes are in (`SLAB_BUFFERS`).
    slab: u32,
    /// Where the file's counts start (one per counted output: `block_native::counted`).
    tot_base: u32,
    /// A program in passes (`native_ir::Passes`, the forward carry only): where the file's slot
    /// words start in the output buffer, block `b`'s slot `k` at `slot_base + b * slots + k`.
    slot_base: u32,
}

/// A batch's results: each file's outputs and extract positions, the launches it took, and their
/// GPU seconds.
pub struct BatchRun {
    pub files: Vec<BlockRun>,
    pub launches: usize,
    pub seconds: f64,
}

/// The rows of a batch's slabs (`MAX_SLABS`), in every kernel that reads a file's bytes.
const SLAB_BUFFERS: [&str; crate::slabs::MAX_SLABS] = ["s0", "s1", "s2", "s3"];

/// The lane's eight bit-planes `p0..p7` from its chunks `c0..`: `transpose8` on each 8 bytes,
/// then byte `k` of every group gathered into plane `k` (for 64-byte lanes, an 8x8 byte
/// transpose).
fn planes(width: LaneWidth) -> String {
    let groups = width.bytes() / 8;
    let mut out = String::new();
    for g in 0..groups {
        let (c, half) = (g / 2, if g % 2 == 0 { ("x", "y") } else { ("z", "w") });
        out.push_str(&format!(
            "  uint64_t t{g} = transpose8(U64(c{c}.{}) | (U64(c{c}.{}) << 32u));\n",
            half.0, half.1
        ));
    }
    if groups == 8 {
        for (sh, mask, step) in [(32, 0x0000_0000_FFFF_FFFFu64, 4), (16, 0x0000_FFFF_0000_FFFF, 2), (8, 0x00FF_00FF_00FF_00FF, 1)] {
            for g in (0..8).filter(|g| g & step == 0) {
                let h = g + step;
                out.push_str(&format!(
                    "  {{ const uint64_t a = t{g}, b = t{h}; t{g} = (a & {mask:#018x}ul) | ((b & {mask:#018x}ul) << {sh}u); \
                     t{h} = ((a >> {sh}u) & {mask:#018x}ul) | (b & ~{mask:#018x}ul); }}\n"
                ));
            }
        }
        for k in 0..8 {
            out.push_str(&format!("  const uint64_t p{k} = t{k};\n"));
        }
    } else {
        for k in 0..8 {
            let parts: Vec<String> = (0..groups)
                .map(|g| {
                    let byte = format!("((t{g} >> {}u) & 0xfful)", k * 8);
                    if g == 0 { byte } else { format!("({byte} << {}u)", g * 8) }
                })
                .collect();
            out.push_str(&format!("  const BM p{k} = BMC({});\n", parts.join(" | ")));
        }
    }
    out
}

/// Tables of `m`-bit carry vectors: apply, compose (`a` then `b`), the identity, and the group's
/// exclusive scan (Hillis-Steele over shuffles, a step a doubling) with the group's total. The 1-bit
/// tables are `block_common.uglykl`'s (`tapply1`, ...); this writes the others.
fn table_helpers(m: usize) -> String {
    assert!(m > 1, "the 1-bit tables are the common code's");
    let mask = (1u32 << m) - 1;
    let n = 1usize << m;
    let compose: Vec<String> = (0..n).map(|c| format!("(tapply{m}(b, tapply{m}(a, {c}u)) << {}u)", c * m)).collect();
    let identity: u32 = (0..n as u32).map(|c| c << (c as usize * m)).sum();
    let step = |d: u32| format!("\x20 u = ukl_sg_shuffle_up(x, {d}u __UKL_ARGS); x = SEL(x, tcompose{m}(u, x), l >= {d}u);\n");
    let steps: String = [1, 2, 4, 8].into_iter().map(step).collect::<String>() + "#if GROUP_LANES > 16\n" + &step(16) + "#endif\n";
    format!(
        "UKL_INLINE uint32_t tapply{m}(uint32_t t, uint32_t c) {{ return (t >> (c * {m}u)) & {mask}u; }}\n\
         UKL_INLINE uint32_t tcompose{m}(uint32_t a, uint32_t b) {{ return {}; }}\n\
         #define TID{m} {identity}u\n\
         UKL_INLINE uint32_t texclusive{m}(uint32_t t, uint32_t l, UKL_PRIVATE uint32_t* total __UKL_PARAMS) {{\n\
         \x20 uint32_t x = t, u;\n{steps}\
         \x20 *total = ukl_sg_shuffle(x, GROUP_LANES - 1u __UKL_ARGS);\n\
         \x20 u = ukl_sg_shuffle_up(x, 1u __UKL_ARGS);\n\
         \x20 return SEL(u, TID{m}, l == 0u);\n}}\n\n",
        compose.join(" | ")
    )
}

/// A program's form in a full group: one past the file's first and inside the file, so that
/// every `within` is all ones and the values the CPU's known bits find all zeros or all ones in
/// a full block (`lane_plan::Plan::full`) are constants. Only those two: a lane word is
/// narrower than a block, and a constant of both kinds is the same at every width. A carry
/// (`toggle`, `escaped`, `fill`) is never folded: its carry is the same in every body.
fn full_form(native: &BlockNative) -> BlockNative {
    let plan = rerail_compiler::lane_plan::Plan::of(native);
    let mut form = native.clone();
    let members = group_members(native);
    for (at, inst) in form.insts.iter_mut().enumerate() {
        if members[at] {
            continue;
        }
        let folded = match (*inst, plan.full[at]) {
            (BlockInst::Toggle(_) | BlockInst::Escaped(_) | BlockInst::Fill(..), _) => None,
            (BlockInst::Within(_), _) => Some(u64::MAX),
            (_, Some(c)) if c == 0 || c == u64::MAX => Some(c),
            _ => None,
        };
        if let Some(c) = folded {
            *inst = BlockInst::Const(c);
        }
    }
    form
}

/// Whether each instruction is a fixed point's member: its value is the loop's, never folded.
fn group_members(native: &BlockNative) -> Vec<bool> {
    let mut members = vec![false; native.insts.len()];
    for g in &native.groups {
        for &m in &g.members {
            members[m] = true;
        }
    }
    members
}

/// A program's form in a full group of ASCII bytes: `full_form`, with each byte-local value known
/// on ASCII bytes (`lane_plan::Plan::ascii`) a constant: a UTF-8 check's, say.
fn ascii_form(native: &BlockNative) -> BlockNative {
    let plan = rerail_compiler::lane_plan::Plan::of(native);
    let mut form = full_form(native);
    let members = group_members(native);
    for (at, inst) in form.insts.iter_mut().enumerate() {
        if inst.carried() || !plan.lanes[at] || members[at] {
            continue;
        }
        if let Some(one) = plan.ascii[at] {
            *inst = BlockInst::Const(if one { u64::MAX } else { 0 });
        }
    }
    form
}

/// Where `emit`'s word is: its first byte's chunk `base` (16-byte chunks) and stream byte `offset`
/// (for `const`'s phase), and whose it is. An empty `base`: the word's chunks `c0..` and the byte
/// before them, `before`, are loaded already (the serial loop loads a block ahead). `lanes`: the
/// SIMD group's lanes hold one stream's words, so a fixed point's rounds go on while any lane's
/// change (a vote); else each thread runs its own stream, and its own rounds.
pub(crate) struct Word<'a> {
    pub base: &'a str,
    pub offset: &'a str,
    pub lanes: bool,
}

/// The instructions in `chosen`, as straight-line statements for a word of `width` at `word`.
/// `carried` writes each carried instruction's statements.
fn emit(p: usize, native: &BlockNative, width: LaneWidth, chosen: &[usize], word: Word<'_>, mut carried: impl FnMut(usize, &BlockInst, &mut String)) -> String {
    let Word { base, offset, lanes } = word;
    let chunks = width.bytes() / 16;
    let bits = width.bytes();
    let insts: Vec<&BlockInst> = chosen.iter().map(|&at| &native.insts[at]).collect();
    let mut out = String::new();
    let back = insts.iter().any(|i| matches!(i, BlockInst::Pair { .. } | BlockInst::PlaneBefore(_)));
    let loaded = base.is_empty();
    if !loaded && (back || insts.iter().any(|i| matches!(i, BlockInst::EqByte(_) | BlockInst::Class { .. } | BlockInst::Plane(_) | BlockInst::Prefix(..)))) {
        for c in 0..chunks {
            out.push_str(&format!("  const B4 c{c} = load16(bytes, f, {base} + {c}u);\n"));
        }
    }
    // The byte before the word, read from the source (no carry); and, for `pair` by lookups,
    // the word's bytes one back.
    if back && !loaded {
        out.push_str(&format!("  const uint32_t before = file_byte_before(bytes, f, {base});\n"));
    }
    let used: std::collections::BTreeSet<u16> = insts
        .iter()
        .filter_map(|i| match **i {
            BlockInst::Class { table, .. } | BlockInst::Pair { table, .. } => Some(table),
            _ => None,
        })
        .collect();
    for t in used {
        for c in 0..chunks {
            out.push_str(&format!("  const B4 k{t}_{c} = classes_p{p}_t{t}(c{c});\n"));
        }
    }
    let used_before: std::collections::BTreeSet<u16> =
        insts.iter().filter_map(|i| if let BlockInst::Pair { before, .. } = **i { Some(before) } else { None }).collect();
    if !used_before.is_empty() {
        for c in 0..chunks {
            let prior = if c == 0 { "before".to_owned() } else { format!("(c{}.w >> 24u)", c - 1) };
            out.push_str(&format!("  const B4 b{c} = bytes_before(c{c}, {prior});\n"));
        }
    }
    for t in used_before {
        for c in 0..chunks {
            out.push_str(&format!("  const B4 kb{t}_{c} = classes_p{p}_t{t}(b{c});\n"));
        }
    }
    if insts.iter().any(|i| matches!(i, BlockInst::Plane(_) | BlockInst::PlaneBefore(_))) {
        out.push_str(&planes(width));
    }
    // Each instruction's statements, into `out`.
    let mut one = |at: usize, out: &mut String| {
        let inst = &native.insts[at];
        if inst.carried() {
            carried(at, inst, out);
            return;
        }
        let rhs = match *inst {
            BlockInst::Cmp { .. } | BlockInst::Bit { .. } => unreachable!("a row program's, never a byte program's"),
            BlockInst::Class { table, bits: sel } => {
                let s4 = u32::from(sel) * 0x0101_0101;
                (0..chunks)
                    .map(|c| {
                        let part = format!("BMC(nz16(b4_ands(k{table}_{c}, {s4:#010x}u)))");
                        if c == 0 { part } else { format!("({part} << {}u)", c * 16) }
                    })
                    .collect::<Vec<_>>()
                    .join(" | ")
            }
            BlockInst::Plane(k) => format!("p{k}"),
            BlockInst::PlaneBefore(k) => format!("BMC((BMC(p{k}) << 1u) | BMC((before >> {k}u) & 1u))"),
            BlockInst::Pair { before, table, bits: sel } => {
                let s4 = u32::from(sel) * 0x0101_0101;
                (0..chunks)
                    .map(|c| {
                        let part = format!("BMC(nz16(b4_ands(b4_and(kb{before}_{c}, k{table}_{c}), {s4:#010x}u)))");
                        if c == 0 { part } else { format!("({part} << {}u)", c * 16) }
                    })
                    .collect::<Vec<_>>()
                    .join(" | ")
            }
            BlockInst::EqByte(byte) => {
                let c4 = u32::from(byte) * 0x0101_0101;
                (0..chunks)
                    .map(|c| {
                        let part = format!("BMC(eq16(c{c}, {c4:#010x}u))");
                        if c == 0 { part } else { format!("({part} << {}u)", c * 16) }
                    })
                    .collect::<Vec<_>>()
                    .join(" | ")
            }
            // The pattern's bits from the word's first byte on: a right rotation by its phase.
            BlockInst::Const(c) => format!("BMC(rotr64({c:#018x}ul, U64(({offset}) & 63u)))"),
            BlockInst::Not(a) => format!("BMC(~v{a})"),
            BlockInst::And(a, b) => format!("v{a} & v{b}"),
            BlockInst::Or(a, b) => format!("v{a} | v{b}"),
            BlockInst::Xor(a, b) => format!("v{a} ^ v{b}"),
            BlockInst::AndNot(a, b) => format!("BMC(v{a} & ~v{b})"),
            // The bits of the word's bytes below the stream's end.
            BlockInst::Within(0) => format!("within_lane(len, {offset})"),
            BlockInst::Within(d) => format!("BMC(within_lane(len + {d}u, {offset}) & ~within_lane({d}u, {offset}))"),
            // The period's bits shifted up to the word's first byte of the residue.
            BlockInst::Phase(k, r) => {
                let bits = rerail_compiler::block::period_bits(k);
                format!("BMC({bits:#018x}ul << (({r}u + {k}u - (({offset}) % {k}u)) % {k}u))")
            }
            BlockInst::Prev(..) | BlockInst::Toggle(_) | BlockInst::Escaped(_) | BlockInst::Fill(..) => unreachable!(),
            // Bit j of each byte's place in its file: below 6 a pattern of period 2^(j+1), from
            // the word's first byte (a lane's bytes start at a multiple of its width); from 6 on
            // the same in all of a word's bytes, at most 64 of them.
            BlockInst::Position(j) if j < 6 => {
                let pattern = (0..64u32).filter(|i| i >> j & 1 == 1).fold(0u64, |a, i| a | 1 << i);
                format!("BMC({pattern:#018x}ul >> U64(({offset}) & 63u))")
            }
            BlockInst::Position(j) => format!("BMC(0) - BMC((U32({offset}) >> {j}u) & 1u)"),
            BlockInst::Carry(_) => unreachable!("a carry is its group's loop's (`emit`)"),
            // `a >> k` in the word (decision 0330): `a` has no bits past a file's first
            // `EARLY_BITS`, which lie in its first word at every width.
            BlockInst::Early(a, k) => format!("BMC(v{a} >> {k}u)"),
            // The low `n` bits of the file's first word where its first `n` bytes are `w`'s
            // (decision 0335): the word's first chunk, compared once.
            BlockInst::Prefix(n, w) => {
                let cut = if n >= 8 { u64::MAX } else { (1u64 << (8 * n)) - 1 };
                format!("(({offset}) == 0u && len >= {n}u && ((U64(c0.x) | (U64(c0.y) << 32u)) & {cut:#x}ul) == {w:#x}ul) ? BMC({:#x}ul) : BMC(0)", (1u64 << n) - 1)
            }
            BlockInst::Walk { .. } | BlockInst::Args(..) => unreachable!("the CPU's form only: Metal runs a walk's rounds"),
            // A slot word of a pass before (`native_ir::Passes`): the forward carry's, a block a
            // word, in the output buffer `out`.
            BlockInst::Load(k) => format!("out[f.slot_base + ({offset}) / 64u * SLOTS{p} + {k}u]"),
            BlockInst::BackFill(..) => unreachable!("a backward pass's (`metal_block_serial::back_kernels`)"),
        };
        out.push_str(&format!("  const BM v{at} = {rhs};\n"));
    };
    // The order: `chosen`'s, each fixed point (`rounds until settled`) one step where its members
    // can all run (`schedule`).
    for step in schedule(native, chosen) {
        match step {
            Step::One(at) => one(at, &mut out),
            Step::Group(g) => group_loop(native, g, lanes, &mut |at, out| one(at, out), &mut out),
        }
    }
    let _ = bits;
    out
}

/// A step of `emit`: an instruction, or a fixed point's group (`BlockNative::groups`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    One(usize),
    Group(usize),
}

/// `chosen` in an order its values can be made in: each instruction once its operands are, the
/// first ready first; a group once every operand of its members outside it is, its members
/// together. As Cranelift places a group (`native_cpu_block`).
fn schedule(native: &BlockNative, chosen: &[usize]) -> Vec<Step> {
    let mut group_of = vec![None; native.insts.len()];
    for (g, group) in native.groups.iter().enumerate() {
        for &m in &group.members {
            group_of[m] = Some(g);
        }
    }
    if chosen.iter().all(|&at| group_of[at].is_none()) {
        return chosen.iter().map(|&at| Step::One(at)).collect();
    }
    let wanted: std::collections::BTreeSet<usize> = chosen.iter().copied().collect();
    // A value not chosen is made already (an earlier level's), or not at all.
    let mut done: Vec<bool> = (0..native.insts.len()).map(|at| !wanted.contains(&at)).collect();
    let mut ran = vec![false; native.groups.len()];
    let mut out = Vec::with_capacity(chosen.len());
    loop {
        let mut progress = false;
        for &at in chosen {
            if !done[at] && group_of[at].is_none() && native.insts[at].operands().iter().all(|&o| done[o]) {
                out.push(Step::One(at));
                done[at] = true;
                progress = true;
            }
        }
        for (g, group) in native.groups.iter().enumerate() {
            if !ran[g] && group.members.iter().all(|&m| wanted.contains(&m)) && group.members.iter().all(|&m| native.insts[m].operands().iter().all(|&o| done[o] || group_of[o] == Some(g))) {
                out.push(Step::Group(g));
                for &m in &group.members {
                    done[m] = true;
                }
                ran[g] = true;
                progress = true;
            }
        }
        if !progress {
            break;
        }
    }
    if let Some(&at) = chosen.iter().find(|&&at| !done[at]) {
        let why: Vec<String> = native.insts[at].operands().iter().filter(|&&o| !done[o]).map(|&o| format!("%{o} {:?} (group {:?}, chosen {})", native.insts[o], group_of[o], wanted.contains(&o))).collect();
        let blocked: Vec<String> = native
            .groups
            .iter()
            .enumerate()
            .filter(|&(g, _)| !ran[g])
            .flat_map(|(g, group)| group.members.iter().flat_map(move |&m| native.insts[m].operands().into_iter().map(move |o| (g, m, o))).collect::<Vec<_>>())
            .filter(|&(g, _, o)| !done[o] && group_of[o] != Some(g))
            .map(|(g, m, o)| format!("group {g}: %BM {:?} reads %{o} {:?} (group {:?}, chosen {})", native.insts[m], native.insts[o], group_of[o], wanted.contains(&o)))
            .collect();
        panic!("%{at} {:?} (group {:?}) is never made: waits on {why:?}; {blocked:?}", native.insts[at], group_of[at]);
    }
    out
}

/// Group `g`'s loop (`rounds until settled`): its members made again from its carries until no
/// carry changes in any thread of the SIMD group (`lanes`; else in the thread), at most `MAX_ROUNDS` rounds a block of the
/// group. Every member is a variable before the loop, so the words after it read the last
/// round's; the carried members' outs too (`o`, `co`), read by the block's tail. A thread that
/// has settled makes the same values again: a fixed point.
fn group_loop(native: &BlockNative, g: usize, lanes: bool, one: &mut dyn FnMut(usize, &mut String), out: &mut String) {
    let group = &native.groups[g];
    for &x in &group.members {
        out.push_str(&format!("  BM v{x} = BMC(0);\n"));
        if matches!(native.insts[x], BlockInst::Toggle(_) | BlockInst::Escaped(_) | BlockInst::Fill(..)) {
            out.push_str(&format!("  uint32_t o{x} = 0u, co{x} = 0u;\n"));
        }
    }
    for (k, &(c, _)) in group.carries.iter().enumerate() {
        let BlockInst::Carry(init) = native.insts[c] else { unreachable!("a carry") };
        out.push_str(&format!("  BM gc{g}_{k} = v{init};\n"));
    }
    let rounds = rerail_compiler::block::MAX_ROUNDS;
    out.push_str(&format!("  for (uint32_t r{g} = 0u; r{g} < {rounds}u * GROUP_LANES; r{g}++) {{\n"));
    for (k, &(c, _)) in group.carries.iter().enumerate() {
        out.push_str(&format!("    v{c} = gc{g}_{k};\n"));
    }
    for &x in &group.members {
        if matches!(native.insts[x], BlockInst::Carry(_)) {
            continue;
        }
        let mut text = String::new();
        one(x, &mut text);
        // The member's own declarations become assignments to the variables above.
        let text = text
            .replace(&format!("const BM v{x} = "), &format!("v{x} = "))
            .replace(&format!("const uint64_t v{x} = "), &format!("v{x} = "))
            .replace(&format!("uint32_t o{x} = 0u;"), &format!("o{x} = 0u;"))
            .replace(&format!("const uint32_t co{x} = "), &format!("co{x} = "));
        for line in text.lines() {
            out.push_str(&format!("  {line}\n"));
        }
    }
    let changed: Vec<String> = group.carries.iter().enumerate().map(|(k, &(_, x))| format!("v{x} != gc{g}_{k}")).collect();
    out.push_str(&format!("    const bool more{g} = {};\n", changed.join(" || ")));
    for (k, &(_, x)) in group.carries.iter().enumerate() {
        out.push_str(&format!("    gc{g}_{k} = v{x};\n"));
    }
    // A vote is reached by every lane or none: in the serial form, whose threads run segments of
    // their own, it is the thread's own.
    let any = if lanes { format!("sg_any(more{g} __UKL_ARGS)") } else { format!("more{g}") };
    out.push_str(&format!("    if (!{any}) break;\n  }}\n"));
}

/// One launch: its kernel, its buffers and parameters (four words) by row name (a kernel binds
/// those of its rows it declares, as Metal's kernels took the buffer indices they declared), and
/// the grid as threadgroups of `threads`.
struct Pass {
    name: &'static str,
    kernel: lane::Kernel,
    buffers: Vec<(&'static str, Buffer)>,
    args: Vec<(&'static str, [u32; 4])>,
    groups: usize,
    threads: usize,
}

/// The passes in order, in one command; returns its device seconds.
fn encode_passes(passes: &[Pass]) -> Result<f64, Error> {
    let _scope = crate::launch::LaunchScope::enter();
    let mut command = lane::Command::new("Rerail block program");
    for pass in passes {
        let mut launch = command.launch(&pass.kernel, pass.name);
        for (name, buffer) in &pass.buffers {
            launch = launch.buffer_if(name, buffer);
        }
        for (name, value) in &pass.args {
            launch = launch.value_if(name, value);
        }
        launch.groups([pass.groups as u64, 1, 1], [pass.threads as u64, 1, 1]);
    }
    let times = command.run().inspect_err(|e| eprintln!("block program: {e:?}"))?;
    Ok(times.device)
}
