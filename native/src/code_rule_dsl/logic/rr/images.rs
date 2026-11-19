//! Where rule images live (decision 0264): shipped with the engine, and a store per engine build for the rest.
//!
//! - **Shipped.** A bundle of images (`Bundle`), built once for the engine's library and packs and placed beside the
//!   engine (`rerail-library.images`, or `RERAIL_SHIPPED_IMAGES`). It is mapped from its file, not read: a run
//!   touches the pages of the images it opens. Each image is 16-byte aligned in it, as `program_image::Image` keeps
//!   its words, so a reader that holds mapped memory can read one in place.
//! - **Per build.** An image the bundle does not hold (a library file or a pack edited since it was built) is made
//!   and kept in a store every project of the engine's build shares (`store`): a second project, or the next run,
//!   reads it there. The bundle is its seed: a lookup reads the bundle first, so the store holds only what differs.
//!
//! An image's key names the engine's build, the image format and the bytes it was made from, so a bundle built by
//! another build, or for other files, is never read: its keys are not asked for.
//!
//! The bundle's layout: `RRIMAGES`, the version (u32) and the count (u32); then one entry per image, sorted by key:
//! the key's sha256 (32 bytes), the offset and the length (u64 each); then the images, each at a 16-byte boundary.

use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use crate::content_store::{ContentStore, HashKey, SourceId};

const MAGIC: &[u8; 8] = b"RRIMAGES";
const VERSION: u32 = 1;
const HEADER: usize = 16;
const ENTRY: usize = 48;

/// A bundle of images, mapped from its file (`open`).
pub struct Bundle {
    bytes: &'static [u8],
    count: usize,
}

impl Bundle {
    /// The bundle in `path`, checked: its header, and every entry inside the file. `None` when there is none, or it
    /// is not a bundle of this version.
    pub fn open(path: &Path) -> Option<Bundle> {
        let bytes = map(path)?;
        if bytes.len() < HEADER || &bytes[..8] != MAGIC || u32::from_le_bytes(bytes[8..12].try_into().ok()?) != VERSION {
            return None;
        }
        let count = u32::from_le_bytes(bytes[12..16].try_into().ok()?) as usize;
        let bundle = Bundle { bytes, count };
        if HEADER + count * ENTRY > bytes.len() {
            return None;
        }
        for k in 0..count {
            let (_, off, len) = bundle.entry(k);
            if off.checked_add(len).is_none_or(|end| end > bytes.len()) || off % 16 != 0 {
                return None;
            }
        }
        Some(bundle)
    }

    fn entry(&self, k: usize) -> (&[u8], usize, usize) {
        let e = &self.bytes[HEADER + k * ENTRY..HEADER + (k + 1) * ENTRY];
        let word = |at: usize| u64::from_le_bytes(e[at..at + 8].try_into().expect("eight bytes")) as usize;
        (&e[..32], word(32), word(40))
    }

    /// The image kept under `key`.
    pub fn get(&self, key: HashKey) -> Option<&'static [u8]> {
        let want = key.sha256_bytes()?;
        let (mut lo, mut hi) = (0, self.count);
        while lo < hi {
            let mid = (lo + hi) / 2;
            let (k, off, len) = self.entry(mid);
            match k.cmp(want.as_slice()) {
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
                std::cmp::Ordering::Equal => return Some(&self.bytes[off..off + len]),
            }
        }
        None
    }

    /// How many images it holds.
    pub fn len(&self) -> usize {
        self.count
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }
}

/// `path`'s bytes, mapped read-only for the process (never unmapped: the bundle is the engine's, as its code is).
#[cfg(unix)]
fn map(path: &Path) -> Option<&'static [u8]> {
    use std::os::unix::io::AsRawFd;
    let file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len() as usize;
    if len == 0 {
        return None;
    }
    // SAFETY: a read-only, private mapping of a file this process opened; it is never unmapped, and the slice made of
    // it is only read. A bundle replaced while mapped keeps its old pages (the replace makes a new file).
    let at = unsafe { libc::mmap(std::ptr::null_mut(), len, libc::PROT_READ, libc::MAP_PRIVATE, file.as_raw_fd(), 0) };
    if at == libc::MAP_FAILED {
        return None;
    }
    // SAFETY: `at` maps `len` bytes, for the rest of the process.
    Some(unsafe { std::slice::from_raw_parts(at.cast::<u8>(), len) })
}

/// Where no mapping is, the file read once for the process.
#[cfg(not(unix))]
fn map(path: &Path) -> Option<&'static [u8]> {
    Some(Box::leak(std::fs::read(path).ok()?.into_boxed_slice()))
}

/// The bundle shipped with the engine: `RERAIL_SHIPPED_IMAGES` (a path, or `off`), else `rerail-library.images`
/// beside the engine's executable.
pub fn shipped() -> Option<&'static Bundle> {
    static SHIPPED: OnceLock<Option<Bundle>> = OnceLock::new();
    SHIPPED
        .get_or_init(|| {
            let path = match std::env::var("RERAIL_SHIPPED_IMAGES") {
                Ok(v) if v == "off" => return None,
                Ok(v) => PathBuf::from(v),
                Err(_) => std::env::current_exe().ok()?.parent()?.join("rerail-library.images"),
            };
            let bundle = Bundle::open(&path)?;
            crate::code_rule_dsl::logic::profile::decision("shipped images (decision 0264)", || format!("{} images, from {}", bundle.len(), path.display()));
            Some(bundle)
        })
        .as_ref()
}

