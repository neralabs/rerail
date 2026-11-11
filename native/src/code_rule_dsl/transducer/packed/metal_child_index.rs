//! Input-bound cardinality proofs and shared indexes for finite child joins.
#[cfg(target_os = "macos")]
use super::metal_batch_abi::MetalProgram;
use super::packed_batch::PackedBatch;
use super::*;
use std::sync::OnceLock;

type RegisterPaths = Vec<Vec<usize>>;
type ProgramPaths = Vec<Option<RegisterPaths>>;

/// A single child selected by an infallible guard in an initial linear prefix.
#[cfg(target_os = "macos")]
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[repr(u8)]
pub(super) enum InitialSelector {
    Field = 0,
    Kind = 1,
    FirstAny = 2,
    FirstNamed = 3,
}

#[cfg(target_os = "macos")]
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct InitialChild {
    pub guard_offset: usize,
    pub selector: InitialSelector,
    pub maximum_children: usize,
}

#[derive(Default)]
struct Cardinality {
    global: HashMap<u32, usize>,
    kinds: HashMap<(u32, u32), usize>,
    unique: Vec<u32>,
}
impl Cardinality {
    fn add(&mut self, kind: u32, entries: &[(u32, u32)]) {
        for group in entries.chunk_by(|left, right| left.0 == right.0) {
            let key = group[0].0;
            let count = group.len();
            let global = self.global.entry(key).or_default();
            *global = (*global).max(count);
            let by_kind = self.kinds.entry((kind, key)).or_default();
            *by_kind = (*by_kind).max(count);
            if count == 1 {
                self.unique.push(group[0].1);
            }
        }
    }
    fn bounded(&self, parent: Option<u32>, key: u32) -> bool {
        parent
            .map_or_else(
                || self.global.get(&key),
                |kind| self.kinds.get(&(kind, key)),
            )
            .copied()
            .unwrap_or(0)
            <= 1
    }
}

pub(super) struct ChildIndex {
    owner: Arc<PackedBatch>,
    fields: Cardinality,
    kinds: Cardinality,
    paths: OnceLock<ProgramPaths>,
    max_children: usize,
    max_children_by_kind: HashMap<u32, usize>,
}
impl ChildIndex {
    pub(super) fn new(owner: Arc<PackedBatch>) -> Result<Self, Error> {
        let mut index = Self {
            owner,
            fields: Cardinality::default(),
            kinds: Cardinality::default(),
            paths: OnceLock::new(),
            max_children: 0,
            max_children_by_kind: HashMap::new(),
        };
        let mut field_keys = HashSet::new();
        let mut kind_keys = HashSet::new();
        let mut has_child = false;
        for pg in &index.owner.programs {
            let words = &index.owner.words[pg.structural_start as usize
                ..(pg.structural_start + pg.structural_count) as usize];
            for (pc, &word) in words.iter().enumerate() {
                if split(word).0 != CHILD {
                    continue;
                }
                has_child = true;
                let (field, kind) = selectors(words, pc);
                if let Some(at) = field {
                    field_keys.insert(u32::from(split(words[at]).2));
                }
                if let Some(at) = kind {
                    kind_keys.insert(u32::from(split(words[at]).2));
                }
            }
        }
        if !has_child {
            return Ok(index);
        }
        let mut fields = Vec::new();
        let mut kinds = Vec::new();
        for (parent, node) in index.owner.nodes.iter().enumerate() {
            index.max_children = index.max_children.max(node.child_count as usize);
            let maximum = index.max_children_by_kind.entry(node.kind).or_default();
            *maximum = (*maximum).max(node.child_count as usize);
            if field_keys.is_empty() && kind_keys.is_empty() {
                continue;
            }
            fields.clear();
            kinds.clear();
            let start = node.child_start as usize;
            let end = start
                .checked_add(node.child_count as usize)
                .ok_or(Error::InvalidAnchor)?;
            for &id in index
                .owner
                .children
                .get(start..end)
                .ok_or(Error::InvalidAnchor)?
            {
                let child = index
                    .owner
                    .nodes
                    .get(id as usize)
                    .ok_or(Error::InvalidAnchor)?;
                if child.parent as usize != parent {
                    return Err(Error::InvalidAnchor);
                }
                if field_keys.contains(&child.field) {
                    fields.push((child.field, id));
                }
                if kind_keys.contains(&child.kind) {
                    kinds.push((child.kind, id));
                }
            }
            fields.sort_unstable();
            kinds.sort_unstable();
            index.fields.add(node.kind, &fields);
            index.kinds.add(node.kind, &kinds);
        }
        Ok(index)
    }
    pub(super) fn validate(&self, owner: &Arc<PackedBatch>) -> Result<(), Error> {
        if !Arc::ptr_eq(&self.owner, owner) {
            return Err(Error::InvalidAnchor);
        }
        Ok(())
    }
    #[cfg(target_os = "macos")]
    pub(super) fn field_nodes(&self) -> &[u32] {
        &self.fields.unique
    }
    #[cfg(target_os = "macos")]
    pub(super) fn kind_nodes(&self) -> &[u32] {
        &self.kinds.unique
    }

