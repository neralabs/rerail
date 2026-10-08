//! The workspace call graph: summaries linked by name, and per `calls` term the definitions that
//! reach its seed. A term is a sparse reachability question: a backward walk over callers from its
//! seed definitions, whose cost is the region it reaches, not the graph. The dense closure over
//! strongly connected components (`close`, 64 terms a word) remains as the tests' oracle.

use super::names::Name;
use super::php;
use super::summary::{BASE, CLASS, DECORATED, METACLASS, OPEN, SHADOWED, Summary, UNKNOWN_BASE};
use crate::code_rule_dsl::language::Family;
use rayon::prelude::*;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;

/// Terms of one dense closure: bit `i` is the `i`th term.
#[cfg(test)]
pub(crate) type Bits = u64;

/// Alias and star-import hops one name may take before it is open.
const MAX_HOPS: u32 = 16;

/// A chain's last definition: it matches the seed itself.
#[cfg(test)]
const SEED_HOP: u32 = u32::MAX - 1;
/// No chain: the definition does not reach the term's seed.
#[cfg(test)]
const NO_HOP: u32 = u32::MAX;

/// How a definition reaches a term's seed: it is a seed, or calls the definition it reaches
/// through (the next on a shortest chain).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Hop {
    Seed,
    Via(DefRef),
}

/// The definitions reaching a term's seeds, each with its hop.
pub(crate) type Region = HashMap<DefRef, Hop>;

/// A definition by its file's slot and its index in the file's summary: unchanged by other files'
/// updates.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct DefRef {
    file: u32,
    local: u32,
}

impl DefRef {
    /// The slot of the definition's file.
    pub(crate) fn file(self) -> u32 {
        self.file
    }
}

/// What a name links to.
#[derive(Clone, Debug, Default, PartialEq)]
struct Link {
    defs: Vec<DefRef>,
    /// The name denotes a module's member no definition or alias gives.
    open: bool,
}

/// A linked name: what it links to, the names linking it followed (aliases, star imports), and
/// the files whose summaries call it.
#[derive(Clone, Debug)]
struct Memo {
    defs: Box<[DefRef]>,
    open: bool,
    followed: Box<[Name]>,
    users: Vec<u32>,
}

impl Memo {
    fn set(&mut self, link: Link, followed: Vec<Name>) {
        self.defs = link.defs.into_boxed_slice();
        self.open = link.open;
        self.followed = followed.into_boxed_slice();
    }
}

/// A name's answer for every term of a dense closure: the call reaches a seed, or may.
#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) struct Status {
    pub(crate) reach: Bits,
    /// Not known to reach: a definition on the way calls something unknown. Never set with
    /// `reach` for the same term.
    pub(crate) open: Bits,
}

/// One definition's inputs for every term of a dense closure.
#[cfg(test)]
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Seed {
    pub(crate) matched: Bits,
    pub(crate) unknown: Bits,
}

/// One linked file: its summary, its definitions' resolved call edges, and their callers.
#[derive(Clone, Debug)]
struct GraphFile {
    path: Arc<str>,
    summary: Arc<Summary>,
    /// `(definition, callee definition)`, sorted.
    edges: Vec<(u32, DefRef)>,
    /// Definitions that call something open.
    open: Vec<u32>,
    /// The definitions calling each of the file's, in number order:
    /// `callers[caller_offsets[d]..caller_offsets[d + 1]]`.
    caller_offsets: Vec<u32>,
    callers: Vec<DefRef>,
}

/// A module's entry: the modules it `import *`s, and whether its summary is open (a syntax error
/// may hide a definition).
type Module = (Arc<[Name]>, bool);

/// What one file puts in the name tables, under its module's names.
#[derive(Default)]
struct Contribution {
    defs: HashMap<Name, Vec<(u32, u8)>>,
    aliases: HashMap<Name, Vec<Name>>,
    opaque: HashMap<Name, Vec<()>>,
    modules: HashMap<Name, Vec<Module>>,
    supers: HashMap<Name, Vec<(Name, u8)>>,
    children: HashMap<Name, Vec<Name>>,
}

impl Contribution {
    fn of(path: &str, summary: &Summary) -> Self {
        let mut contribution = Self::default();
        // A file is named by its package module, and by its path where that differs.
        let module = summary.module.map(|module| summary.interned(module));
        let alternate = path_module(path)
            .filter(|alternate| Some(alternate.as_str()) != module.map(Name::text));
        let names = |name: &Name| {
            let renamed = module
                .zip(alternate.as_deref())
                .and_then(|(module, alternate)| {
                    let rest = name.text().strip_prefix(module.text())?;
                    (rest.is_empty() || rest.starts_with('.'))
                        .then(|| Name::new(&format!("{alternate}{rest}")))
                });
            std::iter::once(name.clone()).chain(renamed)
        };
        for (local, def) in summary.defs.iter().enumerate() {
            let name = summary.interned(def.name);
            match name.family() {
                // A definition a later binding replaces is no longer what its name denotes.
                Family::Python if def.flags & SHADOWED != 0 => {}
                Family::Python => {
                    for name in names(name) {
                        contribution
                            .defs
                            .entry(name)
                            .or_default()
                            .push((local as u32, def.flags));
                    }
                }
                // PHP compares function, class and method names case-insensitively: a definition
                // is linked by its lowercased name.
                Family::Php => {
                    let text = name.text();
                    let key = if text.bytes().any(|byte| byte.is_ascii_uppercase()) {
                        Name::new(&text.to_ascii_lowercase())
                    } else {
                        name.clone()
                    };
                    contribution
                        .defs
                        .entry(key)
                        .or_default()
                        .push((local as u32, def.flags));
                }
                Family::Ecma | Family::Blade => {}
            }
        }
        for (key, target, kind) in &summary.supers {
            let target = summary.interned(*target);
            for key in names(summary.interned(*key)) {
                contribution
                    .supers
                    .entry(key.clone())
                    .or_default()
                    .push((target.clone(), *kind));
                if *kind == php::PARENT {
                    contribution
                        .children
                        .entry(target.clone())
                        .or_default()
                        .push(key);
                }
            }
        }
        for (alias, target) in &summary.aliases {
            for name in names(summary.interned(*alias)) {
                contribution
                    .aliases
                    .entry(name)
                    .or_default()
                    .push(summary.interned(*target).clone());
            }
        }
        for opaque in &summary.opaque {
            for name in names(summary.interned(*opaque)) {
                contribution.opaque.entry(name).or_default().push(());
            }
        }
        let stars = summary
            .stars
            .iter()
            .map(|star| summary.interned(*star).clone())
            .collect::<Arc<[_]>>();
        let alternate = alternate.map(|alternate| Name::new(&alternate));
        for name in module.cloned().into_iter().chain(alternate) {
            contribution
                .modules
                .insert(name, vec![(Arc::clone(&stars), summary.open)]);
        }
        contribution
    }
}

/// Entries by name, each with the slot of the file it comes from.
type Entries<T> = HashMap<Name, Vec<(u32, T)>>;

/// Replaces `file`'s entries in `table` by `new`'s where they differ from `old`'s, collecting the
/// names whose entries changed.
fn replace<T: PartialEq>(
    table: &mut Entries<T>,
    file: u32,
    mut old: HashMap<Name, Vec<T>>,
    new: HashMap<Name, Vec<T>>,
    mut changed: Option<&mut HashSet<Name>>,
) {
    let drop = |table: &mut Entries<T>, name: &Name| {
        if let Some(entries) = table.get_mut(name) {
            entries.retain(|(owner, _)| *owner != file);
            if entries.is_empty() {
                table.remove(name);
            }
        }
    };
    for (name, after) in new {
        match old.remove(&name) {
            Some(before) if before == after => continue,
            Some(_) => drop(table, &name),
            None => {}
        }
        if let Some(changed) = changed.as_deref_mut() {
            changed.insert(name.clone());
        }
        let entries = table.entry(name).or_default();
        entries.extend(after.into_iter().map(|entry| (file, entry)));
        entries.shrink_to_fit();
    }
    for name in old.into_keys() {
        drop(table, &name);
        if let Some(changed) = changed.as_deref_mut() {
            changed.insert(name);
        }
    }
}

/// `work` over `items`, on the worker pool when there are many.
fn parallel<I: Sync, R: Send>(items: &[I], work: impl Fn(&I) -> R + Sync + Send) -> Vec<R> {
    if items.len() < 256 {
        items.iter().map(work).collect()
    } else {
        crate::WORK_POOL.install(|| items.par_iter().map(work).collect())
    }
}

/// What an update changed: the slots whose definitions or edges may have changed (those
/// replaced or removed, and those calling a name whose link changed), and whether the graph's
/// shape did (its numbering, a definition's flags or openness, or an edge), without which every
/// walk over it finds what it did. `touched` holds the definitions whose flags, openness or
/// edges may have changed, `dropped` those no longer in the graph.
pub(crate) struct GraphUpdate {
    pub(crate) slots: Vec<u32>,
    pub(crate) reshaped: bool,
    pub(crate) touched: Vec<DefRef>,
    pub(crate) dropped: Vec<DefRef>,
}

/// The items in one of two sorted lists but not the other.
fn sorted_difference<'a>(a: &'a [u32], b: &'a [u32]) -> impl Iterator<Item = &'a u32> {
    a.iter()
        .filter(|item| b.binary_search(item).is_err())
        .chain(b.iter().filter(|item| a.binary_search(item).is_err()))
}

/// One resolved edge of a file: `(definition, callee definition)`.
type FileEdge = (u32, DefRef);

