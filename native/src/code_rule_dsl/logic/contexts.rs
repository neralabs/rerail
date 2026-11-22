//! Contexts (decision 0597): code inside a file's code (a Python string annotation, a Vue directive's value) analysed
//! by rules of its own, as a pass over its roots, reading what it needs of its host's facts explicitly.
//!
//! A program declares a context and what passes read of each other with settings:
//!
//! ```text
//! .context annotation of python roots ann_root exports ann_type, ann_uses
//!                                                  the context, its host, the host's relation of its roots, and
//!                                                  what other passes may read of it
//! .from python binding, scope_node for annotation  what it reads of its host: `binding` in its rules is the host's
//! .from annotation ann_uses                         what the host (the program's language) reads of it
//! .from annotation ann_type as quoted_type          read under another name
//! ```
//!
//! A context's relations are those its exports reach through the rules, but for what it reads `.from` another pass
//! and the views (relations no rule derives): each is the context's own instance of the rules, `rr_cx_<context>_<r>`,
//! the library's rules included as they are. A reader's `.from` names read the source pass's relation. The host keeps
//! the relations it reaches itself; one only a context reaches is the context's alone. A cycle through two passes is
//! refused, naming its relations: the passes run in an order (0597 rule 4).
//!
//! The pass's roots bound its views (step 2 of 0597's workplan); here the views are shared.
//!
//! A relation name cannot hold a `.` (a clause's `.` ends it), so a read of another pass is by a name `.from`
//! declares, not by a qualified name. `.export` is phase 0's already (a file's exports), so a context's are on its
//! `.context` line.

use super::program_image::{AtomRef, Builder, Rewrite, Version, fields::ProgramFields};
use super::rr::Error;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// The prefix of a context's instance of a relation (`instance`).
pub const PREFIX: &str = "rr_cx_";

/// Context `context`'s instance of relation `rel`.
pub fn instance(context: &str, rel: &str) -> String {
    format!("{PREFIX}{context}_{rel}")
}

/// A context a program declares (`.context`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Context {
    pub name: String,
    /// The pass whose rules derive its roots: the program's language, or another context.
    pub host: String,
    /// The host's relation whose first column is each root's node.
    pub roots: String,
    /// What other passes may read of it (`exports`).
    pub exports: BTreeSet<String>,
    /// Made already (a `.pass`): its instance is in the program, and a read of it is resolved to that.
    pub made: bool,
}

/// A read of one pass by another (`.from`): `local` in `reader`'s rules is `rel` of `source`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Read {
    pub reader: String,
    pub source: String,
    pub rel: String,
    pub local: String,
}

/// What a program declares of its passes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Declared {
    /// The program's own pass: its language.
    pub language: String,
    pub contexts: BTreeMap<String, Context>,
    pub reads: Vec<Read>,
}

impl Declared {
    /// Whether the program declares a context to make or a read to resolve.
    pub fn any(&self) -> bool {
        self.contexts.values().any(|c| !c.made) || !self.reads.is_empty()
    }

    /// The relation `reader` reads under `local`, where it is another pass's (`.from`).
    fn read_of(&self, reader: &str, local: &str) -> Option<&Read> {
        self.reads.iter().find(|r| r.reader == reader && r.local == local)
    }

    /// `rel` of pass `pass` as the program names it: the language's by its own name, a context's by its instance.
    fn name_in(&self, pass: &str, rel: &str) -> String {
        if pass == self.language { rel.to_owned() } else { instance(pass, rel) }
    }

    /// The pass a relation of the rewritten program is of.
    pub fn pass_of<'a>(&'a self, rel: &str) -> &'a str {
        rel.strip_prefix(PREFIX)
            .and_then(|rest| self.contexts.keys().filter(|c| rest.strip_prefix(c.as_str()).is_some_and(|t| t.starts_with('_'))).max_by_key(|c| c.len()))
            .map_or(self.language.as_str(), |c| c.as_str())
    }
}

fn bad<T>(message: String) -> Result<T, Error> {
    Err(Error::Syntax(message))
}

/// A setting's words, a comma one of them.
fn words(text: &str) -> Vec<String> {
    text.replace(',', " , ").split_whitespace().map(str::to_owned).collect()
}

