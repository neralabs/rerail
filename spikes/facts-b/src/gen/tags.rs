//! The tag tree of the membership model: tag 0 is the per-path base, a cluster tag is a cluster
//! base's delta from its parent, a worktree tag is the worktree's delta from its parent (a
//! cluster, or the base). A tag's records are signed differences from its parent's view; the view
//! of tag t is the sum of the records tagged along `chain(t)` = [0, …, parent(t), t].
use std::cell::RefCell;

pub type W = u32;
pub const BASE: W = 0;

thread_local! {
    static PARENT: RefCell<Vec<W>> = const { RefCell::new(Vec::new()) };
}

/// Attach `t` under `p` (before any record of `t` enters the dataflow).
pub fn set_parent(t: W, p: W) {
    PARENT.with(|x| {
        let mut x = x.borrow_mut();
        if x.len() <= t as usize {
            x.resize(t as usize + 1, BASE);
        }
        x[t as usize] = p;
    })
}

pub fn parent(t: W) -> W {
    if t == BASE {
        return BASE;
    }
    PARENT.with(|x| x.borrow().get(t as usize).copied().unwrap_or(BASE))
}

/// [0, …, parent(t), t] (just [0] for the base).
pub fn chain(t: W) -> Vec<W> {
    let mut c = vec![t];
    let mut x = t;
    while x != BASE {
        x = parent(x);
        c.push(x);
    }
    c.reverse();
    c
}

/// `a` is `b` or one of its ancestors: `a`'s records are part of `b`'s view.
pub fn visible(a: W, b: W) -> bool {
    if a == BASE || a == b {
        return true;
    }
    let mut x = b;
    while x != BASE {
        x = parent(x);
        if x == a {
            return true;
        }
    }
    false
}

/// The tag of a product of a record tagged `a` and one tagged `b` (the bilinear expansion of a
/// join): the deeper of two tags on one chain; none across chains (a worktree never sees another
/// worktree's delta). Both base: base. One base, one delta: the delta (ΔA⋈B_B, A_B⋈ΔB). Both the
/// same delta: that delta (ΔA⋈ΔB).
pub fn product(a: W, b: W) -> Option<W> {
    if a == b && a != BASE && crate::mutant::on(4) {
        return None;
    }
    if visible(a, b) {
        Some(b)
    } else if visible(b, a) {
        Some(a)
    } else {
        None
    }
}
