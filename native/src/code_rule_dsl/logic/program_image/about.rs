//! A version's other fields (its `.about` rows, `program_facts::write_about`), read by type: the answers, the
//! natives, the lattices and the gates, each relation by its id. An analysis reads these, never the rows.
//!
//! Every relation an about row names has an id (`RELATION_COLUMNS`, made so by the builder), as every relation a
//! rule, a declaration, a fact or the inputs and outputs name has: so an analysis keys all of them by `RelId`.
use super::build::Builder;
use super::version::Version;
use super::{AtomId, Cell, RelId};

/// The columns of each about relation that name a relation (column 0 is the program's name). The builder gives each
/// such cell's atom a relation id.
pub(crate) const RELATION_COLUMNS: &[(&str, &[usize])] = &[
    ("answer", &[1, 3]),
    ("asked", &[1, 3]),
    ("native_dfs", &[1, 2, 3]),
    ("native_roots", &[1, 2]),
    ("native_match", &[1, 2]),
    ("native_group", &[1]),
    ("native_text_test", &[1, 2]),
    ("native_text_pattern", &[1]),
    ("native_python_candidates", &[1, 2, 3]),
    ("native_python", &[1, 3, 4]),
    ("native_components", &[1, 2, 3]),
    ("native_reach", &[1, 2, 3]),
    ("native_reach_through", &[1, 2, 3]),
    ("lattice", &[1]),
    ("lattice_merge", &[1]),
    ("lattice_widen", &[1]),
    ("gate", &[3]),
    ("gate_quiet", &[3]),
    ("gate_dep", &[3]),
    ("nullary", &[1]),
    ("qualifier", &[1]),
    ("tape_column", &[1]),
    ("demand", &[2]),
];

/// The relation columns of about relation `rel`.
pub(crate) fn relation_columns(rel: &str) -> &'static [usize] {
    RELATION_COLUMNS.iter().find(|(r, _)| *r == rel).map_or(&[], |(_, c)| c)
}

/// A native relation's kind (`front::Native`'s variants).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeKind {
    DfsFirst,
    MatchAll,
    TextTest,
    PythonCandidates,
    Python,
    Components,
    Reach,
    ReachThrough,
}

/// A native relation: its kind, the relations it reads (`front::Native::reads`, in its order) and its every field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeRef {
    pub rel: RelId,
    pub kind: NativeKind,
    pub reads: Vec<RelId>,
    pub spec: NativeSpec,
}

/// A native relation's fields (`front::Native`'s, variant by variant): each relation by its id, each other name by
/// its atom.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NativeSpec {
    DfsFirst { edge: RelId, target: RelId, roots: Option<RelId> },
    /// Its groups in order.
    MatchAll { input: RelId, pattern: AtomId, groups: Vec<AtomId>, positions: bool },
    /// Its patterns in order.
    TextTest { input: RelId, column: usize, op: AtomId, patterns: Vec<AtomId> },
    PythonCandidates { input: RelId, targets: RelId, context: AtomId, member: AtomId },
    Python { kind: AtomId, input: RelId, targets: Option<RelId> },
    Components { edge: RelId, own: RelId },
    Reach { seed: RelId, edge: RelId },
    ReachThrough { reach: RelId, map: RelId },
}

/// A lattice relation's declaration, as the analyses read it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LatticeRef {
    pub rel: RelId,
    /// The key columns before the value column.
    pub keys: usize,
    /// `LatticeDecl::lattice`: the merge's name, else the value column's type.
    pub lattice: AtomId,
    /// `LatticeDecl::ty`, `merge` and `widen`.
    pub ty: AtomId,
    pub merge: Option<AtomId>,
    pub widen: Option<u32>,
}

