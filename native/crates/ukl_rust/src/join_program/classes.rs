//! The join shapes' classes (the join lab's design, 2026-11-27, §4 and steps 6–7): a shape's stages written from
//! small generic parts, each part fixed at compile time by its stage's form.
//!
//! A stage's **form** (`form_of`) is a word of the record's fields the program's work branches on for each binding:
//! its mode, how its candidates are found (`probe`: a relation's rows, an index's group, a node's own row, a parent's
//! children, a climb, a subtree), how a candidate's columns are read (`read`: a relation, a view, a `has` walk), its
//! keys and which of them are a binding's, its dead bits, its screen, its tests, its row program's operations, its
//! binds, its passed counts, its live length, its last output and its counter. A part is a function generic over
//! the form (`const F: u64`), so each stage's work is compiled with those fields as constants: its branches on them
//! gone, its loops over its keys, tests, operations and binds of fixed length. Everything else (columns, slots,
//! constants, sizes, addresses) is read from the record as the program reads it.
//!
//! A **class** is a shape's stages (`Class`): its key (`rerail_join_shapes::Shape::key`), its name, and the forms its
//! stages had in the runs the catalogue saw, each one's parts (`StageClass`). The classes are generated
//! (`table.rs`, by `join-lab classes`) from the captures' labels and the catalogue, and registered in `CLASSES` by key.
//!
//! The program calls a class (`join_run_classed`) for a launch the caller names classes for (its runs' shapes'): each
//! job of a stage kind whose record's form is one of theirs runs the class's parts in place of the program's work, and
//! is counted (`Classes::ran`); a job none fits runs the program's work, its form named (`Classes::unfit`). The parts
//! make the program's rows and counts exactly: the lab holds a launch run by its classes to the engine's
//! (`join-lab run --engine class`), as it holds the program's.
//!
//! Parts by name (`part_names`): `Scan<Rel>` and `ScanKind<view>` (a relation's or a view's rows), `ByValue<Rel>`
//! (an index's group), `Hash` and `Hash<NEG>` (a semi- and an anti-join's probe), `OwnRead` (a node's own row by its
//! id), `Children`, `Climb`, `Subtree` (the tape's walks), `Cmp` (a filter), `Lattice` (an aggregate), `Witness`,
//! `Seeded` (the component's own rows), `Text<names>` and `Text<textop>` (the program's text work, a stage's), and
//! `Emit` (a binding's slots and binds out). A view's walks (`Children`, `Climb`, `Subtree`) find their candidates
//! through the program's walk of the tape (`St::probe`, `St::walk_tape`): what they cost is the tape's reads, not
//! the branches.

use super::*;
use core::sync::atomic::Ordering::Relaxed;

/// The form's layout's version, in its top byte: a table written for another layout is not this one's.
pub const FORM_VERSION: u64 = 1;

/// How a stage's candidates are found (`St::probe`'s ways, in its order).
pub const P_ROWS: u32 = 0;
pub const P_INDEX: u32 = 1;
pub const P_OWN: u32 = 2;
pub const P_CHILDREN: u32 = 3;
pub const P_CLIMB: u32 = 4;
pub const P_SUBTREE: u32 = 5;

/// How a candidate's columns are read (`St::rget`'s ways).
pub const R_REL: u32 = 0;
pub const R_VIEW: u32 = 1;
/// A `has` walk's rows (`v_has` 1: its climb's, with each candidate's kind tested, `St::has_member`; 2: its subtree's).
pub const R_HAS1: u32 = 2;
pub const R_HAS2: u32 = 3;

pub const fn mode(f: u64) -> u32 {
    (f & 15) as u32
}
pub const fn probe(f: u64) -> u32 {
    ((f >> 4) & 7) as u32
}
pub const fn read(f: u64) -> u32 {
    ((f >> 7) & 3) as u32
}
pub const fn nkeys(f: u64) -> u32 {
    ((f >> 9) & 15) as u32
}
/// Whether key `i` is a binding's slot (else a constant).
pub const fn key_slot(f: u64, i: u32) -> bool {
    (f >> (13 + i)) & 1 != 0
}
pub const fn dead(f: u64) -> bool {
    (f >> 21) & 1 != 0
}
pub const fn screened(f: u64) -> bool {
    (f >> 22) & 1 != 0
}
pub const fn ntests(f: u64) -> u32 {
    ((f >> 23) & 15) as u32
}
pub const fn nops(f: u64) -> u32 {
    ((f >> 27) & 127) as u32
}
pub const fn nbinds(f: u64) -> u32 {
    ((f >> 34) & 31) as u32
}
pub const fn npassed(f: u64) -> u32 {
    ((f >> 39) & 31) as u32
}
/// Whether its relation is read to its live length (`r_len`, `St::rows`).
pub const fn live(f: u64) -> bool {
    (f >> 44) & 1 != 0
}
/// Whether its last output gates its rows (`last`).
pub const fn gated(f: u64) -> bool {
    (f >> 45) & 1 != 0
}
pub const fn counted(f: u64) -> bool {
    (f >> 46) & 1 != 0
}
/// Whether its view is probed by a parent or a node's own id (`v_parent`): a join's finds are then not kept.
pub const fn parented(f: u64) -> bool {
    (f >> 47) & 1 != 0
}
pub const fn version(f: u64) -> u64 {
    f >> 56
}

