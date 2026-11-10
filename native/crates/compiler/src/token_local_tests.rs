//! The facts of `token_local` (doc 25, step 1): exact facts on small grammars and the checker's
//! catching of wrong claims. Every claim on the `.rr` grammar is checked on its corpus in the
//! `.rr` parser's crate (`rerail_rr_parser`'s `token_local_tests`).

use std::collections::BTreeSet;

use crate::bitlex_masks::{self as bitlex_compile, Named};
use crate::block::BlockProgram;
use crate::token_local::{self, Fact};

/// A small grammar compiled, its block program and its named values; `outputs` are mask names.
struct Small {
    program: BlockProgram,
    values: Vec<Named>,
}

fn small(text: &str, outputs: &[&str]) -> Small {
    let stmts = ::bitlex::syntax::parse(text).expect("parses");
    let compiled = bitlex_compile::compile(&stmts, outputs, None).expect("compiles");
    let program = crate::rir_asm::assemble(&compiled.text()).expect("assembles").block_programs[0].clone();
    Small { program, values: compiled.values }
}

impl Small {
    fn named(&self, name: &str) -> &Named {
        self.values.iter().find(|v| v.name == name).unwrap_or_else(|| panic!("no value `{name}`"))
    }

    /// The first name of operation `op` (names sorted), or `%op`.
    fn name_of(&self, op: u16) -> String {
        self.values.iter().find(|v| v.op == op).map_or_else(|| format!("{:?}", self.program.ops[usize::from(op)]), |v| v.name.clone())
    }

    fn delay_of(&self, op: u16) -> i32 {
        self.values.iter().find(|v| v.op == op).map_or(0, |v| i32::from(v.delay))
    }

    /// `name`'s window as each leaf's name and its offset in bytes from the value's byte:
    /// `(w, -1)` is `w` at the byte before.
    fn window(&self, facts: &[Fact], name: &str) -> BTreeSet<(String, i32)> {
        let v = self.named(name);
        let local = facts[usize::from(v.op)].local.as_ref().unwrap_or_else(|| panic!("`{name}` is carried"));
        // Leaf bit `q + o` with `q = p + d`: the leaf's byte `p + d + o - d_leaf`.
        local.leaves.iter().map(|&(l, o)| (self.name_of(l), i32::from(v.delay) + o - self.delay_of(l))).collect()
    }

    /// `name`'s anchors, each as the anchor's name and how many bytes before the value's byte
    /// its bit is: `(s, 2)` is "wherever `name` is set, `s` is set two bytes before".
    fn anchored(&self, facts: &[Fact], anchors: &[&str], name: &str) -> BTreeSet<(String, i32)> {
        let v = self.named(name);
        facts[usize::from(v.op)]
            .anchors
            .iter()
            .map(|&(i, s)| {
                // Bit `q` set implies anchor bit `q - s`: byte `p + d - s - d_anchor`.
                let a = self.named(anchors[i]);
                (anchors[i].to_owned(), s + i32::from(a.delay) - i32::from(v.delay))
            })
            .collect()
    }

    fn facts(&self, anchors: &[&str]) -> Vec<Fact> {
        let ops: Vec<u16> = anchors.iter().map(|a| self.named(a).op).collect();
        token_local::facts(&self.program, &ops)
    }

    fn check(&self, anchors: &[&str], facts: &[Fact], files: &[&[u8]]) -> token_local::Checked {
        let ops: Vec<u16> = anchors.iter().map(|a| self.named(a).op).collect();
        token_local::check(&self.program, &ops, facts, files)
    }
}

fn set(items: &[(&str, i32)]) -> BTreeSet<(String, i32)> {
    items.iter().map(|&(n, o)| (n.to_owned(), o)).collect()
}

/// Bytes of the small grammars' alphabet, pseudo-random, with every pair of bytes in them.
fn files() -> Vec<Vec<u8>> {
    let alphabet = b"ab xyz_.\n'";
    let mut out = vec![b"ab a b ba xab".to_vec(), Vec::new(), b"a".to_vec()];
    let mut x: u64 = 0x9e37_79b9_7f4a_7c15;
    for len in [63usize, 64, 65, 200, 1000] {
        out.push(
            (0..len)
                .map(|_| {
                    x ^= x << 13;
                    x ^= x >> 7;
                    x ^= x << 17;
                    alphabet[(x % alphabet.len() as u64) as usize]
                })
                .collect(),
        );
    }
    out
}