    /// A failed unique child guard can reject a seed only when every losing
    /// child reaches that guard before the original task can exhaust its work.
    #[cfg(target_os = "macos")]
    pub(super) fn seed_selector(
        &self,
        words: &[u32],
        pc: usize,
        parent: Option<u32>,
    ) -> Option<(u32, bool)> {
        let (field, kind) = selectors(words, pc);
        let last = field.into_iter().chain(kind).max()?;
        let children = parent.map_or(self.max_children, |kind| {
            self.max_children_by_kind.get(&kind).copied().unwrap_or(0)
        });
        if children.saturating_mul(last - pc + 1).saturating_add(64) > MAX_WORK_PER_TASK {
            return None;
        }
        if let Some(at) = field
            && self.fields.bounded(parent, u32::from(split(words[at]).2))
        {
            return Some((u32::from(split(words[at]).2), false));
        }
        kind.filter(|&at| self.kinds.bounded(parent, u32::from(split(words[at]).2)))
            .map(|at| (u32::from(split(words[at]).2), true))
    }

    fn parent_kind(words: &[u32], path: &[usize]) -> Option<u32> {
        let mut parent = None;
        let mut cursors = BTreeMap::new();
        for &at in path {
            let (op, _, operand) = split(words[at]);
            match op {
                KIND => parent = Some(u32::from(operand)),
                SAVE_CURSOR => {
                    cursors.retain(|&slot, _| slot <= operand);
                    cursors.insert(operand, parent);
                }
                RESTORE_CURSOR => parent = cursors.get(&operand).copied().flatten(),
                CHILD | NEXT | PARENT => parent = None,
                _ => {}
            }
        }
        parent
    }

    fn selected_guard(
        &self,
        words: &[u32],
        path: &[usize],
        pc: usize,
    ) -> Option<(usize, bool, usize)> {
        let parent = Self::parent_kind(words, path);
        let (field, kind) = selectors(words, pc);
        let guard = field
            .filter(|&at| self.fields.bounded(parent, u32::from(split(words[at]).2)))
            .into_iter()
            .chain(kind.filter(|&at| self.kinds.bounded(parent, u32::from(split(words[at]).2))))
            .min()?;
        // Unselected children still consume logical instructions. Bound their
        // guard work even on paths that stop before EMIT.
        let children = parent.map_or(self.max_children, |kind| {
            self.max_children_by_kind.get(&kind).copied().unwrap_or(0)
        });
        Some((guard, Some(guard) == kind, children))
    }

    fn prove(&self, words: &[u32], path: &[usize], pc: usize) -> Option<usize> {
        let (guard, _, children) = self.selected_guard(words, path, pc)?;
        Some(children.saturating_mul(guard - pc))
    }

    #[cfg(target_os = "macos")]
    pub(super) fn initial_child(
        &self,
        words: &[u32],
        path: &[usize],
        pc: usize,
    ) -> Option<InitialChild> {
        let parent = Self::parent_kind(words, path);
        let maximum_children = parent.map_or(self.max_children, |kind| {
            self.max_children_by_kind.get(&kind).copied().unwrap_or(0)
        });
        let mut selected = self
            .selected_guard(words, path, pc)
            .map(|(guard, kind, _)| {
                (
                    guard,
                    if kind {
                        InitialSelector::Kind
                    } else {
                        InitialSelector::Field
                    },
                )
            });
        for (at, &word) in words.iter().enumerate().skip(pc + 1).take(64) {
            match split(word) {
                (FIRST, mode, _) => {
                    if selected.is_none_or(|(guard, _)| at < guard) {
                        selected = Some((
                            at,
                            match mode {
                                0 => InitialSelector::FirstAny,
                                NAMED => InitialSelector::FirstNamed,
                                _ => return None,
                            },
                        ));
                    }
                    break;
                }
                (FIELD | KIND | FLAGS | SIZE | LAST | ABSENT_FIELD | SUPERTYPE, _, _) => {}
                _ => break,
            }
        }
        let (guard, selector) = selected?;
        // Named rank needs only a bounded metadata walk. FIRST has rank zero
        // among named children, regardless of the complete list's cardinality.
        if (split(words[pc]).1 == NAMED
            && matches!(selector, InitialSelector::Field | InitialSelector::Kind)
            && maximum_children > 64)
            || maximum_children
                .saturating_mul(guard - pc + 1)
                .saturating_add(64)
                > MAX_WORK_PER_TASK
        {
            return None;
        }
        Some(InitialChild {
            guard_offset: guard - pc,
            selector,
            maximum_children,
        })
    }