/// The form of stage record `s` (the module's doc).
fn form_of(s: &Stage) -> u64 {
    // A filter's candidate is its binding (`St::candidates`), whatever its keys.
    let probe = if s.nkeys == 0 || s.mode == 3 {
        P_ROWS
    } else if s.v_has != 0 {
        if s.v_has == 1 { P_CLIMB } else { P_SUBTREE }
    } else if s.v_parent == 2 {
        P_OWN
    } else if s.v_parent != 0 {
        P_CHILDREN
    } else {
        P_INDEX
    };
    let read = match (s.v_has, s.r_view) {
        (0, 0) => R_REL,
        (0, _) => R_VIEW,
        (1, _) => R_HAS1,
        _ => R_HAS2,
    };
    let nkeys = s.nkeys.min(8);
    let keys = (0..nkeys).fold(0u64, |a, i| a | u64::from(s.key_kind[i as usize] == 1) << i);
    let screen = s.screen != 0 && s.screen <= s.nkeys && s.screen <= 8;
    u64::from(s.mode.min(15))
        | u64::from(probe) << 4
        | u64::from(read) << 7
        | u64::from(nkeys) << 9
        | keys << 13
        | u64::from(s.r_dead != 0) << 21
        | u64::from(screen) << 22
        | u64::from(s.ntests.min(8)) << 23
        | u64::from(s.nops.min(64)) << 27
        | u64::from(s.nbinds.min(16)) << 34
        | u64::from(s.npassed.min(16)) << 39
        | u64::from(s.r_len != 0) << 44
        | u64::from(s.last != NONE) << 45
        | u64::from(s.counter != NONE) << 46
        | u64::from(s.v_parent != 0) << 47
        | FORM_VERSION << 56
}

/// The form of the stage record at `p` (the host's encoding of it, `metal_join::Stage`, whose layout is this one's).
///
/// # Safety
///
/// `p` is a stage record's first byte, aligned as one.
pub unsafe fn form_of_record(p: *const u8) -> u64 {
    unsafe { form_of(&*(p as *const Stage)) }
}

/// The parts a stage of form `f` is made of, by name (the module's doc).
pub fn part_names(f: u64) -> Vec<&'static str> {
    let walk = match (probe(f), read(f)) {
        (P_ROWS, R_REL) => "Scan<Rel>",
        (P_ROWS, R_VIEW) => "ScanKind<view>",
        (P_ROWS, _) => "Scan<has>",
        (P_INDEX, R_REL) => "ByValue<Rel>",
        (P_INDEX, _) => "ByValue<view>",
        (P_OWN, _) => "OwnRead",
        (P_CHILDREN, _) => "Children",
        (P_CLIMB, _) => "Climb",
        _ => "Subtree",
    };
    let mut parts = match mode(f) {
        1 if probe(f) == P_INDEX => vec!["Hash"],
        2 if probe(f) == P_INDEX => vec!["Hash<NEG>"],
        1 => vec![walk, "Semi"],
        2 => vec![walk, "Semi<NEG>"],
        3 => vec!["Cmp"],
        4 => vec![walk, "Lattice"],
        5 => vec![walk, "Witness"],
        6 => vec!["Text<names>"],
        7 => vec!["Text<textop>"],
        _ => vec![walk],
    };
    if nops(f) > 0 {
        parts.push("Ops");
    }
    if nbinds(f) > 0 {
        parts.push("Binds");
    }
    parts.push("Emit");
    parts
}

/// A stage of a class: its form and its parts' work, each of the program's item kinds a stage's job runs.
pub struct StageClass {
    pub form: u64,
    /// Threads [t0, t1) of a job of kind `kind` (`PK_*`); false where its parts have no work of that kind.
    run: unsafe fn(kind: u32, st: &St, j: &Job, b: &Bufs, t0: u32, t1: u32) -> bool,
}

impl StageClass {
    pub fn parts(&self) -> Vec<&'static str> {
        part_names(self.form)
    }
}

/// The parts of form `F`, as a class's stage.
pub const fn stage<const F: u64>() -> StageClass {
    StageClass { form: F, run: run::<F> }
}

/// A shape's class: its key and name, and its stages' forms as the runs had them.
pub struct Class {
    pub key: u64,
    pub name: &'static str,
    pub stages: &'static [&'static StageClass],
}

mod table;
pub use table::CLASSES;

/// The class registered for the shape of key `key`.
pub fn class_of(key: u64) -> Option<&'static Class> {
    CLASSES.iter().find(|c| c.key == key)
}

