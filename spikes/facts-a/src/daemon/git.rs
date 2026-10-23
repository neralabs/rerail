//! Bulk sync from the Git object database: a commit's tree as (path, blob id) for the files the
//! per-file engine reads (`.py`, and `.rs` for regex clauses, as `facts-a repo` walks them), and
//! blob bytes on demand (only for blob ids the store does not know).
use gix::bstr::ByteSlice;

pub struct TreeFile {
    pub path: String,
    pub oid: [u8; 20],
}

pub struct Odb {
    repo: gix::Repository,
}

fn wanted(path: &str) -> bool {
    path.ends_with(".py") || path.ends_with(".rs")
}

impl Odb {
    pub fn open(git_dir: &std::path::Path) -> Result<Odb, String> {
        let repo = gix::open(git_dir).map_err(|e| format!("{}: {e}", git_dir.display()))?;
        if repo.object_hash() != gix::hash::Kind::Sha1 {
            return Err(format!("{}: only SHA-1 repositories are supported", git_dir.display()));
        }
        Ok(Odb { repo })
    }

    /// Every wanted blob of `commit`'s tree (a full hex commit id), in path order.
    pub fn tree_files(&self, commit: &str) -> Result<Vec<TreeFile>, String> {
        let id = gix::ObjectId::from_hex(commit.as_bytes()).map_err(|e| format!("commit {commit}: {e}"))?;
        let commit = self.repo.find_commit(id).map_err(|e| format!("commit {commit}: {e}"))?;
        let tree = commit.tree().map_err(|e| format!("tree of {id}: {e}"))?;
        let mut out = Vec::new();
        self.visit(tree, "", &mut out)?;
        out.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(out)
    }

    fn visit(&self, tree: gix::Tree<'_>, prefix: &str, out: &mut Vec<TreeFile>) -> Result<(), String> {
        let decoded = tree.decode().map_err(|e| format!("tree {prefix}: {e}"))?;
        for entry in decoded.entries {
            let name = entry.filename.to_str().map_err(|_| format!("{prefix}: a file name is not UTF-8"))?;
            let path = if prefix.is_empty() { name.to_owned() } else { format!("{prefix}/{name}") };
            if entry.mode.is_tree() {
                let child = self.repo.find_object(entry.oid).map_err(|e| format!("{path}: {e}"))?.try_into_tree().map_err(|e| format!("{path}: {e}"))?;
                self.visit(child, &path, out)?;
            } else if entry.mode.is_blob() && wanted(&path) {
                let oid: [u8; 20] = entry.oid.as_bytes().try_into().map_err(|_| format!("{path}: not a SHA-1 id"))?;
                out.push(TreeFile { path, oid });
            }
        }
        Ok(())
    }

    pub fn blob(&self, oid: &[u8; 20]) -> Result<Vec<u8>, String> {
        let id = gix::ObjectId::try_from(&oid[..]).map_err(|e| format!("blob id: {e}"))?;
        let object = self.repo.find_object(id).map_err(|e| format!("blob {id}: {e}"))?;
        Ok(object.detach().data)
    }
}

pub fn hex20(s: &str) -> Option<[u8; 20]> {
    if s.len() != 40 {
        return None;
    }
    let mut out = [0u8; 20];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(s.get(2 * i..2 * i + 2)?, 16).ok()?;
    }
    Some(out)
}
