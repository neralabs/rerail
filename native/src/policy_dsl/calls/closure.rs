//! The workspace call graph: summaries linked by name, and per `calls` term the definitions that
//! reach its seed. A term is a sparse reachability question: a backward walk over callers from its
//! seed definitions, whose cost is the region it reaches, not the graph. The dense closure over
//! strongly connected components (`close`, 64 terms a word) remains as the tests' oracle.

use super::names::Name;
use super::php;
use super::summary::{CLASS, OPEN, Summary};
use crate::policy_dsl::language::Family;
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

/// One linked file: its summary, and its definitions' resolved call edges.
#[derive(Clone, Debug)]
struct GraphFile {
    path: Arc<str>,
    summary: Arc<Summary>,
    /// `(definition, callee definition)`, sorted.
    edges: Vec<(u32, DefRef)>,
    /// Definitions that call something open.
    open: Vec<u32>,
}

/// A module's entry: the modules it `import *`s, and whether its summary is open (a syntax error
/// may hide a definition).
type Module = (Arc<[Name]>, bool);

/// What one file puts in the name tables, under its module's names.
#[derive(Default)]
struct Contribution {
    defs: HashMap<Name, Vec<(u32, u8)>>,
    aliases: HashMap<Name, Vec<Name>>,
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
            let (key, target) = (summary.interned(*key), summary.interned(*target));
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
                    .push(key.clone());
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

type Staged = (Arc<str>, u32, Option<Arc<Summary>>, Option<Arc<Summary>>);
/// A file's resolved edges, and its definitions calling something open.
type FileEdges = (Vec<(u32, DefRef)>, Vec<u32>);

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
    modules: Entries<Module>,
    /// PHP: class-likes' parents, traits and interfaces, and trait adaptations, by lowercased
    /// class (or `class.method`) name.
    supers: Entries<(Name, u8)>,
    /// PHP: the classes that extend each class, by lowercased name.
    children: Entries<Name>,
    links: HashMap<Name, Memo>,
    /// Per slot, its first definition's number.
    first: Vec<u32>,
    /// Per definition: its file's slot, index in the file, and flags.
    defs: Vec<GraphDef>,
    /// Call edges, per definition: `edges[offsets[d]..offsets[d + 1]]`.
    offsets: Vec<u32>,
    edges: Vec<u32>,
    /// The same edges reversed, per callee: `callers[caller_offsets[d]..caller_offsets[d + 1]]`,
    /// in number order.
    caller_offsets: Vec<u32>,
    callers: Vec<u32>,
    /// Definitions that call something unknown, or whose file is open.
    open: Vec<bool>,
}