/// The classes a launch is run with (`join_run_classed`): their stages by form, and the jobs' ranges each ran. Its
/// groups may run on several threads at once (the CPU device's): what it counts is counted atomically.
pub struct Classes {
    stages: Vec<&'static StageClass>,
    /// The stage jobs' ranges its classes ran, those whose work a class leaves to the program (a text's), and those
    /// of stage records no class fits.
    ran: [core::sync::atomic::AtomicU64; 3],
    /// The forms of the stage records no class fit (each once).
    unfit: std::sync::Mutex<Vec<u64>>,
}

impl Classes {
    /// The classes of `keys` (a launch's runs' shapes); the first key with none is an error.
    pub fn of(keys: &[u64]) -> Result<Classes, u64> {
        let mut stages: Vec<&'static StageClass> = Vec::new();
        for &k in keys {
            let c = class_of(k).ok_or(k)?;
            for s in c.stages {
                if !stages.iter().any(|x| x.form == s.form) {
                    stages.push(s);
                }
            }
        }
        Ok(Classes { stages, ran: Default::default(), unfit: Default::default() })
    }

    /// The ranges its classes ran, left to the program, and of no class (`ran`).
    pub fn ran(&self) -> [u64; 3] {
        self.ran.each_ref().map(|a| a.load(Relaxed))
    }

    /// The forms of the stage records no class fit.
    pub fn unfit(&self) -> Vec<u64> {
        self.unfit.lock().map_or_else(|e| e.into_inner().clone(), |u| u.clone())
    }

    /// The class of stage record `s`: its form found among its stages' (a range's few loads and compares, beside the
    /// range's work).
    fn of_record(&self, s: *const Stage) -> Option<&'static StageClass> {
        // SAFETY: a job's stage record, which the launch's buffers hold.
        let f = form_of(unsafe { &*s });
        let c = self.stages.iter().copied().find(|c| c.form == f);
        if c.is_none() {
            let mut u = self.unfit.lock().unwrap_or_else(|e| e.into_inner());
            if !u.contains(&f) {
                u.push(f);
            }
        }
        c
    }

    /// Threads [t0, t1) of job `j` of kind `kind` by its stage's class: false where none fits (the program runs them).
    pub(super) unsafe fn range(&self, kind: u32, j: &Job, b: &Bufs, t0: u32, t1: u32) -> bool {
        let rec = match kind {
            PK_STAGE_COUNT | PK_SCAN_COUNT | PK_WITNESS_COUNT | PK_SCAN_SCATTER => j.s,
            PK_STAGE_SCATTER => j.ws,
            _ => return false,
        };
        let Some(c) = self.of_record(rec) else {
            self.ran[2].fetch_add(1, Relaxed);
            return false;
        };
        let st = St::new(unsafe { &*rec }, b.l, j.r, j.rf);
        let done = unsafe { (c.run)(kind, &st, j, b, t0, t1) };
        self.ran[if done { 0 } else { 1 }].fetch_add(1, Relaxed);
        done
    }
}

// ---- The parts: a stage's reads, its candidates, its tests, its row program, its rows out, each of form `F`. ----

/// Column `col` of candidate row `r` (`St::rget`, its way of reading fixed).
#[inline(always)]
unsafe fn rget<const F: u64>(st: &St, r: u32, col: u32) -> i64 {
    let s = st.s;
    if read(F) == R_REL {
        return unsafe { rel_get(st.r as *const u8, s.r_at, r, col, st.far) as i64 };
    }
    let has = read(F) == R_HAS1 || read(F) == R_HAS2;
    let col = if has { 1 } else { col };
    let mut f = st.found.borrow_mut();
    if let (q, Some(row)) = st.last.get()
        && q == r
    {
        return unsafe { row.get(col, &mut f) };
    }
    let row = unsafe { if has { vrow(1, s.v_dir, r, &f) } else { vrow(s.r_view, s.v_dir, r, &f) } };
    st.last.set((r, Some(row)));
    unsafe { row.get(col, &mut f) }
}

#[inline(always)]
unsafe fn row_live<const F: u64>(st: &St, r: u32) -> bool {
    !dead(F) || unsafe { rel_dead(st.r as *const u8, st.s.r_dead, r) } == 0
}

#[inline(always)]
fn key_value<const F: u64>(st: &St, t: u32, k: u32) -> i64 {
    let s = st.s;
    // SAFETY: a binding's slot, as the source reads it.
    if key_slot(F, k) { unsafe { st.slot(s.key_slot[k as usize], t) } } else { s.key_const[k as usize] }
}

#[inline(always)]
unsafe fn keys_are<const F: u64>(st: &St, r: u32, kv: &[i64; 8]) -> bool {
    (0..nkeys(F)).all(|i| unsafe { rget::<F>(st, r, st.s.key_rcol[i as usize]) } == kv[i as usize])
}

/// Whether candidate `r` has the binding's keys and passes the relation's column tests (`St::matches`).
#[inline(always)]
unsafe fn matches<const F: u64>(st: &St, r: u32, kv: &[i64; 8], key: bool) -> bool {
    if read(F) == R_HAS1 {
        return ntests(F) == 0;
    }
    if read(F) == R_HAS2 {
        return ntests(F) == 0 && unsafe { st.has_member(r) };
    }
    if !unsafe { row_live::<F>(st, r) } {
        return false;
    }
    let s = st.s;
    (key || unsafe { keys_are::<F>(st, r, kv) }) && (0..ntests(F)).all(|i| unsafe { rget::<F>(st, r, s.test_a[i as usize]) == rget::<F>(st, r, s.test_b[i as usize]) })
}

