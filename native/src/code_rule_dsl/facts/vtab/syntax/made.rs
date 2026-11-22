//! A batch's documents and rows, each made when first read (decision 0494).
//!
//! A batch holds its documents unmade. A document's projection (its rank index, which the views read) is made on its
//! first read (`Docs`). Its rows, the relations the batch makes off the tapes (`text`, `op`, the matchers'), are made
//! in a group with the other documents the read needs (`Rows`), numbered after the rows of the groups made before it.
//! A batch read whole from its first read makes one group, in key order: one relation per relation, as an eager batch.

use super::*;
#[cfg(any(target_os = "macos", target_os = "linux"))]
use crate::code_rule_dsl::facts::vtab::DeviceView;
use crate::code_rule_dsl::facts::vtab::{ReadRow, RowCells, RowStream};
use crate::code_rule_dsl::logic::profile;
use std::ops::Range;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A document's views' counts (decision 0494): its nodes, fielded nodes, siblings after it and places, as its
/// projection has them; the documents' table keeps them.
pub type ViewCounts = [u32; 4];

/// A document's row of the documents' table's counts (`Batch::kept_counts`, `retain::put_base_counts`): its key, path,
/// digest, views' counts and facts' size, and whether it is a file's own tree (a guest's path is its host's).
pub struct Kept {
    pub key: u32,
    pub path: String,
    pub digest: [u8; 32],
    pub counts: ViewCounts,
    pub size: FactsSize,
    pub file: bool,
}

/// A projection's views' counts.
pub(super) fn view_counts(p: &Projected) -> ViewCounts {
    [p.len(), p.fielded(), p.after_siblings(), p.locs()]
}

/// A batch's documents (decision 0494): each one's projection made on its first read; its key and tag known before.
pub struct Docs {
    /// Shared with the batches it grows into (decision 0431 §6): a document is made once, whichever reads it first.
    slots: Vec<Arc<Slot>>,
    keys: Vec<u32>,
    tags: Vec<u8>,
    /// Each document's views' counts, where given (the documents' table's).
    given: Vec<Option<ViewCounts>>,
    context: Context,
    /// How many projections are made (R2's tests read it).
    made: AtomicUsize,
}

struct Slot {
    input: Input,
    made: OnceLock<MadeDoc>,
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
    /// The files' lines, by key, that their guests are placed by: shared with the batches it grows into and the tape
    /// values admitted to them (decision 0431 §6, §8), whose hosts may be no document of theirs.
    pub(super) lines_of: Arc<parking_lot::RwLock<HashMap<u32, Arc<crate::code_rule_dsl::lines::LineTable>>>>,
    /// The first failure making a document or a group (a verdict raised, too many positions): every source of the
    /// batch reports it (`Source::failure`), so the statement reading it fails rather than read the rows as absent.
    pub(super) failure: parking_lot::Mutex<Option<String>>,
}

