//! A batch's documents and rows, each made when first read (decision 0474).
//!
//! A batch holds its documents unmade. A document's projection (its rank index, which the views read) is made on its
//! first read (`Docs`). Its rows, the relations the batch makes off the tapes (`text`, `op`, the matchers'), are made
//! in a group with the other documents the read needs (`Rows`), numbered after the rows of the groups made before it.
//! A batch read whole from its first read makes one group, in key order: one relation per relation, as an eager batch.

use super::*;
#[cfg(target_os = "macos")]
use crate::code_rule_dsl::facts::vtab::DeviceView;
use crate::code_rule_dsl::facts::vtab::{ReadRow, RowCells, RowStream};
use crate::code_rule_dsl::logic::profile;
use std::ops::Range;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A document's views' counts (decision 0474): its nodes, fielded nodes, siblings after it and places, as its
/// projection has them; the documents' table keeps them.
pub type ViewCounts = [u32; 4];

/// A projection's views' counts.
pub(super) fn view_counts(p: &Projected) -> ViewCounts {
    [p.len(), p.fielded(), p.after_siblings(), p.locs()]
}

/// A batch's documents (decision 0474): each one's projection made on its first read; its key and tag known before.
pub struct Docs {
    slots: Vec<Slot>,
    keys: Vec<u32>,
    tags: Vec<u8>,
    /// Each document's views' counts, where given (the documents' table's).
    given: Vec<Option<ViewCounts>>,
    context: Context,
    /// How many projections are made (R2's tests read it).
    made: AtomicUsize,
}

struct Slot {
    input: Document,
    /// Shared with the batch it was built again from (`Batch::take_groups`), where made there.
    made: OnceLock<Arc<MadeDoc>>,
}

/// A made document: its projection, whether the policy is withheld it (it does not parse), and its first error.
pub(super) struct MadeDoc {
    pub(super) p: Projected,
    pub(super) withheld: bool,
    pub(super) error: Option<(u32, String)>,
}

/// What a document's projection is made with.
pub(super) struct Context {
    pub(super) dialects: Arc<Dialects>,
    pub(super) projection: Arc<Projection>,
    pub(super) block: Arc<Block>,
    /// Each guest's host.
    pub(super) host: HashMap<u32, u32>,
    /// The first failure making a document or a group (a verdict raised, too many positions): every source of the
    /// batch reports it (`Source::failure`), so the statement reading it fails rather than read the rows as absent.
    pub(super) failure: parking_lot::Mutex<Option<String>>,
}

