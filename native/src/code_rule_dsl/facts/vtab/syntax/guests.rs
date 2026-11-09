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
//! parse_of(x, g, t)           `@parse(g, x)` is tape t (decision 0058 §4): a value document places it
//! splice_of(t, n, t2, t3)     `@splice(t, n, t2)` is tape t3
//! unparse_of(t, x)            `@unparse(t)` is x, for a guest's or a value's tape
//! ```
//!
//! A withheld guest (one that does not parse, `.on guest_error skip`) has its `guest`, `doc_role`, `tape_of` and
//! `sub_tape` rows, and no `doc_root` or `parsed`: it has no nodes.

use super::{Document, Guest, Made, Placement, Projected, Relation};

#[derive(Default)]
pub(super) struct GuestRows {
    guest: Vec<[i64; 4]>,
    role: Vec<[i64; 2]>,
    root: Vec<[i64; 2]>,
    tape: Vec<[i64; 2]>,
    parsed: Vec<[i64; 3]>,
    sub_tape: Vec<[i64; 3]>,
    host_node: Vec<[i64; 2]>,
    parse_of: Vec<[i64; 3]>,
    splice_of: Vec<[i64; 4]>,
    unparse_of: Vec<[i64; 2]>,
}

fn hex(digest: &[u8; 32]) -> String {
    super::tape_hex(digest)
}

impl GuestRows {
    /// Document `p`'s rows (`d` is its document, `guest` its place when it is one); `docs` are the batch's documents
    /// projected before it, its host among them.
    /// `wants` says whether `host_node` and `unparse_of` are read.
    pub(super) fn document(&mut self, p: &Projected, d: &Document, guest: Option<&Guest>, withheld: bool, docs: &[Projected], [host_nodes, unparse]: [bool; 2], string: &mut dyn FnMut(&str) -> usize) {
        let key = i64::from(p.key);
        let tape = string(&hex(&d.digest)) as i64;
        self.tape.push([key, tape]);
        self.role.push([key, string(guest.map_or("primary", |g| g.role.as_str())) as i64]);
        let root = (!withheld && p.len() > 0).then(|| p.global(0));
        if let Some(r) = root {
            self.root.push([key, r]);
        }
        let Some(g) = guest else { return };
        if unparse {
            self.unparse_of.push([tape, string(super::unparsed(d)) as i64]);
        }
        let grammar = string(&g.grammar) as i64;
        // A value document has no host: what made it is its row.
        match &g.made {
            Made::Host => {}
            Made::Parse { text } => {
                self.parse_of.push([string(text) as i64, grammar, tape]);
                return;
            }
            Made::Splice { tape: t, node, with } => {
                self.splice_of.push([string(t) as i64, *node, string(with) as i64, tape]);
                return;
            }
        }
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
            ("parse_of", rel(&["x", "g", "t"], &mut self.parse_of.iter().map(|&[x, g, t]| vec![atom(x), atom(g), atom(t)]))),
            ("splice_of", rel(&["t", "n", "t2", "t3"], &mut self.splice_of.iter().map(|&[t, n, t2, t3]| vec![atom(t), n, atom(t2), atom(t3)]))),
            ("unparse_of", rel(&["t", "x"], &mut self.unparse_of.iter().map(|&[t, x]| vec![atom(t), atom(x)]))),
        ]
    }
}
