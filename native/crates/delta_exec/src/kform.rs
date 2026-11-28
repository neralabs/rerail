//! K-form: a program as rows of eight relations (decision 0686; Bootstrap from one tape, §3). Each row's first column
//! is its module, the document its rules were read from: the partition column. Rule and term ids are that document's
//! node ids, local to it (L9): they renumber on any edit, and the module is the grain.
//!
//! - `k_rule(m, rule, head)`: a rule of module `m`, writing relation `head`;
//! - `k_step(m, rule, k, j, kind, rel)`: a step of its body, at key `(k, j)`: kind `atom`, `not` or `cmp` (`rel` a
//!   `cmp`'s operator). Steps run in key order. As read, step `k` is at `(k, -1)`; a lowering that places a filter
//!   after atom `p` puts it at `(p, f)`, with no arithmetic (the rebuild plan, O2);
//! - `k_arg(m, rule, k, j, i, term)`: the step's `i`-th argument; `(-1, -1)` is the head;
//! - `k_var(m, term, name)`, `k_str(m, term, s)`, `k_num(m, term, n)`, `k_wild(m, term)`: what a term is;
//! - `k_order(m, k)`: module `m`'s stratum, its file's place in the bootstrap (`boot(path, k)`): strata are files.
//!
//! What a program means is fixed by §3's five sentences, and nothing else is: the executor never reorders, rewrites
//! or stratifies a rule.

/// The K-form relations, by name, with their arities.
pub const RELATIONS: [(&str, usize); 8] = [
    ("k_rule", 3),
    ("k_step", 6),
    ("k_arg", 6),
    ("k_var", 3),
    ("k_str", 3),
    ("k_num", 3),
    ("k_wild", 2),
    ("k_order", 2),
];

/// The K-form relation at `RELATIONS[i]`'s trace shape: partitioned by module, sorted in column order, every
/// column 32 bits (module and node ids, keys, places, symbols) but a number's value, `k_num`'s last: a stored id
/// takes the 32 bits it fits in, and only a value the program writes may need 64.
pub fn shape(i: usize) -> rerail_blocks::layer::Shape {
    use rerail_blocks::layer::{ColType, Shape};
    let (name, arity) = RELATIONS[i];
    let columns = (0..arity).map(|c| if name == "k_num" && c == 2 { ColType::I64 } else { ColType::I32 }).collect();
    Shape::run(columns, (0..arity as u16).collect(), Some(0))
}

/// Why a program cannot run as written: its module and rule (node ids), and what.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refused {
    pub module: i64,
    pub rule: i64,
    pub why: String,
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "module {} rule {}: {}", self.module, self.rule, self.why)
    }
}

/// Each relation's columns that hold a rule's or a term's id: a node id of the module's tape, local to it (L9).
pub const ID_COLS: [&[usize]; 8] = [&[1], &[1], &[1, 5], &[1], &[1], &[1], &[1], &[]];

/// K-form rows (by relation, in `RELATIONS`' order) read by value: each module's ids, which are its own and renumber
/// on any edit (L9, 0686's O3), read as their place among the ids its rows hold, in order. Two programs with the same
/// rules, read in the same order, read the same whatever ids their tapes gave them (a test's reading: what leaves a
/// module is judged by value).
#[cfg(any(test, feature = "test-hooks"))]
pub fn by_value(rows: &[std::collections::BTreeSet<Vec<i64>>]) -> Vec<std::collections::BTreeSet<Vec<i64>>> {
    use std::collections::{BTreeMap, BTreeSet};
    // copy: derived (a test's reading of the rows)
    let mut ids: BTreeMap<i64, BTreeSet<i64>> = BTreeMap::new();
    for (k, rel) in rows.iter().enumerate() {
        for r in rel {
            ids.entry(r[0]).or_default().extend(ID_COLS[k].iter().map(|&c| r[c]));
        }
    }
    let place: BTreeMap<(i64, i64), i64> = ids.iter().flat_map(|(&m, s)| s.iter().enumerate().map(move |(i, &id)| ((m, id), i as i64))).collect();
    rows.iter()
        .enumerate()
        .map(|(k, rel)| {
            rel.iter()
                .map(|r| {
                    let mut v = r.clone();
                    for &c in ID_COLS[k] {
                        v[c] = place[&(r[0], r[c])];
                    }
                    v
                })
                .collect()
        })
        .collect()
}
