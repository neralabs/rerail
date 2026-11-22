//! Guests (decisions 0055, 0431): the code a file holds in another grammar, or in its own grammar as a string.
//!
//! - A file's programs (an HTML script, a Vue view, a Markdown inline): its language's declaration (`embeds`), made
//!   from its tape with the chunk's batch (`programs`, decision 0431 §4), each a document underneath whose nodes are
//!   the file's.
//! - A string a rule parses (`rr_embed_req(at, grammar)`: what a rule's `parsed(at, grammar, r)` or
//!   `sub_tape(at, grammar, t)` asks for, from the literals that bind `at`, `rr::lower`): a tape value, admitted in
//!   the program's fixpoint (`admit`, decision 0431 §8).
//! - A tape operation's value (`@parse(g, x)`, `@splice`): made by the slice's rounds (`make`), at most [`DEPTH`].
//!
//! A string's text:
//! - a string node, for any grammar: its contents. A plain one (no escapes, no `r` prefix) is a copied stretch of the
//!   host, so its nodes have the file's lines and columns; an escaped one, or a concatenation of strings, is its
//!   decoded value, and every node stands at the string. A bytes literal or an f-string gives no guest, as Ruff
//!   parses neither as a type;
//! - any other node: its text, copied.
//!
//! A grammar name is a dialect label (`python`, `sql`, `typescript`, …) or `annotation`: Python's expression, parsed
//! as `( … )` with the expression inside the parentheses as its root, as rrlab does. A text a rule parses is a tape value
//! (decision 0431): equal texts in one run are parsed once (`parse`), and its tape is stored with its text, so a later
//! run finds it and parses nothing (R6). A program's tape (`taped`, read off its embedder's parse) is not stored yet.

use rerail_blocks::vector::Vector;
use crate::code_rule_dsl::facts::core::{self, Built};
use crate::code_rule_dsl::facts::vtab::syntax::{self, Document, Guest, Made, Placement};
use crate::content_store::guest_keys::GuestKeys;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;
use crate::code_rule_dsl::logic::program_image::Version;

/// The guest relations the request `DEMAND` answers in the fixpoint (decision 0431 §8): a value's nodes are reached
/// only through them, so a reader of any is ordered after the strings are asked.
pub(super) const ANSWERED: [&str; 5] = ["parsed", "sub_tape", "guest_of", "guest_place", "host_node"];
/// Of those, the ones the answer gives as rows, not only as the value's part.
const GIVEN: [&str; 2] = ["parsed", "sub_tape"];

/// What request `req` (`DEMANDS`) answers, and of that what its answer gives as rows: a string's guest relations
/// (`DEMAND`); `@parse`'s and `@splice`'s value, reached through `parse_of` and `splice_of` alone (decision 0058 §4:
/// its `tape_of`, `unparse_of` and nodes are the value's part, bound when it is admitted, and every reader reaches
/// them through the value's tape).
fn answered_by(req: &str) -> (&'static [&'static str], &'static [&'static str]) {
    match req {
        DEMAND => (&ANSWERED, &GIVEN),
        PARSE => (&["parse_of"], &["parse_of"]),
        SPLICE => (&["splice_of"], &["splice_of"]),
        _ => (&[], &[]),
    }
}

/// The requests program `v` derives that its fixpoint answers (decision 0431 §8, Q325): each of `DEMANDS` a rule
/// derives, with what it answers (`answered`), where that is read. A relation is one request's answer (`answers::gate`):
/// a producer's that two requests' rules read is the first's.
pub fn answering(v: &Version) -> BTreeMap<String, (Vec<String>, Vec<String>)> {
    let mut claimed: BTreeSet<String> = BTreeSet::new();
    let mut out = BTreeMap::new();
    let read = |r: &str| v.inputs().any(|a| v.text(a) == r);
    for r in DEMANDS.into_iter().filter(|r| v.rel(r).is_some_and(|id| v.heads(id).next().is_some()) && answered_by(r).0.iter().any(|a| read(a))) {
        let (mut produced, mut given) = answered(v, r);
        produced.retain(|p| claimed.insert(p.clone()));
        given.retain(|g| produced.contains(g));
        if !produced.is_empty() {
            out.insert(r.to_owned(), (produced, given));
        }
    }
    out
}