impl Docs {
    /// `documents`, sorted by key and distinct, with the views' counts `given` of some, by key.
    pub(super) fn new(documents: Vec<Document>, context: Context, given: &HashMap<u32, ViewCounts>) -> Self {
        let keys: Vec<u32> = documents.iter().map(|d| d.key).collect();
        let tags = documents.iter().map(|d| crate::code_rule_dsl::facts::core::facts_tag(&d.facts)).collect();
        let given = keys.iter().map(|k| given.get(k).copied()).collect();
        let slots = documents.into_iter().map(|input| Slot { input, made: OnceLock::new() }).collect();
        Self { slots, keys, tags, given, context, made: AtomicUsize::new(0) }
    }
    /// Whether every document's views' counts were given: the views' row space is known with none made.
    pub fn given_all(&self) -> bool {
        !self.given.is_empty() && self.given.iter().all(Option::is_some)
    }
    /// Document `i`'s views' counts: those given, else its projection's (made if it is not).
    pub fn counts(&self, i: usize) -> ViewCounts {
        match (self.slots[i].made.get(), self.given[i]) {
            (Some(m), _) => view_counts(&m.p),
            (None, Some(c)) => c,
            (None, None) => view_counts(&self.made(i).p),
        }
    }
    pub fn len(&self) -> usize {
        self.slots.len()
    }
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }
    /// Document `i`'s key, made or not.
    pub fn key(&self, i: usize) -> u32 {
        self.keys[i]
    }
    pub fn keys(&self) -> &[u32] {
        &self.keys
    }
    /// Document `i`'s tape tag (`Facts::tag`), made or not.
    pub fn tag(&self, i: usize) -> u8 {
        self.tags[i]
    }
    /// The index of the document of key `key`.
    pub fn position(&self, key: u32) -> Option<usize> {
        self.keys.binary_search(&key).ok()
    }
    /// What document `i` is made from: its facts, tape, path and place.
    pub fn input(&self, i: usize) -> &Document {
        &self.slots[i].input
    }
    /// Document `i`'s projection, made if it is not.
    pub fn get(&self, i: usize) -> Option<&Projected> {
        (i < self.slots.len()).then(|| &self.made(i).p)
    }
    /// Every document's projection, in key order, each made as it is reached: one passed over (`skip`, `nth`) is
    /// not made.
    pub fn iter(&self) -> DocsIter<'_> {
        DocsIter { docs: self, at: 0..self.slots.len() }
    }
    /// Whether document `i`'s projection is made.
    pub fn is_made(&self, i: usize) -> bool {
        self.slots[i].made.get().is_some()
    }
    /// How many projections are made.
    pub fn made_count(&self) -> usize {
        self.made.load(Ordering::Relaxed)
    }
    /// The first failure making a document, if one failed.
    pub fn failure(&self) -> Option<String> {
        self.context.failure.lock().clone()
    }
    pub(super) fn fail(&self, why: String) {
        self.context.failure.lock().get_or_insert(why);
    }
    /// The address the read hints are keyed by (`Batch::doc`).
    pub(super) fn addr(&self) -> usize {
        self.slots.as_ptr() as usize
    }
    /// Document `i`, made if it is not.
    pub(super) fn made(&self, i: usize) -> &MadeDoc {
        self.slots[i].made.get_or_init(|| {
            self.made.fetch_add(1, Ordering::Relaxed);
            let made = self.project(i);
            // Its counts as given (the documents' table's): the views' rows were placed by them.
            if let Some(given) = self.given[i]
                && given != view_counts(&made.p)
            {
                self.fail(format!(
                    "{}: its views' counts (nodes, fielded, siblings, places) are {:?}, the documents' table gave {given:?} (decision 0474)",
                    self.slots[i].input.path,
                    view_counts(&made.p)
                ));
            }
            Arc::new(made)
        })
    }
    /// The first `n` documents' projections `from` made, shared: they are made in its block, which this batch's
    /// projections are made in too (`Batch::take_groups`).
    fn take_made(&self, from: &Docs, n: usize) {
        for i in 0..n {
            if let Some(m) = from.slots[i].made.get() {
                let _ = self.slots[i].made.set(Arc::clone(m));
            }
        }
    }
    /// Document `i`'s projection (decision 0289 §3: in the batch's block), its verdict and its module.
    fn project(&self, i: usize) -> MadeDoc {
        let d = &self.slots[i].input;
        let projection = &*self.context.projection;
        let shape = projection.shape;
        let core = shape == Shape::Core;
        let tag = self.tags[i];
        // A text-only document has no dialect, and no node.
        let dialect = if tag == TEXT { None } else { self.context.dialects.get(tag) };
        // `core.rr`: a document that does not parse is withheld from the policy (no facts) but its `doc`, path and
        // `syntax_error` rows; its outcome is Unknown(parse). T's shape keeps every document. A guest that does not
        // parse is withheld too, unless the policy keeps its partial tree (decision 0055 §6).
        let guest = d.guest.as_deref().filter(|_| core);
        let error = match dialect {
            Some(dialect) if (core && (projection.wants("syntax_error") || guest.is_some())) || (!core && projection.verdict) => {
                core_rr::verdict(&d.facts, dialect).unwrap_or_else(|raised| {
                    self.fail(core_rr::failure(&d.path, &raised).to_string());
                    None
                })
            }
            _ => None,
        };
        let withheld = core && error.is_some() && !guest.is_some_and(|g| projection.keeps_partial(&g.grammar));
        let python_lines = dialect.is_some_and(|d| d.family == Family::Python);
        // A guest's nodes have their places in its file: the file's lines, where the batch holds the file.
        let placed = guest.map(|g| (g, self.file_lines(g.host).unwrap_or_else(|| Arc::new(core_rr::Lines::shared(Arc::clone(&d.facts.source), python_lines, &self.context.block)))));
        let new_span = profile::span(|| "projected".into());
        let mut p = Projected::narrowed(d.key, Arc::clone(&d.facts), dialect, shape, withheld, placed, projection.node_anchors.as_ref().filter(|_| core), &self.context.block);
        drop(new_span);
        // A program that reads `end_point` has each node's end point among the positions, after the lines.
        let nodes = p.len();
        if core && projection.wants("end_point")
            && let Some(x) = p.extra.as_mut()
        {
            x.ends = nodes;
        }
        p.module = crate::code_rule_dsl::module_path::ModulePath::dotted(d.module.as_deref().unwrap_or(&module(&d.path)));
        // Its positions (tokens, text, lines) in the top slots (`granules::span_low`): past them ids would clash.
        if p.defs.is_some() && u64::from(p.locs() - p.len()) > granules::POSITIONS {
            self.fail(format!("{} has more than {} positions (tokens and lines: core.rr's span ids)", d.path, granules::POSITIONS));
        }
        MadeDoc { p, withheld, error }
    }
    /// The lines of the file a guest of `host` is in (through guests of guests), where the batch holds that file in
    /// core.rr's shape.
    fn file_lines(&self, host: u32) -> Option<Arc<core_rr::Lines>> {
        let mut k = host;
        for _ in 0..=self.context.host.len() {
            match self.context.host.get(&k) {
                Some(&h) => k = h,
                None => break,
            }
        }
        let i = self.position(k)?;
        if self.slots[i].input.guest.is_some() {
            return None;
        }
        self.made(i).p.extra.as_ref().map(|x| Arc::clone(&x.lines))
    }
}

impl std::ops::Index<usize> for Docs {
    type Output = Projected;
    fn index(&self, i: usize) -> &Projected {
        &self.made(i).p
    }
}

impl<'a> IntoIterator for &'a Docs {
    type Item = &'a Projected;
    type IntoIter = DocsIter<'a>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// `Docs::iter`: each document's projection made as it is reached, none passed over.
#[derive(Clone)]
pub struct DocsIter<'a> {
    docs: &'a Docs,
    at: Range<usize>,
}

impl<'a> Iterator for DocsIter<'a> {
    type Item = &'a Projected;
    fn next(&mut self) -> Option<&'a Projected> {
        self.at.next().map(|i| &self.docs.made(i).p)
    }
    fn nth(&mut self, n: usize) -> Option<&'a Projected> {
        self.at.nth(n).map(|i| &self.docs.made(i).p)
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.at.size_hint()
    }
}

impl DoubleEndedIterator for DocsIter<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.at.next_back().map(|i| &self.docs.made(i).p)
    }
}

impl ExactSizeIterator for DocsIter<'_> {}

/// The views' row offsets by document: each made from the documents' counts on its first read.
#[derive(Default)]
pub(super) struct Offsets {
    pub(super) nodes: OnceLock<Vec<usize>>,
    pub(super) children: OnceLock<Vec<usize>>,
    pub(super) fields: OnceLock<Vec<usize>>,
    pub(super) siblings: OnceLock<Vec<usize>>,
    pub(super) locs: OnceLock<Vec<usize>>,
}