/// A list of names separated by commas, each possibly `name as local`, up to `stop` or the end: the names, and the
/// words left.
fn names(words: &[String], stop: &str) -> Result<(Vec<(String, String)>, Vec<String>), Error> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < words.len() && words[i] != stop {
        let name = words[i].clone();
        let mut local = name.clone();
        i += 1;
        if words.get(i).is_some_and(|w| w == "as") {
            local = words.get(i + 1).cloned().ok_or_else(|| Error::Syntax(format!("`{name} as` names no relation")))?;
            i += 2;
        }
        out.push((name, local));
        match words.get(i).map(String::as_str) {
            Some(",") => i += 1,
            Some(w) if w == stop => {}
            None => {}
            Some(w) => return bad(format!("`{w}` where a `,` or `{stop}` was expected")),
        }
    }
    Ok((out, words[i..].to_vec()))
}

/// The passes program `v` declares (`.context`, `.from`), checked: every pass named is declared, a context's host and
/// roots exist, a read of a context is of what it exports, a reader names each relation once.
pub fn declared(v: &Version) -> Result<Declared, Error> {
    let settings = ProgramFields::settings(v);
    let language = settings.iter().find(|(k, _)| k == "language").map(|(_, t)| t.trim().trim_matches('"').to_owned()).unwrap_or_default();
    let mut d = Declared { language, ..Declared::default() };
    let shape = |text: &str| Error::Syntax(format!(".context {}: expected `NAME of HOST roots RELATION exports RELATION, …`", text.trim()));
    for (key, text) in settings.iter().filter(|(k, _)| k == "context" || k == "pass") {
        let w = words(text);
        let [name, of, host, roots, rel, exports, rest @ ..] = w.as_slice() else { return Err(shape(text)) };
        if of != "of" || roots != "roots" || exports != "exports" {
            return Err(shape(text));
        }
        let (rels, left) = names(rest, "")?;
        if !left.is_empty() || rels.is_empty() || rels.iter().any(|(r, l)| r != l) {
            return Err(shape(text));
        }
        let c = Context { name: name.clone(), host: host.clone(), roots: rel.clone(), exports: rels.into_iter().map(|(r, _)| r).collect(), made: key == "pass" };
        if d.contexts.insert(name.clone(), c).is_some() {
            return bad(format!(".{key} {name}: declared twice"));
        }
    }
    for (_, text) in settings.iter().filter(|(k, _)| k == "from") {
        let w = words(text);
        let Some((source, rest)) = w.split_first() else { return bad(".from names no pass".to_owned()) };
        let (rels, left) = names(rest, "for")?;
        let reader = match left.as_slice() {
            [] => d.language.clone(),
            [f, reader] if f == "for" => reader.clone(),
            _ => return bad(format!(".from {}: expected `PASS RELATION [as NAME], … [for PASS]`", text.trim())),
        };
        for (rel, local) in rels {
            d.reads.push(Read { reader: reader.clone(), source: source.clone(), rel, local });
        }
    }
    let pass = |p: &str| p == d.language || d.contexts.contains_key(p);
    for c in d.contexts.values() {
        if !pass(&c.host) {
            return bad(format!(".context {}: its host {} is no pass of the program", c.name, c.host));
        }
        if c.name == d.language {
            return bad(format!(".context {}: the program's language is its own pass", c.name));
        }
        match v.columns_of(&c.roots).or_else(|| c.made.then(|| v.columns_of(&instance(&c.host, &c.roots))).flatten()) {
            Some(cols) if cols.first().is_some_and(|(_, t)| t == "node") => {}
            Some(_) => return bad(format!(".context {}: its roots {} have no node as their first column", c.name, c.roots)),
            None => return bad(format!(".context {}: its roots {} are not declared", c.name, c.roots)),
        }
        for e in &c.exports {
            let derived = if c.made { instance(&c.name, e) } else { e.clone() };
            if v.rel(&derived).is_none_or(|r| v.heads(r).next().is_none()) {
                return bad(format!(".context {} exports {e}: no rule derives it", c.name));
            }
        }
    }
    let mut seen: BTreeMap<(String, String), (String, String)> = BTreeMap::new();
    // The same read twice (two policies of a pack) is one.
    let mut kept = Vec::new();
    for r in std::mem::take(&mut d.reads) {
        if !kept.contains(&r) {
            kept.push(r);
        }
    }
    d.reads = kept;
    for r in &d.reads {
        for p in [&r.reader, &r.source] {
            if !pass(p) {
                return bad(format!(".from {} {}: {p} is no pass of the program", r.source, r.rel));
            }
        }
        if r.reader == r.source {
            return bad(format!(".from {} {}: a pass reads its own relations as they are", r.source, r.rel));
        }
        if let Some(c) = d.contexts.get(&r.source) {
            if !c.exports.contains(&r.rel) {
                return bad(format!(".from {} {}: {} does not export {}", r.source, r.rel, r.source, r.rel));
            }
        } else if v.rel(&r.rel).is_none() && !v.declares(&r.rel) {
            return bad(format!(".from {} {}: no such relation", r.source, r.rel));
        }
        if d.contexts.get(&r.reader).is_some_and(|c| c.made) {
            return bad(format!(".from {} {} for {}: {} is made already, its reads with it", r.source, r.rel, r.reader, r.reader));
        }
        if seen.insert((r.reader.clone(), r.local.clone()), (r.source.clone(), r.rel.clone())).is_some() {
            return bad(format!(".from {}: {} names two relations {}", r.source, r.reader, r.local));
        }
    }
    Ok(d)
}

