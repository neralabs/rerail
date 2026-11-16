//! macOS: one FSEvents stream per watched root, delivered on one serial dispatch queue.
//!
//! A stream watches the root's whole tree (`folders` is not used) with file-level events, no
//! defer, no latency and `WatchRoot`, so a rename, move or delete of the root or a folder above it
//! arrives (as `RootChanged`, or as an event of the root's own path) and ends the watch once the
//! folder it started on is no longer at the root's path. FSEvents' own drops (`MustScanSubDirs`,
//! `UserDropped`, `KernelDropped`) and mounts under the root make the root `lost`. FSEvents
//! reports real paths; a root is a physical path, so each event's path is the root's followed by
//! the entry's, its letters' case and composition aside (the volume may tell neither). An event
//! whose path does not read as under the root (a root spelled through a symlink or firmlink)
//! makes it `lost`. A root that is not there is not watched.
//!
//! The streams run on the queue `rerail-tree-watch`, which libdispatch serves from its own
//! threads: no thread here is the engine's to join. Every stream is stopped, invalidated and
//! released on that queue, so no callback runs during or after it, and `close` runs an empty block
//! on the queue after the last one before releasing the queue.

use super::Backend;
use super::notices::Notices;
use crate::panic_boundary::contain_panic;
use std::collections::HashMap;
use std::ffi::{CStr, c_char, c_void};
use std::os::unix::fs::MetadataExt;
use std::sync::Arc;

type CFIndex = isize;
type CFTypeRef = *const c_void;
type FSEventStreamRef = *mut c_void;
type DispatchQueue = *mut c_void;

/// `FSEventStreamContext`.
#[repr(C)]
struct StreamContext {
    version: CFIndex,
    info: *mut c_void,
    retain: Option<extern "C" fn(*const c_void) -> *const c_void>,
    release: Option<extern "C" fn(*const c_void)>,
    copy_description: Option<extern "C" fn(*const c_void) -> CFTypeRef>,
}

/// `CFArrayCallBacks`, only ever passed by address.
#[repr(C)]
struct CFArrayCallBacks {
    _fields: [usize; 5],
}

type StreamCallback = extern "C" fn(
    stream: *const c_void,
    info: *mut c_void,
    count: usize,
    paths: *mut c_void,
    flags: *const u32,
    ids: *const u64,
);

#[link(name = "CoreServices", kind = "framework")]
unsafe extern "C" {
    fn FSEventStreamCreate(
        allocator: CFTypeRef,
        callback: StreamCallback,
        context: *const StreamContext,
        paths: CFTypeRef,
        since_when: u64,
        latency: f64,
        flags: u32,
    ) -> FSEventStreamRef;
    fn FSEventStreamSetDispatchQueue(stream: FSEventStreamRef, queue: DispatchQueue);
    fn FSEventStreamStart(stream: FSEventStreamRef) -> u8;
    fn FSEventStreamStop(stream: FSEventStreamRef);
    fn FSEventStreamInvalidate(stream: FSEventStreamRef);
    fn FSEventStreamRelease(stream: FSEventStreamRef);
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFTypeArrayCallBacks: CFArrayCallBacks;
    fn CFStringCreateWithBytes(
        allocator: CFTypeRef,
        bytes: *const u8,
        length: CFIndex,
        encoding: u32,
        external: u8,
    ) -> CFTypeRef;
    fn CFArrayCreate(
        allocator: CFTypeRef,
        values: *const CFTypeRef,
        count: CFIndex,
        callbacks: *const CFArrayCallBacks,
    ) -> CFTypeRef;
    fn CFStringCompare(one: CFTypeRef, other: CFTypeRef, options: usize) -> CFIndex;
    fn CFRelease(object: CFTypeRef);
}

// libdispatch, part of libSystem.
unsafe extern "C" {
    fn dispatch_queue_create(label: *const c_char, attributes: *const c_void) -> DispatchQueue;
    fn dispatch_sync_f(
        queue: DispatchQueue,
        context: *mut c_void,
        work: extern "C" fn(*mut c_void),
    );
    fn dispatch_release(object: *mut c_void);
}