/// What a group made: the rows `Batch::make_rows` made for some documents, before they are bound.
pub(super) struct Made {
    pub(super) relations: Vec<(&'static str, Relation)>,
    /// The `match_all` relations (`logic::strings::as_relations`).
    pub(super) matched: Vec<(String, Relation)>,
    /// The `ast` matchers' relations (decision 0106).
    pub(super) waved: Vec<(String, Arc<dyn Source>)>,
    /// The relations made in place of views (`Batch::made`).
    pub(super) read: BTreeSet<&'static str>,
    /// The group's distinct strings and their symbols: its vocabulary, which the text tests read (decision 0037).
    pub(super) strings: Vec<String>,
    pub(super) symbols: Vec<u32>,
}

/// A group: documents whose rows were made together, and each relation's rows of them.
pub(super) struct Group {
    pub(super) docs: Vec<usize>,
    /// Each relation's rows, in `Rows::names`' order.
    pub(super) rels: Vec<Arc<dyn Source>>,
    /// Each relation's rows before this group's: a row's id is its base and its place in the group.
    pub(super) bases: Vec<usize>,
    pub(super) strings: Vec<String>,
    pub(super) symbols: Vec<u32>,
}

/// The batch's rows: the groups made, and the relations a group makes.
pub(super) struct Rows {
    state: parking_lot::Mutex<State>,
    /// The relations a group makes, by name: what an empty group makes, before any document is (`Batch::new`).
    pub(super) names: Vec<String>,
    /// An empty group's relations, which answer what a relation is (its columns, its indexes) before its rows are
    /// made.
    pub(super) template: Vec<Arc<dyn Source>>,
    /// How each relation's first column names its row's document.
    pub(super) by: Vec<ByDocument>,
    /// How many of `names` are the batch's relations (`Made::relations`), which a program's derived relations replace;
    /// the `match_all` and the matchers' follow.
    pub(super) relations: usize,
    /// The batch's relations' names as the producers name them (`Made::relations`).
    pub(super) statics: Vec<&'static str>,
    /// Every document's rows, once every one is made.
    complete: OnceLock<Complete>,
    /// Each relation's rows as its kept statistics count them (`Batch::give_totals`), where given.
    counted: OnceLock<Vec<Option<usize>>>,
}

#[derive(Default)]
struct State {
    groups: Vec<Arc<Group>>,
    /// Each document's group.
    of: Vec<Option<u32>>,
    /// Each relation's rows made.
    totals: Vec<usize>,
}

/// Every document's rows: one group's relation, or the groups' read through their bases (`Concat`).
struct Complete {
    groups: Vec<Arc<Group>>,
    concat: Vec<Concat>,
}

/// How a relation's row names its document: by its first column, as the rule library's declarations type it
/// (`docs/rule-syntax/lib`). There a first column `n` is always a `node` or a `loc` (an id, `d << 32 | …`) and `d`
/// or `g` always a document's key; other names are ambiguous (`p` is `token`'s node and `ast_match`'s pattern), so
/// such a relation is not routed: any read of it makes every document. A group's rows of a routed relation name its
/// own documents there, so a probe of that column makes the documents it names alone; a group whose rows name
/// another fails, named.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ByDocument {
    /// The document's key (`doc(d, …)`, `guest(g, …)`), in this column.
    Key(usize),
    /// A node's or a span's id, `d << 32 | …`, in this column.
    Id(usize),
    /// Not routed.
    No,
}

impl ByDocument {
    fn of(columns: &[String]) -> Self {
        match columns.first().map(String::as_str) {
            Some("d" | "g") => Self::Key(0),
            Some("n") => Self::Id(0),
            _ => Self::No,
        }
    }
    /// The column that names the document.
    fn column(self) -> Option<usize> {
        match self {
            Self::Key(c) | Self::Id(c) => Some(c),
            Self::No => None,
        }
    }
    /// The key of the document `value` (of `column`) names, if it names one.
    fn document(self, value: i64) -> Option<u32> {
        match self {
            Self::Key(_) => u32::try_from(value).ok(),
            Self::Id(_) => u32::try_from(value >> 32).ok().filter(|_| value >= 0),
            Self::No => None,
        }
    }
}

impl Rows {
    /// Each relation's rows, by name, as its kept statistics count them: given once, before a relation is read.
    pub(super) fn give_totals(&self, totals: &HashMap<String, usize>) {
        let _ = self.counted.set(self.names.iter().map(|n| totals.get(n).copied()).collect());
    }
    /// Relation `rel`'s rows as given, if they were.
    fn counted(&self, rel: usize) -> Option<usize> {
        self.counted.get().and_then(|c| c[rel])
    }
    pub(super) fn new(documents: usize, names: Vec<String>, template: Vec<Arc<dyn Source>>, statics: Vec<&'static str>) -> Self {
        let state = State { groups: Vec::new(), of: vec![None; documents], totals: vec![0; names.len()] };
        let by = template.iter().map(|t| ByDocument::of(&t.columns())).collect();
        Self { state: parking_lot::Mutex::new(state), names, template, by, relations: statics.len(), statics, complete: OnceLock::new(), counted: OnceLock::new() }
    }
    /// The groups made.
    pub(super) fn groups(&self) -> usize {
        self.state.lock().groups.len()
    }
}

impl Batch {
    /// The rows of `want`'s documents made: those not made yet, as one group (decision 0474). A group is made
    /// unlocked (its prefilter and matchers run on the pool, where a waiting thread runs other jobs, one of which may
    /// read this batch), and published only if no other read made one of its documents meanwhile; else it is made
    /// again of those still unmade.
    pub(super) fn ensure(&self, want: &[usize]) {
        loop {
            let need: Vec<usize> = {
                let s = self.rows.state.lock();
                let mut need: Vec<usize> = want.iter().copied().filter(|&i| s.of[i].is_none()).collect();
                need.sort_unstable();
                need.dedup();
                need
            };
            if need.is_empty() {
                return;
            }
            let made = self.make_rows(&need).and_then(|m| self.group_rels(m, &need));
            let mut s = self.rows.state.lock();
            if need.iter().any(|&i| s.of[i].is_some()) {
                continue;
            }
            let (rels, strings, symbols) = match made {
                Ok(made) => made,
                Err(why) => {
                    // Its documents have no rows, and every source of the batch reports why.
                    self.docs.fail(why.to_string());
                    let empty = self.rows.template.iter().map(|t| Arc::new(Relation::new(&t.columns().iter().map(String::as_str).collect::<Vec<_>>())) as Arc<dyn Source>).collect();
                    (empty, Vec::new(), Vec::new())
                }
            };
            let bases = s.totals.clone();
            for (total, r) in s.totals.iter_mut().zip(&rels) {
                *total += r.len();
            }
            let at = s.groups.len() as u32;
            for &i in &need {
                s.of[i] = Some(at);
            }
            profile::decision("a batch's documents made by group (decision 0474)", || format!("{} of {} documents", need.len(), self.docs.len()));
            s.groups.push(Arc::new(Group { docs: need, rels, bases, strings, symbols }));
            return;
        }
    }