/// The rules of `v` each pass holds (`declared`'s module doc): for each context, the relations it holds, by their
/// names in the rules as written.
fn held(v: &Version, d: &Declared) -> BTreeMap<String, BTreeSet<String>> {
    let derived = |x: &str| v.rel(x).is_some_and(|r| v.heads(r).next().is_some());
    d.contexts
        .values()
        .filter(|c| !c.made)
        .map(|c| {
            let mut set: BTreeSet<String> = c.exports.clone();
            let mut todo: Vec<String> = set.iter().cloned().collect();
            while let Some(rel) = todo.pop() {
                let Some(id) = v.rel(&rel) else { continue };
                for r in v.heads(id) {
                    for a in r.reads() {
                        let x = v.rel_name(a.rel());
                        if d.read_of(&c.name, x).is_none() && derived(x) && set.insert(x.to_owned()) {
                            todo.push(x.to_owned());
                        }
                    }
                }
            }
            (c.name.clone(), set)
        })
        .collect()
}

/// A rule of pass `pass` written with each relation as that pass names it.
struct Renaming<'a> {
    v: &'a Version,
    d: &'a Declared,
    pass: &'a str,
    /// The relations the pass holds, where it is a context (`held`).
    own: Option<&'a BTreeSet<String>>,
}

impl Renaming<'_> {
    fn rename(&self, rel: &str) -> Option<String> {
        if let Some(r) = self.d.read_of(self.pass, rel) {
            return Some(self.d.name_in(&r.source, &r.rel));
        }
        self.own.filter(|own| own.contains(rel)).map(|_| instance(self.pass, rel))
    }
}

impl Rewrite for Renaming<'_> {
    fn head_as(&mut self, a: AtomRef<'_>) -> Option<String> {
        self.rename(self.v.rel_name(a.rel()))
    }
    fn atom_as(&mut self, a: AtomRef<'_>) -> Option<(String, bool, bool)> {
        self.rename(self.v.rel_name(a.rel())).map(|r| (r, a.neg(), a.gated()))
    }
}