#[inline(always)]
unsafe fn rows<const F: u64>(st: &St) -> u32 {
    let s = st.s;
    if !live(F) {
        return s.nr;
    }
    let len = unsafe { rel_len(st.r as *const u8, s.r_at) };
    if len > s.r_from { (len - s.r_from).min(s.nr) } else { 0 }
}

/// The candidates of keys `kv` (`St::probe`): a relation's rows and an index's group of form `F`'s; the tape's walks
/// the program's.
#[inline(always)]
unsafe fn probe_of<const F: u64>(st: &St, kv: &[i64; 8]) -> Walk {
    let s = st.s;
    match probe(F) {
        P_ROWS => Walk { lo: s.r_from, hi: s.r_from.wrapping_add(unsafe { rows::<F>(st) }), how: 0, tag: 0 },
        P_INDEX => {
            let h = key_hash(&kv[..nkeys(F) as usize]);
            let g = unsafe { rel_group(st.r as *const u8, s.r_at, s.r_index, h) };
            let lo = (g >> 32) as u32;
            Walk { lo, hi: lo.wrapping_add(g as u32), how: 1, tag: mark_tag(h) }
        }
        P_OWN => unsafe { st.by_own(kv) },
        _ => unsafe { st.probe(kv) },
    }
}

/// A binding's candidates and its keys' values (`St::candidates`).
#[inline(always)]
unsafe fn candidates<const F: u64>(st: &St, t: u32, f: *const u8, kv: &mut [i64; 8]) -> Walk {
    let s = st.s;
    if mode(F) == 3 {
        return Walk { lo: 0, hi: 1, how: 0, tag: 0 };
    }
    for i in 0..nkeys(F) {
        kv[i as usize] = key_value::<F>(st, t, i);
    }
    if screened(F) {
        let h = bloom_hash(kv[s.screen as usize - 1] as u64);
        if unsafe { bloom_may(f, s.screen_at as u64, h) } == 0 {
            return Walk { lo: 0, hi: 0, how: 0, tag: 0 };
        }
    }
    unsafe { probe_of::<F>(st, kv) }
}

/// The candidates of keys `kv` in their walk's order (`St::each`): each one's row, place, and whether its keys are
/// known to be `kv`.
#[inline(always)]
unsafe fn each<const F: u64>(st: &St, w: Walk, kv: &[i64; 8], mut f: impl FnMut(u32, u32, bool) -> bool) {
    let s = st.s;
    let mut q = w.lo;
    if matches!(probe(F), P_CHILDREN | P_CLIMB) && (w.how == 2 || w.how == 3) && q != NONE {
        let known = st.parent_known(w.how);
        unsafe { st.walk_tape(w.how, q, |r| f(r, r, known)) };
        return;
    }
    if probe(F) != P_INDEX {
        while q < w.hi {
            if !f(q, q, false) {
                return;
            }
            q = q.wrapping_add(1);
        }
        return;
    }
    // An index group's (decision 0671): the places of the key's tag, each one's key known by the last of the tag.
    let mut last = 0u32;
    while q < w.hi {
        let m = unsafe { rel_group_mark(st.r as *const u8, s.r_at, s.r_index, q) };
        if (m ^ w.tag) & !MARK_SAME != 0 {
            q = q.wrapping_add(1);
            continue;
        }
        let row = unsafe { rel_group_row(st.r as *const u8, s.r_at, s.r_index, q) };
        let key = if last != 0 && m & MARK_SAME != 0 { last == 1 } else { unsafe { keys_are::<F>(st, row, kv) } };
        last = if key { 1 } else { 2 };
        if key && !f(row, q, true) {
            return;
        }
        q = q.wrapping_add(1);
    }
}

#[inline(always)]
unsafe fn cand_row<const F: u64>(st: &St, q: u32) -> u32 {
    if probe(F) == P_INDEX { unsafe { rel_group_row(st.r as *const u8, st.s.r_at, st.s.r_index, q) } } else { q }
}

#[inline(always)]
unsafe fn colval<const F: u64>(st: &St, k: u32, t: u32, r: u32) -> i64 {
    let (s, k) = (st.s, k as usize);
    let v = if s.col_kind[k] == 0 { unsafe { rget::<F>(st, r, s.col_idx[k]) } } else { unsafe { st.slot(s.col_idx[k], t) } };
    if s.col_doc[k] != 0 { v >> 32 } else { v }
}