/// What request `req` answers in program `v`'s fixpoint (decision 0431 §8), and of that what its answer gives as rows.
/// It produces each relation of `answered_by` that `v` reads, and each relation `req`'s own rules read that a producer
/// makes off a value's tape, directly or through the relations they derive (`syntax::taped`: a string nested in a value is a string annotation once its outer one is
/// parsed, and is asked in a later wave). It gives as rows `answered_by`'s and the producers'; the others are read on the
/// values' parts, and a view's (`kind`, `node`) is never given.
pub(super) fn answered(v: &Version, req: &str) -> (Vec<String>, Vec<String>) {
    let read = |r: &str| v.inputs().any(|a| v.text(a) == r);
    let demand = v.rel(req);
    let (answers, gives) = answered_by(req);
    let mut produced: Vec<String> = answers.iter().filter(|r| read(r)).map(|r| (*r).to_owned()).collect();
    let mut given: Vec<String> = gives.iter().filter(|r| read(r)).map(|r| (*r).to_owned()).collect();
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

/// `embed`'s demand: a string made a tape value and parsed (decision 0431 §8), answered in the fixpoint.
pub const DEMAND: &str = DEMANDS[0];
/// `@parse(g, x)`'s and `@splice(t, n, t2)`'s demands (decision 0058 §4, `rr::lower`).
pub const PARSE: &str = DEMANDS[1];
pub const SPLICE: &str = DEMANDS[2];
/// The demands rules make (`rr::lower`'s, in its order).
pub const DEMANDS: [&str; 3] = crate::code_rule_dsl::logic::rr::lower::DEMANDS;
/// The relations a rule reads guests and values through (`rr::lower`'s guest relations): a program none of whose
/// needed rules reads one makes no guest.
pub const READS: [&str; 11] = ["parsed", "sub_tape", "rr_program", "doc_grammar", "tape_of", "host_node", "guest_of", "guest_place", "parse_of", "unparse_of", "splice_of"];
/// Every relation a request is read from: a run with a rule deriving one makes guests.
pub const REQUESTS: [&str; 3] = DEMANDS;
/// The engine's relations a combined run shares across its programs (`corpus::combine` prefixes no program's): the
/// requests, which the runner reads by name, and the guest policy.
pub const SHARED: [&str; 4] = [DEMAND, PARSE, SPLICE, crate::code_rule_dsl::facts::vtab::syntax::GUEST_POLICY];

/// What a guest request gave (`Guests::guest`).
pub enum Got {
    /// A document made.
    New(Box<Document>),
    /// A place more of tape value `key`, made before.
    Placed(u32, syntax::TapePlace),
    /// None: no text, no grammar, not parsed, or not seen.
    Not,
}

/// A value a rule makes (decision 0058 §4), asked inside the fixpoint (`Guests::admit_value`). A string a rule parses
/// is asked as its place and grammar (`Guests::admit`).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Request {
    /// `@parse(grammar, text)`, asked at location `at` (the asking document's).
    Parse { at: i64, grammar: String, text: String },
    /// `@splice(_, node, with)`: node's tape, with node's text replaced by tape `with`'s.
    Splice { node: i64, with: String },
}

/// The embedding a grammar name names for `host` (decision 0222): one its dialect embeds programs through
/// (`GrammarDialect::embeds`), by its registry id (`script-block` for a `.vue` file).
fn embedding(host: &Document, name: &str) -> Option<crate::code_rule_dsl::language::Via> {
    let dialect = crate::code_rule_dsl::language::from_tape_tag(core::facts_tag(&host.facts))?;
    dialect.embeds().iter().map(|&(via, _)| via).find(|via| via.id() == name)
}

/// A grammar name's dialect label, and the fragment it parses (decision 0466: from a node kind, its tapes under its
/// own tag): a fragment the registry declares (decision 0082), or a dialect's label, a whole text.
fn grammar(name: &str) -> Option<(&'static str, Option<core::Fragment>)> {
    if let Some((dialect, _)) = crate::code_rule_dsl::language::fragment(name) {
        return crate::code_rule_dsl::language::label(dialect).zip(Some(core::Fragment::named(name)));
    }
    let dialect = crate::code_rule_dsl::language::dialect_by_label(name)?;
    crate::code_rule_dsl::language::label(dialect).map(|label| (label, None))
}

/// The tag a grammar's tapes are stored under: a fragment's own, else its dialect's.
fn tape_tag_of(label: &str, fragment: Option<core::Fragment>) -> Option<i64> {
    fragment.map(|f| i64::from(f.tag)).or_else(|| core::label_tag(label))
}

