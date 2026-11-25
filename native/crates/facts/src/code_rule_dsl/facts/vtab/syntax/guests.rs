//! Decision 0055's relations of a batch: every document's tape, and each guest's place.
//!
//! ```text
//! tape_of(d, t)               document d places tape t (its content key, hex)
//! rr_program(d, f, grammar)   d is the document underneath file f's program of `grammar` (decision 0431 §4), for
//!                             the engine's rules alone (findings, memo components): a rule names a program by its
//!                             file and place
//! parsed(at, grammar, r)      the root r of the guest at host node or span `at`, parsed as `grammar`; a fragment's
//!                             is the expression inside its wrapper
//! sub_tape(at, grammar, t)    the guest at `at`'s tape
//! host_node(n, h)             h is the innermost host node over the bytes guest node n stands for
//! parse_of(x, g, t)           `@parse(g, x)` is tape t (decision 0058 §4): a value document places it
//! splice_of(t, n, t2, t3)     `@splice(t, n, t2)` is tape t3
//! unparse_of(t, x)            `@unparse(t)` is x, for a guest's or a value's tape
//! ```
//!
//! A withheld guest (one that does not parse, `.on guest_error skip`) has its `rr_program`, `tape_of` and `sub_tape`
//! rows, and no `parsed`: it has no nodes.

use rerail_blocks::vector::Vector;
use super::{Document, Guest, Made, Placement, Projected, Relation, TapePlace};

#[derive(Default)]
pub struct GuestRows {
    program: Vector<[i64; 3]>,
    grammar: Vec<[i64; 2]>,
    tape: Vector<[i64; 2]>,
    parsed: Vector<[i64; 3]>,
    sub_tape: Vector<[i64; 3]>,
    host_node: Vector<[i64; 2]>,
    guest_of: Vector<[i64; 2]>,
    guest_place: Vector<[i64; 4]>,
    parse_of: Vector<[i64; 3]>,
    splice_of: Vector<[i64; 4]>,
    unparse_of: Vector<[i64; 2]>,
}

fn hex(digest: &[u8; 32]) -> String {
    super::tape_hex(digest)
}

/// The documents a guest's host is found among (decision 0055): a batch's (`made::Docs`, each made when first read,
/// decision 0494), which hold the documents of the batches it grew from (decision 0431 §6).
/// A guest's relations of its place in its host (decision 0055): their first column is the host's node, and a guest
/// makes their rows (`made::ByDocument::Host`).
pub(super) const HOST_ROUTED: &[&str] = &["parsed", "sub_tape"];

/// The guests' relations' other columns of a node of the guest itself (`made::Rows::also`): `parsed`'s root.
pub(super) const OWN_NODES: &[(&str, &[usize])] = &[("parsed", &[2])];

pub(super) trait Hosts {
    /// The projection of the document of key `key`.
    fn host(&self, key: u32) -> Option<&Projected>;
}

impl Hosts for super::made::Docs {
    fn host(&self, key: u32) -> Option<&Projected> {
        self.position(key).map(|i| &self[i])
    }
}

/// A batch's documents, then (a tape value admitted alone, decision 0431 §8) the chunk's it is admitted to.
pub(super) struct HostsOf<'a> {
    pub(super) docs: &'a super::made::Docs,
    pub(super) outer: Option<&'a super::made::Docs>,
}

impl Hosts for HostsOf<'_> {
    fn host(&self, key: u32) -> Option<&Projected> {
        self.docs.host(key).or_else(|| self.outer?.host(key))
    }
}