#[inline(always)]
unsafe fn arith<const F: u64>(st: &St, j: u32, t: u32, r: u32) -> Option<i64> {
    let a = &st.s.arith[j as usize];
    let x = if a[3] & 1 != 0 { unsafe { st.parval(a[1], t) } } else { unsafe { colval::<F>(st, a[1], t, r) } };
    let y = if a[3] & 2 != 0 { unsafe { st.parval(a[2], t) } } else { unsafe { colval::<F>(st, a[2], t, r) } };
    match a[0] {
        0 => x.checked_add(y),
        1 => x.checked_sub(y),
        2 => x.checked_mul(y),
        3 => x.checked_div(y),
        4 => x.checked_rem(y),
        5 => Some(x.max(y)),
        6 => Some(x.min(y)),
        _ => unsafe { node_order(st.s.v_dir, x, &mut { NONE }) },
    }
}

#[inline(always)]
unsafe fn defined<const F: u64>(st: &St, t: u32, r: u32) -> bool {
    let s = st.s;
    (0..nbinds(F) as usize).all(|b| s.bind_kind[b] != 3 || unsafe { arith::<F>(st, s.bind_idx[b], t, r) }.is_some())
}

/// The row program (`St::eval`), of `nops(F)` operations.
#[inline(always)]
unsafe fn eval<const F: u64>(st: &St, t: u32, r: u32) -> u64 {
    let s = st.s;
    let mut v = 0u64;
    for x in 0..nops(F) {
        let o = &s.ops[x as usize];
        let bit = |k: u32| (v >> (k & 63)) & 1;
        let b = match o[0] {
            0 => {
                let w = (o[3] >> 8) & 255;
                let a = lane_of(unsafe { colval::<F>(st, o[1], t, r) }, w);
                let c = if o[2] & 0x8000_0000 != 0 { unsafe { st.parval(o[2] & 0xffff, t) } } else { unsafe { colval::<F>(st, o[2], t, r) } };
                let c = lane_of(c, w);
                match o[3] & 255 {
                    0 => a == c,
                    1 => a != c,
                    2 => a < c,
                    _ => a <= c,
                }
            }
            1 => o[1] != 0,
            2 => bit(o[1]) == 0,
            3 => bit(o[1]) & bit(o[2]) != 0,
            4 => bit(o[1]) | bit(o[2]) != 0,
            5 => bit(o[1]) ^ bit(o[2]) != 0,
            _ => bit(o[1]) & !bit(o[2]) & 1 != 0,
        };
        v |= (b as u64) << x;
    }
    v
}

/// Whether a candidate's row program lets it out (its last output).
#[inline(always)]
fn let_out<const F: u64>(st: &St, v: u64) -> bool {
    !gated(F) || st.out_bit(v, st.s.last)
}

/// Binding `t`'s row `r`'s binds out at `at` of `o` (`St::write_binds`).
#[inline(always)]
unsafe fn write_binds<const F: u64>(st: &St, t: u32, r: u32, o: *mut i64, at: u32) {
    let s = st.s;
    for b in 0..nbinds(F) as usize {
        let i = s.bind_idx[b];
        let v = match s.bind_kind[b] {
            3 => unsafe { arith::<F>(st, i, t, r) }.unwrap_or(0),
            0 => unsafe { colval::<F>(st, i, t, r) },
            1 => unsafe { st.parval(i, t) },
            _ => unsafe { rget::<F>(st, r, i) },
        };
        unsafe { col_put(o, s.bind_slot[b], s.cap_out, at, v) };
    }
}

/// `Emit`: binding `t`'s row `r` out at `at` of `o`, its slots then its binds (`St::write_row`).
#[inline(always)]
unsafe fn write_row<const F: u64>(st: &St, t: u32, r: u32, o: *mut i64, at: u32) {
    let s = st.s;
    for k in 0..s.nslots {
        unsafe { col_put(o, k, s.cap_out, at, st.slot(k, t)) };
    }
    unsafe { write_binds::<F>(st, t, r, o, at) };
}

/// `Lattice`: an aggregate over the binding's matching rows (`St::aggregate`).
unsafe fn aggregate<const F: u64>(st: &St, t: u32, w: Walk, kv: &[i64; 8]) -> Result<i64, u32> {
    let s = st.s;
    let kind = s.agg[0] & 0xff;
    let (mut cnt, mut sum, mut mn, mut mx, mut any, mut over) = (0i64, 0i64, 0i64, 0i64, false, false);
    unsafe {
        each::<F>(st, w, kv, |r, _, key| {
            if matches::<F>(st, r, kv, key) {
                cnt += 1;
                if s.agg[1] != NONE {
                    let x = rget::<F>(st, r, s.agg[1]);
                    let (y, o) = sum.overflowing_add(x);
                    over |= o;
                    sum = y;
                    mn = if any { mn.min(x) } else { x };
                    mx = if any { mx.max(x) } else { x };
                    any = true;
                }
            }
            true
        })
    };
    if kind == 1 && over {
        return Err(2);
    }
    if (kind == 2 || kind == 3) && !any {
        return Err(1);
    }
    let v = match kind {
        0 => cnt,
        1 => sum,
        2 => mn,
        _ => mx,
    };
    if (s.agg[0] >> 8) & 1 == 0 && unsafe { st.slot(s.bind_slot[0], t) } != v {
        return Err(1);
    }
    Ok(v)
}