/// Records the callers `slot`'s edges lose (`old` but not `new`) and gain (`new` but not `old`),
/// both sorted, by callee file, and those callers as `touched`; whether any differ.
fn diff_edges(
    slot: u32,
    (old, new): (&[FileEdge], &[FileEdge]),
    first: &[u32],
    changes: &mut BTreeMap<u32, CallerChanges>,
    touched: &mut Vec<DefRef>,
) -> bool {
    let (mut old, mut new) = (old.iter().peekable(), new.iter().peekable());
    let mut differ = false;
    loop {
        let (removed, added) = match (old.peek(), new.peek()) {
            (None, None) => return differ,
            (Some(a), Some(b)) if a == b => {
                old.next();
                new.next();
                continue;
            }
            (Some(a), Some(b)) if a < b => (old.next(), None),
            (Some(_), None) => (old.next(), None),
            _ => (None, new.next()),
        };
        differ = true;
        if let Some(&(owner, to)) = removed {
            let caller = DefRef {
                file: slot,
                local: owner,
            };
            touched.push(caller);
            changes
                .entry(to.file)
                .or_default()
                .0
                .push((to.local, caller));
        }
        if let Some(&(owner, to)) = added {
            let caller = DefRef {
                file: slot,
                local: owner,
            };
            touched.push(caller);
            let number = first[slot as usize] + owner;
            changes
                .entry(to.file)
                .or_default()
                .1
                .push((to.local, number, caller));
        }
    }
}

/// Lays out `file`'s callers again: those it had but `changes`' removed, merged in number order
/// with its added ones. Callers kept stay in order: numbers shift with their file's place.
fn relay_callers(file: &mut GraphFile, first: &[u32], changes: &mut CallerChanges) {
    let (removals, additions) = changes;
    removals.sort_unstable();
    additions.sort_unstable_by_key(|(local, number, _)| (*local, *number));
    let count = file.summary.defs.len();
    let number = |def: &DefRef| first[def.file as usize] + def.local;
    let mut offsets = Vec::with_capacity(count + 1);
    let mut callers = Vec::with_capacity(file.callers.len() + additions.len());
    offsets.push(0);
    let (mut removal, mut addition) = (0, 0);
    for local in 0..count as u32 {
        let had = match file.caller_offsets.get(local as usize + 1) {
            Some(&end) => &file.callers[file.caller_offsets[local as usize] as usize..end as usize],
            None => &[],
        };
        let start = removal;
        while removals.get(removal).is_some_and(|(at, _)| *at == local) {
            removal += 1;
        }
        let removed = &removals[start..removal];
        let mut kept = had
            .iter()
            .filter(|caller| {
                removed
                    .binary_search_by(|(_, removed)| removed.cmp(caller))
                    .is_err()
            })
            .peekable();
        while let Some(&(at, added, caller)) = additions.get(addition)
            && at == local
        {
            while let Some(caller) = kept.next_if(|caller| number(caller) < added) {
                callers.push(*caller);
            }
            callers.push(caller);
            addition += 1;
        }
        callers.extend(kept);
        offsets.push(callers.len() as u32);
    }
    debug_assert_eq!(
        addition,
        additions.len(),
        "every added caller calls a definition"
    );
    file.caller_offsets = offsets;
    file.callers = callers;
}

/// Whether a lookup of a name reads an entry `keys` changed: a prefix of it is a key, or it is a
/// key's parent, whose members a key adds or removes.
fn reader(keys: &HashSet<Name>) -> impl Fn(&Name) -> bool + '_ {
    let parents = keys
        .iter()
        .filter_map(|key| key.parent().cloned())
        .collect::<HashSet<_>>();
    // Such a name shares a key's root: most names are told apart by their root alone, without
    // walking their lineage.
    let mut roots = keys
        .iter()
        .map(|key| key.root().clone())
        .collect::<Vec<_>>();
    roots.sort_unstable();
    roots.dedup();
    move |name: &Name| {
        roots.binary_search(name.root()).is_ok()
            && (parents.contains(name) || name.lineage().any(|name| keys.contains(name)))
    }
}

/// What a Python class contributes to the graph's `subclasses` and `orphans`, and what its
/// bases' lookups read.
#[derive(Clone, Debug)]
struct ClassMemo {
    /// The workspace classes it derives from directly, by their own names.
    bases: Vec<Name>,
    /// A base of it no name denotes.
    orphan: bool,
    followed: Box<[Name]>,
    /// Names a lookup through a star import found no one had named: once one is, the lookup
    /// may find it.
    missing: Box<[Box<str>]>,
}

type Staged = (Arc<str>, u32, Option<Arc<Summary>>, Option<Arc<Summary>>);
/// A file's resolved edges, and its definitions calling something open.
type FileEdges = (Vec<(u32, DefRef)>, Vec<u32>);
/// A callee file's caller changes: `(callee, caller)` removed, and `(callee, caller's number,
/// caller)` added.
type CallerChanges = (Vec<(u32, DefRef)>, Vec<(u32, u32, DefRef)>);

// Per-thread so concurrently running tests cannot perturb each other's counts.
#[cfg(test)]
thread_local! {
    /// Definitions whose callers an update laid out again.
    pub(crate) static RELAID_DEFS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// A linked graph of every summary. Files may be replaced one at a time: only the names whose
/// link a replacement can change are linked again, and only the files calling them re-resolved.
#[derive(Clone, Default)]
pub(crate) struct Graph {
    /// Files by slot; a removed file's slot is reused.
    slots: Vec<Option<GraphFile>>,
    free: Vec<u32>,
    /// Slots by path: the order definitions are numbered in.
    paths: BTreeMap<Arc<str>, u32>,
    /// Definitions by qualified name: their index in the file, and flags.
    by_name: Entries<(u32, u8)>,
    aliases: Entries<Name>,
    /// Python: module and class members bound to a value no name denotes.
    opaque: Entries<()>,
    modules: Entries<Module>,
    /// PHP: class-likes' parents, traits and interfaces, and trait adaptations, by lowercased
    /// class (or `class.method`) name. Python: classes' bases and metaclasses.
    supers: Entries<(Name, u8)>,
    /// The classes that extend each class (PHP: by lowercased name; Python: by the name the
    /// subclass writes its base by).
    children: Entries<Name>,
    /// Python: each workspace class's direct subclasses, by their definitions' own names.
    subclasses: HashMap<Name, Vec<Name>>,
    /// Python: the classes a base of which no name denotes, which may derive from any class.
    orphans: Vec<Name>,
    /// Python: each workspace class's contribution to `subclasses` and `orphans`.
    class_memos: HashMap<Name, ClassMemo>,
    links: HashMap<Name, Memo>,
    /// Linked names by the roots an update's changed names must share to relink them (`reader`):
    /// each name's own root and those of the names it followed. An entry may be stale (a name
    /// since unlinked, or no longer following that root): lookups read the name's memo, and the
    /// index is rebuilt once it holds twice the entries it held when last built.
    link_roots: HashMap<Name, Vec<Name>>,
    link_root_entries: usize,
    link_roots_built: usize,
    /// Per slot, its first definition's number: definitions are numbered by file path, then in
    /// the file.
    first: Vec<u32>,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug)]
struct GraphDef {
    file: u32,
    local: u32,
    flags: u8,
}

/// The graph laid out by number, for the dense closure.
#[cfg(test)]
struct Dense {
    defs: Vec<GraphDef>,
    open: Vec<bool>,
    /// Call edges, per definition: `edges[offsets[d]..offsets[d + 1]]`.
    offsets: Vec<u32>,
    edges: Vec<u32>,
}

#[cfg(test)]
impl Dense {
    fn successors(&self, def: u32) -> &[u32] {
        &self.edges[self.offsets[def as usize] as usize..self.offsets[def as usize + 1] as usize]
    }
}

/// Graph size, for measurements.
#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct GraphStats {
    pub(crate) files: usize,
    pub(crate) defs: usize,
    pub(crate) edges: usize,
    /// Distinct callee names per file, summed.
    pub(crate) targets: usize,
    pub(crate) open_targets: usize,
    /// Targets that link to no definition: outside the workspace, or no edge statically.
    pub(crate) unlinked_targets: usize,
    pub(crate) open_defs: usize,
}

/// The dotted name a file's path gives it from the deepest directory whose name is no Python
/// identifier (a checkout's own directory): what `import` finds with that directory on
/// `sys.path`, namespace packages included. `pkg/__init__.py` is `pkg`.
pub(crate) fn path_module(path: &str) -> Option<String> {
    let path = path
        .strip_suffix(".py")
        .or_else(|| path.strip_suffix(".pyi"))?;
    let mut segments = path.split('/').collect::<Vec<_>>();
    if segments.len() > 1 && segments.last() == Some(&"__init__") {
        segments.pop();
    }
    let identifier = |segment: &&str| {
        let mut chars = segment.chars();
        chars
            .next()
            .is_some_and(|first| first == '_' || first.is_alphabetic())
            && chars.all(|c| c == '_' || c.is_alphanumeric())
    };
    let valid = segments.iter().rev().take_while(|s| identifier(s)).count();
    (valid > 0).then(|| segments[segments.len() - valid..].join("."))
}

impl Graph {
    /// Links every summary. Definitions are numbered by file path, so the result is independent
    /// of order.
    pub(crate) fn link(files: Vec<(Arc<str>, Arc<Summary>)>) -> Self {
        let mut graph = Self::default();
        graph.update(
            files
                .into_iter()
                .map(|(path, summary)| (path, Some(summary))),
        );
        graph
    }