/// A version's other fields, read once from its about rows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct About {
    /// Each request and a relation it produces, by request then in order (`Program::answers`).
    pub answers: Vec<(RelId, RelId)>,
    /// Each native relation, by relation id.
    pub natives: Vec<NativeRef>,
    /// Each lattice relation, by relation id.
    pub lattice: Vec<LatticeRef>,
    /// How many gates the program has (`Program::gates`).
    pub gates: usize,
    /// The relations of no columns (`Program::nullary`), by relation id.
    pub nullary: Vec<RelId>,
    /// Each relation's qualifiers (`Program::qualifiers`): (the relation's name, the word), sorted.
    pub qualifiers: Vec<(AtomId, AtomId)>,
    /// The demanded relations (`Program::demand`), by name, sorted.
    pub demand: Vec<AtomId>,
    /// The demanded relations in the program's order (`Program::demand`, `*` among them).
    pub demand_list: Vec<AtomId>,
    /// The gates' relations (`Program::gates`' keys), by name, sorted.
    pub gate_keys: Vec<AtomId>,
    /// Each gate in the program's order (`Program::gates`).
    pub gate_list: Vec<GateRef>,
    /// Each kind set and its kinds, in order (`Program::kindsets`), by name.
    pub kindsets: Vec<(AtomId, Vec<AtomId>)>,
    /// Each relation's columns declared `tape` (`Program::tapes`), sorted.
    pub tapes: Vec<(RelId, usize)>,
    /// The join shape directives, in the program's order (`Program::shapes`).
    pub shapes: Vec<ShapeRef>,
}

/// A gate (`front::Gate`): its key, its relation, its level, its source, and the relations it quiets and depends on,
/// in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GateRef {
    pub key: AtomId,
    pub rel: Option<RelId>,
    pub level: usize,
    pub src: Option<AtomId>,
    pub quiet: Vec<RelId>,
    pub deps: Vec<RelId>,
}

/// A join shape directive (`front::Shape`), each part by its atom.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShapeRef {
    pub head: AtomId,
    pub lit: AtomId,
    pub wait: ShapeWaitRef,
    pub text: AtomId,
}

/// When a shape's literal may join (`front::ShapeWait`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShapeWaitRef {
    After(AtomId),
    Bound(Vec<AtomId>),
}

