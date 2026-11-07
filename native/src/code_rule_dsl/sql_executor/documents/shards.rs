//! Per-family document indexes with cached subtree estimates and dense native row ids.
use super::{
    tree::{Link, Measure, Node},
    *,
};
use std::sync::OnceLock;

#[derive(Clone, Default)]
struct Column {
    single: usize,
    batch: usize,
    max_single: usize,
    max_batch: usize,
    no_index: usize,
    no_range: usize,
    no_doc: usize,
}
#[derive(Clone, Default)]
pub(super) struct Stats {
    rows: usize,
    estimate: usize,
    lazy: usize,
    columns: Vec<Column>,
}
impl Stats {
    fn of(s: &dyn Source) -> Self {
        let lazy = usize::from(s.made_on_lookup());
        Self {
            rows: if lazy == 0 { s.len() } else { 0 },
            estimate: s.estimated_len(),
            lazy,
            columns: (0..s.columns().len())
                .map(|c| {
                    let (single, batch) =
                        (s.estimated_matches(c, false), s.estimated_matches(c, true));
                    Column {
                        single,
                        batch,
                        max_single: single,
                        max_batch: batch,
                        no_index: usize::from(!s.indexed(c)),
                        no_range: usize::from(!s.range_indexed(c)),
                        no_doc: usize::from(!s.kept_to_doc(c)),
                    }
                })
                .collect(),
        }
    }
    fn add(&mut self, other: &Self) {
        self.rows += other.rows;
        self.estimate += other.estimate;
        self.lazy += other.lazy;
        for (a, b) in self.columns.iter_mut().zip(&other.columns) {
            a.single += b.single;
            a.batch += b.batch;
            a.max_single = a.max_single.max(b.max_single);
            a.max_batch = a.max_batch.max(b.max_batch);
            a.no_index += b.no_index;
            a.no_range += b.no_range;
            a.no_doc += b.no_doc;
        }
    }
}
#[derive(Clone)]
pub(super) struct Part {
    source: Arc<dyn Source>,
    stats: Stats,
}
impl Measure<u32> for Part {
    type Summary = Stats;
    fn measure(&self, _: &u32, left: Option<&Stats>, right: Option<&Stats>) -> Stats {
        let mut s = self.stats.clone();
        if let Some(l) = left {
            s.add(l)
        }
        if let Some(r) = right {
            s.add(r)
        }
        s
    }
}
type Tree = Node<u32, Arc<Part>>;
impl Measure<u32> for Arc<Part> {
    type Summary = Stats;
    fn measure(&self, key: &u32, left: Option<&Stats>, right: Option<&Stats>) -> Stats {
        self.as_ref().measure(key, left, right)
    }
}
struct Scan {
    start: usize,
    end: usize,
    part: Arc<Part>,
}
struct Probes {
    ids: HashMap<(u32, usize), usize>,
    rows: Vec<(Arc<Part>, usize)>,
}
const PROBE: usize = 1usize << 31;
// Native producer schemas identify these columns as locations or document ids. A numeric
// range index alone does not identify a node column (literal values and ordinals have one too).
fn domains(name: &str) -> (&'static [usize], &'static [usize]) {
    match name {
        "node" => (&[1], &[0]),
        "core_node" => (&[2], &[0]),
        "child" | "within" | "span_within" | "next_sibling" => (&[0, 1], &[]),
        "field" => (&[0, 2], &[]),
        "tape" => (&[0, 1], &[]),
        "docof" => (&[0], &[1]),
        "doc" | "pkg" | "path_part" | "file_name" | "source" | "src_line" | "split_line"
        | "lf_line" | "line_count" | "syntax_error" | "doc_role" | "doc_root" | "tape_of" => {
            (&[], &[0])
        }
        "name" | "text" | "str" | "strprefix" | "comment" | "value" | "static_value" | "intval"
        | "op" | "op_t" | "line" | "end_line" | "col" | "visit_index" | "ast_depth"
        | "tree_digest" | "spread" | "str_part" | "str_part_word" | "str_part_start" => (&[0], &[]),
        "first_token" | "last_token" | "token_before" | "token_after" => (&[0, 1], &[]),
        _ => (&[], &[]),
    }
}
pub(super) struct Family {
    root: Link<u32, Arc<Part>>,
    empty: Arc<dyn Source>,
    columns: Vec<String>,
    doc: Option<usize>,
    locations: &'static [usize],
    documents: &'static [usize],
    scan: OnceLock<Vec<Scan>>,
    probes: parking_lot::Mutex<Probes>,
}
impl Family {
    pub fn empty(name: &str, source: Arc<dyn Source>) -> Self {
        let (locations, documents) = domains(name);
        Self {
            columns: source.columns(),
            doc: source.doc_column(),
            locations,
            documents,
            empty: source,
            root: None,
            scan: OnceLock::new(),
            probes: parking_lot::Mutex::new(Probes {
                ids: HashMap::new(),
                rows: vec![],
            }),
        }
    }
    pub fn changed(
        &self,
        changes: &[(u32, Option<Arc<dyn Source>>)],
        work: &mut usize,
    ) -> Option<Self> {
        let mut root = self.root.clone();
        for (key, source) in changes {
            let part = source
                .as_ref()
                .map(|source| {
                    Arc::new(Part {
                        stats: Stats::of(source.as_ref()),
                        source: source.clone(),
                    })
                })
                .filter(|p| p.stats.lazy != 0 || p.stats.rows != 0);
            if part.is_some() || Tree::get(&root, key).is_some() {
                root = Tree::set(&root, *key, part, work)
            }
        }
        let same = match (&self.root, &root) {
            (None, None) => true,
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            _ => false,
        };
        if same {
            return None;
        }
        Some(Self {
            root,
            empty: self.empty.clone(),
            columns: self.columns.clone(),
            doc: self.doc,
            locations: self.locations,
            documents: self.documents,
            scan: OnceLock::new(),
            probes: parking_lot::Mutex::new(Probes {
                ids: HashMap::new(),
                rows: vec![],
            }),
        })
    }
    fn stats(&self) -> Option<&Stats> {
        self.root.as_ref().map(|n| &n.summary)
    }
    fn lazy(&self) -> bool {
        self.stats().is_some_and(|s| s.lazy > 0)
    }
    fn scan(&self) -> &[Scan] {
        self.scan.get_or_init(|| {
            let mut rows = vec![];
            let mut start = 0;
            Tree::visit(&self.root, &mut |n| {
                let end = start + n.value.source.len();
                rows.push(Scan {
                    start,
                    end,
                    part: n.value.clone(),
                });
                start = end;
            });
            rows
        })
    }
    fn probe(&self, key: u32, part: &Arc<Part>, base: usize, row: usize) -> usize {
        if !self.lazy() {
            return base + row;
        }
        let mut p = self.probes.lock();
        let id = if let Some(&id) = p.ids.get(&(key, row)) {
            id
        } else {
            let id = p.rows.len();
            assert!(id < PROBE);
            p.rows.push((part.clone(), row));
            p.ids.insert((key, row), id);
            id
        };
        PROBE | id
    }
    fn visit(root: &Link<u32, Arc<Part>>, base: usize, f: &mut impl FnMut(u32, &Arc<Part>, usize)) {
        if let Some(n) = root {
            Self::visit(&n.left, base, f);
            let at = base + n.left.as_ref().map_or(0, |n| n.summary.rows);
            f(n.key, &n.value, at);
            Self::visit(&n.right, at + n.value.stats.rows, f)
        }
    }
    fn get(&self, key: u32) -> Option<(&Arc<Part>, usize)> {
        let mut node = self.root.as_deref();
        let mut base = 0;
        while let Some(n) = node {
            let left = n.left.as_ref().map_or(0, |n| n.summary.rows);
            match key.cmp(&n.key) {
                std::cmp::Ordering::Equal => return Some((&n.value, base + left)),
                std::cmp::Ordering::Less => node = n.left.as_deref(),
                std::cmp::Ordering::Greater => {
                    base += left + n.value.stats.rows;
                    node = n.right.as_deref()
                }
            }
        }
        None
    }
    fn collect(&self, probe: impl Fn(&dyn Source) -> Option<Vec<usize>>) -> Option<Vec<usize>> {
        let mut rows = vec![];
        let mut indexed = true;
        Self::visit(
            &self.root,
            0,
            &mut |key, p, base| match probe(p.source.as_ref()) {
                Some(found) => rows.extend(found.into_iter().map(|r| self.probe(key, p, base, r))),
                None => indexed = false,
            },
        );
        indexed.then_some(rows)
    }
}
impl Source for Family {
    fn columns(&self) -> Vec<String> {
        self.columns.clone()
    }
    fn len(&self) -> usize {
        if self.lazy() {
            self.scan().last().map_or(0, |s| s.end)
        } else {
            self.stats().map_or(0, |s| s.rows)
        }
    }
    fn estimated_len(&self) -> usize {
        self.stats().map_or(0, |s| s.estimate)
    }
    fn estimated_matches(&self, c: usize, b: bool) -> usize {
        self.stats().map_or(0, |s| {
            let x = &s.columns[c];
            match (self.doc == Some(c), b) {
                (true, true) => x.max_batch,
                (true, false) => x.max_single,
                (false, true) => x.batch,
                (false, false) => x.single,
            }
        })
    }
    fn indexed(&self, c: usize) -> bool {
        self.stats()
            .map_or_else(|| self.empty.indexed(c), |s| s.columns[c].no_index == 0)
    }
    fn range_indexed(&self, c: usize) -> bool {
        self.stats().map_or_else(
            || self.empty.range_indexed(c),
            |s| s.columns[c].no_range == 0,
        )
    }
    fn made_on_lookup(&self) -> bool {
        self.lazy()
    }
    fn doc_column(&self) -> Option<usize> {
        self.doc
    }
    fn kept_to_doc(&self, c: usize) -> bool {
        self.stats().is_some_and(|s| s.columns[c].no_doc == 0)
    }
    fn lookup(&self, c: usize, values: &[i64]) -> Option<Vec<usize>> {
        let location = self.locations.contains(&c);
        let document = self.documents.contains(&c) || self.doc == Some(c);
        if (location || document) && values.iter().all(|&v| v >= 0) {
            let mut rows = vec![];
            for &v in values {
                let encoded = if location { v >> 32 } else { v };
                if let Ok(key) = u32::try_from(encoded) {
                    if let Some((p, base)) = self.get(key) {
                        rows.extend(
                            p.source
                                .lookup(c, &[v])?
                                .into_iter()
                                .map(|r| self.probe(key, p, base, r)),
                        )
                    }
                }
            }
            Some(rows)
        } else {
            self.collect(|s| s.lookup(c, values))
        }
    }
    fn lookup_in(&self, c: usize, values: &[i64], doc: i64) -> Option<Vec<usize>> {
        let Ok(key) = u32::try_from(doc) else {
            return Some(vec![]);
        };
        let Some((p, base)) = self.get(key) else {
            return Some(vec![]);
        };
        Some(
            p.source
                .lookup_in(c, values, doc)?
                .into_iter()
                .map(|r| self.probe(key, p, base, r))
                .collect(),
        )
    }
    fn range_lookup(
        &self,
        c: usize,
        lo: Option<(i64, bool)>,
        hi: Option<(i64, bool)>,
    ) -> Option<Vec<usize>> {
        self.collect(|s| s.range_lookup(c, lo, hi))
    }
    fn value(&self, r: usize, c: usize) -> i64 {
        if self.lazy() {
            if r & PROBE != 0 {
                let p = self.probes.lock();
                let (part, row) = &p.rows[r & !PROBE];
                return part.source.value(*row, c);
            }
            let scan = self.scan();
            let i = scan.partition_point(|s| s.end <= r);
            return scan[i].part.source.value(r - scan[i].start, c);
        }
        let mut node = self.root.as_deref().expect("row in nonempty family");
        let mut r = r;
        loop {
            let left = node.left.as_ref().map_or(0, |n| n.summary.rows);
            if r < left {
                node = node.left.as_deref().unwrap()
            } else if r < left + node.value.stats.rows {
                return node.value.source.value(r - left, c);
            } else {
                r -= left + node.value.stats.rows;
                node = node.right.as_deref().unwrap()
            }
        }
    }
}