const LETTERS: &str = "lexer t\nclass a = 'a'\nclass b = 'b'\nclass w = 'a'..'z'\nclass q = '\\''\n";

#[test]
fn a_local_value_reads_its_leaves_at_fixed_offsets() {
    let g = small(&format!("{LETTERS}mask m = a & next(b) & ~prev(w)\nmask n = prev(m, 3) | next(w, 2)\n"), &["m", "n"]);
    let facts = g.facts(&[]);
    // `~` is cleaned at the file's end by `within`: a leaf of the position alone.
    assert_eq!(g.window(&facts, "m"), set(&[("a", 0), ("b", 1), ("w", -1), ("Within(1)", 1)]));
    // `n` reads `m` three bytes back: `m`'s leaves at -3, and `w` two bytes on.
    assert_eq!(g.window(&facts, "n"), set(&[("a", -3), ("b", -2), ("w", -4), ("w", 2), ("Within(1)", -2), ("Within(2)", 2)]));
    let files = files();
    let refs: Vec<&[u8]> = files.iter().map(Vec::as_slice).collect();
    let c = g.check(&[], &facts, &refs);
    assert!(c.wrong.is_empty(), "{:?}", c.wrong);
    assert!(c.local >= 2);
}

#[test]
fn a_carried_value_is_a_leaf_of_its_readers() {
    let g = small(&format!("{LETTERS}mask f = fill(q, q)\nmask m = f & next(w)\nmask t = toggle(q) & a\n"), &["m", "t"]);
    let facts = g.facts(&[]);
    assert!(facts[usize::from(g.named("f").op)].local.is_none(), "a fill is carried");
    assert_eq!(g.window(&facts, "m"), set(&[("f", 0), ("w", 1), ("Within(1)", 1)]));
    let t = g.window(&facts, "t");
    assert!(t.contains(&("a".to_owned(), 0)) && t.len() == 2, "{t:?}");
    // Read by the fill: `q`'s carried readers.
    let q = &facts[usize::from(g.named("q").op)];
    assert!(q.carried_readers.contains(&g.named("f").op), "{q:?}");
    let files = files();
    let refs: Vec<&[u8]> = files.iter().map(Vec::as_slice).collect();
    let c = g.check(&[], &facts, &refs);
    assert!(c.wrong.is_empty(), "{:?}", c.wrong);
}

#[test]
fn a_value_is_anchored_by_and_shifted_by_prev() {
    let text = format!(
        "{LETTERS}mask s = a & ~prev(w)\nmask x = s & next(b)\nmask y = prev(x, 2) & w\nmask z = x | (s & b)\nmask u = x | b\nmask n = ~x & a\nmask v = b & ~x\n"
    );
    let g = small(&text, &["s", "x", "y", "z", "u", "n", "v"]);
    let anchors = ["s"];
    let facts = g.facts(&anchors);
    assert_eq!(g.anchored(&facts, &anchors, "s"), set(&[("s", 0)]));
    assert_eq!(g.anchored(&facts, &anchors, "x"), set(&[("s", 0)]));
    assert_eq!(g.anchored(&facts, &anchors, "y"), set(&[("s", 2)]));
    assert_eq!(g.anchored(&facts, &anchors, "z"), set(&[("s", 0)]), "both sides anchored");
    for none in ["u", "n", "v"] {
        assert_eq!(g.anchored(&facts, &anchors, none), set(&[]), "`{none}` is not anchored");
    }
    let files = files();
    let refs: Vec<&[u8]> = files.iter().map(Vec::as_slice).collect();
    let c = g.check(&anchors, &facts, &refs);
    assert!(c.wrong.is_empty(), "{:?}", c.wrong);
    assert!(c.anchored >= 4);
}