impl About {
    /// The about rows of `v`, read.
    pub fn of(v: &Version) -> About {
        let mut about = About::default();
        let rel = |c: Cell| match c {
            Cell::Str(a) => v.rel_of(a),
            Cell::Int(_) | Cell::Branded(_) => None,
        };
        let atom = |c: Cell| match c {
            Cell::Str(a) => Some(a),
            Cell::Int(_) | Cell::Branded(_) => None,
        };
        let mut merges: Vec<(RelId, AtomId)> = Vec::new();
        let mut widens: Vec<(RelId, u32)> = Vec::new();
        let mut roots: Vec<(RelId, RelId)> = Vec::new();
        let mut groups: Vec<(RelId, i64, AtomId)> = Vec::new();
        let mut patterns: Vec<(RelId, i64, AtomId)> = Vec::new();
        let mut demand: Vec<(i64, AtomId)> = Vec::new();
        let mut answers: Vec<(RelId, i64, RelId)> = Vec::new();
        let int = |c: Option<Cell>| match c {
            Some(Cell::Int(i)) => Some(i),
            _ => None,
        };
        let mut gates: Vec<(i64, GateRef)> = Vec::new();
        let mut gate_parts: Vec<(i64, bool, i64, RelId)> = Vec::new();
        let mut gate_srcs: Vec<(i64, AtomId)> = Vec::new();
        let mut kinds: Vec<(AtomId, i64, AtomId)> = Vec::new();
        let mut sets: Vec<AtomId> = Vec::new();
        let mut shapes: Vec<(i64, ShapeRef)> = Vec::new();
        let mut shape_after: Vec<(i64, AtomId)> = Vec::new();
        let mut shape_bound: Vec<(i64, i64, AtomId)> = Vec::new();
        for f in v.about_facts() {
            let name = v.text(f.name());
            for row in f.rows() {
                let row: Vec<Cell> = row.collect();
                let r = |k: usize| row.get(k).copied().and_then(rel);
                let a = |k: usize| row.get(k).copied().and_then(atom);
                // A relation cell that may be empty (`Python`'s `targets`): an empty one names no relation.
                let opt = |k: usize| r(k).filter(|&x| !v.rel_name(x).is_empty());
                let native = |kind, cols: &[usize], spec: Option<NativeSpec>| {
                    let reads = cols.iter().filter_map(|&k| r(k)).collect();
                    r(1).zip(spec).map(|(rel, spec)| NativeRef { rel, kind, reads, spec })
                };
                match name {
                    "answer" => {
                        if let (Some(req), Some(Cell::Int(i)), Some(p)) = (r(1), row.get(2).copied(), r(3)) {
                            answers.push((req, i, p));
                        }
                    }
                    "native_dfs" => about.natives.extend(native(NativeKind::DfsFirst, &[2, 3], r(2).zip(r(3)).map(|(edge, target)| NativeSpec::DfsFirst { edge, target, roots: None }))),
                    "native_roots" => roots.extend(r(1).zip(r(2))),
                    "native_match" => about.natives.extend(native(
                        NativeKind::MatchAll,
                        &[2],
                        r(2).zip(a(3)).map(|(input, pattern)| NativeSpec::MatchAll { input, pattern, groups: Vec::new(), positions: int(row.get(4).copied()).is_some_and(|p| p != 0) }),
                    )),
                    "native_text_test" => about.natives.extend(native(
                        NativeKind::TextTest,
                        &[2],
                        r(2).zip(int(row.get(3).copied())).zip(a(4)).map(|((input, column), op)| NativeSpec::TextTest { input, column: column as usize, op, patterns: Vec::new() }),
                    )),
                    "native_text_pattern" => {
                        if let (Some(rel), Some(i), Some(p)) = (r(1), int(row.get(2).copied()), a(3)) {
                            patterns.push((rel, i, p));
                        }
                    }
                    "native_group" => {
                        if let (Some(rel), Some(i), Some(g)) = (r(1), int(row.get(2).copied()), a(3)) {
                            groups.push((rel, i, g));
                        }
                    }
                    "native_python_candidates" => about.natives.extend(native(
                        NativeKind::PythonCandidates,
                        &[2, 3],
                        match (r(2), r(3), a(4), a(5)) {
                            (Some(input), Some(targets), Some(context), Some(member)) => Some(NativeSpec::PythonCandidates { input, targets, context, member }),
                            _ => None,
                        },
                    )),
                    // An empty `targets` is none: it names no relation.
                    "native_python" => about.natives.extend(native(NativeKind::Python, &[3, 4], a(2).zip(r(3)).map(|(kind, input)| NativeSpec::Python { kind, input, targets: opt(4) }))),
                    "native_components" => about.natives.extend(native(NativeKind::Components, &[2, 3], r(2).zip(r(3)).map(|(edge, own)| NativeSpec::Components { edge, own }))),
                    "native_reach" => about.natives.extend(native(NativeKind::Reach, &[2, 3], r(2).zip(r(3)).map(|(seed, edge)| NativeSpec::Reach { seed, edge }))),
                    "native_reach_through" => about.natives.extend(native(NativeKind::ReachThrough, &[2, 3], r(2).zip(r(3)).map(|(reach, map)| NativeSpec::ReachThrough { reach, map }))),
                    "lattice" => {
                        if let (Some(rel), Some(Cell::Int(keys)), Some(ty)) = (r(1), row.get(2).copied(), row.get(3).copied().and_then(atom)) {
                            about.lattice.push(LatticeRef { rel, keys: keys as usize, lattice: ty, ty, merge: None, widen: None });
                        }
                    }
                    "nullary" => about.nullary.extend(r(1)),
                    "lattice_merge" => merges.extend(r(1).zip(row.get(2).copied().and_then(atom))),
                    "lattice_widen" => widens.extend(r(1).zip(int(row.get(2).copied()).map(|k| k as u32))),
                    "gate" => {
                        about.gates += 1;
                        about.gate_keys.extend(row.get(2).copied().and_then(atom));
                        if let (Some(k), Some(key), Some(level)) = (int(row.get(1).copied()), row.get(2).copied().and_then(atom), int(row.get(4).copied())) {
                            gates.push((k, GateRef { key, rel: r(3), level: level as usize, src: None, quiet: Vec::new(), deps: Vec::new() }));
                        }
                    }
                    "gate_src" => gate_srcs.extend(int(row.get(1).copied()).zip(row.get(2).copied().and_then(atom))),
                    "gate_quiet" | "gate_dep" => {
                        if let (Some(k), Some(i), Some(rel)) = (int(row.get(1).copied()), int(row.get(2).copied()), r(3)) {
                            gate_parts.push((k, name == "gate_dep", i, rel));
                        }
                    }
                    "kindset" => sets.extend(row.get(1).copied().and_then(atom)),
                    "kindset_kind" => {
                        if let (Some(set), Some(k), Some(kind)) = (row.get(1).copied().and_then(atom), int(row.get(2).copied()), row.get(3).copied().and_then(atom)) {
                            kinds.push((set, k, kind));
                        }
                    }
                    "tape_column" => {
                        if let (Some(rel), Some(i)) = (r(1), int(row.get(2).copied())) {
                            about.tapes.push((rel, i as usize));
                        }
                    }
                    "shape" => {
                        if let (Some(k), Some(head), Some(lit), Some(text)) = (int(row.get(1).copied()), row.get(2).copied().and_then(atom), row.get(3).copied().and_then(atom), row.get(4).copied().and_then(atom)) {
                            shapes.push((k, ShapeRef { head, lit, wait: ShapeWaitRef::Bound(Vec::new()), text }));
                        }
                    }
                    "shape_after" => shape_after.extend(int(row.get(1).copied()).zip(row.get(2).copied().and_then(atom))),
                    "shape_bound" => {
                        if let (Some(k), Some(i), Some(x)) = (int(row.get(1).copied()), int(row.get(2).copied()), row.get(3).copied().and_then(atom)) {
                            shape_bound.push((k, i, x));
                        }
                    }
                    "qualifier" => about.qualifiers.extend(row.get(1).copied().and_then(atom).zip(row.get(2).copied().and_then(atom))),
                    "demand" => {
                        about.demand.extend(row.get(2).copied().and_then(atom));
                        demand.extend(int(row.get(1).copied()).zip(a(2)));
                    }
                    _ => {}
                }
            }
        }
        // A dfs's roots are read after its edge and target (`Native::reads`).
        for (rel, root) in roots {
            if let Some(n) = about.natives.iter_mut().find(|n| n.rel == rel) {
                n.reads.push(root);
                if let NativeSpec::DfsFirst { roots, .. } = &mut n.spec {
                    *roots = Some(root);
                }
            }
        }
        patterns.sort_unstable_by_key(|&(rel, i, _)| (rel, i));
        for (rel, _, p) in patterns {
            if let Some(NativeRef { spec: NativeSpec::TextTest { patterns, .. }, .. }) = about.natives.iter_mut().find(|n| n.rel == rel) {
                patterns.push(p);
            }
        }
        groups.sort_unstable_by_key(|&(rel, i, _)| (rel, i));
        for (rel, _, g) in groups {
            if let Some(NativeRef { spec: NativeSpec::MatchAll { groups, .. }, .. }) = about.natives.iter_mut().find(|n| n.rel == rel) {
                groups.push(g);
            }
        }
        demand.sort_unstable_by_key(|d| d.0);
        about.demand_list = demand.into_iter().map(|d| d.1).collect();
        for (rel, merge) in merges {
            if let Some(l) = about.lattice.iter_mut().find(|l| l.rel == rel) {
                l.lattice = merge;
                l.merge = Some(merge);
            }
        }
        for (rel, k) in widens {
            if let Some(l) = about.lattice.iter_mut().find(|l| l.rel == rel) {
                l.widen = Some(k);
            }
        }
        gates.sort_unstable_by_key(|g| g.0);
        gate_parts.sort_unstable();
        for (k, src) in gate_srcs {
            if let Some(g) = gates.iter_mut().find(|g| g.0 == k) {
                g.1.src = Some(src);
            }
        }
        for (k, dep, _, rel) in gate_parts {
            if let Some(g) = gates.iter_mut().find(|g| g.0 == k) {
                if dep { g.1.deps.push(rel) } else { g.1.quiet.push(rel) }
            }
        }
        about.gate_list = gates.into_iter().map(|g| g.1).collect();
        kinds.sort_unstable_by_key(|&(set, k, _)| (v.text(set), k));
        sets.sort_unstable_by_key(|&s| v.text(s));
        about.kindsets = sets.into_iter().map(|set| (set, kinds.iter().filter(|k| k.0 == set).map(|k| k.2).collect())).collect();
        about.tapes.sort_unstable();
        shapes.sort_unstable_by_key(|s| s.0);
        shape_bound.sort_unstable();
        for (k, a) in shape_after {
            if let Some(s) = shapes.iter_mut().find(|s| s.0 == k) {
                s.1.wait = ShapeWaitRef::After(a);
            }
        }
        for (k, _, x) in shape_bound {
            if let Some(s) = shapes.iter_mut().find(|s| s.0 == k)
                && let ShapeWaitRef::Bound(vs) = &mut s.1.wait
            {
                vs.push(x);
            }
        }
        about.shapes = shapes.into_iter().map(|s| s.1).collect();
        answers.sort_unstable_by_key(|&(req, i, _)| (v.rel_name(req), i));
        about.answers = answers.into_iter().map(|(req, _, p)| (req, p)).collect();
        about.natives.sort_unstable_by_key(|n| n.rel);
        about.lattice.sort_unstable_by_key(|l| l.rel);
        about.nullary.sort_unstable();
        about.qualifiers.sort_unstable();
        about.qualifiers.dedup();
        about.demand.sort_unstable();
        about.gate_keys.sort_unstable();
        about
    }