// ---- A stage's work, each of its item kinds, as the program's (`stage_count_one`, ...) of form `F`. ----

/// The parts' work of form `F` for threads [t0, t1) of a job of kind `kind`.
unsafe fn run<const F: u64>(kind: u32, st: &St, j: &Job, b: &Bufs, t0: u32, t1: u32) -> bool {
    // Its text work is the program's (`Text<names>`, `Text<textop>`): a text's reads, not its stage's branches.
    if mode(F) == 6 || mode(F) == 7 {
        return false;
    }
    unsafe {
        match kind {
            PK_STAGE_COUNT => count_range::<F>(st, j, b, t0, t1),
            PK_STAGE_SCATTER => scatter_range::<F>(st, j, b, t0, t1),
            PK_SCAN_COUNT => scan_count_range::<F>(st, j, b, t0, t1),
            PK_SCAN_SCATTER => scan_scatter_range::<F>(st, j, b, t0, t1),
            PK_WITNESS_COUNT => witness_count_range::<F>(st, j, b, t0, t1),
            _ => return false,
        }
    }
    true
}

unsafe fn count_range<const F: u64>(st: &St, j: &Job, b: &Bufs, t0: u32, t1: u32) {
    let s = st.s;
    let t1 = t1.min(s.cap_in.saturating_add(1));
    if t0 >= t1 {
        return;
    }
    let n = unsafe { (*j.totals.add(s.in_slot as usize)).min(s.cap_in) };
    unsafe { fill_u32(b.c, t0.max(n), t1, 0) };
    let mut acc = [0u32; 17];
    for t in t0..t1.min(n) {
        unsafe { count_one::<F>(st, j, b, t, &mut acc) };
    }
    if counted(F) {
        for (m, &a) in acc.iter().enumerate().filter(|(_, a)| **a > 0) {
            unsafe { at(j.counters.add(s.counter as usize + m)).fetch_add(a, SeqCst) };
        }
    }
}

/// Binding `t`'s rows out, counted (`stage_count_one`).
#[inline(always)]
unsafe fn count_one<const F: u64>(st: &St, j: &Job, b: &Bufs, t: u32, acc: &mut [u32; 17]) {
    let s = st.s;
    let counts = b.c;
    let mut kv = [0i64; 8];
    let w = unsafe { candidates::<F>(st, t, j.f, &mut kv) };
    if mode(F) == 4 {
        let got = unsafe { aggregate::<F>(st, t, w, &kv) };
        if got == Err(2) && counted(F) {
            unsafe { at(j.counters.add(s.counter as usize)).store(1, SeqCst) };
        }
        unsafe { *counts.add(t as usize) = got.is_ok() as u32 };
        return;
    }
    let exists = mode(F) == 1 || mode(F) == 2;
    let (mut c, mut p0, mut found) = (0u32, 0u32, false);
    let mut q_counts = [0u32; 16];
    let passed = npassed(F) as usize;
    // Its probe is a relation's rows or an index's group (`how` at most 1) where it is no walk of the tape.
    let kept = mode(F) == 0 && matches!(read(F), R_REL | R_VIEW) && !parented(F) && w.hi.wrapping_sub(w.lo) <= FINDS_MAX;
    let mut finds = 0u32;
    unsafe {
        each::<F>(st, w, &kv, |r, q, key| {
            if if mode(F) == 3 { !row_live::<F>(st, r) } else { !matches::<F>(st, r, &kv, key) } {
                return true;
            }
            if exists {
                found = true;
                return false;
            }
            p0 += 1;
            let ok = defined::<F>(st, t, r);
            let v = if nops(F) > 0 { eval::<F>(st, t, r) } else { 0 };
            for (q, &bit) in q_counts.iter_mut().zip(&s.passed).take(passed) {
                *q += (ok && (bit == NONE || st.out_bit(v, bit))) as u32;
            }
            if ok && let_out::<F>(st, v) {
                if kept {
                    finds |= 1 << (q.wrapping_sub(w.lo) & 31);
                }
                c += 1;
            }
            true
        })
    };
    if exists {
        c = (found != (mode(F) == 2)) as u32;
    } else if mode(F) == 0 {
        unsafe { *b.v.add(t as usize) = if kept { (u64::from(w.lo) | u64::from(finds) << 32) as i64 } else { FINDS_WALK } };
    }
    unsafe { *counts.add(t as usize) = c };
    if !exists {
        acc[0] += p0;
        for (a, &q) in acc[1..].iter_mut().zip(&q_counts).take(passed) {
            *a += q;
        }
    }
}

