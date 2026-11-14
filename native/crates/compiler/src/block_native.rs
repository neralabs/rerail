//! The block programs' native IR: a block program (`block::BlockProgram`) lowered for the code
//! generators, one straight-line body of mask words per 64-byte block, and its passes.

/// A block program lowered for the code generators: the ephemeral native IR that Cranelift
/// (`native_cpu_block`) and MSL (`metal_block_program`) both emit, one straight-line body per
/// 64-byte block. Each instruction is one value, a mask word, built only from bit operations;
/// there is no branch and no loop inside a body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockNative {
    pub tables: Vec<crate::block::ClassTable>,
    pub insts: Vec<BlockInst>,
    /// The values written as masks. Output `k` is value `outputs[k]` at delay
    /// `output_delays[k]`: every backend writes the stream's mask, bit `p` the value's bit
    /// `p + d`, cut at the stream's end. No host step shifts it.
    pub outputs: Vec<u16>,
    /// The outputs (indexes into `outputs`) whose positions the extract pass writes: each set
    /// bit of the stream's mask as its byte position, in order, at the offset of the bits before
    /// it (a running count, carried from block to block).
    pub extracts: Vec<usize>,
    /// The outputs (indexes into `outputs`) tested: each file's flag, whether the stream's mask
    /// has a bit (`block::BlockProgram::totals`).
    pub totals: Vec<usize>,
    /// The outputs (indexes into `outputs`) tier 2 reads as masks (`block::BlockProgram::masks`).
    pub masks: Vec<usize>,
    /// How many bytes past the stream the program runs (`block::BlockProgram::delay`).
    pub delay: u8,
    /// Each output's delay (`block::BlockProgram::output_delay`).
    pub output_delays: Vec<u8>,
    /// The fixed points (`block::BlockProgram::rounds`): each group's members and its carries
    /// with their nexts.
    pub groups: Vec<crate::block::Group>,
    /// The token records (`block::BlockProgram::records`): the record pass after the passes.
    pub records: Option<crate::records::Records>,
    /// The groups that are regions (`block::BlockProgram::walks`): Metal runs their rounds; the
    /// CPU walks them (`cpu_form`).
    pub walks: Vec<crate::block::Walk>,
}

/// An instruction of a block body.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BlockInst {
    /// The block's bytes compared with a byte, as a mask: on the CPU four 16-byte compares and a
    /// bitmask; on Metal SWAR over the lane's `uint4`s.
    EqByte(u8),
    /// The block's bytes in any class of `bits` of table `table`: on the CPU two table lookups
    /// per 16 bytes (`tbl`); on Metal, see `metal_block_program::ClassStrategy`.
    Class { table: u16, bits: u8 },
    /// Bit `k` of every byte of the block: one bit-plane (`BlockNative::planes_form`).
    Plane(u8),
    /// The bytes whose byte before is in `before`'s classes and which are in `table`'s, for a
    /// shared class of `bits` (`block::BlockOp::Pair`): on the CPU the class bytes of the block
    /// loaded one byte back, ANDed with the block's own, then one test.
    Pair { before: u16, table: u16, bits: u8 },
    /// Bit `k` of every byte's byte before (0x00 before the stream): one bit-plane of the
    /// source one byte back, for `pair` in the planes form. Local: the byte before a lane is read
    /// from the source.
    PlaneBefore(u8),
    /// A 64-bit pattern repeated over the stream.
    Const(u64),
    Not(u16),
    And(u16, u16),
    Or(u16, u16),
    Xor(u16, u16),
    AndNot(u16, u16),
    /// The carried operations (`block::carry`): on the CPU the carry passes in a register from
    /// block to block; on Metal by a scan of lane summaries (`metal_block_program`).
    Prev(u16, u8),
    Toggle(u16),
    Escaped(u16),
    /// `fill(s, e)` (`block::BlockOp::Fill`): one add per word, its carry a borrow.
    Fill(u16, u16),
    /// `back_fill(s, e)` (`block::BlockOp::BackFill`): `fill` over the bits reversed, its carry
    /// from the next block. Never in a body: a pass of its own, last block to first (`passes`).
    BackFill(u16, u16),
    /// The word a pass before stored in slot `k` for the block (`passes`): a value of the
    /// program made in an earlier pass, as that pass made it.
    Load(u16),
    /// The bytes inside the stream at a delay (`block::BlockOp::Within`), from its length.
    Within(u8),
    /// The bytes whose position is `r` mod `k` (`block::BlockOp::Phase`), from the position.
    Phase(u8, u8),
    /// Plane `j` of each byte's position (`block::BlockOp::Position`), from the position.
    Position(u8),
    /// A fixed point's carry (`block::BlockOp::Carry`): its initial value's word in a block's
    /// first round, its next's after.
    Carry(u16),
    /// A row program's comparison (`block::BlockOp::Cmp`): 64 rows' lanes compared, one bit
    /// each.
    Cmp { col: u16, op: crate::block::CmpOp, rhs: crate::block::Rhs, width: u8 },
    /// A row program's set test (`block::BlockOp::Bit`): 64 rows' values looked up in a set.
    Bit { set: u16, col: u16 },
    /// Value `out` of walk `walk` (`block::Walk::made`), on the CPU (`cpu_form`): the walk runs
    /// once a block, a call, when the first of its values is needed. `args` is its inputs.
    Walk { walk: u16, out: u8, args: u16 },
    /// Two of a walk's inputs, or an input and the inputs before it: what `Walk` reads, so that
    /// every pass orders and keeps them as it does any operand. No word of its own.
    Args(u16, u16),
}

