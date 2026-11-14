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
use crate::content_store::ContentStore;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
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
/// The relations a rule reads guests and values through (`rr::lower`'s guest relations): a program none of whose
/// needed rules reads one makes no guest.
pub(super) const READS: [&str; 10] = ["parsed", "sub_tape", "guest", "doc_root", "doc_role", "tape_of", "host_node", "parse_of", "unparse_of", "splice_of"];
/// Every relation a request is read from: a run with a rule deriving one makes guests.
pub(in crate::code_rule_dsl) const REQUESTS: [&str; 5] = ["embed", "embed_value", DEMAND, PARSE, SPLICE];
/// The engine's relations a combined run shares across its programs (`corpus::combine` prefixes no program's): the
/// requests, which the runner reads by name, and the guest policy.
pub(super) const SHARED: [&str; 6] = ["embed", "embed_value", DEMAND, PARSE, SPLICE, crate::code_rule_dsl::facts::vtab::syntax::GUEST_POLICY];

/// One document asked for.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(super) enum Request {
    /// The code at host node `at`, in `grammar`, with its text when it is a value (`embed_value`).
    Guest { at: i64, grammar: String, role: String, text: Option<String> },
    /// `@parse(grammar, text)`, asked at location `at` (the asking document's).
    Parse { at: i64, grammar: String, text: String },
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
    // `at`, the location that asked, says whose the value is; equal texts are one value (`Guests::value`).
    for c in rows(PARSE, 3) {
        if let [at, g, x] = c.as_slice()
            && let Some(at) = int(at)
        {
            out.push(Request::Parse { at, grammar: (*g).to_owned(), text: (*x).to_owned() });
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
    let tag = core::facts_tag(f);
    let label = crate::code_rule_dsl::language::from_tape_tag(tag).and_then(crate::code_rule_dsl::language::label).unwrap_or("");
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

/// The guests' makers' state for one chunk: the run's language and shape, the hosts' numberings, the guests made, and
/// the keys taken.
#[derive(Default)]
pub(super) struct Guests {
    run: &'static str,
    /// The run's projection: its node ids are what requests name (`syntax::numbering`).
    shape: syntax::Shape,
    numbering: HashMap<u32, syntax::Numbering>,
    made: HashSet<(i64, String)>,
    asked: HashSet<Request>,
    taken: HashSet<u32>,
    /// Each guest's host file (`memo` attributes a guest's rows to it).
    pub(super) host_of: HashMap<i64, i64>,
    /// Each value made, by grammar and what made it: its key. A value is one per text in a chunk.
    values: HashMap<(String, String), i64>,
    /// Each value's files: those whose rules asked for it (O96). `memo` attributes its rows to each.
    files: HashMap<i64, BTreeSet<i64>>,
    /// Made, and given no guest (no text to parse, or no grammar), this chunk.
    pub(super) refused: usize,
}

impl Guests {
    /// For a run over `run`'s files (a dialect label) in `shape`.
    pub(super) fn new(run: &'static str, shape: syntax::Shape) -> Self {
        Self { run, shape, ..Self::default() }
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
                Request::Parse { at, grammar, text } => {
                    let asker = super::memo::document_of(at);
                    if self.known(&grammar, &format!("parse:{text}"), asker) {
                        continue;
                    }
                    self.value(&by_key, asker, &grammar, text.clone(), Made::Parse { text }, cache)?
                }
                Request::Splice { node, with } => {
                    let asker = super::memo::document_of(node);
                    if self.known("", &format!("splice:{node}:{with}"), asker) {
                        continue;
                    }
                    match self.splice(&by_key, docs, node, &with)? {
                        Some((grammar, text, tape, _)) => self.value(&by_key, asker, &grammar, text, Made::Splice { tape, node, with }, cache)?,
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
        let ids = self.numbering(host)?;
        // A node id's low bits are its local id only under preorder ids (`syntax::Numbering`).
        let Some(local) = ids.local((at & 0xffff_ffff) as u32) else { return Ok(None) };
        let ids = &ids.nodes;
        let placed = match text {
            Some(text) => {
                let Some(&(id, _)) = ids.get(local as usize) else { return Ok(None) };
                Some((text, Placement::At([host.facts.start[id as usize], host.facts.end[id as usize]])))
            }
            None => host_text(host, ids, local),
        };
        let Some((text, place)) = placed else { return Ok(None) };
        let (text, place) = if wrap { wrapped(text, place) } else { (text, place) };
        let Some((facts, root, digest)) = parse(&host.path, label, wrap, text, cache)? else { return Ok(None) };
        let key = key(&[host.path.as_bytes(), &at.to_le_bytes(), grammar_name.as_bytes()], label == self.run, &mut self.taken);
        self.host_of.insert(i64::from(key), self.host_of.get(&i64::from(host_key)).copied().unwrap_or(i64::from(host_key)));
        let in_file = host.guest.as_ref().map_or_else(|| place.clone(), |h| h.in_file.then(place.clone()));
        Ok(Some(Document {
            key,
            path: host.path.clone(),
            core: format!("{}#guest:{grammar_name}:{at}", host.core),
            digest,
            facts,
            guest: Some(Arc::new(Guest { host: host_key, at, grammar: grammar_name.to_owned(), role: role.to_owned(), root, place, in_file, made: Made::Host })),
            module: host.module.clone(),
            tape: None,
        }))
    }

    /// A value document (decision 0058 §4): `text` parsed as `grammar_name`, with no host, in the hidden documents,
    /// asked for by document `asker`. Its places are its own text's. It is its askers' files' (O96): `memo` keeps its
    /// rows with each, and its path is the first's.
    fn value(&mut self, by_key: &HashMap<u32, &Document>, asker: i64, grammar_name: &str, text: String, made: Made, cache: Option<&ContentStore>) -> Result<Option<Document>, String> {
        let Some((label, wrap)) = grammar(grammar_name) else { return Ok(None) };
        let (identity, known_as) = match &made {
            Made::Splice { node, with, .. } => (format!("splice:{node}:{with}"), String::new()),
            _ => (format!("parse:{text}"), grammar_name.to_owned()),
        };
        let files = self.files_of(asker);
        let path = files.first().and_then(|f| u32::try_from(*f).ok()).and_then(|f| by_key.get(&f)).map_or_else(String::new, |d| d.path.clone());
        let path = path.as_str();
        let (text, _) = if wrap { wrapped(text, Placement::At([0, 0])) } else { (text, Placement::At([0, 0])) };
        let Some((facts, root, digest)) = parse(path, label, wrap, text, cache)? else { return Ok(None) };
        let key = key(&[b"value", grammar_name.as_bytes(), identity.as_bytes()], false, &mut self.taken);
        self.values.insert((known_as, identity), i64::from(key));
        self.files.insert(i64::from(key), files);
        Ok(Some(Document {
            key,
            path: path.to_owned(),
            core: format!("#value:{grammar_name}:{key}"),
            digest,
            facts,
            guest: Some(Arc::new(Guest { host: 0, at: 0, grammar: grammar_name.to_owned(), role: "value".to_owned(), root, place: Placement::At([0, 0]), in_file: Placement::At([0, 0]), made })),
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

/// `text` parsed as `label` through the tape cache, its root (a wrapped expression's) and its facts' digest; `None`
/// when the parser gives no facts. Kept for the process by grammar and text: a pack's guests repeat their texts (one
/// annotation string in many files, the same guests in every chunk and round), and the facts are a function of the
/// two alone, so each distinct text is looked up in the store, restored and digested once.
fn parse(path: &str, label: &str, wrap: bool, text: String, cache: Option<&ContentStore>) -> Result<Option<Parsed>, String> {
    use std::sync::{Mutex, OnceLock};
    /// The texts kept, in bytes, before the memo starts again (the facts kept are a small multiple of it).
    const KEPT_BYTES: usize = 8 << 20;
    static KEPT: OnceLock<Mutex<(usize, HashMap<(String, bool, String), Option<Parsed>>)>> = OnceLock::new();
    let kept = KEPT.get_or_init(Default::default);
    let key = (label.to_owned(), wrap, text);
    if let Some(hit) = kept.lock().expect("the parsed guests").1.get(&key) {
        crate::code_rule_dsl::prof::count("guests.parse_kept", 1);
        return Ok(hit.clone());
    }
    let file = crate::code_rule_dsl::facts::core::Source { language: label.to_owned(), source: Arc::new(key.2.clone()) };
    // A guest is made in a wave, so no SQL is read or written for it (decision 0104): its tape is one this process
    // queued, else it is parsed and its source and tape are the store's writer's, behind the run.
    let built = match cache.map(|cache| core::build_in_wave(&file, cache, None, None, None)) {
        Some(Ok(built)) => Ok(built),
        Some(Err(_)) => {
            crate::code_rule_dsl::prof::count("guests.cache_failed", 1);
            core::build(&file)
        }
        None => core::build(&file),
    }
    .map_err(|e| format!("{path}: {e}"))?;
    let parsed = match built {
        Built::Facts(facts) => {
            let root = if wrap { wrapped_root(&facts, label) } else { 0 };
            let digest = core::digest(&facts);
            Some((Arc::new(facts), root, digest))
        }
        _ => None,
    };
    let mut kept = kept.lock().expect("the parsed guests");
    if kept.0 + key.2.len() > KEPT_BYTES {
        *kept = Default::default();
    }
    kept.0 += key.2.len();
    kept.1.insert(key, parsed.clone());
    Ok(parsed)
}

/// The root of a wrapped expression `( e )`: `e`, the one named child of the parenthesized expression the module's
/// one statement is, when the text parses so; else the module's first named child (a partial tree's).
fn wrapped_root(f: &core::Facts, label: &str) -> u32 {
    let Some(dialect) = crate::code_rule_dsl::language::dialect_by_label(label) else { return 0 };
    let kinds = dialect.kinds();
    let name = |id: u32| kinds.kind_name(core::kind_code(f.kind[id as usize])).unwrap_or("");
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
