//! The maintain mode of a non-recursive component (decision 0364 stage 3): removal joins over the versions of what
//! changed below, a recheck of each candidate with its head bound, and the add joins as seeds.
use super::spec::hex;
use super::*;

fn connection() -> Connection {
    let path = crate::code_rule_dsl::fixtures::temp();
    drop(crate::code_rule_dsl::fixtures::open(&path));
    let db = Connection::open(path).unwrap();
    crate::code_rule_dsl::logic::native::register(&db).unwrap();
    db
}

fn rel_of(rows: &[&[i64]], types: &[u8]) -> InRel {
    let mut r = Rel::new("x", types);
    for row in rows {
        r.push(row);
    }
    InRel::moved(r)
}

/// `rows`, and 40 more no rule here reads (from 1,000 on): a start from it keeps few enough dead to be in place.
fn padded(rows: &[&[i64]], types: &[u8]) -> InRel {
    let mut r = Rel::new("x", types);
    for row in rows {
        r.push(row);
    }
    for i in 0..40 {
        r.push(&vec![1000 + i; types.len()]);
    }
    InRel::moved(r)
}

/// `live`, less the padding.
fn unpadded(rows: Vec<Vec<i64>>) -> Vec<Vec<i64>> {
    rows.into_iter().filter(|r| r[0] < 1000).collect()
}

fn live(rel: &InRel) -> Vec<Vec<i64>> {
    let mut out: Vec<Vec<i64>> = rel.live_rows().expect("flat").chunks_exact(rel.k).map(<[i64]>::to_vec).collect();
    out.sort();
    out
}

fn rows(rel: &InRel) -> Vec<Vec<i64>> {
    let mut out: Vec<Vec<i64>> = (0..rel.len()).map(|i| { let r = rel.at(i); (0..rel.k).map(|c| r.at(c)).collect() }).collect();
    out.sort();
    out
}

/// `h(x, y) :- a(x), e(x, y).  h(x, y) :- b(x, y).` with `a` maintained below (2 gone, 4 added) and `b`, `e`
/// unchanged. The rows `a`'s 2 derived are candidates; (2, 20) stays, as `b` derives it, and (2, 21) is killed in
/// place; (4, 40) is added. The relation is its kept one, its versions the change: nothing else is made.
#[test]
fn a_maintained_component_kills_the_candidates_no_rule_derives_and_adds_the_new_rows() {
    let db = connection();
    let a0 = Arc::new(padded(&[&[1], &[2], &[3]], b"i"));
    let mut a = Rel::based("mm_a", b"i", &a0, Some(&rel_of(&[&[2]], b"i"))).expect("few dead");
    a.push(&[4]);
    adopt(&db, "mm_a", Arc::new(InRel::moved(a)));
    adopt(&db, "mm_e", Arc::new(rel_of(&[&[1, 10], &[2, 20], &[2, 21], &[3, 30], &[4, 40]], b"ii")));
    adopt(&db, "mm_b", Arc::new(rel_of(&[&[2, 20]], b"ii")));
    adopt(&db, "mm_h_kept", Arc::new(padded(&[&[1, 10], &[2, 20], &[2, 21], &[3, 30]], b"ii")));
    let v = |ver: &str, t: &str| format!("v:{ver}:{}", hex(t));
    let r = |t: &str| format!("r:{}", hex(t));
    // A removal per changed atom (a's rows gone over e, unchanged, as it is), the rechecks of both rules, an add per changed atom.
    let remove = format!("pos {} v0\npos {} v0 v1\nhead v0 v1", v("m", "mm_a"), r("mm_e"));
    let recheck_a = format!("pos {} v0\npos {} v0 v1\nhead v0 v1", r("mm_a"), r("mm_e"));
    let recheck_b = format!("pos {} v0 v1\nhead v0 v1", r("mm_b"));
    let add = format!("pos {} v0\npos {} v0 v1\nhead v0 v1", v("p", "mm_a"), r("mm_e"));
    let spec = format!(
        "name mm_h\nrel mm_h ii base {} -\nnowrite\nremove 0 {}\nrecheck 0 {}\nrecheck 0 {}\nseed 0 {}",
        hex("mm_h_kept"), hex(&remove), hex(&recheck_a), hex(&recheck_b), hex(&add)
    );
    open(&db, &spec).unwrap();
    run(&db, "mm_h", 0).unwrap();
    // Each line's work by its plan steps, for the planning budget (Q145): the removal read a's row gone and found
    // its two candidates; each recheck ran; the counts restart once read.
    let [removes, adds, rechecks] = maint_step_counts(&db, "mm_h").unwrap();
    let worked = |line: &Vec<(i64, i64)>| line.iter().any(|&(i, o)| i > 0 || o > 0);
    assert_eq!((removes.len(), adds.len(), rechecks.len()), (1, 0, 2), "one removal, two rechecks");
    assert!(worked(&removes[0]), "the removal's steps counted: {removes:?}");
    assert!(rechecks.iter().all(worked), "each recheck's steps counted: {rechecks:?}");
    let [removes, _, rechecks] = maint_step_counts(&db, "mm_h").unwrap();
    assert!(!removes.iter().chain(&rechecks).any(worked), "read once: {removes:?} {rechecks:?}");
    close(&db, "mm_h").unwrap();
    let h = held(&db, "mm_h").expect("the maintained relation");
    assert_eq!(unpadded(live(&h)), vec![vec![1, 10], vec![2, 20], vec![3, 30], vec![4, 40]]);
    assert_eq!(rows(&h.version_view(Ver::Minus).expect("a version")), vec![vec![2, 21]], "only the row with no derivation left is gone");
    assert_eq!(rows(&h.version_view(Ver::Plus).expect("a version")), vec![vec![4, 40]]);
    let kept = held(&db, "mm_h_kept").expect("kept");
    assert!(h.rows.shares(&kept.rows), "it is its kept relation, killed and added to in place");
}