impl BlockInst {
    /// Whether the instruction carries across blocks.
    pub fn carried(&self) -> bool {
        matches!(self, BlockInst::Prev(..) | BlockInst::Toggle(_) | BlockInst::Escaped(_) | BlockInst::Fill(..) | BlockInst::BackFill(..) | BlockInst::Walk { .. })
    }

    /// The values the instruction reads.
    pub fn operands(&self) -> Vec<usize> {
        match *self {
            BlockInst::Not(a) | BlockInst::Prev(a, _) | BlockInst::Toggle(a) | BlockInst::Escaped(a) | BlockInst::Carry(a) | BlockInst::Walk { args: a, .. } => vec![usize::from(a)],
            BlockInst::And(a, b) | BlockInst::Or(a, b) | BlockInst::Xor(a, b) | BlockInst::AndNot(a, b) | BlockInst::Fill(a, b) | BlockInst::BackFill(a, b) | BlockInst::Args(a, b) => {
                vec![usize::from(a), usize::from(b)]
            }
            _ => vec![],
        }
    }
}

impl BlockNative {
    pub fn lower(program: &crate::block::BlockProgram) -> Self {
        use crate::block::BlockOp;
        let insts = program
            .ops
            .iter()
            .map(|op| match *op {
                BlockOp::Eq(byte) => BlockInst::EqByte(byte),
                BlockOp::Class { table, bits } => BlockInst::Class { table, bits },
                BlockOp::Pair { before, table, bits } => BlockInst::Pair { before, table, bits },
                BlockOp::Const(c) => BlockInst::Const(c),
                BlockOp::Not(a) => BlockInst::Not(a),
                BlockOp::And(a, b) => BlockInst::And(a, b),
                BlockOp::Or(a, b) => BlockInst::Or(a, b),
                BlockOp::Xor(a, b) => BlockInst::Xor(a, b),
                BlockOp::AndNot(a, b) => BlockInst::AndNot(a, b),
                BlockOp::Prev(a, k) => BlockInst::Prev(a, k),
                BlockOp::Toggle(a) => BlockInst::Toggle(a),
                BlockOp::Escaped(a) => BlockInst::Escaped(a),
                BlockOp::Fill(a, b) => BlockInst::Fill(a, b),
                BlockOp::BackFill(a, b) => BlockInst::BackFill(a, b),
                BlockOp::Within(d) => BlockInst::Within(d),
                BlockOp::Phase(k, r) => BlockInst::Phase(k, r),
                BlockOp::Position(j) => BlockInst::Position(j),
                BlockOp::Carry(a) => BlockInst::Carry(a),
                BlockOp::Cmp { col, op, rhs, width } => BlockInst::Cmp { col, op, rhs, width },
                BlockOp::Bit { set, col } => BlockInst::Bit { set, col },
            })
            .collect();
        Self {
            tables: program.tables.clone(),
            insts,
            outputs: program.outputs.clone(),
            extracts: program.extract_outputs(),
            totals: program.total_outputs(),
            masks: program.mask_outputs(),
            delay: program.delay,
            output_delays: (0..program.outputs.len()).map(|k| program.output_delay(k)).collect(),
            groups: crate::block::groups(&program.ops, &program.rounds).expect("a verified program"),
            records: program.records.clone(),
            walks: program.walks.clone(),
        }
    }

