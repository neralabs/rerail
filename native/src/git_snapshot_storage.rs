//! The on-disk representation of objects, independent of decoded Git facts.

use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub struct Storage {
    roots: Vec<PathBuf>,
    loose: Sha256,
    packs: Sha256,
    indices: Option<Vec<PackIndex>>,
    watched: BTreeMap<PathBuf, [u8; 32]>,
}

impl std::fmt::Debug for Storage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Storage").field("roots", &self.roots).finish_non_exhaustive()
    }
}

enum PackIndex {
    Single(gix::odb::pack::index::File),
    Multi(gix::odb::pack::multi_index::File),
}

impl PackIndex {
    fn observe(&self, id: &gix::hash::oid, files: &BTreeMap<PathBuf, [u8; 32]>, stamp: &mut Sha256) -> Option<()> {
        let (index_path, pack_path) = match self {
            Self::Single(index) => {
                if index.lookup(id).is_none() { return Some(()); }
                (index.path(), index.path().with_extension("pack"))
            }
            Self::Multi(index) => {
                let Some(entry) = index.lookup(id) else { return Some(()) };
                let (pack, _) = index.pack_id_and_pack_offset_at_index(entry).ok()?;
                let name = index.index_names().get(pack as usize)?;
                (index.path(), index.path().parent()?.join(name).with_extension("pack"))
            }
        };
        for path in [index_path, pack_path.as_path()] {
            hash_path(stamp, path);
            stamp.update(files.get(path)?);
        }
        Some(())
    }
}

impl Storage {
    pub fn of(repo: &gix::Repository) -> Option<Self> {
        let store = repo.objects.store_ref();
        let mut roots = vec![store.path().to_owned()];
        roots.extend(store.alternate_db_paths().ok()?);
        let mut loose = Sha256::new();
        let mut packs = Sha256::new();
        loose.update(b"git-loose-storage-v1");
        packs.update(b"git-pack-storage-v1");
        let mut indices = Some(Vec::new());
        let mut watched = BTreeMap::new();
        for root in &roots {
            hash_path(&mut loose, root);
            watch(&mut watched, Some(&mut loose), root.join("info/alternates"))?;
            let directory = root.join("pack");
            watch(&mut watched, None, directory.clone())?;
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
                hash_path(&mut packs, &file);
                watch(&mut watched, Some(&mut packs), file.clone())?;
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
            }
        }
        let storage = Self { roots, loose, packs, indices, watched };
        storage.is_current().then_some(storage)
    }

    pub fn is_current(&self) -> bool {
        self.watched.iter().all(|(path, expected)| {
            let mut digest = Sha256::new();
            stat(&mut digest, path).is_some() && &<[u8; 32]>::from(digest.finalize()) == expected
        })
    }

    pub fn object(&self, id: gix::ObjectId) -> Option<String> {
        let oid = id.to_string();
        let mut stamp = self.loose.clone();
        // More than one pack can supply the same object. Include each possible source.
        if self.indices.as_ref().and_then(|indices| {
            indices.iter().try_for_each(|index| index.observe(id.as_ref(), &self.watched, &mut stamp))
        }).is_none() {
            stamp.update(self.packs.clone().finalize());
        }
        for root in &self.roots {
            stat(&mut stamp, &root.join(&oid[..2]).join(&oid[2..]))?;
        }
        Some(format!("readable:{}", crate::content_store::observe::hex(&stamp.finalize().into())))
    }
}

fn watch(watched: &mut BTreeMap<PathBuf, [u8; 32]>, combined: Option<&mut Sha256>, path: PathBuf) -> Option<()> {
    let metadata = metadata(&path)?;
    let mut digest = Sha256::new();
    hash_stat(&mut digest, metadata.as_ref());
    if let Some(combined) = combined {
        hash_stat(combined, metadata.as_ref());
    }
    watched.insert(path, digest.finalize().into());
    Some(())
}

fn hash_path(digest: &mut Sha256, path: &Path) {
    let bytes = path.as_os_str().as_encoded_bytes();
    digest.update((bytes.len() as u64).to_le_bytes());
    digest.update(bytes);
}

fn stat(digest: &mut Sha256, path: &Path) -> Option<()> {
    hash_stat(digest, metadata(path)?.as_ref());
    Some(())
}

fn metadata(path: &Path) -> Option<Option<std::fs::Metadata>> {
    match std::fs::metadata(path) {
        Ok(metadata) => Some(Some(metadata)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Some(None),
        Err(_) => None,
    }
}

fn hash_stat(digest: &mut Sha256, metadata: Option<&std::fs::Metadata>) {
    let Some(metadata) = metadata else {
        digest.update([0]);
        return;
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
}