    /// `from`'s groups taken as this batch's own (Q355), before any read of it: a batch built again with more
    /// documents (a chunk's guests, decision 0055) has the rows its documents were made in the batch before, shared,
    /// not made again. Only where `from`'s documents are this batch's first, the same documents in the same places,
    /// and its rows are made as this batch's are (the projection, the regexes, the markers, the matchers); a document's
    /// rows name it alone (`group_rels`), so they do not depend on the documents beside it. Its projections are
    /// shared too where this batch's block is `from`'s (`grown`): a batch's projections are in its block (decision
    /// 0289 §3). Whether they were taken.
    pub(super) fn take_groups(&self, from: &Batch) -> bool {
        let n = from.docs.len();
        let same_documents = n <= self.docs.len() && (0..n).all(|i| from.docs.key(i) == self.docs.key(i) && from.docs.input(i).digest == self.docs.input(i).digest);
        let same_making = *from.projection == *self.projection
            && from.metal == self.metal
            && from.markers == self.markers
            && from.matches.len() == self.matches.len()
            && from.matches.iter().zip(&self.matches).all(|(a, b)| a.as_str() == b.as_str())
            && match (&from.wave, &self.wave) {
                (None, None) => true,
                (Some((a, x)), Some((b, y))) => a.same(b) && x == y,
                _ => false,
            }
            && from.rows.names == self.rows.names;
        if !same_documents || !same_making || from.docs.failure().is_some() {
            return false;
        }
        let taken = from.rows.state.lock();
        let mut s = self.rows.state.lock();
        if !s.groups.is_empty() {
            return false;
        }
        s.groups = taken.groups.clone();
        // copy: not block data (each document's group number, the batch's bookkeeping; the groups' rows are shared).
        s.of[..n].copy_from_slice(&taken.of[..n]);
        s.totals = taken.totals.clone();
        if Arc::ptr_eq(&from.block, &self.block) {
            self.docs.take_made(&from.docs, n);
        }
        true
    }

    /// A group's rows as its relations, in `Rows::names`' order: it makes each the batch names and no other, and
    /// (but `NOT_BY_DOCUMENT`'s) each row's first column names one of its documents.
    fn group_rels(&self, made: Made, docs: &[usize]) -> StoreResult<(Vec<Arc<dyn Source>>, Vec<String>, Vec<u32>)> {
        let Made { relations, matched, waved, read: _, strings, symbols } = made;
        let named: Vec<(String, Arc<dyn Source>)> = relations
            .into_iter()
            .map(|(name, r)| (name.to_owned(), Arc::new(r) as Arc<dyn Source>))
            .chain(matched.into_iter().map(|(name, r)| (name, Arc::new(r) as Arc<dyn Source>)))
            .chain(waved)
            .collect();
        let names: Vec<&str> = named.iter().map(|(n, _)| n.as_str()).collect();
        if names != self.rows.names.iter().map(String::as_str).collect::<Vec<_>>() {
            return Err(StoreError::Invalid(format!("a group of a batch made the relations {names:?}, not the batch's {:?} (decision 0474)", self.rows.names)));
        }
        let keys: BTreeSet<u32> = docs.iter().map(|&i| self.docs.key(i)).collect();
        for ((name, r), &by) in named.iter().zip(&self.rows.by) {
            let Some(c) = by.column() else { continue };
            if let Some(row) = (0..r.len()).find(|&row| !by.document(r.value(row, c)).is_some_and(|k| keys.contains(&k))) {
                return Err(StoreError::Invalid(format!(
                    "{name}: a row's column {c} ({}) names a document outside its group {keys:?}, which a probe of it would not make (decision 0474)",
                    r.value(row, c)
                )));
            }
        }
        Ok((named.into_iter().map(|(_, r)| r).collect(), strings, symbols))
    }

    /// Every document's rows, made if they are not: those unmade as one group.
    fn complete(&self) -> &Complete {
        if let Some(c) = self.rows.complete.get() {
            return c;
        }
        self.ensure(&(0..self.docs.len()).collect::<Vec<_>>());
        self.rows.complete.get_or_init(|| {
            let groups = self.rows.state.lock().groups.clone();
            let concat: Vec<Concat> = (0..self.rows.names.len()).map(|rel| Concat { rel, groups: groups.clone(), template: Arc::clone(&self.rows.template[rel]) }).collect();
            // Each relation's rows as given (its kept statistics'): its `len` was answered by them.
            for (rel, c) in concat.iter().enumerate() {
                if let Some(given) = self.rows.counted(rel)
                    && given != c.len()
                {
                    self.docs.fail(format!("{}: {} rows made, its kept statistics counted {given} (decision 0474)", self.rows.names[rel], c.len()));
                }
            }
            Complete { groups, concat }
        })
    }

    /// The groups made so far.
    fn made_groups(&self) -> Vec<Arc<Group>> {
        self.rows.state.lock().groups.clone()
    }

    /// The groups holding the documents `keys` name, made if they are not.
    fn groups_of(&self, keys: impl IntoIterator<Item = u32>) -> Vec<Arc<Group>> {
        let docs: BTreeSet<usize> = keys.into_iter().filter_map(|k| self.docs.position(k)).collect();
        let docs: Vec<usize> = docs.into_iter().collect();
        self.ensure(&docs);
        let s = self.rows.state.lock();
        let mut at: Vec<u32> = docs.iter().filter_map(|&i| s.of[i]).collect();
        at.sort_unstable();
        at.dedup();
        at.into_iter().map(|g| Arc::clone(&s.groups[g as usize])).collect()
    }