/// A negated atom's rows gone drive a checked add: `h(x) :- s(x), !n(x).` with 2 gone from `n` and 3 gone and
/// added again (in its minus and its plus both). 2 is added; 3 is not, as `n` holds it still.
#[test]
fn a_checked_add_adds_only_what_a_rule_derives_now() {
    let db = connection();
    let n0 = Arc::new(padded(&[&[2], &[3], &[5]], b"i"));
    let mut n = Rel::based("mn_n", b"i", &n0, Some(&rel_of(&[&[2], &[3]], b"i"))).expect("few dead");
    n.push(&[3]);
    adopt(&db, "mn_n", Arc::new(InRel::moved(n)));
    adopt(&db, "mn_s", Arc::new(rel_of(&[&[1], &[2], &[3], &[5]], b"i")));
    adopt(&db, "mn_h_kept", Arc::new(padded(&[&[1]], b"i")));
    let (v, r) = (|ver: &str, t: &str| format!("v:{ver}:{}", hex(t)), |t: &str| format!("r:{}", hex(t)));
    let add = format!("pos {} v0\npos {} v0\nhead v0", v("m", "mn_n"), r("mn_s"));
    let recheck = format!("pos {} v0\nneg {} v0\nhead v0", r("mn_s"), r("mn_n"));
    open(&db, &format!("name mn_h\nrel mn_h i base {} -\nnowrite\naddchecked 0 {}\nrecheck 0 {}", hex("mn_h_kept"), hex(&add), hex(&recheck))).unwrap();
    run(&db, "mn_h", 0).unwrap();
    close(&db, "mn_h").unwrap();
    assert_eq!(unpadded(live(&held(&db, "mn_h").expect("held"))), vec![vec![1], vec![2]]);
}