impl Docs {
    /// `documents`, sorted by key and distinct, with the views' counts `given` of some, by key.
    pub(super) fn new(documents: Vec<Input>, context: Context, given: &HashMap<u32, ViewCounts>) -> Self {
        let keys: Vec<u32> = documents.iter().map(Input::key).collect();
        let tags = documents.iter().map(Input::tag).collect();
        let given = keys.iter().map(|k| given.get(k).copied()).collect();
        let slots = documents.into_iter().map(|input| Arc::new(Slot { input, made: OnceLock::new() })).collect();
        Self { slots, keys, tags, given, context, made: AtomicUsize::new(0) }
    }
    /// `prior`'s documents and `documents` (sorted by key, none of `prior`'s), in key order: `prior`'s shared, as
    /// made there (decision 0431 §6), `documents` unmade, with the views' counts `given` of some. Each one's index here
    /// is returned for `prior`'s, in its order.
    pub(super) fn grown(prior: &Docs, documents: Vec<Input>, context: Context, given: &HashMap<u32, ViewCounts>) -> (Self, Vec<usize>) {
        let added = Self::new(documents, context, given);
        let Self { slots: new_slots, keys: new_keys, tags: new_tags, given: new_given, context, .. } = added;
        let n = prior.slots.len() + new_slots.len();
        let (mut slots, mut keys, mut tags, mut given, mut at) = (Vec::with_capacity(n), Vec::with_capacity(n), Vec::with_capacity(n), Vec::with_capacity(n), Vec::with_capacity(prior.slots.len()));
        let (mut i, mut j) = (0, 0);
        while i < prior.slots.len() || j < new_slots.len() {
            if j == new_slots.len() || (i < prior.slots.len() && prior.keys[i] < new_keys[j]) {
                at.push(slots.len());
                slots.push(Arc::clone(&prior.slots[i]));
                keys.push(prior.keys[i]);
                tags.push(prior.tags[i]);
                given.push(prior.given[i]);
                i += 1;
            } else {
                slots.push(Arc::clone(&new_slots[j]));
                keys.push(new_keys[j]);
                tags.push(new_tags[j]);
                given.push(new_given[j]);
                j += 1;
            }
        }
        (Self { slots, keys, tags, given, context, made: AtomicUsize::new(0) }, at)
    }
    /// Whether the documents' views' counts were given: the views' row space is known with none made but those
    /// whose document is made already (an edit's changed ones, `Input::Made`), whose counts are their own
    /// projection's (`counts`). A batch given none (a cold run's) is not.
    pub fn given_all(&self) -> bool {
        self.given.iter().any(Option::is_some) && self.given.iter().zip(&self.slots).all(|(g, s)| g.is_some() || s.input.is_made())
    }
    /// Whether every document's views' counts were given, an edit's changed ones too: none to keep anew.
    pub fn given_every(&self) -> bool {
        self.given.iter().all(Option::is_some)
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
    /// What document `i` is made from: its facts, tape, path and place, built if it is pending (`Pending`, Q332). One
    /// that could not be built fails the batch (`failure`).
    pub fn input(&self, i: usize) -> &Document {
        let input = &self.slots[i].input;
        let d = input.document();
        if let Some(why) = input.failure() {
            self.fail(why.to_owned());
        }
        d
    }
    /// Document `i`'s module as the corpus names it; none is built for it.
    pub fn module(&self, i: usize) -> Option<&str> {
        self.slots[i].input.module()
    }
    /// Document `i`'s place in its host, a guest's; none is built for it.
    pub fn guest(&self, i: usize) -> Option<&Arc<Guest>> {
        self.slots[i].input.guest()
    }
    /// Document `i`'s path, digest and facts' size, as given; none is built for them.
    pub fn path(&self, i: usize) -> &str {
        self.slots[i].input.path()
    }
    pub fn digest(&self, i: usize) -> [u8; 32] {
        self.slots[i].input.digest()
    }
    pub fn size(&self, i: usize) -> FactsSize {
        self.slots[i].input.size()
    }
    /// How many of the documents given pending (`Pending`) were built.
    pub fn built_count(&self) -> usize {
        self.slots.iter().filter(|s| matches!(s.input, Input::Pending(_)) && s.input.is_made()).count()
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
                    "{}: its views' counts (nodes, fielded, siblings, places) are {:?}, the documents' table gave {given:?} (decision 0494)",
                    self.slots[i].input.path(),
                    view_counts(&made.p)
                ));
            }
            made
        })
    }
    /// Document `i`'s projection (decision 0289 §3: in the batch's block), its verdict and its module.
    fn project(&self, i: usize) -> MadeDoc {
        let d = self.input(i);
        let projection = &*self.context.projection;
        let shape = projection.shape;
        let core = shape == Shape::Core;
        // Its facts' tag: the one given (`tag`), or a text's where it could not be built (`Pending`).
        let tag = crate::code_rule_dsl::facts::core::facts_tag(&d.facts);
        // A text-only document has no dialect, and no node.
        let dialect = if tag == TEXT { None } else { self.context.dialects.get(tag) };
        // `core.rerail`: a document that does not parse is withheld from the policy (no facts) but its `doc`, path and
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
        // A guest's nodes have their places in its file: the file's lines, where the batch holds the file.
        let placed = guest.map(|g| (g, self.file_lines(g.host).unwrap_or_else(|| Arc::clone(&d.facts.lines))));
        let new_span = profile::span(|| "projected".into());
        let mut p = Projected::narrowed(d.key, Arc::clone(&d.facts), dialect, shape, withheld, placed, projection.node_anchors.as_ref().filter(|_| core), &self.context.block);
        // T's shape takes a program's whole tree, not as a guest, but its nodes are its file's too (Q324, decision 0512).
        p.file = Guest::file_key(d.guest.as_deref(), d.key);
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
            self.fail(format!("{} has more than {} positions (tokens and lines: core.rerail's span ids)", d.path, granules::POSITIONS));
        }
        // A file's lines, for its guests a later round or an admitted value places in it.
        if core && guest.is_none() && let Some(x) = &p.extra {
            self.context.lines_of.write().insert(d.key, Arc::clone(&x.lines));
        }
        MadeDoc { p, withheld, error }
    }
    /// The lines of the file a guest of `host` is in (through guests of guests), where the batch holds that file in
    /// core.rerail's shape.
    fn file_lines(&self, host: u32) -> Option<Arc<crate::code_rule_dsl::lines::LineTable>> {
        let mut k = host;
        for _ in 0..=self.context.host.len() {
            match self.context.host.get(&k) {
                Some(&h) => k = h,
                None => break,
            }
        }
        if let Some(i) = self.position(k) {
            if self.slots[i].input.guest().is_some() {
                return None;
            }
            return self.made(i).p.extra.as_ref().map(|x| Arc::clone(&x.lines));
        }
        // A file of the batch this one grew from, or the chunk a tape value is admitted to.
        self.context.lines_of.read().get(&k).cloned()
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

/// What a group made: the rows `Batch::make_rows` made for some documents, before they are bound, each relation's
/// staged (or, a batch's first group, written) to go on in its block relation.
pub(super) struct Made {
    pub(super) relations: Vec<(&'static str, Relation)>,
    /// The `match_all` relations (`logic::strings::as_relations`).
    pub(super) matched: Vec<(String, Relation)>,
    /// The `ast` matchers' relations (decision 0106).
    pub(super) waved: Vec<(String, Relation)>,
    /// The relations made in place of views (`Batch::made`).
    pub(super) read: BTreeSet<&'static str>,
    /// Where the strings the group added begin among the batch's (`Shared::strings`): the text tests of an admitted
    /// value read those.
    pub(super) strings_from: usize,
}

/// A group: documents whose rows were made together, and each relation's rows of them, a range of its block relation.
pub(super) struct Group {
    pub(super) docs: Vec<usize>,
    /// Each relation's rows of the group, in `Rows::names`' order.
    pub(super) ranges: Vec<Range<usize>>,
}

/// The batch's rows (decision 0494): each relation one block relation (`Relation::grown`), which every group goes on
/// in, in the order made, so the CPU and the device read one relation in one block; a group's rows a range of it.
pub(super) struct Rows {
    state: parking_lot::Mutex<State>,
    /// Woken when a group goes on in the relations or its claims are given up (`Batch::ensure`).
    made: parking_lot::Condvar,
    /// The groups made, those another read's made first and so set aside too: `State::groups`' length, unless two
    /// reads made one document.
    makes: std::sync::atomic::AtomicUsize,
    /// The relations a group makes, by name: what an empty group makes, before any document is (`Batch::new`).
    pub(super) names: Vec<String>,
    /// An empty group's relations, which answer what a relation is (its columns, its indexes) before its rows are
    /// made.
    pub(super) template: Vec<Arc<Relation>>,
    /// How each relation's first column names its row's document.
    pub(super) by: Vec<ByDocument>,
    /// Each relation's other columns that hold a node of the row's own document (`d << 32 | …`): a matcher's node
    /// captures (`AstWave::node_columns`). A probe of one makes the documents its values name, as one of `by`'s does.
    also: Vec<Vec<usize>>,
    /// Each relation's column of its document's own path (`doc(d, path, …)`), if it has one: a read of it by a
    /// path is routed to that path's document (`Batch::keys_of_paths`), as one by the document's key is.
    path_column: Vec<Option<usize>>,
    /// The same for its document's module (`doc(d, _, module)`, `Batch::keys_of_modules`).
    module_column: Vec<Option<usize>>,
    /// How many of `names` are the batch's relations (`Made::relations`), which a program's derived relations replace;
    /// the `match_all` and the matchers' follow.
    pub(super) relations: usize,
    /// The batch's relations' names as the producers name them (`Made::relations`).
    pub(super) statics: Vec<&'static str>,
    /// Each relation, every document's rows made.
    complete: OnceLock<Vec<Arc<Relation>>>,
    /// Each relation's rows as its kept statistics count them (`Batch::give_totals`), where given.
    counted: OnceLock<Vec<Option<usize>>>,
    /// Each relation's distinct values in each column its rules read, as its kept statistics count them
    /// (`Batch::give_distinct`), where given: a probe's estimate before its rows are made.
    distinct: Vec<OnceLock<Vec<Option<usize>>>>,
}

#[derive(Default)]
struct State {
    groups: Vec<Arc<Group>>,
    /// Each document's group.
    of: Vec<Option<u32>>,
    /// Each relation as the last group left it: every group's rows, in the order made.
    rels: Vec<Arc<Relation>>,
    /// Each document's maker while a group of it is being made (`Batch::ensure`): another read waits for it rather
    /// than make it again.
    claimed: Vec<Option<std::thread::ThreadId>>,
    /// The threads making a group of this batch now, once a group: a thread on this list makes what it reads itself,
    /// never waits (its own group is beneath it on its stack).
    makers: Vec<std::thread::ThreadId>,
}

/// A group's claims on its documents (`State::claimed`), given up as it ends, made or not (a panic too).
struct Claim<'a> {
    rows: &'a Rows,
    me: std::thread::ThreadId,
    docs: Vec<usize>,
}

impl Drop for Claim<'_> {
    fn drop(&mut self) {
        let mut s = self.rows.state.lock();
        for &i in &self.docs {
            if s.claimed[i] == Some(self.me) {
                s.claimed[i] = None;
            }
        }
        if let Some(at) = s.makers.iter().position(|t| *t == self.me) {
            s.makers.swap_remove(at);
        }
        drop(s);
        self.rows.made.notify_all();
    }
}

