//! Macro expansion on the device (decision 0448), `pp_expand.metal`. Built so far: the setup's
//! definitions (S1, S2, S4), read from the tapes in place.

use crate::Error;
use crate::pp_blocks::{Arena, Buf, Grid, Lay, Scan, Sort};
use crate::pp_eval::{Front, TO_ALL, TO_DEFS, TO_VIS, Unit, fewer, name_key, on_grid};

/// A definition (`PpDef` in the MSL): the unit's nodes of its name, its `params` node (0: none)
/// and its body's first leaf; the leaves of each; its flags; its first parameter slot and item.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PpDef {
    pub name: u32,
    pub params: u32,
    pub n_params: u32,
    pub body: u32,
    pub n_body: u32,
    pub flags: u32,
    pub pfirst: u32,
    pub ifirst: u32,
}

/// `PpDef::flags`.
pub const D_DEFINE: u32 = 1;
pub const D_FNLIKE: u32 = 2;
pub const D_VARIADIC: u32 = 4;
pub const D_BAD: u32 = 8;
/// An item's parameter where it is none, and its flags above it.
pub const NO_PARAM: u32 = 0xffff;
pub const I_PASTE: u32 = 1 << 16;
pub const I_SPACE: u32 = 1 << 17;

/// The setup's definitions on the device: one a directive row, each parameter list's leaves'
/// keys (a parameter's at its leaf, 0 elsewhere), and each body leaf's item.
pub struct Defs {
    pub defs: Buf,
    pcount: Buf,
    bcount: Buf,
    pfirst: Buf,
    ifirst: Buf,
    pscan: Scan,
    bscan: Scan,
    pub pkeys: Buf,
    pub items: Buf,
    /// Set where a store's room ran out (decision 0448 §2): the unit runs again at the next rung.
    pub room: Buf,
    cap: u32,
}

/// The rungs of the setup's stores: rung `k` holds `nodes >> (RUNGS - 1 - k)` leaves, so the last
/// holds every node of the unit, the most there can be.
pub const RUNGS: u32 = 4;

impl Defs {
    /// Room for `unit`'s rows, and at rung `rung` for their parameters' and bodies' leaves: each
    /// a node, so the last rung's, the unit's nodes, bounds them.
    pub fn new(arena: &Arena, unit: &Unit, rung: u32) -> Result<Defs, Error> {
        let rows = unit.dir_bound;
        let nodes = (unit.node_bound >> (RUNGS - 1 - rung.min(RUNGS - 1))).max(256);
        let words = |k: usize| arena.zeroed::<u32>(k.max(1));
        Ok(Defs {
            defs: arena.zeroed::<PpDef>(rows)?,
            pcount: words(rows)?,
            bcount: words(rows)?,
            pfirst: words(rows)?,
            ifirst: words(rows)?,
            pscan: Scan::new(arena)?,
            bscan: Scan::new(arena)?,
            pkeys: arena.zeroed::<u64>(nodes)?,
            items: words(nodes)?,
            room: words(1)?,
            cap: nodes as u32,
        })
    }

    /// The definitions' slots of the layout: S1 (`pd_defs`, a thread a row), the scans of the
    /// counts, S2 (`pd_params`, a thread a parameter list's leaf) and S4 (`pd_items`, a thread a
    /// body leaf), as phases of `pp_setup`.
    pub fn lay(&self, l: &mut Lay) {
        let va = name_key(b"__VA_ARGS__");
        l.at("DEFS", &self.defs).at("PCOUNT", &self.pcount).at("BCOUNT", &self.bcount).at("PFIRST", &self.pfirst).at("IFIRST", &self.ifirst);
        l.at("PTOTAL", &self.pscan.total).at("ITOTAL", &self.bscan.total).at("PKEYS", &self.pkeys).at("ITEMS", &self.items).at("ROOM", &self.room);
        l.word("CAP", self.cap).word("VA_LO", va as u32).word("VA_HI", (va >> 32) as u32);
    }
}

/// The setup's definitions, read back for tests.
pub struct DefsRead {
    pub defs: Vec<PpDef>,
    pub pkeys: Vec<u64>,
    pub items: Vec<u32>,
}

/// The front passes and the definitions over `unit`, in one command buffer, read back: from
/// rung 0, and at the next where a store's room ran out.
pub fn definitions(unit: &Unit) -> Result<(crate::pp_eval::Evaluated, DefsRead), Error> {
    definitions_from(unit, 0)
}

