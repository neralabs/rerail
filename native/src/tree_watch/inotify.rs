//! Linux: one inotify instance, one watch per folder a root's last read walked into, and one per
//! folder above a root.
//!
//! A watch reports a change to any entry of its own folder and nothing deeper, so a folder the
//! read left out (an ignored `node_modules`) costs no watch and reports nothing. A folder created
//! in a watched folder is reported as `renamed`; the host's next read passes the new folder set.
//! Watches are a budget every program of the user shares (`fs.inotify.max_user_watches`), so the
//! engine takes at most a quarter of it, and at most 131,072: a root past that is not watched. The
//! folders above the roots, a handful each and shared between them, are watched past the budget,
//! and count toward a later root's. A root on a file system inotify cannot see others' writes on
//! (NFS, SMB, FUSE, 9p) is not watched either.
//!
//! A watch follows its folder, not its path. The root's own delete, move or unmount ends its
//! watch, and so does that of any folder above it up to `/`: each is watched for its own going
//! only. A folder above that cannot be watched (unreadable, or past the kernel's limit) leaves the
//! root watched without it, and a rename of that folder goes unreported. A folder below the root
//! that is deleted or moved away is let go of with the folders below it. The kernel's overflow
//! (`IN_Q_OVERFLOW`) makes every root `lost`, and may have dropped those folders' own events as
//! well, so every `watch` adds each folder again: the kernel hands back the descriptor it holds
//! for the same folder, and another descriptor means another folder now sits at that path, which
//! the watch then stands for.
//!
//! One thread, `rerail-tree-watch`, reads the instance; the host's thread adds and removes
//! watches under the same lock that thread resolves them with.

use super::notices::Notices;
use super::{Backend, join_relative};
use crate::panic_boundary::contain_panic;
use std::collections::{HashMap, HashSet};
use std::ffi::{CString, OsStr};
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;

const MAX_WATCHES: usize = 131_072;
/// The budget when `max_user_watches` cannot be read: a quarter of old kernels' default.
const FALLBACK_WATCHES: usize = 8_192 / 4;

const NAMED: u32 = libc::IN_CREATE | libc::IN_DELETE | libc::IN_MOVED_FROM | libc::IN_MOVED_TO;
const CONTENT: u32 = libc::IN_MODIFY | libc::IN_ATTRIB;
const SELF_GONE: u32 = libc::IN_DELETE_SELF | libc::IN_MOVE_SELF | libc::IN_UNMOUNT;
/// A folder above a root: only its own going. Added to the watch's mask (`IN_MASK_ADD`), so a
/// watch it shares with a folder under another root keeps reporting that folder's entries.
const ABOVE: u32 = libc::IN_DELETE_SELF
    | libc::IN_MOVE_SELF
    | libc::IN_ONLYDIR
    | libc::IN_DONT_FOLLOW
    | libc::IN_MASK_ADD;
const MASK: u32 = NAMED
    | CONTENT
    | libc::IN_DELETE_SELF
    | libc::IN_MOVE_SELF
    | libc::IN_ONLYDIR
    | libc::IN_DONT_FOLLOW
    | libc::IN_EXCL_UNLINK;

/// File systems whose every write the local kernel sees (`statfs`'s `f_type`).
const LOCAL_FILESYSTEMS: [u32; 19] = [
    0xEF53,      // ext2, ext3, ext4
    0x5846_5342, // xfs
    0x9123_683E, // btrfs
    0x0102_1994, // tmpfs
    0x8584_58F6, // ramfs
    0xF2F5_2010, // f2fs
    0x2FC1_2FC1, // zfs
    0x794C_7630, // overlayfs
    0x5265_4973, // reiserfs
    0x3153_464A, // jfs
    0xCA45_1A4E, // bcachefs
    0x3434,      // nilfs2
    0x482B,      // hfsplus
    0x2011_BAB0, // exfat
    0x4D44,      // vfat
    0x5346_544E, // ntfs3
    0xF15F,      // ecryptfs
    0x7371_7368, // squashfs
    0x1501_3346, // udf
];

