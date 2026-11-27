//! `semi_change`'s and `change_rows`' checks: over every pair of states of two traces, the rows are
//! `blocks::change::semi`'s (the change form's reference), the parts the keys of the rows it retracts whole, and the
//! keys the change of `B`'s keys, against the two states read cold.

use super::*;
use crate::contract::push_layer;
use crate::drop::Parts;
use crate::trace::{RollUp, Trace};
use rerail_blocks::bistate::{ColumnIndex, Read, RowId};
use rerail_blocks::change::{self, Sign};
use rerail_blocks::layer::{ColType, Shape};
use rerail_blocks::relation::View;
use std::collections::BTreeSet;

type Rows = Vec<(Vec<i64>, i32)>;

/// What the operation wrote: its rows, its parts and its keys.
type Written = (BTreeSet<(Vec<i64>, i32)>, BTreeSet<i64>, BTreeSet<(i64, i32)>);

fn row(s: &View<'_>, id: RowId) -> Vec<i64> {
    (0..Read::arity(s)).map(|c| Read::get(s, id, ColumnIndex::new(c as u16))).collect()
}

/// The keys in column `col` of state `head` of `t`, read cold.
fn keys(dir: &Directory, t: &Trace, head: Option<BlockId>, col: u16) -> BTreeSet<i64> {
    let Some(h) = head else { return BTreeSet::new() };
    let l = t.read(dir, &[Some(h)]).unwrap();
    let table = l.table();
    let s = t.state(&table, Some(h)).unwrap();
    Read::live_ids(&s).map(|id| Read::get(&s, id, ColumnIndex::new(col))).collect()
}

/// A trace of `(doc, value)` rows, partitioned by doc, and one of want rows `(port, doc)`, each over three layers; its
/// heads, the empty state first.
fn traces(reg: &crate::op::Runner, dir: &mut Directory) -> (Trace, Vec<Option<BlockId>>, Trace, Vec<Option<BlockId>>) {
    let mut a = Trace::new(&Shape::run(vec![ColType::I32, ColType::I64], vec![0, 1], Some(0))).unwrap().with_roll_up(RollUp::Never);
    let mut b = Trace::new(&crate::contract::want_shape()).unwrap().with_roll_up(RollUp::Never);
    let r = |d: i64, v: i64| vec![d, v];
    let w = |d: i64| vec![0, d];
    let al: [Rows; 3] = [
        vec![(r(1, 10), 1), (r(2, 20), 1), (r(3, 30), 1), (r(4, 40), 1)],
        // Doc 1 edited, doc 3 gone, doc 5 new.
        vec![(r(1, 10), -1), (r(1, 11), 1), (r(3, 30), -1), (r(5, 50), 1)],
        // Doc 2 edited, doc 3 back.
        vec![(r(2, 20), -1), (r(2, 21), 1), (r(3, 31), 1)],
    ];
    let bl: [Rows; 3] = [
        vec![(w(1), 1), (w(2), 1), (w(3), 1)],
        // 2 no longer wanted, 4 and 5 newly.
        vec![(w(2), -1), (w(4), 1), (w(5), 1)],
        // 2 wanted again, 1 not.
        vec![(w(2), 1), (w(1), -1)],
    ];
    let (mut ah, mut bh) = (vec![None], vec![None]);
    for (x, y) in al.iter().zip(&bl) {
        ah.push(Some(push_layer(reg, dir, &mut a, x).unwrap()));
        bh.push(Some(push_layer(reg, dir, &mut b, y).unwrap()));
    }
    (a, ah, b, bh)
}

fn written(dir: &Directory, rows: BlockId, parts: BlockId, keys: Option<BlockId>) -> Written {
    let runs = |id: BlockId| {
        let l = dir.read("t", &[id]).unwrap();
        let r = Runs::new(l.block(0));
        assert_eq!(r.needed(), 0);
        (0..r.len()).map(|i| ((0..r.head.shape.arity()).map(|c| r.get(i, c)).collect::<Vec<_>>(), r.diff(i))).collect::<Vec<_>>()
    };
    let mut ps = BTreeSet::new();
    Parts::sealed(dir, parts).unwrap().each(dir, &mut |p| assert!(ps.insert(p))).unwrap();
    let ks = keys.map_or(BTreeSet::new(), |k| runs(k).into_iter().map(|(r, d)| (r[0], d)).collect());
    (runs(rows).into_iter().collect(), ps, ks)
}

