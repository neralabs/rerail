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
use crate::code_rule_dsl::facts::vtab::syntax::{self, Document, Guest, Placement};
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

/// One guest asked for: the code at host node `at`, in `grammar`, with its text when it is a value.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(super) struct Request {
    pub(super) at: i64,
    pub(super) grammar: String,
    pub(super) role: String,
    pub(super) text: Option<String>,
}

/// The requests of a slice run's outputs, in order; the library's only when `library` (`.guests on`).
pub(super) fn requests(outputs: &BTreeMap<String, Vec<String>>, library: bool) -> Vec<Request> {
    let mut out = Vec::new();
    let int = |s: &str| s.parse::<i64>().ok();
    if library {
        for line in outputs.get("embed").into_iter().flatten() {
            let c: Vec<&str> = line.splitn(4, '\t').collect();
            if let [_, at, g, r] = c.as_slice()
                && let Some(at) = int(at)
            {
                out.push(Request { at, grammar: (*g).to_owned(), role: (*r).to_owned(), text: None });
            }
        }
        for line in outputs.get("embed_value").into_iter().flatten() {
            let c: Vec<&str> = line.splitn(5, '\t').collect();
            if let [_, at, g, r, t] = c.as_slice()
                && let Some(at) = int(at)
            {
                out.push(Request { at, grammar: (*g).to_owned(), role: (*r).to_owned(), text: Some((*t).to_owned()) });
            }
        }
    }
    for line in outputs.get(DEMAND).into_iter().flatten() {
        let c: Vec<&str> = line.splitn(2, '\t').collect();
        if let [at, g] = c.as_slice()
            && let Some(at) = int(at)
        {
            out.push(Request { at, grammar: (*g).to_owned(), role: "fragment".to_owned(), text: None });
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

/// A guest's key: in `2^30..2^31`, past every file's (`memo::doc_keys` gives files keys below 2^30) and with
/// `key << 32` positive, a function of its host's path, place and grammar, so a memoised row's guest nodes keep
/// their ids.
fn key(host: &str, at: i64, grammar: &str, taken: &mut HashSet<u32>) -> u32 {
    const SPAN: u32 = 1 << 30;
    let mut h = Sha256::new();
    h.update(host.as_bytes());
    h.update(at.to_le_bytes());
    h.update(grammar.as_bytes());
    let hash: [u8; 32] = h.finalize().into();
    let mut k = u32::from_le_bytes(hash[..4].try_into().expect("4 bytes")) % SPAN;
    while !taken.insert(SPAN | k) {
        k = (k + 1) % SPAN;
    }
    SPAN | k
}

/// A Python-style string literal's parts: prefix letters, quote, and the contents' byte range in `text`.
fn string_parts(text: &str) -> Option<(String, usize, usize)> {
    let q = text.find(['\'', '"'])?;
    let prefix = text[..q].to_ascii_lowercase();
    let rest = &text[q..];
    let quote = ["\"\"\"", "'''", "\"", "'"].into_iter().find(|quote| rest.starts_with(quote) && rest.len() >= 2 * quote.len() && rest.ends_with(quote))?;
    Some((prefix, q + quote.len(), text.len() - quote.len()))
}

/// The text of host node `local` to parse, and where it stands: a string's contents (copied when plain, decoded
/// otherwise), any other node's text.
fn host_text(host: &Document, ids: &[(u32, &'static str)], local: u32) -> Option<(String, Placement)> {
    let f = &*host.facts;
    let &(id, kind) = ids.get(local as usize)?;
    let (start, end) = (f.start[id as usize], f.end[id as usize]);
    let node = [start, end];
    match kind {
        "string" => {
            let text = f.text(id);
            let (prefix, a, b) = string_parts(text)?;
            if prefix.contains(['b', 'f', 't']) {
                return None;
            }
            let content = &text[a..b];
            if !prefix.contains('r') && !content.contains('\\') {
                return Some((content.to_owned(), Placement::Copied { delta: i64::from(start) + a as i64, within: node }));
            }
            Some((crate::code_rule_dsl::value::python::str_value(text)?, Placement::At(node)))
        }
        "concatenated_string" => {
            let mut value = String::new();
            for &c in f.children(id) {
                if ids.iter().any(|&(i, k)| i == c && k == "string") {
                    let text = f.text(c);
                    let (prefix, _, _) = string_parts(text)?;
                    if prefix.contains(['b', 'f', 't']) {
                        return None;
                    }
                    value.push_str(&crate::code_rule_dsl::value::python::str_value(text)?);
                }
            }
            Some((value, Placement::At(node)))
        }
        _ => Some((f.text(id).to_owned(), Placement::Copied { delta: i64::from(start), within: node })),
    }
}

/// The guests' makers' state for one chunk: the hosts' numberings, the guests made, and the keys taken.
#[derive(Default)]
pub(super) struct Guests {
    numbering: HashMap<u32, Vec<(u32, &'static str)>>,
    made: HashSet<(i64, String)>,
    taken: HashSet<u32>,
    /// Each guest's host file (`memo` attributes a guest's rows to it).
    pub(super) host_of: HashMap<i64, i64>,
    /// Made, and given no guest (no text to parse, or no grammar), this chunk.
    pub(super) refused: usize,
}

impl Guests {
    /// The documents `requests` ask for that are not made yet; `docs` holds their hosts.
    pub(super) fn make(&mut self, requests: Vec<Request>, docs: &[Document], cache: Option<&ContentStore>) -> Result<Vec<Document>, String> {
        let by_key: HashMap<u32, &Document> = docs.iter().map(|d| (d.key, d)).collect();
        let mut out = Vec::new();
        for r in requests {
            if !self.made.insert((r.at, r.grammar.clone())) {
                continue;
            }
            let host_key = (r.at >> 32) as u32;
            let Some(host) = by_key.get(&host_key) else {
                self.refused += 1;
                continue;
            };
            let Some((label, wrap)) = grammar(&r.grammar) else {
                self.refused += 1;
                continue;
            };
            let local = (r.at & 0xffff_ffff) as u32;
            let placed = match &r.text {
                Some(text) => {
                    let ids = self.numbering(host)?;
                    let &(id, _) = match ids.get(local as usize) {
                        Some(x) => x,
                        None => {
                            self.refused += 1;
                            continue;
                        }
                    };
                    Some((text.clone(), Placement::At([host.facts.start[id as usize], host.facts.end[id as usize]])))
                }
                None => {
                    let ids = self.numbering(host)?;
                    host_text(host, ids, local)
                }
            };
            let Some((text, place)) = placed else {
                self.refused += 1;
                continue;
            };
            let (text, place) = if wrap {
                let place = match place {
                    Placement::Copied { delta, within } => Placement::Copied { delta: delta - 1, within },
                    at => at,
                };
                (format!("({text})"), place)
            } else {
                (text, place)
            };
            let file = FileInput::new(host.path.clone(), label.to_owned(), Arc::new(text), None, None, "guest").map_err(|e| format!("{}: {e:?}", host.path))?;
            // The cache is shared by every run on the machine; a failed write there (O85) parses without it.
            let built = match cache.map(|cache| core::build_cached(&file, cache)) {
                Some(Ok(built)) => Ok(built),
                Some(Err(_)) => {
                    crate::code_rule_dsl::prof::count("guests.cache_failed", 1);
                    core::build(&file)
                }
                None => core::build(&file),
            }
            .map_err(|e| format!("{}: {e}", host.path))?;
            let Built::Facts(facts) = built else {
                self.refused += 1;
                continue;
            };
            let root = if wrap { wrapped_root(&facts, label) } else { 0 };
            let key = key(&host.path, r.at, &r.grammar, &mut self.taken);
            self.host_of.insert(i64::from(key), self.host_of.get(&i64::from(host_key)).copied().unwrap_or(i64::from(host_key)));
            let digest = core::digest(&facts);
            let in_file = host.guest.as_ref().map_or(place, |h| h.in_file.then(place));
            out.push(Document {
                key,
                path: host.path.clone(),
                core: format!("{}#guest:{}:{}", host.core, r.grammar, r.at),
                digest,
                facts: Arc::new(facts),
                guest: Some(Arc::new(Guest { host: host_key, at: r.at, grammar: r.grammar.clone(), role: r.role.clone(), root, place, in_file })),
            });
        }
        Ok(out)
    }

    fn numbering(&mut self, host: &Document) -> Result<&Vec<(u32, &'static str)>, String> {
        if !self.numbering.contains_key(&host.key) {
            let ids = syntax::numbering(host).map_err(|e| e.to_string())?;
            self.numbering.insert(host.key, ids);
        }
        Ok(&self.numbering[&host.key])
    }
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