    /// The groups' strings and symbols, every document made: the batch's vocabulary, each string once.
    pub(super) fn vocabulary(&self) -> (Vec<String>, Vec<u32>) {
        let mut seen = HashSet::new();
        let (mut strings, mut symbols) = (Vec::new(), Vec::new());
        for g in &self.complete().groups {
            for (s, &sym) in g.strings.iter().zip(&g.symbols) {
                if seen.insert(sym) {
                    strings.push(s.clone());
                    symbols.push(sym);
                }
            }
        }
        (strings, symbols)
    }
}

/// A relation over several groups, every document made: each group's rows after its base.
struct Concat {
    rel: usize,
    groups: Vec<Arc<Group>>,
    template: Arc<dyn Source>,
}

impl Concat {
    fn at(&self, row: usize) -> (&Arc<dyn Source>, usize) {
        let g = self.groups.partition_point(|g| g.bases[self.rel] <= row).max(1) - 1;
        let group = &self.groups[g];
        (&group.rels[self.rel], row - group.bases[self.rel])
    }
}

impl Source for Concat {
    fn columns(&self) -> Vec<String> {
        self.template.columns()
    }
    fn len(&self) -> usize {
        self.groups.last().map_or(0, |g| g.bases[self.rel] + g.rels[self.rel].len())
    }
    fn value(&self, row: usize, column: usize) -> i64 {
        let (r, row) = self.at(row);
        r.value(row, column)
    }
    fn row_at(&self, row: usize, columns: &[usize], out: &mut [i64]) {
        let (r, row) = self.at(row);
        r.row_at(row, columns, out)
    }
    fn indexed(&self, column: usize) -> bool {
        self.template.indexed(column)
    }
    fn text_column(&self, column: usize) -> bool {
        self.template.text_column(column)
    }
    fn estimated_matches(&self, column: usize, batch: bool) -> usize {
        self.groups.iter().map(|g| g.rels[self.rel].estimated_matches(column, batch)).sum::<usize>().max(1)
    }
    fn lookup(&self, column: usize, values: &[i64]) -> Option<Vec<usize>> {
        let mut rows = Vec::new();
        for g in &self.groups {
            let base = g.bases[self.rel];
            rows.extend(g.rels[self.rel].lookup(column, values)?.into_iter().map(|r| base + r));
        }
        Some(rows)
    }
    fn failure(&self) -> Option<String> {
        None
    }
}

/// A relation a batch makes (`text`, `op`, the matchers'), over its groups (decision 0474): every document made, the
/// one group's rows when one group holds them all (a batch read whole); before, what is known without making a
/// document, and a probe of the first column makes the documents it names.
pub(super) struct MadeRows {
    pub(super) batch: Arc<Batch>,
    pub(super) rel: usize,
}

impl MadeRows {
    fn template(&self) -> &dyn Source {
        &*self.batch.rows.template[self.rel]
    }
    /// What a read reads once every document is made: the one group's relation, or the groups' (`Concat`).
    fn target(&self) -> &dyn Source {
        let c = self.batch.complete();
        if c.groups.len() == 1 { &*c.groups[0].rels[self.rel] } else { &c.concat[self.rel] }
    }
    /// `target`, if every document is made.
    fn done(&self) -> Option<&dyn Source> {
        let c = self.batch.rows.complete.get()?;
        Some(if c.groups.len() == 1 { &*c.groups[0].rels[self.rel] } else { &c.concat[self.rel] })
    }
    /// How the relation's first column names its row's document.
    fn by(&self) -> ByDocument {
        self.batch.rows.by[self.rel]
    }
    /// Each group holding the documents `values` of the first column name, with its base: made if they are not.
    fn routed(&self, values: &[i64]) -> Vec<(Arc<dyn Source>, usize)> {
        let by = self.by();
        self.batch.groups_of(values.iter().filter_map(|&v| by.document(v))).into_iter().map(|g| (Arc::clone(&g.rels[self.rel]), g.bases[self.rel])).collect()
    }
    /// The rows a probe of `column` by `values` matches, the documents they name made: `None` for a column that
    /// names no document.
    fn probe(&self, column: usize, values: &[i64], each: impl Fn(&dyn Source, &[i64]) -> Option<Vec<usize>>) -> Option<Vec<usize>> {
        if self.by().column() != Some(column) {
            return None;
        }
        let mut rows = Vec::new();
        for (r, base) in self.routed(values) {
            rows.extend(each(&*r, values)?.into_iter().map(|row| base + row));
        }
        Some(rows)
    }
    /// The relation and the row a row of the groups made so far is, if it is one.
    fn made_row(&self, row: usize) -> Option<(Arc<dyn Source>, usize)> {
        let groups = self.batch.made_groups();
        let g = groups.iter().find(|g| (g.bases[self.rel]..g.bases[self.rel] + g.rels[self.rel].len()).contains(&row))?;
        Some((Arc::clone(&g.rels[self.rel]), row - g.bases[self.rel]))
    }
}