    /// Replaces, adds (`Some`) or removes (`None`) files, and numbers the result as `link` would.
    pub(crate) fn update(
        &mut self,
        changes: impl IntoIterator<Item = (Arc<str>, Option<Arc<Summary>>)>,
    ) -> GraphUpdate {
        let mut orphans = Vec::new();
        // Each change's slot, and its file's old and new summary.
        let mut staged = Vec::new();
        // The replaced and removed files as they were linked.
        let mut previous = HashMap::new();
        let mut renumber = false;
        for (path, summary) in changes {
            let known = self.paths.get(&path).copied();
            let old = known.and_then(|slot| self.slots[slot as usize].take());
            if let Some(old) = &old {
                // The file no longer calls what its old summary called.
                for target in &old.summary.targets {
                    let name = old.summary.interned(*target);
                    if let Some(memo) = self.links.get_mut(name) {
                        memo.users.retain(|user| Some(*user) != known);
                        if memo.users.is_empty() {
                            orphans.push(name.clone());
                        }
                    }
                }
            }
            let slot = match (known, &summary) {
                (Some(slot), _) => slot,
                (None, None) => continue,
                (None, Some(_)) => match self.free.pop() {
                    Some(slot) => slot,
                    None => {
                        self.slots.push(None);
                        self.slots.len() as u32 - 1
                    }
                },
            };
            renumber |= known.is_none() || summary.is_none();
            let old = old.map(|old| {
                let summary = Arc::clone(&old.summary);
                previous.insert(slot, old);
                summary
            });
            staged.push((path, slot, old, summary));
        }
        let contribution = |(path, _, old, new): &Staged| {
            let of = |summary: &Option<Arc<Summary>>| {
                summary
                    .as_ref()
                    .map(|summary| Contribution::of(path, summary))
                    .unwrap_or_default()
            };
            (of(old), of(new))
        };
        let contributions = parallel(&staged, contribution);
        // Before anything is linked, no link can go stale.
        let mut keys = (!self.links.is_empty()).then(HashSet::new);
        let mut replaced = Vec::new();
        let mut resized = Vec::new();
        let mut reshaped = false;
        let mut removed = Vec::new();
        let mut touched = Vec::new();
        let mut dropped = Vec::new();
        for ((path, slot, _, summary), (before, after)) in staged.into_iter().zip(contributions) {
            replace(
                &mut self.by_name,
                slot,
                before.defs,
                after.defs,
                keys.as_mut(),
            );
            replace(
                &mut self.aliases,
                slot,
                before.aliases,
                after.aliases,
                keys.as_mut(),
            );
            replace(
                &mut self.opaque,
                slot,
                before.opaque,
                after.opaque,
                keys.as_mut(),
            );
            replace(
                &mut self.modules,
                slot,
                before.modules,
                after.modules,
                keys.as_mut(),
            );
            replace(
                &mut self.supers,
                slot,
                before.supers,
                after.supers,
                keys.as_mut(),
            );
            replace(
                &mut self.children,
                slot,
                before.children,
                after.children,
                keys.as_mut(),
            );
            match summary {
                Some(summary) => {
                    self.paths.insert(Arc::clone(&path), slot);
                    // Callers in other files keep calling the definitions they did: links that
                    // change are resolved again below, and their callers laid out again.
                    let (caller_offsets, callers) = match previous.get_mut(&slot) {
                        Some(old) => {
                            let (old_defs, new_defs) = (&old.summary.defs, &summary.defs);
                            if old_defs.len() != new_defs.len() {
                                resized.push(slot);
                            }
                            dropped.extend((new_defs.len()..old_defs.len()).map(|local| DefRef {
                                file: slot,
                                local: local as u32,
                            }));
                            reshaped |= old.summary.open != summary.open
                                || old_defs.len() != new_defs.len()
                                || old_defs
                                    .iter()
                                    .zip(new_defs.iter())
                                    .any(|(old, new)| old.flags != new.flags);
                            (
                                std::mem::take(&mut old.caller_offsets),
                                std::mem::take(&mut old.callers),
                            )
                        }
                        None => Default::default(),
                    };
                    touched.extend(
                        (0..summary.defs.len() as u32).map(|local| DefRef { file: slot, local }),
                    );
                    self.slots[slot as usize] = Some(GraphFile {
                        path,
                        summary,
                        edges: Vec::new(),
                        open: Vec::new(),
                        caller_offsets,
                        callers,
                    });
                    replaced.push(slot);
                }
                None => {
                    self.paths.remove(&path);
                    self.free.push(slot);
                    removed.push(slot);
                    let count = previous[&slot].summary.defs.len() as u32;
                    dropped.extend((0..count).map(|local| DefRef { file: slot, local }));
                }
            }
        }
        // A name links through its own entries, its prefixes', and those of the names it
        // followed; a followed class through its members' (a lookup in its method resolution
        // order), a late-bound Python name through its class's subclasses.
        let linked = keys.is_some();
        let mut keys = keys.unwrap_or_default();
        let mut orphans_changed = false;
        if !linked || !keys.is_empty() {
            let (bases, orphans) = self.python_classes(linked.then_some(&keys));
            if linked {
                keys.extend(bases);
                orphans_changed = orphans;
            }
        }
        let reads = reader(&keys);
        let is_stale = |name: &Name, memo: &Memo| {
            reads(name)
                || memo.followed.iter().any(&reads)
                || (orphans_changed && name.text().ends_with('^'))
        };
        let stale = if keys.is_empty() {
            Vec::new()
        } else if orphans_changed {
            self.links
                .iter()
                .filter(|(name, memo)| is_stale(name, memo))
                .map(|(name, _)| name.clone())
                .collect::<Vec<_>>()
        } else {
            // A name `reads` shares a key's root, itself or through a name it followed.
            let mut roots = keys.iter().map(Name::root).collect::<Vec<_>>();
            roots.sort_unstable();
            roots.dedup();
            let mut seen = HashSet::new();
            roots
                .into_iter()
                .filter_map(|root| self.link_roots.get(root))
                .flatten()
                .filter(|name| seen.insert(*name))
                .filter(|name| {
                    self.links
                        .get(*name)
                        .is_some_and(|memo| is_stale(name, memo))
                })
                .cloned()
                .collect::<Vec<_>>()
        };
        #[cfg(test)]
        assert_eq!(
            stale.iter().collect::<HashSet<_>>(),
            self.links
                .iter()
                .filter(|(name, memo)| !keys.is_empty() && is_stale(name, memo))
                .map(|(name, _)| name)
                .collect::<HashSet<_>>(),
            "the root index finds every stale link"
        );
        let mut relinked = BTreeSet::new();
        for name in stale {
            let Some(mut memo) = self.links.remove(&name) else {
                continue;
            };
            let (link, followed) = self.link_followed(&name);
            if link.defs[..] != memo.defs[..] || link.open != memo.open {
                relinked.extend(memo.users.iter().copied());
            }
            if memo.users.is_empty() {
                continue;
            }
            if memo.followed[..] != followed[..] {
                self.index_link(&name, &followed);
            }
            memo.set(link, followed);
            self.links.insert(name, memo);
        }
        self.link_calls(&replaced);
        // Files whose callee links changed resolve their edges again; they call what they did.
        relinked.extend(replaced);
        let mut relinked = relinked.into_iter().collect::<Vec<_>>();
        let resolved = parallel(&relinked, |slot| self.resolve_edges(*slot));
        renumber |= !resized.is_empty();
        if renumber {
            self.number();
        }
        reshaped |= renumber;
        // Callers per callee file: those of edges a file no longer has, and of its new ones.
        let mut callers = BTreeMap::<u32, CallerChanges>::new();
        for (&slot, resolved) in relinked.iter().zip(resolved) {
            let (Some(file), Some((edges, open))) = (&mut self.slots[slot as usize], resolved)
            else {
                continue;
            };
            let (old_edges, old_open) = match previous.remove(&slot) {
                Some(old) => (old.edges, old.open),
                None => (
                    std::mem::take(&mut file.edges),
                    std::mem::take(&mut file.open),
                ),
            };
            for &owner in sorted_difference(&old_open, &open) {
                touched.push(DefRef {
                    file: slot,
                    local: owner,
                });
                reshaped = true;
            }
            reshaped |= diff_edges(
                slot,
                (&old_edges, &edges),
                &self.first,
                &mut callers,
                &mut touched,
            );
            (file.edges, file.open) = (edges, open);
        }
        // What remains was removed: none of its edges remain.
        for (slot, old) in previous {
            diff_edges(
                slot,
                (&old.edges, &[]),
                &self.first,
                &mut callers,
                &mut touched,
            );
        }
        for slot in resized {
            callers.entry(slot).or_default();
        }
        self.relay(callers);
        // A name no file calls any more is linked again if one does.
        for name in orphans {
            if self
                .links
                .get(&name)
                .is_some_and(|memo| memo.users.is_empty())
            {
                self.links.remove(&name);
            }
        }
        if linked {
            self.compact_link_roots();
        } else {
            self.link_roots_built = self.link_root_entries;
        }
        relinked.extend(removed);
        relinked.sort_unstable();
        relinked.dedup();
        GraphUpdate {
            slots: relinked,
            reshaped,
            touched,
            dropped,
        }
    }

    /// Lays out again the callers of the files `changes` names, keeping each list in number
    /// order.
    fn relay(&mut self, changes: BTreeMap<u32, CallerChanges>) {
        let mut jobs = changes
            .into_iter()
            .filter_map(|(slot, changes)| Some((slot, self.slots[slot as usize].take()?, changes)))
            .collect::<Vec<_>>();
        #[cfg(test)]
        RELAID_DEFS.with(|count| {
            count.set(
                count.get()
                    + jobs
                        .iter()
                        .map(|(_, file, _)| file.summary.defs.len())
                        .sum::<usize>(),
            )
        });
        let first = &self.first;
        if jobs.len() < 64 {
            for (_, file, changes) in &mut jobs {
                relay_callers(file, first, changes);
            }
        } else {
            crate::WORK_POOL.install(|| {
                jobs.par_iter_mut()
                    .for_each(|(_, file, changes)| relay_callers(file, first, changes))
            });
        }
        for (slot, file, _) in jobs {
            self.slots[slot as usize] = Some(file);
        }
    }

