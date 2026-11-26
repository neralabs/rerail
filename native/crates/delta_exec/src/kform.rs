//! K-form: a program as rows of nine relations (decision 0686; Bootstrap from one tape, §3). Each row's first column
//! is its module, the document its rules were read from: the partition column. Rule and term ids are that document's
//! node ids, local to it (L9): they renumber on any edit, and the module is the grain.
//!
//! - `k_rule(m, rule, head)`: a rule of module `m`, writing relation `head`;
//! - `k_step(m, rule, k, j, kind, rel)`: a step of its body, at key `(k, j)`: kind `atom`, `not` or `cmp` (`rel` a
//!   `cmp`'s operator). Steps run in key order. As read, step `k` is at `(k, -1)`; a lowering that places a filter
//!   after atom `p` puts it at `(p, f)`, with no arithmetic (the rebuild plan, O2);
//! - `k_arg(m, rule, k, j, i, term)`: the step's `i`-th argument; `(-1, -1)` is the head;
//! - `k_var(m, term, name)`, `k_str(m, term, s)`, `k_num(m, term, n)`, `k_wild(m, term)`: what a term is;
//! - `k_order(m, k)`: module `m`'s stratum, its file's place in the bootstrap (`boot(path, k)`): strata are files;
//! - `k_decl(m, rel, i, type)`: module `m` declares relation `rel` (`.decl`, K1-d), its `i`-th column of type `type`.
//!   A step or a head of another arity is refused, as a relation of two declarations that differ is; K0 declares
//!   none.
//!   A lattice (0686, L4-i) is two more places, which no column has: `k_decl(m, L, -1, f)`, lattice `L`'s join is the
//!   function `f` (`.lattice L<base> { Lub -> @f }`); `k_decl(m, rel, -2, k)`, relation `rel` is declared `widen k`.
//!   A relation whose last column's type is `flat`, `max`, `min` or a lattice keeps one row a key (`phase::Lattice`).
//!
//! What a program means is fixed by §3's five sentences, and nothing else is: the executor never reorders, rewrites
//! or stratifies a rule.

/// The K-form relations, by name, with their arities.
pub const RELATIONS: [(&str, usize); 9] = [
    ("k_rule", 3),
    ("k_step", 6),
    ("k_arg", 6),
    ("k_var", 3),
    ("k_str", 3),
    ("k_num", 3),
    ("k_wild", 2),
    ("k_order", 2),
    ("k_decl", 4),
];

/// The K-form relation at `RELATIONS[i]`'s trace shape: partitioned by module, sorted in column order, every
/// column 32 bits (module and node ids, keys, places, symbols) but a number's value (`k_num`'s last) and a name
/// (`k_rule`'s head, `k_step`'s relation, `k_var`'s variable): a stored id takes the 32 bits it fits in, a value the
/// program writes may need 64, and a name is a symbol or a node's name (`tape::name`, `-(d·2³² + n)`), which does not
/// fit 32.
pub fn shape(i: usize) -> rerail_blocks::layer::Shape {
    use rerail_blocks::layer::{ColType, Shape};
    let (name, arity) = RELATIONS[i];
    let wide = |c: usize| matches!((name, c), ("k_num", 2) | ("k_rule", 2) | ("k_step", 5) | ("k_var", 2));
    let columns = (0..arity).map(|c| if wide(c) { ColType::I64 } else { ColType::I32 }).collect();
    Shape::run(columns, (0..arity as u16).collect(), Some(0))
}

/// Why a program cannot run as written: its module and rule (node ids), and what.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refused {
    pub module: i64,
    pub rule: i64,
    pub why: String,
}

impl Refused {
    /// The refusal with its module named, by its document's path (B§7: "A refusal names the rule's module and
    /// node").
    pub fn named(&self, module: &str) -> String {
        format!("module {module} rule {}: {}", self.rule, self.why)
    }
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "module {} rule {}: {}", self.module, self.rule, self.why)
    }
}
