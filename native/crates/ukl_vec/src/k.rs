//! The kernel tape (decision 0567 §2.2): a program lowered from its C tape, in the tape format. Each node is an
//! operation of one machine type; its children are its operands, in order. A node's span is its payload in the
//! tape's payload words (its "source", as a C tape's span is its text): the words at its start are its own, by its
//! kind (`Kind`'s docs), and its span runs to the end of its last descendant's.
//!
//! Every value is a machine type's bits in a `u64` (`M`). An lvalue is an address: a variable is `LOCAL` (its place
//! in the thread's frame) or `GLOBAL`, read by `LOAD` and written by `STORE`. A structure's value is its address
//! (`M::agg`): a load of one is that address, a store of one copies its bytes. Every conversion is a `CAST`.

/// A machine type: the low byte its kind, the bytes above it a structure's size (`AGG`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct M(pub u32);

impl M {
    pub const VOID: M = M(0);
    /// `bool`: a byte, 0 or 1.
    pub const B: M = M(1);
    pub const I8: M = M(2);
    pub const U8: M = M(3);
    pub const I16: M = M(4);
    pub const U16: M = M(5);
    pub const I32: M = M(6);
    pub const U32: M = M(7);
    pub const I64: M = M(8);
    pub const U64: M = M(9);
    pub const F32: M = M(10);
    pub const F64: M = M(11);
    /// A pointer: an address, 64 bits.
    pub const PTR: M = M(12);
    const AGG: u32 = 13;

    /// A structure (or an array) of `size` bytes, its value its address.
    pub const fn agg(size: u32) -> M {
        M(Self::AGG | size << 8)
    }

    pub const fn kind(self) -> u32 {
        self.0 & 0xff
    }

    pub const fn is_agg(self) -> bool {
        self.kind() == Self::AGG
    }

    /// Its bytes in memory.
    pub const fn bytes(self) -> u32 {
        match self.kind() {
            0 => 0,
            1..=3 => 1,
            4 | 5 => 2,
            6 | 7 | 10 => 4,
            8 | 9 | 11 | 12 => 8,
            _ => self.0 >> 8,
        }
    }

    pub const fn signed(self) -> bool {
        matches!(self.kind(), 2 | 4 | 6 | 8)
    }

    pub const fn float(self) -> bool {
        matches!(self.kind(), 10 | 11)
    }

    /// Its bits, for an integer.
    pub const fn bits(self) -> u32 {
        if self.kind() == 1 { 1 } else { self.bytes() * 8 }
    }
}

