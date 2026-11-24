//! A node's definition unit (`def_unit(n, u)`, core.rerail): the innermost definition holding it, read off the
//! granule table where ids are by definition (`granules`), else climbed on the tree. No row is made: the view's rows
//! are the nodes', and a unit's nodes are walked on the tape.
//!
//! A **unit** is a definition (`granules::DEF`: a function, a class, …) with the wrapper that claims it (`WRAPPER`: a
//! Python decorator's `decorated_definition`), which is the unit's root. A node's unit is the innermost unit whose root
//! is a strict ancestor of it, a wrapper's own children (its decorators, its definition) left to the unit around it:
//! a node inside a decorator is its definition's, the decorator and the definition themselves are not. A node no unit
//! holds is the document root's; the root has none.
//!
//! Those are a granule's own nodes: a definition starts a granule with its wrapper (`granules::cut`), so a node's
//! unit is its granule's definition, the granules of statements climbed past. A document whose granules are all
//! folded into its root (`Table::whole`), or numbered in preorder (no table), is climbed on the tree instead, which
//! gives the same unit (`units_on_granules_are_the_climb`).

use super::granules::{DEF, WRAPPER};
use super::{Dialect, NONE, Projected};

impl Projected {
    fn unit_flags(&self, d: &Dialect, local: u32) -> u8 {
        d.granule_kind(d.code(self.f(), self.node(local)))
    }
    /// The definition wrapper `local` claims (its first definition child), if it is a wrapper that claims one.
    fn claimed(&self, d: &Dialect, local: u32) -> Option<u32> {
        if self.unit_flags(d, local) & WRAPPER == 0 {
            return None;
        }
        self.children(local).find(|&c| self.unit_flags(d, c) & DEF != 0)
    }
    /// The definition of the unit rooted at `local`, if a unit's root is there: a definition no wrapper claims, or a
    /// wrapper's claimed definition.
    fn unit_at(&self, d: &Dialect, local: u32) -> Option<u32> {
        if self.unit_flags(d, local) & DEF != 0 {
            let up = self.parent(local as usize);
            return (up == NONE || self.claimed(d, up) != Some(local)).then_some(local);
        }
        self.claimed(d, local)
    }

    /// Node `local`'s unit (its definition's local id), the root (0) where no unit holds it; `None` for the root.
    pub fn unit_of(&self, d: &Dialect, local: u32) -> Option<u32> {
        if local == 0 {
            return None;
        }
        match self.table().filter(|t| !t.whole()) {
            Some(t) => Some(self.unit_on_granules(d, t, local)),
            None => Some(self.unit_on_tree(d, local)),
        }
    }

    /// `unit_of` on the granule table: the node's granule, or the one around it where the node is the granule's start
    /// or a wrapper start's child; then up the granules to the first a definition starts.
    fn unit_on_granules(&self, d: &Dialect, t: &super::granules::Table, local: u32) -> u32 {
        let x = self.node(local);
        let Some(mut g) = t.granule_of(x) else { return 0 };
        let (s, up) = t.start_up(g);
        let s = self.local_of_node(s);
        if s == local || (self.unit_flags(d, s) & WRAPPER != 0 && self.parent(local as usize) == s) {
            let Some(up) = up else { return 0 };
            g = up;
        }
        loop {
            let (s, up) = t.start_up(g);
            if let Some(u) = self.unit_at(d, self.local_of_node(s)) {
                return u;
            }
            let Some(up) = up else { return 0 };
            g = up;
        }
    }

    /// `unit_of` climbed on the tree: the first strict ancestor that roots a unit, past a wrapper whose own child the
    /// node is.
    pub fn unit_on_tree(&self, d: &Dialect, local: u32) -> u32 {
        let mut a = self.parent(local as usize);
        while a != NONE && a != 0 {
            if let Some(u) = self.unit_at(d, a) {
                // A wrapper's own children are the unit around it's.
                if !(self.unit_flags(d, a) & WRAPPER != 0 && self.parent(local as usize) == a) {
                    return u;
                }
            }
            a = self.parent(a as usize);
        }
        0
    }

    /// The nodes whose unit is `u` (`unit_of`'s inverse), in preorder: `u`'s root's subtree, less the root and a
    /// wrapper root's own children, each nested unit's root (and a wrapper's children) kept and the rest of it
    /// skipped. Nothing where `u` is no unit.
    pub fn each_of_unit(&self, d: &Dialect, u: u32, mut f: impl FnMut(u32)) {
        let root = if u == 0 {
            0
        } else {
            if self.unit_at(d, u) != Some(u) {
                // A claimed definition's root is its wrapper.
                let up = self.parent(u as usize);
                if up == NONE || self.unit_flags(d, u) & DEF == 0 || self.claimed(d, up) != Some(u) {
                    return;
                }
                up
            } else {
                u
            }
        };
        // A wrapper root's own children are the unit around it's: its children's subtrees are walked, not they.
        if root != 0 && self.unit_flags(d, root) & WRAPPER != 0 {
            self.each_child(root, |c| self.walk_unit(d, c + 1, c + 1 + self.size(c as usize), &mut f));
        } else {
            self.walk_unit(d, root + 1, root + 1 + self.size(root as usize), &mut f);
        }
    }

    /// The nodes `lo..end` (a subtree's, in preorder) of the unit around them: each nested unit's root and a wrapper
    /// root's children, the rest of the nested unit skipped.
    fn walk_unit(&self, d: &Dialect, lo: u32, end: u32, f: &mut impl FnMut(u32)) {
        let mut l = lo;
        while l < end {
            f(l);
            if self.unit_at(d, l).is_some() {
                if self.unit_flags(d, l) & WRAPPER != 0 {
                    self.each_child(l, &mut *f);
                }
                l += 1 + self.size(l as usize);
            } else {
                l += 1;
            }
        }
    }
}