/// What one watch descriptor stands for to one root: a folder under it (`""` for the root), or,
/// with no folder, a folder above it.
#[derive(Clone, PartialEq, Eq)]
struct Owner {
    root: String,
    folder: Option<String>,
}

impl Owner {
    fn under(root: &str, folder: &str) -> Self {
        Self {
            root: root.to_owned(),
            folder: Some(folder.to_owned()),
        }
    }

    fn above(root: &str) -> Self {
        Self {
            root: root.to_owned(),
            folder: None,
        }
    }
}

/// Every watch: who each descriptor stands for (one folder may be under two roots, a nested
/// worktree's, or above one root and under another), each root's descriptor per folder, and the
/// descriptors of the folders above each root.
#[derive(Default)]
struct Watches {
    owners: HashMap<i32, Vec<Owner>>,
    roots: HashMap<String, HashMap<String, i32>>,
    above: HashMap<String, Vec<i32>>,
}

impl Watches {
    /// How many descriptors this backend would hold with `root` watching `folders`.
    fn count_with(&self, root: &str, folders: usize) -> usize {
        let held = self.roots.get(root).map_or(0, HashMap::len);
        (self.owners.len() + folders).saturating_sub(held)
    }

    fn own(&mut self, descriptor: i32, owner: Owner) {
        let owners = self.owners.entry(descriptor).or_default();
        if !owners.contains(&owner) {
            owners.push(owner);
        }
    }

    /// Lets `owner` go of `descriptor`, removing the watch once no one holds it.
    fn release(&mut self, instance: RawFd, descriptor: i32, owner: &Owner) {
        let Some(owners) = self.owners.get_mut(&descriptor) else {
            return;
        };
        owners.retain(|held| held != owner);
        if owners.is_empty() {
            self.owners.remove(&descriptor);
            // SAFETY: a descriptor of this instance; a watch the kernel already dropped fails harmlessly.
            unsafe { libc::inotify_rm_watch(instance, descriptor) };
        }
    }

    /// Releases every watch of `root`.
    fn release_root(&mut self, instance: RawFd, root: &str) {
        for (folder, descriptor) in self.roots.remove(root).unwrap_or_default() {
            self.release(instance, descriptor, &Owner::under(root, &folder));
        }
        self.release_above(instance, root, &[]);
    }

    /// Releases `root`'s watches of the folders above it, but those of `kept`.
    fn release_above(&mut self, instance: RawFd, root: &str, kept: &[i32]) {
        let owner = Owner::above(root);
        for descriptor in self.above.remove(root).unwrap_or_default() {
            if !kept.contains(&descriptor) {
                self.release(instance, descriptor, &owner);
            }
        }
    }

    /// Releases `root`'s watch of `folder`, when it holds one.
    fn release_folder(&mut self, instance: RawFd, root: &str, folder: &str) {
        let held = self
            .roots
            .get_mut(root)
            .and_then(|folders| folders.remove(folder));
        if let Some(descriptor) = held {
            self.release(instance, descriptor, &Owner::under(root, folder));
        }
    }

    /// Releases `root`'s watches of `folder` and of every folder below it.
    fn release_below(&mut self, instance: RawFd, root: &str, folder: &str) {
        let Some(folders) = self.roots.get_mut(root) else {
            return;
        };
        let below = |held: &String| {
            held.strip_prefix(folder)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
        };
        let gone: Vec<(String, i32)> = folders.extract_if(|held, _| below(held)).collect();
        for (folder, descriptor) in gone {
            self.release(instance, descriptor, &Owner::under(root, &folder));
        }
    }

    /// Forgets `descriptor`, which the kernel dropped (its folder went).
    fn forget(&mut self, descriptor: i32) {
        for owner in self.owners.remove(&descriptor).unwrap_or_default() {
            if let Some(folder) = owner.folder {
                if let Some(folders) = self.roots.get_mut(&owner.root) {
                    folders.remove(&folder);
                }
            } else if let Some(above) = self.above.get_mut(&owner.root) {
                above.retain(|held| *held != descriptor);
            }
        }
    }

    /// How many roots hold a watch.
    #[cfg(test)]
    fn root_count(&self) -> usize {
        self.roots
            .values()
            .filter(|folders| !folders.is_empty())
            .count()
    }
}