/// A node's kind; its payload words in its doc.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u16)]
pub enum Kind {
    /// The tape's root: `[functions]`; its children the functions' bodies, in the order written, then `Data`.
    Program = 1,
    /// A function: `[id, frame bytes, threadgroup bytes, return type, parameters, (offset, type) a parameter]`; one
    /// child, its body.
    Func,
    /// The program's constant memory (its tables and strings): `[bytes]`, then the bytes, padded to a word.
    Data,
    /// Statements in order (as an expression, an initializer list's stores: its value 0).
    Block,
    /// An expression evaluated for its effect: one child.
    Expr,
    /// `[site]`: `cond, then [, else]`.
    If,
    /// `[site]`: `cond, body`.
    While,
    /// `[site]`: `body, cond`.
    Do,
    /// `[site]`: `cond, body, step`: a `for` (its init a statement before it), `continue` going to `step`.
    For,
    /// `[site]`.
    Break,
    /// `[site]`.
    Continue,
    /// `[has value, site]`: its value, if any. In a vector tape `[has value, site, slot, type]`: the value stored to
    /// its inlined call's result at frame offset `slot` (`NO_SLOT`: the kernel's own, the thread ends).
    Return,
    /// Nothing.
    Empty,
    /// `[type, low, high]`: a constant.
    Const,
    /// `[offset, size, inner, variable]`: the address of byte `inner` of the variable of `size` bytes at `offset` of
    /// the thread's frame; `variable` its number, its function's parameters first then its declarations in order
    /// (variables of scopes apart share offsets). A vector tape's frame holds each variable once a lane, and its
    /// number is its inlined call's (`vector`).
    Local,
    /// `[offset]`: the address of the program's constant memory at `offset`.
    Global,
    /// `[offset]`: the address of the group's threadgroup memory at `offset` (a `SHARED` row's).
    Shared,
    /// `[type]`: the value at its child's address.
    Load,
    /// `[type]`: its second child's value stored at its first's address; the value.
    Store,
    /// `[type, bytes]`: `bytes` zero bytes at its child's address.
    Zero,
    /// `[op, type]`: a binary operation on two values of `type` (a comparison's result a `bool`).
    Bin,
    /// `[op, type]`: a unary one.
    Un,
    /// `[from, to]`: a conversion.
    Cast,
    /// `[type]`: `cond ? a : b`, only the one taken evaluated.
    Cond,
    /// `[and]`: `a && b` (`and` 1) or `a || b`, short-circuit; a `bool`.
    Logic,
    /// `[function, type, temp]`: a call of the program's function, its arguments the children; an aggregate's
    /// result copied to the caller's frame at `temp`, its value that address.
    Call,
    /// `[builtin, type, arguments, site, argument types...]`: one of the intrinsics (`Builtin`), its call's place
    /// in the source `Program::sites[site]`.
    Builtin,
    /// `a, b`: `b`'s value.
    Comma,
    /// `[op, lhs type, op type, post]`: an assignment through `op` (`Op::None`: `++`/`--` as `+= 1`) at its first
    /// child's address, of its second child's value (of `op type`); the new value, or (`post`) the old.
    Update,
    /// `[bytes]`: `bytes` bytes copied from its second child's address to its first's; the first's address.
    Copy,
    /// A vector tape's inlined call (decision 0567 §2.2): `[type, slot, temp]`; its children its parameters' stores,
    /// then the callee's body. Its value is its result at frame offset `slot` (the body's `Return`s store it there),
    /// an aggregate's copied to the caller's `temp` (its value that address).
    Inline,
    /// A code tape's one node (`code`): `[header, parameters, instructions]`.
    Code,
    /// `[site, type]`: a `switch`, its first child its value (of `type`), then its cases in order. A `break` in it
    /// leaves it; a `continue` goes on to the loop it is in.
    Switch,
    /// A vector tape's function in a context it is called in (decision 0567 §2.2): `[body, function, frame, slot,
    /// type, result, parameters, (offset, type) a parameter]`, its child its body. Its locals are at its frame's
    /// offsets, its result at `slot` (past its frame: the bytes its callers read), the variable `result`. A vector
    /// tape's `Call` is `[function, type, temp, body]`.
    Body,
    /// `[default, low, high]`: a `switch`'s case, its label the value `low | high << 32` (none for `default`); its
    /// child its statements, which the lanes that took the case before fall through into.
    Case,
}

/// A `Return`'s slot when it ends the thread (the kernel's own body's).
pub const NO_SLOT: u32 = u32::MAX;

/// An `if`'s, a `while`'s or a `for`'s second word: its condition's hint (`UKL_LIKELY`, `UKL_UNLIKELY`), else 0.
/// A loop's `UKL_LIKELY`: its lanes go on together, so rounds where all do are worth a test.
pub const IF_LIKELY: u32 = 1;
pub const IF_UNLIKELY: u32 = 2;

impl Kind {
    pub const LAST: u16 = Kind::Case as u16;

    pub fn of(k: u16) -> Kind {
        assert!((1..=Self::LAST).contains(&k), "a kernel tape's kind: {k}");
        // SAFETY: `Kind` is `repr(u16)`, its values 1 to `LAST` in order.
        unsafe { std::mem::transmute::<u16, Kind>(k) }
    }
}

/// A binary or unary operation (`Bin`, `Un`, `Update`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Op {
    None,
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Shl,
    Shr,
    And,
    Or,
    Xor,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    /// Unary.
    Neg,
    Not,
    LNot,
}