unsafe fn scatter_range<const F: u64>(st: &St, j: &Job, b: &Bufs, t0: u32, t1: u32) {
    let s = st.s;
    let (offsets, o) = (b.c, b.o);
    let o0 = unsafe { *offsets };
    let n = unsafe { (*j.totals.add(s.in_slot as usize)).min(s.cap_in) };
    if t0 == 0 && t1 > 0 {
        unsafe { *j.totals.add(s.out_slot as usize) = (*offsets.add(n as usize)).wrapping_sub(o0) };
    }
    if mode(F) == 1 || mode(F) == 2 {
        // A binding kept as it is: its slots a run's copy of each column, its binds a row at a time.
        unsafe {
            kept_runs(offsets, o0, t0, t1.min(n), |t, a, len| {
                let len = below(a, len, s.cap_out);
                for k in 0..s.nslots {
                    copy_col(colp(st.l, k, s.cap_in, t), colp_mut(o, k, s.cap_out, a), len, false);
                }
                for i in 0..len {
                    write_binds::<F>(st, t + i, 0, o, a + i);
                }
            })
        };
        return;
    }
    for t in t0..t1.min(n) {
        unsafe { scatter_one::<F>(st, j, b, t, o0) };
    }
}

/// Binding `t`'s rows out written (`stage_scatter_one`).
#[inline(always)]
unsafe fn scatter_one<const F: u64>(st: &St, j: &Job, b: &Bufs, t: u32, o0: u32) {
    let s = st.s;
    let (offsets, o) = (b.c, b.o);
    let mut a = unsafe { (*offsets.add(t as usize)).wrapping_sub(o0) };
    if a >= s.cap_out {
        return;
    }
    let kept = unsafe { (*offsets.add(t as usize + 1)).wrapping_sub(o0) } > a;
    if mode(F) == 4 {
        if kept {
            let mut kv = [0i64; 8];
            let w = unsafe { candidates::<F>(st, t, j.f, &mut kv) };
            let v = unsafe { aggregate::<F>(st, t, w, &kv) };
            unsafe { write_row::<F>(st, t, 0, o, a) };
            if (s.agg[0] >> 8) & 1 != 0 {
                unsafe { col_put(o, s.bind_slot[0], s.cap_out, a, v.unwrap_or(0)) };
            }
        }
        return;
    }
    if mode(F) == 0 && unsafe { *b.v.add(t as usize) } as u32 != NONE {
        // Its finds (`count_one`'s): its rows out, each found once, none walked again.
        let found = unsafe { *b.v.add(t as usize) } as u64;
        let (first, mut bits) = (found as u32, (found >> 32) as u32);
        while bits != 0 && a < s.cap_out {
            let q = first + bits.trailing_zeros();
            unsafe { write_row::<F>(st, t, if nkeys(F) == 0 { q } else { cand_row::<F>(st, q) }, o, a) };
            bits &= bits - 1;
            a += 1;
        }
        return;
    }
    let mut kv = [0i64; 8];
    let w = unsafe { candidates::<F>(st, t, j.f, &mut kv) };
    unsafe {
        each::<F>(st, w, &kv, |r, _, key| {
            if a >= s.cap_out {
                return false;
            }
            if if mode(F) == 3 { !row_live::<F>(st, r) } else { !matches::<F>(st, r, &kv, key) } {
                return true;
            }
            if !defined::<F>(st, t, r) || (gated(F) && !st.out_bit(eval::<F>(st, t, r), s.last)) {
                return true;
            }
            write_row::<F>(st, t, r, o, a);
            a += 1;
            true
        })
    };
}

/// A scan's keys and candidates (`St::scan_cands`).
#[inline(always)]
unsafe fn scan_cands<const F: u64>(st: &St, j: &Job) -> ([i64; 8], ScanCands) {
    let s = st.s;
    let mut kv = [0i64; 8];
    for i in 0..nkeys(F) as usize {
        kv[i] = if key_slot(F, i as u32) { unsafe { col(st.l, s.key_slot[i], s.cap_in, 0) } } else { s.key_const[i] };
    }
    let rows = if unsafe { *j.totals.add(s.in_slot as usize) } == 0 { 0 } else { unsafe { rows::<F>(st) } };
    if !matches!(probe(F), P_INDEX | P_OWN) || rows == 0 {
        return (kv, ScanCands { lo: s.r_from, n: rows, tag: 0, grouped: false });
    }
    let w = unsafe { probe_of::<F>(st, &kv) };
    (kv, ScanCands { lo: w.lo, n: w.hi.wrapping_sub(w.lo).min(rows), tag: w.tag, grouped: probe(F) == P_INDEX })
}

#[inline(always)]
unsafe fn scan_cand<const F: u64>(st: &St, c: &ScanCands, r: u32) -> Option<u32> {
    if r >= c.n {
        return None;
    }
    if probe(F) != P_INDEX {
        return Some(c.lo.wrapping_add(r));
    }
    let s = st.s;
    let q = c.lo.wrapping_add(r);
    let m = unsafe { rel_group_mark(st.r as *const u8, s.r_at, s.r_index, q) };
    if (m ^ c.tag) & !MARK_SAME != 0 {
        return None;
    }
    Some(unsafe { rel_group_row(st.r as *const u8, s.r_at, s.r_index, q) })
}