/// What the reading thread and the host's thread share.
struct Shared {
    instance: OwnedFd,
    notices: Arc<Notices>,
    watches: Mutex<Watches>,
    stop: AtomicBool,
    /// The reading thread failed: every watch ended and none is taken any more.
    failed: AtomicBool,
}

impl Shared {
    fn watches(&self) -> MutexGuard<'_, Watches> {
        self.watches.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Reports one event of `descriptor` (`name`: the entry of its folder, absent for the folder's
    /// own events).
    fn dispatch(&self, descriptor: i32, mask: u32, name: Option<&str>) {
        if mask & libc::IN_Q_OVERFLOW != 0 {
            self.notices.lost_all();
            return;
        }
        let mut watches = self.watches();
        let owners = watches.owners.get(&descriptor).cloned().unwrap_or_default();
        for owner in &owners {
            match name {
                Some(name) => self.entry(owner, mask, name),
                None => self.folder(&mut watches, owner, mask),
            }
        }
        if mask & libc::IN_IGNORED != 0 {
            watches.forget(descriptor);
        }
    }

    /// An entry's event, reported for a folder under a root; an entry of a folder above one is
    /// not the root's.
    fn entry(&self, owner: &Owner, mask: u32, name: &str) {
        let Some(folder) = &owner.folder else {
            return;
        };
        let path = join_relative(folder, name);
        if mask & NAMED != 0 {
            self.notices.renamed(&owner.root, path.clone());
        }
        if mask & CONTENT != 0 {
            self.notices.changed(&owner.root, path);
        }
    }

    /// A watched folder's own event: the going of the root, or of a folder above it, ends the
    /// watch. A folder below goes as an entry of its parent's watch; its own watch, and those
    /// below it, no longer stand for their paths and are let go of. Its unmount is also told, as
    /// lost.
    fn folder(&self, watches: &mut Watches, owner: &Owner, mask: u32) {
        if mask & SELF_GONE == 0 {
            return;
        }
        let instance = self.instance.as_raw_fd();
        let Some(folder) = owner.folder.as_deref().filter(|folder| !folder.is_empty()) else {
            self.notices.ended(&owner.root);
            watches.release_root(instance, &owner.root);
            return;
        };
        if mask & libc::IN_UNMOUNT != 0 {
            self.notices.lost(&owner.root);
        }
        watches.release_below(instance, &owner.root, folder);
    }
}

pub(super) struct Inotify {
    shared: Arc<Shared>,
    wake: OwnedFd,
    thread: Option<JoinHandle<()>>,
    budget: usize,
}

/// `fd` as owned, or the OS error a negative one stands for.
fn owned(fd: RawFd) -> io::Result<OwnedFd> {
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: a descriptor the caller just opened and owns.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

/// A quarter of the user's inotify watches, at most [`MAX_WATCHES`].
fn watch_budget() -> usize {
    std::fs::read_to_string("/proc/sys/fs/inotify/max_user_watches")
        .ok()
        .and_then(|text| text.trim().parse::<usize>().ok())
        .map_or(FALLBACK_WATCHES, |limit| (limit / 4).min(MAX_WATCHES))
}

fn c_path(path: &Path) -> Option<CString> {
    CString::new(path.as_os_str().as_bytes()).ok()
}

/// Whether `root` is on a file system inotify sees every write on.
fn local_filesystem(root: &str) -> bool {
    let Some(path) = c_path(Path::new(root)) else {
        return false;
    };
    let mut stat = std::mem::MaybeUninit::<libc::statfs>::uninit();
    // SAFETY: a NUL-terminated path and room for one `statfs`.
    if unsafe { libc::statfs(path.as_ptr(), stat.as_mut_ptr()) } != 0 {
        return false;
    }
    // SAFETY: `statfs` filled it.
    let kind = unsafe { stat.assume_init() }.f_type;
    // `f_type`'s width differs by target; every magic fits in 32 bits.
    #[allow(clippy::unnecessary_cast)]
    LOCAL_FILESYSTEMS.contains(&(kind as u32))
}

/// Why a folder could not be watched.
enum Refused {
    /// The folder is gone (or no longer a folder): the root changed since its read.
    Gone,
    /// The kernel's watch limit is reached.
    Limit,
    Other,
}

impl Inotify {
    pub(super) fn open(notices: Arc<Notices>) -> Option<Self> {
        Self::open_with_budget(notices, watch_budget())
    }

    pub(super) fn open_with_budget(notices: Arc<Notices>, budget: usize) -> Option<Self> {
        // SAFETY: plain syscalls; their descriptors are owned at once.
        let instance =
            owned(unsafe { libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC) }).ok()?;
        let wake =
            owned(unsafe { libc::eventfd(0, libc::EFD_NONBLOCK | libc::EFD_CLOEXEC) }).ok()?;
        let shared = Arc::new(Shared {
            instance,
            notices,
            watches: Mutex::new(Watches::default()),
            stop: AtomicBool::new(false),
            failed: AtomicBool::new(false),
        });
        let thread_shared = Arc::clone(&shared);
        let wake_fd = wake.as_raw_fd();
        let thread = std::thread::Builder::new()
            .name("rerail-tree-watch".to_owned())
            .spawn(move || run(&thread_shared, wake_fd))
            .ok()?;
        Some(Self {
            shared,
            wake,
            thread: Some(thread),
            budget,
        })
    }

    /// Stops the reading thread and waits for it; false when it was stopped already.
    fn stop_reading(&mut self) -> bool {
        let Some(thread) = self.thread.take() else {
            return false;
        };
        self.shared.stop.store(true, Ordering::Release);
        let one: u64 = 1;
        // SAFETY: eight bytes to this backend's eventfd.
        unsafe { libc::write(self.wake.as_raw_fd(), (&raw const one).cast(), 8) };
        let _ = thread.join();
        true
    }

    /// Watches `path` with `mask`: the descriptor, the one the instance already holds when it
    /// watches the same folder.
    fn add_watch(&self, path: &Path, mask: u32) -> Result<i32, Refused> {
        let path = c_path(path).ok_or(Refused::Other)?;
        // SAFETY: a NUL-terminated path on this backend's instance.
        let descriptor = unsafe {
            libc::inotify_add_watch(self.shared.instance.as_raw_fd(), path.as_ptr(), mask)
        };
        if descriptor < 0 {
            return Err(match io::Error::last_os_error().raw_os_error() {
                Some(libc::ENOENT | libc::ENOTDIR) => Refused::Gone,
                Some(libc::ENOSPC) => Refused::Limit,
                _ => Refused::Other,
            });
        }
        Ok(descriptor)
    }

    /// Watches `folder` under `root`, held or not. Another descriptor than the one held means
    /// another folder now sits at that path (the held one was deleted or moved away, and an
    /// overflow may have dropped its own events): the held watch is let go of, as it is when the
    /// path cannot be watched. Another root folder is lost: what was written in it before this
    /// watch went unseen, and the old root's own events, now no one's, end nothing.
    fn add(&self, watches: &mut Watches, root: &str, folder: &str) -> Result<(), Refused> {
        let added = self.add_watch(&Path::new(root).join(folder), MASK);
        let replaced = match added {
            Ok(descriptor) => {
                watches.own(descriptor, Owner::under(root, folder));
                let folders = watches.roots.entry(root.to_owned()).or_default();
                let held = folders.insert(folder.to_owned(), descriptor);
                held.filter(|held| *held != descriptor)
            }
            Err(_) => watches
                .roots
                .get_mut(root)
                .and_then(|folders| folders.remove(folder)),
        };
        if let Some(descriptor) = replaced {
            if folder.is_empty() && added.is_ok() {
                self.shared.notices.lost(root);
            }
            let instance = self.shared.instance.as_raw_fd();
            watches.release(instance, descriptor, &Owner::under(root, folder));
        }
        added.map(|_| ())
    }

    /// Watches each of `wanted` under `root`. False when the root itself or the limit refused.
    fn add_wanted(&self, watches: &mut Watches, root: &str, wanted: &HashSet<&str>) -> bool {
        for folder in wanted {
            if let Err(refused) = self.add(watches, root, folder)
                && !self.goes_on_without(root, folder, refused)
            {
                return false;
            }
        }
        true
    }

    /// Whether `root`'s watch goes on without `folder`, which was refused; a folder gone since the
    /// read is reported as renamed.
    fn goes_on_without(&self, root: &str, folder: &str, refused: Refused) -> bool {
        match refused {
            Refused::Limit => false,
            _ if folder.is_empty() => false,
            Refused::Gone => {
                self.shared.notices.renamed(root, folder.to_owned());
                true
            }
            Refused::Other => true,
        }
    }

    /// Releases the watches of `root`'s folders not in `wanted`.
    fn drop_unwanted(&self, watches: &mut Watches, root: &str, wanted: &HashSet<&str>) {
        let unwanted: Vec<String> = watches
            .roots
            .get(root)
            .into_iter()
            .flat_map(HashMap::keys)
            .filter(|folder| !wanted.contains(folder.as_str()))
            .cloned()
            .collect();
        for folder in unwanted {
            watches.release_folder(self.shared.instance.as_raw_fd(), root, &folder);
        }
    }

    /// Watches each folder above `root` up to `/` (which cannot go) for its own going, which ends
    /// the root; one watch stands for a folder above any number of roots. The root is watched
    /// without a folder that cannot be.
    fn watch_above(&self, watches: &mut Watches, root: &str) {
        let above: Vec<i32> = Path::new(root)
            .ancestors()
            .skip(1)
            .filter(|folder| folder.parent().is_some())
            .filter_map(|folder| self.add_watch(folder, ABOVE).ok())
            .collect();
        for descriptor in &above {
            watches.own(*descriptor, Owner::above(root));
        }
        watches.release_above(self.shared.instance.as_raw_fd(), root, &above);
        watches.above.insert(root.to_owned(), above);
    }
}

impl Backend for Inotify {
    fn watch(&mut self, root: &str, folders: &[String]) -> bool {
        if self.shared.failed.load(Ordering::Acquire) || !local_filesystem(root) {
            return false;
        }
        let mut wanted: HashSet<&str> = folders.iter().map(String::as_str).collect();
        wanted.insert("");
        let mut watches = self.shared.watches();
        let fits = watches.count_with(root, wanted.len()) <= self.budget;
        if fits && self.add_wanted(&mut watches, root, &wanted) {
            self.drop_unwanted(&mut watches, root, &wanted);
            self.watch_above(&mut watches, root);
            return true;
        }
        watches.release_root(self.shared.instance.as_raw_fd(), root);
        false
    }

