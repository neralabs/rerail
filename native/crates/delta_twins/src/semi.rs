//! `semi_change` and `change_rows` (decision 0686; `blocks::change::semi` as an operation over blocks): a trace's
//! change, each of its rows with its sign, into a δ runs block, read where its layers lie.
//!
//! - `semi_change`: the change of `A ⋉ B` on `A.a_col = B.b_col` (`A ▷ B`, its rows whose key `B` does not hold,
//!   with `neg`), `A` and `B` each between two states of its trace: a row of `A` gone that matched before is gone;
//!   one added that matches after is added; a kept row whose key `B`'s change made match is added, and one whose key
//!   it made match no more is gone. And `B`'s change projected on its key, into a second runs block: a key `B` holds
//!   now and held in no row before +1, one it held and holds no more −1, each once.
//! - `change_rows`: `A`'s change alone (`A ⋉ B` with every key matching): each row gone (its before state's values,
//!   −1) and each added (+1).
//!
//! Both list the keys of the rows of one sign in a parts block (`drop::put_part`, then sealed): of the rows gone, the
//! partitions the change retracts; of the rows added, those it names.
//!
//! - L1's selection is `semi_change` of its tape port with its want (on the document), or `change_rows` of its tape
//!   port with no want: the parts are the documents it retracts (−1), the added rows those it reads, and the keys'
//!   change its asks of the tape port (P§7.1's demand out).
//! - The boot's forgetting is `semi_change` of the document set, cold, with no ask for it (`neg`): the parts are the
//!   documents nothing asks for (+1), which `drop_parts` retracts from the set and its places.
//!
//! Slots: `A`'s after head and `B`'s (read; the layers below each, the before state's among them, reached by
//! reference), the rows' runs block, the parts block and the keys' runs block (write). Nothing is gathered: a key is
//! taken once by its first changed row of `B` with that key, found by a probe of the state it is in.

use crate::drop::{put_part, seal_parts};
use crate::{Ctx, Tile, TwinPhase};
use rerail_blocks::bistate::{Bistate, ColumnIndex, Read, RowId};
use rerail_blocks::gens::Table;
use rerail_blocks::layer::{Runs, runs_claim_in, runs_set_in};
use rerail_blocks::relation::View;

crate::params! {
    /// The traces' heads (block ids in the descriptor table; `NONE`: the empty state), their arities and the columns
    /// joined on.
    pub struct SemiP {
        pub a_before: u64,
        pub a_after: u64,
        pub b_before: u64,
        pub b_after: u64,
        pub a_arity: u32,
        pub b_arity: u32,
        pub a_col: u32,
        pub b_col: u32,
        /// 1: `A ▷ B`, the rows of `A` whose key `B` does not hold.
        pub neg: u32,
        /// The sign whose rows' keys the parts block lists: −1, the partitions the change retracts (L1's); +1, those it
        /// adds (the documents a cold `A ▷ B` names, the boot's forgetting).
        pub parts_sign: i32,
    }
}

/// No head: the empty state.
pub const NONE: u64 = u64::MAX;

/// `semi_change`'s slots.
pub const A_HEAD: usize = 0;
pub const B_HEAD: usize = 1;
pub const ROWS: usize = 2;
pub const PARTS: usize = 3;
pub const KEYS: usize = 4;

/// `change_rows`' slots: `A_HEAD`, then these.
pub const C_ROWS: usize = 1;
pub const C_PARTS: usize = 2;

/// The most columns a row written has (a row read into registers).
pub const MAX_COLS: usize = 16;

/// The state at `head` of a trace of `arity` columns, through the launch's table; `None` for `NONE`.
fn state<'t>(table: &'t Table<'t>, head: u64, arity: usize) -> Option<View<'t>> {
    (head != NONE).then(|| View::chain(table, Some(head as u32), arity).expect("a trace's state, its layers reached by reference"))
}

/// The bistate between `before` and `after`; `None` when both are empty.
fn bistate<'t>(table: &'t Table<'t>, before: u64, after: u64, arity: usize) -> Option<Bistate<View<'t>>> {
    let a = state(table, after, arity)?;
    Some(match state(table, before, arity) {
        None => Bistate::cold(a),
        Some(b) => Bistate::new(b, a),
    })
}

/// Whether row `id` of `s` is kept: live before and after.
fn kept(s: &Bistate<View<'_>>, id: RowId) -> bool {
    s.before().is_some_and(|b| id < Read::end(b) && Read::live(b, id)) && Read::live(s.after(), id)
}