impl Op {
    pub fn of(w: u32) -> Op {
        assert!(w <= Op::LNot as u32, "an operation: {w}");
        // SAFETY: `Op` is `repr(u32)`, its values 0 to `LNot` in order.
        unsafe { std::mem::transmute::<u32, Op>(w) }
    }

    pub fn compares(self) -> bool {
        matches!(self, Op::Eq | Op::Ne | Op::Lt | Op::Le | Op::Gt | Op::Ge)
    }
}

/// The intrinsics a kernel calls (`__ukl_*`, `Builtin::named`; its SIMD groups 32 lanes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Builtin {
    /// `[dim]` arguments: the thread's ids and the grid's sizes.
    LocalId,
    LocalSize,
    GlobalId,
    GlobalSize,
    GroupId,
    NumGroups,
    SubGroupLocalId,
    /// A threadgroup barrier: a sync point.
    Barrier,
    /// A fence: nothing on the CPU, whose memory is coherent.
    MemFence,
    /// A poll of another group's progress (`ukl_poll_yield`, a grid barrier's): the group yields to the groups that
    /// run with it (`run`'s resident groups), as the CPU's coroutines do. Not a sync point.
    Yield,
    /// The SIMD group's: sync points.
    SubGroupBarrier,
    SubGroupBallot,
    SubGroupShuffle,
    SubGroupShuffleUp,
    SubGroupShuffleDown,
    SubGroupShuffleXor,
    /// The SIMD group's sum, minimum or maximum of its argument: every lane's the same (sync points too).
    SubGroupReduceAdd,
    SubGroupReduceMin,
    SubGroupReduceMax,
    /// Relaxed atomics on a 32-bit word: `(pointer, value)`, `(pointer, expected, new)` for `cmpxchg`, `(pointer)`
    /// for a load. A store is an exchange whose old value no one reads.
    AtomicAdd,
    AtomicSub,
    AtomicAnd,
    AtomicOr,
    AtomicXor,
    AtomicXchg,
    AtomicMin,
    AtomicMax,
    AtomicCmpxchg,
    AtomicInc,
    AtomicDec,
    AtomicLoad,
    AtomicStore,
    /// Bit counts, of the argument's type.
    Popcount,
    Clz,
    /// The high half of the product.
    MulHi,
    /// The bits as another type (`as_int`, `as_float`, ...): the result type's.
    As,
    Min,
    Max,
    /// `a * b + c`, rounded once (`__ukl_fma`, and what a floating `a * b + c` contracts to).
    Fma,
    /// A condition most lanes take, or few do (`UKL_LIKELY`, `UKL_UNLIKELY`): the condition, its hint the `if`'s
    /// (`lower`), never a node of a tape.
    Likely,
    Unlikely,
}

impl Builtin {
    pub fn of(w: u32) -> Builtin {
        assert!(w <= Builtin::Unlikely as u32, "a builtin: {w}");
        // SAFETY: `Builtin` is `repr(u32)`, its values 0 to `Unlikely` in order.
        unsafe { std::mem::transmute::<u32, Builtin>(w) }
    }

    /// Whether every thread of its group (or SIMD group) must reach it together.
    pub fn syncs(self) -> bool {
        matches!(
            self,
            Builtin::Barrier | Builtin::SubGroupBarrier | Builtin::SubGroupBallot | Builtin::SubGroupShuffle | Builtin::SubGroupShuffleUp | Builtin::SubGroupShuffleDown | Builtin::SubGroupShuffleXor
                | Builtin::SubGroupReduceAdd
                | Builtin::SubGroupReduceMin
                | Builtin::SubGroupReduceMax
        )
    }

