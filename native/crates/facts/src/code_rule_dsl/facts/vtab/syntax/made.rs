//! A batch's documents and rows, each made when first read (decision 0494).
//!
//! A batch holds its documents unmade. A document's projection (its rank index, which the views read) is made on its
//! first read (`Docs`). Its rows, the relations the batch makes off the tapes (`text`, `op`, the matchers'), are made
//! in a group with the other documents the read needs (`Rows`), numbered after the rows of the groups made before it.
//! A batch read whole from its first read makes one group, in key order: one relation per relation, as an eager batch.

use super::*;
#[cfg(any(target_os = "macos", target_os = "linux", windows))]
use crate::code_rule_dsl::facts::vtab::DeviceView;
use crate::code_rule_dsl::facts::vtab::{ReadRow, RowCells, RowStream};
use crate::code_rule_dsl::logic::profile;
use std::collections::HashSet;
use std::ops::Range;
use std::sync::atomic::{AtomicUsize, Ordering};

mod reads;
pub use reads::RowReads;

/// A document's views' counts (decision 0494): its nodes, fielded nodes, siblings after it, places, tokens, spread
/// nodes (`spread`'s), the token order's places (`tokens`: with a token, one before, one after), `op`'s rows,
/// the nodes with a numbered tape parent and the root (`Item::Child`: `tape_rows`' `structural_before`,
/// `rr_token_named`), and `text`'s rows (`Projected::text_rows`), as its projection has them; the documents' table
/// keeps them.
pub type ViewCounts = [u32; COUNTS];
/// How many views' counts a document has (`ViewCounts`).
pub const COUNTS: usize = 12;

/// A document's row of the documents' table's counts (`Batch::kept_counts`, `retain::put_base_counts`): its key, path,
/// digest, views' counts and facts' size, and whether it is a file's own tree (a guest's path is its host's).
pub struct Kept {
    pub key: u32,
    pub path: String,
    pub digest: [u8; 32],
    pub counts: ViewCounts,
    pub size: FactsSize,
    pub file: bool,
    /// A host file's tree's or program's host and file (decision 0512), and its tape tag: what a warm run makes it
    /// pending with (`syntax::Pending::with_guest`).
    pub guest: Option<(super::Hosted, u8)>,
}

/// A projection's views' counts.
pub(super) fn view_counts(p: &Projected) -> ViewCounts {
    use super::tokens::TokRel;
    [p.len(), p.fielded(), p.after_siblings(), p.locs(), p.tokens(), p.spreads(), p.token_rows(TokRel::First), p.token_rows(TokRel::Before), p.token_rows(TokRel::After), p.ops(), p.parented(), p.text_rows()]
}

/// A batch's views' directory with its search layer's keys (`view_reads::ViewDir`, decision 0353 step 3 on the CPU), in
/// `block`: `n` rows, each its document's key, `keys` in order; the first rows are written when counted
/// (`Batch::firsts`), and each document's reads layer on its first read (`Docs::reads`). Its homes are fixed: row
/// `i`'s tape in home `2i`, its lines in `2i + 1`, the 16z cache in `2n` (`ViewDir::cache` is `2n + 1` once it is
/// made), the codes' directory in `2n + 1` (`ViewDir::codes`: the binary's, decision 0697; its device address
/// written with the first row the device reads). Beside it, each document's views' counts as the documents' table gave them (`Given`, `[counts, 1]`, or
/// zeros where none was given). Its offset in the block, and the given counts'.
pub(super) fn directory(block: &Block, docs: impl Iterator<Item = (u32, Option<ViewCounts>)>, n: usize) -> (usize, usize) {
    use rerail_clif::view_reads::{ViewDir, ViewHome, ViewRow};
    let rows: *mut ViewRow = block.alloc_t(n.max(1));
    let given: *mut Given = block.alloc_t(n.max(1));
    let homes: *mut ViewHome = block.alloc_t(2 * n + 2);
    // Its documents are in key order (`Docs::new`, `Docs::grown`): their order by key is their rows'.
    let by_key: *mut u32 = block.alloc_t(n.max(1));
    let dir: *mut ViewDir = block.alloc_t(1);
    let mut k = 0;
    for (key, counts) in docs.take(n) {
        // SAFETY: row `k` of the rows and the given counts just allocated in `block`, and its two homes, and its place in the order by key.
        unsafe {
            rows.add(k).write(ViewRow { key, homes: [2 * k as u32, 2 * k as u32 + 1], ..Default::default() });
            given.add(k).write(counts.map_or([0; COUNTS + 1], |c| {
                let mut g = [1; COUNTS + 1];
                // copy: not block data (a document's view counts as given, into its directory row)
                g[..COUNTS].copy_from_slice(&c);
                g
            }));
            by_key.add(k).write(k as u32);
            homes.add(2 * k).write(ViewHome::default());
            homes.add(2 * k + 1).write(ViewHome::default());
        }
        k += 1;
    }
    assert_eq!(k, n, "a key for every document");
    // SAFETY: the cache's and the codes' homes, just allocated.
    unsafe {
        homes.add(2 * n).write(ViewHome::default());
        homes.add(2 * n + 1).write(ViewHome { host: rerail_tape_format::codes_dir::bytes().as_ptr() as u64, gpu: 0 });
    }
    let at = block.offset_of(dir);
    // SAFETY: the directory just allocated in `block`.
    unsafe {
        dir.write(ViewDir { at: at as u64, ndocs: n as u32, nhomes: 2 * n as u32 + 2, rows: block.offset_of(rows) as u64, homes: block.offset_of(homes) as u64, by_key: block.offset_of(by_key) as u64, codes: 2 * n as u32 + 1, ..Default::default() })
    };
    (at, block.offset_of(given))
}

/// A document's views' counts as given (`directory`): the counts, then 1 where given.
type Given = [u32; COUNTS + 1];

/// A batch's documents (decision 0494): each one's projection made on its first read; its key and tag known before.
pub struct Docs {
    /// Shared with the batches it grows into (decision 0431 §6): a document is made once, whichever reads it first.
    slots: Vec<Arc<Slot>>,
    /// The views' directory's offset in the batch's block (`view_reads::ViewDir`, decision 0353 step 3 on the CPU):
    /// its search layer, each document's key from the start and its first rows once counted (`Batch::firsts`).
    dir: usize,
    /// Each document's views' counts, where given (the documents' table's): their offset in the batch's block
    /// (`directory`).
    given: usize,
    context: Context,
    /// How many projections are made (R2's tests read it).
    made: AtomicUsize,
    /// The directory's reads layer, each row written on its document's first read through the views (`reads`).
    layer: reads::Layer,
}

struct Slot {
    input: Input,
    made: OnceLock<MadeDoc>,
}

/// Whether two batches' inputs are one document as it is: key, digest, path and tag (decisions 0616, 0683).
fn same_input(a: &Input, b: &Input) -> bool {
    a.key() == b.key() && a.digest() == b.digest() && a.path() == b.path() && a.tag() == b.tag()
}

/// A batch's made documents (decision 0683): their slots, each a projection made once, with the projection they were
/// made with. A chunk's batch hands them to the run's corpus batch, which shares the slot of each document it holds
/// as it is where its projection makes a document as the chunk's did (`Making::alike`), so no document of a run is
/// projected twice. Each projection keeps its block (`cells::Desc`), as a renewed batch's do (decision 0616).
#[derive(Clone)]
pub struct Projections {
    slots: Vec<Arc<Slot>>,
    pub(super) projection: Arc<Projection>,
}

impl Projections {
    /// How many made documents it holds.
    pub fn len(&self) -> usize {
        self.slots.len()
    }
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }
}

/// What of a projection shapes a document's projection made with it (`Docs::project`), as the relations it reads
/// answer it: the rest of the projection but what it reads shapes it as itself. Two projections whose `Making`s and
/// all else but their reads are equal make every document alike (`alike`, decision 0683).
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Making {
    /// `syntax_error` is read: a document's verdict is made, and one that does not parse withheld.
    syntax_error: bool,
    /// A token view is read (`tokens::VIEWS`): the projection numbers its tokens in order.
    token_order: bool,
    /// `text` is read: the walk marks the nodes with text (`View::Text`).
    text: bool,
    /// A source or line span's text is read (`Projection::wants_span_text`).
    source_text: bool,
    line_text: bool,
    /// `end_point` is read: each node's end is among its positions.
    end_point: bool,
}

impl Making {
    pub(super) fn of(p: &Projection) -> Self {
        use crate::code_rule_dsl::logic::front::ast::SpanIn;
        let core = p.shape == Shape::Core;
        Making {
            syntax_error: p.wants("syntax_error"),
            token_order: core && super::tokens::VIEWS.iter().any(|r| p.wants(r)),
            text: core && p.wants("text"),
            source_text: p.wants_span_text(SpanIn::Source),
            line_text: p.wants_span_text(SpanIn::Line),
            end_point: core && p.wants("end_point"),
        }
    }
    /// Whether a batch of `b` makes each document's projection as a batch of `a` does.
    pub(super) fn alike(a: &Projection, b: &Projection) -> bool {
        let bare = |p: &Projection| {
            let mut p = p.clone();
            p.reads = None;
            p.compiled_rx = Default::default();
            format!("{p:?}")
        };
        Self::of(a) == Self::of(b) && bare(a) == bare(b)
    }
}

/// A made document: its projection, whether the policy is withheld it (it does not parse), and its first error.
pub(super) struct MadeDoc {
    pub(super) p: Projected,
    pub(super) withheld: bool,
    pub(super) error: Option<(u32, String)>,
}

/// What a document's projection is made with.
pub(super) struct Context {
    /// The cache the documents' 16z blocks are decoded into (`super::batch_cache`).
    pub(super) cache: Option<Arc<rerail_tape_format::f16::ZCache>>,
    pub(super) dialects: Arc<Dialects>,
    pub(super) projection: Arc<Projection>,
    /// Which nodes have text (`View::Text`), by dialect tag, where the projection reads `text`: each dialect's tables
    /// made once for the batch (`core_rr::TextRows`).
    pub(super) text_rows: Vec<Option<super::core_rr::TextRows>>,
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
    /// The cache their 16z blocks are decoded into (`super::batch_cache`): the batch's.
    pub(super) fn cache(&self) -> Option<&Arc<rerail_tape_format::f16::ZCache>> {
        self.context.cache.as_ref()
    }