/// A recheck reads only the rules whose heads can name the candidate: `h(1, x) :- a(x).  h(2, x) :- b(x).` with 5
/// gone from `a` and `b` holding 5. The candidate (1, 5) is killed: `b`'s rule derives (2, 5), not it.
#[test]
fn a_recheck_takes_no_rule_whose_head_constants_differ_from_the_candidate() {
    let db = connection();
    let a0 = Arc::new(padded(&[&[5], &[6]], b"i"));
    let a = Rel::based("mc_a", b"i", &a0, Some(&rel_of(&[&[5]], b"i"))).expect("few dead");
    adopt(&db, "mc_a", Arc::new(InRel::moved(a)));
    adopt(&db, "mc_b", Arc::new(rel_of(&[&[5]], b"i")));
    adopt(&db, "mc_h_kept", Arc::new(padded(&[&[1, 5], &[1, 6], &[2, 5]], b"ii")));
    let (v, r) = (|ver: &str, t: &str| format!("v:{ver}:{}", hex(t)), |t: &str| format!("r:{}", hex(t)));
    let remove = format!("pos {} v0\nhead i:1 v0", v("m", "mc_a"));
    let recheck_a = format!("pos {} v0\nhead i:1 v0", r("mc_a"));
    let recheck_b = format!("pos {} v0\nhead i:2 v0", r("mc_b"));
    open(&db, &format!("name mc_h\nrel mc_h ii base {} -\nnowrite\nremove 0 {}\nrecheck 0 {}\nrecheck 0 {}", hex("mc_h_kept"), hex(&remove), hex(&recheck_a), hex(&recheck_b))).unwrap();
    run(&db, "mc_h", 0).unwrap();
    close(&db, "mc_h").unwrap();
    assert_eq!(unpadded(live(&held(&db, "mc_h").expect("held"))), vec![vec![1, 6], vec![2, 5]]);
}

/// A recursive component's drivers report their work (Q145's gap): `p(x, y) :- e(x, y).  p(x, z) :- p(x, y), e(y, z).`
/// with (2, 3) gone from `e` below. The removal's candidate p(2, 3) is killed, and the driver made from the delta
/// probe (its join less `p`) finds the rows that read it from the row: e(3, z), so p(2, 4). Its counts are given
/// with the probe it is made from, the second (after the seed), whose kept plan its work goes to.
#[test]
fn a_recursive_components_driver_reports_its_work_under_its_probe() {
    let db = connection();
    let e0 = Arc::new(padded(&[&[1, 2], &[2, 3], &[3, 4]], b"ii"));
    let e = Rel::based("mm_re", b"ii", &e0, Some(&rel_of(&[&[2, 3]], b"ii"))).expect("few dead");
    adopt(&db, "mm_re", Arc::new(InRel::moved(e)));
    adopt(&db, "mm_rp_kept", Arc::new(padded(&[&[1, 2], &[2, 3], &[3, 4], &[1, 3], &[2, 4], &[1, 4]], b"ii")));
    let v = |ver: &str, t: &str| format!("v:{ver}:{}", hex(t));
    let r = |t: &str| format!("r:{}", hex(t));
    let remove = format!("pos {} v0 v1\nhead v0 v1", v("m", "mm_re"));
    let add = format!("pos {} v0 v1\nhead v0 v1", v("p", "mm_re"));
    let recheck_e = format!("pos {} v0 v1\nhead v0 v1", r("mm_re"));
    let recheck_p = format!("pos c0 v0 v1\npos {} v1 v2\nhead v0 v2", r("mm_re"));
    let probe = format!("pos d0 v0 v1\npos {} v1 v2\nhead v0 v2", r("mm_re"));
    let spec = format!(
        "name mm_rp\nrel mm_rp ii base {} -\nnowrite\nleveled 0\nkeptold\nremove 0 {}\nrecheck 0 {}\nrecheck 0 {}\nseed 0 {}\njoin 0 0 {}",
        hex("mm_rp_kept"), hex(&remove), hex(&recheck_e), hex(&recheck_p), hex(&add), hex(&probe)
    );
    open(&db, &spec).unwrap();
    run(&db, "mm_rp", 0).unwrap();
    let drivers = driver_step_counts(&db, "mm_rp").unwrap();
    let worked = |line: &Vec<(i64, i64)>| line.iter().any(|&(i, o)| i > 0 || o > 0);
    assert_eq!(drivers.iter().map(|d| d.0).collect::<Vec<_>>(), vec![1], "one driver, made from the second probe: {drivers:?}");
    assert!(worked(&drivers[0].1), "the driver's steps counted: {drivers:?}");
    assert!(!driver_step_counts(&db, "mm_rp").unwrap().iter().any(|d| worked(&d.1)), "read once");
    close(&db, "mm_rp").unwrap();
}