#[test]
fn a_fixed_points_members_are_carried() {
    let text = format!("{LETTERS}rounds until settled {{\n  carry h = empty\n  mask o = q & ~h\n  next h = prev(fill(o, q))\n}}\nmask m = o & next(a)\n");
    let g = small(&text, &["m"]);
    let facts = g.facts(&[]);
    assert!(facts[usize::from(g.named("o").op)].local.is_none(), "a member is carried");
    assert_eq!(g.window(&facts, "m"), set(&[("o", 0), ("a", 1), ("Within(1)", 1)]));
    let files = files();
    let refs: Vec<&[u8]> = files.iter().map(Vec::as_slice).collect();
    let c = g.check(&[], &facts, &refs);
    assert!(c.wrong.is_empty(), "{:?}", c.wrong);
}

#[test]
fn the_checker_refuses_wrong_claims() {
    let text = format!("{LETTERS}mask s = a & ~prev(w)\nmask x = s & next(b)\nmask u = x | b\n");
    let g = small(&text, &["x", "u"]);
    let anchors = ["s"];
    let files = files();
    let refs: Vec<&[u8]> = files.iter().map(Vec::as_slice).collect();
    let good = g.facts(&anchors);
    assert!(g.check(&anchors, &good, &refs).wrong.is_empty());
    // A leaf dropped from a window: the value reads a leaf the window does not hold.
    let mut bad = good.clone();
    let x = usize::from(g.named("x").op);
    let leaf = *bad[x].local.as_ref().expect("local").leaves.iter().next().expect("a leaf");
    bad[x].local.as_mut().expect("local").leaves.remove(&leaf);
    assert!(!g.check(&anchors, &bad, &refs).wrong.is_empty(), "a missing leaf is caught");
    // A leaf at a wrong offset.
    let mut bad = good.clone();
    let l = bad[x].local.as_mut().expect("local");
    let moved: BTreeSet<(u16, i32)> = l.leaves.iter().map(|&(y, o)| (y, o + 1)).collect();
    l.leaves = moved;
    assert!(!g.check(&anchors, &bad, &refs).wrong.is_empty(), "a moved window is caught");
    // An anchor `u` does not have, and a wrong shift for `x`'s.
    let mut bad = good.clone();
    bad[usize::from(g.named("u").op)].anchors.insert((0, 0));
    assert!(!g.check(&anchors, &bad, &refs).wrong.is_empty(), "a false anchor is caught");
    let mut bad = good;
    let a = bad[x].anchors.iter().next().copied().expect("x is anchored");
    bad[x].anchors = [(a.0, a.1 + 1)].into();
    assert!(!g.check(&anchors, &bad, &refs).wrong.is_empty(), "a wrong shift is caught");
}

#[test]
fn the_per_bit_rule_equals_the_interpreter() {
    let g = small(&format!("{LETTERS}mask f = fill(q, q)\nmask m = (a & next(b) & ~prev(w)) | (prev(f, 2) & ~next(a, 3))\n"), &["m"]);
    let facts = g.facts(&[]);
    let m = g.named("m");
    let local = facts[usize::from(m.op)].local.clone().expect("local");
    for bytes in files() {
        let (values, spans) = g.program.all_values(&[&bytes]);
        let span = spans[0].clone();
        let bit = |y: u16, q: i64| q >= 0 && (q as usize) < 64 * span.len() && values[usize::from(y)][span.start + q as usize / 64] >> (q % 64) & 1 == 1;
        for q in 0..64 * span.len() as i64 {
            let made = token_local::eval(&g.program, &local, m.op, q, &|y, at| bit(y, at)).expect("the window holds every read");
            assert_eq!(made, bit(m.op, q), "bit {q} of {:?}", String::from_utf8_lossy(&bytes));
        }
    }
}

#[test]
fn the_readers_are_the_outputs_reached() {
    let g = small(&format!("{LETTERS}mask k = a & next(b)\nmask m = k & w\nmask f = fill(k, w)\n"), &["m", "f"]);
    let facts = g.facts(&[]);
    let k = &facts[usize::from(g.named("k").op)];
    assert_eq!(k.outputs, [0usize, 1].into(), "`k` reaches both outputs");
    assert_eq!(k.carried_readers, [g.named("f").op].into());
    assert!(facts[usize::from(g.named("m").op)].carried_readers.is_empty());
}

