//! `drop_parts`'s checks: a state's rows in the listed partitions, read through its layers where they lie.

use super::*;
use crate::contract::push_layer;
use crate::trace::{RollUp, Trace, TraceOps};
use rerail_blocks::layer::{ColType, Shape};

/// Over two layers, the second killing a row of the first: the live rows of partitions 1 and 3 alone, each with the
/// sign asked, and none of 2 or 4.
#[test]
fn the_live_rows_of_the_listed_partitions_through_every_layer() {
    let mut reg = Registry::default();
    let (traces, ops) = (TraceOps::declare(&mut reg).unwrap(), DropOps::declare(&mut reg).unwrap());
    let mut dir = Directory::default();
    let mut t = Trace::new(&Shape::run(vec![ColType::I32, ColType::I64], vec![0, 1], Some(0))).unwrap().with_roll_up(RollUp::Never);
    let r = |d: i64, v: i64| vec![d, v];
    push_layer(&reg, &traces, &mut dir, &mut t, &[(r(1, 10), 1), (r(1, 11), 1), (r(2, 20), 1), (r(3, 30), 1)]).unwrap();
    let head = push_layer(&reg, &traces, &mut dir, &mut t, &[(r(1, 11), -1), (r(3, 31), 1), (r(4, 40), 1)]).unwrap();
    let runs = t.open(&mut dir, 16);
    // Listed out of order and twice: sealed, each once.
    let parts = Parts::write(&mut dir, 3, |put| [3, 1, 3].into_iter().for_each(put)).unwrap();
    assert_eq!(parts.len(), 2);
    assert_eq!(drop_parts(&reg, &ops, &mut dir, head, &parts, -1, runs).unwrap(), 3);
    parts.release(&mut dir);
    let l = dir.read("t", &[runs]).unwrap();
    let got = drop_facts(&l.table(), 2, l.block(0));
    assert_eq!(got, vec![vec![1, 10, -1], vec![3, 30, -1], vec![3, 31, -1]]);
}

/// Past the runs block's room: the claims past it counted as needed, so the driver runs again at the size it needed.
#[test]
fn past_its_room_the_claims_are_counted_as_needed() {
    let mut reg = Registry::default();
    let (traces, ops) = (TraceOps::declare(&mut reg).unwrap(), DropOps::declare(&mut reg).unwrap());
    let mut dir = Directory::default();
    let mut t = Trace::new(&Shape::run(vec![ColType::I32, ColType::I64], vec![0, 1], Some(0))).unwrap();
    let pairs: Vec<(Vec<i64>, i32)> = (0..10).map(|v| (vec![1, v], 1)).collect();
    let head = push_layer(&reg, &traces, &mut dir, &mut t, &pairs).unwrap();
    let runs = t.open(&mut dir, 4);
    let parts = Parts::write(&mut dir, 1, |put| put(1)).unwrap();
    drop_parts(&reg, &ops, &mut dir, head, &parts, 1, runs).unwrap();
    parts.release(&mut dir);
    let l = dir.read("t", &[runs]).unwrap();
    let r = Runs::new(l.block(0));
    assert!(r.needed() > 0);
    assert_eq!(r.len() + r.needed(), 10);
}