    /// Each instruction with its operands renamed by `m`.
    pub fn renamed(inst: BlockInst, m: impl Fn(u16) -> u16) -> BlockInst {
        match inst {
            BlockInst::Not(a) => BlockInst::Not(m(a)),
            BlockInst::And(a, b) => BlockInst::And(m(a), m(b)),
            BlockInst::Or(a, b) => BlockInst::Or(m(a), m(b)),
            BlockInst::Xor(a, b) => BlockInst::Xor(m(a), m(b)),
            BlockInst::AndNot(a, b) => BlockInst::AndNot(m(a), m(b)),
            BlockInst::Prev(a, k) => BlockInst::Prev(m(a), k),
            BlockInst::Toggle(a) => BlockInst::Toggle(m(a)),
            BlockInst::Escaped(a) => BlockInst::Escaped(m(a)),
            BlockInst::Fill(a, b) => BlockInst::Fill(m(a), m(b)),
            BlockInst::BackFill(a, b) => BlockInst::BackFill(m(a), m(b)),
            BlockInst::Carry(a) => BlockInst::Carry(m(a)),
            BlockInst::Walk { walk, out, args } => BlockInst::Walk { walk, out, args: m(args) },
            BlockInst::Args(a, b) => BlockInst::Args(m(a), m(b)),
            other => other,
        }
    }

    /// A walk with its values renamed by `m`.
    fn walk_renamed(w: &crate::block::Walk, m: impl Fn(u16) -> u16) -> crate::block::Walk {
        let kinds = w.kinds.iter().map(|k| crate::block::WalkKind { opener: m(k.opener), closer: m(k.closer), open: m(k.open), fill: m(k.fill), empty: k.empty.map(|(e, n)| (m(e), n)), ..*k }).collect();
        crate::block::Walk { group: w.group, hidden: m(w.hidden), kinds }
    }

    /// The CPU's form (decision 0139): each walk's group made by the walk, not by rounds. The
    /// values the walk makes become its `Walk`s, reading its inputs through `Args`; the group
    /// goes, and with it whatever only the rounds read (an `else`'s `back_fill`, and the
    /// backward pass it took). Metal runs the program as it is.
    pub fn cpu_form(&self) -> BlockNative {
        if self.walks.is_empty() {
            return self.clone();
        }
        let mut insts = self.insts.clone();
        let mut walked = vec![false; self.groups.len()];
        for (w, walk) in self.walks.iter().enumerate() {
            walked[usize::from(walk.group)] = true;
            // Each `Args` reads the one before it and one input more.
            let reads = walk.reads();
            let mut args = reads[0];
            for &r in &reads[1..] {
                insts.push(BlockInst::Args(args, r));
                args = (insts.len() - 1) as u16;
            }
            for (out, v) in walk.made().into_iter().enumerate() {
                insts[usize::from(v)] = BlockInst::Walk { walk: w as u16, out: out as u8, args };
            }
        }
        // A `Walk` now reads a later `Args`: the program is put back in order with the `Args`
        // where its walk's first value was, then pruned to what the outputs read.
        let n = self.insts.len();
        let mut order: Vec<usize> = Vec::with_capacity(insts.len());
        let mut placed = vec![false; insts.len()];
        fn place(x: usize, insts: &[BlockInst], placed: &mut [bool], order: &mut Vec<usize>) {
            if placed[x] {
                return;
            }
            placed[x] = true;
            for o in insts[x].operands() {
                place(o, insts, placed, order);
            }
            order.push(x);
        }
        let mut keep = vec![false; insts.len()];
        let mut todo: Vec<usize> = self.outputs.iter().map(|&v| usize::from(v)).collect();
        for (_, group) in self.groups.iter().enumerate().filter(|(g, _)| !walked[*g]) {
            todo.extend(group.carries.iter().flat_map(|&(c, x)| [c, x]));
        }
        while let Some(x) = todo.pop() {
            if !std::mem::replace(&mut keep[x], true) {
                todo.extend(insts[x].operands());
                // A kept group's carry reads its next, a round later.
                for group in self.groups.iter().enumerate().filter(|(g, _)| !walked[*g]).map(|(_, g)| g) {
                    if let Some(&(_, next)) = group.carries.iter().find(|p| p.0 == x) {
                        todo.push(next);
                    }
                }
            }
        }
        for x in 0..insts.len() {
            if keep[x] && x < n {
                place(x, &insts, &mut placed, &mut order);
            }
        }
        let mut number = vec![u16::MAX; insts.len()];
        for (k, &x) in order.iter().enumerate() {
            number[x] = k as u16;
        }
        let m = |v: u16| number[usize::from(v)];
        let place_of = |x: usize| usize::from(number[x]);
        BlockNative {
            tables: self.tables.clone(),
            insts: order.iter().map(|&x| Self::renamed(insts[x], m)).collect(),
            outputs: self.outputs.iter().map(|&v| m(v)).collect(),
            extracts: self.extracts.clone(),
            totals: self.totals.clone(),
            masks: self.masks.clone(),
            delay: self.delay,
            output_delays: self.output_delays.clone(),
            records: self.records.clone(),
            groups: self
                .groups
                .iter()
                .enumerate()
                .filter(|(g, _)| !walked[*g])
                .map(|(_, g)| {
                    let mut members: Vec<usize> = g.members.iter().filter(|&&x| keep[x]).map(|&x| place_of(x)).collect();
                    members.sort_unstable();
                    crate::block::Group { members, carries: g.carries.iter().map(|&(c, x)| (place_of(c), place_of(x))).collect() }
                })
                .collect(),
            walks: self.walks.iter().map(|w| Self::walk_renamed(w, m)).collect(),
        }
    }

