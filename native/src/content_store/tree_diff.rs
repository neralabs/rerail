//! The Git root-tree comparison that seeds the push batch (docs/final-shape/03-architecture.md
//! §2, §15). Two root trees are walked together, one level per statement, over the stored
//! `git_trees`/`git_tree_entries` rows (the reachability walk in `reclamation.rs` is its pattern).
//! A subtree whose hash is the same on both sides is never read: that is where an edit's cost
//! stops growing with the repository (Tempo E31–E34).

use super::{ContentStore, HashKey, StoreError, StoreResult};
use std::collections::{BTreeMap, HashMap};

/// `git_tree_entries.object_kind_id` of a subtree.
const KIND_TREE: i64 = 1;

/// One path whose object changed between two root trees: a directory (its tree hash) or a file
/// or submodule (its blob or commit hash). `digest` is `ABSENT` where the path no longer exists.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct TreeChange {
    pub path: String,
    pub tree: bool,
    pub digest: [u8; 32],
}

/// The changes between two root trees and how many stored trees the walk read.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TreeDiff {
    pub changes: Vec<TreeChange>,
    pub trees_read: usize,
}

struct Entry {
    tree: bool,
    digest: [u8; 32],
    child: Option<i64>,
}

impl ContentStore {
    /// The paths whose object differs between root tree `old` (none: nothing was observed) and
    /// `new`, the root itself (path "") included, sorted. Both sides are walked from stored
    /// complete trees; a subtree whose tree is not stored contributes only its own path.
    pub fn tree_changes(&self, old: Option<HashKey>, new: HashKey) -> StoreResult<TreeDiff> {
        let connection = self.connection.lock().expect("content store mutex poisoned");
        let mut tree_id = connection.prepare_cached(
            "SELECT t.tree_id FROM hashes h INDEXED BY hashes_by_prefix JOIN git_trees t ON t.tree_hash_id=h.hash_id \
             WHERE h.algorithm_id=?1 AND h.lane0=?2 AND h.lane1=?3 AND h.lane2=?4 AND h.lane3=?5 AND t.complete=1",
        )?;
        let mut root = |key: HashKey| -> StoreResult<i64> {
            tree_id
                .query_row(rusqlite::params_from_iter(super::rows::hash_params(key)), |row| row.get(0))
                .map_err(|error| match error {
                    rusqlite::Error::QueryReturnedNoRows => {
                        StoreError::Invalid("root tree is not stored".to_owned())
                    }
                    error => error.into(),
                })
        };
        let mut diff = TreeDiff::default();
        if old == Some(new) {
            return Ok(diff);
        }
        let old_root = old.map(&mut root).transpose()?;
        let new_root = root(new)?;
        diff.changes.push(TreeChange { path: String::new(), tree: true, digest: digest(new.lanes) });
        let mut entries = connection.prepare_cached(
            "SELECT i.value,s.value,e.object_kind_id,h.lane0,h.lane1,h.lane2,h.lane3,c.tree_id \
             FROM json_each(?1) i JOIN git_tree_entries e ON e.tree_id=i.value \
             JOIN strings s ON s.string_id=e.name_string_id JOIN hashes h ON h.hash_id=e.target_hash_id \
             LEFT JOIN git_trees c ON e.object_kind_id=?2 AND c.tree_hash_id=e.target_hash_id AND c.complete=1",
        )?;
        let mut level = vec![(String::new(), old_root, Some(new_root))];
        while !level.is_empty() {
            let ids = level
                .iter()
                .flat_map(|(_, old, new)| [*old, *new])
                .flatten()
                .collect::<Vec<_>>();
            diff.trees_read += ids.len();
            let encoded = serde_json::to_string(&ids).map_err(|error| StoreError::Invalid(error.to_string()))?;
            let mut listed: HashMap<i64, BTreeMap<String, Entry>> = HashMap::new();
            let rows = entries.query_map(rusqlite::params![encoded, KIND_TREE], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    [row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?],
                    row.get::<_, Option<i64>>(7)?,
                ))
            })?;
            for row in rows {
                let (tree, name, kind, lanes, child) = row?;
                let entry = Entry { tree: kind == KIND_TREE, digest: digest(lanes), child };
                listed.entry(tree).or_default().insert(name, entry);
            }
            let empty = BTreeMap::new();
            let mut next = Vec::new();
            for (prefix, old, new) in level {
                let side = |id: Option<i64>| id.and_then(|id| listed.get(&id)).unwrap_or(&empty);
                let (old, new) = (side(old), side(new));
                let names = old.keys().chain(new.keys().filter(|name| !old.contains_key(*name)));
                for name in names {
                    let path = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
                    let (before, after) = (old.get(name), new.get(name));
                    for tree in [false, true] {
                        let before = before.filter(|entry| entry.tree == tree);
                        let after = after.filter(|entry| entry.tree == tree);
                        if before.map(|entry| entry.digest) == after.map(|entry| entry.digest) {
                            continue;
                        }
                        let digest = after.map_or(super::ABSENT, |entry| entry.digest);
                        diff.changes.push(TreeChange { path: path.clone(), tree, digest });
                        if tree {
                            next.push((path.clone(), before.and_then(|e| e.child), after.and_then(|e| e.child)));
                        }
                    }
                }
            }
            level = next;
        }
        diff.changes.sort();
        Ok(diff)
    }

    /// `tree_changes` from the root whose digest an observer last bound (`old`, of `new`'s hash
    /// algorithm). A last root whose trees were collected compares as nothing observed: every path
    /// is listed, and rebinding the unchanged ones changes nothing.
    pub fn tree_changes_since(&self, old: Option<[u8; 32]>, new: HashKey) -> StoreResult<TreeDiff> {
        let old = match old.map(|digest| HashKey::from_padded(new.algorithm, digest)) {
            Some(old) if self.tree_stored(old)? => Some(old),
            _ => None,
        };
        self.tree_changes(old, new)
    }

    fn tree_stored(&self, key: HashKey) -> StoreResult<bool> {
        let connection = self.connection.lock().expect("content store mutex poisoned");
        connection
            .prepare_cached(
                "SELECT EXISTS (SELECT 1 FROM hashes h INDEXED BY hashes_by_prefix JOIN git_trees t ON t.tree_hash_id=h.hash_id \
                 WHERE h.algorithm_id=?1 AND h.lane0=?2 AND h.lane1=?3 AND h.lane2=?4 AND h.lane3=?5 AND t.complete=1)",
            )?
            .query_row(rusqlite::params_from_iter(super::rows::hash_params(key)), |row| row.get(0))
            .map_err(StoreError::from)
    }
}

fn digest(lanes: [i64; 4]) -> [u8; 32] {
    let mut digest = [0; 32];
    for (chunk, lane) in digest.chunks_exact_mut(8).zip(lanes) {
        chunk.copy_from_slice(&lane.to_be_bytes());
    }
    digest
}

#[cfg(test)]
mod tests;