impl Source for MadeRows {
    fn python_batch(&self) -> Option<Arc<Batch>> {
        match self.done() {
            Some(t) => t.python_batch(),
            None => self.template().python_batch(),
        }
    }
    fn row_stream(&self,_range:Range<usize>,_reverse:bool)->Option<Box<dyn RowStream>> {
        self.done().and_then(|t| t.row_stream(_range, _reverse))
    }
    fn direct_all(&self,_equal:&[(usize,i64)])->Option<Vec<ReadRow>> {
        self.done().and_then(|t| t.direct_all(_equal))
    }
    fn direct_one(&self,_column:usize,_values:&[i64])->Option<Vec<ReadRow>> {
        self.done().and_then(|t| t.direct_one(_column, _values))
    }
    fn prepare_lookup(&self,_columns:&[usize]) {
        if let Some(t) = self.done() {
            t.prepare_lookup(_columns)
        }
    }
    fn caches_rows(&self)->bool {
        match self.done() {
            Some(t) => t.caches_rows(),
            None => self.template().caches_rows(),
        }
    }
    fn row_cells(&self,_row:usize)->Option<(i64,Arc<dyn RowCells>)> {
        self.done().and_then(|t| t.row_cells(_row))
    }
    fn prepare_index(&self, _columns:&[usize]) {
        if let Some(t) = self.done() {
            t.prepare_index(_columns)
        }
    }
    fn primary_key_len(&self)->Option<usize> {
        match self.done() {
            Some(t) => t.primary_key_len(),
            None => self.template().primary_key_len(),
        }
    }
    fn lookup_primary_batch(&self, _values:&[Vec<i64>])->Option<Vec<usize>> {
        self.done().and_then(|t| t.lookup_primary_batch(_values))
    }
    fn frozen(&self) -> Option<Arc<dyn Source>> {
        self.done().and_then(|t| t.frozen())
    }
    fn revision(&self) -> Option<u64> {
        self.done().and_then(|t| t.revision())
    }
    fn cell(&self,row:usize,column:usize)-> rusqlite::types::Value {
        self.target().cell(row, column)
    }
    fn row_id(&self,row:usize)->i64 {
        self.target().row_id(row)
    }
    fn writable(&self)->bool {
        match self.done() {
            Some(t) => t.writable(),
            None => self.template().writable(),
        }
    }
    fn ordered(&self,_equal:&[usize],_order:&[(usize,bool)])->bool {
        self.done().is_some_and(|t| t.ordered(_equal, _order))
    }
    fn ordered_range(&self,_equal:&[(usize,i64)])->Option<Range<usize>> {
        self.done().and_then(|t| t.ordered_range(_equal))
    }
    fn write(&self,_old:Option<i64>,_new:Option<i64>,_values:Option<Vec<Option<i64>>>,_replace:bool,_ignore:bool)-> rusqlite::Result<i64> {
        self.target().write(_old, _new, _values, _replace, _ignore)
    }
    fn text_column(&self, _column: usize) -> bool {
        match self.done() {
            Some(t) => t.text_column(_column),
            None => self.template().text_column(_column),
        }
    }
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        None
    }
    fn columns(&self) -> Vec<String> {
        match self.done() {
            Some(t) => t.columns(),
            None => self.template().columns(),
        }
    }
    fn len(&self) -> usize {
        match (self.done(), self.batch.rows.counted(self.rel)) {
            (None, Some(n)) => n,
            _ => self.target().len(),
        }
    }
    fn row_at(&self, row: usize, columns: &[usize], out: &mut [i64]) {
        if let Some(t) = self.done() {
            return t.row_at(row, columns, out);
        }
        match self.made_row(row) {
            Some((r, row)) => r.row_at(row, columns, out),
            None => self.target().row_at(row, columns, out),
        }
    }
    fn for_rows(&self, columns: &[usize], each: &mut dyn FnMut(usize, &[i64])) {
        self.target().for_rows(columns, each)
    }
    fn lookup_read(&self, _column: usize, _value: i64, _columns: &[usize], _each: &mut dyn FnMut(usize, &[i64])) -> bool {
        if let Some(t) = self.done() {
            return t.lookup_read(_column, _value, _columns, _each);
        }
        if self.by().column() != Some(_column) {
            return self.target().lookup_read(_column, _value, _columns, _each);
        }
        // One value names one document, so one group.
        match self.routed(std::slice::from_ref(&_value)).into_iter().next() {
            Some((r, base)) => r.lookup_read(_column, _value, _columns, &mut |row, v| _each(base + row, v)),
            None => true,
        }
    }
    fn for_rows_in(&self, range: Range<usize>, columns: &[usize], each: &mut dyn FnMut(usize, &[i64]) -> bool) {
        self.target().for_rows_in(range, columns, each)
    }
    fn documents_of(&self, column: usize, each: &mut dyn FnMut(usize, i64)) {
        self.target().documents_of(column, each)
    }
    fn rows_of(&self, columns: &[usize], out: &mut Vec<i64>) {
        self.target().rows_of(columns, out)
    }
    fn estimated_len(&self) -> usize {
        match (self.done(), self.batch.rows.counted(self.rel)) {
            (None, Some(n)) => n,
            _ => self.target().estimated_len(),
        }
    }
    fn rows_owned(&self, _g: usize, _owned: &dyn Fn(i64) -> bool) -> Option<Vec<usize>> {
        self.done().and_then(|t| t.rows_owned(_g, _owned))
    }
    fn estimated_matches(&self, column: usize, batch: bool) -> usize {
        self.target().estimated_matches(column, batch)
    }
    fn lookup_cost(&self, _column: usize) -> f64 {
        match self.done() {
            Some(t) => t.lookup_cost(_column),
            None => self.template().lookup_cost(_column),
        }
    }
    fn estimated_matches_all(&self, columns: &[usize]) -> usize {
        self.target().estimated_matches_all(columns)
    }
    fn composite(&self) -> bool {
        match self.done() {
            Some(t) => t.composite(),
            None => self.template().composite(),
        }
    }
    fn lookup_all(&self, _equal: &[(usize, i64)]) -> Option<Vec<usize>> {
        self.done().and_then(|t| t.lookup_all(_equal))
    }
    /// The documents' rows by group (decision 0474): a group of theirs alone holds theirs and no other; in a group
    /// with others, theirs are those its routing column names (none for a relation not routed).
    fn rows_of_documents(&self, docs: &std::collections::BTreeSet<u32>) -> Option<Vec<usize>> {
        let at: BTreeSet<usize> = docs.iter().filter_map(|&k| self.batch.docs.position(k)).collect();
        let by = self.by();
        let mut rows = Vec::new();
        for g in self.batch.groups_of(docs.iter().copied()) {
            let (r, base) = (&g.rels[self.rel], g.bases[self.rel]);
            if g.docs.iter().all(|i| at.contains(i)) {
                rows.extend(base..base + r.len());
            } else {
                let c = by.column()?;
                rows.extend((0..r.len()).filter(|&row| by.document(r.value(row, c)).is_some_and(|k| docs.contains(&k))).map(|row| base + row));
            }
        }
        rows.sort_unstable();
        Some(rows)
    }
    fn doc_column(&self) -> Option<usize> {
        match self.done() {
            Some(t) => t.doc_column(),
            None => self.template().doc_column(),
        }
    }
    fn lookup_in(&self, _column: usize, _values: &[i64], _doc: i64) -> Option<Vec<usize>> {
        if let Some(t) = self.done() {
            return t.lookup_in(_column, _values, _doc);
        }
        let mut rows = Vec::new();
        let groups = self.batch.groups_of(u32::try_from(_doc).ok());
        for (r, base) in groups.into_iter().map(|g| (Arc::clone(&g.rels[self.rel]), g.bases[self.rel])) {
            rows.extend(r.lookup_in(_column, _values, _doc)?.into_iter().map(|row| base + row));
        }
        Some(rows)
    }
    fn kept_to_doc(&self, _column: usize) -> bool {
        match self.done() {
            Some(t) => t.kept_to_doc(_column),
            None => self.template().kept_to_doc(_column),
        }
    }
    fn filter_eq(&self, eqs: &[(usize, i64)]) -> Vec<u32> {
        self.target().filter_eq(eqs)
    }
    #[cfg(target_os = "macos")]
    fn device_view(&self) -> Option<DeviceView<'_>> {
        self.done().and_then(|t| t.device_view())
    }
    fn in_block(&self) -> Option<crate::code_rule_dsl::facts::rows::InBlock> {
        self.done().and_then(|t| t.in_block())
    }
    fn own_index(&self, _columns: &[usize]) -> Option<(crate::code_rule_dsl::facts::rows::InBlock, usize)> {
        self.done().and_then(|t| t.own_index(_columns))
    }
    fn made_on_lookup(&self) -> bool {
        false
    }
    fn complete_rows(&self) -> bool {
        true
    }
    fn expect_lookups(&self, _column: usize, _keys: f64) {
        if let Some(t) = self.done() {
            t.expect_lookups(_column, _keys)
        }
    }
    fn statistics(&self) -> Option<Vec<(String, i64, i64, i64, i64)>> {
        self.done().and_then(|t| t.statistics())
    }
    fn value_counts(&self, _wanted: &[(usize, i64)]) -> Option<Vec<i64>> {
        self.done().and_then(|t| t.value_counts(_wanted))
    }
    fn failure(&self) -> Option<String> {
        self.batch.docs.failure().or_else(|| self.done().and_then(|t| t.failure()))
    }
    fn failed(&self) -> Option<String> {
        self.batch.docs.failure().or_else(|| self.done().and_then(|t| t.failed()))
    }
    fn indexed(&self, column: usize) -> bool {
        match self.done() {
            Some(t) => t.indexed(column),
            None => self.template().indexed(column),
        }
    }
    fn range_upper_compatible(&self, _lower: usize, _upper: usize) -> bool {
        match self.done() {
            Some(t) => t.range_upper_compatible(_lower, _upper),
            None => self.template().range_upper_compatible(_lower, _upper),
        }
    }
    fn range_indexed(&self, _column: usize) -> bool {
        match self.done() {
            Some(t) => t.range_indexed(_column),
            None => self.template().range_indexed(_column),
        }
    }
    fn range_lookup(&self, _column: usize, _lower: Option<(i64, bool)>, _upper: Option<(i64, bool)>) -> Option<Vec<usize>> {
        if let Some(t) = self.done() {
            return t.range_lookup(_column, _lower, _upper);
        }
        // A range of the column that names a document, within known bounds: its documents' keys, each made.
        if let (Some((lo, _)), Some((hi, _))) = (_lower, _upper)
            && self.by().column() == Some(_column)
            && let (Some(first), Some(last)) = (self.by().document(lo), self.by().document(hi))
        {
            let keys = self.batch.docs.keys().iter().copied().filter(|k| (first..=last).contains(k));
            let groups = self.batch.groups_of(keys.collect::<Vec<_>>());
            let mut rows = Vec::new();
            for (r, base) in groups.into_iter().map(|g| (Arc::clone(&g.rels[self.rel]), g.bases[self.rel])) {
                rows.extend(r.range_lookup(_column, _lower, _upper)?.into_iter().map(|row| base + row));
            }
            return Some(rows);
        }
        self.target().range_lookup(_column, _lower, _upper)
    }
    fn lookup(&self, column: usize, values: &[i64]) -> Option<Vec<usize>> {
        if let Some(t) = self.done() {
            return t.lookup(column, values);
        }
        self.probe(column, values, |r, v| r.lookup(column, v)).or_else(|| self.target().lookup(column, values))
    }
    fn keys_rows(&self, _column: usize) -> bool {
        match self.done() {
            Some(t) => t.keys_rows(_column),
            None => self.template().keys_rows(_column),
        }
    }
    fn key_row(&self, _column: usize, _value: i64) -> Option<usize> {
        if let Some(t) = self.done() {
            return t.key_row(_column, _value);
        }
        if self.by().column() != Some(_column) {
            return self.target().key_row(_column, _value);
        }
        self.routed(std::slice::from_ref(&_value)).into_iter().find_map(|(r, base)| r.key_row(_column, _value).map(|row| base + row))
    }
    fn lookup_eq(&self, _column: usize, _value: i64, _eqs: &[(usize, i64)]) -> Option<Vec<usize>> {
        if let Some(t) = self.done() {
            return t.lookup_eq(_column, _value, _eqs);
        }
        self.probe(_column, std::slice::from_ref(&_value), |r, _| r.lookup_eq(_column, _value, _eqs)).or_else(|| self.target().lookup_eq(_column, _value, _eqs))
    }
    fn value(&self, row: usize, column: usize) -> i64 {
        if let Some(t) = self.done() {
            return t.value(row, column);
        }
        match self.made_row(row) {
            Some((r, row)) => r.value(row, column),
            None => self.target().value(row, column),
        }
    }
    fn sorted_rows(&self, _column: usize, _value: i64) -> Option<&[u32]> {
        self.done().and_then(|t| t.sorted_rows(_column, _value))
    }
    fn sorted_range(&self, _column: usize, _value: i64) -> Option<std::ops::Range<u32>> {
        self.done().and_then(|t| t.sorted_range(_column, _value))
    }
    fn sorted_range_at(&self, column: usize, value: i64, _at: &mut u32) -> Option<std::ops::Range<u32>> {
        self.done().and_then(|t| t.sorted_range_at(column, value, _at))
    }
    fn sorts_rows(&self, column: usize) -> bool {
        self.done().is_some_and(|t| t.sorts_rows(column))
    }
    fn order(&self) -> Option<Vec<usize>> {
        self.done().and_then(|t| t.order())
    }
    fn kept(&self) -> bool {
        self.done().is_some_and(|t| t.kept())
    }
}