    /// The same program with every `class` written as a boolean formula of the eight bit-planes
    /// (`Plane`): an OR over the selected class bits of a function of the low four planes and one
    /// of the high four, each by Shannon expansion on its highest plane, constants folded and
    /// common terms shared. Metal runs this form (it measured faster than table lookups); the
    /// CPU keeps `class`, which is two `tbl`s per 16 bytes there.
    pub fn planes_form(&self) -> BlockNative {
        let mut f = PlaneFormulas::default();
        let mut at: Vec<u16> = Vec::with_capacity(self.insts.len());
        for inst in &self.insts {
            let m = |x: u16| at[usize::from(x)];
            let index = match *inst {
                BlockInst::Class { table, bits } => {
                    let t = self.tables[usize::from(table)];
                    let mut word = Term::Zero;
                    for c in (0..8).filter(|c| bits >> c & 1 == 1) {
                        let lo = (0..16).fold(0u16, |a, n| a | (u16::from(t.lo[n] >> c & 1) << n));
                        let hi = (0..16).fold(0u16, |a, n| a | (u16::from(t.hi[n] >> c & 1) << n));
                        let l = f.function(0, 4, lo);
                        let h = f.function(4, 4, hi);
                        let both = f.and(l, h);
                        word = f.or(word, both);
                    }
                    f.index(word)
                }
                // The same with the byte before's planes (8..16) for `before`'s functions.
                BlockInst::Pair { before, table, bits } => {
                    let (p, t) = (self.tables[usize::from(before)], self.tables[usize::from(table)]);
                    let half = |n: &[u8; 16], c: usize| (0..16).fold(0u16, |a, i| a | (u16::from(n[i] >> c & 1) << i));
                    let mut word = Term::Zero;
                    for c in (0..8).filter(|c| bits >> c & 1 == 1) {
                        let pl = f.function(8, 4, half(&p.lo, c));
                        let ph = f.function(12, 4, half(&p.hi, c));
                        let l = f.function(0, 4, half(&t.lo, c));
                        let h = f.function(4, 4, half(&t.hi, c));
                        let a = f.and(pl, ph);
                        let b = f.and(l, h);
                        let both = f.and(a, b);
                        word = f.or(word, both);
                    }
                    f.index(word)
                }
                BlockInst::Not(a) => f.push(BlockInst::Not(m(a))),
                BlockInst::And(a, b) => f.push(BlockInst::And(m(a), m(b))),
                BlockInst::Or(a, b) => f.push(BlockInst::Or(m(a), m(b))),
                BlockInst::Xor(a, b) => f.push(BlockInst::Xor(m(a), m(b))),
                BlockInst::AndNot(a, b) => f.push(BlockInst::AndNot(m(a), m(b))),
                BlockInst::Prev(a, k) => f.push(BlockInst::Prev(m(a), k)),
                BlockInst::Toggle(a) => f.push(BlockInst::Toggle(m(a))),
                BlockInst::Escaped(a) => f.push(BlockInst::Escaped(m(a))),
                BlockInst::Fill(a, b) => f.push(BlockInst::Fill(m(a), m(b))),
                BlockInst::BackFill(a, b) => f.push(BlockInst::BackFill(m(a), m(b))),
                BlockInst::Carry(a) => f.push(BlockInst::Carry(m(a))),
                BlockInst::Walk { walk, out, args } => f.push(BlockInst::Walk { walk, out, args: m(args) }),
                BlockInst::Args(a, b) => f.push(BlockInst::Args(m(a), m(b))),
                other => f.push(other),
            };
            at.push(index);
        }
        let place = |x: usize| usize::from(at[x]);
        BlockNative {
            tables: self.tables.clone(),
            insts: f.insts,
            outputs: self.outputs.iter().map(|&v| at[usize::from(v)]).collect(),
            extracts: self.extracts.clone(),
            totals: self.totals.clone(),
            masks: self.masks.clone(),
            delay: self.delay,
            output_delays: self.output_delays.clone(),
            records: self.records.clone(),
            walks: self.walks.iter().map(|w| Self::walk_renamed(w, |v| at[usize::from(v)])).collect(),
            groups: self
                .groups
                .iter()
                .map(|g| {
                    // Two members alike are one value now: listed once.
                    let mut members: Vec<usize> = g.members.iter().map(|&x| place(x)).collect();
                    members.sort_unstable();
                    members.dedup();
                    crate::block::Group { members, carries: g.carries.iter().map(|&(c, x)| (place(c), place(x))).collect() }
                })
                .collect(),
        }
    }
}