/// Whether state `s` holds a row whose column `col` is `k`.
fn has(s: Option<&View<'_>>, col: ColumnIndex, k: i64) -> bool {
    s.is_some_and(|s| Read::matching(s, &[col], &[k]).next().is_some())
}

/// `f` of each of `B`'s keys on `col` that its change made held (`up`) or no longer held, once: at its first row of the
/// state it is held in, all of which are changed rows (a kept row would hold it in both states).
fn flipped(b: &Bistate<View<'_>>, col: ColumnIndex, up: bool, f: &mut dyn FnMut(i64)) {
    let (to, from) = if up { (Some(b.after()), b.before()) } else { (b.before(), Some(b.after())) };
    let Some(to) = to else { return };
    let mut one = |id: RowId| {
        let k = Read::get(to, id, col);
        if !has(from, col, k) && Read::matching(to, &[col], &[k]).next() == Some(id) {
            f(k);
        }
    };
    if up { b.added().for_each(&mut one) } else { b.gone().for_each(&mut one) }
}

/// Each row of `A`'s change in `A ⋉ B` (`A ▷ B` with `neg`; `b`: none, every key matches), with its sign and the
/// state its values are read from.
fn walk<'t>(a: &Bistate<View<'t>>, b: Option<&Bistate<View<'t>>>, ac: ColumnIndex, bc: ColumnIndex, neg: bool, f: &mut dyn FnMut(i32, &View<'t>, RowId)) {
    let matches = |s: Option<&View<'_>>, k: i64| b.is_none() || has(s, bc, k) != neg;
    if let Some(ab) = a.before() {
        for id in a.gone() {
            if matches(b.and_then(Bistate::before), Read::get(ab, id, ac)) {
                f(-1, ab, id);
            }
        }
    }
    for id in a.added() {
        if matches(b.map(Bistate::after), Read::get(a.after(), id, ac)) {
            f(1, a.after(), id);
        }
    }
    let (Some(b), Some(_)) = (b, a.before()) else { return };
    // A key `B` came to hold: its kept rows come into `A ⋉ B` (go from `A ▷ B`); one it no longer holds, the reverse.
    for (up, sign) in [(true, 1), (false, -1)] {
        let sign = if neg { -sign } else { sign };
        flipped(b, bc, up, &mut |k| {
            for id in Read::matching(a.after(), &[ac], &[k]) {
                if kept(a, id) {
                    f(sign, a.after(), id);
                }
            }
        });
    }
}