/// `definitions` from rung `rung`.
pub fn definitions_from(unit: &Unit, rung: u32) -> Result<(crate::pp_eval::Evaluated, DefsRead), Error> {
    let mut groups = crate::pp_blocks::GROUPS;
    let (front, defs) = loop {
        let arena = Arena::new();
        let front = Front::new(&arena, unit, false, groups)?;
        let defs = Defs::new(&arena, unit, rung)?;
        arena.lease()?;
        let mut l = Lay::new();
        front.lay(&mut l, TO_DEFS);
        defs.lay(&mut l);
        if on_grid(&arena, unit, &front, &l, groups)?.is_some() {
            break (front, defs);
        }
        groups = fewer(groups)?;
    };
    // SAFETY: the device finished writing `room`.
    if unsafe { *defs.room.host().cast::<u32>() } != 0 {
        if rung + 1 >= RUNGS {
            return Err(Error::BackendCapacity);
        }
        drop((front, defs));
        return definitions_from(unit, rung + 1);
    }
    let ev = front.read()?;
    let rows = ev.rows.len();
    // SAFETY: the device finished; each buffer holds at least the counts read.
    let (pt, bt) = unsafe { (*defs.pscan.total.host().cast::<u32>() as usize, *defs.bscan.total.host().cast::<u32>() as usize) };
    let read = unsafe {
        DefsRead {
            defs: std::slice::from_raw_parts(defs.defs.host().cast::<PpDef>(), rows).to_vec(),
            pkeys: std::slice::from_raw_parts(defs.pkeys.host().cast::<u64>(), pt).to_vec(),
            items: std::slice::from_raw_parts(defs.items.host().cast::<u32>(), bt).to_vec(),
        }
    };
    Ok((ev, read))
}

/// An instance of a file in the unit's layout (`PpInst` in the MSL): its file, its first
/// directive and its first stretch in `Layout`'s arrays.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PpInst {
    pub file: u32,
    pub dir_first: u32,
    pub gap_first: u32,
}

/// The unit's include tree laid out: P1's answer, the test input of 0448 until P1's includes
/// and conditionals run on the device. By instance, in the unit's order; by directive and by
/// stretch (a file's `k`-th, before its directive `k`; the last after them all) of each
/// instance, its slot and whether it is active.
pub struct Layout {
    pub insts: Vec<PpInst>,
    pub dir_slots: Vec<u32>,
    pub dir_active: Vec<u32>,
    pub gap_slots: Vec<u32>,
    pub gap_active: Vec<u32>,
    /// The built-in macros' names' keys: rows at slot 0.
    pub builtins: Vec<u64>,
    /// The dialect's options as the paste relexer reads them (`relex_options`).
    pub relex_options: u32,
    /// Each file's `__FILE__`: the string literal of its presumed path (its `"` and `\` escaped),
    /// by the unit's file index; then `__DATE__`'s, `__TIME__`'s and `__TIMESTAMP__`'s.
    pub file_literals: Vec<Vec<u8>>,
}

/// What a lookup answers where it is no definition's row (`pp_expand.metal`'s `L_*`).
pub const L_NOT_NAME: u32 = 0xffff_ffff;
pub const L_NONE: u32 = 0xffff_fffe;
pub const L_BUILTIN: u32 = 0xffff_fffd;
/// The built-ins the device expands to one token, and those that are plain names in text
/// (`__has_include`, `__has_include_next`, `__has_embed`); a definition's row is below `L_IDENT`.
pub const L_FILE: u32 = 0xffff_fffc;
pub const L_LINE: u32 = 0xffff_fffb;
pub const L_DATE: u32 = 0xffff_fffa;
pub const L_TIME: u32 = 0xffff_fff9;
pub const L_TIMESTAMP: u32 = 0xffff_fff8;
pub const L_IDENT: u32 = 0xffff_fff7;

/// A built-in's lookup value, by its name.
fn builtin_value(key: u64) -> u32 {
    let named: [(&[u8], u32); 8] = [
        (b"__LINE__", L_LINE),
        (b"__FILE__", L_FILE),
        (b"__DATE__", L_DATE),
        (b"__TIME__", L_TIME),
        (b"__TIMESTAMP__", L_TIMESTAMP),
        (b"__has_include", L_IDENT),
        (b"__has_include_next", L_IDENT),
        (b"__has_embed", L_IDENT),
    ];
    named.iter().find(|(n, _)| name_key(n) == key).map_or(L_BUILTIN, |&(_, v)| v)
}