    fn unwatch(&mut self, root: &str) {
        self.shared
            .watches()
            .release_root(self.shared.instance.as_raw_fd(), root);
    }

    fn close(&mut self) {
        if !self.stop_reading() {
            return;
        }
        *self.shared.watches() = Watches::default();
    }

    #[cfg(test)]
    fn roots(&self) -> usize {
        self.shared.watches().root_count()
    }
}

impl Drop for Inotify {
    fn drop(&mut self) {
        self.close();
    }
}

/// The reading thread: reads until stopped. A panic ends every watch and stops the reading.
fn run(shared: &Shared, wake: RawFd) {
    let read = contain_panic("tree watch", || {
        read_events(shared, wake);
        Ok(())
    });
    if read.is_err() {
        shared.failed.store(true, Ordering::Release);
        shared.notices.ended_all();
    }
}

fn read_events(shared: &Shared, wake: RawFd) {
    let mut buffer = vec![0_u8; 64 * 1024];
    while !shared.stop.load(Ordering::Acquire) {
        if wait_readable(shared.instance.as_raw_fd(), wake) {
            drain(shared, &mut buffer);
        }
    }
}

/// Waits until the instance or `wake` is readable; true when the instance is.
fn wait_readable(instance: RawFd, wake: RawFd) -> bool {
    let mut fds = [
        libc::pollfd {
            fd: instance,
            events: libc::POLLIN,
            revents: 0,
        },
        libc::pollfd {
            fd: wake,
            events: libc::POLLIN,
            revents: 0,
        },
    ];
    // SAFETY: two pollfds; no timeout, `close` wakes it.
    let ready = unsafe { libc::poll(fds.as_mut_ptr(), 2, -1) };
    ready > 0 && fds[0].revents & libc::POLLIN != 0
}

/// Reads and reports every queued event.
fn drain(shared: &Shared, buffer: &mut [u8]) {
    loop {
        // SAFETY: reads at most the buffer's length into it.
        let read = unsafe {
            libc::read(
                shared.instance.as_raw_fd(),
                buffer.as_mut_ptr().cast(),
                buffer.len(),
            )
        };
        let Ok(length) = usize::try_from(read) else {
            return;
        };
        if length == 0 {
            return;
        }
        parse(&buffer[..length], |descriptor, mask, name| {
            shared.dispatch(descriptor, mask, name)
        });
    }
}

/// Calls `event` for each `inotify_event` in `bytes`, with its name when it has one.
fn parse(bytes: &[u8], mut event: impl FnMut(i32, u32, Option<&str>)) {
    let header = std::mem::size_of::<libc::inotify_event>();
    let mut offset = 0;
    while offset + header <= bytes.len() {
        // SAFETY: a whole header lies at `offset`; it is read without assuming alignment.
        let raw = unsafe {
            std::ptr::read_unaligned(bytes.as_ptr().add(offset).cast::<libc::inotify_event>())
        };
        let start = offset + header;
        let end = (start + raw.len as usize).min(bytes.len());
        let name = name_of(&bytes[start..end]);
        event(raw.wd, raw.mask, name.as_deref());
        offset = end;
    }
}

/// An event's name: its bytes up to the first NUL, none when empty.
fn name_of(bytes: &[u8]) -> Option<String> {
    let length = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    (length > 0).then(|| {
        OsStr::from_bytes(&bytes[..length])
            .to_string_lossy()
            .into_owned()
    })
}

#[cfg(test)]
mod tests {
    use super::{Inotify, drain, local_filesystem, parse};
    use crate::tree_watch::notices::Notices;
    use crate::tree_watch::tests::{Scratch, wait_for};
    use crate::tree_watch::{Backend, TreeNotice};
    use std::os::fd::AsRawFd;
    use std::sync::Arc;

