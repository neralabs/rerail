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
//! - any other node: its text, copied;
//! - `embed_value`'s text, standing at `at`.
//!
//! A grammar name is a dialect label (`python`, `sql`, `typescript`, …) or `annotation`: Python's expression, parsed
//! as `( … )` with the expression inside the parentheses as its root, as rrlab does. Its tape is the tape cache's,
//! keyed by its text, so equal texts in any file are parsed once.

use crate::code_rule_dsl::facts::core::{self, Built};
use crate::code_rule_dsl::facts::vtab::syntax::{self, Document, Guest, Made, Placement};
use crate::code_rule_dsl::model::FileInput;
use crate::content_store::ContentStore;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

/// Rounds of the embed slice per chunk: embeds of embeds, to this depth. A guard, not a limit anyone should reach.
pub(super) const DEPTH: usize = 4;

/// The relations requests are read from.
pub(super) const LIBRARY: [&str; 2] = ["embed", "embed_value"];
pub(super) const DEMAND: &str = "rr_embed_req";
/// `@parse(g, x)`'s and `@splice(t, n, t2)`'s demands (decision 0058 §4, `rr::lower`).
pub(super) const PARSE: &str = "rr_parse_req";
pub(super) const SPLICE: &str = "rr_splice_req";
/// The demands rules make.
pub(super) const DEMANDS: [&str; 3] = [DEMAND, PARSE, SPLICE];
/// Every relation a request is read from: a run with a rule deriving one makes guests.
pub(in crate::code_rule_dsl) const REQUESTS: [&str; 5] = ["embed", "embed_value", DEMAND, PARSE, SPLICE];
/// The engine's relations a combined run shares across its programs (`corpus::combine` prefixes no program's).
pub(super) const SHARED: [&str; 4] = [DEMAND, PARSE, SPLICE, crate::code_rule_dsl::facts::vtab::syntax::GUEST_POLICY];

/// One document asked for.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(super) enum Request {
    /// The code at host node `at`, in `grammar`, with its text when it is a value (`embed_value`).
    Guest { at: i64, grammar: String, role: String, text: Option<String> },
    /// `@parse(grammar, text)`.
    Parse { grammar: String, text: String },
    /// `@splice(_, node, with)`: node's tape, with node's text replaced by tape `with`'s.
    Splice { node: i64, with: String },
}

/// The requests of a slice run's outputs, in order. A library's embeds are made whatever a program shows: `.guests
/// off` hides them from its rules (decision 0058 §1), and `guest` still reaches them.
pub(super) fn requests(outputs: &BTreeMap<String, Vec<String>>) -> Vec<Request> {
    let mut out = Vec::new();
    let int = |s: &str| s.parse::<i64>().ok();
    let rows = |rel: &str, n: usize| outputs.get(rel).into_iter().flatten().map(move |line| line.splitn(n, '\t').collect::<Vec<&str>>());
    for c in rows("embed", 4) {
        if let [_, at, g, r] = c.as_slice()
            && let Some(at) = int(at)
        {
            out.push(Request::Guest { at, grammar: (*g).to_owned(), role: (*r).to_owned(), text: None });
        }
    }
    for c in rows("embed_value", 5) {
        if let [_, at, g, r, t] = c.as_slice()
            && let Some(at) = int(at)
        {
            out.push(Request::Guest { at, grammar: (*g).to_owned(), role: (*r).to_owned(), text: Some((*t).to_owned()) });
        }
    }
    for c in rows(DEMAND, 2) {
        if let [at, g] = c.as_slice()
            && let Some(at) = int(at)
        {
            out.push(Request::Guest { at, grammar: (*g).to_owned(), role: "fragment".to_owned(), text: None });
        }
    }
    // The text is last: it may hold a tab.
    // `at`, the location that asked, is not part of what is made: equal texts are one value.
    for c in rows(PARSE, 3) {
        if let [_, g, x] = c.as_slice() {
            out.push(Request::Parse { grammar: (*g).to_owned(), text: (*x).to_owned() });
        }
    }
    for c in rows(SPLICE, 2) {
        if let [n, t2] = c.as_slice()
            && let Some(node) = int(n)
        {
            out.push(Request::Splice { node, with: (*t2).to_owned() });
        }
    }
    out.sort();
    out.dedup();
    out
}