impl GuestRows {
    /// Document `p`'s rows (`d` is its document, `guest` its place when it is one); `docs` are the batch's documents
    /// projected before it, its host among them.
    /// `wants` says whether `host_node`, `unparse_of`, `guest_of` and `guest_place` are read.
    pub(super) fn document(&mut self, p: &Projected, d: &Document, guest: Option<&Guest>, withheld: bool, docs: &dyn Hosts, [host_nodes, unparse, guest_of, guest_place]: [bool; 4], string: &mut dyn FnMut(&str) -> usize) {
        let key = i64::from(p.key());
        let tape = string(&hex(&d.digest)) as i64;
        self.tape.push([key, tape]);
        // A file's grammar, read off its tape (a program's is its `rr_program` row).
        if guest.is_none()
            && d.facts.tape.is_some()
            && let Some(grammar) = crate::code_rule_dsl::language::from_tape_tag(d.facts.tag).and_then(crate::code_rule_dsl::language::label)
        {
            self.grammar.push([key, string(grammar) as i64]);
        }
        let root = (!withheld && p.len() > 0).then(|| p.global(0));
        let Some(g) = guest else { return };
        if unparse {
            self.unparse_of.push([tape, string(super::unparsed(p.f(), d)) as i64]);
        }
        let grammar = string(&g.grammar) as i64;
        // A value document has no host: what made it is its row.
        match &g.made {
            Made::Host => {}
            // A file's own tree (decision 0512): the file itself, no program of it.
            Made::File => return,
            // A tape value (decision 0431): a row of each relation for each place, and no `rr_program` row: it is
            // no file's program.
            Made::Tape { places } => {
                self.tape_places(p, places, grammar, tape, root, docs, [host_nodes, guest_of, guest_place]);
                return;
            }
            Made::Parse { text } => {
                self.parse_of.push([string(text) as i64, grammar, tape]);
                return;
            }
            Made::Splice { tape: t, node, with } => {
                self.splice_of.push([string(t) as i64, *node, string(with) as i64, tape]);
                return;
            }
        }
        self.program.push([key, i64::from(g.file), grammar]);
        self.sub_tape.push([g.at, grammar, tape]);
        let Some(r) = root else { return };
        self.parsed.push([g.at, grammar, r]);
        if guest_of {
            self.guest_of.extend(p.node_ids().map(|(_, _, n)| [n, g.at]));
        }
        // A program's nodes stand at its host's bytes through its place (a Vue template's piece, decision 0597): a
        // finding at one stands there, its own line and column in the file, as a tape value's does at its string.
        if guest_place {
            self.guest_places(p, docs, g.host, g.at, &g.place);
        }
        // A decoded value stands at its node as a whole: its nodes have no host node of their own.
        if !host_nodes || matches!(g.place, Placement::At(_)) {
            return;
        }
        self.host_nodes(p, docs, g.host, g.at, &g.place);
    }

    /// The rows of tape value `p` (document `d`) at `places`, places it gained since the batch held it (decision 0431
    /// §6: its nodes are projected once; a round adds the rows of its new places).
    pub(super) fn places_added(&mut self, p: &Projected, d: &Document, grammar: &str, places: &[TapePlace], withheld: bool, docs: &dyn Hosts, [host_nodes, guest_of, guest_place]: [bool; 3], string: &mut dyn FnMut(&str) -> usize) {
        let tape = string(&hex(&d.digest)) as i64;
        let grammar = string(grammar) as i64;
        let root = (!withheld && p.len() > 0).then(|| p.global(0));
        self.tape_places(p, places, grammar, tape, root, docs, [host_nodes, guest_of, guest_place]);
    }

    /// A tape value's rows at each of `places` (decision 0431): no `guest` row, it is no document's guest.
    #[allow(clippy::too_many_arguments)]
    fn tape_places(&mut self, p: &Projected, places: &[TapePlace], grammar: i64, tape: i64, root: Option<i64>, docs: &dyn Hosts, [host_nodes, guest_of, guest_place]: [bool; 3]) {
        for at in places {
            self.sub_tape.push([at.at, grammar, tape]);
            if let Some(r) = root {
                self.parsed.push([at.at, grammar, r]);
                if guest_of {
                    self.guest_of.extend(p.node_ids().map(|(_, _, n)| [n, at.at]));
                }
                if guest_place {
                    self.guest_places(p, docs, at.host, at.at, &at.place);
                }
                if host_nodes && !matches!(at.place, Placement::At(_)) {
                    self.host_nodes(p, docs, at.host, at.at, &at.place);
                }
            }
        }
    }