    fn program_paths(&self) -> &ProgramPaths {
        self.paths.get_or_init(|| {
            self.owner
                .programs
                .iter()
                .map(|pg| {
                    let batch = &self.owner;
                    let first = pg.structural_start as usize;
                    let end = first.checked_add(pg.structural_count as usize)?;
                    let words = batch.words.get(first..end)?;
                    let child_work = std::cell::Cell::new(0usize);
                    let paths = super::execution_shape::register_paths_for_with_children(
                        words,
                        pg.flags & 1 != 0,
                        |operand| {
                            batch
                                .values
                                .get((pg.value_start + u32::from(operand)) as usize)
                                .is_some_and(|value| {
                                    value.kind <= 1
                                        && batch
                                            .strings
                                            .get((pg.string_start + value.operand) as usize)
                                            .is_some_and(|string| string.byte_length <= 64)
                                })
                        },
                        |path, pc| {
                            if pg.flags & 2 != 0 {
                                return false;
                            }
                            if let Some(work) = self.prove(words, path, pc) {
                                child_work.set(child_work.get().saturating_add(work));
                                true
                            } else {
                                false
                            }
                        },
                    );
                    if child_work
                        .get()
                        .saturating_add(super::execution_shape::REGISTER_VISIT_LIMIT)
                        > MAX_WORK_PER_TASK
                    {
                        return None;
                    }
                    paths
                })
                .collect()
        })
    }
    pub(super) fn eligible(&self, program: usize) -> bool {
        self.program_paths()
            .get(program)
            .is_some_and(Option::is_some)
    }
    #[cfg(target_os = "macos")]
    pub(super) fn paths(&self, program: &MetalProgram) -> Result<Option<&Vec<Vec<usize>>>, Error> {
        let index = self
            .owner
            .programs
            .binary_search_by_key(&program.structural_start, |pg| pg.structural_start)
            .map_err(|_| Error::InvalidAnchor)?;
        if self.owner.programs[index] != *program {
            return Err(Error::InvalidAnchor);
        }
        Ok(self.program_paths()[index].as_ref())
    }
}

/// These guards constrain the selected child without changing its identity.
/// Branches, predicates, edits, and navigation stop the common guard prefix.
pub(super) fn selectors(words: &[u32], pc: usize) -> (Option<usize>, Option<usize>) {
    let mut field = None;
    let mut kind = None;
    for (at, &word) in words.iter().enumerate().skip(pc + 1).take(64) {
        match split(word).0 {
            FIELD => {
                field.get_or_insert(at);
            }
            KIND => {
                kind.get_or_insert(at);
            }
            FLAGS | SIZE | FIRST | LAST | ABSENT_FIELD | SUPERTYPE => {}
            _ => break,
        }
    }
    (field, kind)
}

#[cfg(target_os = "macos")]
pub(super) fn lookup_source(words: &[u32], pc: usize, mode: u8) -> Result<String, Error> {
    let (field, kind) = selectors(words, pc);
    if field.is_none() && kind.is_none() {
        return Err(Error::BackendFailure);
    }
    let mut source = "if(live) { uint child=NONE;\n".to_owned();
    for (at, lookup) in [(field, 0), (kind, 1)] {
        if let Some(at) = at {
            writeln!(source, "if(child==NONE) child=unique_child(v,node,(v.words[pg.structural_start+{at}u]>>16),{lookup}u);").unwrap();
        }
    }
    writeln!(
        source,
        "node=child; live=node!=NONE{}; }}",
        if mode == NAMED {
            " && (v.nodes[node].flags&1u)!=0u"
        } else {
            ""
        }
    )
    .unwrap();
    Ok(source)
}