/// A text test's relation (decision 0037) over a batch (decision 0474): a test of a string. A probe tests the probed
/// strings (each once a run, `Vocabulary::rows`); a scan lists the batch's strings that pass, every document made.
/// Nothing of it changes with an edit. In a batch whose documents are made by probes (`Batch::lazy`) a join probes it.
pub(super) struct TextTest {
    batch: Arc<Batch>,
    k: usize,
    tests: Arc<crate::code_rule_dsl::logic::strings::Vocabulary>,
    atoms: Arc<crate::content_store::atoms::Atoms>,
    /// The strings that passed a probe, in row order, and each one's row.
    probed: parking_lot::Mutex<(Vec<i64>, HashMap<i64, usize>)>,
    /// Every string of the batch that passes, the probed first (their rows kept): made by the first scan.
    whole: OnceLock<Relation>,
}

impl TextTest {
    pub(super) fn new(batch: &Arc<Batch>, k: usize, tests: &Arc<crate::code_rule_dsl::logic::strings::Vocabulary>, atoms: Arc<crate::content_store::atoms::Atoms>) -> Self {
        Self { batch: Arc::clone(batch), k, tests: Arc::clone(tests), atoms, probed: Default::default(), whole: OnceLock::new() }
    }
    /// The batch's passing strings, made outside the cell (they make documents on the pool, decision 0474's
    /// `ensure`), then kept.
    fn whole(&self) -> &Relation {
        if let Some(w) = self.whole.get() {
            return w;
        }
        let (strings, symbols) = self.batch.vocabulary();
        let passing = self.tests.rows(&strings, &symbols).swap_remove(self.k);
        let probed = self.probed.lock();
        let mut r = Relation::new(&["s"]);
        for &s in probed.0.iter().chain(passing.iter().filter(|s| !probed.1.contains_key(s))) {
            r.push(&[s]);
        }
        drop(probed);
        self.whole.get_or_init(|| r)
    }
    /// The rows of the probed strings that pass, each tested once.
    fn probe(&self, values: &[i64]) -> Vec<usize> {
        let fresh: Vec<i64> = {
            let probed = self.probed.lock();
            let mut fresh: Vec<i64> = values.iter().copied().filter(|v| !probed.1.contains_key(v)).collect();
            fresh.sort_unstable();
            fresh.dedup();
            fresh
        };
        let (mut strings, mut symbols) = (Vec::new(), Vec::new());
        for &v in &fresh {
            if let Ok(sym) = u32::try_from(v)
                && let Ok(Some(text)) = self.atoms.value(sym)
            {
                strings.push(text);
                symbols.push(sym);
            }
        }
        let passing = if strings.is_empty() { Vec::new() } else { self.tests.rows(&strings, &symbols).swap_remove(self.k) };
        let mut probed = self.probed.lock();
        for s in passing {
            if !probed.1.contains_key(&s) {
                let row = probed.0.len();
                probed.0.push(s);
                probed.1.insert(s, row);
            }
        }
        values.iter().filter_map(|v| probed.1.get(v).copied()).collect()
    }
}