    pub fn native(&self, rel: RelId) -> Option<&NativeRef> {
        self.natives.binary_search_by_key(&rel, |n| n.rel).ok().map(|k| &self.natives[k])
    }
    pub fn lattice_of(&self, rel: RelId) -> Option<AtomId> {
        self.lattice_decl(rel).map(|l| l.lattice)
    }
    /// Whether relation `rel` (by name) has qualifier `word` (`Program::qualified`).
    pub fn qualified(&self, rel: AtomId, word: AtomId) -> bool {
        self.qualifiers.binary_search(&(rel, word)).is_ok()
    }
    pub fn lattice_decl(&self, rel: RelId) -> Option<&LatticeRef> {
        self.lattice.binary_search_by_key(&rel, |l| l.rel).ok().map(|k| &self.lattice[k])
    }
    /// Every produced relation with its request (`answers::produced`).
    pub fn produced(&self) -> impl Iterator<Item = (RelId, RelId)> + '_ {
        self.answers.iter().map(|&(req, p)| (p, req))
    }
}

/// The about rows' program name (`program_image::ABOUT`): each row's first cell.
/// The request record (`Program::asked_by`, decision 0174: the `asked` rows): each request relation's rules, in its
/// order among the program's, each with the relations whose readers it serves.
pub type Asked = std::collections::BTreeMap<String, Vec<std::collections::BTreeSet<String>>>;