/// A document's key past every file's (`memo::doc_keys` gives files keys below `GUEST_KEYS`), with `key << 32`
/// positive: a function of what it is (a guest's host path, place and grammar), so a memoised row's guest nodes keep
/// their ids. A code guest in the run's language takes `GUEST_KEYS..RUN_VALUE_KEYS`, which a policy shows; a
/// tape value in it `RUN_VALUE_KEYS..HIDDEN_KEYS`; every other document `HIDDEN_KEYS..2^31` (decisions 0058 §1, 0512).
/// A key's range (its base and span), the place in it its parts hash to, and the guest's identity (decision 0391):
/// a SHA-256 of the range and the parts.
fn first_key(parts: &[&[u8]], range: Range) -> (u32, u32, u32, [u8; 32]) {
    use crate::code_rule_dsl::facts::vtab::syntax::{RUN_VALUE_KEYS, GUEST_KEYS, HIDDEN_KEYS};
    let (base, end) = match range {
        Range::Code => (GUEST_KEYS, RUN_VALUE_KEYS),
        Range::RunValue => (RUN_VALUE_KEYS, HIDDEN_KEYS),
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

/// The key of the tree of a file of another grammar that a run reads for its programs (decisions 0470 step 2, 0512):
/// a hidden one, by its path, claimed as a guest's is, so no guest takes it.
pub(super) fn hidden_file_key(keys: &GuestKeys, path: &str) -> u32 {
    let (base, span, first, identity) = first_key(&[b"host-file", path.as_bytes()], Range::Hidden);
    keys.key(identity, base, span, first, path)
}

/// A file the run reads for its programs (decision 0512), parsed in its own grammar as `parsed`, as the two documents
/// it is to the run: the file, a text document under its own key (its `doc` row, its lines and their text, shown as
/// any file's), and its tree, a guest of the file (`Made::File`) under a hidden key (`hidden_file_key`), standing at
/// the file's whole text. Both read the one source and line table (`core::text_of`).
pub fn file_and_tree(parsed: Document, keys: &GuestKeys) -> [Document; 2] {
    let text = Arc::new(core::text_of(&parsed.facts));
    let file = parsed.key;
    let grammar = crate::code_rule_dsl::language::from_tape_tag(core::facts_tag(&parsed.facts)).and_then(crate::code_rule_dsl::language::label).unwrap_or_default().to_owned();
    let text_doc = Document { key: file, path: parsed.path.clone(), core: parsed.core.clone(), digest: core::digest(&text), facts: text, tape: None, guest: None, module: parsed.module.clone(), unpackaged: parsed.unpackaged };
    let tree = Document {
        key: hidden_file_key(keys, &parsed.path),
        core: format!("{}#file:{grammar}", parsed.core),
        guest: Some(Arc::new(file_tree_guest(file, grammar, parsed.facts.source.len()))),
        ..parsed
    };
    [text_doc, tree]
}

/// The place of the tree of file `file` (`file_and_tree`), of `grammar` and `len` bytes: its file's guest standing at
/// its whole text. No row reads a file tree's `at` (`Made::File` has no `sub_tape` or `parsed` row): its file's first
/// position.
pub(super) fn file_tree_guest(file: u32, grammar: String, len: usize) -> Guest {
    let whole = Placement::Copied { delta: 0, within: [0, u32::try_from(len).unwrap_or(u32::MAX)] };
    Guest { host: file, file, at: i64::from(file) << 32, grammar, role: "file".to_owned(), root: 0, place: whole.clone(), in_file: whole, made: Made::File }
}

/// Whether `d` is a file the run reads for its programs, as parsed and not yet split (`file_and_tree`).
pub fn unsplit_host(d: &Document, host: &dyn Fn(&str) -> bool) -> bool {
    d.guest.is_none() && host(&d.path) && core::facts_tag(&d.facts) != core::TEXT
}

/// Which keys a document takes (`key`).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Range {
    Code,
    RunValue,
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
    let _ = host;
    let f = ids.f()?;
    let (id, kind) = ids.node(local)?;
    let (start, end) = (f.start(id), f.end(id));
    let node = [start, end];
    let tag = core::facts_tag(f.facts);
    let label = crate::code_rule_dsl::language::from_tape_tag(tag).and_then(crate::code_rule_dsl::language::label).unwrap_or("");
    let is_string = |kind: &str| matches!(kind, "string" | "string_literal" | "raw_string_literal" | "interpreted_string_literal");
    if is_string(kind) {
        return string_text(label, kind, f.text(id), start, node);
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

/// The guests' makers' state for one chunk: the run's language and shape, the hosts' numberings, the guests made, and
/// the run's guest keys (the store's, decision 0391).
#[derive(Default)]
pub struct Guests {
    run: &'static str,
    /// The run's projection: its node ids are what requests name (`syntax::numbering`).
    shape: syntax::Shape,
    numbering: HashMap<u32, syntax::Numbering>,
    made: HashSet<(i64, String)>,
    asked: HashSet<Request>,
    keys: Arc<GuestKeys>,
    /// Each guest's host file (`memo` attributes a guest's rows to it).
    pub host_of: HashMap<i64, i64>,
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
    /// The files' programs made this chunk (decision 0431 step 6, `Report::programs`): read off a file's record, and
    /// made by an embedder from a file whose record is not known.
    pub programs_read: usize,
    pub programs_made: usize,
}

/// A program to place in its file (`Guests::placed_program`): an embedder's, or one read off the file's record (decision
/// 0431 step 6), its parts shared with the record, not copied.
struct Held {
    dialect: crate::code_rule_dsl::language::GrammarDialect,
    parse: crate::code_rule_dsl::embed::Parse,
    text: Arc<String>,
    segments: Arc<crate::code_rule_dsl::embed::segments::SegmentMap>,
    at: Option<std::ops::Range<usize>>,
}

impl Held {
    fn made(program: crate::code_rule_dsl::embed::Program) -> Self {
        Held { dialect: program.dialect, parse: program.parse, text: Arc::new(program.text), segments: Arc::new(program.segments), at: program.at }
    }

    fn kept(image: &crate::code_rule_dsl::tape::DerivedImage) -> Self {
        Held {
            dialect: image.dialect(),
            parse: crate::code_rule_dsl::embed::Parse::Tape(image.tape.clone()),
            text: Arc::clone(&image.text),
            segments: Arc::clone(image.source_map()),
            at: image.at.map(|(start, end)| start as usize..end as usize),
        }
    }
}

/// Which guests a program can see (decision 0058 §1, `hiding`): a guest it cannot see adds no row any of its rules
/// reads, so a chunk running it makes none. Ordered by range: each sees the ranges before it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Seen {
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
            Range::RunValue => self >= Seen::Annotations,
            Range::Hidden => self == Seen::All,
        }
    }
}

