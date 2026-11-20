//! Guest documents (decision 0055): the code a file holds in another grammar, or in its own grammar as a string,
//! made into documents of the chunk before the policy runs.
//!
//! A chunk's guests come from requests its rules make:
//! - `embed(d, at, grammar, role)` and `embed_value(d, at, grammar, role, text)`: a library's declared embeds
//!   (`.guests off` leaves them out);
//! - `rr_embed_req(at, grammar)`: what a rule's `parsed(at, grammar, r)` or `sub_tape(at, grammar, t)` asks for, from
//!   the literals that bind `at` (`rr::lower`). Its role is "fragment".
//!
//! The rules that make requests (the embed slice) run over the chunk, the guests are parsed and the batch is rebound
//! with them, and the slice runs again over the new documents: a string inside a parsed annotation is asked for in
//! the next round. At most [`DEPTH`] rounds; each round's guests are parts of the last round's.
//!
//! A guest's text:
//! - a string node, for any grammar: its contents. A plain one (no escapes, no `r` prefix) is a copied stretch of the
//!   host, so its nodes have the file's lines and columns; an escaped one, or a concatenation of strings, is its
//!   decoded value, and every node stands at the string. A bytes literal or an f-string gives no guest, as Ruff
//!   parses neither as a type;
//! - a Markdown `inline` or `pipe_table_cell` node: its text without its `block_continuation`s, as tree-sitter-md
//!   gives its inline grammar (decision 0238); each piece is a copied stretch, so its nodes have the file's places;
//! - any other node: its text, copied;
//! - `embed_value`'s text, standing at `at`.
//!
//! A grammar name is a dialect label (`python`, `sql`, `typescript`, …) or `annotation`: Python's expression, parsed
//! as `( … )` with the expression inside the parentheses as its root, as rrlab does. A text a rule parses is a tape value
//! (decision 0431): equal texts in one run are parsed once (`parse`), and its tape is stored with its text, so a later
//! run finds it and parses nothing (R6). A program's tape (`taped`) is not stored yet (step 3).

use crate::code_rule_dsl::facts::core::{self, Built};
use crate::code_rule_dsl::facts::vtab::syntax::{self, Document, Guest, Made, Placement};
use crate::content_store::guest_keys::GuestKeys;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;
use crate::code_rule_dsl::logic::program_image::Version;

/// Rounds of the embed slice per chunk: embeds of embeds, to this depth. A guard, not a limit anyone should reach.
pub(super) const DEPTH: usize = 4;

/// The relations requests are read from.
pub(super) const LIBRARY: [&str; 2] = ["embed", "embed_value"];
/// The guest relations the request `DEMAND` answers in the fixpoint (decision 0431 §8): a value's nodes are reached
/// only through them, so a reader of any is ordered after the strings are asked.
pub(super) const ANSWERED: [&str; 5] = ["parsed", "sub_tape", "guest_of", "guest_place", "host_node"];
/// Of those, the ones the answer gives as rows, not only as the value's part.
const GIVEN: [&str; 2] = ["parsed", "sub_tape"];

/// What `DEMAND` answers in program `v`'s fixpoint (decision 0431 §8), and of that what its answer gives as rows. It
/// produces each guest relation `v` reads (`ANSWERED`), and each relation `DEMAND`'s own rules read that a producer
/// makes off a value's tape, directly or through the relations they derive (`syntax::taped`: a string nested in a value is a string annotation once its outer one is
/// parsed, and is asked in a later wave). It gives as rows `GIVEN`'s and the producers'; the others are read on the
/// values' parts, and a view's (`kind`, `node`) is never given.
pub(super) fn answered(v: &Version) -> (Vec<String>, Vec<String>) {
    let read = |r: &str| v.inputs().any(|a| v.text(a) == r);
    let demand = v.rel(DEMAND);
    let mut produced: Vec<String> = ANSWERED.into_iter().filter(|r| read(r)).map(str::to_owned).collect();
    let mut given: Vec<String> = GIVEN.into_iter().filter(|r| read(r)).map(str::to_owned).collect();
    // The relations the request reads, through the rules that derive them (a context relation's, `rr_ctx`).
    let mut seen: BTreeSet<crate::code_rule_dsl::logic::program_image::RelId> = demand.into_iter().collect();
    let mut todo: Vec<_> = seen.iter().copied().collect();
    while let Some(rel) = todo.pop() {
        for r in v.rules().filter(|r| r.head().rel() == rel) {
            for a in r.body().filter_map(|l| l.atom()) {
                let id = a.rel();
                if !seen.insert(id) {
                    continue;
                }
                let name = v.rel_name(id);
                if read(name) {
                    if syntax::taped(name) && !produced.iter().any(|p| p == name) {
                        produced.push(name.to_owned());
                        given.push(name.to_owned());
                    }
                } else {
                    todo.push(id);
                }
            }
        }
    }
    (produced, given)
}

/// `parsed`'s demand (decision 0055): the strings a rule asks to have parsed.
pub(super) const DEMAND: &str = "rr_embed_req";
/// `@parse(g, x)`'s and `@splice(t, n, t2)`'s demands (decision 0058 §4, `rr::lower`).
pub(super) const PARSE: &str = "rr_parse_req";
pub(super) const SPLICE: &str = "rr_splice_req";
/// The demands rules make.
pub(super) const DEMANDS: [&str; 3] = [DEMAND, PARSE, SPLICE];
/// The relations a rule reads guests and values through (`rr::lower`'s guest relations): a program none of whose
/// needed rules reads one makes no guest.
pub(in crate::code_rule_dsl) const READS: [&str; 13] = ["parsed", "sub_tape", "guest", "doc_root", "doc_grammar", "doc_role", "tape_of", "host_node", "guest_of", "guest_place", "parse_of", "unparse_of", "splice_of"];
/// Every relation a request is read from: a run with a rule deriving one makes guests.
pub(in crate::code_rule_dsl) const REQUESTS: [&str; 5] = ["embed", "embed_value", DEMAND, PARSE, SPLICE];
/// The engine's relations a combined run shares across its programs (`corpus::combine` prefixes no program's): the
/// requests, which the runner reads by name, and the guest policy.
pub(super) const SHARED: [&str; 6] = ["embed", "embed_value", DEMAND, PARSE, SPLICE, crate::code_rule_dsl::facts::vtab::syntax::GUEST_POLICY];