const UTF8: u32 = 0x0800_0100;
const SINCE_NOW: u64 = u64::MAX;
/// No wait before delivering: with `NoDefer` the first change after a quiet spell comes at once,
/// and what FSEvents holds by then comes together.
const LATENCY_SECONDS: f64 = 0.0;

const COMPARE_CASE_INSENSITIVE: usize = 0x01;
const COMPARE_NONLITERAL: usize = 0x10;

const CREATE_NO_DEFER: u32 = 0x02;
const CREATE_WATCH_ROOT: u32 = 0x04;
const CREATE_FILE_EVENTS: u32 = 0x10;

const MUST_SCAN_SUB_DIRS: u32 = 0x01;
const USER_DROPPED: u32 = 0x02;
const KERNEL_DROPPED: u32 = 0x04;
const ROOT_CHANGED: u32 = 0x20;
const MOUNT: u32 = 0x40;
const UNMOUNT: u32 = 0x80;
const ITEM_CREATED: u32 = 0x100;
const ITEM_REMOVED: u32 = 0x200;
const ITEM_INODE_META_MOD: u32 = 0x400;
const ITEM_RENAMED: u32 = 0x800;
const ITEM_MODIFIED: u32 = 0x1000;
const ITEM_FINDER_INFO_MOD: u32 = 0x2000;
const ITEM_CHANGE_OWNER: u32 = 0x4000;
const ITEM_XATTR_MOD: u32 = 0x8000;
const ITEM_CLONED: u32 = 0x40_0000;

const DROPPED: u32 = MUST_SCAN_SUB_DIRS | USER_DROPPED | KERNEL_DROPPED | MOUNT | UNMOUNT;
const NAMED: u32 = ITEM_CREATED | ITEM_REMOVED | ITEM_RENAMED;
const CONTENT: u32 = ITEM_MODIFIED
    | ITEM_INODE_META_MOD
    | ITEM_FINDER_INFO_MOD
    | ITEM_CHANGE_OWNER
    | ITEM_XATTR_MOD
    | ITEM_CLONED;

/// What one stream's callback reports to, owned by the stream (released with it).
struct Context {
    root: String,
    /// The root folder's device and inode when its stream started.
    identity: Option<(u64, u64)>,
    notices: Arc<Notices>,
}

/// The device and inode of the folder at `root`, or none.
fn identity(root: &str) -> Option<(u64, u64)> {
    std::fs::metadata(root)
        .ok()
        .map(|metadata| (metadata.dev(), metadata.ino()))
}

extern "C" fn release_context(info: *const c_void) {
    // SAFETY: `info` is the `Arc<Context>` pointer `start_stream` gave the stream, released once.
    unsafe { Arc::decrement_strong_count(info.cast::<Context>()) };
}

extern "C" fn on_events(
    _stream: *const c_void,
    info: *mut c_void,
    count: usize,
    paths: *mut c_void,
    flags: *const u32,
    _ids: *const u64,
) {
    // SAFETY: the stream holds its context until it is released, after its last callback.
    let context = unsafe { &*info.cast_const().cast::<Context>() };
    let reported = contain_panic("tree watch event", || {
        for index in 0..count {
            // SAFETY: without `UseCFTypes`, `paths` is `count` C strings and `flags` `count` flags.
            let (path, flag) = unsafe {
                let path = CStr::from_ptr(*paths.cast::<*const c_char>().add(index));
                (path.to_string_lossy(), *flags.add(index))
            };
            report(context, &path, flag);
        }
        Ok(())
    });
    if reported.is_err() {
        context.notices.ended(&context.root);
    }
}

/// Records one event of `context`'s root: `path` (absolute) and its FSEvents flags.
fn report(context: &Context, path: &str, flags: u32) {
    if flags & ROOT_CHANGED != 0 {
        root_moved(context);
        return;
    }
    if flags & DROPPED != 0 {
        context.notices.lost(&context.root);
    }
    match relative_to(&context.root, path) {
        // The root's own rename or removal comes as its own event; its other events are told by
        // the entries that changed.
        Some("") if flags & NAMED != 0 => root_moved(context),
        Some("") => {}
        Some(relative) => entry(context, relative, flags),
        // FSEvents reports only paths under the root: one that does not read as under it spells
        // the root another way (through a symlink or firmlink), so what changed cannot be told.
        None => context.notices.lost(&context.root),
    }
}