    /// An adapter whose reading thread is stopped: events wait in the kernel's queue until the
    /// test reports them with [`deliver`] or drops them with [`overflow`].
    fn paused(notices: &Arc<Notices>) -> Inotify {
        let mut backend = Inotify::open(Arc::clone(notices)).expect("inotify opens");
        assert!(backend.stop_reading());
        backend
    }

    /// Reports every queued event, as the reading thread does.
    fn deliver(backend: &Inotify) {
        drain(&backend.shared, &mut vec![0; 64 * 1024]);
    }

    /// Drops every queued event and reports the overflow, as the kernel does past its queue limit.
    fn overflow(backend: &Inotify) {
        let instance = backend.shared.instance.as_raw_fd();
        let mut buffer = [0_u8; 4096];
        // SAFETY: reads at most the buffer's length into it.
        while unsafe { libc::read(instance, buffer.as_mut_ptr().cast(), buffer.len()) } > 0 {}
        backend.shared.dispatch(-1, libc::IN_Q_OVERFLOW, None);
    }

    /// `root`'s notice among those taken.
    fn notice_of(notices: &Notices, root: &str) -> TreeNotice {
        notices
            .take()
            .into_iter()
            .find(|notice| notice.root == root)
            .expect("a notice of the root")
    }

    fn names(notice: &TreeNotice, path: &str) -> bool {
        notice.renamed.iter().any(|renamed| renamed == path)
    }