/// What a guest request gave (`Guests::guest`).
pub(super) enum Got {
    /// A document made.
    New(Document),
    /// A place more of tape value `key`, made before.
    Placed(u32, syntax::TapePlace),
    /// None: no text, no grammar, not parsed, or not seen.
    Not,
}

/// One document asked for.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(super) enum Request {
    /// The code at host node `at`, in `grammar`, with its text when it is a value (`embed_value`). `tape`: a string a
    /// rule parses (`parsed`, `sub_tape`, `embed_value`), made a tape value (decision 0431); else a library's
    /// `embed`, a program its file holds. Sorted first, a program is made before a rule's request for the same place.
    Guest { tape: bool, at: i64, grammar: String, role: String, text: Option<String> },
    /// `@parse(grammar, text)`, asked at location `at` (the asking document's).
    Parse { at: i64, grammar: String, text: String },
    /// `@splice(_, node, with)`: node's tape, with node's text replaced by tape `with`'s.
    Splice { node: i64, with: String },
}

/// The location a slice's output line asks at (`requests`): `embed`'s and `embed_value`'s second column, a demand's
/// first. Whose request it is: that location's file's (decision 0225 §2).
pub(super) fn asked_at(rel: &str, rows: &crate::code_rule_dsl::logic::plan::output::Rows, i: usize) -> Option<i64> {
    let col = match rel {
        "embed" | "embed_value" => 1,
        DEMAND | PARSE | SPLICE => 0,
        _ => return None,
    };
    match rows.cell(i, col) {
        crate::code_rule_dsl::logic::plan::output::Cell::Int(at) if col < rows.width() => Some(at),
        _ => None,
    }
}