    /// `documents`, sorted by key and distinct, with the views' counts `given` of some, by key.
    pub(super) fn new(documents: Vec<Input>, context: Context, given: &HashMap<u32, ViewCounts>) -> Self {
        let (dir, given) = directory(&context.block, documents.iter().map(|d| (d.key(), given.get(&d.key()).copied())), documents.len());
        let slots: Vec<Arc<Slot>> = documents.into_iter().map(|input| Arc::new(Slot { input, made: OnceLock::new() })).collect();
        let layer = reads::Layer::new(slots.len());
        Self { slots, dir, given, context, made: AtomicUsize::new(0), layer }
    }
    /// `prior`'s documents and `documents` (sorted by key, none of `prior`'s), in key order: `prior`'s shared, as
    /// made there (decision 0431 §6), `documents` unmade, with the views' counts `given` of some. Each one's index here
    /// is returned for `prior`'s, in its order.
    pub(super) fn grown(prior: &Docs, documents: Vec<Input>, context: Context, given: &HashMap<u32, ViewCounts>) -> (Self, Vector<usize>) {
        let new_slots: Vec<Arc<Slot>> = documents.into_iter().map(|input| Arc::new(Slot { input, made: OnceLock::new() })).collect();
        let n = prior.slots.len() + new_slots.len();
        let (mut slots, mut counts, mut at) = (Vec::with_capacity(n), Vec::with_capacity(n), Vector::with_capacity(prior.slots.len()));
        let (mut i, mut j) = (0, 0);
        while i < prior.slots.len() || j < new_slots.len() {
            if j == new_slots.len() || (i < prior.slots.len() && prior.key(i) < new_slots[j].input.key()) {
                at.push(slots.len());
                slots.push(Arc::clone(&prior.slots[i]));
                counts.push(prior.given(i));
                i += 1;
            } else {
                slots.push(Arc::clone(&new_slots[j]));
                counts.push(given.get(&new_slots[j].input.key()).copied());
                j += 1;
            }
        }
        let (dir, given) = directory(&context.block, slots.iter().map(|s| s.input.key()).zip(counts), slots.len());
        let layer = reads::Layer::new(slots.len());
        (Self { slots, dir, given, context, made: AtomicUsize::new(0), layer }, at)
    }
    /// `documents` (sorted by key, distinct), those `held` holds as they are (key, digest and path) sharing its slots,
    /// as made there (decision 0616): their projections are not made again. Returned with each of `held`'s documents'
    /// index here, where it is shared.
    pub(super) fn renewed(held: &Docs, documents: Vec<Input>, context: Context, given: &HashMap<u32, ViewCounts>) -> (Self, Vec<Option<usize>>) {
        let mut docs = Self::new(documents, context, given);
        let mut at = vec![None; held.len()];
        for i in 0..docs.len() {
            let Some(j) = held.position(docs.key(i)) else { continue };
            if same_input(&held.slots[j].input, &docs.slots[i].input) {
                docs.slots[i] = Arc::clone(&held.slots[j]);
                at[j] = Some(i);
            }
        }
        (docs, at)
    }
    /// Each of `from`'s made documents that it holds as they are (key, digest, path and tag), and has not made, shares
    /// that slot, as made there (decision 0683): its projection is not made again. How many are shared.
    pub(super) fn share(&mut self, from: &Projections) -> usize {
        let mut shared = 0;
        for slot in &from.slots {
            let Some(i) = self.position(slot.input.key()) else { continue };
            if self.slots[i].made.get().is_none() && same_input(&slot.input, &self.slots[i].input) {
                self.slots[i] = Arc::clone(slot);
                shared += 1;
            }
        }
        shared
    }
    /// Its documents made so far, with the projection they were made with (`Projections`, decision 0683).
    pub(super) fn made_slots(&self) -> Vec<Arc<Slot>> {
        self.slots.iter().filter(|s| s.made.get().is_some()).cloned().collect()
    }
    /// The files' lines its guests are placed by (`Context::lines_of`), which a batch renewed from it shares.
    pub(super) fn lines_of(&self) -> &Arc<parking_lot::RwLock<HashMap<u32, Arc<crate::code_rule_dsl::lines::LineTable>>>> {
        &self.context.lines_of
    }
    /// Whether the documents' views' counts were given: the views' row space is known with none made but those
    /// whose document is made already (an edit's changed ones, `Input::Made`), whose counts are their own
    /// projection's (`counts`). A batch given none (a cold run's) is not.
    pub fn given_all(&self) -> bool {
        let mut any = false;
        for (i, s) in self.slots.iter().enumerate() {
            match self.given(i) {
                Some(_) => any = true,
                None if s.input.is_made() => {}
                None => return false,
            }
        }
        any
    }
    /// Whether every document's views' counts were given, an edit's changed ones too: none to keep anew.
    pub fn given_every(&self) -> bool {
        (0..self.slots.len()).all(|i| self.given(i).is_some())
    }
    /// Document `i`'s views' counts as the documents' table gave them, in the batch's block (`directory`).
    fn given(&self, i: usize) -> Option<ViewCounts> {
        assert!(i < self.slots.len(), "document {i} of {}", self.slots.len());
        // SAFETY: the given counts `directory` wrote in the batch's block, which the context holds; `i` is one of its.
        let g = unsafe { self.context.block.ptr::<Given>(self.given).add(i).read() };
        (g[COUNTS] != 0).then(|| std::array::from_fn(|k| g[k]))
    }
    /// Document `i`'s views' counts: those given, else its projection's (made if it is not).
    pub fn counts(&self, i: usize) -> ViewCounts {
        match (self.slots[i].made.get(), self.given(i)) {
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
    /// Document `i`'s key, made or not: its directory row's.
    pub fn key(&self, i: usize) -> u32 {
        assert!(i < self.slots.len(), "document {i} of {}", self.slots.len());
        // SAFETY: the directory `new` made in the batch's block, which the context holds; row `i` is one of its.
        unsafe { rerail_clif::view_reads::ViewDir::key_of(self.dir(), i as u32) }
    }
    /// Every document's key, in order.
    pub fn keys(&self) -> impl ExactSizeIterator<Item = u32> + '_ {
        (0..self.slots.len()).map(|i| self.key(i))
    }
    /// The views' directory, in the batch's block.
    pub fn dir(&self) -> *const rerail_clif::view_reads::ViewDir {
        self.context.block.ptr::<rerail_clif::view_reads::ViewDir>(self.dir).cast_const()
    }
    /// Its offset in the batch's block.
    pub fn dir_at(&self) -> usize {
        self.dir
    }
    /// The first document whose key is `key` or past it.
    pub fn first_from(&self, key: u32) -> usize {
        let (mut lo, mut hi) = (0, self.slots.len());
        while lo < hi {
            let m = (lo + hi) / 2;
            if self.key(m) < key { lo = m + 1 } else { hi = m }
        }
        lo
    }
    /// Document `i`'s tape tag (`Facts::tag`), made or not.
    pub fn tag(&self, i: usize) -> u8 {
        self.slots[i].input.tag()
    }
    /// The index of the document of key `key`.
    pub fn position(&self, key: u32) -> Option<usize> {
        self.position_from(key, rerail_clif::view_reads::NO_DOC)
    }
    /// `position`, the document `memo` tried first (the directory's search, `ViewDir::doc_of_key`).
    pub fn position_from(&self, key: u32, memo: u32) -> Option<usize> {
        // SAFETY: as `key`.
        let d = unsafe { rerail_clif::view_reads::ViewDir::doc_of_key(self.dir(), key, memo) };
        (d != rerail_clif::view_reads::NO_DOC).then_some(d as usize)
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
    pub fn guest(&self, i: usize) -> Option<super::GuestOf<'_>> {
        self.slots[i].input.guest()
    }
    /// Document `i`'s file (decision 0431 §4): a program's is its file's, any other document's its own key.
    pub fn file(&self, i: usize) -> u32 {
        Guest::file_key(self.guest(i), self.key(i))
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
    /// The address the read memos are kept by (`Batch::doc`): its directory's.
    pub(super) fn addr(&self) -> usize {
        self.dir() as usize
    }
    /// Whether document `i` is withheld (it does not parse, and the policy withholds it): it has no nodes' rows.
    pub fn withheld(&self, i: usize) -> bool {
        self.made(i).withheld
    }
    /// Document `i`, made if it is not.
    pub(super) fn made(&self, i: usize) -> &MadeDoc {
        self.slots[i].made.get_or_init(|| {
            self.made.fetch_add(1, Ordering::Relaxed);
            let made = self.project(i);
            // Its counts as given (the documents' table's): the views' rows were placed by them.
            if let Some(given) = self.given(i)
                && given != view_counts(&made.p)
            {
                self.fail(format!(
                    "{}: its views' counts (nodes, fielded, siblings, places) are {:?}, the documents' table gave {given:?} (decision 0494)",
                    self.slots[i].input.path(),
                    view_counts(&made.p)
                ));
            }
            // A test's: the document `RERAIL_TEST_FAIL_DOC` names fails as one whose counts differ, so a run's
            // readers are shown to stop on a batch's failure (`a_batchs_failure_fails_a_native_chunk`).
            #[cfg(any(test, feature = "test-hooks"))]
            if std::env::var_os("RERAIL_TEST_FAIL_DOC").is_some_and(|p| self.slots[i].input.path().ends_with(&*p.to_string_lossy())) {
                self.fail(format!("{}: failed by the test (decision 0494)", self.slots[i].input.path()));
            }
            made
        })
    }
    /// Document `i`'s nodes as its projection would number them, by their codes as the `node` view reads them
    /// (`Dialect::code`): `(nodes, each code's nodes)`, read off its index (decision 0656 step 4) with none made.
    /// `None` where the index does not answer for the projection. It does where the projection numbers every named
    /// node: core.rerail's shape with no anchors, a file's own tree (no guest), its root named, every named node under
    /// a named parent (`DocIndex::named_all_rooted`), and the document not withheld (no verdict, as `project` asks).
    pub(super) fn indexed_codes(&self, i: usize) -> Option<(u32, Vec<(u16, u32)>)> {
        let projection = &*self.context.projection;
        if projection.shape != Shape::Core || projection.node_anchors.is_some() {
            return None;
        }
        let d = self.input(i);
        let tag = crate::code_rule_dsl::facts::core::facts_tag(&d.facts);
        if d.guest.is_some() {
            return None;
        }
        if tag == TEXT || d.facts.is_empty() {
            return Some((0, Vec::new()));
        }
        let dialect = self.context.dialects.get(tag)?;
        self.read_facts(i, |f| {
            if !f.named(0) {
                return None;
            }
            let index = f.index();
            if !index.named_all_rooted() {
                return None;
            }
            if projection.wants("syntax_error") && core_rr::verdict(f, dialect).ok()?.is_some() {
                return None;
            }
            let codes: Vec<(u16, u32)> = index.counts().filter(|&(c, _)| index.named(c)).map(|(c, n)| (kind_code(f.code_entry(c).0), n)).collect();
            Some((codes.iter().map(|c| c.1).sum(), codes))
        })
    }
    /// `f` over document `i`'s facts, made or not, read through the batch's cache (or, a document whose blocks its
    /// entries do not hold, in a batch of its own, `Facts::alone`, as `Projected::narrowed` reads them).
    pub(super) fn read_facts<R>(&self, i: usize, f: impl FnOnce(FactsIn<'_>) -> R) -> R {
        let facts = &self.input(i).facts;
        match self.context.cache.as_deref().filter(|c| facts.z_tape().is_none_or(|t| c.entry_size() >= t.entry_size())) {
            Some(cache) => f(facts.read_in(Some(cache))),
            None => facts.alone(f),
        }
    }
    /// Document `i`'s projection (decision 0289 §3: in the batch's block), its verdict and its module.
    fn project(&self, i: usize) -> MadeDoc {
        let d = self.input(i);
        let projection = &*self.context.projection;
        let making = Making::of(projection);
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
            Some(dialect) if (core && (making.syntax_error || guest.is_some())) || (!core && projection.verdict) => {
                core_rr::verdict(d.facts.read_in(self.context.cache.as_deref()), dialect).unwrap_or_else(|raised| {
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
        let token_order = making.token_order;
        // `text`'s rows (`View::Text`): which nodes have text, and which positions.
        let reads_text = making.text;
        // The batch's tables for the dialect; one made here for a dialect the batch had none of when it was made.
        let made_here;
        let text_rows = match dialect.filter(|_| reads_text) {
            Some(dialect) => match self.context.text_rows.get(usize::from(tag)).and_then(Option::as_ref) {
                Some(rows) => Some(rows),
                None => {
                    made_here = super::core_rr::TextRows::new(&self.context.projection, dialect);
                    Some(&made_here)
                }
            },
            None => None,
        };
        let spans = super::TextSpans {
            tokens: projection.token_text || std::env::var("RERAIL_TOKEN_TEXT").is_ok_and(|v| v == "all"),
            source: making.source_text,
            lines: making.line_text,
        };
        let text = reads_text.then_some((text_rows, spans));
        let mut p = Projected::narrowed(d.key, Arc::clone(&d.facts), dialect, shape, withheld, placed, projection.node_anchors.as_ref().filter(|_| core), token_order, text, &self.context.block, self.context.cache.clone());
        // T's shape takes a program's whole tree, not as a guest, but its nodes are its file's too (Q324, decision 0512).
        p.set_file(Guest::file_key(d.guest.as_deref().map(Guest::of), d.key));
        drop(new_span);
        // A program that reads `end_point` has each node's end point among the positions, after the lines.
        let nodes = p.len();
        if making.end_point
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

/// Whether the views' first rows are in the directory (`Batch::firsts`): written once, from the documents' counts,
/// on the first read of any view's rows.
#[derive(Default)]
pub(super) struct Offsets {
    pub(super) counted: OnceLock<()>,
    /// The slots no view's counts give (`tape_rows::free_slots`), written first and alone: no document made.
    pub(super) free: OnceLock<()>,
}

/// A view's first rows by document, read in the batch's views' directory (decision 0353 step 3 on the CPU): document
/// `i`'s at `at(i)`, the view's rows in all at `at(len)` (`total`).
#[derive(Clone, Copy)]
pub struct Firsts<'a> {
    dir: *const rerail_clif::view_reads::ViewDir,
    k: usize,
    n: usize,
    _batch: std::marker::PhantomData<&'a Docs>,
}

impl Firsts<'_> {
    /// Over `docs`' directory, view slot `k` (`view_reads::ROWS`' order), its first rows written.
    pub(super) fn new(docs: &Docs, k: usize) -> Firsts<'_> {
        Firsts { dir: docs.dir(), k, n: docs.len(), _batch: std::marker::PhantomData }
    }
    /// Document `i`'s first row (`i == len()`: the rows in all).
    #[inline]
    pub fn at(&self, i: usize) -> usize {
        assert!(i <= self.n, "document {i} of {}", self.n);
        // SAFETY: the directory of the documents this borrows, its first rows written (`Batch::firsts`).
        unsafe { rerail_clif::view_reads::ViewDir::first_of(self.dir, self.k, i as u32) as usize }
    }
    /// The view's rows in all.
    #[inline]
    pub fn total(&self) -> usize {
        self.at(self.n)
    }
    /// How many documents.
    pub fn len(&self) -> usize {
        self.n
    }
    /// Document `i`'s rows.
    pub fn range(&self, i: usize) -> std::ops::Range<usize> {
        self.at(i)..self.at(i + 1)
    }
    /// The document whose rows hold `row`, `memo` tried first (the directory's search, `ViewDir::doc_of_row`).
    #[inline]
    pub fn doc_of(&self, row: usize, memo: u32) -> usize {
        // SAFETY: as `at`.
        unsafe { rerail_clif::view_reads::ViewDir::doc_of_row(self.dir, self.k as u32, row as u32, memo) as usize }
    }
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
    /// The strings its rows used, by index among the batch's, each once: kept where its batch keeps them
    /// (`Batch::keeps_strings`, a session's corpus batch, decision 0616).
    pub(super) used: Option<Vec<u32>>,
}

/// A group: documents whose rows were made together, and each relation's rows of them, a range of its block relation.
/// The columns of a made relation a group indexes (`Group::index`).
const INDEXED: usize = 8;

/// A group's index of one column of one relation (an index in its relation's block, decision 0353): nothing where
/// the group's rows are in the column's order, else its rows in that order (stable: equal values in row order). A
/// probe is a binary search. A hash map of a row list a value was most of a made relation's first probes' time.
pub(super) enum ColIndex {
    /// Runs of rows each in the column's order, whose values do not overlap, by their values: a group's documents'
    /// rows, each document's in node order (its node ids' high half is the document's key). A probe finds its run,
    /// and within it the row its values place it at (`Group::each_match`).
    Runs(Vec<SortedRun>),
    /// The rows in the column's order, and every `ORDER_FENCE`th of their values (`fence`): a probe searches the
    /// fence, contiguous, then at most `ORDER_FENCE` rows, a random read each (a node's ids are its granules', in no
    /// order of the walk's: `text`'s probes by node searched every row, 240 ns of a probe's 320).
    Order { order: Vec<u32>, fence: Vec<i64> },
}

/// Rows an `ColIndex::Order` fence entry stands for.
const ORDER_FENCE: usize = 16;

/// A run of a group's rows in a column's order (`ColIndex::Runs`): rows `start..end`, their values `first..=last`.
#[derive(Clone, Copy)]
pub(super) struct SortedRun {
    first: i64,
    last: i64,
    start: u32,
    end: u32,
}

/// The first of `0..n` where `below` is false (`below` true then false).
fn partition(n: usize, below: impl Fn(usize) -> bool) -> usize {
    let (mut lo, mut hi) = (0, n);
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if below(mid) { lo = mid + 1 } else { hi = mid }
    }
    lo
}

/// The first of `lo..hi` where `below` is false (`below` true then false), searched out from `near` (the last
/// probe's): a join that probes nodes in order finds its match a row or two on, in as many reads, not a search of
/// the whole group.
fn partition_near(lo: usize, hi: usize, near: usize, below: impl Fn(usize) -> bool) -> usize {
    if !(lo..hi).contains(&near) {
        return lo + partition(hi - lo, |i| below(lo + i));
    }
    let (from, to) = if below(near) {
        // Past `near`: steps doubling until one is not below.
        let (mut prev, mut step) = (near, 1);
        loop {
            let next = near + step;
            if next >= hi {
                break (prev + 1, hi);
            }
            if !below(next) {
                break (prev + 1, next);
            }
            prev = next;
            step *= 2;
        }
    } else {
        // At or before `near`: steps doubling back until one is below.
        let (mut prev, mut step) = (near, 1);
        loop {
            if near < lo + step {
                break (lo, prev);
            }
            let next = near - step;
            if below(next) {
                break (next + 1, prev);
            }
            prev = next;
            step *= 2;
        }
    };
    from + partition(to - from, |i| below(from + i))
}

#[cfg(test)]
mod partition_tests {
    use super::{partition, partition_near, ColIndex, Group, Relation, Source};

    /// A group of three documents' rows, each document's in node order and the documents not (its runs), and the
    /// same rows shuffled (its order): a probe of every value, from every last row, finds the rows a scan finds.
    #[test]
    fn a_groups_runs_and_order_find_a_scans_rows() {
        let doc = |d: i64, locals: &[i64]| locals.iter().map(move |&l| d << 32 | l).collect::<Vec<_>>();
        let runs: Vec<i64> = [doc(7, &[0, 1, 1, 2, 5, 6, 9]), doc(3, &[0, 2, 3, 3, 3, 4, 8, 10]), doc(5, &[1, 2, 4, 6, 7, 8, 11, 12])].concat();
        let mut shuffled = runs.clone();
        shuffled.swap(1, 20);
        shuffled.swap(4, 12);
        let mut h = 0x9e37_79b9u64;
        let scattered: Vec<i64> = (0..300).map(|_| { h ^= h << 13; h ^= h >> 7; h ^= h << 17; (h % 7) as i64 + 2 << 32 | (h >> 20) as i64 % 90 }).collect();
        for (values, is_runs) in [(runs, true), (shuffled, false), (scattered, false)] {
            let mut r = Relation::new(&["n", "s"]);
            r.push(&[-5, 0]);
            for (i, &v) in values.iter().enumerate() {
                r.push(&[v, i as i64]);
            }
            let range = 1..1 + values.len();
            let g = Group::new(rerail_blocks::vector![0], vec![range.clone()], None);
            assert_eq!(matches!(g.index(0, 0, &r), ColIndex::Runs(_)), is_runs);
            assert!(r.range_reader(range.clone(), 0).is_some(), "a group's rows in one part are read where they lie");
            let probes: Vec<i64> = values.iter().flat_map(|&v| [v - 1, v, v + 1]).chain([0, -5, i64::MAX, 4 << 32 | 1]).collect();
            for v in probes {
                let want: Vec<usize> = range.clone().filter(|&row| Source::value(&r, row, 0) == v).collect();
                for near in [0u32, 1, 5, 9, 17, 23, u32::MAX] {
                    let (mut got, mut at) = (Vec::new(), near);
                    g.each_match(0, 0, &r, v, &mut at, |row| got.push(row));
                    assert_eq!(got, want, "value {v:#x} from {near}, runs {is_runs}");
                }
            }
        }
    }

    /// From every start, in and out of the range, the search out from it finds what a whole search finds: over
    /// sorted values with runs of one value, every value probed, and the ones between.
    #[test]
    fn a_search_from_the_last_row_finds_the_whole_searchs_row() {
        let values: Vec<i64> = [0, 0, 1, 3, 3, 3, 4, 7, 7, 9, 10, 10, 10, 10, 12].to_vec();
        for (lo, hi) in [(0, values.len()), (2, 9), (5, 6), (4, 4)] {
            for v in -1..14 {
                let whole = lo + partition(hi - lo, |i| values[lo + i] < v);
                for near in 0..values.len() + 2 {
                    assert_eq!(partition_near(lo, hi, near, |i| values[i] < v), whole, "{lo}..{hi}, value {v}, from {near}");
                }
            }
        }
    }
}

pub(super) struct Group {
    pub(super) docs: Vector<usize>,
    /// Each relation's rows of the group, in `Rows::names`' order.
    pub(super) ranges: Vec<Range<usize>>,
    /// Per (relation, column) probed: the group's rows by their value there, made at the first probe (an index in
    /// its relation's block, decision 0353). A group's rows never change, so it stands. A probe without it walked the
    /// group's every row: a maintained join's probes of `docof`, one document's group each, were quadratic.
    /// One slot a (relation, column), `INDEXED` columns a relation, set once: a probe reads it with no lock.
    index: Box<[std::sync::OnceLock<ColIndex>]>,
    /// The strings its rows used (`Made::used`), where its batch keeps them: a renewed batch's vocabulary is its
    /// groups' (decision 0616).
    strings: Option<Arc<[u32]>>,
}

impl Group {
    fn new(docs: rerail_blocks::vector::Vector<usize>, ranges: Vec<Range<usize>>, strings: Option<Arc<[u32]>>) -> Self {
        let index = (0..ranges.len() * INDEXED).map(|_| std::sync::OnceLock::new()).collect();
        Group { docs, ranges, index, strings }
    }

    /// The group's rows of relation `rel` (held in `r`) whose `column` is a key of the index, made if it is not.
    fn index(&self, rel: usize, column: usize, r: &Relation) -> &ColIndex {
        assert!(column < INDEXED, "a made relation of at most {INDEXED} columns");
        self.index[rel * INDEXED + column].get_or_init(|| {
            let range = self.ranges[rel].clone();
            let read = r.range_reader(range.clone(), column);
            let at = |row: usize| read.as_ref().map_or_else(|| Source::value(r, row, column), |c| c.get(row as u32));
            // The rows' runs in the column's order, read once: kept as runs where they do not overlap, and are long.
            let mut runs = Vec::new();
            if !range.is_empty() {
                let (mut start, mut first, mut prev) = (range.start, at(range.start), at(range.start));
                for row in range.start + 1..range.end {
                    let v = at(row);
                    if v < prev {
                        runs.push(SortedRun { first, last: prev, start: start as u32, end: row as u32 });
                        (start, first) = (row, v);
                    }
                    prev = v;
                }
                runs.push(SortedRun { first, last: prev, start: start as u32, end: range.end as u32 });
            }
            runs.sort_unstable_by_key(|r| r.first);
            if 8 * runs.len() <= range.len().max(8) && runs.windows(2).all(|w| w[0].last < w[1].first) {
                return ColIndex::Runs(runs);
            }
            // Each value read once and the pairs sorted (ties by row: a stable order by value), as `Relation::index`
            // does: a sort reading a row's value at every comparison read each about 2 log n times.
            let mut pairs: Vec<(i64, u32)> = range.map(|row| (at(row), row as u32)).collect();
            pairs.sort_unstable();
            let fence = pairs.iter().step_by(ORDER_FENCE).map(|p| p.0).collect();
            ColIndex::Order { order: pairs.into_iter().map(|(_, row)| row).collect(), fence }
        })
    }
    /// The group's rows of `rel` (in `r`) whose `column` is `value`, in row order, each given to `each`.
    /// `near`: the row the last probe's search ended at (a cursor's), searched out from and left at this one's.
    fn each_match(&self, rel: usize, column: usize, r: &Relation, value: i64, near: &mut u32, mut each: impl FnMut(usize)) {
        let range = self.ranges[rel].clone();
        // The group's rows lie in one part: the column resolved once a probe, one load a row of the search.
        let read = r.range_reader(range.clone(), column);
        let at = |row: usize| read.as_ref().map_or_else(|| Source::value(r, row, column), |c| c.get(row as u32));
        match self.index(rel, column, r) {
            ColIndex::Runs(runs) => {
                // The run that can hold the value, then searched out from the row the values place it at (a made
                // relation's node ids are dense in their document: a row or two from it).
                let k = runs.partition_point(|r| r.last < value);
                let Some(&SortedRun { first, last, start, end }) = runs.get(k).filter(|r| r.first <= value) else { return };
                let (lo, hi) = (start as usize, end as usize);
                let guess = if last > first { lo + (((value - first) as u128 * (hi - lo - 1) as u128) / (last - first) as u128) as usize } else { lo };
                let from = partition_near(lo, hi, guess, |i| at(i) < value);
                *near = from as u32;
                let mut row = from;
                while row < hi && at(row) == value {
                    each(row);
                    row += 1;
                }
            }
            ColIndex::Order { order, fence } => {
                // The first fence entry at or past the value bounds the first row at or past it: after the entry
                // before, at or before this one.
                let f = fence.partition_point(|&v| v < value);
                let (lo, hi) = if f == 0 { (0, 0) } else { ((f - 1) * ORDER_FENCE + 1, (f * ORDER_FENCE).min(order.len())) };
                let from = lo + partition(hi - lo, |i| at(order[lo + i] as usize) < value);
                for &row in order[from..].iter().take_while(|&&row| at(row as usize) == value) {
                    each(row as usize);
                }
            }
        }
    }
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
    /// Each relation's column of its row's document's key, beside the node that routes it (`docof(n, d)`'s `d`): a
    /// read of it by a key is routed to the documents whose rows hold that key (`Batch::keys_of_documents`).
    key_column: Vec<Option<usize>>,
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
    /// Each document's group and the relations as that group left them, set once as the group goes on: a probe of a
    /// document's rows (`MadeRows::read_one`) reads them with no lock and no count taken. The relations are views
    /// of the latest's parts (`Relation::grown` shares its rows), so keeping them keeps no row the batch does not.
    published: Vec<OnceLock<Arc<Published>>>,
}

/// A group as it went on (`Rows::published`).
pub(super) struct Published {
    pub(super) group: Arc<Group>,
    pub(super) rels: Vec<Arc<Relation>>,
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
    docs: Vector<usize>,
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
    /// The column of a row's document's key in a relation routed by its node (`docof(n, d)`).
    fn key_column(name: &str, columns: &[String]) -> Option<usize> {
        match Self::of(name, columns) {
            Self::Id(c) => columns.iter().enumerate().position(|(i, n)| i != c && n == "d"),
            _ => None,
        }
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
    pub(super) fn new(documents: usize, names: Vec<String>, template: Vec<Arc<Relation>>, statics: Vec<&'static str>, prior: Option<(&HashMap<String, Arc<Relation>>, Vector<usize>)>, node_columns: &HashMap<String, Vec<usize>>) -> Self {
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
            state.groups.push(Arc::new(Group::new(docs, ranges, None)));
        }
        let by: Vec<ByDocument> = names.iter().zip(&template).map(|(n, t)| ByDocument::of(n, &Source::columns(t.as_ref()))).collect();
        let distinct = names.iter().map(|_| OnceLock::new()).collect();
        let path_column = names.iter().zip(&template).map(|(n, t)| ByDocument::path_column(n, &Source::columns(t.as_ref()))).collect();
        let module_column = names.iter().zip(&template).map(|(n, t)| ByDocument::module_column(n, &Source::columns(t.as_ref()))).collect();
        let key_column = names.iter().zip(&template).map(|(n, t)| ByDocument::key_column(n, &Source::columns(t.as_ref()))).collect();
        let also = names.iter().zip(&by).map(|(n, b)| node_columns.get(n).map_or_else(Vec::new, |cs| cs.iter().copied().filter(|&c| Some(c) != b.column()).collect())).collect();
        let mut this = Self { state: parking_lot::Mutex::new(state), made: parking_lot::Condvar::new(), makes: Default::default(), names, template, by, also, path_column, module_column, key_column, relations: statics.len(), statics, complete: OnceLock::new(), counted: OnceLock::new(), distinct, published: (0..documents).map(|_| OnceLock::new()).collect() };
        if let Some(g) = this.state.get_mut().groups.first().cloned() {
            let s = this.state.get_mut();
            let p = Arc::new(Published { group: g, rels: s.rels.clone() });
            for &i in &p.group.docs {
                let _ = this.published[i].set(Arc::clone(&p));
            }
        }
        this
    }
    /// The groups of `held` (the batch a session's last run made, decision 0616) every document of which this batch
    /// shares (`at`: each of `held`'s documents' index here), taken as made: each relation starts as the held
    /// relation's runs of their rows, read where they lie (`Relation::of_runs`), and the groups this batch makes go
    /// on after them. A run is a stretch of consecutive groups taken; at most `MAX_SEGMENTS - 1` parts a relation,
    /// so a relation's parts stay few as it grows (bound as one, decision 0353 step 6): past that, the runs of fewest rows are left, their documents
    /// made again. Nothing is taken where the relations are not the same. Returns the groups, documents and runs
    /// taken, and the groups left past the limit.
    pub(super) fn take(&self, held: &Rows, at: &[Option<usize>]) -> [usize; 4] {
        let same = self.names == held.names && self.template.iter().zip(&held.template).all(|(a, b)| Source::columns(a.as_ref()) == Source::columns(b.as_ref()));
        if !same {
            return [0; 4];
        }
        let h = held.state.lock();
        let takes = |g: &Group| g.strings.is_some() && !g.docs.is_empty() && g.docs.iter().all(|&i| at[i].is_some());
        // The stretches of consecutive groups taken: their rows are consecutive in every relation.
        let mut runs: Vec<Range<usize>> = Vec::new();
        for (g, group) in h.groups.iter().enumerate() {
            if !takes(group) {
                continue;
            }
            match runs.last_mut() {
                Some(r) if r.end == g => r.end = g + 1,
                _ => runs.push(g..g + 1),
            }
        }
        let span = |run: &Range<usize>, rel: usize| h.groups[run.start].ranges[rel].start..h.groups[run.end - 1].ranges[rel].end;
        let rows = |run: &Range<usize>| (0..self.names.len()).map(|rel| span(run, rel).len()).sum::<usize>();
        let limit = crate::code_rule_dsl::facts::rows::MAX_SEGMENTS - 1;
        let mut by_rows: Vec<usize> = (0..runs.len()).collect();
        by_rows.sort_by_key(|&r| std::cmp::Reverse(rows(&runs[r])));
        let mut parts = vec![0usize; self.names.len()];
        let mut chosen = vec![false; runs.len()];
        for r in by_rows {
            let more: Vec<usize> = (0..self.names.len()).map(|rel| h.rels[rel].parts_in(span(&runs[r], rel))).collect();
            if parts.iter().zip(&more).all(|(p, m)| p + m <= limit) {
                parts.iter_mut().zip(&more).for_each(|(p, m)| *p += m);
                chosen[r] = true;
            }
        }
        let left: usize = runs.iter().zip(&chosen).filter(|(_, c)| !**c).map(|(r, _)| r.len()).sum();
        let runs: Vec<Range<usize>> = runs.into_iter().zip(chosen).filter_map(|(r, c)| c.then_some(r)).collect();
        if runs.is_empty() {
            return [0, 0, 0, left];
        }
        let mut s = self.state.lock();
        let mut start = vec![0usize; self.names.len()];
        let (mut groups, mut docs) = (0, 0);
        for run in &runs {
            for group in &h.groups[run.clone()] {
                let ranges: Vec<Range<usize>> = (0..self.names.len())
                    .map(|rel| {
                        let from = span(run, rel).start;
                        start[rel] + group.ranges[rel].start - from..start[rel] + group.ranges[rel].end - from
                    })
                    .collect();
                let mine: Vector<usize> = group.docs.iter().map(|&i| at[i].expect("a taken group's document")).collect();
                let g = s.groups.len() as u32;
                for &i in &mine {
                    s.of[i] = Some(g);
                }
                docs += mine.len();
                groups += 1;
                s.groups.push(Arc::new(Group::new(mine, ranges, group.strings.clone())));
            }
            for (rel, at) in start.iter_mut().enumerate() {
                *at += span(run, rel).len();
            }
        }
        for rel in 0..self.names.len() {
            let columns = Source::columns(self.template[rel].as_ref());
            let pieces: Vec<(&Relation, Range<usize>)> = runs.iter().map(|run| (h.rels[rel].as_ref(), span(run, rel))).collect();
            s.rels[rel] = Arc::new(Relation::of_runs(&columns, &pieces));
        }
        let rels = s.rels.clone();
        for g in &s.groups[s.groups.len() - groups..] {
            let p = Arc::new(Published { group: Arc::clone(g), rels: rels.clone() });
            for &i in &g.docs {
                let _ = self.published[i].set(Arc::clone(&p));
            }
        }
        [groups, docs, runs.len(), left]
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
    ///
    /// A session's batch, which the next run renews from (decision 0616), is made in groups of at most
    /// `RERAIL_SESSION_GROUP` documents, in their order, whether a probe or a whole read asks for them: an edit then
    /// makes again only its changed documents' groups. They go on in the same block relations.
    ///
    /// The profile counts the documents made, by whether a probe (its documents) or a whole read (every document)
    /// asked for them, and the groups (`batch: documents made by a probe`, `… by a whole read`, `batch: groups made`):
    /// what settles a group's size, and whether a relation's whole reads make documents no probe names.
    pub(super) fn ensure(&self, want: &[usize]) {
        let size = session_group();
        let whole = want.len() >= self.docs.len();
        if self.keeps_strings && want.len() > size {
            let mut unmade: Vec<usize> = {
                let s = self.rows.state.lock();
                want.iter().copied().filter(|&i| s.of[i].is_none()).collect()
            };
            unmade.sort_unstable();
            unmade.dedup();
            if unmade.len() > size {
                for group in unmade.chunks(size) {
                    self.ensure_group(group, whole);
                }
            }
        }
        self.ensure_group(want, whole)
    }

    /// Every document's rows made, as one group where none is made yet (`ensure`): a chunk's batch, on the worker that
    /// projects it ahead of its evaluation (as `Batch::build` made them before decision 0494), so its evaluation reads
    /// them made and makes none inside its joins (30 s of PostHog's chunks' CPU, in groups a read asked for).
    pub fn make_every(&self) {
        let _made = profile::span(|| "a chunk's documents made ahead".to_owned());
        let every: Vec<usize> = (0..self.docs.len()).collect();
        self.ensure(&every);
    }

    /// Its documents made so far, for the run's corpus batch to share (decision 0683).
    pub fn projections(&self) -> Projections {
        Projections { slots: self.docs.made_slots(), projection: Arc::clone(&self.projection) }
    }

    /// `ensure`'s documents not made yet, as one group (`whole`: a whole read asked for them).
    fn ensure_group(&self, want: &[usize], whole: bool) {
        let me = std::thread::current().id();
        loop {
            // What to make, and its claim, under one lock: a document another thread claims between the two would be
            // made by both, and one group set aside.
            let (need, waits, claim) = {
                let mut s = self.rows.state.lock();
                let making = s.makers.contains(&me);
                let mut need: Vector<usize> = rerail_blocks::vector::Vector::new();
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
                    // Another thread's group: the pool's next job, else a wait for a group to end. Its time is the
                    // wait's span's, not the reading step's (t56 step 1); a job it runs charges its own scopes.
                    drop(s);
                    let _waited = profile::span(|| "batch: waited for another thread's make".to_owned());
                    if rayon::yield_now() != Some(rayon::Yield::Executed) {
                        let mut s = self.rows.state.lock();
                        if want.iter().any(|&i| s.of[i].is_none() && s.claimed[i].is_some_and(|t| t != me)) {
                            self.rows.made.wait_for(&mut s, std::time::Duration::from_millis(1));
                        }
                    }
                    continue;
                }
                if need.is_empty() {
                    debug_assert!(!waits);
                    return;
                }
                let docs: Vector<usize> = need.iter().copied().filter(|&i| s.claimed[i].is_none()).collect();
                for &i in &docs {
                    s.claimed[i] = Some(me);
                }
                s.makers.push(me);
                (need, waits, Claim { rows: &self.rows, me, docs })
            };
            let made = {
                // The make's own span: the reading step that asked for it is not charged it (t56 step 1).
                let _made = profile::span(|| "batch: rows made".to_owned());
                self.make_rows(&need, self.rows.first(), &[]).and_then(|mut m| {
                    let used: Option<Arc<[u32]>> = m.used.take().map(Arc::from);
                    self.group_rels(m, &need).map(|r| (r, used))
                })
            };
            let makes = self.rows.makes.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            crate::code_rule_dsl::prof::count(if whole { "batch: documents made by a whole read" } else { "batch: documents made by a probe" }, need.len());
            crate::code_rule_dsl::prof::count("batch: groups made", 1);
            if makes < 4 && std::env::var_os("RERAIL_WHOLE_READ_TRACE").is_some() {
                eprintln!("RERAIL_WHOLE_READ_TRACE group {makes} of {} documents: {}", need.len(), std::backtrace::Backtrace::force_capture());
            }
            let mut s = self.rows.state.lock();
            if need.iter().any(|&i| s.of[i].is_some()) {
                drop(s);
                drop(claim);
                continue;
            }
            let (rels, used) = match made {
                Ok(made) => made,
                Err(why) => {
                    // Its documents have no rows, and every source of the batch reports why.
                    self.docs.fail(why.to_string());
                    (self.rows.template.iter().map(|t| Relation::new(&Source::columns(t.as_ref()).iter().map(String::as_str).collect::<Vec<_>>())).collect(), Some(Arc::from(Vec::new())))
                }
            };
            let mut ranges = Vec::with_capacity(rels.len());
            for (at, r) in s.rels.iter_mut().zip(rels) {
                let start = Source::len(at.as_ref());
                *at = Arc::new(if start == 0 { r.growable() } else { at.grown(r) });
                ranges.push(start..Source::len(at.as_ref()));
            }
            if rerail_prof::heap::on() {
                let made = s.of.iter().filter(|g| g.is_some()).count() + need.len();
                for (i, at) in s.rels.iter().enumerate() {
                    note_source(self as *const Batch as usize, i, &self.rows.names[i], at, made, self.docs.len());
                }
            }
            let g = s.groups.len() as u32;
            for &i in &need {
                s.of[i] = Some(g);
            }
            profile::decision("a batch's documents made by group (decision 0494)", || format!("{} of {} documents", need.len(), self.docs.len()));
            let group = Arc::new(Group::new(need, ranges, used));
            s.groups.push(Arc::clone(&group));
            let p = Arc::new(Published { group, rels: s.rels.clone() });
            for &i in &p.group.docs {
                let _ = self.rows.published[i].set(Arc::clone(&p));
            }
            drop(s);
            drop(claim);
            if !waits {
                return;
            }
        }
    }

    /// A measure's sample of its documents (`SAMPLE_EVERY`, `SAMPLE_DOCS`): one document in 16 made, at least 32 (every
    /// one of a batch of fewer), and the groups they are in, with the documents those hold. `None` for a batch of no documents. Its first
    /// documents: the rest, made later as one group (`complete`), go on after them, so each relation's rows stay in
    /// the order of their ids, its key columns keys and its columns' ranges read in place. A sample spread over the
    /// batch put the rest's rows before some of the sample's, and every join read them by index (ab188: 2.6 s more of
    /// the joins' own CPU on the M3's 300 files).
    fn sample_groups(&self) -> Option<(Vec<Arc<Group>>, usize)> {
        let n = self.docs.len();
        if n == 0 {
            return None;
        }
        let k = n.min(SAMPLE_DOCS.max(n / SAMPLE_EVERY));
        let groups = self.groups_of((0..k).map(|i| self.docs.key(i)));
        let covered: usize = groups.iter().map(|g| g.docs.len()).sum();
        (covered > 0).then_some((groups, covered))
    }

    /// How many of its documents' rows are made.
    #[doc(hidden)]
    pub fn documents_with_rows(&self) -> usize {
        self.rows.state.lock().of.iter().filter(|g| g.is_some()).count()
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
    /// Every relation, every document's rows made: those unmade as one group. (Not as groups side by side: a maker
    /// running a pool job that reads the batch makes what that job reads, so many makers at once made documents twice
    /// and more, and one run grew past 20 GB. The group's own work runs on the pool instead: its matcher wave by parts
    /// of its tapes, `ast_wave::matched`.)
    pub(super) fn complete(&self) -> &[Arc<Relation>] {
        if let Some(c) = self.rows.complete.get() {
            return c;
        }
        self.ensure(&(0..self.docs.len()).collect::<Vec<_>>());
        profile::once(&self.rows.complete, "batch: every relation's rows", || {
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

    /// The keys of the documents whose paths' atoms are `values` (a read of `doc` by its path): its documents' paths
    /// interned once, none made. `None` where they could not be.
    pub(super) fn keys_of_paths(&self, values: &[i64]) -> Option<Vec<u32>> {
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

    /// The keys of the documents a node or span of which can have one of the texts whose atoms are `values` (a read of
    /// `text` by its string, decision 0656 step 4): each document's index asked (`DocIndex::may_have_text`), none
    /// made. A text-only document's source searched. `None` where a value names no text.
    fn keys_with_texts(&self, values: &[i64]) -> Option<Vec<u32>> {
        let texts: Vec<String> = values.iter().map(|&v| self.atoms.value(u32::try_from(v).ok()?).ok().flatten()).collect::<Option<_>>()?;
        Some(
            (0..self.docs.len())
                .filter(|&i| {
                    let facts = &self.docs.input(i).facts;
                    let source = facts.source.as_bytes();
                    if facts.tape.is_none() {
                        return texts.iter().any(|t| memchr::memmem::find(source, t.as_bytes()).is_some());
                    }
                    self.docs.read_facts(i, |f| {
                        let index = f.index();
                        texts.iter().any(|t| index.may_have_text(t.as_bytes(), source))
                    })
                })
                .map(|i| self.docs.key(i))
                .collect(),
        )
    }

    /// The keys of the documents whose `doc` rows' modules' atoms are `values` (a read of `doc` by its module): each
    /// file's module (`syntax::doc_module`, from its path and the corpus's name for it) interned once, none made. A
    /// tape value has no `doc` row. `None` where they could not be.
    pub(super) fn keys_of_modules(&self, values: &[i64]) -> Option<Vec<u32>> {
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

    /// The keys of the documents whose rows name document key `values` (`docof`'s `d`): the document of each key, and
    /// the guests made of a file's code, whose `d` is their file's (`Docs::file`, decision 0431 §4). None made.
    fn keys_of_documents(&self, values: &[i64]) -> Vec<u32> {
        let by = self.by_file_key.get_or_init(|| {
            let mut by: HashMap<u32, Vec<u32>> = HashMap::new();
            for i in 0..self.docs.len() {
                by.entry(self.docs.file(i)).or_default().push(self.docs.key(i));
            }
            by
        });
        values.iter().filter_map(|&v| u32::try_from(v).ok()).filter_map(|v| by.get(&v)).flatten().copied().collect()
    }

    /// `doc`'s statistics, of the documents' paths and modules, none made (`Source::statistics`, as
    /// `syntax::measured` gives them): a row a file (a tape value has none), its key, its path and its module. Its
    /// rows' first column is a document key, so every row is one group's. Measured as rows, the planner's measure of
    /// `doc` made every document's rows of every relation of its group (33.9 MB of node rows at the M3's 300-file
    /// peak, ab161). `None` with no file.
    pub(super) fn doc_statistics(&self) -> Option<Vec<(String, i64, i64, i64, i64)>> {
        self.keys_of_modules(&[])?;
        let by = self.by_module.get()?.as_ref()?;
        let rows: i64 = by.values().map(|k| k.len() as i64).sum();
        if rows == 0 {
            return None;
        }
        let squares = |counts: &mut dyn Iterator<Item = i64>| counts.fold((0i64, 0i64), |(k, s), n| (k + 1, s + n * n));
        let mut paths: Vec<&str> = (0..self.docs.len()).filter(|&i| !self.docs.guest(i).is_some_and(|g| matches!(g.made, super::Made::Tape { .. }))).map(|i| self.docs.path(i)).collect();
        paths.sort_unstable();
        let (path_ndv, path_sq) = squares(&mut paths.chunk_by(|a, b| a == b).map(|run| run.len() as i64));
        let (module_ndv, module_sq) = squares(&mut by.values().map(|k| k.len() as i64));
        Some(vec![("d".into(), rows, rows, rows, rows), ("path".into(), rows, path_ndv, path_sq, path_sq), ("module".into(), rows, module_ndv, module_sq, module_sq)])
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
                        // A file's tree (decision 0512) is its file's guest, as a program is its host's.
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

    /// The groups holding the documents `keys` name, made if they are not.
    fn groups_of(&self, keys: impl IntoIterator<Item = u32>) -> Vec<Arc<Group>> {
        let docs: BTreeSet<usize> = keys.into_iter().filter_map(|k| self.docs.position(k)).collect();
        let docs: Vector<usize> = docs.into_iter().collect();
        self.ensure(&docs);
        let s = self.rows.state.lock();
        let mut at: Vector<u32> = docs.iter().filter_map(|&i| s.of[i]).collect();
        at.sort_unstable();
        at.dedup();
        at.into_iter().map(|g| Arc::clone(&s.groups[g as usize])).collect()
    }

    /// The batch's vocabulary, every document made: its strings and their symbols, each string once.
    pub(super) fn vocabulary(&self) -> (Vec<String>, Vec<u32>) {
        if self.rows.complete.get().is_none() {
            profile::decision("a batch's every document made by a whole read of a relation (decision 0494)", || {
                let unmade = self.rows.state.lock().of.iter().filter(|g| g.is_none()).count();
                format!("the batch's vocabulary (a text test): {unmade} of {} documents unmade, read in {}", self.docs.len(), profile::here().unwrap_or_default())
            });
        }
        self.complete();
        // And the strings the batch's views hold, interned as they are read (`tape_rows::strings`): walked outside the
        // lock (a read may make a document, which takes it), then those the rows' strings lack.
        let mut added: HashSet<String> = HashSet::new();
        super::tape_rows::strings(self, &|rel| self.projection.wants(rel), &mut |s| {
            if !added.contains(s) {
                added.insert(s.to_owned());
            }
        });
        let shared = self.shared.lock();
        // Its strings are the held batches' too (decision 0616): those its live groups' rows used, where each group
        // kept them; else the shared vocabulary.
        let used: Option<Vec<u32>> = self.renewed.then(|| {
            let s = self.rows.state.lock();
            let mut used: Vec<u32> = Vec::new();
            for g in &s.groups {
                used.extend(g.strings.as_ref()?.iter().copied());
            }
            used.sort_unstable();
            used.dedup();
            Some(used)
        }).flatten();
        let mut atoms = used.unwrap_or_else(|| shared.vocabulary.clone());
        drop(shared);
        // And the views' strings, interned: those the rows' strings lack.
        if !added.is_empty() {
            let added: Vec<&str> = added.iter().map(String::as_str).collect();
            let had: HashSet<u32> = atoms.iter().copied().collect();
            atoms.extend(self.atoms.intern_all(&added).expect("the views' strings interned").iter().copied().filter(|a| !had.contains(a)));
        }
        texts_of(&*self.atoms, &atoms).expect("the atoms directory reads the batch's vocabulary")
    }
}

/// The documents a group of a session's batch holds at most (decision 0616): `RERAIL_SESSION_GROUP`, default 1 (a
/// group's other documents were made for no reader: the edits made 2.5 times the documents in groups of 4).
fn session_group() -> usize {
    std::env::var("RERAIL_SESSION_GROUP").ok().and_then(|v| v.parse::<usize>().ok()).filter(|&n| n > 0).unwrap_or(1)
}

/// A relation a batch makes (`text`, `op`, the matchers'), over its groups (decision 0494): every document made, the
/// one group's rows when one group holds them all (a batch read whole); before, what is known without making a
/// document, and a probe of the first column makes the documents it names.

pub(super) struct MadeRows {
    pub(super) batch: Arc<Batch>,
    pub(super) rel: usize,
    /// Its statistics before its rows are made, where no count gives them: measured on a sample of its documents
    /// (`sampled`), made once.
    pub(super) sampled: OnceLock<Option<Vec<(String, i64, i64, i64, i64)>>>,
}

/// A measure of a batch's relation that no count gives makes one document in `SAMPLE_EVERY` of the batch's, at least
/// `SAMPLE_DOCS` (`MadeRows::sampled`): the planner's measure made every document of the batch to measure one
/// relation (decision 0494's whole read), so all the batch's rows were made, and their heap staged, before any rule
/// ran (ab175: `pkg`, `col`, `op` and `tape_of` each made a batch whole that way, 33 MB of node rows staged at the M3's
/// 300-file peak). At least `SAMPLE_DOCS`: a relation a few documents hold (an AST matcher's) is noise in a sample of
/// two, and the planner led joins with it (ab189: `rr_ast_12_1` measured on 2 of 40 documents, 2 rows, estimated 40
/// where 455 were; the joins' own CPU 2.9 s more on the M3's 300 files). A batch of fewer is its own sample: measured
/// exactly, on its groups' rows, with no whole read.
const SAMPLE_EVERY: usize = 16;
const SAMPLE_DOCS: usize = 32;

impl MadeRows {
    fn template(&self) -> &dyn Source {
        &*self.batch.rows.template[self.rel]
    }
    /// What a read reads once every document is made: the relation, whole.
    fn target(&self) -> &dyn Source {
        if self.batch.rows.complete.get().is_none() {
            // With the span that read it whole (a measure's, none since its sample, `MadeRows::sampled`, or a run's) and how
            // many documents the read makes: a batch of an edit's changed files is made whole at no cost, one of the
            // corpus at the corpus's.
            if std::env::var_os("RERAIL_WHOLE_READ_TRACE").is_some() {
                eprintln!("RERAIL_WHOLE_READ_TRACE {}: {}", self.batch.rows.names[self.rel], std::backtrace::Backtrace::force_capture());
            }
            profile::decision("a batch's every document made by a whole read of a relation (decision 0494)", || {
                let unmade = self.batch.rows.state.lock().of.iter().filter(|g| g.is_none()).count();
                format!("{}: {unmade} of {} documents unmade, read in {}", self.batch.rows.names[self.rel], self.batch.docs.len(), profile::here().unwrap_or_default())
            });
        }
        &*self.batch.complete()[self.rel]
    }
    /// `target`, if every document is made.
    fn done(&self) -> Option<&dyn Source> {
        self.batch.rows.complete.get().map(|c| &*c[self.rel] as &dyn Source)
    }
    /// Whether every document's rows are made: a whole read reads them, and makes none.
    pub(super) fn is_done(&self) -> bool {
        self.done().is_some()
    }
    /// The rows a probe of `columns` matches, by its sampled statistics (`sampled`): its rows over the most distinct
    /// values of any of them.
    fn sampled_matches(&self, columns: &[usize]) -> Option<usize> {
        let s = self.sampled()?;
        let rows = s.first()?.1.max(0) as usize;
        let ndv = columns.iter().filter_map(|&c| s.get(c)).map(|c| c.2.max(1) as usize).max()?;
        Some(rows.div_ceil(ndv))
    }
    /// The rows a measure reads (`sampled`, `value_counts`): one document in `SAMPLE_EVERY` of the batch's, made, and
    /// every row of the groups they are in (a group made before may hold others: theirs are measured too), with the
    /// documents those groups hold. `None` for a batch of no documents.
    fn sample(&self) -> Option<(Arc<Relation>, Vec<usize>, usize)> {
        let (groups, covered) = self.batch.sample_groups()?;
        let r = self.batch.made_so_far(self.rel);
        let rows = groups.iter().flat_map(|g| g.ranges[self.rel].clone()).collect();
        Some((r, rows, covered))
    }
    /// Its statistics measured on a sample of its documents (`SAMPLE_EVERY`), those documents made and no other, and
    /// scaled to the batch's documents (`syntax::scaled_statistics`). Counted by value in an array in the run's
    /// blocks. Each row's document is its first column's high half, as `syntax::measured` takes it. `None` for a
    /// batch of no documents.
    fn sampled(&self) -> Option<Vec<(String, i64, i64, i64, i64)>> {
        self.sampled
            .get_or_init(|| {
                let (r, rows, covered) = self.sample()?;
                let n = self.batch.docs.len();
                profile::decision("a relation measured on a sample of its batch's documents (decision 0494)", || format!("{}: {covered} of {n} documents, {} rows", self.batch.rows.names[self.rel], rows.len()));
                let mut pairs: rerail_blocks::vector::Vector<[i64; 2]> = crate::code_rule_dsl::facts::rows::vector_in(None, rows.len());
                let measured: Vec<(String, i64, i64, i64, i64)> = self
                    .columns()
                    .into_iter()
                    .enumerate()
                    .map(|(c, name)| {
                        pairs.resize(0, [0, 0]);
                        for &row in &rows {
                            pairs.push([Source::value(r.as_ref(), row, c), Source::value(r.as_ref(), row, 0) >> 32]);
                        }
                        pairs.sort_unstable();
                        let (mut ndv, mut sq, mut sqd) = (0i64, 0i64, 0i64);
                        for value in pairs.chunk_by(|a, b| a[0] == b[0]) {
                            ndv += 1;
                            sq += (value.len() * value.len()) as i64;
                            for doc in value.chunk_by(|a, b| a[1] == b[1]) {
                                sqd += (doc.len() * doc.len()) as i64;
                            }
                        }
                        (name, rows.len() as i64, ndv, sq, sqd)
                    })
                    .collect();
                if rows.is_empty() {
                    return Some(measured);
                }
                let all = (rows.len() as u128 * n as u128 / covered as u128) as usize;
                Some(super::scaled_statistics(&measured, rows.len(), all))
            })
            .clone()
    }
    /// Whether `column` is core.rerail's `text(n, s)`'s string: a read by it is routed to the documents whose index
    /// says a node or span can have the text (`Batch::keys_with_texts`).
    fn is_text_string(&self, column: usize) -> bool {
        column == 1 && self.batch.rows.names[self.rel] == "text" && self.template().columns() == ["n", "s"]
    }
    /// How the relation's first column names its row's document.
    fn by(&self) -> ByDocument {
        self.batch.rows.by[self.rel]
    }
    /// The rows a read of `column` by `values` matches (each also passing `keep`), the documents they name made, in
    /// their groups' ranges: `None` for a column that names no document (by its key, or `doc`'s by its path).
    fn routed(&self, column: usize, values: &[i64], eqs: &[(usize, i64)]) -> Option<(Arc<Relation>, rerail_blocks::vector::Vector<usize>)> {
        let Some(keys) = self.route(column, values) else {
            profile::decision("a batch's every document made by a read of a relation no document routes (decision 0494)", || format!("{} column {column} (document column {:?})", self.batch.rows.names[self.rel], self.by().column()));

            return None;
        };
        self.rows_at(keys, column, values, |r, row| eqs.iter().all(|&(c, v)| Source::value(r, row, c) == v))
    }
    /// `lookup_read` of one value of the column that names a row's document (`by`), where it names one document: that
    /// document's group made if it is not, and its rows by the group's index of the column, read as found. No set,
    /// list or row buffer a probe: a probe of a made relation by its document's node is one group's index read. `false`
    /// where the read is not of that shape (the general route reads it).
    /// `hint`: the step's cursor's (`ProbeCursor::hint`): the last probe's document plus one and the row its search
    /// ended at, each checked before it is used.
    fn read_one(&self, column: usize, value: i64, columns: &[usize], hint: &mut [u32; 2], each: &mut dyn FnMut(usize, &[i64])) -> bool {
        let by = self.by();
        if by.column() != Some(column) || matches!(by, ByDocument::Host(_)) || columns.len() > 16 {
            return false;
        }
        let Some(key) = by.document(value) else { return false };
        let docs = &self.batch.docs;
        let last = hint[0].wrapping_sub(1) as usize;
        let doc = if last < docs.len() && docs.key(last) == key {
            last
        } else {
            let Some(doc) = docs.position(key) else { return false };
            *hint = [doc as u32 + 1, u32::MAX];
            doc
        };
        // The document's group as it went on: read with no lock, nothing counted.
        let p = match self.batch.rows.published[doc].get() {
            Some(p) => p,
            None => {
                let before = self.batch.rows.groups();
                self.batch.ensure(&[doc]);
                if self.batch.rows.groups() > before {
                    profile::decision("a batch's documents made by a probe (decision 0494)", || format!("{} column {column}", self.batch.rows.names[self.rel]));
                }
                let Some(p) = self.batch.rows.published[doc].get() else { return false };
                p
            }
        };
        let (g, r) = (&p.group, &p.rels[self.rel]);
        let mut v = [0i64; 16];
        g.each_match(self.rel, column, r, value, &mut hint[1], |row| {
            Source::row_at(r.as_ref(), row, columns, &mut v[..columns.len()]);
            each(row, &v[..columns.len()]);
        });
        true
    }
    /// Whether every read of `column` is routed (`route`): its document column (not a host's, whose guests may not
    /// be known), a node of the row's own document, or its document's key.
    fn routes(&self, column: usize) -> bool {
        let by = self.by();
        (by.column() == Some(column) && !matches!(by, ByDocument::Host(_)))
            || self.batch.rows.also[self.rel].contains(&column)
            || self.batch.rows.key_column[self.rel] == Some(column)
            || self.is_text_string(column)
    }
    /// The documents whose rows a read of `column` by `values` can find, none made: `None` for a column that names
    /// no document.
    fn route(&self, column: usize, values: &[i64]) -> Option<Vec<u32>> {
        let by = self.by();
        Some(if by.column() == Some(column) && matches!(by, ByDocument::Host(_)) {
            // A host's node: the guests of the documents named, which make the rows.
            let guests = self.batch.guests_of()?;
            values.iter().filter_map(|&v| by.document(v)).flat_map(|h| guests.get(&h).into_iter().flatten().copied()).collect()
        } else if by.column() == Some(column) {
            values.iter().filter_map(|&v| by.document(v)).collect()
        } else if self.batch.rows.also[self.rel].contains(&column) {
            // A node of the row's own document: every value must name one of the batch's, or the read is not routed
            // (a row of a document left unmade could hold it).
            values.iter().map(|&v| ByDocument::Id(column).document(v).filter(|&k| self.batch.docs.position(k).is_some())).collect::<Option<_>>()?
        } else if self.batch.rows.key_column[self.rel] == Some(column) {
            self.batch.keys_of_documents(values)
        } else if self.batch.rows.path_column[self.rel] == Some(column) {
            self.batch.keys_of_paths(values)?
        } else if self.batch.rows.module_column[self.rel] == Some(column) {
            self.batch.keys_of_modules(values)?
        } else if self.is_text_string(column) {
            self.batch.keys_with_texts(values)?
        } else {
            return None;
        })
    }
    /// The rows of documents `keys` whose `column` is one of `values` (each also passing `keep`), those documents
    /// made, through each group's index of the column.
    fn rows_at(&self, keys: Vec<u32>, column: usize, values: &[i64], keep: impl Fn(&Relation, usize) -> bool) -> Option<(Arc<Relation>, rerail_blocks::vector::Vector<usize>)> {
        let before = self.batch.rows.groups();
        let groups = self.batch.groups_of(keys);
        if self.batch.rows.groups() > before {
            profile::decision("a batch's documents made by a probe (decision 0494)", || format!("{} column {column}", self.batch.rows.names[self.rel]));
        }
        let r = self.batch.made_so_far(self.rel);
        let mut rows = rerail_blocks::vector::Vector::new();
        for g in groups {
            // A group's rows by the column's value, found by its index, not a walk of every row of it.
            let mut near = u32::MAX;
            for &v in values {
                g.each_match(self.rel, column, &r, v, &mut near, |row| {
                    if keep(&r, row) {
                        rows.push(row);
                    }
                });
            }
        }
        rows.sort_unstable();
        rows.dedup();
        Some((r, rows))
    }
}

impl Source for MadeRows {
    fn syntax_batch(&self) -> Option<Arc<dyn std::any::Any + Send + Sync>> {
        match self.done() {
            Some(t) => t.syntax_batch(),
            None => self.template().syntax_batch(),
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
    fn lookup_primary_batch(&self, _values:&[Vector<i64>])->Option<rerail_blocks::vector::Vector<usize>> {
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
        Some(self)
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
    /// By the document column, the step's cursor leads: the last probe's document and row (`read_one`).
    fn lookup_read_at(&self, column: usize, value: i64, columns: &[usize], cursor: &mut crate::code_rule_dsl::facts::vtab::ProbeCursor, each: &mut dyn FnMut(usize, &[i64])) -> bool {
        self.read_one(column, value, columns, &mut cursor.hint, each) || self.lookup_read(column, value, columns, each)
    }
    fn lookup_read(&self, _column: usize, _value: i64, _columns: &[usize], _each: &mut dyn FnMut(usize, &[i64])) -> bool {
        // By the document column, the document's group first, made or whole: its rows lie together in one part, a
        // search of them, not of the whole relation's index (a fence and a search of every document's rows).
        if self.read_one(_column, _value, _columns, &mut [0; 2], _each) {
            return true;
        }
        if let Some(t) = self.done() {
            return t.lookup_read(_column, _value, _columns, _each);
        }
        let Some((r, rows)) = self.routed(_column, std::slice::from_ref(&_value), &[]) else {
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
    /// The rows whose `column` is one of `keys`, of the documents the keys name (a semi-join's rows at the nodes the
    /// exports hold, decision 0458): those documents made, no other. Refused for a column that names no document.
    fn for_rows_keyed(&self, column: usize, keys: &rustc_hash::FxHashSet<i64>, columns: &[usize], each: &mut dyn FnMut(usize, &[i64]) -> bool) -> bool {
        let by = self.by();
        if by.column() != Some(column) || matches!(by, ByDocument::Host(_)) {
            return false;
        }
        // Its documents' groups, made if they are not: their ranges hold every row a key names.
        let docs: BTreeSet<u32> = keys.iter().filter_map(|&v| by.document(v)).collect();
        let groups = self.batch.groups_of(docs);
        let r = self.batch.made_so_far(self.rel);
        let (rel, mut row) = (r.as_ref(), vec![0i64; columns.len()]);
        for at in groups.iter().flat_map(|g| g.ranges[self.rel].clone()) {
            if !keys.contains(&Source::value(rel, at, column)) {
                continue;
            }
            Source::row_at(rel, at, columns, &mut row);
            if !each(at, &row) {
                return true;
            }
        }
        true
    }
    fn documents_of(&self, column: usize, each: &mut dyn FnMut(usize, i64)) {
        self.target().documents_of(column, each)
    }
    /// Without making a document: a relation every document made is read; one counted, by its count; else it may
    /// hold rows wherever the batch has a document.
    fn may_hold_rows(&self) -> bool {
        match (self.done(), self.batch.rows.counted(self.rel)) {
            (Some(t), _) => t.len() > 0,
            (None, Some(n)) => n > 0,
            (None, None) => self.batch.docs.len() > 0,
        }
    }
    /// Before its rows are made, its counts as given, else `doc`'s its documents' (a row a file): the target's estimate
    /// is its made rows', which makes every document (the planner's measure of `doc` did, ab162).
    fn estimated_len(&self) -> usize {
        match (self.done(), self.batch.rows.counted(self.rel)) {
            (None, Some(n)) => n,
            (None, None) => match self.sampled() {
                Some(s) => s.first().map_or(0, |c| c.1 as usize),
                None => self.target().estimated_len(),
            },
            _ => self.target().estimated_len(),
        }
    }
    fn rows_owned(&self, _g: usize, _owned: &dyn Fn(i64) -> bool) -> Option<rerail_blocks::vector::Vector<usize>> {
        self.done().and_then(|t| t.rows_owned(_g, _owned))
    }
    fn estimated_matches(&self, column: usize, batch: bool) -> usize {
        // Before its rows are made, by its kept statistics where they were given, as the made relation estimates
        // (`Relation::estimated_matches`, of its rows and the column's distinct values): no document made for it.
        // Without its distinct values (a semi-join's are of its held nodes alone, decision 0458), a value matches a
        // document's rows: an estimate never makes the batch.
        let given = || self.batch.rows.distinct(self.rel, column).unwrap_or(self.batch.docs.len());
        match (self.done(), self.batch.rows.counted(self.rel)) {
            (None, Some(n)) => {
                let one = n.div_ceil(given().max(1)).max(1);
                if batch { one * 8 } else { one }
            }
            // Else by its sample's (`sampled`): a planner's estimate of a probe made the whole batch.
            (None, None) if let Some(one) = self.sampled_matches(&[column]) => {
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
        // Before its rows are made: a sample's where its rows are not counted, else the most selective column's; no
        // document made for an estimate.
        match (self.done(), self.batch.rows.counted(self.rel)) {
            (None, None) if let Some(one) = self.sampled_matches(columns) => one,
            (None, Some(_)) => columns.iter().map(|&c| self.estimated_matches(c, false)).min().unwrap_or_else(|| self.estimated_len()),
            _ => self.target().estimated_matches_all(columns),
        }
    }
    fn composite(&self) -> bool {
        match self.done() {
            Some(t) => t.composite(),
            None => self.template().composite(),
        }
    }
    fn lookup_all(&self, _equal: &[(usize, i64)]) -> Option<rerail_blocks::vector::Vector<usize>> {
        self.done().and_then(|t| t.lookup_all(_equal))
    }
    /// The documents' rows by group (decision 0494): a group of theirs alone holds theirs and no other; in a group
    /// with others, theirs are those its routing column names (none for a relation not routed).
    fn rows_of_documents(&self, docs: &std::collections::BTreeSet<u32>) -> Option<rerail_blocks::vector::Vector<usize>> {
        let at: BTreeSet<usize> = docs.iter().filter_map(|&k| self.batch.docs.position(k)).collect();
        let by = self.by();
        let groups = self.batch.groups_of(docs.iter().copied());
        let r = self.batch.made_so_far(self.rel);
        let mut rows = rerail_blocks::vector::Vector::new();
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
    fn lookup_in(&self, _column: usize, _values: &[i64], _doc: i64) -> Option<rerail_blocks::vector::Vector<usize>> {
        if let Some(t) = self.done() {
            return t.lookup_in(_column, _values, _doc);
        }
        // The document's group: its rows whose `column` is one of `values` (a group may hold others too).
        let groups = self.batch.groups_of(u32::try_from(_doc).ok());
        let r = self.batch.made_so_far(self.rel);
        let by = self.by();
        let mut rows = rerail_blocks::vector::Vector::new();
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
    fn filter_eq(&self, eqs: &[(usize, i64)]) -> Vector<u32> {
        self.target().filter_eq(eqs)
    }
    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    fn device_view(&self) -> Option<DeviceView> {
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
    /// The made relation's, or before its rows are made, its sample's (`sampled`).
    fn statistics(&self) -> Option<Vec<(String, i64, i64, i64, i64)>> {
        match self.done() {
            Some(t) => t.statistics(),
            None => self.sampled(),
        }
    }
    /// The made relation's; before, counted on its sample of documents (`sample`) and scaled to the batch's: a count
    /// of a program's constants read rows across the whole relation, which made every document.
    fn value_counts(&self, _wanted: &[(usize, i64)]) -> Option<Vector<i64>> {
        if let Some(t) = self.done() {
            return t.value_counts(_wanted);
        }
        let (r, rows, covered) = self.sample()?;
        let scale = self.batch.docs.len() as f64 / covered as f64;
        let mut counts = rerail_blocks::vector![0i64; _wanted.len()];
        for &row in &rows {
            for (n, &(c, v)) in counts.iter_mut().zip(_wanted) {
                *n += i64::from(Source::value(r.as_ref(), row, c) == v);
            }
        }
        Some(counts.iter().map(|&n| (n as f64 * scale).round() as i64).collect())
    }
    fn failure(&self) -> Option<String> {
        self.batch.docs.failure().or_else(|| self.done().and_then(|t| t.failure()))
    }
    fn failed(&self) -> Option<String> {
        self.batch.docs.failure().or_else(|| self.done().and_then(|t| t.failed()))
    }
    /// Unmade, a column its documents route (`routes`) too: its lookup makes the documents a value names, each
    /// group's index of the column read. Without it, a probe of a place relation's spans (`places::Places`) by a node
    /// built an index of every row, which made every document.
    fn indexed(&self, column: usize) -> bool {
        match self.done() {
            Some(t) => t.indexed(column),
            None => self.routes(column) || self.template().indexed(column),
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
    fn range_lookup(&self, _column: usize, _lower: Option<(i64, bool)>, _upper: Option<(i64, bool)>) -> Option<rerail_blocks::vector::Vector<usize>> {
        if let Some(t) = self.done() {
            return t.range_lookup(_column, _lower, _upper);
        }
        // A range of the column that names a document, within known bounds: its documents' keys, each made.
        if let (Some((lo, _)), Some((hi, _))) = (_lower, _upper)
            && self.by().own_column() == Some(_column)
            && let (Some(first), Some(last)) = (self.by().document(lo), self.by().document(hi))
        {
            let keys = self.batch.docs.keys().filter(|k| (first..=last).contains(k));
            let groups = self.batch.groups_of(keys.collect::<Vec<_>>());
            let r = self.batch.made_so_far(self.rel);
            let within = |v: i64| _lower.is_none_or(|(lo, inc)| if inc { v >= lo } else { v > lo }) && _upper.is_none_or(|(hi, inc)| if inc { v <= hi } else { v < hi });
            let mut rows = rerail_blocks::vector::Vector::new();
            for g in groups {
                rows.extend(g.ranges[self.rel].clone().filter(|&row| within(Source::value(r.as_ref(), row, _column))));
            }
            rows.sort_unstable();
            return Some(rows);
        }
        self.target().range_lookup(_column, _lower, _upper)
    }
    fn lookup(&self, column: usize, values: &[i64]) -> Option<rerail_blocks::vector::Vector<usize>> {
        if let Some(t) = self.done() {
            return t.lookup(column, values);
        }
        self.routed(column, values, &[]).map(|(_, rows)| rows).or_else(|| self.target().lookup(column, values))
    }
    fn looks_up_columns(&self) -> bool {
        self.done().is_none()
    }
    fn lookup_columns(&self, columns: &[usize], values: &[i64]) -> Option<rerail_blocks::vector::Vector<usize>> {
        if self.done().is_some() || columns.len() != values.len() {
            return None;
        }
        // The documents every bound column that routes names: a row of the probe is of each's documents.
        let mut lead = None;
        let mut keys: Option<BTreeSet<u32>> = None;
        for (i, (&c, &v)) in columns.iter().zip(values).enumerate() {
            let Some(at) = self.route(c, std::slice::from_ref(&v)) else { continue };
            lead.get_or_insert(i);
            let at: BTreeSet<u32> = at.into_iter().collect();
            keys = Some(match keys {
                Some(k) => k.intersection(&at).copied().collect(),
                None => at,
            });
        }
        let (lead, keys) = (lead?, keys?);
        let (_, rows) = self.rows_at(keys.into_iter().collect(), columns[lead], &values[lead..=lead], |r, row| columns.iter().zip(values).all(|(&c, &v)| Source::value(r, row, c) == v))?;
        Some(rows)
    }
    fn keys_rows(&self, _column: usize) -> bool {
        match self.done() {
            Some(t) => t.keys_rows(_column),
            None => self.template().keys_rows(_column),
        }
    }
    /// A key column's row. `keys_rows` answers from the template before the rows are made, and a key's values are one
    /// row each in any order of the rows: but a measure's sample makes its documents' groups first
    /// (`Batch::sample_groups`), so the made relation's rows may not be in the order of its ids, and its column then
    /// not a key it searches (`Relation::key_row`). Then the row is its index's (`sorted_rows`), nothing allocated.
    fn key_row(&self, _column: usize, _value: i64) -> Option<usize> {
        let of = |t: &dyn Source| {
            if t.keys_rows(_column) {
                return t.key_row(_column, _value);
            }
            match t.sorted_rows(_column, _value) {
                Some(rows) => rows.first().map(|&r| r as usize),
                None => t.lookup(_column, &[_value]).and_then(|rows| rows.first().copied()),
            }
        };
        if let Some(t) = self.done() {
            return of(t);
        }
        match self.routed(_column, std::slice::from_ref(&_value), &[]) {
            Some((_, rows)) => rows.first().copied(),
            None => of(self.target()),
        }
    }
    fn lookup_eq(&self, _column: usize, _value: i64, _eqs: &[(usize, i64)]) -> Option<rerail_blocks::vector::Vector<usize>> {
        if let Some(t) = self.done() {
            return t.lookup_eq(_column, _value, _eqs);
        }
        self.routed(_column, std::slice::from_ref(&_value), _eqs)
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
    fn order(&self) -> Option<rerail_blocks::vector::Vector<usize>> {
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
    /// Its rows before the scan, and the strings a scan would test, measured on a sample of the batch's documents
    /// (`sampled`).
    sampled: OnceLock<Option<(usize, usize)>>,
}

/// The text tests' answers over a run's strings (decision 0037): shared by every test of a batch and of the batches
/// it grows into (decision 0431 §6). A string probed or scanned is tested against every test at once
/// (`Vocabulary::rows`), and each test's passing strings kept in its own rows.
pub(super) struct Tested {
    tests: Arc<crate::code_rule_dsl::logic::strings::Vocabulary>,
    atoms: Arc<dyn crate::content_store::AtomTable>,
    state: parking_lot::Mutex<TestedState>,
}

/// Every string tested, and each test's passing strings in row order with each one's row.
struct TestedState {
    tested: rerail_blocks::maps::BlockSet<i64>,
    passed: Vec<Passed>,
}

#[derive(Default)]
struct Passed {
    rows: rerail_blocks::vector::Vector<i64>,
    at: rerail_blocks::maps::BlockHash<i64, usize>,
}

impl Tested {
    pub(super) fn new(tests: &Arc<crate::code_rule_dsl::logic::strings::Vocabulary>, atoms: Arc<dyn crate::content_store::AtomTable>) -> Self {
        let passed = (0..tests.tests.len()).map(|_| Passed::default()).collect();
        Self { tests: Arc::clone(tests), atoms, state: parking_lot::Mutex::new(TestedState { tested: Default::default(), passed }) }
    }
    /// Whether these are the answers of `tests`.
    pub(super) fn of(&self, tests: &Arc<crate::code_rule_dsl::logic::strings::Vocabulary>) -> bool {
        Arc::ptr_eq(&self.tests, tests)
    }
    /// Test the strings not tested yet against every test, outside the lock, and keep each test's passing ones.
    /// `strings(i)`: the string of `symbols[i]`.
    fn test<'s>(&self, strings: impl Fn(usize) -> &'s str, symbols: &[u32]) {
        use rerail_blocks::vector::Vector;
        let fresh: Vector<u32> = {
            let s = self.state.lock();
            (0..symbols.len() as u32).filter(|&i| !s.tested.contains_key(&i64::from(symbols[i as usize]))).collect()
        };
        if fresh.is_empty() {
            return;
        }
        let syms: Vector<u32> = fresh.iter().map(|&i| symbols[i as usize]).collect();
        let passing = self.tests.rows(|j| strings(fresh[j] as usize), &syms);
        let mut s = self.state.lock();
        for &sym in syms.iter() {
            s.tested.put(i64::from(sym));
        }
        for (p, rows) in s.passed.iter_mut().zip(passing) {
            for v in rows {
                if !p.at.contains_key(&v) {
                    let n = p.rows.len();
                    p.at.insert(v, n);
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
        self.test(|i| strings[i].as_str(), &symbols);
        // `text`'s strings are the view's (decision 0656 step 4), read off the tape and interned as a read of them
        // interns them (`text_atoms`, which fills the document's term table its later reads take): none is among the
        // groups' strings. A document's at a time, its texts interned together, each tested where it lies, and each
        // symbol once a run (`Vocabulary::rows`' `seen`).
        if batch.shape == Shape::Core && batch.projection.wants("text") {
            for doc in 0..batch.docs.len() {
                let p = &batch.docs[doc];
                let rows: Vec<(u32, u32)> = (0..p.text_rows()).map(|i| p.text_local(i)).collect();
                let (mut texts, mut syms) = (Vec::new(), Vec::new());
                for (&(local, x), a) in rows.iter().zip(p.text_atoms(&*batch.atoms, &rows)) {
                    if let Ok(a) = u32::try_from(a) {
                        texts.push(p.text_str(local, x));
                        syms.push(a);
                    }
                }
                self.test(|i| texts[i], &syms);
            }
        }
        scanned.store(true, Ordering::Release);
    }
}

impl TextTest {
    /// Test `k` of `tested`'s tests over `batch`; `scanned`, the batch's tests' one flag.
    pub(super) fn new(batch: &Arc<Batch>, k: usize, tested: &Arc<Tested>, scanned: &Arc<std::sync::atomic::AtomicBool>) -> Self {
        Self { batch: Arc::clone(batch), k, tested: Arc::clone(tested), scanned: Arc::clone(scanned), whole: OnceLock::new(), sampled: OnceLock::new() }
    }
    /// Its rows estimated on a sample of the batch's documents (`Batch::sample_groups`): the strings made so far (the
    /// sample's among them) tested, as a probe tests them (each once, its verdict kept), and those passing scaled by
    /// the documents. The planner's measure of a text test scanned the batch's whole vocabulary, which made every
    /// document (ab183: `strtest_0`'s measure made six batches whole, 33 MB of node rows staged at the M3's peak).
    fn sampled(&self) -> Option<usize> {
        self.sample().map(|(rows, _)| rows)
    }
    /// The sample's rows and strings, both scaled to the batch's documents (`sampled`).
    fn sample(&self) -> Option<(usize, usize)> {
        *self.sampled.get_or_init(|| {
            let (_, covered) = self.batch.sample_groups()?;
            let symbols: Vec<i64> = self.batch.shared.lock().vocabulary.iter().map(|&a| i64::from(a)).collect();
            let passed = self.probe(&symbols).len();
            profile::decision("a text test measured on a sample of its batch's documents (decision 0494)", || format!("{}: {covered} of {} documents, {passed} of {} strings", self.tested.tests.tests[self.k].0, self.batch.docs.len(), symbols.len()));
            // At least one, as a lazy batch's estimate is: a sample that passes none says it passes few, not none.
            let scaled = |n: usize| ((n as u128 * self.batch.docs.len() as u128 / covered as u128) as usize).max(1);
            Some((scaled(passed), scaled(symbols.len())))
        })
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
        // The run's chunks' passing strings, given (`Batch::give_passed`): no document made for them.
        if let Some(passed) = self.batch.passed_given(self.k) {
            profile::decision("a text test read whole (decision 0494)", || format!("{}: the chunks' passing strings, no document made", self.tested.tests.tests[self.k].0));
            let r = Relation::from_rows(&["s"], &mut passed.iter().map(|&v| std::iter::once(v).collect()));
            return self.whole.get_or_init(|| r);
        }
        self.tested.scan(&self.batch, &self.scanned);
        let s = self.tested.state.lock();
        let r = Relation::from_rows(&["s"], &mut s.passed[self.k].rows.iter().map(|&v| rerail_facts_base::row![v]));
        drop(s);
        self.whole.get_or_init(|| r)
    }
    /// The rows of the probed strings that pass, each tested once.
    fn probe(&self, values: &[i64]) -> rerail_blocks::vector::Vector<usize> {
        use rerail_blocks::vector::Vector;
        let fresh: Vector<i64> = {
            let s = self.tested.state.lock();
            let mut fresh: Vector<i64> = values.iter().copied().filter(|v| !s.tested.contains_key(v)).collect();
            fresh.sort_unstable();
            fresh.dedup();
            fresh
        };
        let (mut strings, mut symbols) = (rerail_blocks::maps::BlockStrings::default(), Vector::<u32>::new());
        for &v in fresh.iter() {
            if let Ok(sym) = u32::try_from(v)
                && let Ok(Some(text)) = self.tested.atoms.value(sym)
            {
                strings.push(&text);
                symbols.push(sym);
            }
        }
        self.tested.test(|i| strings.get(i), &symbols);
        let mut s = self.tested.state.lock();
        // A value no atom names is tested too: it passes nothing.
        for &v in fresh.iter() {
            s.tested.put(v);
        }
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
    /// Without testing a string: scanned, by its rows; given its rows' count, by it; else it may hold rows (its scan
    /// is the batch's every string, made and tested).
    fn may_hold_rows(&self) -> bool {
        match (self.whole.get(), self.batch.given_rows(&self.tested.tests.tests[self.k].0)) {
            (Some(w), _) => w.len() > 0,
            (None, Some(n)) => n > 0,
            (None, None) => true,
        }
    }
    fn estimated_len(&self) -> usize {
        match self.whole.get() {
            Some(w) => w.len(),
            None if self.batch.lazy() => self.tested.state.lock().passed[self.k].rows.len().max(1),
            None => self.sampled().unwrap_or_else(|| self.whole().len()),
        }
    }
    /// A key of strings: each passing string one row. Before the scan, its rows as sampled (`sampled`).
    fn statistics(&self) -> Option<Vec<(String, i64, i64, i64, i64)>> {
        let n = match self.whole.get() {
            Some(w) => w.len(),
            None if self.batch.lazy() => return None,
            None => self.sampled()?,
        } as i64;
        Some(vec![("s".to_owned(), n, n, n, n)])
    }
    /// A probed string's row as probed (the whole keeps them first, in their order); a row past them, the whole's.
    fn value(&self, row: usize, column: usize) -> i64 {
        if let Some(w) = self.whole.get() {
            return Source::value(w, row, column);
        }
        if let Some(&s) = self.tested.state.lock().passed[self.k].rows.get(row) {
            return s;
        }
        Source::value(self.whole(), row, column)
    }
    fn indexed(&self, column: usize) -> bool {
        column == 0
    }
    fn estimated_matches(&self, _column: usize, batch: bool) -> usize {
        if batch { 8 } else { 1 }
    }
    /// Before its scan, in a batch not yet made whole, a scan tests every string of the batch, every document made
    /// for them (ab202: `exists_25`'s `strtest_29($p), doc($d, $p, _)` led with the test, 81 rows priced, which made
    /// the chunk's 260 documents' every relation, 61 MB of the block of sources at the M3's peak, for 71 paths):
    /// the strings a row it gives, as sampled. A probe tests only what it is given.
    /// A batch made by its reads (`Batch::lazy`) is not sampled for it, as its statistics are not.
    fn scan_cost(&self) -> f64 {
        if self.whole.get().is_some() || self.batch.rows.complete.get().is_some() || self.batch.lazy() {
            return 1.0;
        }
        self.sample().map_or(1.0, |(rows, strings)| (strings as f64 / rows as f64).max(1.0))
    }
    fn lookup(&self, column: usize, values: &[i64]) -> Option<rerail_blocks::vector::Vector<usize>> {
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

/// Under a heap profile, each batch's relations as they grow (`note_source`): what a batch's block of sources holds at
/// the footprint's peak, by relation, with the spans its first and latest groups were made in and how many of its
/// documents were. Each entry is the relation's latest (a group grows it into a new one), kept while anything holds it.
#[derive(Default)]
struct SourcesHeld(parking_lot::Mutex<HashMap<(usize, usize), SourceNote>>);

struct SourceNote {
    name: String,
    rel: std::sync::Weak<Relation>,
    first: u32,
    last: u32,
    made: usize,
    docs: usize,
}

fn note_source(batch: usize, i: usize, name: &str, rel: &Arc<Relation>, made: usize, docs: usize) {
    let held = profile::machine::<SourcesHeld>();
    let mut m = held.0.lock();
    m.retain(|_, n| n.rel.strong_count() > 0);
    let span = rerail_prof::heap::span();
    let e = m.entry((batch, i)).or_insert_with(|| SourceNote { name: name.to_owned(), rel: Arc::downgrade(rel), first: span, last: span, made, docs });
    e.rel = Arc::downgrade(rel);
    e.last = span;
    e.made = made;
}

/// The batches' relations held now, the most bytes first: (bytes in their blocks, name, documents made, the batch's
/// documents, the span of its first group, of its latest).
pub fn sources_held() -> Vec<(usize, String, usize, usize, u32, u32)> {
    let held = profile::machine::<SourcesHeld>();
    let m = held.0.lock();
    let mut out: Vec<_> = m.values().filter_map(|n| n.rel.upgrade().map(|r| (r.block_bytes(), n.name.clone(), n.made, n.docs, n.first, n.last))).filter(|e| e.0 > 0).collect();
    out.sort_by_key(|e| std::cmp::Reverse(e.0));
    out
}