    /// The builtin of an intrinsic's name (`None`: not a builtin): the `Vec` backend's spellings (`ukl_vec_shim.h`,
    /// `ukl/backend.h`'s `UKL_VEC` section), one name an operation.
    pub fn named(name: &str) -> Option<Builtin> {
        Some(match name {
            "__ukl_local_id" => Builtin::LocalId,
            "__ukl_local_size" => Builtin::LocalSize,
            "__ukl_global_id" => Builtin::GlobalId,
            "__ukl_global_size" => Builtin::GlobalSize,
            "__ukl_group_id" => Builtin::GroupId,
            "__ukl_num_groups" => Builtin::NumGroups,
            "__ukl_lane" => Builtin::SubGroupLocalId,
            "__ukl_barrier" => Builtin::Barrier,
            "__ukl_fence" => Builtin::MemFence,
            "__ukl_yield" => Builtin::Yield,
            "__ukl_sg_barrier" => Builtin::SubGroupBarrier,
            "__ukl_sg_ballot" => Builtin::SubGroupBallot,
            "__ukl_sg_shuffle" => Builtin::SubGroupShuffle,
            "__ukl_sg_shuffle_up" => Builtin::SubGroupShuffleUp,
            "__ukl_sg_shuffle_down" => Builtin::SubGroupShuffleDown,
            "__ukl_sg_shuffle_xor" => Builtin::SubGroupShuffleXor,
            "__ukl_sg_sum" => Builtin::SubGroupReduceAdd,
            "__ukl_sg_min" => Builtin::SubGroupReduceMin,
            "__ukl_sg_max" => Builtin::SubGroupReduceMax,
            "__ukl_atomic_add" => Builtin::AtomicAdd,
            "__ukl_atomic_sub" => Builtin::AtomicSub,
            "__ukl_atomic_and" => Builtin::AtomicAnd,
            "__ukl_atomic_or" => Builtin::AtomicOr,
            "__ukl_atomic_xor" => Builtin::AtomicXor,
            "__ukl_atomic_xchg" => Builtin::AtomicXchg,
            "__ukl_atomic_min" => Builtin::AtomicMin,
            "__ukl_atomic_max" => Builtin::AtomicMax,
            "__ukl_atomic_cas" => Builtin::AtomicCmpxchg,
            "__ukl_atomic_inc" => Builtin::AtomicInc,
            "__ukl_atomic_dec" => Builtin::AtomicDec,
            "__ukl_atomic_load" => Builtin::AtomicLoad,
            "__ukl_atomic_store" => Builtin::AtomicStore,
            "__ukl_popcount" => Builtin::Popcount,
            "__ukl_clz" => Builtin::Clz,
            "__ukl_mulhi" => Builtin::MulHi,
            "__ukl_min" => Builtin::Min,
            "__ukl_max" => Builtin::Max,
            "__ukl_fma" => Builtin::Fma,
            "__ukl_likely" => Builtin::Likely,
            "__ukl_unlikely" => Builtin::Unlikely,
            _ if as_type(name).is_some() => Builtin::As,
            _ => return None,
        })
    }
}

/// The type a reinterpretation `__ukl_as_<type>` names (`__ukl_as_int32_t`, `__ukl_as_float`): C's fixed-width
/// integers and its floats.
pub fn as_type(name: &str) -> Option<&str> {
    let t = name.strip_prefix("__ukl_as_")?;
    matches!(t, "int8_t" | "uint8_t" | "int16_t" | "uint16_t" | "int32_t" | "uint32_t" | "int64_t" | "uint64_t" | "float" | "double").then_some(t)
}

/// A vector tape's node's field (`vector`), its bits: its value's uniformity (`LEVEL`, a `Level`), whether its
/// subtree reaches a sync point (`SYNC`: a barrier or a SIMD-group operation) or a threadgroup barrier (`GSYNC`: it
/// runs for the whole group at once, the cut), whether its condition's uniformity is checked as it runs (`CHECK`), and
/// whether its subtree has an effect (`EFFECT`).
/// A kernel tape's nodes have no field.
pub const LEVEL: u16 = 7;
pub const SYNC: u16 = 8;
pub const GSYNC: u16 = 16;
pub const CHECK: u16 = 32;
/// Its subtree has an effect or reads its lanes as a whole (a store, an atomic, a SIMD-group operation, an inlined
/// call): a uniform value of one without is computed once for its SIMD group.
pub const EFFECT: u16 = 64;