/// S6–S8 and the lookups (R4): the text map, the visibility rows and their sort, the active
/// stretches, and each stretch token's meaning where it stands.
pub struct Vis {
    // The layout, on the device.
    insts: Buf,
    n_insts: Buf,
    dslot: Buf,
    dactive: Buf,
    n_dirs: Buf,
    gslot: Buf,
    gactive: Buf,
    pub(crate) n_gaps: Buf,
    n_builtin: u32,
    nd: usize,
    ng: usize,
    // The text map: a word a node of the unit.
    owner_in: Buf,
    heads: Buf,
    owner: Buf,
    tflag: Buf,
    trank: Buf,
    tscan: Scan,
    pub(crate) text_nodes: Buf,
    // The rows: a directive of the layout each, after the built-ins'.
    rflag: Buf,
    rat: Buf,
    rscan: Scan,
    pub(crate) sort_keys: Buf,
    pub(crate) name_keys: Buf,
    pub(crate) values: Buf,
    pub(crate) n_rows: Buf,
    pub(crate) sort: Sort,
    // The stretches: a word each, and a lookup a text token.
    gcounts: Buf,
    pub(crate) gfirsts: Buf,
    /// The stretches in the unit's order: their keys (slots), sorted with their places, and
    /// each one's slot in that order.
    gkeys: Buf,
    gsort: Sort,
    pub(crate) sslot: Buf,
    pub(crate) gat: Buf,
    pub(crate) gscan: Scan,
    pub lookups: Buf,
    pub bad: Buf,
}

/// The top level's tokens at most: each instance's file's nodes.
pub fn top_bound(unit: &Unit, lay: &Layout) -> usize {
    lay.insts.iter().map(|i| unit.file_nodes[i.file as usize] as usize).sum::<usize>().max(1)
}

const NO_INST: [PpInst; 1] = [PpInst { file: 0, dir_first: 0, gap_first: 0 }];

impl Vis {
    pub fn new(arena: &Arena, unit: &Unit, lay: &Layout) -> Result<Vis, Error> {
        let words = |v: &[u32]| arena.from_slice(if v.is_empty() { &[0] } else { v });
        let zeroed = |k: usize| arena.zeroed::<u32>(k.max(1));
        let nodes = unit.node_bound;
        let top = top_bound(unit, lay);
        let (nd, ng, nb) = (lay.dir_slots.len(), lay.gap_slots.len(), lay.builtins.len());
        let rows = nd + nb;
        // The built-ins' rows, at slot 0: the configuration's, written before the passes.
        let mut sk = vec![0u64; rows.max(1)];
        let mut nk = vec![0u64; rows.max(1)];
        let mut vals = vec![0u32; rows.max(1)];
        for (k, &key) in lay.builtins.iter().enumerate() {
            sk[k] = key & 0xffff_ffff_0000_0000;
            nk[k] = key;
            vals[k] = builtin_value(key);
        }
        Ok(Vis {
            insts: arena.from_slice(if lay.insts.is_empty() { &NO_INST } else { &lay.insts })?,
            n_insts: words(&[lay.insts.len() as u32])?,
            dslot: words(&lay.dir_slots)?,
            dactive: words(&lay.dir_active)?,
            n_dirs: words(&[nd as u32])?,
            gslot: words(&lay.gap_slots)?,
            gactive: words(&lay.gap_active)?,
            n_gaps: words(&[ng as u32])?,
            n_builtin: nb as u32,
            nd,
            ng,
            owner_in: zeroed(nodes)?,
            heads: zeroed(nodes)?,
            owner: zeroed(nodes)?,
            tflag: zeroed(nodes)?,
            trank: zeroed(nodes)?,
            tscan: Scan::new(arena)?,
            text_nodes: zeroed(nodes)?,
            rflag: zeroed(nd)?,
            rat: zeroed(nd)?,
            rscan: Scan::new(arena)?,
            sort_keys: arena.from_slice(&sk)?,
            name_keys: arena.from_slice(&nk)?,
            values: arena.from_slice(&vals)?,
            n_rows: zeroed(1)?,
            sort: Sort::new(arena, rows)?,
            gcounts: zeroed(ng)?,
            gfirsts: zeroed(ng)?,
            gkeys: arena.zeroed::<u64>(ng.max(1))?,
            gsort: Sort::new(arena, ng)?,
            sslot: zeroed(ng)?,
            gat: zeroed(ng)?,
            gscan: Scan::new(arena)?,
            lookups: zeroed(top)?,
            bad: zeroed(1)?,
        })
    }