    #[test]
    fn an_overflow_loses_every_watched_root() {
        let notices = Arc::new(Notices::default());
        let mut backend = Inotify::open(Arc::clone(&notices)).expect("inotify opens");
        let (one, two) = (Scratch::new("overflow-one"), Scratch::new("overflow-two"));
        for root in [one.root(), two.root()] {
            notices.start(&root);
            assert!(backend.watch(&root, &[String::new()]));
        }
        backend.shared.dispatch(-1, libc::IN_Q_OVERFLOW, None);
        let taken = notices.take();
        assert_eq!(taken.len(), 2);
        assert!(taken.iter().all(|notice| notice.lost && !notice.ended));
        backend.close();
    }

    #[test]
    fn only_the_folders_given_are_watched_and_a_new_folder_is_renamed() {
        let notices = Arc::new(Notices::default());
        let mut backend = Inotify::open(Arc::clone(&notices)).expect("inotify opens");
        let scratch = Scratch::new("folders");
        scratch.write("src/a.ts", "a");
        scratch.write("node_modules/x/index.js", "x");
        let root = scratch.root();
        notices.start(&root);
        assert!(backend.watch(&root, &[String::new(), "src".to_owned()]));
        scratch.write("node_modules/x/index.js", "y");
        std::fs::create_dir(scratch.path("fresh")).expect("create folder");
        scratch.write("src/a.ts", "b");
        let notice = wait_for(&notices, &root, |notice| {
            notice.changed.iter().any(|path| path == "src/a.ts")
        });
        assert!(notice.renamed.contains(&"fresh".to_owned()));
        assert!(
            notice
                .changed
                .iter()
                .chain(&notice.renamed)
                .all(|path| !path.starts_with("node_modules/"))
        );
        backend.close();
    }