#[derive(Clone, Copy, Debug)]
struct GraphDef {
    file: u32,
    local: u32,
    flags: u8,
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
    /// Returns the slots whose definitions or edges may have changed: those replaced or removed,
    /// and those calling a name whose link changed.
    pub(crate) fn update(
        &mut self,
        changes: impl IntoIterator<Item = (Arc<str>, Option<Arc<Summary>>)>,
    ) -> Vec<u32> {
        let mut orphans = Vec::new();
        // Each change's slot, and its file's old and new summary.
        let mut staged = Vec::new();
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
            staged.push((path, slot, old.map(|old| old.summary), summary));
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
        let mut removed = Vec::new();
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
                    self.slots[slot as usize] = Some(GraphFile {
                        path,
                        summary,
                        edges: Vec::new(),
                        open: Vec::new(),
                    });
                    replaced.push(slot);
                }
                None => {
                    self.paths.remove(&path);
                    self.free.push(slot);
                    removed.push(slot);
                }
            }
        }
        // A name links through its own entries, its prefixes', and those of the names it
        // followed; a class links through its `__init__`.
        let mut keys = keys.unwrap_or_default();
        let classes = keys
            .iter()
            .filter(|key| key.text().ends_with(".__init__"))
            .filter_map(|key| key.parent().cloned())
            .collect::<Vec<_>>();
        keys.extend(classes);
        let reads = |name: &Name| name.lineage().any(|name| keys.contains(name));
        let stale = if keys.is_empty() {
            Vec::new()
        } else {
            self.links
                .iter()
                .filter(|(name, memo)| reads(name) || memo.followed.iter().any(reads))
                .map(|(name, _)| name.clone())
                .collect::<Vec<_>>()
        };
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
            memo.set(link, followed);
            self.links.insert(name, memo);
        }
        self.link_calls(&replaced);
        // Files whose callee links changed resolve their edges again; they call what they did.
        relinked.extend(replaced);
        let mut relinked = relinked.into_iter().collect::<Vec<_>>();
        let resolved = parallel(&relinked, |slot| self.resolve_edges(*slot));
        for (&slot, resolved) in relinked.iter().zip(resolved) {
            if let (Some(file), Some((edges, open))) = (&mut self.slots[slot as usize], resolved) {
                (file.edges, file.open) = (edges, open);
            }
        }
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
        self.number();
        relinked.extend(removed);
        relinked.sort_unstable();
        relinked.dedup();
        relinked
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

    /// Numbers definitions by file path and lays out their edges.
    fn number(&mut self) {
        self.first = vec![0; self.slots.len()];
        self.defs.clear();
        for &slot in self.paths.values() {
            self.first[slot as usize] = self.defs.len() as u32;
            let file = self.slots[slot as usize]
                .as_ref()
                .expect("a path's file is linked");
            self.defs.extend(
                file.summary
                    .defs
                    .iter()
                    .enumerate()
                    .map(|(local, def)| GraphDef {
                        file: slot,
                        local: local as u32,
                        flags: def.flags,
                    }),
            );
        }
        self.open = self.defs.iter().map(|def| def.flags & OPEN != 0).collect();
        self.offsets = vec![0; self.defs.len() + 1];
        self.edges.clear();
        let mut targets = Vec::new();
        for &slot in self.paths.values() {
            let file = self.slots[slot as usize]
                .as_ref()
                .expect("a path's file is linked");
            let base = self.first[slot as usize];
            if file.summary.open {
                let end = base as usize + file.summary.defs.len();
                self.open[base as usize..end].fill(true);
            }
            for owner in &file.open {
                self.open[(base + owner) as usize] = true;
            }
            let mut edges = file.edges.iter().peekable();
            for owner in 0..file.summary.defs.len() as u32 {
                targets.clear();
                while let Some((_, to)) = edges.next_if(|(from, _)| *from == owner) {
                    targets.push(self.first[to.file as usize] + to.local);
                }
                targets.sort_unstable();
                self.edges.extend_from_slice(&targets);
                self.offsets[(base + owner) as usize + 1] = self.edges.len() as u32;
            }
        }
        // Callers per callee, in number order: counted, then filled caller by caller.
        let mut starts = vec![0u32; self.defs.len() + 1];
        for &to in &self.edges {
            starts[to as usize + 1] += 1;
        }
        for index in 1..starts.len() {
            starts[index] += starts[index - 1];
        }
        let mut fill = starts.clone();
        let mut callers = vec![0; self.edges.len()];
        for from in 0..self.defs.len() as u32 {
            for &to in self.successors(from) {
                callers[fill[to as usize] as usize] = from;
                fill[to as usize] += 1;
            }
        }
        self.callers = callers;
        self.caller_offsets = starts;
    }

    #[cfg(test)]
    fn number_of(&self, def: DefRef) -> u32 {
        self.first[def.file as usize] + def.local
    }

    /// `def`'s number, while its file is linked and holds it.
    fn numbered(&self, def: DefRef) -> Option<u32> {
        let file = self.slots.get(def.file as usize)?.as_ref()?;
        (def.local < file.summary.defs.len() as u32)
            .then(|| self.first[def.file as usize] + def.local)
    }

    pub(crate) fn def_ref(&self, number: u32) -> DefRef {
        let GraphDef { file, local, .. } = self.defs[number as usize];
        DefRef { file, local }
    }

    fn callers_of(&self, def: u32) -> &[u32] {
        let range = self.caller_offsets[def as usize]..self.caller_offsets[def as usize + 1];
        &self.callers[range.start as usize..range.end as usize]
    }

    /// The definitions `allow` lets a chain pass that reach one of `seeds` through definitions it
    /// lets pass, each with its hop: a backward walk over callers, whose cost is the region it
    /// finds. Breadth-first from the seeds and over callers in number order, so each hop is the
    /// first callee of a shortest chain, whatever order files were linked in.
    pub(crate) fn reach(&self, seeds: &[DefRef], allow: impl Fn(u8) -> bool) -> Region {
        let mut starts = seeds
            .iter()
            .filter_map(|def| self.numbered(*def))
            .filter(|def| allow(self.defs[*def as usize].flags))
            .collect::<Vec<_>>();
        starts.sort_unstable();
        starts.dedup();
        let mut region = Region::with_capacity(starts.len());
        let mut queue = std::collections::VecDeque::with_capacity(starts.len());
        for start in starts {
            region.insert(self.def_ref(start), Hop::Seed);
            queue.push_back(start);
        }
        while let Some(def) = queue.pop_front() {
            let callee = self.def_ref(def);
            for &caller in self.callers_of(def) {
                if !allow(self.defs[caller as usize].flags) {
                    continue;
                }
                if let std::collections::hash_map::Entry::Vacant(entry) =
                    region.entry(self.def_ref(caller))
                {
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
        sources: impl IntoIterator<Item = u32>,
        allow: impl Fn(u8) -> bool,
    ) -> HashSet<DefRef> {
        let mut found = HashSet::new();
        let mut stack = sources
            .into_iter()
            .filter(|def| allow(self.defs[*def as usize].flags))
            .collect::<Vec<_>>();
        stack.retain(|def| found.insert(self.def_ref(*def)));
        while let Some(def) = stack.pop() {
            for &caller in self.callers_of(def) {
                if allow(self.defs[caller as usize].flags) && found.insert(self.def_ref(caller)) {
                    stack.push(caller);
                }
            }
        }
        found
    }

    /// The definitions that call something unknown, or whose file is open.
    pub(crate) fn open_defs(&self) -> impl Iterator<Item = u32> + '_ {
        (0..self.defs.len() as u32).filter(|def| self.open[*def as usize])
    }

    /// The numbers of `slot`'s definitions.
    pub(crate) fn file_defs(&self, slot: u32) -> std::ops::Range<u32> {
        match self.slots.get(slot as usize).and_then(Option::as_ref) {
            Some(file) => {
                let first = self.first[slot as usize];
                first..first + file.summary.defs.len() as u32
            }
            None => 0..0,
        }
    }

    /// The files calling one of `defs`.
    pub(crate) fn calling(&self, defs: &HashSet<DefRef>) -> Vec<Arc<str>> {
        let calls = |file: &&GraphFile| {
            file.summary.targets.iter().any(|target| {
                self.links
                    .get(file.summary.interned(*target))
                    .is_some_and(|memo| memo.defs.iter().any(|def| defs.contains(def)))
            })
        };
        self.slots
            .iter()
            .flatten()
            .filter(calls)
            .map(|file| Arc::clone(&file.path))
            .collect()
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
            defs: self.defs.len(),
            edges: self.edges.len(),
            open_defs: self.open.iter().filter(|open| **open).count(),
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

    #[cfg(test)]
    /// Strongly connected components in reverse topological order: callees first.
    pub(crate) fn components(&self) -> Vec<Vec<u32>> {
        tarjan(self.defs.len(), |def| self.successors(def))
    }

    fn successors(&self, def: u32) -> &[u32] {
        &self.edges[self.offsets[def as usize] as usize..self.offsets[def as usize + 1] as usize]
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
    fn flags(&self, def: DefRef) -> u8 {
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

    fn link_python(&self, name: &Name, hops: u32, followed: &mut Vec<Name>) -> Link {
        if let Some(entries) = self.by_name.get(name) {
            let mut defs = Vec::new();
            for (file, (local, flags)) in entries {
                let def = DefRef {
                    file: *file,
                    local: *local,
                };
                self.callable(def, *flags, name, &mut defs);
            }
            defs.sort_unstable_by_key(|def| self.path_order(*def));
            return Link { defs, open: false };
        }
        if let Some(entries) = self.aliases.get(name) {
            let targets = entries.iter().map(|(_, target)| target.clone());
            return self.union(targets.collect(), hops, followed);
        }
        // The longest prefix something names: an alias to follow, a definition whose attribute
        // this is, or a module whose member this is.
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
            if self.by_name.contains_key(prefix) {
                // An attribute of a function or class no definition names: an inherited method or
                // an attribute set at run time. The static graph has no edge for it.
                return Link::default();
            }
            if let Some(entries) = self.modules.get(prefix) {
                // A member of a workspace module that neither a definition nor an alias gives: a
                // value the module assigns (its call has no static callee, like `obj.m()`), or a
                // submodule's. Only a star import can bind it to a definition, and only an open
                // summary can hide one.
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

    /// What calling definition `def` (named `name`) runs: the function itself, or a class's
    /// `__init__`.
    fn callable(&self, def: DefRef, flags: u8, name: &Name, defs: &mut Vec<DefRef>) {
        if flags & CLASS == 0 {
            defs.push(def);
            return;
        }
        let Some(init) = Name::get(&format!("{}.__init__", name.text())) else {
            return;
        };
        if let Some(entries) = self.by_name.get(&init) {
            defs.extend(
                entries
                    .iter()
                    .filter(|(_, (_, flags))| flags & CLASS == 0)
                    .map(|(file, (local, _))| DefRef {
                        file: *file,
                        local: *local,
                    }),
            );
        }
    }

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
        let count = self.defs.len();
        debug_assert_eq!(seeds.len(), count);
        let allow = self
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
        for (index, component) in self.components().into_iter().enumerate() {
            for def in &component {
                member[*def as usize] = index as u32;
            }
            // Edges leaving the component read final answers; inside it, iterate to a fixpoint.
            for def in &component {
                let d = *def as usize;
                let mut r = seeds[d].matched & allow[d];
                record(&mut next, d, r, SEED_HOP);
                let mut o = seeds[d].unknown | if self.open[d] { Bits::MAX } else { 0 };
                for to in self.successors(*def) {
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
            if component.len() > 1 || self.successors(component[0]).contains(&component[0]) {
                let mut changed = true;
                while changed {
                    changed = false;
                    for def in &component {
                        let d = *def as usize;
                        let (mut r, mut o) = (reach[d], open[d]);
                        for to in self.successors(*def) {
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
        let to = self.number_of(to);
        self.successors(self.number_of(from)).contains(&to)
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
        self.defs.len()
    }

    #[cfg(test)]
    /// Definition `def`'s file path and index in its summary.
    pub(crate) fn def_file(&self, def: u32) -> (&Arc<str>, &Arc<Summary>, u32) {
        let GraphDef { file, local, .. } = self.defs[def as usize];
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