    /// The visibility's slots of the layout: the text map (a max scan of each node's owner, a
    /// scan of the text flags), the rows (a flag and a scan, then a sort by name), the stretches
    /// (a sort of their keys, their counts and a scan) and the lookups, as phases of `pp_setup`.
    pub fn lay(&self, l: &mut Lay) {
        l.at("INSTS", &self.insts).at("N_INSTS", &self.n_insts).at("DSLOT", &self.dslot).at("DACTIVE", &self.dactive).at("N_DIRS", &self.n_dirs);
        l.at("GSLOT", &self.gslot).at("GACTIVE", &self.gactive).at("N_GAPS", &self.n_gaps);
        l.word("N_BUILTIN", self.n_builtin).word("ND", self.nd as u32).word("NG", self.ng as u32);
        l.at("OWNER_IN", &self.owner_in).at("VHEADS", &self.heads).at("OWNER", &self.owner).at("TFLAG", &self.tflag).at("TRANK", &self.trank);
        l.at("TTOTAL", &self.tscan.total).at("TEXT_NODES", &self.text_nodes).at("RFLAG", &self.rflag).at("RAT", &self.rat).at("RTOTAL", &self.rscan.total);
        l.at("SORT_KEYS", &self.sort_keys).at("NAME_KEYS", &self.name_keys).at("VALUES", &self.values).at("N_ROWS", &self.n_rows);
        l.at("SORT_KB", &self.sort.keys_b).at("SORT_VA", &self.sort.vals_a).at("SORT_VB", &self.sort.vals_b).at("SORT_HIST", &self.sort.hist);
        l.at("GCOUNTS", &self.gcounts).at("GFIRSTS", &self.gfirsts).at("GKEYS", &self.gkeys);
        l.at("GSORT_KB", &self.gsort.keys_b).at("GSORT_VA", &self.gsort.vals_a).at("GSORT_VB", &self.gsort.vals_b).at("GSORT_HIST", &self.gsort.hist);
        l.at("SSLOT", &self.sslot).at("GAT", &self.gat).at("GTOTAL", &self.gscan.total).at("LOOKUPS", &self.lookups).at("VBAD", &self.bad);
    }

    /// The lookups, read back for tests (after the command buffer is done).
    pub fn read(&self) -> Result<Vec<u32>, Error> {
        // SAFETY: the device finished; `lookups` holds the scan's total of words.
        unsafe {
            if *self.bad.host().cast::<u32>() != 0 {
                return Err(Error::BackendCapacity);
            }
            let n = *self.gscan.total.host().cast::<u32>() as usize;
            Ok(std::slice::from_raw_parts(self.lookups.host().cast::<u32>(), n).to_vec())
        }
    }
}

/// A list (`PxList` in the MSL): 24 words; an argument (`PxArg`): 3.
const LIST_WORDS: usize = 24;
const ARG_WORDS: usize = 3;

/// Why a unit is refused (`pp_expand.metal`'s `X_*`). `X_ROOM` reruns it at the next rung.
pub const X_BUILTIN: u32 = 1;
pub const X_PAST_END: u32 = 4;
pub const X_PASTE: u32 = 8;
pub const X_DEPTH: u32 = 16;
pub const X_ROOM: u32 = 32;
pub const X_QUEUE: u32 = 64;
pub const X_HASH: u32 = 128;
pub const X_ACROSS: u32 = 256;
pub const X_VA_OPT: u32 = 512;
pub const X_RELEX: u32 = 1024;
pub const X_SPLICE: u32 = 2048;
/// A grid barrier's bound reached: the grid's groups were not resident together (decision 0460 §4).
pub const X_BARRIER: u32 = 4096;

/// The expansion's rungs: rung `k`'s arena holds `2^(k+1)` tokens a top token.
pub const EXP_RUNGS: u32 = 5;
/// The close table's keys: a biased depth (16 bits) above a position (24 bits).
const CLOSE_BITS: u32 = 40;
/// A string literal's tape kind.
const K_LITERAL: u32 = 30;