impl Version {
    /// The request record, as the `asked` rows state it.
    pub fn asked(&self) -> Asked {
        let mut asked = Asked::new();
        for f in self.about_facts().into_iter().filter(|f| self.text(f.name()) == "asked") {
            for row in f.rows() {
                let cells: Vec<Cell> = row.collect();
                let (Some(Cell::Str(req)), Some(&Cell::Int(k)), Some(Cell::Str(b))) = (cells.get(1), cells.get(2), cells.get(3)) else { continue };
                let Ok(k) = usize::try_from(k) else { continue };
                let readers = asked.entry(self.text(*req).to_owned()).or_default();
                if readers.len() <= k {
                    readers.resize(k + 1, Default::default());
                }
                readers[k].insert(self.text(*b).to_owned());
            }
        }
        asked
    }
    /// Each request relation of the record with how many readers' entries it holds and how many rules it has: the
    /// record holds where the two are equal.
    pub fn asked_against_rules(&self) -> Vec<(String, usize, usize)> {
        self.asked().into_iter().map(|(req, readers)| {
            let rules = self.rel(&req).map_or(0, |id| self.heads(id).count());
            (req, readers.len(), rules)
        }).collect()
    }
}

fn program_cell(b: &mut Builder<'_>) -> Cell {
    Cell::Str(b.atom(super::ABOUT))
}