/// The store the run's rule images are kept in (decision 0264): the engine build's, shared by every project
/// (`RERAIL_IMAGE_STORE`: a path, or `project` for the run's own store); a test's own store under test.
pub fn store() -> Option<Arc<ContentStore>> {
    let project = || crate::code_rule_dsl::logic::plan::corpus::phases::reading_store();
    if cfg!(any(test, feature = "test-hooks")) {
        return project();
    }
    let path = match std::env::var("RERAIL_IMAGE_STORE") {
        Ok(v) if v == "project" => return project(),
        Ok(v) => PathBuf::from(v),
        Err(_) => crate::code_rule_dsl::logic::plan::corpus::build_store("rerail-images", None),
    };
    ContentStore::open_images(&path).ok().map(Arc::new)
}

/// The image kept under `key`: the shipped bundle's, else `store`'s, with its source there (for its lease).
pub fn read(store: Option<&ContentStore>, key: HashKey) -> Option<(Cow<'static, [u8]>, Option<SourceId>)> {
    if let Some(bytes) = shipped().and_then(|b| b.get(key)) {
        crate::code_rule_dsl::prof::count("rr.image_shipped", 1);
        return Some((Cow::Borrowed(bytes), None));
    }
    let store = store?;
    let id = store.source_by_alias(key).ok()??;
    let bytes = store.get_source(id).ok()?;
    Some((Cow::Owned(bytes), Some(id)))
}

/// An image `read` found: its bytes, and the store's source for it (`None` for one shipped with the engine).
pub type Found = (Cow<'static, [u8]>, Option<SourceId>);

/// `read` of many keys, in their order: the shipped ones from the bundle, the rest from the store in a few
/// statements (their aliases, then their bytes), not three statements each from as many threads.
pub fn read_all(store: Option<&ContentStore>, keys: &[HashKey]) -> Vec<Option<Found>> {
    let mut out: Vec<Option<Found>> = keys.iter().map(|key| shipped().and_then(|b| b.get(*key)).map(|bytes| (Cow::Borrowed(bytes), None))).collect();
    let Some(store) = store else { return out };
    let asked: Vec<usize> = (0..keys.len()).filter(|&i| out[i].is_none()).collect();
    let Ok(ids) = store.sources_by_aliases(&asked.iter().map(|&i| keys[i]).collect::<Vec<_>>()) else { return out };
    let found: Vec<(usize, SourceId)> = asked.iter().zip(ids).filter_map(|(&i, id)| Some((i, id?))).collect();
    let Ok(mut bytes) = store.get_sources_bulk(&found.iter().map(|(_, id)| *id).collect::<Vec<_>>()) else { return out };
    for (i, id) in found {
        if let Some(b) = bytes.remove(&id) {
            out[i] = Some((Cow::Owned(b.to_vec()), Some(id)));
        }
    }
    crate::code_rule_dsl::prof::count("rr.image_shipped", out.iter().filter(|f| f.as_ref().is_some_and(|f| f.1.is_none())).count());
    out
}

/// With `RERAIL_EXPORT_IMAGES=<dir>`, `bytes` written there as `<key>.img` too: what a bundle is packed from
/// (`pack`).
pub fn export(key: HashKey, bytes: &[u8]) {
    let Some(dir) = std::env::var_os("RERAIL_EXPORT_IMAGES") else { return };
    let Some(digest) = key.sha256_bytes() else { return };
    let dir = PathBuf::from(dir);
    let _ = std::fs::create_dir_all(&dir);
    let name: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    let (tmp, path) = (dir.join(format!("{name}.tmp{}", std::process::id())), dir.join(format!("{name}.img")));
    if std::fs::write(&tmp, bytes).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}

/// The bundle of the images exported to `dir` (`export`), written to `out`: how many it holds.
pub fn pack(dir: &Path, out: &Path) -> std::io::Result<usize> {
    let mut images: Vec<([u8; 32], Vec<u8>)> = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        let Some(stem) = path.file_name().and_then(|n| n.to_str()).and_then(|n| n.strip_suffix(".img")) else { continue };
        if stem.len() != 64 {
            continue;
        }
        let mut key = [0u8; 32];
        for (i, b) in key.iter_mut().enumerate() {
            *b = u8::from_str_radix(&stem[2 * i..2 * i + 2], 16).map_err(|e| std::io::Error::other(e.to_string()))?;
        }
        images.push((key, std::fs::read(&path)?));
    }
    images.sort_by_key(|a| a.0);
    let mut head = Vec::with_capacity(HEADER + images.len() * ENTRY);
    head.extend_from_slice(MAGIC);
    head.extend_from_slice(&VERSION.to_le_bytes());
    head.extend_from_slice(&(images.len() as u32).to_le_bytes());
    let mut at = (HEADER + images.len() * ENTRY).next_multiple_of(16);
    let mut body = Vec::new();
    for (key, bytes) in &images {
        head.extend_from_slice(key);
        head.extend_from_slice(&(at as u64).to_le_bytes());
        head.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
        body.extend_from_slice(bytes);
        let end = (at + bytes.len()).next_multiple_of(16);
        body.resize(body.len() + (end - at - bytes.len()), 0);
        at = end;
    }
    head.resize((HEADER + images.len() * ENTRY).next_multiple_of(16), 0);
    let tmp = out.with_extension(format!("tmp{}", std::process::id()));
    std::fs::write(&tmp, [head, body].concat())?;
    std::fs::rename(&tmp, out)?;
    Ok(images.len())
}