/// The tape tags of the grammars `v`'s requests name (decision 0431 R6: the tags whose tapes a run reads before its
/// first wave); `None` where a request's grammar is not a constant, so any grammar may be asked.
pub fn grammar_tags(v: &crate::code_rule_dsl::logic::program_image::Version) -> Option<std::collections::BTreeSet<i64>> {
    use crate::code_rule_dsl::logic::program_image::TermRef;
    let mut tags = std::collections::BTreeSet::new();
    for r in v.rules() {
        let column = match v.rel_name(r.head().rel()) {
            DEMAND | PARSE => 1,
            _ => continue,
        };
        match r.head().args().nth(column) {
            Some(TermRef::Str(a)) => {
                if let Some(tag) = grammar(v.text(a)).and_then(|(label, fragment)| tape_tag_of(label, fragment)) {
                    tags.insert(tag);
                }
            }
            _ => return None,
        }
    }
    Some(tags)
}

/// Whether `t` is a document bound `combining::FoldOwn` widened to take the annotations in (above `RUN_VALUE_KEYS`):
/// a rule compared with it reads the annotations' documents, which only their requests (`DEMAND`) make.
pub fn widened(t: crate::code_rule_dsl::logic::program_image::TermRef<'_>) -> bool {
    use crate::code_rule_dsl::facts::vtab::syntax::{RUN_VALUE_KEYS, HIDDEN_KEYS};
    matches!(t, crate::code_rule_dsl::logic::program_image::TermRef::Int(n) if n > i64::from(RUN_VALUE_KEYS) && (n <= i64::from(HIDDEN_KEYS) || n > i64::from(RUN_VALUE_KEYS) << 32))
}