/// A value's uniformity (decision 0567 §2.4), in order: the same in every thread of the group; the same by the
/// kernel's protocol (an atomic that leaves its word as it is, at a uniform address: the same in a SIMD group, and
/// across the group as the kernel's synchronization makes it, checked where control turns on it); the same in each
/// SIMD group; a function of the lane's index in its SIMD group and of uniform values (the same at a lane in each
/// SIMD group: what every SIMD group computes alike from memory the group shares, `lid % 32`); the thread's own id in
/// its group (`get_local_id(0)`, a 1-D group's: its SIMD group's and its lane's, apart); varying from lane to lane.
///
/// Not a chain: `SimdGroup` and `Lane` differ in what they share, and join as `Varying`; `Lid` joins with anything
/// but itself and `Uniform` as `Varying` (an operation on it is the caller's to tell, `vector`). Each threshold
/// (`<= Agreed`: the same in the whole group; `<= SimdGroup`: in the SIMD group) keeps its sense in this order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Level {
    Uniform,
    Agreed,
    SimdGroup,
    Lane,
    Lid,
    Varying,
}

impl Level {
    pub fn of(bits: u16) -> Level {
        [Level::Uniform, Level::Agreed, Level::SimdGroup, Level::Lane, Level::Lid, Level::Varying, Level::Varying, Level::Varying][usize::from(bits & LEVEL)]
    }

    /// The level of a value made of values of levels `self` and `o`.
    pub fn join(self, o: Level) -> Level {
        match (self, o) {
            (x, Level::Uniform) | (Level::Uniform, x) => x,
            (x, y) if x == y => x,
            (Level::Lid, _) | (_, Level::Lid) => Level::Varying,
            (Level::SimdGroup, Level::Lane) | (Level::Lane, Level::SimdGroup) => Level::Varying,
            (x, y) => x.max(y),
        }
    }

    /// The join of `ls`.
    pub fn all(ls: impl IntoIterator<Item = Level>) -> Level {
        ls.into_iter().fold(Level::Uniform, Level::join)
    }
}

/// The tape's language tag: none of a grammar's.
pub const TAG: u8 = 0xfe;

/// A kernel tape being written: its nodes in preorder (key, start, end, size), and its payload words.
#[derive(Default)]
pub struct Builder {
    nodes: Vec<(u32, u32, u32, u32)>,
    pub words: Vec<u32>,
}

impl Builder {
    /// Opens a node of `kind` with payload `words`: its index.
    pub fn open(&mut self, kind: Kind, words: &[u32]) -> usize {
        let start = self.words.len() as u32;
        self.words.extend_from_slice(words);
        self.nodes.push((rerail_tape_format::key(kind as u16, 0, 0), start, start, 0));
        self.nodes.len() - 1
    }

    /// Closes node `n`, opened last of those open: its subtree and its span end here.
    pub fn close(&mut self, n: usize) {
        let (size, end) = ((self.nodes.len() - n) as u32, self.words.len() as u32);
        self.nodes[n].2 = end;
        self.nodes[n].3 = size;
    }

    /// A node of `kind` with no children.
    pub fn leaf(&mut self, kind: Kind, words: &[u32]) -> usize {
        let n = self.open(kind, words);
        self.close(n);
        n
    }

    /// The nodes so far.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Node `n`'s payload word `k` changed (a forward reference filled in).
    pub fn set_word(&mut self, n: usize, k: usize, w: u32) {
        let at = self.nodes[n].1 as usize + k;
        self.words[at] = w;
    }

    /// Node `n`'s key, as written.
    pub fn key(&self, n: usize) -> u32 {
        self.nodes[n].0
    }

    /// Node `n`'s subtree, once closed.
    pub fn size(&self, n: usize) -> usize {
        self.nodes[n].3 as usize
    }

    /// Node `n`'s payload word `k`.
    pub fn word(&self, n: usize, k: usize) -> u32 {
        self.words[self.nodes[n].1 as usize + k]
    }