/// The expansion's stores (0448 §2), placed before the command buffer, at a rung:
/// - by top token (`top`): its stretch head, depth change and depth, its `)` flag and place, the
///   close table and its sort, its call's end, the reach's max scan, its state, the outermost
///   flags and their scan, its piece of the output and that piece's place;
/// - the arena: tokens (a ref, flags, a child and a depth each), lists, arguments, the queue (two
///   items a list);
/// - the output (refs), and the control words (the arena's tops, the refusals).
pub struct Exp {
    list_cap: u32,
    out_cap: u32,
    q_cap: u32,
    words: Buf,
    /// The scratch tokens (`#`'s and `##`'s): their bytes, and each one's (start, end, kind).
    /// `st_cap` (start, end, kind) words, then `sb_cap` bytes and slack.
    scratch: Buf,
    st_cap: usize,
    theads: Buf,
    delta: Buf,
    depth: Buf,
    close: Buf,
    close_at: Buf,
    close_scan: Scan,
    close_keys: Buf,
    close_sort: Sort,
    cend: Buf,
    heads: Buf,
    reach: Buf,
    state: Buf,
    oflag: Buf,
    oid: Buf,
    oscan: Scan,
    v: Buf,
    obase: Buf,
    /// The top level's rollback: the exit-bearing calls flagged, their scan, their places in
    /// order, and the live claims' running end.
    xflag: Buf,
    xat: Buf,
    xscan: Scan,
    xpos: Buf,
    xcover: Buf,
    vscan: Scan,
    lists: Buf,
    /// The tokens' columns, `tok_cap` words each: the ref, the flags, the child, the depth and the
    /// origin (the list whose substitution wrote it).
    tokens: Buf,
    args: Buf,
    slots: Buf,
    qctl: Buf,
    pub ctl: Buf,
    pub out: Buf,
}

impl Exp {
    pub fn new(arena: &Arena, unit: &Unit, lay: &Layout, rung: u32) -> Result<Exp, Error> {
        let zeroed = |k: usize| arena.zeroed::<u32>(k.max(1));
        let top = top_bound(unit, lay);
        // Each rung doubles the whole arena, so a short text with a large expansion fits too.
        let tok = (top + 4096) << (rung + 1);
        let lists = top / 2 + tok / 4 + 256;
        let args = tok / 2 + 256;
        let out = top + tok;
        let q = 2 * lists + 64;
        if tok >= u32::MAX as usize / 2 || q >= u32::MAX as usize / 2 {
            return Err(Error::BackendCapacity);
        }
        let va_opt = name_key(b"__VA_OPT__");
        // The scratch: bytes (64 KB at rung 0, doubling) and tokens, in one buffer: the tokens (an
        // even number, so the bytes after them are 8-byte aligned), then the bytes. The first of
        // each are the files' `__FILE__` literals, placed here.
        let nf = unit.len() + 3;
        let lit_bytes: usize = (0..nf).map(|f| lay.file_literals.get(f).map_or(2, Vec::len)).sum();
        let (sb, st) = (((64usize << 10) << rung) + lit_bytes, (tok / 4 + 256 + nf).next_multiple_of(2));
        let mut scratch = vec![0u8; st * 12 + sb + 16];
        let mut at = 0usize;
        for f in 0..nf {
            let lit: &[u8] = lay.file_literals.get(f).map_or(b"\"\"", Vec::as_slice);
            scratch[st * 12 + at..st * 12 + at + lit.len()].copy_from_slice(lit);
            let tok3 = [at as u32, (at + lit.len()) as u32, K_LITERAL];
            for (k, w) in tok3.iter().enumerate() {
                scratch[f * 12 + 4 * k..f * 12 + 4 * k + 4].copy_from_slice(&w.to_le_bytes());
            }
            at += lit.len();
        }
        // The control words: the scratch's tops past the literals.
        let mut ctl = [0u32; 8];
        ctl[4] = lit_bytes as u32;
        ctl[5] = nf as u32;
        let words = [unit.len() as u32, tok as u32, lists as u32, args as u32, out as u32, q as u32, va_opt as u32, (va_opt >> 32) as u32, sb as u32, st as u32, lay.relex_options];
        Ok(Exp {
            list_cap: lists as u32,
            out_cap: out as u32,
            q_cap: q as u32,
            words: arena.from_slice(&words)?,
            // Slack past the last byte: a name's key reads 8 bytes at a time.
            scratch: arena.from_slice(&scratch)?,
            st_cap: st,
            theads: zeroed(top)?,
            delta: zeroed(top)?,
            depth: zeroed(top)?,
            close: zeroed(top)?,
            close_at: zeroed(top)?,
            close_scan: Scan::new(arena)?,
            close_keys: arena.zeroed::<u64>(top)?,
            close_sort: Sort::new(arena, top)?,
            cend: zeroed(top)?,
            heads: zeroed(top)?,
            reach: zeroed(top)?,
            state: zeroed(top)?,
            oflag: zeroed(top)?,
            oid: zeroed(top)?,
            oscan: Scan::new(arena)?,
            v: zeroed(top)?,
            obase: zeroed(top)?,
            xflag: zeroed(top)?,
            xat: zeroed(top)?,
            xscan: Scan::new(arena)?,
            xpos: zeroed(top)?,
            xcover: zeroed(top)?,
            vscan: Scan::new(arena)?,
            lists: zeroed(lists * LIST_WORDS)?,
            tokens: zeroed(tok * 5)?,
            args: zeroed(args * ARG_WORDS)?,
            slots: zeroed(q)?,
            qctl: zeroed(4)?,
            ctl: arena.from_slice(&ctl)?,
            out: zeroed(out)?,
        })
    }