/// The requests of a slice run's outputs, in order. A library's embeds are made whatever a program shows: `.guests
/// off` hides them from its rules (decision 0058 §1), and `guest` still reaches them.
pub(super) fn requests(outputs: &BTreeMap<String, std::sync::Arc<crate::code_rule_dsl::logic::plan::output::Rows>>) -> Vec<Request> {
    use crate::code_rule_dsl::logic::plan::output::Cell;
    let mut out = Vec::new();
    // Each request relation's rows of `n` values, as the run handed them (typed: nothing read from text).
    let rows = |rel: &str, n: usize| {
        outputs.get(rel).filter(|r| r.width() == n).into_iter().flat_map(move |r| (0..r.len()).map(move |i| (0..n).map(|c| r.cell(i, c)).collect::<Vec<Cell<'_>>>()))
    };
    for c in rows("embed", 4) {
        if let [_, Cell::Int(at), Cell::Text(g), Cell::Text(r)] = c[..] {
            out.push(Request::Guest { tape: false, at, grammar: g.to_owned(), role: r.to_owned(), text: None });
        }
    }
    for c in rows("embed_value", 5) {
        if let [_, Cell::Int(at), Cell::Text(g), Cell::Text(r), Cell::Text(t)] = c[..] {
            // A library's value placed at a node: its file's, as an embed is (decision 0431).
            out.push(Request::Guest { tape: false, at, grammar: g.to_owned(), role: r.to_owned(), text: Some(t.to_owned()) });
        }
    }
    for c in rows(DEMAND, 2) {
        if let [Cell::Int(at), Cell::Text(g)] = c[..] {
            out.push(Request::Guest { tape: true, at, grammar: g.to_owned(), role: "fragment".to_owned(), text: None });
        }
    }
    // `at`, the location that asked, says whose the value is; equal texts are one value (`Guests::value`).
    for c in rows(PARSE, 3) {
        if let [Cell::Int(at), Cell::Text(g), Cell::Text(x)] = c[..] {
            out.push(Request::Parse { at, grammar: g.to_owned(), text: x.to_owned() });
        }
    }
    for c in rows(SPLICE, 2) {
        if let [Cell::Int(node), Cell::Text(t2)] = c[..] {
            out.push(Request::Splice { node, with: t2.to_owned() });
        }
    }
    out.sort();
    out.dedup();
    out
}

/// The embedding a grammar name names for `host` (decision 0222): one its dialect embeds programs through
/// (`GrammarDialect::embeds`), by its registry id (`script-block` for a `.vue` file).
fn embedding(host: &Document, name: &str) -> Option<crate::code_rule_dsl::language::Via> {
    let dialect = crate::code_rule_dsl::language::from_tape_tag(core::facts_tag(&host.facts))?;
    dialect.embeds().iter().map(|&(via, _)| via).find(|via| via.id() == name)
}

/// A grammar name's dialect label, and whether its text is parsed in a wrapper (an expression's `( … )`): a
/// fragment the registry declares (decision 0082), or a dialect's label.
fn grammar(name: &str) -> Option<(&'static str, bool)> {
    if let Some((dialect, start)) = crate::code_rule_dsl::language::fragment(name) {
        return crate::code_rule_dsl::language::label(dialect).map(|label| (label, start == "expression"));
    }
    let dialect = crate::code_rule_dsl::language::dialect_by_label(name)?;
    crate::code_rule_dsl::language::label(dialect).map(|label| (label, false))
}

/// A document's key past every file's (`memo::doc_keys` gives files keys below `GUEST_KEYS`), with `key << 32`
/// positive: a function of what it is (a guest's host path, place and grammar), so a memoised row's guest nodes keep
/// their ids. A code guest in the run's language takes `GUEST_KEYS..ANNOTATION_KEYS`, which a policy shows; an
/// annotation `ANNOTATION_KEYS..HIDDEN_KEYS`; every other document `HIDDEN_KEYS..2^31` (decision 0058 §1).
/// A key's range (its base and span), the place in it its parts hash to, and the guest's identity (decision 0391):
/// a SHA-256 of the range and the parts.
fn first_key(parts: &[&[u8]], range: Range) -> (u32, u32, u32, [u8; 32]) {
    use crate::code_rule_dsl::facts::vtab::syntax::{ANNOTATION_KEYS, GUEST_KEYS, HIDDEN_KEYS};
    let (base, end) = match range {
        Range::Code => (GUEST_KEYS, ANNOTATION_KEYS),
        Range::Annotation => (ANNOTATION_KEYS, HIDDEN_KEYS),
        Range::Hidden => (HIDDEN_KEYS, 1u32 << 31),
    };
    let span = end - base;
    let mut h = Sha256::new();
    for p in parts {
        h.update((p.len() as u64).to_le_bytes());
        h.update(p);
    }
    let hash: [u8; 32] = h.finalize().into();
    let identity: [u8; 32] = Sha256::new().chain_update(base.to_le_bytes()).chain_update(hash).finalize().into();
    (base, span, u32::from_le_bytes(hash[..4].try_into().expect("4 bytes")) % span, identity)
}

/// Which keys a document takes (`key`).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Range {
    Code,
    Annotation,
    Hidden,
}

/// A string's text as a placement: copied when its normalized text is a stretch of the host, mapped when it has
/// escapes; `None` for what is not a constant string.
fn string_text(label: &str, kind: &str, text: &str, start: u32, node: [u32; 2]) -> Option<(String, Placement)> {
    let n = syntax::normalized::normalized(label, kind, text)?;
    let place = match n.copied() {
        Some(delta) => Placement::Copied { delta: i64::from(start) + delta, within: node },
        None => {
            let pieces = n.pieces.iter().map(|p| syntax::normalized::Piece { s: p.s + i64::from(start), ..*p }).collect();
            Placement::Mapped { pieces, within: node }
        }
    };
    Some((n.text, place))
}

/// The text of host node `local` to parse, and where it stands: a string's normalized text (decision 0058 §3; a
/// concatenation's parts joined), any other node's text, copied.
fn host_text(host: &Document, ids: &syntax::Numbering, local: u32) -> Option<(String, Placement)> {
    let f = &*host.facts;
    let (id, kind) = ids.node(local)?;
    let (start, end) = (f.start(id), f.end(id));
    let node = [start, end];
    let tag = core::facts_tag(f);
    let label = crate::code_rule_dsl::language::from_tape_tag(tag).and_then(crate::code_rule_dsl::language::label).unwrap_or("");
    let is_string = |kind: &str| matches!(kind, "string" | "string_literal" | "raw_string_literal" | "interpreted_string_literal");
    if is_string(kind) {
        return string_text(label, kind, f.text(id), start, node);
    }
    if label == "markdown" && matches!(kind, "inline" | "pipe_table_cell") {
        return Some(markdown_inline_text(f, id));
    }
    if kind == "concatenated_string" {
        let mut value = String::new();
        let mut pieces = Vec::new();
        let mut tail = i64::from(end);
        for c in f.children(id) {
            let Some(k) = ids.kind_of(c) else { continue };
            if !is_string(k) {
                continue;
            }
            let (text, place) = string_text(label, k, f.text(c), f.start(c), node)?;
            let at = value.len() as u32;
            match place {
                Placement::Copied { delta, .. } => {
                    pieces.push(syntax::normalized::Piece { d: at, s: delta, escape: false });
                    tail = delta + text.len() as i64;
                }
                Placement::Mapped { pieces: p, .. } => {
                    pieces.extend(p[..p.len() - 1].iter().map(|p| syntax::normalized::Piece { d: p.d + at, ..*p }));
                    tail = p.last().map_or(tail, |p| p.s);
                }
                Placement::At(_) | Placement::Segments(_) => return None,
            }
            value.push_str(&text);
        }
        pieces.push(syntax::normalized::Piece { d: value.len() as u32, s: tail, escape: false });
        return Some((value, Placement::Mapped { pieces: pieces.into(), within: node }));
    }
    Some((f.text(id).to_owned(), Placement::Copied { delta: i64::from(start), within: node }))
}

/// The text of a Markdown `inline` or `pipe_table_cell` node, as tree-sitter-md's `MarkdownParser` gives its inline
/// parser: the node's bytes without its named children after the first (its `block_continuation`s, a container's
/// `> ` or indent on a continued line). The pieces are copied stretches of the host, so its nodes have the file's
/// lines and columns.
pub fn markdown_inline_text(f: &core::Facts, id: u32) -> (String, Placement) {
    let (start, end) = (f.start(id), f.end(id));
    let src = f.source.as_bytes();
    let mut text = Vec::new();
    let mut pieces = Vec::new();
    let mut from = start;
    // tree-sitter-md leaves the first child in (its loop starts at the second), as here.
    for &c in f.children(id).iter().skip(1) {
        if syntax::kind_name(f, c) != "block_continuation" {
            continue;
        }
        let (a, b) = (f.start(c), f.end(c));
        pieces.push(syntax::normalized::Piece { d: text.len() as u32, s: i64::from(from), escape: false });
        text.extend_from_slice(&src[from as usize..a as usize]);
        from = b;
    }
    pieces.push(syntax::normalized::Piece { d: text.len() as u32, s: i64::from(from), escape: false });
    text.extend_from_slice(&src[from as usize..end as usize]);
    pieces.push(syntax::normalized::Piece { d: text.len() as u32, s: i64::from(end), escape: false });
    let text = String::from_utf8(text).expect("cut at node bounds, which are characters'");
    if pieces.len() == 2 {
        return (text, Placement::Copied { delta: i64::from(start), within: [start, end] });
    }
    (text, Placement::Mapped { pieces: pieces.into(), within: [start, end] })
}

/// The guests' makers' state for one chunk: the run's language and shape, the hosts' numberings, the guests made, and
/// the run's guest keys (the store's, decision 0391).
#[derive(Default)]
pub(super) struct Guests {
    run: &'static str,
    /// The run's projection: its node ids are what requests name (`syntax::numbering`).
    shape: syntax::Shape,
    numbering: HashMap<u32, syntax::Numbering>,
    made: HashSet<(i64, String)>,
    asked: HashSet<Request>,
    keys: Arc<GuestKeys>,
    /// Each guest's host file (`memo` attributes a guest's rows to it).
    pub(super) host_of: HashMap<i64, i64>,
    /// Each value made, by grammar and what made it: its key. A value is one per text in a chunk.
    values: HashMap<(String, String), i64>,
    /// Each value's files: those whose rules asked for it (O96). `memo` attributes its rows to each.
    files: HashMap<i64, BTreeSet<i64>>,
    /// Made, and given no guest (no text to parse, or no grammar), this chunk.
    pub(super) refused: usize,
    /// The documents the chunk's program can see (`guests_seen`): a guest past them is not made.
    seen: Seen,
    /// The guests not made because the program cannot see them.
    pub(super) unseen: usize,
}

/// Which guests a program can see (decision 0058 §1, `hiding`): a guest it cannot see adds no row any of its rules
/// reads, so a chunk running it makes none. Ordered by range: each sees the ranges before it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Seen {
    /// Files and code in the run's language: a rule that names no guest.
    Code,
    /// And annotations: a relation whose copies for them were folded into it (`combining::FoldOwn` widens its bound).
    Annotations,
    /// Every guest: a rule enters one through a guest relation (`hiding::ENTRIES`, `READS`), or reads an AST
    /// matcher's literal.
    #[default]
    All,
}

