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
//! - `k_order(m, k)`: module `m`'s stratum, from the load that asked for it (§5): strata are files.
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