/// Records an event of `relative`, an entry under the root.
fn entry(context: &Context, relative: &str, flags: u32) {
    if flags & NAMED != 0 {
        context.notices.renamed(&context.root, relative.to_owned());
    }
    if flags & CONTENT != 0 {
        context.notices.changed(&context.root, relative.to_owned());
    }
}

/// The root, or the path to it, was renamed, moved or removed: the watch ends, unless the folder
/// the stream started on is still at the root's path. A stream hears of such a change made just
/// before it started (FSEvents delivers it late: a root watched again after its rename, or just
/// created), and FSEvents keeps a path's earlier flags on its later events, so these come while
/// nothing about the root changed. The check runs when the event arrives: only a root renamed
/// away and back before then goes unheard.
fn root_moved(context: &Context) {
    let same = context.identity.is_some() && identity(&context.root) == context.identity;
    if !same {
        context.notices.ended(&context.root);
    }
}

/// `path` below `root` (`""` for the root itself), comparing the root's letters without case and
/// however its accented letters are composed; none outside it.
fn relative_to<'path>(root: &str, path: &'path str) -> Option<&'path str> {
    let root = root.trim_end_matches('/');
    let head = path.as_bytes().get(..root.len());
    let length = if head.is_some_and(|head| head.eq_ignore_ascii_case(root.as_bytes())) {
        root.len()
    } else {
        // Another composition or case of the root's letters takes other bytes: compare as many of
        // the path's components as the root has.
        let components = root.matches('/').count();
        let length = path
            .match_indices('/')
            .nth(components)
            .map_or(path.len(), |(at, _)| at);
        if !same_spelling(root, &path[..length]) {
            return None;
        }
        length
    };
    let rest = path.get(length..)?;
    if rest.is_empty() {
        return Some("");
    }
    rest.strip_prefix('/')
}

/// Whether `one` and `other` spell the same path, their letters' case and composition (NFC as
/// typed, NFD as HFS+ stored it) aside.
fn same_spelling(one: &str, other: &str) -> bool {
    let (one, other) = (cf_string(one), cf_string(other));
    let same = match (one, other) {
        // SAFETY: two strings created above, released below.
        (Some(one), Some(other)) => unsafe {
            CFStringCompare(one, other, COMPARE_CASE_INSENSITIVE | COMPARE_NONLITERAL) == 0
        },
        _ => false,
    };
    for string in [one, other].into_iter().flatten() {
        // SAFETY: a string created above, released once.
        unsafe { CFRelease(string) };
    }
    same
}

/// A started stream, released by [`stop_stream`] on the queue.
struct Stream(FSEventStreamRef);

pub(super) struct FsEvents {
    notices: Arc<Notices>,
    queue: DispatchQueue,
    streams: HashMap<String, Stream>,
}

impl FsEvents {
    pub(super) fn open(notices: Arc<Notices>) -> Option<Self> {
        // SAFETY: a static NUL-terminated label; a null attribute makes a serial queue.
        let queue =
            unsafe { dispatch_queue_create(c"rerail-tree-watch".as_ptr(), std::ptr::null()) };
        (!queue.is_null()).then(|| Self {
            notices,
            queue,
            streams: HashMap::new(),
        })
    }

    /// Runs `work(context)` on the queue and waits for it.
    fn on_queue(&self, context: *mut c_void, work: extern "C" fn(*mut c_void)) {
        // SAFETY: the queue lives until `close`, which runs nothing on it after releasing it.
        unsafe { dispatch_sync_f(self.queue, context, work) };
    }
}

extern "C" fn stop_stream(stream: *mut c_void) {
    // SAFETY: a started stream this backend created, stopped and released once, on its queue.
    unsafe {
        FSEventStreamStop(stream);
        FSEventStreamInvalidate(stream);
        FSEventStreamRelease(stream);
    }
}

extern "C" fn nothing(_: *mut c_void) {}

/// A `CFString` of `text`, or none; the caller releases it.
fn cf_string(text: &str) -> Option<CFTypeRef> {
    let length = CFIndex::try_from(text.len()).ok()?;
    // SAFETY: `text`'s bytes for its length.
    let string =
        unsafe { CFStringCreateWithBytes(std::ptr::null(), text.as_ptr(), length, UTF8, 0) };
    (!string.is_null()).then_some(string)
}