    /// Links what the files in `slots` call, and records them as its users.
    fn link_calls(&mut self, slots: &[u32]) {
        let summaries = slots
            .iter()
            .filter_map(|slot| {
                Some((
                    *slot,
                    Arc::clone(&self.slots[*slot as usize].as_ref()?.summary),
                ))
            })
            .collect::<Vec<_>>();
        let unlinked = summaries
            .iter()
            .flat_map(|(_, summary)| {
                summary
                    .targets
                    .iter()
                    .map(|target| summary.interned(*target))
            })
            .filter(|name| !self.links.contains_key(*name))
            .collect::<HashSet<_>>()
            .into_iter()
            .cloned()
            .collect::<Vec<_>>();
        let linked = parallel(&unlinked, |name| self.link_followed(name));
        self.links.reserve(unlinked.len());
        for (name, (link, followed)) in unlinked.into_iter().zip(linked) {
            let mut memo = Memo {
                defs: Box::default(),
                open: false,
                followed: Box::default(),
                users: Vec::new(),
            };
            self.index_link(&name, &followed);
            memo.set(link, followed);
            self.links.insert(name, memo);
        }
        for (slot, summary) in &summaries {
            for target in &summary.targets {
                let memo = self
                    .links
                    .get_mut(summary.interned(*target))
                    .expect("every callee name is linked");
                memo.users.push(*slot);
            }
        }
    }

    /// Indexes `name`, which follows `followed`, under the roots a change must share to relink it.
    fn index_link(&mut self, name: &Name, followed: &[Name]) {
        let mut roots = std::iter::once(name)
            .chain(followed)
            .map(Name::root)
            .collect::<Vec<_>>();
        roots.sort_unstable();
        roots.dedup();
        self.link_root_entries += roots.len();
        for root in roots {
            self.link_roots
                .entry(root.clone())
                .or_default()
                .push(name.clone());
        }
    }

    /// Rebuilds the root index once stale entries may outnumber current ones.
    fn compact_link_roots(&mut self) {
        if self.link_root_entries <= 2 * self.link_roots_built.max(self.links.len()) + 1024 {
            return;
        }
        self.link_roots.clear();
        self.link_root_entries = 0;
        let links = std::mem::take(&mut self.links);
        for (name, memo) in &links {
            self.index_link(name, &memo.followed);
        }
        self.links = links;
        self.link_roots_built = self.link_root_entries;
    }

    /// `slot`'s definitions' edges and the definitions calling something open, from the links of
    /// what they call.
    fn resolve_edges(&self, slot: u32) -> Option<FileEdges> {
        let summary = &self.slots[slot as usize].as_ref()?.summary;
        let mut edges = Vec::new();
        let mut open = Vec::new();
        for (owner, name) in &summary.edges {
            let memo = &self.links[summary.interned(*name)];
            if memo.open && open.last() != Some(owner) {
                open.push(*owner);
            }
            edges.extend(memo.defs.iter().map(|to| (*owner, *to)));
        }
        edges.sort_unstable();
        edges.dedup();
        Some((edges, open))
    }

    /// Swaps in `path`'s new summary, which names, calls and defines what its linked one does.
    pub(crate) fn resummarize(&mut self, path: &str, summary: Arc<Summary>) {
        if let Some(&slot) = self.paths.get(path)
            && let Some(file) = &mut self.slots[slot as usize]
        {
            debug_assert_eq!(file.summary.defs.len(), summary.defs.len());
            file.summary = summary;
        }
    }

    /// Numbers definitions by file path: each file's first number. Definitions keep their place
    /// in their file, so a file's own numbers shift by the definitions before it.
    fn number(&mut self) {
        self.first.resize(self.slots.len(), 0);
        let mut next = 0;
        for &slot in self.paths.values() {
            self.first[slot as usize] = next;
            let file = self.slots[slot as usize]
                .as_ref()
                .expect("a path's file is linked");
            next += file.summary.defs.len() as u32;
        }
    }

    fn number_of(&self, def: DefRef) -> u32 {
        self.first[def.file as usize] + def.local
    }

    /// Whether `def`'s file is linked and holds it.
    pub(crate) fn holds(&self, def: DefRef) -> bool {
        self.slots
            .get(def.file as usize)
            .and_then(Option::as_ref)
            .is_some_and(|file| (def.local as usize) < file.summary.defs.len())
    }

    /// Whether `def` calls something unknown, or its file is open.
    pub(crate) fn is_open(&self, def: DefRef) -> bool {
        let file = self.slots[def.file as usize]
            .as_ref()
            .expect("a linked definition's file is linked");
        file.summary.open
            || file.summary.defs[def.local as usize].flags & OPEN != 0
            || file.open.binary_search(&def.local).is_ok()
    }