impl Source for TextTest {
    fn columns(&self) -> Vec<String> {
        vec!["s".to_owned()]
    }
    fn len(&self) -> usize {
        self.whole().len()
    }
    fn estimated_len(&self) -> usize {
        match self.whole.get() {
            Some(w) => w.len(),
            None if self.batch.lazy() => self.probed.lock().0.len().max(1),
            None => self.whole().len(),
        }
    }
    fn value(&self, row: usize, column: usize) -> i64 {
        match self.whole.get() {
            Some(w) => Source::value(w, row, column),
            None => self.probed.lock().0[row],
        }
    }
    fn indexed(&self, column: usize) -> bool {
        column == 0
    }
    fn estimated_matches(&self, _column: usize, batch: bool) -> usize {
        if batch { 8 } else { 1 }
    }
    fn lookup(&self, column: usize, values: &[i64]) -> Option<Vec<usize>> {
        if column != 0 {
            return None;
        }
        match self.whole.get() {
            Some(w) => Source::lookup(w, column, values),
            None => Some(self.probe(values)),
        }
    }
    fn ordered(&self, equal: &[usize], order: &[(usize, bool)]) -> bool {
        self.whole.get().is_some_and(|w| Source::ordered(w, equal, order))
    }
    fn sorted_rows(&self, column: usize, value: i64) -> Option<&[u32]> {
        self.whole.get().and_then(|w| Source::sorted_rows(w, column, value))
    }
    fn made_on_lookup(&self) -> bool {
        self.whole.get().is_none() && self.batch.lazy()
    }
    fn failure(&self) -> Option<String> {
        self.batch.docs.failure()
    }
}