/// The guests `v`'s rules can see. A variable reaches a guest's document only from a guest relation (scoped,
/// `hiding`): every other base read is bounded below `RUN_VALUE_KEYS`, or below `HIDDEN_KEYS` where an annotation's
/// copies were folded in. So a program that reads no guest relation sees annotations only where a comparison is
/// against the widened bound, and the other guests (other grammars, values) never.
pub fn guests_seen(v: &crate::code_rule_dsl::logic::program_image::Version) -> Seen {
    use crate::code_rule_dsl::logic::program_image::Lit;
    let entry = |rel: &str| READS.contains(&rel) || super::hiding::ENTRIES.iter().any(|(r, ..)| *r == rel);
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
    pub fn new(run: &'static str, shape: syntax::Shape, keys: Arc<GuestKeys>) -> Self {
        Self { run, shape, keys, ..Self::default() }
    }

    /// Makes only the guests a program that sees `seen` can see (a chunk of partial hits, whose cone is all it runs).
    pub fn seeing(mut self, seen: Seen) -> Self {
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
    pub fn askers(&self) -> HashMap<i64, Vector<i64>> {
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

    /// A string a rule parses, asked inside the chunk's fixpoint (decision 0431 §8: `rr_embed_req(at, grammar)`'s
    /// answer): its tape value made (`Got::New`, added to `docs`), or a place more of one made before (`Got::Placed`,
    /// its document in `docs` given the place); `Got::Not` for a place asked before, or one with no tape.
    pub fn admit(&mut self, docs: &mut Vec<Document>, at: i64, grammar: &str, parses: &Parses<'_>) -> Result<Got, String> {
        if !self.made.insert((at, grammar.to_owned())) {
            return Ok(Got::Not);
        }
        let got = {
            let by_key: HashMap<u32, &Document> = docs.iter().map(|d| (d.key, d)).collect();
            self.guest(&by_key, at, grammar, parses)?
        };
        match &got {
            Got::New(d) => docs.push(Document::clone(d)),
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

    /// `@parse`'s or `@splice`'s value (`request`), asked inside the chunk's fixpoint (decision 0431 §8, Q325): made and
    /// added to `docs`, or `None` where it was asked before, made before (its askers then grow), or makes no value.
    pub fn admit_value(&mut self, docs: &mut Vec<Document>, request: Request, parses: &Parses<'_>) -> Result<Option<Document>, String> {
        if !self.asked.insert(request.clone()) {
            return Ok(None);
        }
        // A value is never shown (`Range::Hidden`): a program that sees no hidden guest makes none.
        if !self.seen.sees(Range::Hidden) {
            self.unseen += 1;
            return Ok(None);
        }
        let made = {
            let by_key: HashMap<u32, &Document> = docs.iter().map(|d| (d.key, d)).collect();
            match request {
                Request::Parse { at, grammar, text } => {
                    let asker = super::memo::document_of(at);
                    if self.known(&grammar, &format!("parse:{text}"), asker) {
                        return Ok(None);
                    }
                    self.value(&by_key, asker, &grammar, text.clone(), Made::Parse { text }, parses)?
                }
                Request::Splice { node, with } => {
                    let asker = super::memo::document_of(node);
                    if self.known("", &format!("splice:{node}:{with}"), asker) {
                        return Ok(None);
                    }
                    match self.splice(&by_key, docs, node, &with)? {
                        Some((grammar, text, tape, _)) => self.value(&by_key, asker, &grammar, text, Made::Splice { tape, node, with }, parses)?,
                        None => None,
                    }
                }
            }
        };
        match &made {
            Some(d) => docs.push(d.clone()),
            None => self.refused += 1,
        }
        Ok(made)
    }

    /// The guest at host node `at` a rule parses as `grammar_name`: the program its host holds through an embedding
    /// of that name (made with the batch, `seed`, else made here), or the string there, made a tape value (decision
    /// 0431): one per grammar and text, its places the strings it was parsed at; a text made already gains a place.
    fn guest(&mut self, by_key: &HashMap<u32, &Document>, at: i64, grammar_name: &str, parses: &Parses<'_>) -> Result<Got, String> {
        let host_key = (at >> 32) as u32;
        let Some(host) = by_key.get(&host_key).copied() else { return Ok(Got::Not) };
        if let Some(via) = embedding(host, grammar_name) {
            return Ok(self.program(host, at, via, "program")?.map_or(Got::Not, |d| Got::New(Box::new(d))));
        }
        let Some((label, fragment)) = grammar(grammar_name) else { return Ok(Got::Not) };
        // A tape value is never code to rules that do not name it: its nodes are no file's, so a finding on one has no
        // place. In the run's language it is in the run values' range (a type expression, Python's in a Python run);
        // shown, a string the demand asked for only because its rule's inputs hold it (every string, for `type_ctx`)
        // was code to every rule: `"--host"` was `--host`, a doubled minus (B002 on 157 argparse lines).
        let range = if label == self.run { Range::RunValue } else { Range::Hidden };
        if !self.seen.sees(range) {
            self.unseen += 1;
            return Ok(Got::Not);
        }
        let ids = self.numbering(host)?;
        // A node id's low bits are its local id only under preorder ids (`syntax::Numbering`).
        let Some(local) = ids.local((at & 0xffff_ffff) as u32) else { return Ok(Got::Not) };
        let Some((text, place)) = host_text(host, ids, local) else { return Ok(Got::Not) };
        self.tape_value(host, host_key, at, grammar_name, "fragment", label, fragment, text, place, range, parses)
    }

    /// String `at` of `host`, its `text` parsed as `label`: a tape value (decision 0431), keyed by its grammar and
    /// text, not its place. A text made already in this chunk gains the place; else it is parsed (once a run,
    /// `parse`) and made. Its rows are the files' that hold it (`files`, as a value's).
    #[allow(clippy::too_many_arguments)]
    fn tape_value(&mut self, host: &Document, host_key: u32, at: i64, grammar_name: &str, role: &str, label: &str, fragment: Option<core::Fragment>, text: String, place: Placement, range: Range, parses: &Parses<'_>) -> Result<Got, String> {
        let identity = format!("tape:{text}");
        let files = self.files_of(i64::from(host_key));
        let here = syntax::TapePlace { host: host_key, at, place };
        if let Some(key) = self.values.get(&(grammar_name.to_owned(), identity.clone())).copied() {
            self.files.entry(key).or_default().extend(files);
            return Ok(Got::Placed(key as u32, here));
        }
        let Some((facts, root, digest)) = parse(&host.path, label, fragment, text.clone(), parses)? else { return Ok(Got::Not) };
        let key = self.key_for(&[b"tape", grammar_name.as_bytes(), identity.as_bytes()], range, &host.path);
        self.values.insert((grammar_name.to_owned(), identity), i64::from(key));
        self.files.insert(i64::from(key), files);
        let none = Placement::At([0, 0]);
        Ok(Got::New(Box::new(Document {
            key,
            path: host.path.clone(),
            core: format!("#tape:{grammar_name}:{key}"),
            digest,
            facts,
            guest: Some(Arc::new(Guest { host: 0, file: 0, at: 0, grammar: grammar_name.to_owned(), role: role.to_owned(), root, place: none.clone(), in_file: none, made: Made::Tape { places: vec![here] } })),
            // A tape value is no module's, and no package's: its text is a string's, not a file's.
            module: None,
            unpackaged: false,
            tape: None,
        })))
    }

    /// The program host file `host` embeds through `via` (decision 0222): the embedder main registered for it
    /// (`embed::Embedder`; a Vue component's view, `vue::view`) writes its text and its segments from the host's
    /// tape, read in place, and the text is parsed in the dialect the embedder chose (a `<script lang="ts">`'s TypeScript). The
    /// guest's grammar is that dialect's label, as `guest` names what a document is parsed as; its nodes are placed
    /// through the segments (`Placement::Segments`). `None` where the host embeds no program (no script, `lang="coffee"`).
    fn program(&mut self, host: &Document, at: i64, via: crate::code_rule_dsl::language::Via, role: &str) -> Result<Option<Document>, String> {
        let Some(program) = embedded(host, &mut |root, source| crate::code_rule_dsl::embed::embedder(via).build(root, source, &|| false).into_iter().collect()).into_iter().next() else { return Ok(None) };
        self.placed_program(host, Some(at), via, role, Held::made(program))
    }

    /// The files' programs among `docs` (made with the batch, `programs`), as made: their places, under their grammar
    /// and their embeddings' ids, and their files (`host_of`).
    pub fn seed(&mut self, docs: &[Document]) {
        let by_key: HashMap<u32, &Document> = docs.iter().map(|d| (d.key, d)).collect();
        for d in docs {
            let Some(g) = d.guest.as_ref().filter(|g| g.made == Made::Host && g.file != 0 && g.file != d.key) else { continue };
            self.made.insert((g.at, g.grammar.clone()));
            let host = by_key.get(&g.host).and_then(|h| crate::code_rule_dsl::language::from_tape_tag(core::facts_tag(&h.facts)));
            let dialect = crate::code_rule_dsl::language::from_tape_tag(core::facts_tag(&d.facts));
            for &(via, of) in host.map_or(&[][..], |h| h.embeds()) {
                if Some(of) == dialect {
                    self.made.insert((g.at, via.id().to_owned()));
                }
            }
            self.host_of.insert(i64::from(d.key), i64::from(g.file));
        }
    }

    /// The programs each file of `docs` holds that its language declares (decision 0431 §4, step 3): every program
    /// each of its embeddings' embedders builds (`Embedder::build_all`) from the file's tape, read in place, at its
    /// load, before any rule runs. Each is at the innermost node of its file over its bytes, made once: a library's
    /// `embed` of the same place and grammar finds it made.
    pub fn programs(&mut self, docs: &[Document]) -> Result<Vec<Document>, String> {
        let mut out = Vec::new();
        // A file's own tree is a host of its programs (decision 0512), whose file is the file's.
        for host in docs.iter().filter(|d| d.guest.as_ref().is_none_or(|g| g.made == Made::File)) {
            if let Some(g) = host.guest.as_ref() {
                self.host_of.insert(i64::from(host.key), i64::from(g.file));
            }
            // Made at its parse, or read with its stored tape: placed, with no embedder run and nothing parsed.
            if let Some(record) = host.facts.programs.as_ref() {
                for image in record.iter() {
                    self.programs_read += 1;
                    out.extend(self.placed_program(host, None, image.via, "program", Held::kept(image))?);
                }
                continue;
            }
            let Some(dialect) = crate::code_rule_dsl::language::from_tape_tag(core::facts_tag(&host.facts)) else { continue };
            let mut vias: Vec<crate::code_rule_dsl::language::Via> = Vec::new();
            for &(via, _) in dialect.embeds() {
                if !vias.contains(&via) {
                    vias.push(via);
                }
            }
            for via in vias {
                let embedder = crate::code_rule_dsl::embed::embedder(via);
                // An embedder that reads the file's scope table reads it from a document over the host's tape, read in
                // place (decision 0470); another reads the host's nodes.
                let doc = || {
                    let (tape, grammar) = (host.facts.tape.as_ref()?, host.facts.grammar()?);
                    Some(crate::code_rule_dsl::tape::TapeDoc::over(Arc::clone(tape), Arc::clone(grammar), Arc::clone(&host.facts.lines)))
                };
                let programs = if embedder.reads_scope() {
                    host.tape.as_ref().map(|(doc, _)| doc.clone()).or_else(doc).map_or_else(Vec::new, |doc| embedder.build_in_doc(&doc, &|| false))
                } else {
                    embedded(host, &mut |root, source| embedder.build_all(root, source, &|| false))
                };
                for program in programs {
                    self.programs_made += 1;
                    out.extend(self.placed_program(host, None, via, "program", Held::made(program))?);
                }
            }
        }
        Ok(out)
    }

    /// The document of `program`, which `via` built from `host`, at host node `at` (else the innermost node over its
    /// bytes, and made once there); `None` where the run does not see it, or its text does not parse.
    fn placed_program(&mut self, host: &Document, at: Option<i64>, via: crate::code_rule_dsl::language::Via, role: &str, program: Held) -> Result<Option<Document>, String> {
        let Some(label) = crate::code_rule_dsl::language::label(program.dialect) else { return Ok(None) };
        let range = if label == self.run { Range::Code } else { Range::Hidden };
        if !self.seen.sees(range) {
            self.unseen += 1;
            return Ok(None);
        }
        let segments = Arc::clone(&program.segments);
        let place = Placement::Segments(program.segments);
        let at = match at {
            Some(at) => at,
            None => {
                // A program of its own place stands at the innermost node over its bytes, a whole file's at its root;
                // made, it is made under its grammar and its embedding both, as a library's `embed` names either.
                let whole = [0, u32::try_from(host.facts.source.len()).unwrap_or(u32::MAX)];
                let narrow = |b: usize| u32::try_from(b).unwrap_or(u32::MAX);
                let bytes = match &program.at {
                    Some(at) => [narrow(at.start), narrow(at.end)],
                    None if crate::code_rule_dsl::embed::embedder(via).per_place() => place.host([0, narrow(program.text.len())]),
                    None => whole,
                };
                let ids = self.numbering(host)?;
                let Some(at) = (if bytes == whole { ids.root() } else { ids.over(bytes) }) else { return Ok(None) };
                let (by_grammar, by_via) = (self.made.insert((at, label.to_owned())), self.made.insert((at, via.id().to_owned())));
                if !(by_grammar && by_via) {
                    return Ok(None);
                }
                at
            }
        };
        let host_key = (at >> 32) as u32;
        let tape_verdict = crate::code_rule_dsl::embed::embedder(via).tape_verdict();
        let Some(((facts, root, digest), tape)) = taped(&host.path, label, program.text, program.parse, &segments, tape_verdict)? else { return Ok(None) };
        let key = self.key_for(&[host.path.as_bytes(), &at.to_le_bytes(), via.id().as_bytes()], range, &host.path);
        let file = self.host_of.get(&i64::from(host_key)).copied().unwrap_or(i64::from(host_key));
        self.host_of.insert(i64::from(key), file);
        let file = file as u32;
        // A file's own tree stands at its file's bytes as they are (decision 0512): its program's are its own place's.
        let in_file = host.guest.as_ref().filter(|h| h.made != Made::File).map_or_else(|| place.clone(), |h| h.in_file.then(place.clone()));
        Ok(Some(Document {
            key,
            path: host.path.clone(),
            core: format!("{}#guest:{}:{at}", host.core, via.id()),
            digest,
            facts,
            guest: Some(Arc::new(Guest { host: host_key, file, at, grammar: label.to_owned(), role: role.to_owned(), root, place, in_file, made: Made::Host })),
            module: host.module.clone(),
            unpackaged: host.unpackaged,
            tape,
        }))
    }

    /// A value document (decision 0058 §4): `text` parsed as `grammar_name`, with no host, in the hidden documents,
    /// asked for by document `asker`. Its places are its own text's. It is its askers' files' (O96): `memo` keeps its
    /// rows with each, and its path is the first's.
    fn value(&mut self, by_key: &HashMap<u32, &Document>, asker: i64, grammar_name: &str, text: String, made: Made, parses: &Parses<'_>) -> Result<Option<Document>, String> {
        let Some((label, fragment)) = grammar(grammar_name) else { return Ok(None) };
        let (identity, known_as) = match &made {
            Made::Splice { node, with, .. } => (format!("splice:{node}:{with}"), String::new()),
            _ => (format!("parse:{text}"), grammar_name.to_owned()),
        };
        let files = self.files_of(asker);
        let path = files.first().and_then(|f| u32::try_from(*f).ok()).and_then(|f| by_key.get(&f)).map_or_else(String::new, |d| d.path.clone());
        let path = path.as_str();
        let Some((facts, root, digest)) = parse(path, label, fragment, text, parses)? else { return Ok(None) };
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
            unpackaged: false,
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
        // Each document read in a batch of its own (`Facts::alone`): the splice reads two nodes of one and the text of
        // the other.
        let Some(text) = d.facts.alone(|f| {
            let base = syntax::unparsed(f, d);
            // The unparsed text's offset in the document's text (where its root is not its first node).
            let off = match d.guest.as_deref() {
                Some(g) if g.root != 0 => f.start(g.root) as usize,
                _ => 0,
            };
            let (a, b) = (f.start(id) as usize, f.end(id) as usize);
            let (Some(a), Some(b)) = (a.checked_sub(off), b.checked_sub(off)) else { return None };
            if b > base.len() || !base.is_char_boundary(a) || !base.is_char_boundary(b) {
                return None;
            }
            Some(w.facts.alone(|wf| format!("{}{}{}", &base[..a], syntax::unparsed(wf, w), &base[b..])))
        }) else {
            return Ok(None);
        };
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

/// Each program's file among `docs` (`Guests::host_of`'s, for a chunk that makes no guest of its own).
pub fn hosts_of(docs: &[Document]) -> HashMap<i64, i64> {
    docs.iter().filter_map(|d| d.guest.as_ref().filter(|g| matches!(g.made, Made::Host | Made::File) && g.file != 0 && g.file != d.key).map(|g| (i64::from(d.key), i64::from(g.file)))).collect()
}

/// The programs `build` makes from `host`'s tape, read in place where its facts hold it (whichever parser wrote it,
/// the same tape); none for a host with no tape.
fn embedded(host: &Document, build: &mut dyn FnMut(crate::code_rule_dsl::embed::host::HostNode<'_>, &str) -> Vec<crate::code_rule_dsl::embed::Program>) -> Vec<crate::code_rule_dsl::embed::Program> {
    let Some(dialect) = crate::code_rule_dsl::language::from_tape_tag(core::facts_tag(&host.facts)) else { return Vec::new() };
    let Some(tape) = host.facts.tape.as_ref() else { return Vec::new() };
    let Ok(grammar) = crate::code_rule_dsl::tape::Grammar::new(dialect) else { return Vec::new() };
    let cache = tape.image().own_cache();
    let Some(root) = crate::code_rule_dsl::embed::host::Host::new(tape.image(), &grammar, cache.as_ref()).root() else { return Vec::new() };
    build(root, host.facts.source.as_str())
}

/// A guest's or value's tree: its facts, its root (0: a fragment's tape is rooted at its start node, decision 0466),
/// and the facts' digest.
type Parsed = (Arc<core::Facts>, u32, [u8; 32]);

/// The guests' trees by grammar and text, for one run: a pack's guests repeat their texts (one annotation in many files,
/// the same guests in every chunk and round), and the facts are a function of the two alone, so each distinct text is
/// parsed and digested once a run. Held by the run, not the process (no static memo). Decision 0431 R6 (it replaces Q30,
/// which judged a parse cheaper than a store's write and read, about 0.01 s a run on ep2): a text's tape is found among
/// the tapes the last runs named, read before the first wave (decision 0104), or queued for the store behind the run,
/// so a text a run parsed is not parsed again.
#[derive(Default)]
pub struct Parses<'a> {
    kept: std::sync::Mutex<(usize, HashMap<(String, Option<core::Fragment>, String), Option<Parsed>>)>,
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
pub struct ValueStore<'a> {
    pub cache: &'a crate::content_store::ContentStore,
    pub run_tag: Option<i64>,
    pub by_text: &'a HashMap<crate::content_store::HashKey, (crate::content_store::SourceId, crate::content_store::TapeFrame)>,
    pub others: HashMap<(crate::content_store::HashKey, i64), (crate::content_store::SourceId, crate::content_store::TapeFrame)>,
    pub late: &'a super::Late,
}

impl<'a> Parses<'a> {
    /// The parses of a run whose tapes are kept in `store`.
    pub fn new(store: Option<ValueStore<'a>>) -> Self {
        Self { store, ..Self::default() }
    }

    /// The texts this run parsed.
    pub fn parsed(&self) -> usize {
        self.parsed.load(std::sync::atomic::Ordering::Relaxed)
    }
}

/// `text` parsed as `label` (as `fragment`, from its start kind, where it is one), its root and its facts' digest;
/// `None` when the parser gives no facts. Kept for the run by grammar and text (`Parses`).
fn parse(path: &str, label: &str, fragment: Option<core::Fragment>, text: String, parses: &Parses<'_>) -> Result<Option<Parsed>, String> {
    /// The texts kept, in bytes, before the memo starts again (the facts kept are a small multiple of it).
    const KEPT_BYTES: usize = 8 << 20;
    let key = (label.to_owned(), fragment, text);
    if let Some(hit) = parses.kept.lock().expect("the parsed guests").1.get(&key) {
        crate::code_rule_dsl::prof::count("guests.parse_kept", 1);
        return Ok(hit.clone());
    }
    let file = crate::code_rule_dsl::facts::core::Source { language: label.to_owned(), source: Arc::new(key.2.clone()) };
    let built = match (&parses.store, tape_tag_of(label, fragment)) {
        // Decision 0431 R6: found among the last runs' tapes by its text, or queued earlier in this run; else parsed,
        // and queued for the store with its source, which the manifest names.
        (Some(s), Some(tag)) if cfg!(feature = "tape-cache") => {
            use sha2::Digest as _;
            let digest: [u8; 32] = sha2::Sha256::digest(key.2.as_bytes()).into();
            let hash = crate::content_store::HashKey::from_sha256_digest(digest);
            let found = if s.run_tag == Some(tag) { s.by_text.get(&hash) } else { s.others.get(&(hash, tag)) };
            match found {
                Some((source, _)) => s.late.lock().expect("the late sources").push(*source),
                None if core::tape_queued(s.cache, &file, &digest, fragment) => {}
                None => {
                    parses.parsed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
            }
            match fragment {
                Some(f) => core::build_fragment_in_wave(&file, f, s.cache, Some(digest), found.map(|(_, f)| f), true),
                None => core::build_in_wave_with_tape(&file, s.cache, Some(digest), found.map(|(_, f)| f), true, false).map(|(b, _)| b),
            }
        }
        _ => {
            parses.parsed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            match fragment {
                Some(f) => core::build_fragment(&file, f),
                None => core::build(&file),
            }
        }
    };
    let built = built.map_err(|e| format!("{path}: {e}"))?;
    let parsed = match built {
        Built::Facts(facts) => {
            let digest = core::digest(&facts);
            Some((Arc::new(facts), 0, digest))
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

/// An embedded program's facts, read off the tape or tree its embedder parsed its text into (`parse`), with its tape
/// kept beside them (decision 0222): the rules over a view read W's values (`value`, a key's text) as over a file,
/// and a value is read off the document's tape. Its tape is not stored (Q30).
fn taped(path: &str, label: &str, text: Arc<String>, parse: crate::code_rule_dsl::embed::Parse, segments: &crate::code_rule_dsl::embed::segments::SegmentMap, tape_verdict: bool) -> Result<Option<(Parsed, Tape)>, String> {
    let file = crate::code_rule_dsl::facts::core::Source { language: label.to_owned(), source: text };
    // Read off the embedder's parse, not parsed again.
    let built = core::build_parsed(&file, parse, segments, true).map_err(|e| format!("{path}: {e}"))?;
    let (Built::Facts(mut facts), tape) = built else { return Ok(None) };
    facts.tape_verdict = tape_verdict;
    let digest = core::digest(&facts);
    let tape = tape.map(|doc| (doc, Arc::new(file) as Arc<dyn crate::code_rule_dsl::facts::core::SourceFile>));
    Ok(Some(((Arc::new(facts), 0, digest), tape)))
}

/// A document's tape in its file's context (`Document::tape`).
type Tape = Option<(crate::code_rule_dsl::tape::TapeDoc, Arc<dyn crate::code_rule_dsl::facts::core::SourceFile>)>;

/// Sources a queued write stores (decision 0102), for the manifest queued after it.
pub type Late = Arc<std::sync::Mutex<Vec<crate::content_store::SourceId>>>;

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
                let k = first_key(&[&x, &y, &z], Range::RunValue).2;
                seen.insert(k, p.clone()).map(|q| (q, p))
            })
            .expect("a pair");
        let key = |g: &mut Guests, p: &str| {
            let [x, y, z] = parts(p);
            g.key_for(&[&x, &y, &z], Range::RunValue, p)
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