    /// The expansion's slots of the layout: the top level's passes (`pp_setup`'s last phases:
    /// its depth, a segmented scan by stretch; the close table, a scan and a 40-bit sort; the
    /// calls; the outermost, a max scan of the reach and a scan; the lists and the queue's seed),
    /// the queue and the top level's rollback and places (`px_expand`), and the check.
    pub fn lay(&self, l: &mut Lay) {
        l.at("THEADS", &self.theads).at("DELTA", &self.delta).at("DEPTH", &self.depth).at("CLOSE", &self.close).at("CLOSE_AT", &self.close_at);
        l.at("CLOSE_TOTAL", &self.close_scan.total).at("CLOSE_KEYS", &self.close_keys);
        l.at("CSORT_KB", &self.close_sort.keys_b).at("CSORT_VA", &self.close_sort.vals_a).at("CSORT_VB", &self.close_sort.vals_b).at("CSORT_HIST", &self.close_sort.hist);
        l.word("CLOSE_BITS", CLOSE_BITS).at("CEND", &self.cend).at("EHEADS", &self.heads).at("REACH", &self.reach).at("STATE", &self.state);
        l.at("OFLAG", &self.oflag).at("OID", &self.oid).at("OTOTAL", &self.oscan.total).at("LISTS", &self.lists).at("SLOTS", &self.slots);
        l.at("CTL", &self.ctl).at("QCTL", &self.qctl).at("XFLAG", &self.xflag).at("XAT", &self.xat).at("XTOTAL", &self.xscan.total);
        l.at("XPOS", &self.xpos).at("XCOVER", &self.xcover).at("V", &self.v).at("OBASE", &self.obase).at("VTOTAL", &self.vscan.total);
        l.at("OUT", &self.out).at("TOKENS", &self.tokens).at("ARGS", &self.args).at("WORDS", &self.words).at("SCRATCH", &self.scratch);
        l.word("LIST_CAP", self.list_cap).word("OUT_CAP", self.out_cap).word("Q_CAP", self.q_cap);
    }

    /// The scratch tokens' bytes and (start, end, kind) (after the command buffer).
    pub fn scratch(&self) -> (Vec<u8>, Vec<[u32; 3]>) {
        // SAFETY: the device finished; the tops are within the capacities where no room ran out.
        unsafe {
            let ctl = self.ctl.host().cast::<u32>();
            let (nb, nt) = (*ctl.add(4) as usize, *ctl.add(5) as usize);
            let bytes = self.scratch.host().add(self.st_cap * 12);
            let nb = nb.min(self.scratch.byte_length() - self.st_cap * 12);
            let nt = nt.min(self.st_cap);
            (std::slice::from_raw_parts(bytes, nb).to_vec(), std::slice::from_raw_parts(self.scratch.host().cast::<[u32; 3]>(), nt).to_vec())
        }
    }

