//! Output digests of the dead-code families: a set's digest maintained by its deltas.
//!
//! Each element is hashed (SHA-256 of a canonical byte form that names strings by their text,
//! not by an interner's ids, so a digest means the same in every process) and the hashes are
//! summed lane by lane (four `u64` lanes, wrapping). Adding an element adds its hash, removing it
//! subtracts it, so a step that changes no element leaves the digest as it was (early cutoff)
//! and a step costs its delta, not the set. Equal sets have equal digests whatever their history.
use crate::code_rule_dsl::facts::y2::dc::{self, St};
use crate::code_rule_dsl::facts::y2::facts::{with, Change, Tab, Val};
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct SetDigest([u64; 4]);

impl SetDigest {
    fn lanes(bytes: &[u8]) -> [u64; 4] {
        let h: [u8; 32] = Sha256::digest(bytes).into();
        std::array::from_fn(|i| u64::from_le_bytes(h[i * 8..i * 8 + 8].try_into().unwrap()))
    }

    pub(crate) fn add(&mut self, bytes: &[u8]) {
        for (a, b) in self.0.iter_mut().zip(Self::lanes(bytes)) {
            *a = a.wrapping_add(b);
        }
    }

    pub(crate) fn remove(&mut self, bytes: &[u8]) {
        for (a, b) in self.0.iter_mut().zip(Self::lanes(bytes)) {
            *a = a.wrapping_sub(b);
        }
    }

    pub(crate) fn toggle(&mut self, bytes: &[u8], added: bool) {
        if added { self.add(bytes) } else { self.remove(bytes) }
    }

    pub(crate) fn bytes(&self) -> [u8; 32] {
        let mut out = [0u8; 32];
        for (i, l) in self.0.iter().enumerate() {
            out[i * 8..i * 8 + 8].copy_from_slice(&l.to_le_bytes());
        }
        out
    }

    /// Every presence change of `changes`, applied.
    pub(crate) fn changes(&mut self, changes: &[Change], buf: &mut Vec<u8>) {
        for c in changes {
            row_bytes(c.tab, &c.row, buf);
            self.toggle(buf, c.added);
        }
    }
}

fn text(s: u32, out: &mut Vec<u8>) {
    with(s, |t| {
        out.extend_from_slice(&(t.len() as u32).to_le_bytes());
        out.extend_from_slice(t.as_bytes());
    });
}

/// A row of Y2's input tables: its table, then each value.
pub(crate) fn row_bytes(tab: Tab, r: &[Val], out: &mut Vec<u8>) {
    out.clear();
    out.push(tab as u8);
    for v in r {
        match v {
            Val::Null => out.push(0),
            Val::Int(i) => {
                out.push(1);
                out.extend_from_slice(&i.to_le_bytes());
            }
            Val::S(s) => {
                out.push(2);
                text(*s, out);
            }
        }
    }
}

/// A product-graph state: its node (a definition, or a module) and its layer (flow, annotated,
/// or the dispatch of a definition), by name.
pub(crate) fn st_bytes(s: St, out: &mut Vec<u8>) {
    let (x, l) = (dc::node(s), dc::layer(s));
    if x & dc::MOD != 0 {
        out.push(b'm');
        text(x & !dc::MOD, out);
    } else {
        out.push(b'd');
        text(x, out);
    }
    match l {
        dc::NONE => out.push(b'f'),
        dc::ANN => out.push(b'a'),
        d => {
            out.push(b'x');
            text(d - 2, out);
        }
    }
}

/// A product-graph edge.
pub(crate) fn edge_bytes(s: St, t: St, out: &mut Vec<u8>) {
    out.clear();
    out.push(b'e');
    st_bytes(s, out);
    st_bytes(t, out);
}

/// A root state of the product graph.
pub(crate) fn seed_bytes(s: St, out: &mut Vec<u8>) {
    out.clear();
    out.push(b'r');
    st_bytes(s, out);
}

/// A reached state.
pub(crate) fn state_bytes(s: St, out: &mut Vec<u8>) {
    out.clear();
    out.push(b's');
    st_bytes(s, out);
}

/// A dead-code finding row.
pub(crate) fn finding_bytes(f: &dc::Finding, out: &mut Vec<u8>) {
    out.clear();
    out.push(b'f');
    text(f.0, out);
    out.extend_from_slice(&f.1.to_le_bytes());
    out.extend_from_slice(&f.2.to_le_bytes());
    text(f.3, out);
    out.extend_from_slice(f.4.as_bytes());
}

/// A worktree member: (path, content), by text.
pub(crate) fn member_bytes(path: &str, content: &str, out: &mut Vec<u8>) {
    out.clear();
    out.extend_from_slice(&(path.len() as u32).to_le_bytes());
    out.extend_from_slice(path.as_bytes());
    out.extend_from_slice(content.as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_set_digest_depends_on_the_set_not_the_order_or_history() {
        let (mut a, mut b) = (SetDigest::default(), SetDigest::default());
        a.add(b"x");
        a.add(b"y");
        b.add(b"y");
        b.add(b"z");
        b.add(b"x");
        assert_ne!(a, b);
        b.remove(b"z");
        assert_eq!(a, b);
        a.remove(b"x");
        a.remove(b"y");
        assert_eq!(a, SetDigest::default());
    }
}