    #[test]
    fn a_folder_moved_away_or_deleted_is_watched_again_at_its_path() {
        let notices = Arc::new(Notices::default());
        let mut backend = paused(&notices);
        let scratch = Scratch::new("folder-gone");
        scratch.write("a/deep/x.ts", "x");
        scratch.write("c/x.ts", "x");
        let root = scratch.root();
        let folders = ["", "a", "a/deep", "c"].map(str::to_owned).to_vec();
        notices.start(&root);
        assert!(backend.watch(&root, &folders));

        // Moved away: the watches follow the folders to `b`, where they no longer stand for `a`.
        std::fs::rename(scratch.path("a"), scratch.path("b")).expect("move a away");
        std::fs::create_dir_all(scratch.path("a/deep")).expect("create a again");
        // Deleted, and created again.
        std::fs::remove_dir_all(scratch.path("c")).expect("delete c");
        std::fs::create_dir(scratch.path("c")).expect("create c again");

        // The host's next read walks into the same folders, now other folders at those paths,
        // before the old folders' own events are handled.
        assert!(backend.watch(&root, &folders));
        scratch.write("a/deep/y.ts", "y");
        scratch.write("c/y.ts", "y");
        scratch.write("b/deep/z.ts", "z");
        deliver(&backend);
        let notice = notice_of(&notices, &root);
        assert!(names(&notice, "a/deep/y.ts") && names(&notice, "c/y.ts"));
        assert!(
            !names(&notice, "a/deep/z.ts"),
            "the moved folder's old watch reports nothing"
        );
        backend.close();
    }

    #[test]
    fn a_folder_replaced_while_an_overflow_drops_its_events_is_watched_again() {
        let notices = Arc::new(Notices::default());
        let mut backend = paused(&notices);
        let scratch = Scratch::new("overflow-replaced");
        scratch.write("gen/deep/x.ts", "x");
        let root = scratch.root();
        let folders = ["", "gen", "gen/deep"].map(str::to_owned).to_vec();
        notices.start(&root);
        assert!(backend.watch(&root, &folders));

        // A branch switch deletes `gen` and creates it again; the kernel drops the old folders'
        // own events past its queue limit, and with them the watches.
        std::fs::remove_dir_all(scratch.path("gen")).expect("delete gen");
        std::fs::create_dir_all(scratch.path("gen/deep")).expect("create gen again");
        overflow(&backend);
        assert!(notice_of(&notices, &root).lost);

        // The host reads the lost root whole and watches the folder set it walked into again.
        assert!(backend.watch(&root, &folders));
        scratch.write("gen/y.ts", "y");
        scratch.write("gen/deep/y.ts", "y");
        deliver(&backend);
        let notice = notice_of(&notices, &root);
        assert!(names(&notice, "gen/y.ts") && names(&notice, "gen/deep/y.ts"));
        backend.close();
    }

