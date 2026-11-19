//! The on-disk representation of objects, independent of decoded Git facts.

use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{OnceLock, atomic::{AtomicU32, Ordering}};

pub struct Storage {
    roots: Vec<PathBuf>,
    loose: Sha256,
    packs: Sha256,
    indices: Option<PackIndices>,
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

/// Small indexes list their first bytes on open. Larger ones learn from unsuccessful lookups.
struct PackIndices {
    files: Vec<PackIndex>,
    large: Vec<LargeIndex>,
    by_prefix: BTreeMap<u8, Vec<usize>>,
}

struct LargeIndex {
    slot: usize,
    misses: AtomicU32,
    prefixes: OnceLock<Box<[u64]>>,
}

impl LargeIndex {
    fn probe(&self, index: &PackIndex, id: &gix::hash::oid, suppliers: &mut BTreeSet<(PathBuf, u64)>) -> Option<usize> {
        const MAX_BITS: u32 = 64 * 1024 * 8;
        let count = index.len();
        // A nearly full bitmap cannot eliminate enough lookups to pay for its construction.
        if count > MAX_BITS {
            index.locate(id, suppliers)?;
            return Some(1);
        }
        if self.prefixes.get().is_none() && self.misses.load(Ordering::Relaxed) < count.div_ceil(32) {
            if !index.locate(id, suppliers)? { self.misses.fetch_add(1, Ordering::Relaxed); }
            return Some(1);
        }
        let bits = (count.next_power_of_two() * 8).min(MAX_BITS);
        let bucket = |oid: &gix::hash::oid| {
            let prefix = u32::from_be_bytes(oid.as_bytes()[..4].try_into().expect("Git object ID"));
            (prefix >> (u32::BITS - bits.ilog2())) as usize
        };
        let words = self.prefixes.get_or_init(|| {
            let mut words = vec![0u64; bits as usize / 64];
            for position in 0..count {
                let at = bucket(index.oid(position));
                words[at / 64] |= 1u64 << (at % 64);
            }
            rerail_prof::count("git.pack_routing_ids", count as usize);
            rerail_prof::count("git.pack_routing_bytes", words.len() * 8);
            words.into_boxed_slice()
        });
        let at = bucket(id);
        if words[at / 64] & (1u64 << (at % 64)) == 0 { return Some(0); }
        index.locate(id, suppliers)?;
        Some(1)
    }
}

impl PackIndices {
    fn new(files: Vec<PackIndex>) -> Self {
        let mut large = Vec::new();
        let mut by_prefix = BTreeMap::<u8, Vec<usize>>::new();
        for (slot, file) in files.iter().enumerate() {
            let count = file.len();
            if count > 256 {
                large.push(LargeIndex { slot, misses: AtomicU32::new(0), prefixes: OnceLock::new() });
                continue;
            }
            // Git stores object IDs in sorted order in both index formats.
            let mut previous = None;
            for position in 0..count {
                let id = file.oid(position);
                let prefix = id.as_bytes()[0];
                if previous != Some(prefix) {
                    by_prefix.entry(prefix).or_default().push(slot);
                    previous = Some(prefix);
                }
            }
        }
        Self { files, large, by_prefix }
    }

    fn suppliers(&self, id: gix::ObjectId) -> Option<BTreeSet<(PathBuf, u64)>> {
        let small = self.by_prefix.get(&id.as_slice()[0]).map_or(&[][..], Vec::as_slice);
        let mut suppliers = BTreeSet::new();
        let mut probes = small.len();
        for route in &self.large {
            probes += route.probe(&self.files[route.slot], id.as_ref(), &mut suppliers)?;
        }
        for &slot in small {
            self.files[slot].locate(id.as_ref(), &mut suppliers)?;
        }
        rerail_prof::count("git.pack_index_probes", probes);
        Some(suppliers)
    }
}

impl PackIndex {
    fn len(&self) -> u32 {
        match self { Self::Single(index) => index.num_objects(), Self::Multi(index) => index.num_objects() }
    }

    fn oid(&self, position: u32) -> &gix::hash::oid {
        match self { Self::Single(index) => index.oid_at_index(position), Self::Multi(index) => index.oid_at_index(position) }
    }

    fn locate(&self, id: &gix::hash::oid, suppliers: &mut BTreeSet<(PathBuf, u64)>) -> Option<bool> {
        let location = match self {
            Self::Single(index) => {
                let Some(entry) = index.lookup(id) else { return Some(false) };
                (index.path().with_extension("pack"), index.pack_offset_at_index(entry))
            }
            Self::Multi(index) => {
                let Some(entry) = index.lookup(id) else { return Some(false) };
                let (pack, offset) = index.pack_id_and_pack_offset_at_index(entry).ok()?;
                let name = index.index_names().get(pack as usize)?;
                (index.path().parent()?.join(name).with_extension("pack"), offset)
            }
        };
        suppliers.insert(location);
        Some(true)
    }
}

impl Storage {
    pub fn of(repo: &gix::Repository) -> Option<Self> {
        let store = repo.objects.store_ref();
        let mut roots = vec![store.path().to_owned()];
        roots.extend(store.alternate_db_paths().ok()?);
        let mut loose = Sha256::new();
        let mut packs = Sha256::new();
        // Earlier observations can contain entries sorted before their Git order was checked.
        loose.update(b"git-object-observation-v3");
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
        let storage = Self { roots, loose, packs, indices: indices.map(PackIndices::new), watched };
        storage.is_current().then_some(storage)
    }