    /// The refusals, and the output's refs where there are none (after the command buffer).
    /// Each store's use and room (`ROOM_STORES`'s order): its top (past the room where it ran
    /// out: a top only grows) and its capacity.
    pub fn room(&self) -> [(u32, u32); 5] {
        // SAFETY: the device finished; `ctl` and `words` are the control and capacity words.
        unsafe {
            let ctl = self.ctl.host().cast::<u32>();
            let w = self.words.host().cast::<u32>();
            [(0, 1), (1, 2), (3, 3), (4, 8), (5, 9)].map(|(t, c)| (*ctl.add(t), *w.add(c)))
        }
    }

    pub fn read(&self) -> (u32, Vec<u32>) {
        // SAFETY: the device finished; `out` holds at least `n_out` words where no room ran out.
        unsafe {
            let refused = *self.ctl.host().cast::<u32>().add(2);
            if refused != 0 {
                return (refused, Vec::new());
            }
            let n = *self.vscan.total.host().cast::<u32>() as usize;
            (0, std::slice::from_raw_parts(self.out.host().cast::<u32>(), n).to_vec())
        }
    }
}

/// A unit expanded on the device: its output's tokens (refs to its tapes' nodes, in order), or
/// why it is refused (`X_*` bits), and the rung it ran at.
pub struct Expanded {
    pub ev: crate::pp_eval::Evaluated,
    pub refused: u32,
    pub out: Vec<u32>,
    pub rung: u32,
    /// The command buffer's launches: 4 for every unit (decision 0460 §4).
    pub launches: usize,
    /// The groups of the grid it ran on, after any halving.
    pub groups: u64,
    /// The command buffer's time on the GPU (its last run's), in seconds.
    pub gpu_seconds: f64,
    /// The scratch tokens (a ref with `SCRATCH` set): their bytes, and each one's (start, end,
    /// kind).
    pub scratch: Vec<u8>,
    pub stoks: Vec<[u32; 3]>,
    /// The arena's use and room at the rung it ran at, store by store (`ROOM_STORES`).
    pub room: [(u32, u32); 5],
}

/// The arena's stores, as `Expanded::room` lists them.
pub const ROOM_STORES: [&str; 5] = ["tokens", "lists", "arguments", "scratch bytes", "scratch tokens"];

/// A ref to a scratch token, not a tape node.
pub const SCRATCH: u32 = 0x8000_0000;

/// The front passes, the definitions, the lookups and the expansion over `unit` laid out as
/// `lay`, in one command buffer; from rung `rung`, and at the next where a store's room ran out.
pub fn expand_from(unit: &Unit, lay: &Layout, rung: u32) -> Result<Expanded, Error> {
    expand_at(unit, lay, rung, crate::pp_blocks::GROUPS)
}

/// `expand_from` on a grid of `groups` groups at first (halved after a failed barrier).
pub fn expand_at(unit: &Unit, lay: &Layout, rung: u32, groups: u64) -> Result<Expanded, Error> {
    let (mut def_rung, mut rung, mut groups) = (0, rung, groups);
    loop {
        let arena = Arena::new();
        let front = Front::new(&arena, unit, false, groups)?;
        let defs = Defs::new(&arena, unit, def_rung)?;
        let vis = Vis::new(&arena, unit, lay)?;
        let exp = Exp::new(&arena, unit, lay, rung)?;
        arena.lease()?;
        let mut l = Lay::new();
        front.lay(&mut l, TO_ALL);
        defs.lay(&mut l);
        vis.lay(&mut l);
        exp.lay(&mut l);
        let Some((launches, gpu_seconds)) = on_grid(&arena, unit, &front, &l, groups)? else {
            groups = fewer(groups)?;
            continue;
        };
        // SAFETY: the device finished writing `room`.
        if unsafe { *defs.room.host().cast::<u32>() } != 0 {
            def_rung += 1;
            if def_rung >= RUNGS {
                return Err(Error::BackendCapacity);
            }
            continue;
        }
        vis.read()?;
        let (refused, out) = exp.read();
        if refused & X_ROOM != 0 && refused & !X_ROOM == 0 && rung + 1 < EXP_RUNGS {
            rung += 1;
            continue;
        }
        let (scratch, stoks) = exp.scratch();
        return Ok(Expanded { ev: front.read()?, refused, out, rung, launches, groups, gpu_seconds, scratch, stoks, room: exp.room() });
    }
}