/// Over every pair of states `i ≤ j` of both traces: `semi_change` gives `blocks::change::semi`'s rows (their values
/// read where they lie), the docs of the rows it retracts, and the change of the wanted docs; as `A ▷ B`, its rows and
/// the docs of the rows it adds; `change_rows` gives the
/// rows of `A`'s change alone and the docs of its gone rows.
#[test]
fn semi_change_is_the_change_forms_semi_join_over_every_pair_of_states() {
    let reg = crate::op::Runner::new().unwrap();
    let mut dir = Directory::default();
    let (a, ah, b, bh) = traces(&reg, &mut dir);
    let asks = Trace::new(&crate::contract::asks_shape()).unwrap();
    let pairs = |n: usize| (0..n).flat_map(move |i| (i..n).map(move |j| (i, j)));
    let mut checked = 0;
    for (i, j) in pairs(ah.len()) {
        for (bi, bj) in pairs(bh.len()) {
            let sa = Side { before: ah[i], after: ah[j], arity: 2, col: 0 };
            let sb = Side { before: bh[bi], after: bh[bj], arity: 2, col: 1 };
            // The reference: `blocks::change::semi` over the same bistates, and `A`'s change read whole.
            let (mut want_rows, mut want_all, mut want_unmatched) = (BTreeSet::new(), BTreeSet::new(), BTreeSet::new());
            if let Some(aj) = ah[j] {
                let heads: Vec<BlockId> = [ah[i], ah[j], bh[bi], bh[bj]].into_iter().flatten().collect();
                let l = dir.read("t", &heads).unwrap();
                let table = l.table();
                let ca = a.change(&table, ah[i], Some(aj)).unwrap();
                let vals = |sign: Sign, id: RowId| match sign {
                    Sign::Gone => (row(ca.before().unwrap(), id), -1),
                    Sign::Added => (row(ca.after(), id), 1),
                };
                if let Some(bjh) = bh[bj] {
                    let cb = b.change(&table, bh[bi], Some(bjh)).unwrap();
                    let on = [(ColumnIndex::new(0), ColumnIndex::new(1))];
                    change::semi(&ca, &cb, &on, false, &mut |s, id| assert!(want_rows.insert(vals(s, id))));
                    change::semi(&ca, &cb, &on, true, &mut |s, id| assert!(want_unmatched.insert(vals(s, id))));
                }
                if let Some(ab) = ca.before() {
                    ca.gone().for_each(|id| assert!(want_all.insert((row(ab, id), -1))));
                }
                ca.added().for_each(|id| assert!(want_all.insert((row(ca.after(), id), 1))));
            }
            // The keys' change, from `B`'s two states read cold; the parts, the docs of the rows gone.
            let (wb, wa) = (keys(&dir, &b, bh[bi], 1), keys(&dir, &b, bh[bj], 1));
            let want_keys: BTreeSet<(i64, i32)> = wa.difference(&wb).map(|&k| (k, 1)).chain(wb.difference(&wa).map(|&k| (k, -1))).collect();
            let gone_docs = |rows: &BTreeSet<(Vec<i64>, i32)>| rows.iter().filter(|(_, d)| *d < 0).map(|(r, _)| r[0]).collect::<BTreeSet<i64>>();

            let (rows, parts, ks) = (a.open(&mut dir, 64), Parts::open(&mut dir, 16).unwrap(), asks.open(&mut dir, 16));
            select(&reg, &mut dir, &sa, Some((&sb, ks)), How::RETRACTED, rows, parts).unwrap();
            let got = written(&dir, rows, parts, Some(ks));
            assert_eq!(got.0, want_rows, "rows, A {i}..{j}, B {bi}..{bj}");
            assert_eq!(got.1, gone_docs(&want_rows), "parts, A {i}..{j}, B {bi}..{bj}");
            assert_eq!(got.2, want_keys, "keys, A {i}..{j}, B {bi}..{bj}");
            [rows, parts, ks].into_iter().for_each(|x| dir.release(x));

            // `A ▷ B`: with `B` empty after, every row of `A`'s change is unmatched (`A`'s change alone); the parts,
            // the docs of the rows it adds.
            if bh[bj].is_none() {
                want_unmatched.clone_from(&want_all);
            }
            let (rows, parts, ks) = (a.open(&mut dir, 64), Parts::open(&mut dir, 16).unwrap(), asks.open(&mut dir, 16));
            select(&reg, &mut dir, &sa, Some((&sb, ks)), How::UNMATCHED, rows, parts).unwrap();
            let got = written(&dir, rows, parts, Some(ks));
            let added_docs = want_unmatched.iter().filter(|(_, d)| *d > 0).map(|(r, _)| r[0]).collect::<BTreeSet<i64>>();
            assert_eq!(got.0, want_unmatched, "unmatched rows, A {i}..{j}, B {bi}..{bj}");
            assert_eq!(got.1, added_docs, "unmatched parts, A {i}..{j}, B {bi}..{bj}");
            [rows, parts, ks].into_iter().for_each(|x| dir.release(x));

            let (rows, parts) = (a.open(&mut dir, 64), Parts::open(&mut dir, 16).unwrap());
            select(&reg, &mut dir, &sa, None, How::RETRACTED, rows, parts).unwrap();
            let got = written(&dir, rows, parts, None);
            assert_eq!(got.0, want_all, "change rows, A {i}..{j}");
            assert_eq!(got.1, gone_docs(&want_all), "change parts, A {i}..{j}");
            [rows, parts].into_iter().for_each(|x| dir.release(x));
            checked += 1;
        }
    }
    assert_eq!(checked, 100);
}