impl Seen {
    fn sees(self, range: Range) -> bool {
        match range {
            Range::Code => true,
            Range::Annotation => self >= Seen::Annotations,
            Range::Hidden => self == Seen::All,
        }
    }
}

/// The guests `v`'s rules can see. A variable reaches a guest's document only from a guest relation (scoped,
/// `hiding`): every other base read is bounded below `ANNOTATION_KEYS`, or below `HIDDEN_KEYS` where an annotation's
/// copies were folded in. So a program that reads no guest relation sees annotations only where a comparison is
/// against the widened bound, and the other guests (other grammars, values) never.
/// The tape tags of the grammars `v`'s requests name (decision 0431 R6: the tags whose tapes a run reads before its
/// first wave); `None` where a request's grammar is not a constant, so any grammar may be asked.
pub(super) fn grammar_tags(v: &crate::code_rule_dsl::logic::program_image::Version) -> Option<std::collections::BTreeSet<i64>> {
    use crate::code_rule_dsl::logic::program_image::TermRef;
    let mut tags = std::collections::BTreeSet::new();
    for r in v.rules() {
        let column = match v.rel_name(r.head().rel()) {
            "embed" | "embed_value" => 2,
            DEMAND | PARSE => 1,
            _ => continue,
        };
        match r.head().args().nth(column) {
            Some(TermRef::Str(a)) => {
                if let Some(tag) = grammar(v.text(a)).and_then(|(label, _)| core::label_tag(label)) {
                    tags.insert(tag);
                }
            }
            _ => return None,
        }
    }
    Some(tags)
}

pub(super) fn guests_seen(v: &crate::code_rule_dsl::logic::program_image::Version) -> Seen {
    use crate::code_rule_dsl::facts::vtab::syntax::{ANNOTATION_KEYS, HIDDEN_KEYS};
    use crate::code_rule_dsl::logic::program_image::{Lit, TermRef};
    let entry = |rel: &str| READS.contains(&rel) || super::hiding::ENTRIES.iter().any(|(r, ..)| *r == rel);
    let widened = |t: TermRef<'_>| matches!(t, TermRef::Int(n) if n > i64::from(ANNOTATION_KEYS) && (n <= i64::from(HIDDEN_KEYS) || n > i64::from(ANNOTATION_KEYS) << 32));
    let mut seen = Seen::Code;
    for r in v.rules() {
        for l in r.body() {
            match l.view() {
                Lit::Atom(a) | Lit::Agg { atom: a, .. } if entry(v.rel_name(a.rel())) => return Seen::All,
                Lit::Ast { .. } => return Seen::All,
                Lit::Cmp { l, r, .. } if widened(l) || widened(r) => seen = Seen::Annotations,
                _ => {}
            }
        }
    }
    seen
}

impl Guests {
    /// For a run over `run`'s files (a dialect label) in `shape`, whose guests' keys are `keys` (read before the
    /// first wave: `ContentStore::guest_keys`, or `GuestKeys::in_memory` without a store).
    pub(super) fn new(run: &'static str, shape: syntax::Shape, keys: Arc<GuestKeys>) -> Self {
        Self { run, shape, keys, ..Self::default() }
    }

    /// Makes only the guests a program that sees `seen` can see (a chunk of partial hits, whose cone is all it runs).
    pub(super) fn seeing(mut self, seen: Seen) -> Self {
        self.seen = seen;
        self
    }

    /// The key of a document hashed from `parts` in `range`, whose host file (a value's first asker) is `path`.
    /// Another document of the run has another key: the run's registry's, which every chunk shares.
    fn key_for(&mut self, parts: &[&[u8]], range: Range, path: &str) -> u32 {
        let (base, span, first, identity) = first_key(parts, range);
        self.keys.key(identity, base, span, first, path)
    }

    /// Each value document's files.
    pub(super) fn askers(&self) -> HashMap<i64, Vec<i64>> {
        self.files.iter().map(|(k, f)| (*k, f.iter().copied().collect())).collect()
    }

    /// The files of document `doc`: a file's own key, a guest's host file, or the files that asked for a value.
    fn files_of(&self, doc: i64) -> BTreeSet<i64> {
        match self.files.get(&doc) {
            Some(files) => files.clone(),
            None => BTreeSet::from([self.host_of.get(&doc).copied().unwrap_or(doc)]),
        }
    }

    /// Value `(grammar, identity)` asked for by document `asker`: true when it is made already (its files grow).
    fn known(&mut self, grammar: &str, identity: &str, asker: i64) -> bool {
        let files = self.files_of(asker);
        match self.values.get(&(grammar.to_owned(), identity.to_owned())) {
            Some(key) => {
                self.files.entry(*key).or_default().extend(files);
                true
            }
            None => false,
        }
    }

    /// The documents `requests` ask for that are not made yet, added to `docs` (which holds their hosts), and the
    /// tape values made before that gained a place (decision 0431): the keys of both, which changed.
    pub(super) fn make(&mut self, requests: Vec<Request>, docs: &mut Vec<Document>, parses: &Parses<'_>) -> Result<Vec<u32>, String> {
        let mut places: Vec<(u32, syntax::TapePlace)> = Vec::new();
        let mut out = {
            let docs: &[Document] = docs;
            self.made_from(requests, docs, parses, &mut places)?
        };
        let mut changed: Vec<u32> = out.iter().map(|d| d.key).collect();
        for (key, place) in places {
            let Some(d) = docs.iter_mut().chain(out.iter_mut()).find(|d| d.key == key) else { continue };
            let Some(g) = d.guest.as_mut() else { continue };
            if let Made::Tape { places } = &mut Arc::make_mut(g).made
                && !places.contains(&place)
            {
                places.push(place);
                changed.push(key);
            }
        }
        docs.append(&mut out);
        changed.sort_unstable();
        changed.dedup();
        Ok(changed)
    }