    /// The definitions `def` calls.
    pub(crate) fn callees_of(&self, def: DefRef) -> impl Iterator<Item = DefRef> + '_ {
        let edges = self.file_edges(def.file);
        let start = edges.partition_point(|(owner, _)| *owner < def.local);
        edges[start..]
            .iter()
            .take_while(move |(owner, _)| *owner == def.local)
            .map(|(_, to)| *to)
    }

    pub(crate) fn callers_of(&self, def: DefRef) -> &[DefRef] {
        let Some(file) = self.slots[def.file as usize].as_ref() else {
            return &[];
        };
        match file.caller_offsets.get(def.local as usize + 1) {
            Some(&end) => {
                &file.callers[file.caller_offsets[def.local as usize] as usize..end as usize]
            }
            None => &[],
        }
    }

    /// The definitions `allow` lets a chain pass that reach one of `seeds` through definitions it
    /// lets pass, each with its hop: a backward walk over callers, whose cost is the region it
    /// finds. Breadth-first from the seeds and over callers in number order, so each hop is the
    /// first callee of a shortest chain, whatever order files were linked in.
    pub(crate) fn reach(&self, seeds: &[DefRef], allow: impl Fn(u8) -> bool) -> Region {
        let mut starts = seeds
            .iter()
            .filter(|def| self.holds(**def) && allow(self.flags(**def)))
            .map(|def| (self.number_of(*def), *def))
            .collect::<Vec<_>>();
        starts.sort_unstable();
        starts.dedup();
        let mut region = Region::with_capacity(starts.len());
        let mut queue = std::collections::VecDeque::with_capacity(starts.len());
        for (_, start) in starts {
            region.insert(start, Hop::Seed);
            queue.push_back(start);
        }
        while let Some(callee) = queue.pop_front() {
            for &caller in self.callers_of(callee) {
                if !allow(self.flags(caller)) {
                    continue;
                }
                if let std::collections::hash_map::Entry::Vacant(entry) = region.entry(caller) {
                    entry.insert(Hop::Via(callee));
                    queue.push_back(caller);
                }
            }
        }
        region
    }

    /// The definitions `allow` lets pass that are one of `sources`, or call one through
    /// definitions it lets pass.
    pub(crate) fn upward(
        &self,
        sources: impl IntoIterator<Item = DefRef>,
        allow: impl Fn(u8) -> bool,
    ) -> HashSet<DefRef> {
        let mut found = HashSet::new();
        let mut stack = sources
            .into_iter()
            .filter(|def| allow(self.flags(*def)))
            .collect::<Vec<_>>();
        stack.retain(|def| found.insert(*def));
        while let Some(def) = stack.pop() {
            for &caller in self.callers_of(def) {
                if allow(self.flags(caller)) && found.insert(caller) {
                    stack.push(caller);
                }
            }
        }
        found
    }

    /// The definitions that call something unknown, or whose file is open.
    pub(crate) fn open_defs(&self) -> impl Iterator<Item = DefRef> + '_ {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(slot, file)| Some((slot as u32, file.as_ref()?)))
            .flat_map(|(slot, file)| {
                let mut calls_open = file.open.iter().copied().peekable();
                file.summary
                    .defs
                    .iter()
                    .zip(0..)
                    .filter(move |(def, local)| {
                        let calls_open = calls_open.next_if_eq(local).is_some();
                        file.summary.open || def.flags & OPEN != 0 || calls_open
                    })
                    .map(move |(_, local)| DefRef { file: slot, local })
            })
    }

    /// `slot`'s definitions.
    pub(crate) fn file_defs(&self, slot: u32) -> impl Iterator<Item = DefRef> {
        let count = self
            .slots
            .get(slot as usize)
            .and_then(Option::as_ref)
            .map_or(0, |file| file.summary.defs.len() as u32);
        (0..count).map(move |local| DefRef { file: slot, local })
    }

    /// The files calling one of `defs`: the users of every name linking to one. Every name a
    /// file calls is linked, with the file among its users.
    pub(crate) fn calling(&self, defs: &HashSet<DefRef>) -> Vec<Arc<str>> {
        let mut wanted = defs.iter().copied().collect::<Vec<_>>();
        wanted.sort_unstable();
        let mut slots = self
            .links
            .values()
            .filter(|memo| {
                memo.defs
                    .iter()
                    .any(|def| wanted.binary_search(def).is_ok())
            })
            .flat_map(|memo| memo.users.iter().copied())
            .collect::<Vec<_>>();
        slots.sort_unstable();
        slots.dedup();
        let files = slots
            .into_iter()
            .filter_map(|slot| Some(Arc::clone(&self.slots.get(slot as usize)?.as_ref()?.path)))
            .collect::<Vec<_>>();
        #[cfg(test)]
        {
            let scanned = self
                .slots
                .iter()
                .flatten()
                .filter(|file| {
                    file.summary.targets.iter().any(|target| {
                        self.links
                            .get(file.summary.interned(*target))
                            .is_some_and(|memo| memo.defs.iter().any(|def| defs.contains(def)))
                    })
                })
                .map(|file| Arc::clone(&file.path))
                .collect::<BTreeSet<_>>();
            assert_eq!(
                files.iter().cloned().collect::<BTreeSet<_>>(),
                scanned,
                "link users name every calling file"
            );
        }
        files
    }

    /// The linked file at `path`.
    pub(crate) fn slot(&self, path: &str) -> Option<u32> {
        self.paths.get(path).copied()
    }

    pub(crate) fn path(&self, slot: u32) -> Option<&Arc<str>> {
        Some(&self.slots.get(slot as usize)?.as_ref()?.path)
    }

    pub(crate) fn summary_of(&self, slot: u32) -> Option<&Arc<Summary>> {
        Some(&self.slots.get(slot as usize)?.as_ref()?.summary)
    }

    /// `slot`'s resolved edges: `(definition, callee)`.
    pub(crate) fn file_edges(&self, slot: u32) -> &[(u32, DefRef)] {
        self.slots
            .get(slot as usize)
            .and_then(Option::as_ref)
            .map_or(&[], |file| &file.edges)
    }

    /// The definition of `path` whose node starts at `start`.
    pub(crate) fn def_at(&self, path: &str, start: u32) -> Option<DefRef> {
        let slot = self.slot(path)?;
        let summary = self.summary_of(slot)?;
        let local = summary.defs.partition_point(|def| def.start < start);
        (summary.defs.get(local)?.start == start).then_some(DefRef {
            file: slot,
            local: local as u32,
        })
    }

    /// `def`'s file and node start.
    #[cfg(test)]
    pub(crate) fn def_site(&self, def: DefRef) -> (&Arc<str>, u32) {
        let file = self.slots[def.file as usize]
            .as_ref()
            .expect("a linked definition's file is linked");
        (&file.path, file.summary.defs[def.local as usize].start)
    }

    /// `def`'s qualified name, as its language writes it.
    pub(crate) fn def_label(&self, def: DefRef) -> std::borrow::Cow<'_, str> {
        let summary = self
            .summary_of(def.file)
            .expect("a reached definition is linked");
        let name = summary.interned(summary.defs[def.local as usize].name);
        match name.family() {
            Family::Php => std::borrow::Cow::Owned(php::label(name.text())),
            Family::Python | Family::Ecma | Family::Blade => {
                std::borrow::Cow::Borrowed(name.text())
            }
        }
    }

    /// What a call of `name` links to, in number order, and whether it may be a module member no
    /// definition gives.
    pub(crate) fn link_of(&self, name: &Name) -> (std::borrow::Cow<'_, [DefRef]>, bool) {
        match self.links.get(name) {
            Some(memo) => (std::borrow::Cow::Borrowed(&memo.defs), memo.open),
            None => {
                let link = self.link_name(name, 0, &mut Vec::new());
                (std::borrow::Cow::Owned(link.defs), link.open)
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn stats(&self) -> GraphStats {
        let mut stats = GraphStats {
            files: self.paths.len(),
            defs: self.def_count(),
            edges: self
                .slots
                .iter()
                .flatten()
                .map(|file| file.edges.len())
                .sum(),
            open_defs: self.open_defs().count(),
            ..GraphStats::default()
        };
        for file in self.slots.iter().flatten() {
            for target in &file.summary.targets {
                stats.targets += 1;
                match self.links.get(file.summary.interned(*target)) {
                    Some(memo) if memo.open => stats.open_targets += 1,
                    Some(memo) if memo.defs.is_empty() => stats.unlinked_targets += 1,
                    None => stats.unlinked_targets += 1,
                    Some(_) => {}
                }
            }
        }
        stats
    }

    /// The graph laid out by number: definitions, openness and call edges.
    #[cfg(test)]
    fn dense(&self) -> Dense {
        let mut defs = Vec::new();
        for &slot in self.paths.values() {
            let file = self.slots[slot as usize]
                .as_ref()
                .expect("a path's file is linked");
            defs.extend(
                file.summary
                    .defs
                    .iter()
                    .zip(0..)
                    .map(|(def, local)| GraphDef {
                        file: slot,
                        local,
                        flags: def.flags,
                    }),
            );
        }
        let mut open = vec![false; defs.len()];
        for def in self.open_defs() {
            open[self.number_of(def) as usize] = true;
        }
        let mut offsets = vec![0];
        let mut edges = Vec::new();
        for def in &defs {
            let file = self.slots[def.file as usize].as_ref().expect("linked");
            let mut targets = file
                .edges
                .iter()
                .filter(|(owner, _)| *owner == def.local)
                .map(|(_, to)| self.number_of(*to))
                .collect::<Vec<_>>();
            targets.sort_unstable();
            edges.extend(targets);
            offsets.push(edges.len() as u32);
        }
        Dense {
            defs,
            open,
            offsets,
            edges,
        }
    }

    /// Each definition's callers, by number, in number order.
    #[cfg(test)]
    pub(crate) fn caller_numbers(&self) -> Vec<Vec<u32>> {
        self.dense()
            .defs
            .iter()
            .map(|def| {
                let def = DefRef {
                    file: def.file,
                    local: def.local,
                };
                self.callers_of(def)
                    .iter()
                    .map(|caller| self.number_of(*caller))
                    .collect()
            })
            .collect()
    }

    #[cfg(test)]
    /// Strongly connected components in reverse topological order: callees first.
    pub(crate) fn components(&self) -> Vec<Vec<u32>> {
        let dense = self.dense();
        tarjan(dense.defs.len(), |def| dense.successors(def))
    }

    /// What `name` links to, and the names linking it followed.
    fn link_followed(&self, name: &Name) -> (Link, Vec<Name>) {
        let mut followed = Vec::new();
        let link = self.link_name(name, 0, &mut followed);
        followed.sort_unstable();
        followed.dedup();
        (link, followed)
    }

    fn link_name(&self, name: &Name, hops: u32, followed: &mut Vec<Name>) -> Link {
        if hops > MAX_HOPS {
            return Link {
                defs: Vec::new(),
                open: true,
            };
        }
        match name.family() {
            Family::Python => self.link_python(name, hops, followed),
            Family::Php => self.link_php(name, hops, followed),
            Family::Ecma | Family::Blade => Link::default(),
        }
    }

    /// Links a PHP callee name. A namespace fallback (`\ns\f|\f`) is the namespaced function
    /// where the workspace declares one, else the global one. A method `\c.m` is, as PHP looks
    /// it up, `c`'s own, then an alias its trait adaptations give, then a used trait's (but those
    /// `insteadof` excludes), then its parent's, then an interface's (an abstract class's); an
    /// abstract method (a trait's) yields to a later step's concrete one. A
    /// late-bound method `\c.m^` is `\c.m` and, unless no override can replace that for a call in
    /// `c` (`sealed_in`), the method every class the workspace derives from `c` looks up. A name
    /// no definition gives (a builtin, a vendor class, an undefined method only `__call` could
    /// answer) has no edge.
    fn link_php(&self, name: &Name, hops: u32, followed: &mut Vec<Name>) -> Link {
        let text = name.text();
        if let Some(base) = text.strip_suffix('^') {
            return self.link_late(&Name::new(base), hops, followed);
        }
        if let Some((namespaced, global)) = text.split_once('|') {
            for candidate in [namespaced, global] {
                let candidate = Name::new(candidate);
                let link = self.link_name(&candidate, hops + 1, followed);
                followed.push(candidate);
                if link.open || !link.defs.is_empty() {
                    return link;
                }
            }
            return Link::default();
        }
        if let Some(entries) = self.by_name.get(name) {
            let mut defs = entries
                .iter()
                .filter(|(_, (_, flags))| flags & CLASS == 0)
                .map(|(file, (local, _))| DefRef {
                    file: *file,
                    local: *local,
                })
                .collect::<Vec<_>>();
            if !defs.is_empty() {
                defs.sort_unstable_by_key(|def| self.path_order(*def));
                return Link { defs, open: false };
            }
        }
        let Some(class) = name.parent() else {
            return Link::default();
        };
        let method = &text[class.text().len() + 1..];
        let supers = |key: &Name, kind: u8| {
            self.supers
                .get(key)
                .into_iter()
                .flatten()
                .filter(move |(_, (_, of))| *of == kind)
                .map(|(_, (target, _))| target)
        };
        let member = |class: &Name| Name::new(&format!("{}.{method}", class.text()));
        // Trait adaptations: `T::m as n`, `m as n`.
        let mut names = supers(name, php::ALIAS).cloned().collect::<Vec<_>>();
        for aliased in supers(name, php::ALIAS_TRAITS) {
            names.extend(
                supers(class, php::TRAIT)
                    .map(|used| Name::new(&format!("{}.{}", used.text(), aliased.text()))),
            );
        }
        let excluded = supers(name, php::EXCLUDE).collect::<Vec<_>>();
        let mut steps = vec![names];
        steps.push(
            supers(class, php::TRAIT)
                .filter(|used| !excluded.contains(used))
                .map(member)
                .collect(),
        );
        steps.push(supers(class, php::PARENT).map(member).collect());
        steps.push(supers(class, php::INTERFACE).map(member).collect());
        // An abstract method (a trait's, an interface's) is what a later step's concrete one
        // implements: it is the answer only where none is.
        let mut declared = None;
        for names in steps {
            if names.is_empty() {
                continue;
            }
            let link = self.union(names, hops, followed);
            if link.open {
                return link;
            }
            if link.defs.is_empty() {
                continue;
            }
            if !link
                .defs
                .iter()
                .all(|def| self.flags(*def) & php::ABSTRACT != 0)
            {
                return link;
            }
            declared.get_or_insert(link);
        }
        declared.unwrap_or_default()
    }

    /// A linked definition's flags.
    pub(crate) fn flags(&self, def: DefRef) -> u8 {
        let file = self.slots[def.file as usize]
            .as_ref()
            .expect("a linked definition's file is linked");
        file.summary.defs[def.local as usize].flags
    }

    /// `method` (`\c.m`) late-bound: what `c` and every class derived from it look up.
    fn link_late(&self, method: &Name, hops: u32, followed: &mut Vec<Name>) -> Link {
        let mut link = self.link_name(method, hops + 1, followed);
        followed.push(method.clone());
        let Some(class) = method.parent() else {
            return link;
        };
        let sealed = !link.defs.is_empty()
            && link
                .defs
                .iter()
                .all(|def| self.sealed_in(class, method, *def));
        if link.open || sealed {
            return link;
        }
        let suffix = &method.text()[class.text().len()..];
        let mut seen = HashSet::from([class.clone()]);
        let mut pending = vec![class.clone()];
        while let Some(class) = pending.pop() {
            for (_, child) in self.children.get(&class).into_iter().flatten() {
                if !seen.insert(child.clone()) {
                    continue;
                }
                let name = Name::new(&format!("{}{suffix}", child.text()));
                let found = self.link_name(&name, hops + 1, followed);
                followed.push(name);
                link.defs.extend(found.defs);
                link.open |= found.open;
                pending.push(child.clone());
            }
        }
        link.defs.sort_unstable_by_key(|def| self.path_order(*def));
        link.defs.dedup();
        link
    }

    /// Whether no override replaces `def`, which `method` (`\c.m`) links to, for a call in `c`.
    /// A final method no derived class declares. A private one binds a call in the class that
    /// has it: `c`'s own, or one `c` imports from a trait (the copy is `c`'s), private as `c`'s
    /// adaptations leave it (`m as private`, `m as public`); not one `c` inherits, which PHP
    /// passes over for an override, nor a constructor, which PHP looks up on the created class.
    fn sealed_in(&self, class: &Name, method: &Name, def: DefRef) -> bool {
        let file = self.slots[def.file as usize]
            .as_ref()
            .expect("a linked definition's file is linked");
        let declared = &file.summary.defs[def.local as usize];
        let name = file.summary.name(declared.name).to_ascii_lowercase();
        let owner = name.rsplit_once('.').map_or("", |(owner, _)| owner);
        let mut private = declared.flags & php::PRIVATE != 0;
        let mut final_method = declared.flags & php::FINAL != 0;
        let supers = |key: &Name| {
            self.supers
                .get(key)
                .into_iter()
                .flatten()
                .map(|(_, (target, kind))| (target, *kind))
        };
        let imported = owner != class.text()
            && supers(class).any(|(used, kind)| kind == php::TRAIT && used.text() == owner);
        if imported {
            for (target, kind) in supers(method) {
                // `T::m as private` adapts `T`'s `m`; `m as private` that of whichever trait has it.
                if target.text().contains('.') && target.text() != name {
                    continue;
                }
                match kind {
                    php::PRIVATE_AS => private = true,
                    php::VISIBLE_AS => private = false,
                    php::FINAL_AS => final_method = true,
                    _ => {}
                }
            }
        }
        if method.text().ends_with(".__construct") {
            final_method
        } else if imported || owner == class.text() {
            final_method || private
        } else {
            final_method && !private
        }
    }

    /// Links a Python callee name: what the name itself names (`member`), else the member of the
    /// longest prefix something names: an alias to follow, a class whose method resolution order
    /// gives the attribute, a function or value whose attribute is set at run time (unknown), or a
    /// module whose member this is. A late-bound name (`pkg.C.m^`, `pkg.C.^` for `cls()`) is the
    /// class's and every workspace subclass's.
    fn link_python(&self, name: &Name, hops: u32, followed: &mut Vec<Name>) -> Link {
        if name.text().ends_with('^') {
            return self.link_late_python(name, hops, followed);
        }
        if let Some(link) = self.member(name, hops, followed) {
            return link;
        }
        for prefix in name.lineage().skip(1) {
            let rest = &name.text()[prefix.text().len()..];
            if let Some(entries) = self.aliases.get(prefix) {
                let targets = entries
                    .iter()
                    .filter(|(_, target)| target != prefix)
                    .map(|(_, target)| Name::new(&format!("{}{rest}", target.text())))
                    .collect::<Vec<_>>();
                if !targets.is_empty() {
                    return self.union(targets, hops, followed);
                }
            }
            if let Some(entries) = self.by_name.get(prefix) {
                let attribute = &rest[1..];
                if !attribute.contains('.')
                    && entries.iter().all(|(_, (_, flags))| flags & CLASS != 0)
                {
                    return self.class_member(prefix, attribute, hops, followed);
                }
                return OPEN_LINK;
            }
            if self.opaque.contains_key(prefix) {
                return OPEN_LINK;
            }
            if let Some(entries) = self.modules.get(prefix) {
                // A member of a workspace module that neither a definition, an alias nor another
                // binding gives: a submodule's. Only a star import can bind it to a definition,
                // and only an open summary can hide one.
                let stars = entries
                    .iter()
                    .flat_map(|(_, (stars, _))| stars.iter())
                    .map(|star| Name::new(&format!("{}{rest}", star.text())))
                    .collect::<Vec<_>>();
                let starred = !stars.is_empty();
                let mut link = self.union(stars, hops, followed);
                let open = entries.iter().any(|(_, (_, open))| *open);
                link.open |= open || (starred && link.defs.is_empty());
                return link;
            }
        }
        // Outside the workspace: the standard library, installed packages, builtins.
        Link::default()
    }

    /// What `name` itself names: its functions, what instantiating its class runs, what it
    /// aliases, and whether a binding of it is a value no name denotes or a decorator's. `None`
    /// where nothing does.
    fn member(&self, name: &Name, hops: u32, followed: &mut Vec<Name>) -> Option<Link> {
        let defs = self.by_name.get(name);
        let aliases = self.aliases.get(name);
        let opaque = self.opaque.contains_key(name);
        if defs.is_none() && aliases.is_none() && !opaque {
            return None;
        }
        let mut link = Link {
            defs: Vec::new(),
            open: opaque,
        };
        let mut class = false;
        for (file, (local, flags)) in defs.into_iter().flatten() {
            link.open |= flags & DECORATED != 0;
            if flags & CLASS == 0 {
                link.defs.push(DefRef {
                    file: *file,
                    local: *local,
                });
            } else {
                class = true;
            }
        }
        if class {
            link.extend(self.instantiate(name, hops + 1, followed));
        }
        if let Some(entries) = aliases {
            let targets = entries.iter().map(|(_, target)| target.clone()).collect();
            link.extend(self.union(targets, hops, followed));
        }
        link.defs.sort_unstable_by_key(|def| self.path_order(*def));
        link.defs.dedup();
        Some(link)
    }

    /// A late-bound name, `pkg.C.m^` (`self.m()`, `cls.m()`) or `pkg.C.^` (`cls()`): the class's
    /// method (instantiation) and every workspace subclass's. Unknown where a class whose base
    /// no name denotes, which may derive from `C`, defines the method.
    fn link_late_python(&self, name: &Name, hops: u32, followed: &mut Vec<Name>) -> Link {
        let Some(class) = name.parent() else {
            return OPEN_LINK;
        };
        let text = name.text();
        let attribute = &text[class.text().len() + 1..text.len() - 1];
        let lookup = |class: &Name, followed: &mut Vec<Name>| {
            if attribute.is_empty() {
                self.instantiate(class, hops + 1, followed)
            } else {
                self.class_member(class, attribute, hops + 1, followed)
            }
        };
        let mut link = lookup(class, followed);
        let mut seen = HashSet::from([class.clone()]);
        let mut pending = vec![class.clone()];
        while let Some(next) = pending.pop() {
            for child in self.subclasses.get(&next).into_iter().flatten() {
                if seen.insert(child.clone()) {
                    link.extend(lookup(child, followed));
                    pending.push(child.clone());
                }
            }
        }
        let methods: &[&str] = if attribute.is_empty() {
            &["__new__", "__init__"]
        } else {
            &[attribute]
        };
        for orphan in &self.orphans {
            followed.push(orphan.clone());
            link.open |= methods.iter().any(|method| {
                Name::get(&format!("{}.{method}", orphan.text())).is_some_and(|member| {
                    self.by_name.contains_key(&member)
                        || self.aliases.contains_key(&member)
                        || self.opaque.contains_key(&member)
                })
            });
        }
        link.defs.sort_unstable_by_key(|def| self.path_order(*def));
        link.defs.dedup();
        link
    }

    /// What calling class `class` runs: `__new__` and `__init__` as its method resolution order
    /// gives them (`object`'s run nothing of the workspace), and a workspace metaclass's
    /// `__call__`.
    fn instantiate(&self, class: &Name, hops: u32, followed: &mut Vec<Name>) -> Link {
        let Some(order) = self.mro(class, hops, followed) else {
            return OPEN_LINK;
        };
        let mut link = Link::default();
        for method in ["__new__", "__init__"] {
            if let Some(found) = self.lookup(&order, method, hops, followed) {
                link.extend(found);
            }
        }
        for entry in &order {
            let Mro::Workspace(class) = entry else {
                continue;
            };
            for (_, (meta, kind)) in self.supers.get(class).into_iter().flatten() {
                if *kind != METACLASS {
                    continue;
                }
                match self.class_of(meta, hops + 1, followed) {
                    Some(Mro::External(_)) => {}
                    Some(Mro::Workspace(meta)) => match self.mro(&meta, hops + 1, followed) {
                        Some(meta) => {
                            if let Some(found) = self.lookup(&meta, "__call__", hops, followed) {
                                link.extend(found);
                            }
                        }
                        None => link.open = true,
                    },
                    None => link.open = true,
                }
            }
        }
        link
    }

    /// Attribute `attribute` of class `class`, as its method resolution order gives it; unknown
    /// where no workspace class of it defines the attribute (an instance's attribute, one set at
    /// run time, another module's class's).
    fn class_member(
        &self,
        class: &Name,
        attribute: &str,
        hops: u32,
        followed: &mut Vec<Name>,
    ) -> Link {
        self.mro(class, hops, followed)
            .and_then(|order| self.lookup(&order, attribute, hops, followed))
            .unwrap_or(OPEN_LINK)
    }

    /// The first class of `order` that defines `attribute`, linked; unknown as well where a class
    /// before it is another module's, which may define it. `None` where no workspace class does.
    fn lookup(
        &self,
        order: &[Mro],
        attribute: &str,
        hops: u32,
        followed: &mut Vec<Name>,
    ) -> Option<Link> {
        let mut uncertain = false;
        for entry in order {
            match entry {
                Mro::Workspace(class) => {
                    let Some(member) = Name::get(&format!("{}.{attribute}", class.text())) else {
                        continue;
                    };
                    if let Some(mut link) = self.member(&member, hops + 1, followed) {
                        followed.push(member);
                        link.open |= uncertain;
                        return Some(link);
                    }
                }
                Mro::External(_) => uncertain = true,
            }
        }
        None
    }

    /// Class `class`'s method resolution order (C3), its workspace classes by name and others'
    /// as `External`. `None` where it is unknown: the name is no one workspace class, a base no
    /// name denotes, an inconsistent order.
    fn mro(&self, class: &Name, hops: u32, followed: &mut Vec<Name>) -> Option<Vec<Mro>> {
        if hops > MAX_HOPS {
            return None;
        }
        followed.push(class.clone());
        let [(_, (_, flags))] = self.by_name.get(class)?.as_slice() else {
            return None;
        };
        if flags & CLASS == 0 {
            return None;
        }
        let mut bases = Vec::new();
        for (_, (target, kind)) in self.supers.get(class).into_iter().flatten() {
            match *kind {
                BASE => bases.push(self.class_of(target, hops + 1, followed)?),
                UNKNOWN_BASE => return None,
                _ => {}
            }
        }
        let mut sequences = Vec::with_capacity(bases.len() + 1);
        for base in &bases {
            sequences.push(match base {
                Mro::Workspace(base) => self.mro(base, hops + 1, followed)?,
                external => vec![external.clone()],
            });
        }
        sequences.push(bases);
        let mut order = vec![Mro::Workspace(class.clone())];
        order.extend(c3(sequences)?);
        Some(order)
    }

    /// The class `name` denotes as a base: a workspace class, through aliases and star imports;
    /// another module's (`External`); `None` where it may be any value.
    fn class_of(&self, name: &Name, hops: u32, followed: &mut Vec<Name>) -> Option<Mro> {
        self.class_noting(name, hops, followed, &mut Vec::new())
    }

    /// `class_of`, noting in `missing` the names a star import was looked through for that no
    /// one has named yet.
    fn class_noting(
        &self,
        name: &Name,
        hops: u32,
        followed: &mut Vec<Name>,
        missing: &mut Vec<Box<str>>,
    ) -> Option<Mro> {
        if hops > MAX_HOPS {
            return None;
        }
        followed.push(name.clone());
        if let Some(entries) = self.by_name.get(name) {
            return entries
                .iter()
                .all(|(_, (_, flags))| flags & CLASS != 0)
                .then(|| Mro::Workspace(name.clone()));
        }
        if self.opaque.contains_key(name) {
            return None;
        }
        let only = |entries: &[(u32, Name)]| {
            let (_, first) = entries.first()?;
            entries
                .iter()
                .all(|(_, target)| target == first)
                .then(|| first.clone())
        };
        if let Some(entries) = self.aliases.get(name) {
            return self.class_noting(&only(entries)?, hops + 1, followed, missing);
        }
        for prefix in name.lineage().skip(1) {
            let rest = &name.text()[prefix.text().len()..];
            if let Some(entries) = self.aliases.get(prefix)
                && let Some(target) = only(entries).filter(|target| target != prefix)
            {
                let renamed = Name::new(&format!("{}{rest}", target.text()));
                return self.class_noting(&renamed, hops + 1, followed, missing);
            }
            if self.by_name.contains_key(prefix) || self.opaque.contains_key(prefix) {
                return None;
            }
            if let Some(entries) = self.modules.get(prefix) {
                // Only a star import binds a workspace module's member no binding gives.
                return entries
                    .iter()
                    .flat_map(|(_, (stars, _))| stars.iter())
                    .find_map(|star| {
                        let text = format!("{}{rest}", star.text());
                        let Some(starred) = Name::get(&text) else {
                            missing.push(text.into_boxed_str());
                            return None;
                        };
                        match self.class_noting(&starred, hops + 1, followed, missing)? {
                            Mro::Workspace(class) => Some(Mro::Workspace(class)),
                            Mro::External(_) => None,
                        }
                    });
            }
        }
        Some(Mro::External(name.clone()))
    }

    /// Python: each class's subclasses and the orphan classes, by text.
    #[cfg(test)]
    pub(crate) fn hierarchy(&self) -> (BTreeMap<String, Vec<String>>, Vec<String>) {
        let text = |names: &[Name]| names.iter().map(|name| name.text().to_owned()).collect();
        (
            self.subclasses
                .iter()
                .map(|(class, children)| (class.text().to_owned(), text(children)))
                .collect(),
            text(&self.orphans),
        )
    }

    /// Brings each workspace class's contribution to `subclasses` and `orphans` up to date: every
    /// class before anything is linked, else those whose lookups read one of `keys`. Returns the
    /// classes whose subclasses changed, and whether the orphans did.
    fn python_classes(&mut self, keys: Option<&HashSet<Name>>) -> (Vec<Name>, bool) {
        let classes = match keys {
            None => {
                self.class_memos.clear();
                self.subclasses.clear();
                self.orphans.clear();
                self.supers.keys().cloned().collect::<Vec<_>>()
            }
            Some(keys) => {
                let reads = reader(keys);
                let mut classes = self
                    .class_memos
                    .iter()
                    .filter(|(class, memo)| {
                        reads(class)
                            || memo.followed.iter().any(&reads)
                            || memo.missing.iter().any(|text| Name::get(text).is_some())
                    })
                    .map(|(class, _)| class.clone())
                    .collect::<HashSet<_>>();
                classes.extend(
                    keys.iter()
                        .filter(|key| self.supers.contains_key(*key))
                        .cloned(),
                );
                classes.into_iter().collect()
            }
        };
        let memos = parallel(&classes, |class| self.python_class(class));
        let mut bases = HashSet::new();
        let mut orphans_changed = false;
        fn by_text(class: &Name) -> impl FnMut(&Name) -> std::cmp::Ordering + '_ {
            move |other| other.text().cmp(class.text())
        }
        for (class, memo) in classes.into_iter().zip(memos) {
            let (new_bases, new_orphan) = memo.as_ref().map_or((Vec::new(), false), |memo| {
                (memo.bases.clone(), memo.orphan)
            });
            let old = match memo {
                Some(memo) => self.class_memos.insert(class.clone(), memo),
                None => self.class_memos.remove(&class),
            };
            let (old_bases, old_orphan) =
                old.map_or((Vec::new(), false), |old| (old.bases, old.orphan));
            for base in old_bases.iter().filter(|base| !new_bases.contains(base)) {
                if let Some(children) = self.subclasses.get_mut(base)
                    && let Ok(at) = children.binary_search_by(by_text(&class))
                {
                    children.remove(at);
                    if children.is_empty() {
                        self.subclasses.remove(base);
                    }
                    bases.insert(base.clone());
                }
            }
            for base in new_bases.iter().filter(|base| !old_bases.contains(base)) {
                let children = self.subclasses.entry(base.clone()).or_default();
                if let Err(at) = children.binary_search_by(by_text(&class)) {
                    children.insert(at, class.clone());
                    bases.insert(base.clone());
                }
            }
            if old_orphan != new_orphan {
                orphans_changed = true;
                match self.orphans.binary_search_by(by_text(&class)) {
                    Ok(at) => {
                        self.orphans.remove(at);
                    }
                    Err(at) => self.orphans.insert(at, class),
                }
            }
        }
        (bases.into_iter().collect(), orphans_changed)
    }

    /// What workspace class `class` contributes to `subclasses` and `orphans`, if it is one.
    fn python_class(&self, class: &Name) -> Option<ClassMemo> {
        let entries = self.supers.get(class)?;
        // A class is keyed by its module's name and by its path's: index it once.
        if class.family() != Family::Python || self.own_name(class).as_ref() != Some(class) {
            return None;
        }
        let mut memo = ClassMemo {
            bases: Vec::new(),
            orphan: false,
            followed: Box::default(),
            missing: Box::default(),
        };
        let (mut followed, mut missing) = (Vec::new(), Vec::new());
        for (_, (target, kind)) in entries {
            match *kind {
                BASE => match self.class_noting(target, 0, &mut followed, &mut missing) {
                    Some(Mro::Workspace(base)) => {
                        if let Some(base) = self.own_name(&base)
                            && !memo.bases.contains(&base)
                        {
                            memo.bases.push(base);
                        }
                    }
                    Some(Mro::External(_)) => {}
                    None => memo.orphan = true,
                },
                UNKNOWN_BASE => memo.orphan = true,
                _ => {}
            }
        }
        memo.followed = followed.into_boxed_slice();
        memo.missing = missing.into_boxed_slice();
        Some(memo)
    }

    /// The name the definition `name` links to gives itself (its module's, not its path's).
    fn own_name(&self, name: &Name) -> Option<Name> {
        let (slot, (local, _)) = self.by_name.get(name)?.first()?;
        let summary = &self.slots[*slot as usize].as_ref()?.summary;
        Some(summary.interned(summary.defs[*local as usize].name).clone())
    }

    fn union(&self, names: Vec<Name>, hops: u32, followed: &mut Vec<Name>) -> Link {
        let mut link = Link::default();
        for name in names {
            let next = self.link_name(&name, hops + 1, followed);
            followed.push(name);
            link.defs.extend(next.defs);
            link.open |= next.open;
        }
        link.defs.sort_unstable_by_key(|def| self.path_order(*def));
        link.defs.dedup();
        link
    }

    /// A definition's place in `link`'s numbering: by file path, then in the file.
    fn path_order(&self, def: DefRef) -> (&str, u32) {
        let file = self.slots[def.file as usize]
            .as_ref()
            .expect("a linked definition's file is linked");
        (&file.path, def.local)
    }
}

/// A link that may be to anything.
const OPEN_LINK: Link = Link {
    defs: Vec::new(),
    open: true,
};

impl Link {
    fn extend(&mut self, other: Link) {
        self.defs.extend(other.defs);
        self.open |= other.open;
    }
}

/// A class in a Python method resolution order: one the workspace defines, by name, or another
/// module's, whose attributes are unknown here.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Mro {
    Workspace(Name),
    External(Name),
}