/// Program `v` with its passes made (`declared`): each context's instance of the rules it holds, each reader's
/// `.from` names reading the source pass, and the relations only a context reaches taken from the host. A program
/// that declares no pass is itself. A cycle through two passes is refused, naming its relations.
pub fn passes(v: &Arc<Version>) -> Result<Arc<Version>, Error> {
    let d = declared(v)?;
    if !d.any() {
        return Ok(Arc::clone(v));
    }
    let held = held(v, &d);
    // The host's relations: those it derives that no context holds, and each a rule of its own reads (a context's
    // relation the host reads by its own name, the library's `unparen`, is the host's too), but for what it reads
    // `.from` a context under that name.
    let in_context: BTreeSet<String> = held.values().flatten().cloned().collect();
    let aliased = |x: &str| d.read_of(&d.language, x).is_some();
    let mut host: BTreeSet<String> = v
        .rules()
        .map(|r| v.rel_name(r.head().rel()))
        .filter(|h| !in_context.contains(*h) && !aliased(h))
        .map(str::to_owned)
        .collect();
    let mut todo: Vec<String> = host.iter().cloned().collect();
    while let Some(rel) = todo.pop() {
        let Some(id) = v.rel(&rel) else { continue };
        for r in v.heads(id) {
            for a in r.reads() {
                let x = v.rel_name(a.rel());
                if in_context.contains(x) && !aliased(x) && host.insert(x.to_owned()) {
                    todo.push(x.to_owned());
                }
            }
        }
    }
    let mut b = Builder::derive(v);
    for r in v.rules() {
        let head = v.rel_name(r.head().rel());
        if host.contains(head) {
            b.rewrite(v, r, &mut Renaming { v, d: &d, pass: &d.language, own: None }).finish().map_err(Error::Syntax)?;
        }
    }
    for (c, own) in &held {
        for rel in own {
            let Some(id) = v.rel(rel) else { continue };
            for r in v.heads(id) {
                b.rewrite(v, r, &mut Renaming { v, d: &d, pass: c, own: Some(own) }).finish().map_err(Error::Syntax)?;
            }
            if let Some(cols) = v.columns_of(rel) {
                let pairs: Vec<(&str, &str)> = cols.iter().map(|(n, t)| (n.as_str(), t.as_str())).collect();
                b.fields().declare(&instance(c, rel), &pairs);
            }
        }
    }
    // A relation only contexts hold is not the host's.
    for rel in in_context.iter().filter(|r| !host.contains(*r)) {
        if v.declares(rel) {
            b.fields().undeclare(rel);
        }
    }
    // The declarations made: each context as a `.pass` (its roots as its host names them), none to make again.
    let mut settings: Vec<(String, String)> = ProgramFields::settings(&**v).into_iter().filter(|(k, _)| k != "context" && k != "from").collect();
    for c in d.contexts.values().filter(|c| !c.made) {
        let exports: Vec<&str> = c.exports.iter().map(String::as_str).collect();
        settings.push(("pass".to_owned(), format!("{} of {} roots {} exports {}", c.name, c.host, d.name_in(&c.host, &c.roots), exports.join(", "))));
    }
    b.fields().set_field(super::program_facts::Field::Settings(&settings));
    let out = b.finish().map_err(Error::Syntax)?.0;
    refuse_crossing_cycles(&out, &d)?;
    Ok(out)
}

/// A pass made (`passes`): a context, its host and its roots, as the made program's `.pass` settings hold them
/// (`.pass annotation of python roots ann_root exports ann_type, …`: the roots as the made program names them).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Made {
    pub name: String,
    pub host: String,
    /// The host's relation of its roots, as the made program names it.
    pub roots: String,
}

/// The passes program `v` was made into (`passes`), from its `.pass` settings.
pub fn made(v: &Version) -> Vec<Made> {
    ProgramFields::settings(v)
        .into_iter()
        .filter(|(k, _)| k == "pass")
        .filter_map(|(_, text)| match words(&text).as_slice() {
            [name, of, host, roots, rel, exports, ..] if of == "of" && roots == "roots" && exports == "exports" => Some(Made { name: name.clone(), host: host.clone(), roots: rel.clone() }),
            _ => None,
        })
        .collect()
}

/// A cycle through relations of two passes is refused, naming the passes and its relations.
fn refuse_crossing_cycles(v: &Version, d: &Declared) -> Result<(), Error> {
    let mut edges: super::front::Edges = BTreeMap::new();
    let mut nodes = BTreeSet::new();
    for r in v.rules() {
        let h = v.rel_name(r.head().rel()).to_owned();
        nodes.insert(h.clone());
        for a in r.reads() {
            let x = v.rel_name(a.rel()).to_owned();
            nodes.insert(x.clone());
            edges.entry(h.clone()).or_default().insert((x, a.neg()));
        }
    }
    for c in super::front::tarjan(&nodes, &edges) {
        let passes: BTreeSet<&str> = c.iter().map(|r| d.pass_of(r)).collect();
        if passes.len() > 1 {
            let rels: Vec<&str> = c.iter().map(String::as_str).take(12).collect();
            return bad(format!(
                "a cycle through the passes {}: {}{} (0597: a pass reads another's finished facts)",
                passes.into_iter().collect::<Vec<_>>().join(" and "),
                rels.join(", "),
                if c.len() > 12 { ", …" } else { "" }
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "contexts_tests.rs"]
mod tests;
