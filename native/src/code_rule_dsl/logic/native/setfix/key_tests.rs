//! A relation's declared keys checked where its rows are written (decision 0361 step 6).
use super::*;

/// A key of any columns (`key(c)`, column 1 of three) is checked as rows are pushed: rows of distinct keys keep it, a
/// row repeated is no second row, and a second row of a key breaks it, reported once (`broke`) and checked no more.
/// Another key of the relation (columns 0 and 2) is checked on its own.
#[test]
fn a_second_row_of_a_declared_key_breaks_it() {
    let mut rel = Rel::new("t", b"iii");
    rel.set_key(vec![1]).unwrap();
    rel.set_key(vec![0, 2]).unwrap();
    assert!(rel.push(&[1, 10, 0]) && rel.push(&[2, 20, 0]) && rel.push(&[3, 30, 1]));
    assert!(!rel.push(&[1, 10, 0]), "a row repeated is no new row");
    assert!(rel.broke.is_empty() && rel.keyed.len() == 2, "every key held");
    // Column 1's 10 again, under other values.
    assert!(rel.push(&[4, 10, 2]));
    assert_eq!(rel.broke, vec![vec![1]]);
    assert_eq!(rel.keyed.iter().map(|k| k.cols.clone()).collect::<Vec<_>>(), vec![vec![0, 2]], "the other key still checked");
    // Checked no more: another repeat of the broken key reports nothing new.
    assert!(rel.push(&[5, 10, 3]));
    assert_eq!(rel.broke, vec![vec![1]]);
    // The other key broken in turn: (1, 0) again.
    assert!(rel.push(&[1, 99, 0]));
    assert_eq!(rel.broke, vec![vec![1], vec![0, 2]]);
    assert!(rel.keyed.is_empty());
    // Declared again, a broken key is not checked again; a key of a column the relation lacks is refused.
    rel.set_key(vec![1]).unwrap();
    assert!(rel.keyed.is_empty());
    assert!(rel.set_key(vec![3]).is_err());
}

/// A key of no columns (layouts' fact for a rule whose constants fix its one row, Q409's `cpl_policy_doc`) holds
/// while the relation has one row, a row repeated being none, and a second row breaks it.
#[test]
fn a_key_of_no_columns_is_one_row_at_most() {
    let mut rel = Rel::new("t", b"i");
    rel.set_key(Vec::new()).unwrap();
    assert!(rel.push(&[7]) && !rel.push(&[7]));
    assert!(rel.broke.is_empty() && rel.keyed.len() == 1, "one row keeps it");
    assert!(rel.push(&[8]));
    assert_eq!(rel.broke, vec![Vec::<usize>::new()], "a second row breaks it");
}

/// The rows a relation holds when a key is declared are checked first; and a key whose row died is the next row's
/// (a maintained run's removal of a row, then the add of another of its key).
#[test]
fn a_declared_key_checks_the_rows_held_and_a_dead_rows_key_is_free() {
    let mut rel = Rel::new("t", b"ii");
    assert!(rel.push(&[1, 10]) && rel.push(&[2, 10]));
    rel.set_key(vec![1]).unwrap();
    assert_eq!(rel.broke, vec![vec![1]], "the rows held break it as it is declared");
    let mut rel = Rel::new("t", b"ii");
    rel.set_key(vec![1]).unwrap();
    assert!(rel.push(&[1, 10]));
    rel.dead = rows::BlockVec::from_iter_in(None, [1u64]);
    rel.ndead = 1;
    assert!(rel.push(&[2, 10]));
    assert!(rel.broke.is_empty(), "the key's row died: the new row takes it");
    assert!(rel.push(&[3, 10]));
    assert_eq!(rel.broke, vec![vec![1]]);
}