    /// A string a rule parses, asked inside the chunk's fixpoint (decision 0431 §8: `rr_embed_req(at, grammar)`'s
    /// answer): its tape value made (`Got::New`, added to `docs`), or a place more of one made before (`Got::Placed`,
    /// its document in `docs` given the place); `Got::Not` for a place asked before, or one with no tape.
    pub(super) fn admit(&mut self, docs: &mut Vec<Document>, at: i64, grammar: &str, parses: &Parses<'_>) -> Result<Got, String> {
        if !self.made.insert((at, grammar.to_owned())) {
            return Ok(Got::Not);
        }
        let got = {
            let by_key: HashMap<u32, &Document> = docs.iter().map(|d| (d.key, d)).collect();
            self.guest(&by_key, true, at, grammar, "fragment", None, parses)?
        };
        match &got {
            Got::New(d) => docs.push(d.clone()),
            Got::Placed(key, place) => {
                if let Some(Made::Tape { places }) = docs.iter_mut().find(|d| d.key == *key).and_then(|d| d.guest.as_mut()).map(|g| &mut Arc::make_mut(g).made)
                    && !places.contains(place)
                {
                    places.push(place.clone());
                }
            }
            Got::Not => self.refused += 1,
        }
        Ok(got)
    }

    /// The documents `requests` ask for that are not made yet; a tape value's further places, in `places`.
    fn made_from(&mut self, requests: Vec<Request>, docs: &[Document], parses: &Parses<'_>, places: &mut Vec<(u32, syntax::TapePlace)>) -> Result<Vec<Document>, String> {
        let by_key: HashMap<u32, &Document> = docs.iter().map(|d| (d.key, d)).collect();
        let mut out: Vec<Document> = Vec::new();
        for r in requests {
            // A guest is one per place and grammar, whatever its role.
            let first = match &r {
                Request::Guest { at, grammar, .. } => self.made.insert((*at, grammar.clone())),
                _ => self.asked.insert(r.clone()),
            };
            if !first {
                continue;
            }
            let made = match r {
                Request::Guest { tape, at, grammar, role, text } => match self.guest(&by_key, tape, at, &grammar, &role, text, parses)? {
                    Got::New(d) => Some(d),
                    Got::Placed(key, place) => {
                        places.push((key, place));
                        continue;
                    }
                    Got::Not => None,
                },
                // A value is never shown (`Range::Hidden`): a program that sees no hidden guest makes none.
                Request::Parse { .. } | Request::Splice { .. } if !self.seen.sees(Range::Hidden) => {
                    self.unseen += 1;
                    continue;
                }
                Request::Parse { at, grammar, text } => {
                    let asker = super::memo::document_of(at);
                    if self.known(&grammar, &format!("parse:{text}"), asker) {
                        continue;
                    }
                    self.value(&by_key, asker, &grammar, text.clone(), Made::Parse { text }, parses)?
                }
                Request::Splice { node, with } => {
                    let asker = super::memo::document_of(node);
                    if self.known("", &format!("splice:{node}:{with}"), asker) {
                        continue;
                    }
                    match self.splice(&by_key, docs, node, &with)? {
                        Some((grammar, text, tape, _)) => self.value(&by_key, asker, &grammar, text, Made::Splice { tape, node, with }, parses)?,
                        None => None,
                    }
                }
            };
            match made {
                Some(d) => out.push(d),
                None => self.refused += 1,
            }
        }
        Ok(out)
    }

    /// The guest at host node `at`: a program its host holds (a library's `embed`), or a string a rule parses
    /// (`tape`), made a tape value (decision 0431): one per grammar and text, its places the strings it was parsed
    /// at; a text made already gains a place.
    #[allow(clippy::too_many_arguments)]
    fn guest(&mut self, by_key: &HashMap<u32, &Document>, tape: bool, at: i64, grammar_name: &str, role: &str, text: Option<String>, parses: &Parses<'_>) -> Result<Got, String> {
        let host_key = (at >> 32) as u32;
        let Some(host) = by_key.get(&host_key).copied() else { return Ok(Got::Not) };
        if let Some(via) = embedding(host, grammar_name) {
            return Ok(self.program(host, at, via, role)?.map_or(Got::Not, Got::New));
        }
        let Some((label, wrap)) = grammar(grammar_name) else { return Ok(Got::Not) };
        // Shown to rules that do not name it: code in the run's language. An annotation (a type expression, Python's
        // in a Python run) is not code: the rules that read one reach it through `parsed`. Shown, a string the
        // demand asked for only because its rule's inputs hold it (every string, for `type_ctx`) was code to every
        // rule: `"--host"` was `--host`, a doubled minus (B002 on 157 argparse lines).
        // A tape value (decision 0431) is never code to such rules: its nodes are no file's, so a finding on one has
        // no place. In the run's language it is in the annotations' range.
        let range = match (label == self.run, grammar_name == "annotation" || tape) {
            (true, false) => Range::Code,
            (true, true) => Range::Annotation,
            (false, _) => Range::Hidden,
        };
        if !self.seen.sees(range) {
            self.unseen += 1;
            return Ok(Got::Not);
        }
        let ids = self.numbering(host)?;
        // A node id's low bits are its local id only under preorder ids (`syntax::Numbering`).
        let Some(local) = ids.local((at & 0xffff_ffff) as u32) else { return Ok(Got::Not) };
        let placed = match text {
            Some(text) => {
                let Some((id, _)) = ids.node(local) else { return Ok(Got::Not) };
                Some((text, Placement::At([host.facts.start(id), host.facts.end(id)])))
            }
            None => host_text(host, ids, local),
        };
        let Some((text, place)) = placed else { return Ok(Got::Not) };
        let (text, place) = if wrap { wrapped(text, place) } else { (text, place) };
        if tape {
            return self.tape_value(host, host_key, at, grammar_name, role, label, wrap, text, place, range, parses);
        }
        let Some((facts, root, digest)) = parse(&host.path, label, wrap, text, parses)? else { return Ok(Got::Not) };
        let key = self.key_for(&[host.path.as_bytes(), &at.to_le_bytes(), grammar_name.as_bytes()], range, &host.path);
        let file = self.host_of.get(&i64::from(host_key)).copied().unwrap_or(i64::from(host_key));
        self.host_of.insert(i64::from(key), file);
        let file = file as u32;
        let in_file = host.guest.as_ref().map_or_else(|| place.clone(), |h| h.in_file.then(place.clone()));
        Ok(Got::New(Document {
            key,
            path: host.path.clone(),
            core: format!("{}#guest:{grammar_name}:{at}", host.core),
            digest,
            facts,
            guest: Some(Arc::new(Guest { host: host_key, file, at, grammar: grammar_name.to_owned(), role: role.to_owned(), root, place, in_file, made: Made::Host })),
            module: host.module.clone(),
            tape: None,
        }))
    }