/// `expand_from` rung 0.
pub fn expand(unit: &Unit, lay: &Layout) -> Result<Expanded, Error> {
    expand_from(unit, lay, 0)
}

/// The front passes, the definitions and the lookups over `unit` laid out as `lay`, in one
/// command buffer: each active stretch token's meaning (`L_*`, or the directive row of its
/// definition), in the layout's order.
pub fn lookups(unit: &Unit, lay: &Layout) -> Result<(crate::pp_eval::Evaluated, Vec<u32>), Error> {
    let (mut rung, mut groups) = (0, crate::pp_blocks::GROUPS);
    loop {
        let arena = Arena::new();
        let front = Front::new(&arena, unit, false, groups)?;
        let defs = Defs::new(&arena, unit, rung)?;
        let vis = Vis::new(&arena, unit, lay)?;
        arena.lease()?;
        let mut l = Lay::new();
        front.lay(&mut l, TO_VIS);
        defs.lay(&mut l);
        vis.lay(&mut l);
        if on_grid(&arena, unit, &front, &l, groups)?.is_none() {
            groups = fewer(groups)?;
            continue;
        }
        // SAFETY: the device finished writing `room`.
        if unsafe { *defs.room.host().cast::<u32>() } != 0 {
            rung += 1;
            if rung >= RUNGS {
                return Err(Error::BackendCapacity);
            }
            continue;
        }
        return Ok((front.read()?, vis.read()?));
    }
}

/// The dialect's options as the relexer reads them (`pp_expand.metal`'s `O_*`): C++, `::`, digit
/// separators, raw strings.
pub fn relex_options(cpp: bool, colons: bool, digit_separators: bool, raw_strings: bool) -> u32 {
    u32::from(cpp) | u32::from(colons) << 1 | u32::from(digit_separators) << 2 | u32::from(raw_strings) << 3
}

/// The relexer's answer where a spelling is no one token, and where the device refuses it.
pub const RELEX_NONE: u32 = 0;
pub const RELEX_REFUSED: u32 = 0xffff;

/// For tests: the relexer's answer for each spelling (a tape kind, `RELEX_NONE` or
/// `RELEX_REFUSED`), under `options`.
pub fn relex(texts: &[Vec<u8>], options: u32) -> Result<Vec<u32>, Error> {
    let mut bytes = Vec::new();
    let mut offsets = vec![0u32];
    for t in texts {
        bytes.extend_from_slice(t);
        offsets.push(bytes.len() as u32);
    }
    bytes.extend_from_slice(&[0; 16]);
    let arena = Arena::new();
    let (b, o) = (arena.from_slice(&bytes)?, arena.from_slice(&offsets)?);
    let out = arena.zeroed::<u32>(texts.len())?;
    arena.lease()?;
    let n = texts.len() as u32;
    crate::pp_eval::run(|enc| enc.run("pd_relex_test", None, &[&b, &o, &out], &[n, options], Grid::Groups(u64::from(n).div_ceil(crate::pp_blocks::TG))))?;
    // SAFETY: the device finished writing `out`.
    Ok(unsafe { std::slice::from_raw_parts(out.host().cast::<u32>(), texts.len()).to_vec() })
}

/// For measurements: the GPU time of a run over `unit` at `groups` groups that ends after stage
/// `last` (`TO_FRONT`, `TO_DEFS`, `TO_VIS`; `TO_ALL` with the expansion; `TO_ALL + 1` the whole
/// setup without it), in seconds; `None` where a barrier failed.
pub fn stage_seconds(unit: &Unit, lay: &Layout, last: u32, stop: u32, groups: u64) -> Result<Option<f64>, Error> {
    let arena = Arena::new();
    let front = Front::new(&arena, unit, false, groups)?;
    let defs = Defs::new(&arena, unit, 0)?;
    let vis = Vis::new(&arena, unit, lay)?;
    let exp = Exp::new(&arena, unit, lay, 0)?;
    arena.lease()?;
    let mut l = Lay::new();
    front.lay(&mut l, last);
    defs.lay(&mut l);
    vis.lay(&mut l);
    exp.lay(&mut l);
    l.word("STOP", stop);
    Ok(on_grid(&arena, unit, &front, &l, groups)?.map(|(_, s)| s))
}