/// A pass over a file's blocks, of a program in passes (`BlockNative::passes`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pass {
    /// First block to first: a body, as a program without passes runs it. Its `Load`s read the
    /// slots of passes before it. Before the last pass, output `k` is stored to slot
    /// `stores[k]`, each block's word as the body made it (no cut, no delay); the last pass's
    /// outputs are the program's.
    Forward { body: BlockNative, stores: Vec<u16> },
    /// Last block to first: each `back_fill` (`block::carry::back_fill`), its start's slot, its
    /// end's, and its own, the carry zero at the file's last block.
    Back(Vec<(u16, u16, u16)>),
}

/// A program as passes over each file's blocks, and the slots they store per block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Passes {
    pub passes: Vec<Pass>,
    pub slots: u16,
    /// Each pass's instructions' places in the whole program (a `Load`'s, the value it loads):
    /// for the profile's report of what is live where (`native_cpu_pressure`). Empty for a
    /// backward pass.
    pub origins: Vec<Vec<u32>>,
}

impl BlockNative {
    /// The program as passes. A `back_fill` reads its operands over the whole file before it
    /// gives its first word, so each value's level is the most `back_fill`s on a way to it (a
    /// fixed point's members all at the highest of theirs): level 0's values in a first forward
    /// pass, each level's `back_fill`s in a backward pass after the level before, its other
    /// values in a forward pass after that. A value read in a later pass, and an output made
    /// before the last, has a slot. A program without `back_fill` is one forward pass.
    ///
    /// Some values made from the bytes and positions alone are remade where they are read, not
    /// stored (`native_cpu_lanes::remade`): simdjson keeps nothing between its phases that it
    /// can cheaply remake from the input, and a later pass's gate can then skip it.
    pub fn passes(&self) -> Passes {
        self.passes_with(&crate::native_cpu_lanes::remade(self))
    }

    /// The values made from the bytes and positions alone: byte tests, classes, constants,
    /// `within`, and logic and `prev`s of them. Any forward pass can make one.
    pub fn pure(&self) -> Vec<bool> {
        let n = self.insts.len();
        let mut pure = vec![false; n];
        for x in 0..n {
            pure[x] = match self.insts[x] {
                BlockInst::Const(_) | BlockInst::Within(_) | BlockInst::EqByte(_) | BlockInst::Class { .. } | BlockInst::Plane(_) | BlockInst::PlaneBefore(_) | BlockInst::Pair { .. } => true,
                BlockInst::Not(_) | BlockInst::And(..) | BlockInst::Or(..) | BlockInst::Xor(..) | BlockInst::AndNot(..) | BlockInst::Prev(..) => self.insts[x].operands().into_iter().all(|o| pure[o]),
                _ => false,
            };
        }
        for g in &self.groups {
            for &m in &g.members {
                pure[m] = false;
            }
        }
        pure
    }