    /// String `at` of `host`, its `text` parsed as `label`: a tape value (decision 0431), keyed by its grammar and
    /// text, not its place. A text made already in this chunk gains the place; else it is parsed (once a run,
    /// `parse`) and made. Its rows are the files' that hold it (`files`, as a value's).
    #[allow(clippy::too_many_arguments)]
    fn tape_value(&mut self, host: &Document, host_key: u32, at: i64, grammar_name: &str, role: &str, label: &str, wrap: bool, text: String, place: Placement, range: Range, parses: &Parses<'_>) -> Result<Got, String> {
        let identity = format!("tape:{wrap}:{text}");
        let files = self.files_of(i64::from(host_key));
        let here = syntax::TapePlace { host: host_key, at, place };
        if let Some(key) = self.values.get(&(grammar_name.to_owned(), identity.clone())).copied() {
            self.files.entry(key).or_default().extend(files);
            return Ok(Got::Placed(key as u32, here));
        }
        let Some((facts, root, digest)) = parse(&host.path, label, wrap, text.clone(), parses)? else { return Ok(Got::Not) };
        let key = self.key_for(&[b"tape", grammar_name.as_bytes(), identity.as_bytes()], range, &host.path);
        self.values.insert((grammar_name.to_owned(), identity), i64::from(key));
        self.files.insert(i64::from(key), files);
        let none = Placement::At([0, 0]);
        Ok(Got::New(Document {
            key,
            path: host.path.clone(),
            core: format!("#tape:{grammar_name}:{key}"),
            digest,
            facts,
            guest: Some(Arc::new(Guest { host: 0, file: 0, at: 0, grammar: grammar_name.to_owned(), role: role.to_owned(), root, place: none.clone(), in_file: none, made: Made::Tape { places: vec![here] } })),
            // A tape value is no module's: its text is a string's, not a file's.
            module: None,
            tape: None,
        }))
    }

    /// The program host file `host` embeds through `via` (decision 0222): the embedder main registered for it
    /// (`embed::Embedder`; a Vue component's view, `vue::view`) writes its text and its segments from the host's
    /// tape, read in place, and the text is parsed in the dialect the embedder chose (a `<script lang="ts">`'s TypeScript). The
    /// guest's grammar is that dialect's label, as `guest` names what a document is parsed as; its nodes are placed
    /// through the segments (`Placement::Segments`). `None` where the host embeds no program (no script, `lang="coffee"`).
    fn program(&mut self, host: &Document, at: i64, via: crate::code_rule_dsl::language::Via, role: &str) -> Result<Option<Document>, String> {
        let host_key = (at >> 32) as u32;
        let Some(dialect) = crate::code_rule_dsl::language::from_tape_tag(core::facts_tag(&host.facts)) else { return Ok(None) };
        let source = host.facts.source.as_str();
        // The host's tape, read in place by the embedder (whichever parser wrote it, the same tape).
        let Ok(grammar) = crate::code_rule_dsl::tape::Grammar::new(dialect) else { return Ok(None) };
        let Some(Ok(image)) = crate::code_rule_dsl::tape::TapeImage::parse(&grammar, source, &|| false) else { return Ok(None) };
        let Some(root) = crate::code_rule_dsl::embed::host::Host::new(&image.primary, &grammar).root() else { return Ok(None) };
        let Some(program) = crate::code_rule_dsl::embed::embedder(via).build(root, source, &|| false) else { return Ok(None) };
        let Some(label) = crate::code_rule_dsl::language::label(program.dialect) else { return Ok(None) };
        let range = if label == self.run { Range::Code } else { Range::Hidden };
        if !self.seen.sees(range) {
            self.unseen += 1;
            return Ok(None);
        }
        let place = Placement::Segments(Arc::new(program.segments));
        let Some(((facts, root, digest), tape)) = taped(&host.path, label, program.text)? else { return Ok(None) };
        let key = self.key_for(&[host.path.as_bytes(), &at.to_le_bytes(), via.id().as_bytes()], range, &host.path);
        let file = self.host_of.get(&i64::from(host_key)).copied().unwrap_or(i64::from(host_key));
        self.host_of.insert(i64::from(key), file);
        let file = file as u32;
        let in_file = host.guest.as_ref().map_or_else(|| place.clone(), |h| h.in_file.then(place.clone()));
        Ok(Some(Document {
            key,
            path: host.path.clone(),
            core: format!("{}#guest:{}:{at}", host.core, via.id()),
            digest,
            facts,
            guest: Some(Arc::new(Guest { host: host_key, file, at, grammar: label.to_owned(), role: role.to_owned(), root, place, in_file, made: Made::Host })),
            module: host.module.clone(),
            tape,
        }))
    }