/// How a relation's row names its document: by its first column, as the rule library's declarations type it
/// (`docs/rule-syntax/lib`). There a first column `n` is always a `node` or a `loc` (an id, `d << 32 | …`) and `d`
/// or `g` always a document's key; other names are ambiguous (`p` is `token`'s node and `ast_match`'s pattern), so
/// such a relation is not routed: any read of it makes every document. A group's rows of a routed relation name its
/// own documents there, so a probe of that column makes the documents it names alone; a group whose rows name
/// another fails, named. A guest's relations of its place in its host (`parsed`, `sub_tape`: `guests::HOST_ROUTED`)
/// name the host's node, and a probe of it makes the host's guests (`Host`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ByDocument {
    /// The document's key (`doc(d, …)`, `guest(g, …)`), in this column.
    Key(usize),
    /// A node's or a span's id, `d << 32 | …`, in this column.
    Id(usize),
    /// A host's node, `h << 32 | …`, in this column, of a row a guest of `h` makes (decision 0055).
    Host(usize),
    /// Not routed.
    No,
}

impl ByDocument {
    fn of(name: &str, columns: &[String]) -> Self {
        if super::guests::HOST_ROUTED.contains(&name) {
            return Self::Host(0);
        }
        match columns.first().map(String::as_str) {
            Some("d" | "g") => Self::Key(0),
            Some("n") => Self::Id(0),
            _ => Self::No,
        }
    }
    /// The column of a row's document's own path, in a relation of the document's key and its path (`doc`).
    fn path_column(name: &str, columns: &[String]) -> Option<usize> {
        Self::named_column(name, columns, "path")
    }
    /// The column of a row's document's module, in a relation of the document's key and its module (`doc`).
    fn module_column(name: &str, columns: &[String]) -> Option<usize> {
        Self::named_column(name, columns, "module")
    }
    fn named_column(name: &str, columns: &[String], column: &str) -> Option<usize> {
        match Self::of(name, columns) {
            Self::Key(_) => columns.iter().position(|c| c == column),
            _ => None,
        }
    }
    /// The column a probe of is routed: the one that names the row's document, or its host's.
    fn column(self) -> Option<usize> {
        match self {
            Self::Key(c) | Self::Id(c) | Self::Host(c) => Some(c),
            Self::No => None,
        }
    }
    /// The column that names the row's own document.
    fn own_column(self) -> Option<usize> {
        match self {
            Self::Key(c) | Self::Id(c) => Some(c),
            Self::Host(_) | Self::No => None,
        }
    }
    /// The key of the document `value` (of `column`) names, if it names one: the row's own, or its host's.
    fn document(self, value: i64) -> Option<u32> {
        match self {
            Self::Key(_) => u32::try_from(value).ok(),
            Self::Id(_) | Self::Host(_) => u32::try_from(value >> 32).ok().filter(|_| value >= 0),
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
    /// Relation `name`'s distinct values in each of its columns (`None` for one its rules do not read), as its kept
    /// statistics count them: of the rows `give_totals` gave.
    pub(super) fn give_distinct(&self, name: &str, distinct: Vec<Option<usize>>) {
        if let Some(rel) = self.names.iter().position(|n| n == name) {
            let _ = self.distinct[rel].set(distinct);
        }
    }
    /// Relation `rel`'s distinct values in `column`, as given, if they were.
    fn distinct(&self, rel: usize, column: usize) -> Option<usize> {
        self.distinct[rel].get().and_then(|d| d.get(column).copied().flatten())
    }
    /// The rows of a batch of `documents`, a group making `names` (`template`'s). `prior`: the relations of the batch
    /// it grows from (decision 0431 §6), complete, by name, whose documents (`prior_docs`, indexes in this batch) are
    /// its first group: each relation goes on from that one's.
    /// `node_columns`: each relation's other columns of nodes of the row's own document, by name (`Rows::also`).
    pub(super) fn new(documents: usize, names: Vec<String>, template: Vec<Arc<Relation>>, statics: Vec<&'static str>, prior: Option<(&HashMap<String, Arc<Relation>>, Vec<usize>)>, node_columns: &HashMap<String, Vec<usize>>) -> Self {
        let mut state = State { groups: Vec::new(), of: vec![None; documents], rels: template.clone(), claimed: vec![None; documents], makers: Vec::new() };
        if let Some((rels, docs)) = prior {
            let mut ranges = Vec::with_capacity(names.len());
            for (rel, name) in names.iter().enumerate() {
                if let Some(r) = rels.get(name).filter(|r| Source::columns(r.as_ref()) == Source::columns(template[rel].as_ref())) {
                    state.rels[rel] = Arc::clone(r);
                }
                ranges.push(0..Source::len(state.rels[rel].as_ref()));
            }
            for &i in &docs {
                state.of[i] = Some(0);
            }
            state.groups.push(Arc::new(Group { docs, ranges }));
        }
        let by: Vec<ByDocument> = names.iter().zip(&template).map(|(n, t)| ByDocument::of(n, &Source::columns(t.as_ref()))).collect();
        let distinct = names.iter().map(|_| OnceLock::new()).collect();
        let path_column = names.iter().zip(&template).map(|(n, t)| ByDocument::path_column(n, &Source::columns(t.as_ref()))).collect();
        let module_column = names.iter().zip(&template).map(|(n, t)| ByDocument::module_column(n, &Source::columns(t.as_ref()))).collect();
        let also = names.iter().zip(&by).map(|(n, b)| node_columns.get(n).map_or_else(Vec::new, |cs| cs.iter().copied().filter(|&c| Some(c) != b.column()).collect())).collect();
        Self { state: parking_lot::Mutex::new(state), made: parking_lot::Condvar::new(), makes: Default::default(), names, template, by, also, path_column, module_column, relations: statics.len(), statics, complete: OnceLock::new(), counted: OnceLock::new(), distinct }
    }
    /// The groups made.
    pub(super) fn groups(&self) -> usize {
        self.state.lock().groups.len()
    }
    /// The groups made, those set aside too (`makes`).
    pub(super) fn makes(&self) -> usize {
        self.makes.load(std::sync::atomic::Ordering::Relaxed)
    }
    /// Whether no group has rows yet: the next is written into its block relations, not staged.
    pub(super) fn first(&self) -> bool {
        self.state.lock().rels.iter().all(|r| Source::len(r.as_ref()) == 0)
    }
    /// Each relation, by name, every document made (`complete` before).
    pub(super) fn whole(&self) -> HashMap<String, Arc<Relation>> {
        let c = self.complete.get().expect("a batch grows once complete");
        self.names.iter().cloned().zip(c.iter().cloned()).collect()
    }
}

impl Batch {
    /// The rows of `want`'s documents made: those not made yet, as one group (decision 0494), which each relation goes
    /// on with in its block relation. A group is made unlocked (its prefilter and matchers run on the pool, where a
    /// waiting thread runs other jobs, one of which may read this batch), and goes on in the relations only if no other
    /// read made one of its documents meanwhile; else it is made again of those still unmade.
    ///
    /// Its documents are claimed while it is made, so reads side by side make each document once: a read finding a
    /// document another thread is making waits for it, running the pool's jobs meanwhile, rather than make it too
    /// (the base relations counted on the pool made the whole batch once each, Q396: four times its work and its
    /// memory on 901 files). A thread making a group of this batch never waits: a read under it (a job it ran while
    /// it waited) makes what it reads, as before, so no two makers wait on each other.
    pub(super) fn ensure(&self, want: &[usize]) {
        let me = std::thread::current().id();
        loop {
            let (need, waits) = {
                let s = self.rows.state.lock();
                let making = s.makers.contains(&me);
                let mut need: Vec<usize> = Vec::new();
                let mut waits = false;
                for &i in want {
                    if s.of[i].is_some() {
                        continue;
                    }
                    match s.claimed[i] {
                        Some(t) if t != me && !making => waits = true,
                        _ => need.push(i),
                    }
                }
                need.sort_unstable();
                need.dedup();
                if need.is_empty() && waits {
                    // Another thread's group: the pool's next job, else a wait for a group to end.
                    drop(s);
                    if rayon::yield_now() != Some(rayon::Yield::Executed) {
                        let mut s = self.rows.state.lock();
                        if want.iter().any(|&i| s.of[i].is_none() && s.claimed[i].is_some_and(|t| t != me)) {
                            self.rows.made.wait_for(&mut s, std::time::Duration::from_millis(1));
                        }
                    }
                    continue;
                }
                (need, waits)
            };
            if need.is_empty() {
                debug_assert!(!waits);
                return;
            }
            let claim = {
                let mut s = self.rows.state.lock();
                let docs: Vec<usize> = need.iter().copied().filter(|&i| s.claimed[i].is_none()).collect();
                for &i in &docs {
                    s.claimed[i] = Some(me);
                }
                s.makers.push(me);
                Claim { rows: &self.rows, me, docs }
            };
            let made = self.make_rows(&need, self.rows.first(), &[]).and_then(|m| self.group_rels(m, &need));
            self.rows.makes.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let mut s = self.rows.state.lock();
            if need.iter().any(|&i| s.of[i].is_some()) {
                drop(s);
                drop(claim);
                continue;
            }
            let rels = match made {
                Ok(rels) => rels,
                Err(why) => {
                    // Its documents have no rows, and every source of the batch reports why.
                    self.docs.fail(why.to_string());
                    self.rows.template.iter().map(|t| Relation::new(&Source::columns(t.as_ref()).iter().map(String::as_str).collect::<Vec<_>>())).collect()
                }
            };
            let mut ranges = Vec::with_capacity(rels.len());
            for (at, r) in s.rels.iter_mut().zip(rels) {
                let start = Source::len(at.as_ref());
                *at = Arc::new(if start == 0 { r.growable() } else { at.grown(r) });
                ranges.push(start..Source::len(at.as_ref()));
            }
            let g = s.groups.len() as u32;
            for &i in &need {
                s.of[i] = Some(g);
            }
            profile::decision("a batch's documents made by group (decision 0494)", || format!("{} of {} documents", need.len(), self.docs.len()));
            s.groups.push(Arc::new(Group { docs: need, ranges }));
            drop(s);
            drop(claim);
            if !waits {
                return;
            }
        }
    }

    /// A group's rows as its relations, in `Rows::names`' order: it makes each the batch names and no other, and each
    /// routed relation's rows name its documents.
    fn group_rels(&self, made: Made, docs: &[usize]) -> StoreResult<Vec<Relation>> {
        let Made { relations, matched, waved, .. } = made;
        let named: Vec<(String, Relation)> = relations.into_iter().map(|(name, r)| (name.to_owned(), r)).chain(matched).chain(waved).collect();
        let names: Vec<&str> = named.iter().map(|(n, _)| n.as_str()).collect();
        if names != self.rows.names.iter().map(String::as_str).collect::<Vec<_>>() {
            return Err(StoreError::Invalid(format!("a group of a batch made the relations {names:?}, not the batch's {:?} (decision 0494)", self.rows.names)));
        }
        let keys: BTreeSet<u32> = docs.iter().map(|&i| self.docs.key(i)).collect();
        // The hosts of the group's guests, which a `Host` column names.
        let hosts: BTreeSet<u32> = docs.iter().filter_map(|&i| self.docs.guest(i).map(|g| g.host)).collect();
        for ((name, r), (&by, also)) in named.iter().zip(self.rows.by.iter().zip(&self.rows.also)) {
            // A host's node is routed (and checked) only where no tape value's rows stand at places (`guests_of`).
            let checked = match by {
                ByDocument::Host(_) if self.guests_of().is_none() => None,
                ByDocument::Host(c) => Some((c, &hosts, by)),
                _ => by.column().map(|c| (c, &keys, by)),
            };
            for (c, within, by) in checked.into_iter().chain(also.iter().map(|&c| (c, &keys, ByDocument::Id(c)))) {
                if let Some(row) = (0..Source::len(r)).find(|&row| !by.document(Source::value(r, row, c)).is_some_and(|k| within.contains(&k))) {
                    return Err(StoreError::Invalid(format!(
                        "{name}: a row's column {c} ({}) names a document outside its group {keys:?} (hosts {hosts:?}), which a probe of it would not make (decision 0494)",
                        Source::value(r, row, c)
                    )));
                }
            }
        }
        Ok(named.into_iter().map(|(_, r)| r).collect())
    }

    /// Whether every document's rows are made.
    pub(super) fn is_complete(&self) -> bool {
        self.rows.complete.get().is_some()
    }
    /// Every relation, every document's rows made: those unmade as one group.
    pub(super) fn complete(&self) -> &[Arc<Relation>] {
        if let Some(c) = self.rows.complete.get() {
            return c;
        }
        self.ensure(&(0..self.docs.len()).collect::<Vec<_>>());
        self.rows.complete.get_or_init(|| {
            let rels = self.rows.state.lock().rels.clone();
            // Each relation's rows as given (its kept statistics'): its `len` was answered by them.
            for (rel, r) in rels.iter().enumerate() {
                if let Some(given) = self.rows.counted(rel)
                    && given != Source::len(r.as_ref())
                {
                    self.docs.fail(format!("{}: {} rows made, its kept statistics counted {given} (decision 0494)", self.rows.names[rel], Source::len(r.as_ref())));
                }
            }
            rels
        })
    }

    /// Relation `rel` as the groups made so far left it.
    fn made_so_far(&self, rel: usize) -> Arc<Relation> {
        Arc::clone(&self.rows.state.lock().rels[rel])
    }

    /// The groups holding the documents `keys` name, made if they are not.
    /// The keys of the documents whose paths' atoms are `values` (a read of `doc` by its path): its documents' paths
    /// interned once, none made. `None` where they could not be.
    fn keys_of_paths(&self, values: &[i64]) -> Option<Vec<u32>> {
        let by = self
            .by_path
            .get_or_init(|| {
                let paths: Vec<&str> = (0..self.docs.len()).map(|i| self.docs.path(i)).collect();
                let atoms = self.atoms.intern_all(&paths).ok()?;
                let mut by: HashMap<i64, Vec<u32>> = HashMap::new();
                for (i, a) in atoms.into_iter().enumerate() {
                    by.entry(i64::from(a)).or_default().push(self.docs.key(i));
                }
                Some(by)
            })
            .as_ref()?;
        Some(values.iter().filter_map(|v| by.get(v)).flatten().copied().collect())
    }

    /// The keys of the documents whose `doc` rows' modules' atoms are `values` (a read of `doc` by its module): each
    /// file's module (`syntax::doc_module`, from its path and the corpus's name for it) interned once, none made. A
    /// tape value has no `doc` row. `None` where they could not be.
    fn keys_of_modules(&self, values: &[i64]) -> Option<Vec<u32>> {
        let by = self
            .by_module
            .get_or_init(|| {
                let core = self.shape == Shape::Core;
                let files: Vec<usize> = (0..self.docs.len()).filter(|&i| !self.docs.guest(i).is_some_and(|g| matches!(g.made, super::Made::Tape { .. }))).collect();
                let modules: Vec<String> = files.iter().map(|&i| super::doc_module(self.docs.path(i), self.docs.module(i), core)).collect();
                let atoms = self.atoms.intern_all(&modules.iter().map(String::as_str).collect::<Vec<_>>()).ok()?;
                let mut by: HashMap<i64, Vec<u32>> = HashMap::new();
                for (&i, a) in files.iter().zip(atoms) {
                    by.entry(i64::from(a)).or_default().push(self.docs.key(i));
                }
                Some(by)
            })
            .as_ref()?;
        Some(values.iter().filter_map(|v| by.get(v)).flatten().copied().collect())
    }

    /// Each host's guests, by key (decision 0055), from the documents' places, none made: what a probe of a guest's
    /// row by its host's node makes (`ByDocument::Host`). `None` where a tape value is among them (decision 0431),
    /// whose rows stand at places in many hosts.
    fn guests_of(&self) -> Option<&HashMap<u32, Vec<u32>>> {
        self.guests_of
            .get_or_init(|| {
                let mut of: HashMap<u32, Vec<u32>> = HashMap::new();
                for i in 0..self.docs.len() {
                    let Some(g) = self.docs.guest(i) else { continue };
                    match g.made {
                        // A file's own tree (decision 0512) is its file's guest, at its whole text.
                        super::Made::Host | super::Made::File => of.entry(g.host).or_default().push(self.docs.key(i)),
                        super::Made::Tape { .. } => return None,
                        // A value document has no host, and no row of a place.
                        super::Made::Parse { .. } | super::Made::Splice { .. } => {}
                    }
                }
                Some(of)
            })
            .as_ref()
    }

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

    /// The batch's vocabulary, every document made: its strings and their symbols, each string once.
    pub(super) fn vocabulary(&self) -> (Vec<String>, Vec<u32>) {
        if self.rows.complete.get().is_none() {
            profile::decision("a batch's every document made by a whole read of a relation (decision 0494)", || "the batch's vocabulary (a text test)".to_owned());
        }
        self.complete();
        let shared = self.shared.lock();
        (shared.strings.clone(), shared.atoms.clone())
    }
}

/// A relation a batch makes (`text`, `op`, the matchers'), over its groups (decision 0494): every document made, the
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
    /// What a read reads once every document is made: the relation, whole.
    fn target(&self) -> &dyn Source {
        if self.batch.rows.complete.get().is_none() {
            profile::decision("a batch's every document made by a whole read of a relation (decision 0494)", || self.batch.rows.names[self.rel].clone());
        }
        &*self.batch.complete()[self.rel]
    }
    /// `target`, if every document is made.
    fn done(&self) -> Option<&dyn Source> {
        self.batch.rows.complete.get().map(|c| &*c[self.rel] as &dyn Source)
    }
    /// How the relation's first column names its row's document.
    fn by(&self) -> ByDocument {
        self.batch.rows.by[self.rel]
    }
    /// The rows a read of `column` by `values` matches (each also passing `keep`), the documents they name made, in
    /// their groups' ranges: `None` for a column that names no document (by its key, or `doc`'s by its path).
    fn routed(&self, column: usize, values: &[i64], keep: impl Fn(&Relation, usize) -> bool) -> Option<(Arc<Relation>, Vec<usize>)> {
        let by = self.by();
        let keys: Vec<u32> = if by.column() == Some(column) && matches!(by, ByDocument::Host(_)) {
            // A host's node: the guests of the documents named, which make the rows.
            let guests = self.batch.guests_of()?;
            values.iter().filter_map(|&v| by.document(v)).flat_map(|h| guests.get(&h).into_iter().flatten().copied()).collect()
        } else if by.column() == Some(column) {
            values.iter().filter_map(|&v| by.document(v)).collect()
        } else if self.batch.rows.also[self.rel].contains(&column) {
            // A node of the row's own document: every value must name one of the batch's, or the read is not routed
            // (a row of a document left unmade could hold it).
            values.iter().map(|&v| ByDocument::Id(column).document(v).filter(|&k| self.batch.docs.position(k).is_some())).collect::<Option<_>>()?
        } else if self.batch.rows.path_column[self.rel] == Some(column) {
            self.batch.keys_of_paths(values)?
        } else if self.batch.rows.module_column[self.rel] == Some(column) {
            self.batch.keys_of_modules(values)?
        } else {
            profile::decision("a batch's every document made by a read of a relation no document routes (decision 0494)", || format!("{} column {column} (document column {:?})", self.batch.rows.names[self.rel], by.column()));
            return None;
        };
        let before = self.batch.rows.groups();
        let groups = self.batch.groups_of(keys);
        if self.batch.rows.groups() > before {
            profile::decision("a batch's documents made by a probe (decision 0494)", || format!("{} column {column}", self.batch.rows.names[self.rel]));
        }
        let r = self.batch.made_so_far(self.rel);
        let mut rows = Vec::new();
        for g in groups {
            rows.extend(g.ranges[self.rel].clone().filter(|&row| values.contains(&Source::value(r.as_ref(), row, column)) && keep(&r, row)));
        }
        Some((r, rows))
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
        let r = self.batch.made_so_far(self.rel);
        if row < Source::len(r.as_ref()) { Source::row_at(r.as_ref(), row, columns, out) } else { self.target().row_at(row, columns, out) }
    }
    fn for_rows(&self, columns: &[usize], each: &mut dyn FnMut(usize, &[i64])) {
        self.target().for_rows(columns, each)
    }
    fn lookup_read(&self, _column: usize, _value: i64, _columns: &[usize], _each: &mut dyn FnMut(usize, &[i64])) -> bool {
        if let Some(t) = self.done() {
            return t.lookup_read(_column, _value, _columns, _each);
        }
        let Some((r, rows)) = self.routed(_column, std::slice::from_ref(&_value), |_, _| true) else {
            return self.target().lookup_read(_column, _value, _columns, _each);
        };
        let mut v = vec![0; _columns.len()];
        for row in rows {
            Source::row_at(r.as_ref(), row, _columns, &mut v);
            _each(row, &v);
        }
        true
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
        // Before its rows are made, by its kept statistics where they were given, as the made relation estimates
        // (`Relation::estimated_matches`, of its rows and the column's distinct values): no document made for it.
        match (self.done(), self.batch.rows.counted(self.rel), self.batch.rows.distinct(self.rel, column)) {
            (None, Some(n), Some(distinct)) => {
                let one = n.div_ceil(distinct.max(1));
                if batch { one * 8 } else { one }
            }
            _ => self.target().estimated_matches(column, batch),
        }
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
    /// The documents' rows by group (decision 0494): a group of theirs alone holds theirs and no other; in a group
    /// with others, theirs are those its routing column names (none for a relation not routed).
    fn rows_of_documents(&self, docs: &std::collections::BTreeSet<u32>) -> Option<Vec<usize>> {
        let at: BTreeSet<usize> = docs.iter().filter_map(|&k| self.batch.docs.position(k)).collect();
        let by = self.by();
        let groups = self.batch.groups_of(docs.iter().copied());
        let r = self.batch.made_so_far(self.rel);
        let mut rows = Vec::new();
        for g in groups {
            let range = g.ranges[self.rel].clone();
            if g.docs.iter().all(|i| at.contains(i)) {
                rows.extend(range);
            } else {
                let c = by.own_column()?;
                rows.extend(range.filter(|&row| by.document(Source::value(r.as_ref(), row, c)).is_some_and(|k| docs.contains(&k))));
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
        // The document's group: its rows whose `column` is one of `values` (a group may hold others too).
        let groups = self.batch.groups_of(u32::try_from(_doc).ok());
        let r = self.batch.made_so_far(self.rel);
        let by = self.by();
        let mut rows = Vec::new();
        for g in groups {
            rows.extend(g.ranges[self.rel].clone().filter(|&row| {
                _values.contains(&Source::value(r.as_ref(), row, _column)) && by.own_column().is_none_or(|c| by.document(Source::value(r.as_ref(), row, c)) == u32::try_from(_doc).ok())
            }));
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
    #[cfg(any(target_os = "macos", target_os = "linux"))]
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
            && self.by().own_column() == Some(_column)
            && let (Some(first), Some(last)) = (self.by().document(lo), self.by().document(hi))
        {
            let keys = self.batch.docs.keys().iter().copied().filter(|k| (first..=last).contains(k));
            let groups = self.batch.groups_of(keys.collect::<Vec<_>>());
            let r = self.batch.made_so_far(self.rel);
            let within = |v: i64| _lower.is_none_or(|(lo, inc)| if inc { v >= lo } else { v > lo }) && _upper.is_none_or(|(hi, inc)| if inc { v <= hi } else { v < hi });
            let mut rows = Vec::new();
            for g in groups {
                rows.extend(g.ranges[self.rel].clone().filter(|&row| within(Source::value(r.as_ref(), row, _column))));
            }
            rows.sort_unstable();
            return Some(rows);
        }
        self.target().range_lookup(_column, _lower, _upper)
    }
    fn lookup(&self, column: usize, values: &[i64]) -> Option<Vec<usize>> {
        if let Some(t) = self.done() {
            return t.lookup(column, values);
        }
        self.routed(column, values, |_, _| true).map(|(_, rows)| rows).or_else(|| self.target().lookup(column, values))
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
        match self.routed(_column, std::slice::from_ref(&_value), |_, _| true) {
            Some((_, rows)) => rows.first().copied(),
            None => self.target().key_row(_column, _value),
        }
    }
    fn lookup_eq(&self, _column: usize, _value: i64, _eqs: &[(usize, i64)]) -> Option<Vec<usize>> {
        if let Some(t) = self.done() {
            return t.lookup_eq(_column, _value, _eqs);
        }
        self.routed(_column, std::slice::from_ref(&_value), |r, row| _eqs.iter().all(|&(c, v)| Source::value(r, row, c) == v))
            .map(|(_, rows)| rows)
            .or_else(|| self.target().lookup_eq(_column, _value, _eqs))
    }
    fn value(&self, row: usize, column: usize) -> i64 {
        if let Some(t) = self.done() {
            return t.value(row, column);
        }
        let r = self.batch.made_so_far(self.rel);
        if row < Source::len(r.as_ref()) { Source::value(r.as_ref(), row, column) } else { self.target().value(row, column) }
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

/// A text test's relation (decision 0037) over a batch (decision 0494): a test of a string. A probe tests the probed
/// strings and a scan the batch's strings, every document made; each string is tested once a run, against every test
/// at once (`Tested`). Nothing of it changes with an edit. In a batch whose documents are made by probes
/// (`Batch::lazy`) a join probes it.
pub(super) struct TextTest {
    batch: Arc<Batch>,
    k: usize,
    tested: Arc<Tested>,
    /// Whether a scan has tested the batch's strings: shared by the batch's tests, so it is scanned once for them all.
    scanned: Arc<std::sync::atomic::AtomicBool>,
    /// Every string of the batch that passes, the probed first (their rows kept): made by the first scan.
    whole: OnceLock<Relation>,
}

/// The text tests' answers over a run's strings (decision 0037): shared by every test of a batch and of the batches
/// it grows into (decision 0431 §6). A string probed or scanned is tested against every test at once
/// (`Vocabulary::rows`), and each test's passing strings kept in its own rows.
pub(super) struct Tested {
    tests: Arc<crate::code_rule_dsl::logic::strings::Vocabulary>,
    atoms: Arc<crate::content_store::atoms::Atoms>,
    state: parking_lot::Mutex<TestedState>,
}

/// Every string tested, and each test's passing strings in row order with each one's row.
struct TestedState {
    tested: HashSet<i64>,
    passed: Vec<Passed>,
}

#[derive(Default)]
struct Passed {
    rows: Vec<i64>,
    at: HashMap<i64, usize>,
}

impl Tested {
    pub(super) fn new(tests: &Arc<crate::code_rule_dsl::logic::strings::Vocabulary>, atoms: Arc<crate::content_store::atoms::Atoms>) -> Self {
        let passed = (0..tests.tests.len()).map(|_| Passed::default()).collect();
        Self { tests: Arc::clone(tests), atoms, state: parking_lot::Mutex::new(TestedState { tested: HashSet::new(), passed }) }
    }
    /// Whether these are the answers of `tests`.
    pub(super) fn of(&self, tests: &Arc<crate::code_rule_dsl::logic::strings::Vocabulary>) -> bool {
        Arc::ptr_eq(&self.tests, tests)
    }
    /// Test the strings not tested yet against every test, outside the lock, and keep each test's passing ones.
    fn test(&self, strings: Vec<String>, symbols: Vec<u32>) {
        let (strings, symbols): (Vec<String>, Vec<u32>) = {
            let s = self.state.lock();
            strings.into_iter().zip(symbols).filter(|(_, sym)| !s.tested.contains(&i64::from(*sym))).unzip()
        };
        if strings.is_empty() {
            return;
        }
        let passing = self.tests.rows(&strings, &symbols);
        let mut s = self.state.lock();
        s.tested.extend(symbols.iter().map(|&sym| i64::from(sym)));
        for (p, rows) in s.passed.iter_mut().zip(passing) {
            for v in rows {
                if !p.at.contains_key(&v) {
                    p.at.insert(v, p.rows.len());
                    p.rows.push(v);
                }
            }
        }
    }
    /// Scan `batch`'s strings, every document made, unless `scanned` says a scan has.
    fn scan(&self, batch: &Batch, scanned: &std::sync::atomic::AtomicBool) {
        if scanned.load(Ordering::Acquire) {
            return;
        }
        let (strings, symbols) = batch.vocabulary();
        self.test(strings, symbols);
        scanned.store(true, Ordering::Release);
    }
}

impl TextTest {
    /// Test `k` of `tested`'s tests over `batch`; `scanned`, the batch's tests' one flag.
    pub(super) fn new(batch: &Arc<Batch>, k: usize, tested: &Arc<Tested>, scanned: &Arc<std::sync::atomic::AtomicBool>) -> Self {
        Self { batch: Arc::clone(batch), k, tested: Arc::clone(tested), scanned: Arc::clone(scanned), whole: OnceLock::new() }
    }
    /// The answers it shares.
    pub(super) fn tested(&self) -> &Arc<Tested> {
        &self.tested
    }
    /// The batch's passing strings, made outside the cell (they make documents on the pool, decision 0494's
    /// `ensure`), then kept.
    fn whole(&self) -> &Relation {
        if let Some(w) = self.whole.get() {
            return w;
        }
        self.tested.scan(&self.batch, &self.scanned);
        let s = self.tested.state.lock();
        let r = Relation::from_rows(&["s"], &mut s.passed[self.k].rows.iter().map(|&v| vec![v]));
        drop(s);
        self.whole.get_or_init(|| r)
    }
    /// The rows of the probed strings that pass, each tested once.
    fn probe(&self, values: &[i64]) -> Vec<usize> {
        let fresh: Vec<i64> = {
            let s = self.tested.state.lock();
            let mut fresh: Vec<i64> = values.iter().copied().filter(|v| !s.tested.contains(v)).collect();
            fresh.sort_unstable();
            fresh.dedup();
            fresh
        };
        let (mut strings, mut symbols) = (Vec::new(), Vec::new());
        for &v in &fresh {
            if let Ok(sym) = u32::try_from(v)
                && let Ok(Some(text)) = self.tested.atoms.value(sym)
            {
                strings.push(text);
                symbols.push(sym);
            }
        }
        self.tested.test(strings, symbols);
        let mut s = self.tested.state.lock();
        // A value no atom names is tested too: it passes nothing.
        s.tested.extend(fresh);
        let at = &s.passed[self.k].at;
        values.iter().filter_map(|v| at.get(v).copied()).collect()
    }
}

impl Source for TextTest {
    fn columns(&self) -> Vec<String> {
        vec!["s".to_owned()]
    }
    /// Its rows as its kept statistics count them where given (`Batch::give_totals`), as a made relation's
    /// (`MadeRows::len`): a run that only counts them tests no string (Q332). Else every string's.
    fn len(&self) -> usize {
        match (self.whole.get(), self.batch.given_rows(&self.tested.tests.tests[self.k].0)) {
            (Some(w), _) => w.len(),
            (None, Some(n)) => n,
            (None, None) => self.whole().len(),
        }
    }
    fn estimated_len(&self) -> usize {
        match self.whole.get() {
            Some(w) => w.len(),
            None if self.batch.lazy() => self.tested.state.lock().passed[self.k].rows.len().max(1),
            None => self.whole().len(),
        }
    }
    /// A probed string's row as probed (the whole keeps them first, in their order); a row past them, the whole's.
    fn value(&self, row: usize, column: usize) -> i64 {
        if let Some(w) = self.whole.get() {
            return Source::value(w, row, column);
        }
        let probed = self.tested.state.lock().passed[self.k].rows.get(row).copied();
        match probed {
            Some(s) => s,
            None => Source::value(self.whole(), row, column),
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