    #[test]
    fn a_root_replaced_before_its_own_events_are_handled_is_lost() {
        let notices = Arc::new(Notices::default());
        let mut backend = paused(&notices);
        let scratch = Scratch::new("root-replaced");
        scratch.write("a.ts", "a");
        let root = scratch.root();
        notices.start(&root);
        assert!(backend.watch(&root, &[String::new()]));
        std::fs::remove_dir_all(&root).expect("delete the root");
        std::fs::create_dir(&root).expect("create it again");
        scratch.write("b.ts", "b");

        // The host watches again before the old root's own events are handled.
        assert!(backend.watch(&root, &[String::new()]));
        let notice = notice_of(&notices, &root);
        assert!(notice.lost && !notice.ended, "{notice:?}");

        // The old root's events, handled now, end nothing; the new root is watched.
        scratch.write("c.ts", "c");
        deliver(&backend);
        let notice = notice_of(&notices, &root);
        assert!(names(&notice, "c.ts") && !notice.ended, "{notice:?}");
        assert_eq!(backend.roots(), 1);
        backend.close();
    }

    #[test]
    fn renaming_a_folder_above_a_root_ends_it() {
        let notices = Arc::new(Notices::default());
        let mut backend = paused(&notices);
        let scratch = Scratch::new("above");
        scratch.write("parent/checkout/a.ts", "a");
        let outer = scratch.path("parent").to_string_lossy().into_owned();
        let inner = scratch
            .path("parent/checkout")
            .to_string_lossy()
            .into_owned();
        for root in [&outer, &inner] {
            notices.start(root);
            assert!(backend.watch(root, &[String::new()]));
        }
        // The outer root's watch of `parent` is the inner root's watch of a folder above it, too.
        backend.unwatch(&outer);
        scratch.write("parent/noise.txt", "n");
        std::fs::rename(scratch.path("parent"), scratch.path("moved")).expect("rename parent");
        deliver(&backend);
        let notice = notice_of(&notices, &inner);
        assert!(notice.ended);
        assert!(
            notice.renamed.is_empty(),
            "an entry of a folder above the root is not the root's"
        );
        assert!(
            backend.shared.watches().owners.is_empty(),
            "an ended root holds no watch"
        );
        backend.close();
    }

    #[test]
    fn a_root_past_the_watch_budget_is_not_watched() {
        let notices = Arc::new(Notices::default());
        let mut backend =
            Inotify::open_with_budget(Arc::clone(&notices), 2).expect("inotify opens");
        let scratch = Scratch::new("budget");
        for folder in ["a", "b", "c"] {
            std::fs::create_dir(scratch.path(folder)).expect("create folder");
        }
        let root = scratch.root();
        assert!(backend.watch(&root, &["a".to_owned()]));
        assert!(!backend.watch(&root, &["a".to_owned(), "b".to_owned(), "c".to_owned()]));
        assert!(
            backend.shared.watches().owners.is_empty(),
            "a refused root holds no watch"
        );
        backend.close();
    }

    #[test]
    fn a_local_file_system_is_watchable_and_proc_is_not() {
        assert!(local_filesystem(&std::env::temp_dir().to_string_lossy()));
        assert!(!local_filesystem("/proc"));
        assert!(!local_filesystem("/no/such/folder"));
    }

    #[test]
    fn events_are_parsed_with_and_without_names() {
        let header = std::mem::size_of::<libc::inotify_event>();
        let mut bytes = Vec::new();
        for (descriptor, mask, name) in [
            (3_i32, libc::IN_MODIFY, &b"a.ts\0\0\0\0"[..]),
            (4, libc::IN_DELETE_SELF, &b""[..]),
        ] {
            bytes.extend_from_slice(&descriptor.to_ne_bytes());
            bytes.extend_from_slice(&mask.to_ne_bytes());
            bytes.extend_from_slice(&0_u32.to_ne_bytes());
            bytes.extend_from_slice(&(name.len() as u32).to_ne_bytes());
            bytes.extend_from_slice(name);
        }
        assert_eq!(bytes.len(), 2 * header + 8);
        let mut seen = Vec::new();
        parse(&bytes, |descriptor, mask, name| {
            seen.push((descriptor, mask, name.map(str::to_owned)))
        });
        assert_eq!(
            seen,
            vec![
                (3, libc::IN_MODIFY, Some("a.ts".to_owned())),
                (4, libc::IN_DELETE_SELF, None)
            ]
        );
    }
}