unsafe fn scan_count_range<const F: u64>(st: &St, j: &Job, b: &Bufs, t0: u32, t1: u32) {
    let s = st.s;
    let t1 = t1.min(s.nr.saturating_add(1));
    if t0 >= t1 {
        return;
    }
    let (kv, cands) = unsafe { scan_cands::<F>(st, j) };
    unsafe { fill_u32(b.c, t0.max(cands.n), t1, 0) };
    for r in t0..t1.min(cands.n) {
        let Some(id) = (unsafe { scan_cand::<F>(st, &cands, r) }) else {
            unsafe { *b.c.add(r as usize) = 0 };
            continue;
        };
        let mut c = 0;
        if unsafe { matches::<F>(st, id, &kv, false) } {
            let v = if nops(F) > 0 { unsafe { eval::<F>(st, 0, id) } } else { 0 };
            if counted(F) {
                unsafe { at(j.counters.add(s.counter as usize)).fetch_add(1, SeqCst) };
                for m in 0..npassed(F) as usize {
                    if s.passed[m] == NONE || st.out_bit(v, s.passed[m]) {
                        unsafe { at(j.counters.add(s.counter as usize + 1 + m)).fetch_add(1, SeqCst) };
                    }
                }
            }
            c = let_out::<F>(st, v) as u32;
        }
        unsafe { *b.c.add(r as usize) = c };
    }
}

unsafe fn scan_scatter_range<const F: u64>(st: &St, j: &Job, b: &Bufs, r0: u32, r1: u32) {
    let s = st.s;
    let offsets = b.c;
    let o0 = unsafe { *offsets };
    let rows = unsafe { rows::<F>(st) };
    if r0 == 0 && r1 > 0 {
        unsafe { *j.totals.add(s.out_slot as usize) = (*offsets.add(rows as usize)).wrapping_sub(o0) };
    }
    let (_, cands) = unsafe { scan_cands::<F>(st, j) };
    unsafe {
        kept_runs(offsets, o0, r0, r1.min(rows).min(cands.n), |r, a, len| {
            for i in 0..below(a, len, s.cap_out) {
                let id = scan_cand::<F>(st, &cands, r + i).expect("a kept candidate is one of the scan's");
                write_row::<F>(st, 0, id, b.o, a + i);
            }
        })
    };
}

unsafe fn witness_count_range<const F: u64>(st: &St, j: &Job, b: &Bufs, t0: u32, t1: u32) {
    let s = st.s;
    let t1 = t1.min(s.cap_in.saturating_add(1));
    let n = unsafe { (*j.totals.add(s.in_slot as usize)).min(s.cap_in) };
    unsafe { fill_u32(b.c, t0.max(n), t1, 0) };
    let nb = s.agg[0].min(3) as usize;
    for t in t0..t1.min(n) {
        let mut kv = [0i64; 8];
        let w = unsafe { candidates::<F>(st, t, j.f, &mut kv) };
        let mut found = 0;
        unsafe {
            each::<F>(st, w, &kv, |r, _, key| {
                if !matches::<F>(st, r, &kv, key) {
                    return true;
                }
                let all = (0..nb).all(|k| witness_probe(&*j.bs.add(k), st.l, t, s.cap_in, st, r, j.w[k], j.wf[k]));
                if all {
                    found = 1;
                }
                !all
            })
        };
        unsafe { *b.c.add(t as usize) = found };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_records_form_names_the_fields_its_parts_fix() {
        // SAFETY: a record of zeros is a stage's (null addresses, no rows).
        let mut s: Stage = unsafe { core::mem::zeroed() };
        s.mode = 1;
        s.nkeys = 2;
        s.key_kind = [1, 0, 0, 0, 0, 0, 0, 0];
        s.r_dead = 64;
        s.ntests = 1;
        s.nops = 3;
        s.nbinds = 2;
        s.npassed = 1;
        s.last = NONE;
        s.counter = 4;
        let f = form_of(&s);
        assert_eq!((mode(f), probe(f), read(f), nkeys(f)), (1, P_INDEX, R_REL, 2));
        assert!(key_slot(f, 0) && !key_slot(f, 1) && dead(f) && !screened(f) && !live(f) && !gated(f) && counted(f) && !parented(f));
        assert_eq!((ntests(f), nops(f), nbinds(f), npassed(f), version(f)), (1, 3, 2, 1, FORM_VERSION));
        assert_eq!(part_names(f), ["Hash", "Ops", "Binds", "Emit"]);
        // A node's own row by its id; a climb; a filter's candidate its binding whatever its keys.
        s.v_parent = 2;
        s.r_view = 1;
        assert_eq!((probe(form_of(&s)), read(form_of(&s)), parented(form_of(&s))), (P_OWN, R_VIEW, true));
        s.v_has = 1;
        assert_eq!((probe(form_of(&s)), read(form_of(&s))), (P_CLIMB, R_HAS1));
        s.mode = 3;
        assert_eq!(probe(form_of(&s)), P_ROWS);
        // A class's stage is its form's parts.
        const S: StageClass = stage::<0x0100_0000_0000_0200>();
        assert_eq!(S.form, 0x0100_0000_0000_0200);
        assert!(class_of(0).is_none());
    }
}
