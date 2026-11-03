use super::*;

/// Two trees in one chunk (kinds 1 = module, 2 = function, 3 = other):
///
/// ```text
/// 0 module            5 module
/// ├ 1 function        └ 6 other
/// │ └ 2 other           └ 7 function
/// │   └ 3 function
/// └ 4 other
/// ```
fn chunk() -> (Vec<u32>, Vec<i64>) {
    (vec![NONE, 0, 1, 2, 0, NONE, 5, 6], vec![1, 2, 3, 2, 3, 1, 3, 2])
}

#[test]
fn nearest_is_each_nodes_closest_ancestor_in_the_set() {
    let (parent, kind) = chunk();
    let trees = Trees { parent: &parent, kind: &kind };
    let sets = [vec![1], vec![2], vec![1, 2]];
    let got = nearest(&trees, &sets, Binding::Cpu).unwrap();
    assert_eq!(got[0], [NONE, 0, 0, 0, 0, NONE, 5, 5]);
    assert_eq!(got[1], [NONE, NONE, 1, 1, NONE, NONE, NONE, NONE]);
    assert_eq!(got[2], [NONE, 0, 1, 1, 0, NONE, 5, 5]);
}

/// The device's work queue solves the same system to the same values (where there is a device).
#[test]
fn the_device_finds_the_cpus_nearest() {
    // A deep tree and a wide one, over three sets.
    let mut parent = vec![NONE];
    let mut kind = vec![1];
    for i in 1..20_000_u32 {
        parent.push(if i % 7 == 0 { i / 2 } else { i - 1 });
        kind.push(i64::from(i % 5));
    }
    let trees = Trees { parent: &parent, kind: &kind };
    let sets = [vec![1], vec![2, 3], vec![4]];
    let cpu = nearest(&trees, &sets, Binding::Cpu).unwrap();
    let metal = nearest(&trees, &sets, Binding::Metal).unwrap();
    assert_eq!(cpu, metal);
    // Climbing parents gives the same answer.
    for (k, set) in sets.iter().enumerate() {
        for node in 0..parent.len() {
            let mut at = parent[node];
            while at != NONE && !set.contains(&kind[at as usize]) {
                at = parent[at as usize];
            }
            assert_eq!(cpu[k][node], at, "set {k} node {node}");
        }
    }
}