    /// A value document (decision 0058 §4): `text` parsed as `grammar_name`, with no host, in the hidden documents,
    /// asked for by document `asker`. Its places are its own text's. It is its askers' files' (O96): `memo` keeps its
    /// rows with each, and its path is the first's.
    fn value(&mut self, by_key: &HashMap<u32, &Document>, asker: i64, grammar_name: &str, text: String, made: Made, parses: &Parses<'_>) -> Result<Option<Document>, String> {
        let Some((label, wrap)) = grammar(grammar_name) else { return Ok(None) };
        let (identity, known_as) = match &made {
            Made::Splice { node, with, .. } => (format!("splice:{node}:{with}"), String::new()),
            _ => (format!("parse:{text}"), grammar_name.to_owned()),
        };
        let files = self.files_of(asker);
        let path = files.first().and_then(|f| u32::try_from(*f).ok()).and_then(|f| by_key.get(&f)).map_or_else(String::new, |d| d.path.clone());
        let path = path.as_str();
        let (text, _) = if wrap { wrapped(text, Placement::At([0, 0])) } else { (text, Placement::At([0, 0])) };
        let Some((facts, root, digest)) = parse(path, label, wrap, text, parses)? else { return Ok(None) };
        let key = self.key_for(&[b"value", grammar_name.as_bytes(), identity.as_bytes()], Range::Hidden, path);
        self.values.insert((known_as, identity), i64::from(key));
        self.files.insert(i64::from(key), files);
        Ok(Some(Document {
            key,
            path: path.to_owned(),
            core: format!("#value:{grammar_name}:{key}"),
            digest,
            facts,
            guest: Some(Arc::new(Guest { host: 0, file: 0, at: 0, grammar: grammar_name.to_owned(), role: "value".to_owned(), root, place: Placement::At([0, 0]), in_file: Placement::At([0, 0]), made })),
            // A value is no module's: its text is a rule's, not a file's.
            module: None,
            tape: None,
        }))
    }

    /// `@splice(_, node, with)`: the grammar, text, tape and path of node's document with node's text replaced by
    /// the text of a document whose tape is `with`; `None` when either is not in the chunk.
    fn splice(&mut self, by_key: &HashMap<u32, &Document>, docs: &[Document], node: i64, with: &str) -> Result<Option<(String, String, String, String)>, String> {
        let Some(d) = by_key.get(&((node >> 32) as u32)).copied() else { return Ok(None) };
        let Some(w) = docs.iter().find(|x| syntax::tape_hex(&x.digest) == with) else { return Ok(None) };
        let ids = self.numbering(d)?;
        let Some((_, id, _)) = ids.at((node & 0xffff_ffff) as u32) else { return Ok(None) };
        let base = syntax::unparsed(d);
        // The unparsed text's offset in the document's text (a wrapped fragment's root starts past the `(`).
        let off = match d.guest.as_deref() {
            Some(g) if g.root != 0 => d.facts.start(g.root) as usize,
            _ => 0,
        };
        let (a, b) = (d.facts.start(id) as usize, d.facts.end(id) as usize);
        let (Some(a), Some(b)) = (a.checked_sub(off), b.checked_sub(off)) else { return Ok(None) };
        if b > base.len() || !base.is_char_boundary(a) || !base.is_char_boundary(b) {
            return Ok(None);
        }
        let text = format!("{}{}{}", &base[..a], syntax::unparsed(w), &base[b..]);
        let grammar = match d.guest.as_deref() {
            Some(g) => g.grammar.clone(),
            None => {
                let tag = core::facts_tag(&d.facts);
                let Some(label) = crate::code_rule_dsl::language::from_tape_tag(tag).and_then(crate::code_rule_dsl::language::label) else { return Ok(None) };
                label.to_owned()
            }
        };
        Ok(Some((grammar, text, syntax::tape_hex(&d.digest), d.path.clone())))
    }

    fn numbering(&mut self, host: &Document) -> Result<&syntax::Numbering, String> {
        if !self.numbering.contains_key(&host.key) {
            let ids = syntax::numbering(host, self.shape).map_err(|e| e.to_string())?;
            self.numbering.insert(host.key, ids);
        }
        Ok(&self.numbering[&host.key])
    }
}

/// A text in its wrapper `( … )`, and its placement one byte on.
fn wrapped(text: String, place: Placement) -> (String, Placement) {
    let place = match place {
        Placement::Copied { delta, within } => Placement::Copied { delta: delta - 1, within },
        Placement::Mapped { pieces, within } => Placement::Mapped { pieces: pieces.iter().map(|p| syntax::normalized::Piece { d: p.d + 1, ..*p }).collect(), within },
        at => at,
    };
    (format!("({text})"), place)
}

/// A guest's or value's tree: its facts, its root (a wrapped expression's), and the facts' digest.
type Parsed = (Arc<core::Facts>, u32, [u8; 32]);

/// The guests' trees by grammar and text, for one run: a pack's guests repeat their texts (one annotation in many files,
/// the same guests in every chunk and round), and the facts are a function of the two alone, so each distinct text is
/// parsed and digested once a run. Held by the run, not the process (no static memo). Decision 0431 R6 (it replaces Q30,
/// which judged a parse cheaper than a store's write and read, about 0.01 s a run on ep2): a text's tape is found among
/// the tapes the last runs named, read before the first wave (decision 0104), or queued for the store behind the run,
/// so a text a run parsed is not parsed again.
#[derive(Default)]
pub(super) struct Parses<'a> {
    kept: std::sync::Mutex<(usize, HashMap<(String, bool, String), Option<Parsed>>)>,
    /// The texts parsed, not found kept (`Report::guest_parses`): decision 0431 R6, a text a run parsed is not
    /// parsed again.
    parsed: std::sync::atomic::AtomicUsize,
    /// Where a text's tape is found or kept (decision 0431 R6); `None` in a run with no store, which parses each text
    /// once a run.
    store: Option<ValueStore<'a>>,
}

/// A run's tapes of the texts its rules parse (decision 0431 R6): found by the text's SHA-256 and its grammar's tag
/// among the tapes the root's last runs named (read before the first wave, decision 0104: the run's grammar's in
/// `by_text`, the files' map, the others in `others`), else queued for the store behind the run and named in its
/// manifest, so the next run finds them.
pub(super) struct ValueStore<'a> {
    pub(super) cache: &'a crate::content_store::ContentStore,
    pub(super) run_tag: Option<i64>,
    pub(super) by_text: &'a HashMap<crate::content_store::HashKey, (crate::content_store::SourceId, crate::content_store::TapeFrame)>,
    pub(super) others: HashMap<(crate::content_store::HashKey, i64), (crate::content_store::SourceId, crate::content_store::TapeFrame)>,
    pub(super) late: &'a super::Late,
}

impl<'a> Parses<'a> {
    /// The parses of a run whose tapes are kept in `store`.
    pub(super) fn new(store: Option<ValueStore<'a>>) -> Self {
        Self { store, ..Self::default() }
    }

    /// The texts this run parsed.
    pub(super) fn parsed(&self) -> usize {
        self.parsed.load(std::sync::atomic::Ordering::Relaxed)
    }
}

