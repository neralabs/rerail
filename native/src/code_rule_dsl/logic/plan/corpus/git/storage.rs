//! The on-disk representation of objects, independent of decoded Git facts.

use sha2::{Digest as _, Sha256};
use std::path::{Path, PathBuf};

pub(super) struct Storage {
    roots: Vec<PathBuf>,
    loose: Sha256,
    packs: Sha256,
    indices: Option<Vec<PackIndex>>,
}

enum PackIndex {
    Single(gix::odb::pack::index::File),
    Multi(gix::odb::pack::multi_index::File),
}

impl PackIndex {
    fn contains(&self, id: &gix::hash::oid) -> bool {
        match self {
            Self::Single(index) => index.lookup(id).is_some(),
            Self::Multi(index) => index.lookup(id).is_some(),
        }
    }
}

impl Storage {
    pub(super) fn of(repo: &gix::Repository) -> Option<Self> {
        let store = repo.objects.store_ref();
        let mut roots = vec![store.path().to_owned()];
        roots.extend(store.alternate_db_paths().ok()?);
        let mut loose = Sha256::new();
        let mut packs = Sha256::new();
        loose.update(b"git-loose-storage-v1");
        packs.update(b"git-pack-storage-v1");
        let mut indices = Some(Vec::new());
        for root in &roots {
            hash_path(&mut loose, root);
            stat(&mut loose, &root.join("info/alternates"))?;
            let directory = root.join("pack");
            let mut files = match std::fs::read_dir(&directory) {
                Ok(entries) => entries.map(|entry| entry.map(|entry| entry.path())).collect::<std::io::Result<Vec<_>>>().ok()?,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
                Err(_) => return None,
            };
            files.sort_unstable();
            for file in files {
                let extension = file.extension().and_then(|ext| ext.to_str());
                let multi = file.file_name().is_some_and(|name| name == "multi-pack-index");
                if !multi && !matches!(extension, Some("idx" | "pack")) {
                    continue;
                }
                if let Some(known) = &mut indices && (multi || extension == Some("idx")) {
                    let index = if multi {
                        gix::odb::pack::multi_index::File::at(&file, None).ok().map(PackIndex::Multi)
                    } else {
                        gix::odb::pack::index::File::at(&file, repo.object_hash()).ok().map(PackIndex::Single)
                    };
                    match index {
                        Some(index) => known.push(index),
                        None => indices = None,
                    }
                }
                hash_path(&mut packs, &file);
                stat(&mut packs, &file)?;
            }
        }
        Some(Self { roots, loose, packs, indices })
    }

    pub(super) fn object(&self, id: gix::ObjectId) -> Option<String> {
        let oid = id.to_string();
        let mut stamp = self.loose.clone();
        // Git can prefer a pack over a loose copy. Only omit pack state when
        // every readable index proves that this object is loose-only.
        if self.indices.as_ref().is_none_or(|indices| indices.iter().any(|index| index.contains(id.as_ref()))) {
            stamp.update(self.packs.clone().finalize());
        }
        for root in &self.roots {
            stat(&mut stamp, &root.join(&oid[..2]).join(&oid[2..]))?;
        }
        Some(format!("readable:{}", super::hex(&stamp.finalize())))
    }
}

fn hash_path(digest: &mut Sha256, path: &Path) {
    let bytes = path.as_os_str().as_encoded_bytes();
    digest.update((bytes.len() as u64).to_le_bytes());
    digest.update(bytes);
}

fn stat(digest: &mut Sha256, path: &Path) -> Option<()> {
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            digest.update([0]);
            return Some(());
        }
        Err(_) => return None,
    };
    digest.update([1]);
    digest.update(metadata.len().to_le_bytes());
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        digest.update(metadata.dev().to_le_bytes());
        digest.update(metadata.ino().to_le_bytes());
        digest.update(metadata.mtime().to_le_bytes());
        digest.update(metadata.mtime_nsec().to_le_bytes());
        digest.update(metadata.ctime().to_le_bytes());
        digest.update(metadata.ctime_nsec().to_le_bytes());
    }
    #[cfg(not(unix))]
    {
        let time = |timestamp: std::io::Result<std::time::SystemTime>| {
            timestamp.ok()?.duration_since(std::time::UNIX_EPOCH).ok().map(|time| time.as_nanos().to_le_bytes())
        };
        digest.update(time(metadata.modified())?);
        digest.update(time(metadata.created())?);
    }
    Some(())
}