/// A one-path `CFArray` of `root`, or none.
fn path_array(root: &str) -> Option<CFTypeRef> {
    let path = cf_string(root)?;
    // SAFETY: the array retains the string, which is released here.
    unsafe {
        let array = CFArrayCreate(std::ptr::null(), &path, 1, &raw const kCFTypeArrayCallBacks);
        CFRelease(path);
        (!array.is_null()).then_some(array)
    }
}

/// Creates and starts `root`'s stream on `queue`, or none (none for a root that is not there).
fn start_stream(root: &str, notices: &Arc<Notices>, queue: DispatchQueue) -> Option<Stream> {
    let folder = identity(root)?;
    let paths = path_array(root)?;
    let info = Arc::into_raw(Arc::new(Context {
        root: root.to_owned(),
        identity: Some(folder),
        notices: Arc::clone(notices),
    }));
    let context = StreamContext {
        version: 0,
        info: info.cast_mut().cast(),
        retain: None,
        release: Some(release_context),
        copy_description: None,
    };
    let flags = CREATE_NO_DEFER | CREATE_WATCH_ROOT | CREATE_FILE_EVENTS;
    // SAFETY: the stream takes the context's one reference (released through `release_context`);
    // `paths` is released here, the stream keeping its own copy.
    unsafe {
        let stream = FSEventStreamCreate(
            std::ptr::null(),
            on_events,
            &context,
            paths,
            SINCE_NOW,
            LATENCY_SECONDS,
            flags,
        );
        CFRelease(paths);
        if stream.is_null() {
            Arc::decrement_strong_count(info);
            return None;
        }
        FSEventStreamSetDispatchQueue(stream, queue);
        if FSEventStreamStart(stream) == 0 {
            FSEventStreamInvalidate(stream);
            FSEventStreamRelease(stream);
            return None;
        }
        Some(Stream(stream))
    }
}

impl Backend for FsEvents {
    fn watch(&mut self, root: &str, _folders: &[String]) -> bool {
        if self.queue.is_null() {
            return false;
        }
        if self.streams.contains_key(root) {
            return true;
        }
        let Some(stream) = start_stream(root, &self.notices, self.queue) else {
            return false;
        };
        self.streams.insert(root.to_owned(), stream);
        true
    }

    fn unwatch(&mut self, root: &str) {
        if let Some(stream) = self.streams.remove(root) {
            self.on_queue(stream.0, stop_stream);
        }
    }

    fn close(&mut self) {
        if self.queue.is_null() {
            return;
        }
        for (_, stream) in std::mem::take(&mut self.streams) {
            self.on_queue(stream.0, stop_stream);
        }
        self.on_queue(std::ptr::null_mut(), nothing);
        // SAFETY: the queue this backend created; nothing runs on it any more.
        unsafe { dispatch_release(self.queue) };
        self.queue = std::ptr::null_mut();
    }

    #[cfg(test)]
    fn roots(&self) -> usize {
        self.streams.len()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Context, DROPPED, ITEM_CREATED, ITEM_MODIFIED, ITEM_RENAMED, MUST_SCAN_SUB_DIRS,
        ROOT_CHANGED, identity, relative_to, report,
    };
    use crate::tree_watch::TreeWatcher;
    use crate::tree_watch::notices::Notices;
    use crate::tree_watch::tests::{Scratch, wait_for_watcher};
    use std::sync::Arc;

    #[test]
    fn an_event_path_is_relative_to_the_root_whatever_the_case_of_its_letters() {
        assert_eq!(
            relative_to("/Users/a/Repo", "/Users/a/Repo/src/x.ts"),
            Some("src/x.ts")
        );
        assert_eq!(
            relative_to("/Users/a/Repo", "/users/A/repo/src/x.ts"),
            Some("src/x.ts")
        );
        assert_eq!(relative_to("/Users/a/Repo/", "/Users/a/Repo/x"), Some("x"));
        assert_eq!(relative_to("/Users/a/Repo", "/Users/a/Repo"), Some(""));
        assert_eq!(relative_to("/Users/a/Repo", "/Users/a/Repository/x"), None);
        assert_eq!(relative_to("/Users/a/Repo", "/Users/a"), None);
        assert_eq!(
            relative_to("/Users/a/Repo", "/private/Users/a/Repo/x"),
            None
        );
    }