/// A grammar name's dialect label, and whether its text is parsed in a wrapper (an expression's `( … )`).
fn grammar(name: &str) -> Option<(&'static str, bool)> {
    if name == "annotation" {
        return Some(("python", true));
    }
    let dialect = crate::code_rule_dsl::language::dialect_by_label(name)?;
    crate::code_rule_dsl::language::label(dialect).map(|label| (label, false))
}

/// A document's key past every file's (`memo::doc_keys` gives files keys below `GUEST_KEYS`), with `key << 32`
/// positive: a function of what it is (a guest's host path, place and grammar), so a memoised row's guest nodes keep
/// their ids. A guest in the run's language takes `GUEST_KEYS..HIDDEN_KEYS`, which a policy shows; every other
/// document `HIDDEN_KEYS..2^31` (decision 0058 §1).
fn key(parts: &[&[u8]], shown: bool, taken: &mut HashSet<u32>) -> u32 {
    use crate::code_rule_dsl::facts::vtab::syntax::{GUEST_KEYS, HIDDEN_KEYS};
    let (base, span) = if shown { (GUEST_KEYS, HIDDEN_KEYS - GUEST_KEYS) } else { (HIDDEN_KEYS, (1u32 << 31) - HIDDEN_KEYS) };
    let mut h = Sha256::new();
    for p in parts {
        h.update((p.len() as u64).to_le_bytes());
        h.update(p);
    }
    let hash: [u8; 32] = h.finalize().into();
    let mut k = u32::from_le_bytes(hash[..4].try_into().expect("4 bytes")) % span;
    while !taken.insert(base + k) {
        k = (k + 1) % span;
    }
    base + k
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
fn host_text(host: &Document, ids: &[(u32, &'static str)], local: u32) -> Option<(String, Placement)> {
    let f = &*host.facts;
    let &(id, kind) = ids.get(local as usize)?;
    let (start, end) = (f.start[id as usize], f.end[id as usize]);
    let node = [start, end];
    let tag = f.kind.first().map_or(0, |k| k / core::KIND_STRIDE);
    let label = u8::try_from(tag).ok().and_then(crate::code_rule_dsl::language::from_tape_tag).and_then(crate::code_rule_dsl::language::label).unwrap_or("");
    let is_string = |kind: &str| matches!(kind, "string" | "string_literal" | "raw_string_literal" | "interpreted_string_literal");
    if is_string(kind) {
        return string_text(label, kind, f.text(id), start, node);
    }
    if kind == "concatenated_string" {
        let mut value = String::new();
        let mut pieces = Vec::new();
        let mut tail = i64::from(end);
        for &c in f.children(id) {
            let Some(&(_, k)) = ids.iter().find(|&&(i, _)| i == c) else { continue };
            if !is_string(k) {
                continue;
            }
            let (text, place) = string_text(label, k, f.text(c), f.start[c as usize], node)?;
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
                Placement::At(_) => return None,
            }
            value.push_str(&text);
        }
        pieces.push(syntax::normalized::Piece { d: value.len() as u32, s: tail, escape: false });
        return Some((value, Placement::Mapped { pieces: pieces.into(), within: node }));
    }
    Some((f.text(id).to_owned(), Placement::Copied { delta: i64::from(start), within: node }))
}

/// The guests' makers' state for one chunk: the run's language, the hosts' numberings, the guests made, and the keys
/// taken.
#[derive(Default)]
pub(super) struct Guests {
    run: &'static str,
    numbering: HashMap<u32, Vec<(u32, &'static str)>>,
    made: HashSet<(i64, String)>,
    asked: HashSet<Request>,
    taken: HashSet<u32>,
    /// Each guest's host file (`memo` attributes a guest's rows to it).
    pub(super) host_of: HashMap<i64, i64>,
    /// Made, and given no guest (no text to parse, or no grammar), this chunk.
    pub(super) refused: usize,
}

impl Guests {
    /// For a run over `run`'s files (a dialect label).
    pub(super) fn new(run: &'static str) -> Self {
        Self { run, ..Self::default() }
    }

    /// The documents `requests` ask for that are not made yet; `docs` holds their hosts.
    pub(super) fn make(&mut self, requests: Vec<Request>, docs: &[Document], cache: Option<&ContentStore>) -> Result<Vec<Document>, String> {
        let by_key: HashMap<u32, &Document> = docs.iter().map(|d| (d.key, d)).collect();
        let mut out = Vec::new();
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
                Request::Guest { at, grammar, role, text } => self.guest(&by_key, at, &grammar, &role, text, cache)?,
                Request::Parse { grammar, text } => self.value(&grammar, text.clone(), Made::Parse { text }, "", cache)?,
                Request::Splice { node, with } => {
                    let spliced = self.splice(&by_key, docs, node, &with)?;
                    match spliced {
                        Some((grammar, text, tape, path)) => self.value(&grammar, text, Made::Splice { tape, node, with }, &path, cache)?,
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

    /// The guest at host node `at`.
    fn guest(&mut self, by_key: &HashMap<u32, &Document>, at: i64, grammar_name: &str, role: &str, text: Option<String>, cache: Option<&ContentStore>) -> Result<Option<Document>, String> {
        let host_key = (at >> 32) as u32;
        let Some(host) = by_key.get(&host_key).copied() else { return Ok(None) };
        let Some((label, wrap)) = grammar(grammar_name) else { return Ok(None) };
        let local = (at & 0xffff_ffff) as u32;
        let ids = self.numbering(host)?;
        let placed = match text {
            Some(text) => {
                let Some(&(id, _)) = ids.get(local as usize) else { return Ok(None) };
                Some((text, Placement::At([host.facts.start[id as usize], host.facts.end[id as usize]])))
            }
            None => host_text(host, ids, local),
        };
        let Some((text, place)) = placed else { return Ok(None) };
        let (text, place) = if wrap { wrapped(text, place) } else { (text, place) };
        let Some((facts, root)) = parse(&host.path, label, wrap, text, cache)? else { return Ok(None) };
        let key = key(&[host.path.as_bytes(), &at.to_le_bytes(), grammar_name.as_bytes()], label == self.run, &mut self.taken);
        self.host_of.insert(i64::from(key), self.host_of.get(&i64::from(host_key)).copied().unwrap_or(i64::from(host_key)));
        let in_file = host.guest.as_ref().map_or_else(|| place.clone(), |h| h.in_file.then(place.clone()));
        Ok(Some(Document {
            key,
            path: host.path.clone(),
            core: format!("{}#guest:{grammar_name}:{at}", host.core),
            digest: core::digest(&facts),
            facts: Arc::new(facts),
            guest: Some(Arc::new(Guest { host: host_key, at, grammar: grammar_name.to_owned(), role: role.to_owned(), root, place, in_file, made: Made::Host })),
        }))
    }

    /// A value document (decision 0058 §4): `text` parsed as `grammar_name`, with no host, in the hidden documents.
    /// Its places are its own text's; its rows belong to no file, so a chunk that reads one is not memoised.
    fn value(&mut self, grammar_name: &str, text: String, made: Made, path: &str, cache: Option<&ContentStore>) -> Result<Option<Document>, String> {
        let Some((label, wrap)) = grammar(grammar_name) else { return Ok(None) };
        let identity = match &made {
            Made::Splice { node, with, .. } => format!("splice:{node}:{with}"),
            _ => format!("parse:{text}"),
        };
        let (text, _) = if wrap { wrapped(text, Placement::At([0, 0])) } else { (text, Placement::At([0, 0])) };
        let Some((facts, root)) = parse(path, label, wrap, text, cache)? else { return Ok(None) };
        let key = key(&[b"value", grammar_name.as_bytes(), identity.as_bytes()], false, &mut self.taken);
        Ok(Some(Document {
            key,
            path: path.to_owned(),
            core: format!("#value:{grammar_name}:{key}"),
            digest: core::digest(&facts),
            facts: Arc::new(facts),
            guest: Some(Arc::new(Guest { host: 0, at: 0, grammar: grammar_name.to_owned(), role: "value".to_owned(), root, place: Placement::At([0, 0]), in_file: Placement::At([0, 0]), made })),
        }))
    }

    /// `@splice(_, node, with)`: the grammar, text, tape and path of node's document with node's text replaced by
    /// the text of a document whose tape is `with`; `None` when either is not in the chunk.
    fn splice(&mut self, by_key: &HashMap<u32, &Document>, docs: &[Document], node: i64, with: &str) -> Result<Option<(String, String, String, String)>, String> {
        let Some(d) = by_key.get(&((node >> 32) as u32)).copied() else { return Ok(None) };
        let Some(w) = docs.iter().find(|x| syntax::tape_hex(&x.digest) == with) else { return Ok(None) };
        let ids = self.numbering(d)?;
        let Some(&(id, _)) = ids.get((node & 0xffff_ffff) as usize) else { return Ok(None) };
        let base = syntax::unparsed(d);
        // The unparsed text's offset in the document's text (a wrapped fragment's root starts past the `(`).
        let off = match d.guest.as_deref() {
            Some(g) if g.root != 0 => d.facts.start[g.root as usize] as usize,
            _ => 0,
        };
        let (a, b) = (d.facts.start[id as usize] as usize, d.facts.end[id as usize] as usize);
        let (Some(a), Some(b)) = (a.checked_sub(off), b.checked_sub(off)) else { return Ok(None) };
        if b > base.len() || !base.is_char_boundary(a) || !base.is_char_boundary(b) {
            return Ok(None);
        }
        let text = format!("{}{}{}", &base[..a], syntax::unparsed(w), &base[b..]);
        let grammar = match d.guest.as_deref() {
            Some(g) => g.grammar.clone(),
            None => {
                let tag = d.facts.kind.first().map_or(0, |k| k / core::KIND_STRIDE);
                let Some(label) = u8::try_from(tag).ok().and_then(crate::code_rule_dsl::language::from_tape_tag).and_then(crate::code_rule_dsl::language::label) else { return Ok(None) };
                label.to_owned()
            }
        };
        Ok(Some((grammar, text, syntax::tape_hex(&d.digest), d.path.clone())))
    }

    fn numbering(&mut self, host: &Document) -> Result<&Vec<(u32, &'static str)>, String> {
        if !self.numbering.contains_key(&host.key) {
            let ids = syntax::numbering(host).map_err(|e| e.to_string())?;
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

/// `text` parsed as `label` through the tape cache, and its root (a wrapped expression's); `None` when the parser
/// gives no facts.
fn parse(path: &str, label: &str, wrap: bool, text: String, cache: Option<&ContentStore>) -> Result<Option<(core::Facts, u32)>, String> {
    let file = FileInput::new(path.to_owned(), label.to_owned(), Arc::new(text), None, None, "guest").map_err(|e| format!("{path}: {e:?}"))?;
    // The cache is shared by every run on the machine; a failed write there (O95) parses without it.
    let built = match cache.map(|cache| core::build_cached(&file, cache)) {
        Some(Ok(built)) => Ok(built),
        Some(Err(_)) => {
            crate::code_rule_dsl::prof::count("guests.cache_failed", 1);
            core::build(&file)
        }
        None => core::build(&file),
    }
    .map_err(|e| format!("{path}: {e}"))?;
    let Built::Facts(facts) = built else { return Ok(None) };
    let root = if wrap { wrapped_root(&facts, label) } else { 0 };
    Ok(Some((facts, root)))
}

/// The root of a wrapped expression `( e )`: `e`, the one named child of the parenthesized expression the module's
/// one statement is, when the text parses so; else the module's first named child (a partial tree's).
fn wrapped_root(f: &core::Facts, label: &str) -> u32 {
    let Some(dialect) = crate::code_rule_dsl::language::dialect_by_label(label) else { return 0 };
    let kinds = dialect.kinds();
    let name = |id: u32| kinds.kind_name((f.kind[id as usize] % core::KIND_STRIDE) as u16).unwrap_or("");
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