    /// The passes, with the `remade` values (pure ones) made in each forward pass that reads
    /// them instead of stored by the first and loaded: only a `back_fill`'s operand among them is
    /// stored, by the first forward pass that makes it.
    pub fn passes_with(&self, remade: &[bool]) -> Passes {
        let n = self.insts.len();
        let back = |x: usize| matches!(self.insts[x], BlockInst::BackFill(..));
        let mut level = vec![0usize; n];
        loop {
            let mut changed = false;
            for x in 0..n {
                let l = self.insts[x].operands().into_iter().map(|o| level[o]).max().unwrap_or(0) + usize::from(back(x));
                if l > level[x] {
                    level[x] = l;
                    changed = true;
                }
            }
            for g in &self.groups {
                let top = g.members.iter().map(|&m| level[m]).max().unwrap_or(0);
                for &m in &g.members {
                    if level[m] < top {
                        level[m] = top;
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }
        let top = level.iter().copied().max().unwrap_or(0);
        if top == 0 {
            return Passes { passes: vec![Pass::Forward { body: self.clone(), stores: Vec::new() }], slots: 0, origins: vec![(0..n as u32).collect()] };
        }
        // Each value's pass: level L's `back_fill`s in pass 2L - 1, its others in pass 2L. A
        // remade value's passes are its readers': `made[p][x]`.
        let pass = |x: usize| 2 * level[x] - usize::from(back(x));
        let last = 2 * top;
        let mut made = vec![vec![false; n]; last + 1];
        for x in (0..n).filter(|&x| !remade[x]) {
            made[pass(x)][x] = true;
        }
        for &v in self.outputs.iter().filter(|&&v| remade[usize::from(v)]) {
            made[last][usize::from(v)] = true;
        }
        for x in (0..n).rev() {
            for o in self.insts[x].operands().into_iter().filter(|&o| remade[o]) {
                if back(x) {
                    made[pass(x) - 1][o] = true;
                }
                for p in (0..=last).step_by(2) {
                    if made[p][x] {
                        made[p][o] = true;
                    }
                }
            }
        }
        let in_pass = |x: usize, p: usize| made[p][x];
        // A remade value with a slot (a `back_fill` reads it) is stored by its first pass.
        let first = |x: usize| (0..=last).find(|&p| made[p][x]);
        let mut slot: Vec<Option<u16>> = vec![None; n];
        let mut slots = 0u16;
        let mut give = |x: usize, slot: &mut Vec<Option<u16>>| {
            if slot[x].is_none() {
                slot[x] = Some(slots);
                slots += 1;
            }
        };
        for x in 0..n {
            for p in (0..=last).filter(|&p| made[p][x]) {
                for o in self.insts[x].operands() {
                    if !in_pass(o, p) {
                        give(o, &mut slot);
                    }
                }
            }
        }
        for &v in &self.outputs {
            if !in_pass(usize::from(v), last) {
                give(usize::from(v), &mut slot);
            }
        }
        let mut passes = Vec::new();
        let mut origins = Vec::new();
        for p in 0..=last {
            if p % 2 == 1 {
                let fills: Vec<(u16, u16, u16)> = (0..n)
                    .filter(|&x| pass(x) == p)
                    .map(|x| match self.insts[x] {
                        BlockInst::BackFill(a, b) => (slot[usize::from(a)].expect("read later"), slot[usize::from(b)].expect("read later"), slot[x].expect("read later")),
                        _ => unreachable!("a back pass holds `back_fill`s"),
                    })
                    .collect();
                passes.push(Pass::Back(fills));
                origins.push(Vec::new());
                continue;
            }
            let mine: Vec<usize> = (0..n).filter(|&x| in_pass(x, p)).collect();
            // The values of passes before read here, loaded first; then the pass's own, in order.
            let mut loads: Vec<usize> = mine.iter().flat_map(|&x| self.insts[x].operands()).filter(|&o| !in_pass(o, p)).collect();
            if p == last {
                loads.extend(self.outputs.iter().map(|&v| usize::from(v)).filter(|&v| !in_pass(v, p)));
            }
            loads.sort_unstable();
            loads.dedup();
            // Each load just before its first reader (a fixed point's first member, for a value
            // its members read): a word loaded at the body's start lives through all of it.
            let mut first_member: Vec<Option<usize>> = vec![None; n];
            for g in self.groups.iter().filter(|g| g.members.iter().all(|&m| in_pass(m, p))) {
                let first = g.members.iter().copied().min().expect("a member");
                for &m in &g.members {
                    first_member[m] = Some(first);
                }
            }
            let mut due: std::collections::BTreeMap<usize, Vec<usize>> = Default::default();
            let mut late = Vec::new();
            for &x in &loads {
                match mine.iter().copied().filter(|&r| self.insts[r].operands().contains(&x)).map(|r| first_member[r].unwrap_or(r)).min() {
                    Some(at) => due.entry(at).or_default().push(x),
                    None => late.push(x),
                }
            }
            let mut local: Vec<Option<u16>> = vec![None; n];
            let mut insts = Vec::with_capacity(loads.len() + mine.len());
            let mut origin: Vec<u32> = Vec::with_capacity(loads.len() + mine.len());
            let load = |x: usize, local: &mut Vec<Option<u16>>, insts: &mut Vec<BlockInst>, origin: &mut Vec<u32>| {
                local[x] = Some(insts.len() as u16);
                insts.push(BlockInst::Load(slot[x].expect("read later")));
                origin.push(x as u32);
            };
            for &x in &mine {
                for &l in due.get(&x).map(Vec::as_slice).unwrap_or(&[]) {
                    load(l, &mut local, &mut insts, &mut origin);
                }
                local[x] = Some(insts.len() as u16);
                origin.push(x as u32);
                let m = |o: u16| local[usize::from(o)].expect("made before");
                insts.push(match self.insts[x] {
                    BlockInst::Not(a) => BlockInst::Not(m(a)),
                    BlockInst::And(a, b) => BlockInst::And(m(a), m(b)),
                    BlockInst::Or(a, b) => BlockInst::Or(m(a), m(b)),
                    BlockInst::Xor(a, b) => BlockInst::Xor(m(a), m(b)),
                    BlockInst::AndNot(a, b) => BlockInst::AndNot(m(a), m(b)),
                    BlockInst::Prev(a, k) => BlockInst::Prev(m(a), k),
                    BlockInst::Toggle(a) => BlockInst::Toggle(m(a)),
                    BlockInst::Escaped(a) => BlockInst::Escaped(m(a)),
                    BlockInst::Fill(a, b) => BlockInst::Fill(m(a), m(b)),
                    BlockInst::Carry(a) => BlockInst::Carry(m(a)),
                    BlockInst::Walk { walk, out, args } => BlockInst::Walk { walk, out, args: m(args) },
                    BlockInst::Args(a, b) => BlockInst::Args(m(a), m(b)),
                    BlockInst::BackFill(..) | BlockInst::Load(_) => unreachable!("not in a forward pass"),
                    other => other,
                });
            }
            // Loads only an output reads: last.
            for &l in &late {
                load(l, &mut local, &mut insts, &mut origin);
            }
            let place = |x: usize| usize::from(local[x].expect("the pass's"));
            let groups: Vec<crate::block::Group> = self
                .groups
                .iter()
                .filter(|g| g.members.iter().all(|&m| in_pass(m, p)))
                .map(|g| crate::block::Group { members: g.members.iter().map(|&x| place(x)).collect(), carries: g.carries.iter().map(|&(c, x)| (place(c), place(x))).collect() })
                .collect();
            // A walk is made in the pass of its values, all in one (its `Walk`s read one `Args`).
            // In another pass it is a walk of no kinds, of nothing (its `hidden` any value).
            let walks: Vec<crate::block::Walk> = self.walks.iter().map(|w| if w.made().iter().all(|&v| in_pass(usize::from(v), p)) { Self::walk_renamed(w, |v| place(usize::from(v)) as u16) } else { crate::block::Walk { group: w.group, hidden: 0, kinds: Vec::new() } }).collect();
            let body = |outputs: Vec<u16>, last: bool| BlockNative {
                walks: walks.clone(),
                tables: self.tables.clone(),
                insts: insts.clone(),
                output_delays: if last { self.output_delays.clone() } else { vec![0; outputs.len()] },
                outputs,
                extracts: if last { self.extracts.clone() } else { Vec::new() },
                totals: if last { self.totals.clone() } else { Vec::new() },
                masks: if last { self.masks.clone() } else { Vec::new() },
                delay: self.delay,
                groups: groups.clone(),
                records: if last { self.records.clone() } else { None },
            };
            if p == last {
                let outputs = self.outputs.iter().map(|&v| local[usize::from(v)].expect("loaded or made")).collect();
                passes.push(Pass::Forward { body: body(outputs, true), stores: Vec::new() });
                origins.push(origin);
            } else {
                let stored: Vec<usize> = mine.iter().copied().filter(|&x| slot[x].is_some() && first(x) == Some(p)).collect();
                if stored.is_empty() {
                    continue;
                }
                let outputs = stored.iter().map(|&x| local[x].expect("the pass's")).collect();
                passes.push(Pass::Forward { body: body(outputs, false), stores: stored.iter().map(|&x| slot[x].expect("stored")).collect() });
                origins.push(origin);
            }
        }
        Passes { passes, slots, origins }
    }
}

/// A formula's value: a constant, or an instruction's.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Term {
    Zero,
    Ones,
    At(u16),
}

/// The instructions of `planes_form`, with formulas shared.
#[derive(Default)]
struct PlaneFormulas {
    insts: Vec<BlockInst>,
    /// Planes 0..8 of the bytes, 8..16 of the bytes before.
    planes: [Option<u16>; 16],
    functions: std::collections::HashMap<(usize, usize, u16), Term>,
    shared: std::collections::HashMap<BlockInst, u16>,
}

impl PlaneFormulas {
    /// An instruction's index, shared with an earlier identical one.
    fn push(&mut self, inst: BlockInst) -> u16 {
        // A carry is its group's own value, never another's, however alike their starts.
        let pure = !matches!(inst, BlockInst::Class { .. } | BlockInst::Pair { .. } | BlockInst::Carry(_));
        if pure {
            if let Some(&i) = self.shared.get(&inst) {
                return i;
            }
        }
        let i = self.insts.len() as u16;
        self.insts.push(inst);
        if pure {
            self.shared.insert(inst, i);
        }
        i
    }

    fn index(&mut self, t: Term) -> u16 {
        match t {
            Term::Zero => self.push(BlockInst::Const(0)),
            Term::Ones => self.push(BlockInst::Const(u64::MAX)),
            Term::At(i) => i,
        }
    }

    fn plane(&mut self, k: usize) -> u16 {
        if let Some(i) = self.planes[k] {
            return i;
        }
        let inst = if k < 8 { BlockInst::Plane(k as u8) } else { BlockInst::PlaneBefore(k as u8 - 8) };
        let i = self.push(inst);
        self.planes[k] = Some(i);
        i
    }

    /// The function of planes `base..base + vars` whose truth table, indexed by those planes'
    /// bits, is the low `2^vars` bits of `tt`.
    fn function(&mut self, base: usize, vars: usize, tt: u16) -> Term {
        let size = 1u32 << vars;
        let full = if size == 16 { u16::MAX } else { (1u16 << size) - 1 };
        let tt = tt & full;
        if tt == 0 {
            return Term::Zero;
        }
        if tt == full {
            return Term::Ones;
        }
        if let Some(&t) = self.functions.get(&(base, vars, tt)) {
            return t;
        }
        let half = size / 2;
        let low = (1u16 << half) - 1;
        let (t0, t1) = (tt & low, (tt >> half) & low);
        let f0 = self.function(base, vars - 1, t0);
        let f1 = self.function(base, vars - 1, t1);
        let x = self.plane(base + vars - 1);
        let term = match (f0, f1) {
            _ if f0 == f1 => f0,
            (Term::Zero, Term::Ones) => Term::At(x),
            (Term::Ones, Term::Zero) => Term::At(self.push(BlockInst::Not(x))),
            (Term::Zero, Term::At(b)) => Term::At(self.push(BlockInst::And(x, b))),
            (Term::At(a), Term::Zero) => Term::At(self.push(BlockInst::AndNot(a, x))),
            (Term::At(a), Term::Ones) => Term::At(self.push(BlockInst::Or(x, a))),
            (Term::Ones, Term::At(b)) => {
                let n = self.push(BlockInst::Not(x));
                Term::At(self.push(BlockInst::Or(b, n)))
            }
            (Term::At(a), Term::At(_)) if t1 == !t0 & low => Term::At(self.push(BlockInst::Xor(x, a))),
            (Term::At(a), Term::At(b)) => {
                // f0 ^ ((f0 ^ f1) & x)
                let d = self.push(BlockInst::Xor(a, b));
                let s = self.push(BlockInst::And(d, x));
                Term::At(self.push(BlockInst::Xor(a, s)))
            }
            _ => unreachable!("constants are handled first"),
        };
        self.functions.insert((base, vars, tt), term);
        term
    }

    fn and(&mut self, a: Term, b: Term) -> Term {
        match (a, b) {
            (Term::Zero, _) | (_, Term::Zero) => Term::Zero,
            (Term::Ones, t) | (t, Term::Ones) => t,
            (Term::At(a), Term::At(b)) => Term::At(self.push(BlockInst::And(a, b))),
        }
    }

    fn or(&mut self, a: Term, b: Term) -> Term {
        match (a, b) {
            (Term::Ones, _) | (_, Term::Ones) => Term::Ones,
            (Term::Zero, t) | (t, Term::Zero) => t,
            (Term::At(a), Term::At(b)) => Term::At(self.push(BlockInst::Or(a, b))),
        }
    }
}
