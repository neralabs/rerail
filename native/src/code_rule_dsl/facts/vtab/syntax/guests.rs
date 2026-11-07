//! Decision 0055's relations of a batch: every document's role, root and tape, and each guest's place.
//!
//! ```text
//! tape_of(d, t)               document d places tape t (its content key, hex)
//! doc_role(d, r)              "primary", "program" or "fragment"
//! doc_root(d, n)              the root of d's tree; a fragment's is the expression inside its wrapper
//! guest(g, host, at, grammar) guest document g is the code at host node or span `at`, parsed as `grammar`
//! parsed(at, grammar, r)      `guest` and `doc_root` joined: the view `core.rr` has always declared
//! sub_tape(at, grammar, t)    `guest` and `tape_of` joined
//! host_node(n, h)             h is the innermost host node over the bytes guest node n stands for
//! ```
//!
//! A withheld guest (one that does not parse, `.on guest_error skip`) has its `guest`, `doc_role`, `tape_of` and
//! `sub_tape` rows, and no `doc_root` or `parsed`: it has no nodes.

use super::{Document, Guest, Placement, Projected, Relation};

#[derive(Default)]
pub(super) struct GuestRows {
    guest: Vec<[i64; 4]>,
    role: Vec<[i64; 2]>,
    root: Vec<[i64; 2]>,
    tape: Vec<[i64; 2]>,
    parsed: Vec<[i64; 3]>,
    sub_tape: Vec<[i64; 3]>,
    host_node: Vec<[i64; 2]>,
}

fn hex(digest: &[u8; 32]) -> String {
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

impl GuestRows {
    /// Document `p`'s rows (`d` is its document, `guest` its place when it is one); `docs` are the batch's documents
    /// projected before it, its host among them.
    pub(super) fn document(&mut self, p: &Projected, d: &Document, guest: Option<&Guest>, withheld: bool, docs: &[Projected], host_nodes: bool, string: &mut dyn FnMut(&str) -> usize) {
        let key = i64::from(p.key);
        let tape = string(&hex(&d.digest)) as i64;
        self.tape.push([key, tape]);
        self.role.push([key, string(guest.map_or("primary", |g| g.role.as_str())) as i64]);
        let root = (!withheld && p.len() > 0).then(|| p.global(0));
        if let Some(r) = root {
            self.root.push([key, r]);
        }
        let Some(g) = guest else { return };
        let grammar = string(&g.grammar) as i64;
        self.guest.push([key, i64::from(g.host), g.at, grammar]);
        self.sub_tape.push([g.at, grammar, tape]);
        let Some(r) = root else { return };
        self.parsed.push([g.at, grammar, r]);
        // A decoded value stands at its node as a whole: its nodes have no host node of their own.
        if !host_nodes || matches!(g.place, Placement::At(_)) {
            return;
        }
        let Ok(h) = docs.binary_search_by_key(&g.host, |q| q.key) else { return };
        let host = &docs[h];
        let at = (g.at & 0xffff_ffff) as u32;
        if at >= host.len() {
            return;
        }
        for local in 0..p.len() {
            let [a, b] = g.place.host(p.span(local));
            let mut over = at;
            while let Some(c) = host.children(over).find(|&c| {
                let [x, y] = host.span(c);
                x <= a && b <= y
            }) {
                over = c;
            }
            self.host_node.push([p.global(local), host.global(over)]);
        }
    }

    pub(super) fn relations(&self, atom: &dyn Fn(i64) -> i64) -> Vec<(&'static str, Relation)> {
        let rel = |columns: &[&str], rows: &mut dyn Iterator<Item = Vec<i64>>| {
            let mut r = Relation::new(columns);
            rows.for_each(|row| r.push(&row));
            r
        };
        vec![
            ("guest", rel(&["g", "host", "at", "grammar"], &mut self.guest.iter().map(|&[g, h, a, s]| vec![g, h, a, atom(s)]))),
            ("doc_role", rel(&["d", "r"], &mut self.role.iter().map(|&[d, r]| vec![d, atom(r)]))),
            ("doc_root", rel(&["d", "n"], &mut self.root.iter().map(|r| r.to_vec()))),
            ("tape_of", rel(&["d", "t"], &mut self.tape.iter().map(|&[d, t]| vec![d, atom(t)]))),
            ("parsed", rel(&["s", "grammar", "root"], &mut self.parsed.iter().map(|&[s, g, r]| vec![s, atom(g), r]))),
            ("sub_tape", rel(&["at", "grammar", "t"], &mut self.sub_tape.iter().map(|&[a, g, t]| vec![a, atom(g), atom(t)]))),
            ("host_node", rel(&["n", "h"], &mut self.host_node.iter().map(|r| r.to_vec()))),
        ]
    }
}