/// `replace_rows` by key: a given row the state holds as it is changes nothing; a changed one's old row goes and it
/// comes; a new key's row comes; over an empty state, the given block is the change itself.
#[test]
fn replace_rows_changes_each_key_whose_row_differs() {
    let reg = crate::op::Runner::new().unwrap();
    let mut dir = Directory::default();
    let mut t = Trace::new(&Shape::run(vec![ColType::I32, ColType::I64, ColType::I64], vec![0, 1, 2], Some(0))).unwrap().with_roll_up(RollUp::Never);
    let given = |dir: &mut Directory, t: &Trace, rows: &[[i64; 3]]| {
        let g = t.open(dir, rows.len());
        let b = dir.block(g).unwrap();
        let pin = rerail_blocks::region::pin_block(&b, "t", true).unwrap();
        for r in rows {
            rerail_blocks::layer::RunsW::new(&pin).push(r, 1).unwrap();
        }
        g
    };
    let g = given(&mut dir, &t, &[[1, 10, 100], [2, 20, 200]]);
    let opened = t.open(&mut dir, 4);
    assert_eq!(replace(&reg, &mut dir, None, 3, 1, g, opened).unwrap(), g);
    dir.release(opened);
    t.commit(&reg, &mut dir, g, crate::op::Tiling::WHOLE).unwrap();
    let g = given(&mut dir, &t, &[[1, 10, 100], [2, 21, 200], [3, 30, 300]]);
    let runs = t.open(&mut dir, 8);
    assert_eq!(replace(&reg, &mut dir, t.head(), 3, 1, g, runs).unwrap(), runs);
    dir.release(g);
    let none = Parts::open(&mut dir, 0).unwrap();
    let got = written(&dir, runs, none, None).0;
    let want: BTreeSet<(Vec<i64>, i32)> = [(vec![2, 20, 200], -1), (vec![2, 21, 200], 1), (vec![3, 30, 300], 1)].into_iter().collect();
    assert_eq!(got, want);
}