/// C3's merge of `sequences` (each base's order, then the bases); `None` where no order is
/// consistent, where Python raises `TypeError`.
fn c3(mut sequences: Vec<Vec<Mro>>) -> Option<Vec<Mro>> {
    let mut order = Vec::new();
    loop {
        sequences.retain(|sequence| !sequence.is_empty());
        if sequences.is_empty() {
            return Some(order);
        }
        let head = sequences
            .iter()
            .map(|sequence| &sequence[0])
            .find(|head| {
                !sequences
                    .iter()
                    .any(|sequence| sequence[1..].contains(head))
            })?
            .clone();
        for sequence in &mut sequences {
            if sequence[0] == head {
                sequence.remove(0);
            }
        }
        order.push(head);
    }
}

impl Graph {
    #[cfg(test)]
    /// Every term's closure over `terms` terms. `seed` gives each definition's own answer;
    /// `through` gives, per definition flags, the terms a chain may pass through it for. Each
    /// reaching (definition, term) keeps the successor it first reached through, which chains
    /// follow.
    pub(crate) fn close(
        &self,
        seeds: &[Seed],
        through: impl Fn(u8) -> Bits,
        terms: usize,
    ) -> Closure {
        let dense = self.dense();
        let count = dense.defs.len();
        debug_assert_eq!(seeds.len(), count);
        let allow = dense
            .defs
            .iter()
            .map(|def| through(def.flags))
            .collect::<Vec<_>>();
        let mut reach = vec![0 as Bits; count];
        let mut open = vec![0 as Bits; count];
        let mut next = vec![NO_HOP; count * terms];
        let mut member = vec![u32::MAX; count];
        // The bits of `gained` `def` now reaches through `hop`.
        let record = |next: &mut [u32], def: usize, gained: Bits, hop: u32| {
            for term in 0..terms {
                if gained & (1 << term) != 0 {
                    next[def * terms + term] = hop;
                }
            }
        };
        let components = tarjan(count, |def| dense.successors(def));
        for (index, component) in components.into_iter().enumerate() {
            for def in &component {
                member[*def as usize] = index as u32;
            }
            // Edges leaving the component read final answers; inside it, iterate to a fixpoint.
            for def in &component {
                let d = *def as usize;
                let mut r = seeds[d].matched & allow[d];
                record(&mut next, d, r, SEED_HOP);
                let mut o = seeds[d].unknown | if dense.open[d] { Bits::MAX } else { 0 };
                for to in dense.successors(*def) {
                    if member[*to as usize] != index as u32 {
                        let gained = reach[*to as usize] & allow[d] & !r;
                        record(&mut next, d, gained, *to);
                        r |= gained;
                        o |= open[*to as usize];
                    }
                }
                reach[d] = r;
                open[d] = o & allow[d];
            }
            if component.len() > 1 || dense.successors(component[0]).contains(&component[0]) {
                let mut changed = true;
                while changed {
                    changed = false;
                    for def in &component {
                        let d = *def as usize;
                        let (mut r, mut o) = (reach[d], open[d]);
                        for to in dense.successors(*def) {
                            if member[*to as usize] == index as u32 {
                                let gained = reach[*to as usize] & allow[d] & !r;
                                record(&mut next, d, gained, *to);
                                r |= gained;
                                o |= open[*to as usize] & allow[d];
                            }
                        }
                        if (r, o) != (reach[d], open[d]) {
                            (reach[d], open[d]) = (r, o);
                            changed = true;
                        }
                    }
                }
            }
        }
        for (reach, open) in reach.iter().zip(open.iter_mut()) {
            *open &= !*reach;
        }
        Closure {
            reach,
            open,
            next,
            terms,
        }
    }