/// A walk of the change, given what to do with each row.
type Walk<'w, 't> = dyn Fn(&mut dyn FnMut(i32, &View<'t>, RowId)) + 'w;

/// The walk written: its rows into `rows` (counted, claimed at once, then written in order), and the key in `ac` of
/// each row of sign `parts_sign` into the open parts block `parts`, sealed (sorted, each once).
fn write(rows: &mut [u8], parts: &mut [u8], arity: usize, ac: ColumnIndex, parts_sign: i32, go: &Walk<'_, '_>) {
    assert!(arity <= MAX_COLS, "a row of at most {MAX_COLS} columns");
    let mut n = 0u32;
    go(&mut |_, _, _| n += 1);
    let mut at = runs_claim_in(rows, n);
    let (mut row, mut np) = ([0i64; MAX_COLS], 0usize);
    go(&mut |sign, s, id| {
        if let Some(a) = at.as_mut() {
            for (k, v) in row[..arity].iter_mut().enumerate() {
                *v = Read::get(s, id, ColumnIndex::new(k as u16));
            }
            runs_set_in(rows, *a, &row[..arity], sign);
            *a += 1;
        }
        if sign == parts_sign {
            put_part(parts, np, Read::get(s, id, ac));
            np += 1;
        }
    });
    seal_parts(parts, np);
}

/// `semi_change`'s one item.
fn semi_change(c: &mut Ctx<'_, '_, '_>, _: Tile) {
    let p = <SemiP as crate::Params>::read(c.params);
    let (ac, bc) = (ColumnIndex::new(p.a_col as u16), ColumnIndex::new(p.b_col as u16));
    let table = c.table;
    let a = bistate(table, p.a_before, p.a_after, p.a_arity as usize);
    let b = bistate(table, p.b_before, p.b_after, p.b_arity as usize);
    let ([], [rows, parts, keys]) = c.views([], [ROWS, PARTS, KEYS]);
    match (&a, &b) {
        (Some(a), Some(b)) => write(rows, parts, p.a_arity as usize, ac, p.parts_sign, &|f| walk(a, Some(b), ac, bc, p.neg != 0, f)),
        // `A` holds nothing, or nothing is wanted, on either side: no row, no partition.
        _ => {
            seal_parts(parts, 0);
        }
    }
    let Some(b) = b else { return };
    let mut n = 0u32;
    for up in [true, false] {
        flipped(&b, bc, up, &mut |_| n += 1);
    }
    let Some(mut at) = runs_claim_in(keys, n) else { return };
    for (up, sign) in [(true, 1), (false, -1)] {
        flipped(&b, bc, up, &mut |k| {
            runs_set_in(keys, at, &[k], sign);
            at += 1;
        });
    }
}

/// `change_rows`' one item.
fn change_rows(c: &mut Ctx<'_, '_, '_>, _: Tile) {
    let p = <SemiP as crate::Params>::read(c.params);
    let ac = ColumnIndex::new(p.a_col as u16);
    let table = c.table;
    let a = bistate(table, p.a_before, p.a_after, p.a_arity as usize);
    let ([], [rows, parts]) = c.views([], [C_ROWS, C_PARTS]);
    match &a {
        Some(a) => write(rows, parts, p.a_arity as usize, ac, p.parts_sign, &|f| walk(a, None, ac, ac, false, f)),
        None => {
            seal_parts(parts, 0);
        }
    }
}

/// `semi_change`: one item.
pub const SEMI_CHANGE: &[TwinPhase] = &[TwinPhase { name: "semi", items: |_| 1, run: semi_change, again: None }];

/// `change_rows`: one item.
pub const CHANGE_ROWS: &[TwinPhase] = &[TwinPhase { name: "change", items: |_| 1, run: change_rows, again: None }];

crate::params! {
    /// `replace_rows`: the state's head (`NONE`: empty), its arity, and how many leading columns are the key.
    pub struct ReplaceP {
        pub head: u64,
        pub arity: u32,
        pub key: u32,
    }
}

/// `replace_rows`' slots: the state's head (read; the layers below by reference), the rows it is given (read: a runs
/// block, its +1 rows, each key once) and its change (write: a δ runs block).
pub const R_HEAD: usize = 0;
pub const R_GIVEN: usize = 1;
pub const R_RUNS: usize = 2;

/// Each change of the state by the rows given, by key: a given row the state holds
/// as it is changes nothing; else the state's rows of its key go (−1) and it comes (+1).
fn replaced<'t>(s: Option<&View<'t>>, given: &Runs<'_>, arity: usize, key: usize, f: &mut dyn FnMut(i32, Option<(&View<'t>, RowId)>, usize)) {
    let cols: [ColumnIndex; MAX_COLS] = core::array::from_fn(|k| ColumnIndex::new(k as u16));
    let mut k = [0i64; MAX_COLS];
    for i in (0..given.len()).filter(|&i| given.diff(i) > 0) {
        for (c, v) in k[..key].iter_mut().enumerate() {
            *v = given.get(i, c);
        }
        let same = |id: RowId| (0..arity).all(|c| Read::get(s.expect("a state"), id, cols[c]) == given.get(i, c));
        let held = s.is_some_and(|s| Read::matching(s, &cols[..key], &k[..key]).any(same));
        if held {
            continue;
        }
        if let Some(s) = s {
            for id in Read::matching(s, &cols[..key], &k[..key]) {
                f(-1, Some((s, id)), i);
            }
        }
        f(1, None, i);
    }
}

/// `replace_rows`' one item: counted, claimed at once, then written in order.
fn replace_rows(c: &mut Ctx<'_, '_, '_>, _: Tile) {
    let p = <ReplaceP as crate::Params>::read(c.params);
    let (arity, key) = (p.arity as usize, p.key as usize);
    assert!(arity <= MAX_COLS && key <= arity, "a row of at most {MAX_COLS} columns, its key among them");
    let s = state(c.table, p.head, arity);
    let ([given], [runs]) = c.views([R_GIVEN], [R_RUNS]);
    let given = Runs::new(given);
    let mut n = 0u32;
    replaced(s.as_ref(), &given, arity, key, &mut |_, _, _| n += 1);
    let Some(mut at) = runs_claim_in(runs, n) else { return };
    let mut row = [0i64; MAX_COLS];
    replaced(s.as_ref(), &given, arity, key, &mut |sign, old, i| {
        for (k, v) in row[..arity].iter_mut().enumerate() {
            *v = match old {
                Some((s, id)) => Read::get(s, id, ColumnIndex::new(k as u16)),
                None => given.get(i, k),
            };
        }
        runs_set_in(runs, at, &row[..arity], sign);
        at += 1;
    });
}

/// `replace_rows`: one item.
pub const REPLACE_ROWS: &[TwinPhase] = &[TwinPhase { name: "replace", items: |_| 1, run: replace_rows, again: None }];