/// `text` parsed as `label`, its root (a wrapped expression's) and its facts' digest; `None` when the parser gives no
/// facts. Kept for the run by grammar and text (`Parses`).
fn parse(path: &str, label: &str, wrap: bool, text: String, parses: &Parses<'_>) -> Result<Option<Parsed>, String> {
    /// The texts kept, in bytes, before the memo starts again (the facts kept are a small multiple of it).
    const KEPT_BYTES: usize = 8 << 20;
    let key = (label.to_owned(), wrap, text);
    if let Some(hit) = parses.kept.lock().expect("the parsed guests").1.get(&key) {
        crate::code_rule_dsl::prof::count("guests.parse_kept", 1);
        return Ok(hit.clone());
    }
    let file = crate::code_rule_dsl::facts::core::Source { language: label.to_owned(), source: Arc::new(key.2.clone()) };
    let built = match (&parses.store, core::label_tag(label)) {
        // Decision 0431 R6: found among the last runs' tapes by its text, or queued earlier in this run; else parsed,
        // and queued for the store with its source, which the manifest names.
        (Some(s), Some(tag)) if cfg!(feature = "tape-cache") => {
            use sha2::Digest as _;
            let digest: [u8; 32] = sha2::Sha256::digest(key.2.as_bytes()).into();
            let hash = crate::content_store::HashKey::from_sha256_digest(digest);
            let found = if s.run_tag == Some(tag) { s.by_text.get(&hash) } else { s.others.get(&(hash, tag)) };
            match found {
                Some((source, _)) => s.late.lock().expect("the late sources").push(*source),
                None if core::tape_queued(s.cache, &file, &digest) => {}
                None => {
                    parses.parsed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
            }
            core::build_in_wave_with_tape(&file, s.cache, Some(digest), found.map(|(_, f)| f), true, false).map(|(b, _)| b)
        }
        _ => {
            parses.parsed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            core::build(&file)
        }
    };
    let built = built.map_err(|e| format!("{path}: {e}"))?;
    let parsed = match built {
        Built::Facts(facts) => {
            let root = if wrap { wrapped_root(&facts, label) } else { 0 };
            let digest = core::digest(&facts);
            Some((Arc::new(facts), root, digest))
        }
        _ => None,
    };
    let mut kept = parses.kept.lock().expect("the parsed guests");
    if kept.0 + key.2.len() > KEPT_BYTES {
        *kept = Default::default();
    }
    kept.0 += key.2.len();
    kept.1.insert(key, parsed.clone());
    Ok(parsed)
}

/// An embedded program's text parsed as `label`, with its tape kept beside the facts (decision 0222): the rules over
/// a view read W's values (`value`, a key's text) as over a file, and a value is read off the document's tape. Not
/// memoised: a host embeds one program. Its tape is not stored, as a guest's (`parse`, Q30).
fn taped(path: &str, label: &str, text: String) -> Result<Option<(Parsed, Tape)>, String> {
    let file = crate::code_rule_dsl::facts::core::Source { language: label.to_owned(), source: Arc::new(text) };
    let built = core::build_with_tape(&file, true).map_err(|e| format!("{path}: {e}"))?;
    let (Built::Facts(facts), tape) = built else { return Ok(None) };
    let digest = core::digest(&facts);
    let tape = tape.map(|doc| (doc, Arc::new(file) as Arc<dyn crate::code_rule_dsl::facts::core::SourceFile>));
    Ok(Some(((Arc::new(facts), 0, digest), tape)))
}

/// A document's tape in its file's context (`Document::tape`).
type Tape = Option<(crate::code_rule_dsl::tape::TapeDoc, Arc<dyn crate::code_rule_dsl::facts::core::SourceFile>)>;

/// The root of a wrapped expression `( e )`: `e`, the one named child of the parenthesized expression the module's
/// one statement is, when the text parses so; else the module's first named child (a partial tree's).
fn wrapped_root(f: &core::Facts, label: &str) -> u32 {
    let Some(dialect) = crate::code_rule_dsl::language::dialect_by_label(label) else { return 0 };
    let kinds = dialect.kinds();
    let name = |id: u32| kinds.kind_name(core::kind_code(f.kind(id))).unwrap_or("");
    let named = |id: u32| -> Vec<u32> { f.children(id).iter().copied().filter(|&c| f.named(c) && !name(c).contains("comment")).collect() };
    let first = named(0).first().copied().unwrap_or(0);
    let path = (|| {
        let [statement] = named(0)[..] else { return None };
        let [paren] = named(statement)[..] else { return None };
        if name(paren) != "parenthesized_expression" {
            return None;
        }
        let [e] = named(paren)[..] else { return None };
        Some(e)
    })();
    path.unwrap_or(first)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two hosts' annotations whose identities hash to one key, made in two chunks of a run: two keys, and the first
    /// identity's key again in a third chunk (decision 0391). Before, each chunk probed only its own keys.
    #[test]
    fn two_chunks_guests_that_hash_alike_take_two_keys() {
        let parts = |p: &str| [p.as_bytes().to_vec(), 0i64.to_le_bytes().to_vec(), b"annotation".to_vec()];
        // A birthday search over host paths: two whose annotation at 0 hashes to one key.
        let mut seen: HashMap<u32, String> = HashMap::new();
        let (a, b) = (0..)
            .map(|i| format!("f{i}.py"))
            .find_map(|p| {
                let [x, y, z] = parts(&p);
                let k = first_key(&[&x, &y, &z], Range::Annotation).2;
                seen.insert(k, p.clone()).map(|q| (q, p))
            })
            .expect("a pair");
        let key = |g: &mut Guests, p: &str| {
            let [x, y, z] = parts(p);
            g.key_for(&[&x, &y, &z], Range::Annotation, p)
        };
        let keys = GuestKeys::in_memory();
        let chunk = || Guests::new("python", syntax::Shape::default(), Arc::clone(&keys));
        let (mut one, mut two, mut three) = (chunk(), chunk(), chunk());
        let ka = key(&mut one, &a);
        let kb = key(&mut two, &b);
        assert_ne!(ka, kb, "{a} and {b} hash alike: one key for two documents");
        assert_eq!(key(&mut three, &a), ka, "an identity keeps its key");
    }
}