    /// Node `n`'s field set to `field` (a vector tape's bits, `LEVEL`, ...).
    pub fn set_field(&mut self, n: usize, field: u16) {
        self.nodes[n].0 |= u32::from(field) << rerail_tape_format::KIND_BITS;
    }

    /// The tape: its image (spans in payload words) and the words.
    pub fn finish(self) -> Tape {
        let bytes: Vec<u8> = self.words.iter().flat_map(|w| w.to_le_bytes()).collect();
        let image = rerail_tape_format::preorder_image(TAG, &bytes, &self.nodes);
        Tape { image, words: self.words }
    }
}

/// A kernel tape: its image, and its payload words (its spans' units).
pub struct Tape {
    pub image: rerail_tape_format::Vector<u8>,
    pub words: Vec<u32>,
}

/// A kernel tape read in place.
pub struct View<'t> {
    pub v: rerail_tape_format::View<'t>,
    pub words: &'t [u32],
}

impl<'t> View<'t> {
    pub fn new(t: &'t Tape) -> View<'t> {
        View { v: rerail_tape_format::View::new(&t.image).expect("a kernel tape's image"), words: &t.words }
    }

    pub fn len(&self) -> usize {
        self.v.len()
    }

    pub fn is_empty(&self) -> bool {
        self.v.is_empty()
    }

    #[inline(always)]
    pub fn kind(&self, i: usize) -> Kind {
        Kind::of((self.v.key(i) & 0x3ff) as u16)
    }

    /// Node `i`'s field (a vector tape's bits).
    #[inline(always)]
    pub fn field(&self, i: usize) -> u16 {
        (self.v.key(i) >> rerail_tape_format::KIND_BITS) as u16 & 0x3f
    }

    pub fn level(&self, i: usize) -> Level {
        Level::of(self.field(i))
    }

    /// Whether node `i` runs for its whole group at once (`GSYNC`).
    pub fn gsync(&self, i: usize) -> bool {
        self.field(i) & GSYNC != 0
    }

    /// Node `i`'s payload, its words from its start (a visit's reads, its start found once).
    #[inline(always)]
    pub fn p(&self, i: usize) -> &'t [u32] {
        &self.words[self.v.start(i) as usize..]
    }

    /// Node `i`'s payload word `k`.
    #[inline(always)]
    pub fn w(&self, i: usize, k: usize) -> u32 {
        self.words[self.v.start(i) as usize + k]
    }

    /// Node `i`'s payload words from `k` on, to its span's end.
    pub fn ws(&self, i: usize, k: usize, n: usize) -> &'t [u32] {
        let s = self.v.start(i) as usize + k;
        &self.words[s..s + n]
    }

    pub fn size(&self, i: usize) -> usize {
        self.v.size(i)
    }

    pub fn children(&self, i: usize) -> impl Iterator<Item = usize> + '_ {
        self.v.children(i)
    }

    /// The tape as text, a node a line indented by its depth: its number, kind, level (a vector tape's) and words.
    pub fn listing(&self) -> String {
        let mut out = String::new();
        let mut ends: Vec<usize> = Vec::new();
        for i in 0..self.len() {
            while ends.last().is_some_and(|&e| i >= e) {
                ends.pop();
            }
            // A node's words run to the next node's (preorder: its first child's, or the one after it).
            let end = if i + 1 < self.len() { self.v.start(i + 1) as usize } else { self.words.len() };
            let words = end.saturating_sub(self.v.start(i) as usize).min(10);
            out += &format!("{:indent$}{i} {:?} {:?} {:?}\n", "", self.kind(i), self.level(i), self.ws(i, 0, words), indent = 2 * ends.len());
            ends.push(i + self.size(i));
        }
        out
    }

    /// Node `i`'s `k`-th child.
    pub fn child(&self, i: usize, k: usize) -> usize {
        // Children follow their parent in preorder, each after the one before's subtree.
        let mut c = i + 1;
        for _ in 0..k {
            c += self.v.size(c);
        }
        c
    }
}
