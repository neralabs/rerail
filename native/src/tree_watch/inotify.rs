//! Linux: one inotify instance, one watch per folder a root's last read walked into.
//!
//! A watch reports a change to any entry of its own folder and nothing deeper, so a folder the
//! read left out (an ignored `node_modules`) costs no watch and reports nothing. A folder created
//! in a watched folder is reported as `renamed`; the host's next read passes the new folder set.
//! Watches are a budget every program of the user shares (`fs.inotify.max_user_watches`), so the
//! engine takes at most a quarter of it, and at most 131,072: a root past that is not watched.
//! Neither is a root on a file system inotify cannot see others' writes on (NFS, SMB, FUSE, 9p).
//! The kernel's overflow (`IN_Q_OVERFLOW`) makes every root `lost`; the root folder's own delete,
//! move or unmount ends its watch.
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

/// A folder of a root one watch descriptor stands for.
#[derive(Clone, PartialEq, Eq)]
struct Owner {
    root: String,
    folder: String,
}

/// Every watch: who each descriptor stands for (one folder may be under two roots, a nested
/// worktree's), and each root's descriptor per folder.
#[derive(Default)]
struct Watches {
    owners: HashMap<i32, Vec<Owner>>,
    roots: HashMap<String, HashMap<String, i32>>,
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
            let owner = Owner {
                root: root.to_owned(),
                folder,
            };
            self.release(instance, descriptor, &owner);
        }
    }

    /// Forgets `descriptor`, which the kernel dropped (its folder went).
    fn forget(&mut self, descriptor: i32) {
        for owner in self.owners.remove(&descriptor).unwrap_or_default() {
            if let Some(folders) = self.roots.get_mut(&owner.root) {
                folders.remove(&owner.folder);
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

    fn entry(&self, owner: &Owner, mask: u32, name: &str) {
        let path = join_relative(&owner.folder, name);
        if mask & NAMED != 0 {
            self.notices.renamed(&owner.root, path.clone());
        }
        if mask & CONTENT != 0 {
            self.notices.changed(&owner.root, path);
        }
    }

    /// A watched folder's own event: the root's going ends the watch; a folder below goes as an
    /// entry of its parent's watch, so only its unmount is told, as lost.
    fn folder(&self, watches: &mut Watches, owner: &Owner, mask: u32) {
        if mask & SELF_GONE == 0 {
            return;
        }
        if owner.folder.is_empty() {
            self.notices.ended(&owner.root);
            watches.release_root(self.instance.as_raw_fd(), &owner.root);
        } else if mask & libc::IN_UNMOUNT != 0 {
            self.notices.lost(&owner.root);
        }
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

    /// Adds the watch of `folder` under `root`.
    fn add(&self, watches: &mut Watches, root: &str, folder: &str) -> Result<(), Refused> {
        let path = c_path(&Path::new(root).join(folder)).ok_or(Refused::Other)?;
        // SAFETY: a NUL-terminated path on this backend's instance.
        let descriptor = unsafe {
            libc::inotify_add_watch(self.shared.instance.as_raw_fd(), path.as_ptr(), MASK)
        };
        if descriptor < 0 {
            return Err(match io::Error::last_os_error().raw_os_error() {
                Some(libc::ENOENT | libc::ENOTDIR) => Refused::Gone,
                Some(libc::ENOSPC) => Refused::Limit,
                _ => Refused::Other,
            });
        }
        let owner = Owner {
            root: root.to_owned(),
            folder: folder.to_owned(),
        };
        watches.own(descriptor, owner);
        watches
            .roots
            .entry(root.to_owned())
            .or_default()
            .insert(folder.to_owned(), descriptor);
        Ok(())
    }

    /// Watches each of `wanted` under `root` not watched yet. False when the root itself or the
    /// limit refused.
    fn add_wanted(&self, watches: &mut Watches, root: &str, wanted: &HashSet<&str>) -> bool {
        for folder in wanted {
            let held = watches
                .roots
                .get(root)
                .is_some_and(|held| held.contains_key(*folder));
            if held {
                continue;
            }
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
        let held = watches.roots.get(root).cloned().unwrap_or_default();
        for (folder, descriptor) in held {
            if wanted.contains(folder.as_str()) {
                continue;
            }
            watches
                .roots
                .get_mut(root)
                .map(|folders| folders.remove(&folder));
            let owner = Owner {
                root: root.to_owned(),
                folder,
            };
            watches.release(self.shared.instance.as_raw_fd(), descriptor, &owner);
        }
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
        let Some(thread) = self.thread.take() else {
            return;
        };
        self.shared.stop.store(true, Ordering::Release);
        let one: u64 = 1;
        // SAFETY: eight bytes to this backend's eventfd.
        unsafe { libc::write(self.wake.as_raw_fd(), (&raw const one).cast(), 8) };
        let _ = thread.join();
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
    use super::{Inotify, local_filesystem, parse};
    use crate::tree_watch::Backend;
    use crate::tree_watch::notices::Notices;
    use crate::tree_watch::tests::{Scratch, wait_for};
    use std::sync::Arc;

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