/// Edits of a version's other fields, by type: a pass states what it changes, and never writes an about row itself.
impl Builder<'_> {
    /// Sets lattice relation `rel`'s key columns.
    pub fn set_lattice_keys(&mut self, rel: &str, keys: usize) {
        let a = self.atom(rel);
        let mut rows = self.about_rows("lattice");
        for row in &mut rows {
            if row.get(1) == Some(&Cell::Str(a)) {
                row[2] = Cell::Int(keys as i64);
            }
        }
        self.set_about_rows("lattice", rows);
    }
    /// Renames readers in the request record (`asked` rows, column 3), as `Program::rename_readers` does: a request
    /// of a merged or folded reader serves the relation it went into. Each rule's readers are a set, so readers
    /// renamed to one are one row.
    pub fn rename_asked_readers(&mut self, rename: &dyn Fn(&str) -> Option<String>) {
        let rows = self.about_rows("asked");
        if !rows.iter().any(|row| matches!(row.get(3), Some(&Cell::Str(a)) if rename(self.text(a)).is_some())) {
            return;
        }
        let mut seen = std::collections::HashSet::new();
        let mut renamed = Vec::with_capacity(rows.len());
        for mut row in rows {
            if let Some(&Cell::Str(a)) = row.get(3)
                && let Some(t) = rename(self.text(a))
            {
                row[3] = Cell::Str(self.atom(&t));
            }
            if seen.insert(format!("{row:?}")) {
                renamed.push(row);
            }
        }
        self.set_about_rows("asked", renamed);
    }
    /// Removes relation `rel`'s native operator (`Program::natives`' entry): its rows of each native about relation,
    /// every one of which names its relation in column 1.
    pub fn drop_native(&mut self, rel: &str) {
        let a = Cell::Str(self.atom(rel));
        for &(name, _) in RELATION_COLUMNS.iter().filter(|(n, _)| n.starts_with("native_")) {
            let rows = self.about_rows(name);
            let n = rows.len();
            let kept: Vec<Vec<Cell>> = rows.into_iter().filter(|r| r.get(1) != Some(&a)).collect();
            if kept.len() != n {
                self.set_about_rows(name, kept);
            }
        }
    }
    /// States about relation `name`'s rows as `from`'s (a version whose ids are this builder's), those `keep` takes.
    pub fn copy_about(&mut self, from: &Version, name: &str, mut keep: impl FnMut(&[Cell]) -> bool) {
        let Some(a) = from.find(name) else { return };
        let Some(f) = from.about_facts().into_iter().find(|f| f.name() == a) else { return };
        let rows: Vec<Vec<Cell>> = f.rows().map(|r| r.collect::<Vec<Cell>>()).filter(|r| keep(r)).collect();
        if !rows.is_empty() {
            self.set_about_rows(name, rows);
        }
    }
    /// Keeps the natives, lattices and gates of the relations `keep` takes, by kind (decision 0244: a split part's
    /// fields, as the tree's `natives`, `lattice` and `gates` retained by relation). The gates kept are numbered again
    /// in their order, as the tree's list writes them; a gate is kept by its key (`Program::gates`' first). A kind
    /// none of whose rows go is left as the parent's.
    pub fn keep_relation_fields(&mut self, natives: &dyn Fn(&str) -> bool, lattice: &dyn Fn(&str) -> bool, gates: &dyn Fn(&str) -> bool) {
        let named = |b: &Self, row: &[Cell], col: usize, keep: &dyn Fn(&str) -> bool| match row.get(col) {
            Some(&Cell::Str(a)) => keep(b.text(a)),
            _ => true,
        };
        let filter = |b: &mut Self, name: &str, keep: &dyn Fn(&Self, &[Cell]) -> bool| {
            let rows = b.about_rows(name);
            let n = rows.len();
            let kept: Vec<Vec<Cell>> = rows.into_iter().filter(|r| keep(b, r)).collect();
            if kept.len() != n {
                b.set_about_rows(name, kept);
            }
        };
        for &(name, _) in RELATION_COLUMNS.iter().filter(|(n, _)| n.starts_with("native_")) {
            filter(self, name, &|b, r| named(b, r, 1, natives));
        }
        for name in ["lattice", "lattice_merge", "lattice_widen"] {
            filter(self, name, &|b, r| named(b, r, 1, lattice));
        }
        let main = self.about_rows("gate");
        let number = |r: &[Cell]| match r.get(1) {
            Some(&Cell::Int(k)) => Some(k),
            _ => None,
        };
        let mut kept: Vec<i64> = main.iter().filter(|r| named(self, r, 2, gates)).filter_map(|r| number(r)).collect();
        if kept.len() == main.len() {
            return;
        }
        kept.sort_unstable();
        let again = |k: i64| kept.binary_search(&k).ok().map(|i| i as i64);
        for name in ["gate", "gate_quiet", "gate_dep", "gate_src"] {
            let rows: Vec<Vec<Cell>> = self
                .about_rows(name)
                .into_iter()
                .filter_map(|mut r| {
                    let k = again(number(&r)?)?;
                    r[1] = Cell::Int(k);
                    Some(r)
                })
                .collect();
            self.set_about_rows(name, rows);
        }
    }
    /// Adds qualifiers `words` (relation, word) to the program's, kept in order of relation then word, each once, as
    /// the tree's map of sets.
    pub fn add_qualifiers<S: AsRef<str>>(&mut self, words: &[(S, S)]) {
        if words.is_empty() {
            return;
        }
        let p = program_cell(self);
        let rows = self.about_rows("qualifier");
        let mut pairs: Vec<(String, String)> = rows.iter().filter_map(|r| match (r.get(1), r.get(2)) {
            (Some(&Cell::Str(a)), Some(&Cell::Str(w))) => Some((self.text(a).to_owned(), self.text(w).to_owned())),
            _ => None,
        }).collect();
        pairs.extend(words.iter().map(|(r, w)| (r.as_ref().to_owned(), w.as_ref().to_owned())));
        pairs.sort_unstable();
        pairs.dedup();
        let rows = pairs.iter().map(|(r, w)| vec![p, Cell::Str(self.atom(r)), Cell::Str(self.atom(w))]).collect();
        self.set_about_rows("qualifier", rows);
    }
    /// States the qualifiers (`Program::qualifiers`): each (relation, word), in order of relation then word, each once,
    /// as the tree's map of sets. Replaces the program's.
    pub fn set_qualifiers<S: AsRef<str>>(&mut self, words: &[(S, S)]) {
        let p = program_cell(self);
        let mut pairs: Vec<(&str, &str)> = words.iter().map(|(r, w)| (r.as_ref(), w.as_ref())).collect();
        pairs.sort_unstable();
        pairs.dedup();
        let rows = pairs.iter().map(|(r, w)| vec![p, Cell::Str(self.atom(r)), Cell::Str(self.atom(w))]).collect();
        self.set_about_rows("qualifier", rows);
    }
    /// Adds the tree's gates `gates` (`Program::gates`' entries, after the program's), each with its source where it
    /// has one (`gate_src`), as `program_facts` writes them.
    pub fn add_tree_gates(&mut self, gates: &[(String, crate::code_rule_dsl::logic::front::ast::Gate)]) {
        if gates.is_empty() {
            return;
        }
        let p = program_cell(self);
        let base = self.about_rows("gate").len() as i64;
        let made: Vec<_> = gates.iter().map(|(key, g)| (key.clone(), g.rel.clone(), g.level, g.quiet.clone(), g.deps.clone())).collect();
        self.add_gates(&made);
        let mut srcs = self.about_rows("gate_src");
        for (k, (_, g)) in gates.iter().enumerate() {
            if let Some(src) = &g.src {
                srcs.push(vec![p, Cell::Int(base + k as i64), Cell::Str(self.atom(src))]);
            }
        }
        self.set_about_rows("gate_src", srcs);
    }
    /// States the answers (`Program::answers`): each request and the relations that answer it, in order. Replaces
    /// the program's.
    pub fn set_answers(&mut self, answers: &std::collections::BTreeMap<String, Vec<String>>) {
        let p = program_cell(self);
        let rows = answers
            .iter()
            .flat_map(|(req, by)| by.iter().enumerate().map(move |(i, b)| (req, i, b)))
            .map(|(req, i, b)| vec![p, Cell::Str(self.atom(req)), Cell::Int(i as i64), Cell::Str(self.atom(b))])
            .collect();
        self.set_about_rows("answer", rows);
    }
    /// Adds gates (`Program::gates`' entries, after the program's): each its key, relation, level, the relations
    /// that must be quiet and the gates it waits on, as `program_facts` writes them.
    pub fn add_gates(&mut self, gates: &[(String, String, usize, Vec<String>, Vec<String>)]) {
        if gates.is_empty() {
            return;
        }
        let p = program_cell(self);
        let (mut main, mut quiet, mut deps) = (self.about_rows("gate"), self.about_rows("gate_quiet"), self.about_rows("gate_dep"));
        let base = main.len() as i64;
        for (k, (key, rel, level, q, d)) in gates.iter().enumerate() {
            let k = base + k as i64;
            main.push(vec![p, Cell::Int(k), Cell::Str(self.atom(key)), Cell::Str(self.atom(rel)), Cell::Int(*level as i64)]);
            for (i, r) in q.iter().enumerate() {
                quiet.push(vec![p, Cell::Int(k), Cell::Int(i as i64), Cell::Str(self.atom(r))]);
            }
            for (i, r) in d.iter().enumerate() {
                deps.push(vec![p, Cell::Int(k), Cell::Int(i as i64), Cell::Str(self.atom(r))]);
            }
        }
        self.set_about_rows("gate", main);
        self.set_about_rows("gate_quiet", quiet);
        self.set_about_rows("gate_dep", deps);
    }
    /// Adds relations `rels` to the relations of no columns (kept in name order, each once, as the tree's set).
    pub fn add_nullary<S: AsRef<str>>(&mut self, rels: &[S]) {
        if rels.is_empty() {
            return;
        }
        let p = program_cell(self);
        let mut rows = self.about_rows("nullary");
        let mut names: Vec<String> = rows.iter().filter_map(|r| match r.get(1) {
            Some(&Cell::Str(a)) => Some(self.text(a).to_owned()),
            _ => None,
        }).collect();
        names.extend(rels.iter().map(|r| r.as_ref().to_owned()));
        names.sort_unstable();
        names.dedup();
        rows = names.iter().map(|n| vec![p, Cell::Str(self.atom(n))]).collect();
        self.set_about_rows("nullary", rows);
    }
    /// States the columns declared `tape` (`Program::tapes`): each relation and column, in the program's order
    /// (relations by name, each's columns in order). Replaces the program's.
    pub fn set_tapes<S: AsRef<str>>(&mut self, tapes: &[(S, usize)]) {
        let p = program_cell(self);
        let rows = tapes.iter().map(|(r, c)| vec![p, Cell::Str(self.atom(r.as_ref())), Cell::Int(*c as i64)]).collect();
        self.set_about_rows("tape_column", rows);
    }
}
