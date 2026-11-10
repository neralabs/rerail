//! rf_kernel_test.cc, in Rust.
//!
//! Jeremy Barnes, 29 December 2018. Copyright (c) 2007 Jeremy Barnes. This file is part of MLDB. Copyright 2015
//! mldb.ai inc. Apache-2.0 (see the LICENSE at `accel/src/forest/mldb/`).
//!
//! Test of the partition index (the header's "tick counter" was stale upstream).

use crate::forest::randomforest_types::PartitionIndex;

#[test]
fn test_partition_index() {
    let none = PartitionIndex::default();
    assert_eq!(none.index, 0);
    assert_eq!(none.path(), "none");
    assert!(none.parent().is_err());
    assert_eq!(none, PartitionIndex::none());
    assert_eq!(none.depth(), -1);

    let root = PartitionIndex::root();
    assert_eq!(root.index, 1);
    assert_eq!(root.path(), "root");
    assert!(root.parent().is_err());
    assert_ne!(root, none);
    assert_eq!(root.depth(), 0);

    let left = root.left_child();
    assert_eq!(left.index, 2);
    assert_eq!(left.depth(), 1);
    assert_eq!(left.parent().unwrap(), root);
    assert_eq!(left.path(), "l");

    let right = root.right_child();
    assert_eq!(right.index, 3);
    assert_eq!(right.depth(), 1);
    assert_eq!(right.parent().unwrap(), root);
    assert_eq!(right.path(), "r");

    let ll = left.left_child();
    let lr = left.right_child();
    let rl = right.left_child();
    let rr = right.right_child();
    assert_eq!(ll.parent().unwrap(), left);
    assert_eq!(lr.parent().unwrap(), left);
    assert_eq!(rl.parent().unwrap(), right);
    assert_eq!(rr.parent().unwrap(), right);
    assert_eq!(ll.depth(), 2);
    assert_eq!(lr.depth(), 2);
    assert_eq!(rl.depth(), 2);
    assert_eq!(rr.depth(), 2);
    assert_eq!(ll.path(), "ll");
    assert_eq!(lr.path(), "lr");
    assert_eq!(rl.path(), "rl");
    assert_eq!(rr.path(), "rr");
    assert_eq!(ll.index, 4);
    assert_eq!(rl.index, 5);
    assert_eq!(lr.index, 6);
    assert_eq!(rr.index, 7);
}