    #[test]
    fn an_event_path_is_relative_to_the_root_whatever_the_composition_of_its_letters() {
        // The root as typed (NFC), the path as HFS+ stored it (NFD), and a non-ASCII case.
        assert_eq!(
            relative_to("/Users/a/Caf\u{e9}", "/Users/a/Cafe\u{301}/src/x.ts"),
            Some("src/x.ts")
        );
        assert_eq!(
            relative_to("/Users/a/\u{c9}t\u{e9}", "/users/a/\u{e9}te\u{301}"),
            Some("")
        );
        assert_eq!(
            relative_to("/Users/a/Caf\u{e9}", "/Users/a/Cafe/src/x.ts"),
            None
        );
    }

    #[test]
    fn dropped_events_lose_the_root_and_a_root_change_ends_it() {
        let notices = Arc::new(Notices::default());
        let context = Context {
            root: "/r".to_owned(),
            identity: None,
            notices: Arc::clone(&notices),
        };
        notices.start("/r");
        report(&context, "/r/a", ITEM_CREATED | ITEM_MODIFIED);
        report(&context, "/r/b", ITEM_RENAMED);
        report(&context, "/r", ITEM_MODIFIED);
        let taken = notices.take();
        assert_eq!(taken.len(), 1);
        assert_eq!(taken[0].renamed, vec!["a", "b"]);
        assert_eq!(taken[0].changed, vec!["a"]);
        assert!(!taken[0].lost && !taken[0].ended);

        // A path that does not read as under the root: what changed there cannot be told.
        report(&context, "/elsewhere/c", ITEM_MODIFIED);
        let taken = notices.take();
        assert!(taken[0].lost && !taken[0].ended && taken[0].changed.is_empty());

        report(&context, "/r", MUST_SCAN_SUB_DIRS);
        let taken = notices.take();
        assert!(taken[0].lost && !taken[0].ended);
        assert_ne!(DROPPED & MUST_SCAN_SUB_DIRS, 0);

        report(&context, "/r", ROOT_CHANGED);
        let taken = notices.take();
        assert!(taken[0].ended);
        assert!(!notices.watching("/r"));
    }

    #[test]
    fn a_move_event_with_the_same_folder_at_the_root_keeps_watching() {
        let scratch = Scratch::new("root-same");
        let root = scratch.root();
        let notices = Arc::new(Notices::default());
        let context = Context {
            root: root.clone(),
            identity: identity(&root),
            notices: Arc::clone(&notices),
        };
        notices.start(&root);
        report(&context, &root, ROOT_CHANGED);
        report(&context, &root, ITEM_CREATED | ITEM_RENAMED);
        assert!(notices.take().is_empty());
        assert!(notices.watching(&root));

        let moved = scratch.path("").with_extension("moved");
        std::fs::rename(scratch.path(""), &moved).expect("rename");
        report(&context, &root, ITEM_RENAMED);
        assert!(notices.take()[0].ended);
        std::fs::rename(&moved, scratch.path("")).expect("rename back");
    }

    #[test]
    fn a_root_spelled_through_a_symlink_is_lost_when_something_under_it_changes() {
        let scratch = Scratch::new("symlinked");
        scratch.write("real/checkout/src/a.txt", "a");
        std::os::unix::fs::symlink(scratch.path("real"), scratch.path("link")).expect("symlink");
        // FSEvents reports the real path, `…/real/checkout/…`, of a root watched as `…/link/checkout`.
        let root = scratch.path("link/checkout").to_string_lossy().into_owned();
        let mut watcher = TreeWatcher::new();
        assert!(
            watcher
                .watch(root.clone(), vec![String::new()])
                .expect("watch")
        );
        scratch.write("real/checkout/src/a.txt", "written");
        let notice = wait_for_watcher(&mut watcher, &root, |notice| notice.lost);
        assert!(!notice.ended);
        watcher.close().expect("close");
    }
}