    #[cfg(test)]
    /// What `name` links to: its memo where a file calls it, else linked now.
    fn linked<'s>(&'s self, name: &Name, computed: &'s mut Option<Link>) -> (&'s [DefRef], bool) {
        match self.links.get(name) {
            Some(memo) => (&memo.defs, memo.open),
            None => {
                let link = computed.insert(self.link_name(name, 0, &mut Vec::new()));
                (&link.defs, link.open)
            }
        }
    }

    #[cfg(test)]
    /// The answer for a call of `name`.
    pub(crate) fn status(&self, closure: &Closure, name: &Name) -> Status {
        let mut computed = None;
        let (defs, open) = self.linked(name, &mut computed);
        let mut status = Status {
            reach: 0,
            open: if open { Bits::MAX } else { 0 },
        };
        for def in defs {
            let def = self.number_of(*def) as usize;
            status.reach |= closure.reach[def];
            status.open |= closure.open[def];
        }
        status.open &= !status.reach;
        status
    }

    #[cfg(test)]
    /// The qualified names from a call of `name` to a definition matching `term`'s seed, along the
    /// hops the closure kept; empty when the call does not reach it.
    pub(crate) fn chain(&self, closure: &Closure, name: &Name, term: usize) -> Vec<String> {
        let bit = 1 << term;
        let mut computed = None;
        let (defs, _) = self.linked(name, &mut computed);
        let Some(mut at) = defs
            .iter()
            .map(|def| self.number_of(*def))
            .find(|def| closure.reach[*def as usize] & bit != 0)
        else {
            return Vec::new();
        };
        let mut chain = Vec::new();
        loop {
            chain.push(self.def_name(at).to_owned());
            match closure.next[at as usize * closure.terms + term] {
                SEED_HOP | NO_HOP => return chain,
                hop => at = hop,
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn link_is_open(&self, name: &str) -> bool {
        self.link_name(&Name::new(name), 0, &mut Vec::new()).open
    }

    /// Whether `from` calls `to`.
    #[cfg(test)]
    pub(crate) fn calls(&self, from: DefRef, to: DefRef) -> bool {
        self.file_edges(from.file).contains(&(from.local, to))
    }

    /// The dense closure's chain from definition `def`.
    #[cfg(test)]
    pub(crate) fn chain_from(&self, closure: &Closure, def: u32, term: usize) -> Vec<String> {
        let mut chain = Vec::new();
        let mut at = def;
        loop {
            chain.push(self.def_name(at).to_owned());
            match closure.next[at as usize * closure.terms + term] {
                SEED_HOP | NO_HOP => return chain,
                hop => at = hop,
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn def_count(&self) -> usize {
        self.paths
            .values()
            .filter_map(|slot| self.summary_of(*slot))
            .map(|summary| summary.defs.len())
            .sum()
    }

    /// The definition numbered `number`.
    #[cfg(test)]
    pub(crate) fn def_ref(&self, number: u32) -> DefRef {
        self.paths
            .values()
            .map(|&slot| DefRef {
                file: slot,
                local: number.wrapping_sub(self.first[slot as usize]),
            })
            .find(|def| self.holds(*def))
            .expect("a definition has every number below the count")
    }

    #[cfg(test)]
    /// Definition `def`'s file path and index in its summary.
    pub(crate) fn def_file(&self, def: u32) -> (&Arc<str>, &Arc<Summary>, u32) {
        let DefRef { file, local } = self.def_ref(def);
        let file = self.slots[file as usize]
            .as_ref()
            .expect("a numbered definition's file is linked");
        (&file.path, &file.summary, local)
    }

    #[cfg(test)]
    pub(crate) fn def_name(&self, def: u32) -> &str {
        let (_, summary, local) = self.def_file(def);
        summary.name(summary.defs[local as usize].name)
    }
}

#[cfg(test)]
impl Closure {
    /// Definition `def`'s reach and open bits.
    pub(crate) fn bits(&self, def: u32) -> (Bits, Bits) {
        (self.reach[def as usize], self.open[def as usize])
    }
}

#[cfg(test)]
/// Every definition's answer per term.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Closure {
    reach: Vec<Bits>,
    open: Vec<Bits>,
    /// Per (definition, term): the successor it first reached the term's seed through, `SEED_HOP`
    /// where it is a seed itself, `NO_HOP` where it does not reach.
    next: Vec<u32>,
    terms: usize,
}

#[cfg(test)]
/// Iterative Tarjan: components in reverse topological order (a component follows every
/// component it has an edge to).
fn tarjan<'g>(count: usize, successors: impl Fn(u32) -> &'g [u32]) -> Vec<Vec<u32>> {
    const UNSEEN: u32 = u32::MAX;
    let mut index = vec![UNSEEN; count];
    let mut low = vec![0u32; count];
    let mut on_stack = vec![false; count];
    let mut stack = Vec::new();
    let mut components = Vec::new();
    let mut next = 0u32;
    let mut frames: Vec<(u32, usize)> = Vec::new();
    for root in 0..count as u32 {
        if index[root as usize] != UNSEEN {
            continue;
        }
        frames.push((root, 0));
        index[root as usize] = next;
        low[root as usize] = next;
        next += 1;
        stack.push(root);
        on_stack[root as usize] = true;
        while let Some(&mut (node, ref mut edge)) = frames.last_mut() {
            let out = successors(node);
            if let Some(&to) = out.get(*edge) {
                *edge += 1;
                if index[to as usize] == UNSEEN {
                    index[to as usize] = next;
                    low[to as usize] = next;
                    next += 1;
                    stack.push(to);
                    on_stack[to as usize] = true;
                    frames.push((to, 0));
                } else if on_stack[to as usize] {
                    low[node as usize] = low[node as usize].min(index[to as usize]);
                }
                continue;
            }
            frames.pop();
            if let Some(&(parent, _)) = frames.last() {
                low[parent as usize] = low[parent as usize].min(low[node as usize]);
            }
            if low[node as usize] == index[node as usize] {
                let mut component = Vec::new();
                loop {
                    let member = stack.pop().expect("tarjan stack holds the component");
                    on_stack[member as usize] = false;
                    component.push(member);
                    if member == node {
                        break;
                    }
                }
                component.sort_unstable();
                components.push(component);
            }
        }
    }
    components
}
