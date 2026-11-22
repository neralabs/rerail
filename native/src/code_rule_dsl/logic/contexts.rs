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
//! The pass's roots bound its views: each node a view of its rules reads is one of its nodes (`NODES`), its roots and
//! what is within them.
//!
//! A relation name cannot hold a `.` (a clause's `.` ends it), so a read of another pass is by a name `.from`
//! declares, not by a qualified name. `.export` is phase 0's already (a file's exports), so a context's are on its
//! `.context` line.

use super::program_image::{AtomRef, Builder, Lit as L, Rewrite, RuleRef, T, TermRef, Version, fields::ProgramFields, t_of};
use super::rr::Error;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// The prefix of a context's instance of a relation (`instance`).
pub const PREFIX: &str = "rr_cx_";

/// Context `context`'s instance of relation `rel`.
pub fn instance(context: &str, rel: &str) -> String {
    format!("{PREFIX}{context}_{rel}")
}

/// A context's nodes (`instance(context, NODES)`): its roots and every node within them. Its rules' views read only
/// these (`view_nodes`).
pub const NODES: &str = "rr_nodes";

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
    /// What a made pass reads of other passes, by the names the made program gives them (`reads` on its `.pass`):
    /// relations its instance reads as they are, never made its own.
    pub reads: BTreeSet<String>,
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
        let (rels, left) = names(rest, "reads")?;
        if rels.is_empty() || rels.iter().any(|(r, l)| r != l) {
            return Err(shape(text));
        }
        let reads: BTreeSet<String> = match left.split_first() {
            None => BTreeSet::new(),
            Some((r, list)) if r == "reads" && key == "pass" => names(list, "")?.0.into_iter().map(|(r, _)| r).collect(),
            _ => return Err(shape(text)),
        };
        let c = Context { name: name.clone(), host: host.clone(), roots: rel.clone(), exports: rels.into_iter().map(|(r, _)| r).collect(), made: key == "pass", reads };
        // A context is declared where its exports are written (each library file adds its own, 3.1): a declaration
        // of a context declared already, of the same host and roots, adds its exports and reads.
        match d.contexts.get_mut(name) {
            None => {
                d.contexts.insert(name.clone(), c);
            }
            Some(was) if (&was.host, &was.roots, was.made) == (&c.host, &c.roots, c.made) => {
                was.exports.extend(c.exports);
                was.reads.extend(c.reads);
            }
            Some(_) => return bad(format!(".{key} {name}: declared twice, of other roots, another host or made")),
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

/// What making or completing the passes of `v` writes (`plan`).
struct Plan {
    /// For each pass, the relations it makes its own now, by their names as written: a pass to make, every relation
    /// its exports reach; a made one, what its instance reaches that it does not hold yet (the library's rules a part
    /// lowered in the library's scope did not hold, completed once the pack is combined).
    new: BTreeMap<String, BTreeSet<String>>,
    /// The host's relations (`passes`'s doc).
    host: BTreeSet<String>,
}

impl Plan {
    fn work(&self, d: &Declared) -> bool {
        d.any() || self.new.values().any(|s| !s.is_empty())
    }
}

/// What `v`'s passes make their own now, and what stays the host's.
fn plan(v: &Version, d: &Declared) -> Plan {
    let derived = |x: &str| v.rel(x).is_some_and(|r| v.heads(r).next().is_some());
    let mut new = BTreeMap::new();
    for c in d.contexts.values() {
        // Read as it is: another pass's (an instance, what it reads `.from`, its roots), or a view.
        let roots = d.name_in(&c.host, &c.roots);
        let stop = |x: &str| x.starts_with(PREFIX) || x == roots || if c.made { c.reads.contains(x) } else { d.read_of(&c.name, x).is_some() } || !derived(x);
        let mut set: BTreeSet<String> = BTreeSet::new();
        let mut todo: Vec<String> = Vec::new();
        if c.made {
            // Its instance as made: what its rules read.
            todo.extend(v.rules().map(|r| v.rel_name(r.head().rel())).filter(|h| h.starts_with(PREFIX) && d.pass_of(h) == c.name).map(str::to_owned));
        } else {
            set.extend(c.exports.iter().cloned());
            todo.extend(c.exports.iter().cloned());
        }
        let mut seen: BTreeSet<String> = todo.iter().cloned().collect();
        while let Some(rel) = todo.pop() {
            let Some(id) = v.rel(&rel) else { continue };
            for r in v.heads(id).filter(|&r| !reads_own_export(v, d, &c.name, r)) {
                for a in r.reads() {
                    let x = v.rel_name(a.rel());
                    if !stop(x) && set.insert(x.to_owned()) && seen.insert(x.to_owned()) {
                        todo.push(x.to_owned());
                    }
                }
            }
        }
        new.insert(c.name.clone(), set);
    }
    // The host's relations: those it derives that no pass makes its own now and that are no pass's instance, and
    // each a rule of its own reads (the library's `unparen`, read by a context and the host alike), but for what it
    // reads `.from` a context under that name.
    let in_context: BTreeSet<&String> = new.values().flatten().collect();
    let aliased = |x: &str| d.read_of(&d.language, x).is_some();
    let mut host: BTreeSet<String> = v
        .rules()
        .map(|r| v.rel_name(r.head().rel()))
        .filter(|h| !in_context.contains(&h.to_string()) && !aliased(h) && !h.starts_with(PREFIX))
        .map(str::to_owned)
        .collect();
    let mut todo: Vec<String> = host.iter().cloned().collect();
    while let Some(rel) = todo.pop() {
        let Some(id) = v.rel(&rel) else { continue };
        for r in v.heads(id) {
            for a in r.reads() {
                let x = v.rel_name(a.rel());
                if in_context.contains(&x.to_string()) && !aliased(x) && host.insert(x.to_owned()) {
                    todo.push(x.to_owned());
                }
            }
        }
    }
    Plan { new, host }
}

/// A rule of pass `pass` written with each relation as that pass names it.
struct Renaming<'a> {
    v: &'a Version,
    d: &'a Declared,
    pass: &'a str,
    /// The relations the pass makes its own now, where it is a context (`Plan::new`).
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

/// The node variables of rule `r` of pass `pass` that a view reads (a positive atom of a relation no rule derives,
/// not another pass's, in a column of type `node`): each joined with the pass's nodes, so a context's views give its
/// roots' nodes alone (0597 rule 2).
/// The views of a guest's host (0431): `host_node(n, h)` and `guest_of(n, s)` give a guest node's host's node, which is
/// no node of the guest's context.
const HOSTS_NODE: &[(&str, usize)] = &[("host_node", 1), ("guest_of", 1)];

fn view_nodes<'v>(v: &'v Version, d: &Declared, pass: &str, r: RuleRef<'v>) -> Vec<T<'v>> {
    let derived = |x: &str| v.rel(x).is_some_and(|id| v.heads(id).next().is_some());
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for l in r.body() {
        let L::Atom(a) = l.view() else { continue };
        let rel = v.rel_name(a.rel());
        if a.neg() || derived(rel) || rel.starts_with(PREFIX) || d.read_of(pass, rel).is_some() {
            continue;
        }
        let Some(cols) = v.columns_of(rel) else { continue };
        for (i, (t, (_, ty))) in a.args().zip(cols.iter()).enumerate() {
            if let TermRef::Var(slot) = t
                && ty == "node"
                && !HOSTS_NODE.contains(&(rel, i))
                && seen.insert(r.var(slot).0)
            {
                out.push(t_of(r, t));
            }
        }
    }
    out
}

/// Whether rule `r` reads what context `c` exports, by another pass's name for it: a host's rule counting the
/// context's rows in a relation of its own (rule 5, `name_node(n) :- guest_names(n)`), so no rule of the context's
/// instance of that relation.
fn reads_own_export(v: &Version, d: &Declared, c: &str, r: RuleRef<'_>) -> bool {
    r.reads().any(|a| {
        let x = v.rel_name(a.rel());
        d.reads.iter().any(|read| read.source == c && read.reader != c && read.local == x)
    })
}

/// Context `c`'s nodes (`NODES`): its roots, and what is within them where the program reads trees (`within`).
fn write_nodes(v: &Version, d: &Declared, c: &Context, b: &mut Builder<'_>) -> Result<(), Error> {
    let nodes = instance(&c.name, NODES);
    let roots = d.name_in(&c.host, &c.roots);
    let arity = v.columns_of(&roots).or_else(|| v.columns_of(&c.roots)).map_or(1, |cols| cols.len());
    let root = |at: &'static str| -> Vec<T<'static>> { std::iter::once(T::Var(at)).chain((1..arity).map(|_| T::Wild)).collect() };
    b.fields().declare(&nodes, &[("n", "node")]);
    b.rule().head(nodes.as_str(), &[T::Var("n")]).atom(roots.as_str(), &root("n")).finish().map_err(Error::Syntax)?;
    if v.columns_of("within").is_some() {
        b.rule().head(nodes.as_str(), &[T::Var("n")]).atom(roots.as_str(), &root("r")).atom("within", &[T::Var("n"), T::Var("r")]).finish().map_err(Error::Syntax)?;
    }
    Ok(())
}

/// The passes of `v` made or completed into `b`, a derivation of `v` (`plan`): the host's rules with its `.from`
/// names read from their passes; each pass's instance, made, or completed with what it reaches; the relations only a
/// pass holds taken from the host; and each pass a `.pass` setting, none to make again.
fn write(v: &Version, d: &Declared, plan: &Plan, b: &mut Builder<'_>) -> Result<(), Error> {
    let finish = |r: Result<super::program_image::RuleId, String>| r.map(|_| ()).map_err(Error::Syntax);
    for r in v.rules() {
        let head = v.rel_name(r.head().rel());
        if plan.host.contains(head) {
            finish(b.rewrite(v, r, &mut Renaming { v, d, pass: &d.language, own: None }).finish())?;
        } else if head.starts_with(PREFIX) {
            // A made pass's instance, completed: its reads of what it makes its own now read that.
            let pass = d.pass_of(head);
            finish(b.rewrite(v, r, &mut Renaming { v, d, pass, own: plan.new.get(pass) }).finish())?;
        }
    }
    for c in d.contexts.values().filter(|c| !c.made) {
        write_nodes(v, d, c, b)?;
    }
    for (c, own) in &plan.new {
        let nodes = instance(c, NODES);
        for rel in own {
            let Some(id) = v.rel(rel) else { continue };
            for r in v.heads(id).filter(|&r| !reads_own_export(v, d, c, r)) {
                let guards = view_nodes(v, d, c, r);
                let mut rb = b.rewrite(v, r, &mut Renaming { v, d, pass: c, own: Some(own) });
                for t in &guards {
                    rb = rb.atom(nodes.as_str(), std::slice::from_ref(t));
                }
                finish(rb.finish())?;
            }
            if let Some(cols) = v.columns_of(rel) {
                let pairs: Vec<(&str, &str)> = cols.iter().map(|(n, t)| (n.as_str(), t.as_str())).collect();
                b.fields().declare(&instance(c, rel), &pairs);
            }
        }
    }
    for rel in plan.new.values().flatten().filter(|r| !plan.host.contains(*r)) {
        if v.declares(rel) {
            b.fields().undeclare(rel);
        }
    }
    let mut settings: Vec<(String, String)> = ProgramFields::settings(v).into_iter().filter(|(k, _)| k != "context" && k != "from").collect();
    for c in d.contexts.values().filter(|c| !c.made) {
        let exports: Vec<&str> = c.exports.iter().map(String::as_str).collect();
        let reads: Vec<String> = d.reads.iter().filter(|r| r.reader == c.name).map(|r| d.name_in(&r.source, &r.rel)).collect();
        let reads = if reads.is_empty() { String::new() } else { format!(" reads {}", reads.join(", ")) };
        settings.push(("pass".to_owned(), format!("{} of {} roots {} exports {}{reads}", c.name, c.host, d.name_in(&c.host, &c.roots), exports.join(", "))));
    }
    b.fields().set_field(super::program_facts::Field::Settings(&settings));
    Ok(())
}

/// Program `v` with its passes made (`declared`), or completed: each context's instance of the rules it holds, each
/// reader's `.from` names reading the source pass, and the relations only a context reaches taken from the host. A
/// program with nothing to make or complete is itself. A cycle through two passes is refused, naming its relations.
pub fn passes(v: &Arc<Version>) -> Result<Arc<Version>, Error> {
    let d = declared(v)?;
    let plan = plan(v, &d);
    if !plan.work(&d) {
        return Ok(Arc::clone(v));
    }
    let mut b = Builder::derive(v);
    write(v, &d, &plan, &mut b)?;
    let out = b.finish().map_err(Error::Syntax)?.0;
    refuse_crossing_cycles(&out, &declared(&out)?)?;
    Ok(out)
}

/// The passes made in a program's lowering (`rr::lowering_tail_on`), before its declarations are checked: a `.from`
/// name is read from its pass, and a part lowered in a library's scope has its passes made of what it holds.
pub struct Passes;

impl super::program_image::manager::ProgramPass for Passes {
    const NAME: &'static str = "0597's passes";
    fn run(&mut self, cx: &super::program_image::manager::Cx<'_>, w: &mut super::program_image::manager::ProgramW<'_>) -> Result<(), String> {
        let v = cx.v();
        let d = declared(v).map_err(|e| e.to_string())?;
        let plan = plan(v, &d);
        if !plan.work(&d) {
            return Ok(());
        }
        write(v, &d, &plan, w.derive()?).map_err(|e| e.to_string())
    }
}

/// A cycle through two passes refused in a program's lowering, once its passes are made (`Passes`).
pub struct CrossingCycles;

impl super::program_image::manager::ProgramPass for CrossingCycles {
    const NAME: &'static str = "0597's passes: no cycle through two";
    fn run(&mut self, cx: &super::program_image::manager::Cx<'_>, _w: &mut super::program_image::manager::ProgramW<'_>) -> Result<(), String> {
        let v = cx.v();
        let d = declared(v).map_err(|e| e.to_string())?;
        if d.contexts.is_empty() {
            return Ok(());
        }
        refuse_crossing_cycles(v, &d).map_err(|e| e.to_string())
    }
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