    /// `guest_place`'s rows of tape value `p`'s nodes at string `at` of document `host`, placed by `place` (decision
    /// 0431): each node's line and column in the host's text. A string inside a tape value is placed in that value's
    /// text, which is no file's.
    fn guest_places(&mut self, p: &Projected, docs: &dyn Hosts, host: u32, at: i64, place: &Placement) {
        let Some(lines) = docs.host(host).and_then(|h| h.extra.as_deref()) else { return };
        for ((_, span), (_, _, n)) in p.spans().zip(p.node_ids()) {
            let (line, _, col) = lines.place(place.host(span));
            self.guest_place.push([n, at, i64::from(line), i64::from(col)]);
        }
    }

    /// `host_node`'s rows of document `p`'s nodes, placed at host node `at` of document `host` by `place`.
    fn host_nodes(&mut self, p: &Projected, docs: &dyn Hosts, host: u32, at: i64, place: &Placement) {
        let Some(host) = docs.host(host) else { return };
        // A node id's low bits are its local id only under preorder ids.
        let Some(at) = host.local_of((at & 0xffff_ffff) as u32) else { return };
        if at >= host.len() {
            return;
        }
        for ((_, span), (_, _, n)) in p.spans().zip(p.node_ids()) {
            let [a, b] = place.bounds(span);
            let mut over = at;
            while let Some(c) = host.children(over).find(|&c| {
                let [x, y] = host.span(c);
                x <= a && b <= y
            }) {
                over = c;
            }
            self.host_node.push([n, host.global(over)]);
        }
    }

    pub(super) fn relations(&self, atom: &dyn Fn(i64) -> i64, rel: super::Make) -> Vec<(&'static str, Relation)> {
        vec![
            ("rr_program", rel(&["d", "file", "grammar"], &mut self.program.iter().map(|&[d, f, s]| rerail_facts_base::row![d, f, atom(s)]))),
            ("doc_grammar", rel(&["d", "g"], &mut self.grammar.iter().map(|&[d, g]| rerail_facts_base::row![d, atom(g)]))),
            ("tape_of", rel(&["d", "t"], &mut self.tape.iter().map(|&[d, t]| rerail_facts_base::row![d, atom(t)]))),
            ("parsed", rel(&["s", "grammar", "root"], &mut self.parsed.iter().map(|&[s, g, r]| rerail_facts_base::row![s, atom(g), r]))),
            ("sub_tape", rel(&["at", "grammar", "t"], &mut self.sub_tape.iter().map(|&[a, g, t]| rerail_facts_base::row![a, atom(g), atom(t)]))),
            ("host_node", rel(&["n", "h"], &mut self.host_node.iter().map(|r| r.iter().copied().collect::<rerail_facts_base::code_rule_dsl::facts::vtab::Row>()))),
            ("guest_of", rel(&["n", "s"], &mut self.guest_of.iter().map(|r| r.iter().copied().collect::<rerail_facts_base::code_rule_dsl::facts::vtab::Row>()))),
            ("guest_place", rel(&["n", "s", "l", "c"], &mut self.guest_place.iter().map(|r| r.iter().copied().collect::<rerail_facts_base::code_rule_dsl::facts::vtab::Row>()))),
            ("parse_of", rel(&["x", "g", "t"], &mut self.parse_of.iter().map(|&[x, g, t]| rerail_facts_base::row![atom(x), atom(g), atom(t)]))),
            ("splice_of", rel(&["t", "n", "t2", "t3"], &mut self.splice_of.iter().map(|&[t, n, t2, t3]| rerail_facts_base::row![atom(t), n, atom(t2), atom(t3)]))),
            ("unparse_of", rel(&["t", "x"], &mut self.unparse_of.iter().map(|&[t, x]| rerail_facts_base::row![atom(t), atom(x)]))),
        ]
    }
}
