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
    ("native_dfs", &[1, 2, 3]),
    ("native_roots", &[1, 2]),
    ("native_match", &[1, 2]),
    ("native_group", &[1]),
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
    PythonCandidates,
    Python,
    Components,
    Reach,
    ReachThrough,
}

/// A native relation: its kind and the relations it reads (`front::Native::reads`, in its order).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeRef {
    pub rel: RelId,
    pub kind: NativeKind,
    pub reads: Vec<RelId>,
}

/// A lattice relation's declaration, as the analyses read it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LatticeRef {
    pub rel: RelId,
    /// The key columns before the value column.
    pub keys: usize,
    /// `LatticeDecl::lattice`: the merge's name, else the value column's type.
    pub lattice: AtomId,
}

/// A version's other fields, read once from its about rows.
#[derive(Clone, Debug, Default)]
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
}

impl About {
    /// The about rows of `v`, read.
    pub fn of(v: &Version) -> About {
        let mut about = About::default();
        let rel = |c: Cell| match c {
            Cell::Str(a) => v.rel_of(a),
            Cell::Int(_) => None,
        };
        let atom = |c: Cell| match c {
            Cell::Str(a) => Some(a),
            Cell::Int(_) => None,
        };
        let mut merges: Vec<(RelId, AtomId)> = Vec::new();
        let mut roots: Vec<(RelId, RelId)> = Vec::new();
        let mut answers: Vec<(RelId, i64, RelId)> = Vec::new();
        for f in v.about_facts() {
            let name = v.text(f.name());
            for row in f.rows() {
                let row: Vec<Cell> = row.collect();
                let r = |k: usize| row.get(k).copied().and_then(rel);
                let native = |kind, cols: &[usize]| {
                    let reads = cols.iter().filter_map(|&k| r(k)).collect();
                    r(1).map(|rel| NativeRef { rel, kind, reads })
                };
                match name {
                    "answer" => {
                        if let (Some(req), Some(Cell::Int(i)), Some(p)) = (r(1), row.get(2).copied(), r(3)) {
                            answers.push((req, i, p));
                        }
                    }
                    "native_dfs" => about.natives.extend(native(NativeKind::DfsFirst, &[2, 3])),
                    "native_roots" => roots.extend(r(1).zip(r(2))),
                    "native_match" => about.natives.extend(native(NativeKind::MatchAll, &[2])),
                    "native_python_candidates" => about.natives.extend(native(NativeKind::PythonCandidates, &[2, 3])),
                    // An empty `targets` is none: it names no relation.
                    "native_python" => about.natives.extend(native(NativeKind::Python, &[3, 4])),
                    "native_components" => about.natives.extend(native(NativeKind::Components, &[2, 3])),
                    "native_reach" => about.natives.extend(native(NativeKind::Reach, &[2, 3])),
                    "native_reach_through" => about.natives.extend(native(NativeKind::ReachThrough, &[2, 3])),
                    "lattice" => {
                        if let (Some(rel), Some(Cell::Int(keys)), Some(ty)) = (r(1), row.get(2).copied(), row.get(3).copied().and_then(atom)) {
                            about.lattice.push(LatticeRef { rel, keys: keys as usize, lattice: ty });
                        }
                    }
                    "nullary" => about.nullary.extend(r(1)),
                    "lattice_merge" => merges.extend(r(1).zip(row.get(2).copied().and_then(atom))),
                    "gate" => about.gates += 1,
                    _ => {}
                }
            }
        }
        // A dfs's roots are read after its edge and target (`Native::reads`).
        for (rel, root) in roots {
            if let Some(n) = about.natives.iter_mut().find(|n| n.rel == rel) {
                n.reads.push(root);
            }
        }
        for (rel, merge) in merges {
            if let Some(l) = about.lattice.iter_mut().find(|l| l.rel == rel) {
                l.lattice = merge;
            }
        }
        answers.sort_unstable_by_key(|&(req, i, _)| (v.rel_name(req), i));
        about.answers = answers.into_iter().map(|(req, _, p)| (req, p)).collect();
        about.natives.sort_unstable_by_key(|n| n.rel);
        about.lattice.sort_unstable_by_key(|l| l.rel);
        about.nullary.sort_unstable();
        about
    }

    pub fn native(&self, rel: RelId) -> Option<&NativeRef> {
        self.natives.binary_search_by_key(&rel, |n| n.rel).ok().map(|k| &self.natives[k])
    }
    pub fn lattice_of(&self, rel: RelId) -> Option<AtomId> {
        self.lattice_decl(rel).map(|l| l.lattice)
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
}