    pub fn is_current(&self) -> bool {
        rerail_prof::count("git.pack_validations", 1);
        rerail_prof::count("git.pack_paths_checked", self.watched.len());
        self.watched.iter().all(|(path, expected)| {
            let mut digest = Sha256::new();
            stat(&mut digest, path).is_some() && &<[u8; 32]>::from(digest.finalize()) == expected
        })
    }

    pub fn object(&self, id: gix::ObjectId) -> Option<String> {
        rerail_prof::count("git.storage_stamp_requests", 1);
        let oid = id.to_string();
        let mut stamp = self.loose.clone();
        if self.indices.as_ref().and_then(|indices| indices.suppliers(id)).and_then(|suppliers| {
            for (pack, offset) in suppliers {
                hash_path(&mut stamp, &pack);
                stamp.update(offset.to_le_bytes());
                stamp.update(self.watched.get(&pack)?);
            }
            Some(())
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
    hash_stat(&mut digest, metadata.as_ref())?;
    if let Some(combined) = combined {
        hash_stat(combined, metadata.as_ref())?;
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
    hash_stat(digest, metadata(path)?.as_ref())
}

#[cfg(not(windows))]
use std::fs::{Metadata as StorageMetadata, metadata as read_metadata};

#[cfg(windows)]
#[path = "git_snapshot_windows_stat.rs"]
mod windows;
#[cfg(windows)]
use windows::{StorageMetadata, read_metadata};

fn metadata(path: &Path) -> Option<Option<StorageMetadata>> {
    match read_metadata(path) {
        Ok(metadata) => Some(Some(metadata)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Some(None),
        Err(_) => None,
    }
}

fn hash_stat(digest: &mut Sha256, metadata: Option<&StorageMetadata>) -> Option<()> {
    let Some(metadata) = metadata else {
        digest.update([0]);
        return Some(());
    };
    digest.update([1]);
    #[cfg(not(windows))]
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
    #[cfg(windows)]
    metadata.hash(digest);
    #[cfg(not(any(unix, windows)))]
    {
        let time = |timestamp: std::io::Result<std::time::SystemTime>| {
            timestamp.ok()?.duration_since(std::time::UNIX_EPOCH).ok().map(|time| time.as_nanos().to_le_bytes())
        };
        digest.update(time(metadata.modified())?);
        digest.update(time(metadata.created())?);
    }
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::process::{Command, Stdio};

    fn pack(repo: &gix::Repository, ids: &[gix::ObjectId]) -> PathBuf {
        let base = repo.git_dir().join("objects/pack/pack");
        let mut git = Command::new("git").arg("pack-objects").arg(&base)
            .current_dir(repo.workdir().unwrap())
            .stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
        let mut input = git.stdin.take().unwrap();
        for id in ids { writeln!(input, "{id}").unwrap(); }
        drop(input);
        let output = git.wait_with_output().unwrap();
        assert!(output.status.success());
        let hash = std::str::from_utf8(&output.stdout).unwrap().trim();
        base.with_file_name(format!("pack-{hash}.pack"))
    }

    #[test]
    fn routes_to_every_supplier_and_observes_loose_changes() {
        let temp = tempfile::tempdir().unwrap();
        let repo = gix::init(temp.path().join("primary")).unwrap();
        let alternate = gix::init(temp.path().join("alternate")).unwrap();
        let id = repo.write_blob(b"target").unwrap().detach();
        assert_eq!(alternate.write_blob(b"target").unwrap().detach(), id);
        let small = pack(&repo, &[id]);
        let other = pack(&alternate, &[id]);
        let mut ids: Vec<_> = (0..256).map(|i| repo.write_blob(i.to_string()).unwrap().detach()).collect();
        ids.push(id);
        let large = pack(&repo, &ids);
        let alt_objects = alternate.git_dir().join("objects").canonicalize().unwrap();
        let objects = repo.git_dir().join("objects");
        std::fs::create_dir_all(objects.join("info")).unwrap();
        std::fs::write(objects.join("info/alternates"), format!("{}\n", alt_objects.display())).unwrap();
        let layout = || Storage::of(&gix::open(repo.git_dir()).unwrap()).unwrap();
        let first = layout();
        let indices = first.indices.as_ref().unwrap();
        let suppliers = indices.suppliers(id).unwrap();
        let paths: BTreeSet<_> = suppliers.iter().map(|(path, _)| path.canonicalize().unwrap()).collect();
        assert_eq!(paths, BTreeSet::from([small, large, other].map(|p| p.canonicalize().unwrap())));
        for _ in 0..10 { assert_eq!(indices.suppliers(id).unwrap(), suppliers); }
        assert!(indices.large.iter().all(|route| route.prefixes.get().is_none()));
        for _ in 0..10 { assert!(indices.suppliers(gix::ObjectId::null(repo.object_hash())).unwrap().is_empty()); }
        assert!(indices.large.iter().any(|route| route.prefixes.get().is_some()));
        for oid in ids { assert!(!indices.suppliers(oid).unwrap().is_empty()); }
        let stamp = first.object(id).unwrap();
        assert!(Command::new("git").args(["multi-pack-index", "write"]).current_dir(repo.workdir().unwrap())
            .status().unwrap().success());
        assert!(!first.is_current());
        let next = layout();
        assert_eq!(next.indices.as_ref().unwrap().suppliers(id).unwrap(), suppliers);
        assert_eq!(next.object(id).unwrap(), stamp);
        let hex = id.to_string();
        std::fs::remove_file(objects.join(&hex[..2]).join(&hex[2..])).unwrap();
        assert!(next.is_current());
        assert_ne!(next.object(id).unwrap(), stamp);
    }
}
