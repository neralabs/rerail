//! What changed on disk since a cursor, from the journal the operating system keeps: the observer's way of knowing
//! which paths to look at again without a `stat` of every path it knows (decision 0105).
//!
//! On macOS that journal is FSEvents: `fseventsd` records every change to a volume with an event id, and replays the
//! changes under some directories since an id (`sinceWhen`). A process of the engine asks it once, as it starts
//! observing a root, for what changed since the id the last process stored, and is told only those paths. Nothing
//! of ours runs between processes: the journal is the system's.
//!
//! **No change missed.** An event reaches `fseventsd` some time after the write. So, as Watchman does, the replay
//! ends only once it has seen the event of a file it wrote itself (a cookie, beside the observer's store, on the
//! same volume as the root): every change made before the cookie has an earlier id, and was replayed before it.
//!
//! **No history.** Without a stored id, on another volume than the one the id was taken on, when the system dropped
//! events or renumbered them, or when the cookie's event does not arrive in time, the answer is `Lost`: every path
//! may have changed, and the observer checks each by its `stat`, as it does without a journal. On other systems
//! there is no journal yet: every answer is `Lost`.

use std::path::{Path, PathBuf};

/// What changed under the roots since a cursor.
#[derive(Debug)]
pub enum Since {
    /// The paths that changed (each a file or a directory, as the root's caller names it), and the cursor to ask
    /// from next time.
    Changed { paths: Vec<PathBuf>, cursor: u64 },
    /// The journal cannot say: everything may have changed. Next time, ask from `cursor`.
    Lost { cursor: Option<u64>, why: &'static str },
}

/// A volume's identity, as the journal's ids are of it (an id is meaningless on another volume).
pub fn volume(path: &Path) -> Option<[u8; 16]> {
    imp::volume(path)
}

/// What changed under `roots` (each a directory, by the path the caller keeps its records under) since `cursor`
/// (`None`: no earlier cursor), the journal synchronised by a cookie written in `cookie_dir`.
pub fn since(roots: &[PathBuf], cursor: Option<u64>, cookie_dir: &Path) -> Since {
    imp::since(roots, cursor, cookie_dir)
}

#[cfg(not(target_os = "macos"))]
mod imp {
    use super::Since;
    use std::path::{Path, PathBuf};

    pub(super) fn volume(_: &Path) -> Option<[u8; 16]> {
        None
    }

    pub(super) fn since(_: &[PathBuf], _: Option<u64>, _: &Path) -> Since {
        Since::Lost { cursor: None, why: "no journal on this system" }
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use super::Since;
    use std::ffi::{CStr, CString, c_char, c_void};
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Condvar, Mutex};
    use std::time::{Duration, Instant};

    type CFIndex = isize;
    type CFRef = *const c_void;
    type Stream = *mut c_void;
    type Callback = extern "C" fn(Stream, *mut c_void, usize, *mut c_void, *const u32, *const u64);

    #[repr(C)]
    struct Context {
        version: CFIndex,
        info: *mut c_void,
        retain: *const c_void,
        release: *const c_void,
        copy_description: *const c_void,
    }

    #[repr(C)]
    struct UuidBytes {
        bytes: [u8; 16],
    }

    #[repr(C)]
    struct ArrayCallBacks {
        version: CFIndex,
        retain: *const c_void,
        release: *const c_void,
        copy_description: *const c_void,
        equal: *const c_void,
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        static kCFTypeArrayCallBacks: ArrayCallBacks;
        fn CFStringCreateWithCString(alloc: CFRef, s: *const c_char, encoding: u32) -> CFRef;
        fn CFArrayCreate(alloc: CFRef, values: *const CFRef, n: CFIndex, callbacks: *const ArrayCallBacks) -> CFRef;
        fn CFRelease(cf: CFRef);
        fn CFUUIDGetUUIDBytes(uuid: CFRef) -> UuidBytes;
    }

    #[link(name = "CoreServices", kind = "framework")]
    unsafe extern "C" {
        fn FSEventStreamCreate(alloc: CFRef, callback: Callback, context: *mut Context, paths: CFRef, since: u64, latency: f64, flags: u32) -> Stream;
        fn FSEventStreamSetDispatchQueue(stream: Stream, queue: *mut c_void);
        fn FSEventStreamStart(stream: Stream) -> u8;
        fn FSEventStreamStop(stream: Stream);
        fn FSEventStreamInvalidate(stream: Stream);
        fn FSEventStreamRelease(stream: Stream);
        fn FSEventsGetCurrentEventId() -> u64;
        fn FSEventsCopyUUIDForDevice(dev: i32) -> CFRef;
    }

    unsafe extern "C" {
        fn dispatch_queue_create(label: *const c_char, attr: *const c_void) -> *mut c_void;
        fn dispatch_release(object: *mut c_void);
    }

    const UTF8: u32 = 0x0800_0100;
    const NO_DEFER: u32 = 0x2;
    const FILE_EVENTS: u32 = 0x10;
    // Events that say the journal lost track: rescan, dropped, ids wrapped, the root moved, a volume came or went.
    const MUST_SCAN: u32 = 0x1;
    const LOST: u32 = 0x2 | 0x4 | 0x8 | 0x20 | 0x40 | 0x80;
    const HISTORY_DONE: u32 = 0x10;
    /// How long a replay may wait for its cookie.
    const WAIT: Duration = Duration::from_secs(2);

    #[derive(Default)]
    struct Seen {
        paths: Vec<(String, u32)>,
        history_done: bool,
        cookie: bool,
        lost: Option<&'static str>,
    }

    struct Shared {
        seen: Mutex<Seen>,
        ready: Condvar,
        cookie: String,
    }

    extern "C" fn callback(_: Stream, info: *mut c_void, n: usize, paths: *mut c_void, flags: *const u32, _ids: *const u64) {
        // SAFETY: `info` is the `Shared` that `since` keeps alive until the stream is invalidated, and the arrays
        // hold `n` entries for the callback's duration (FSEvents' contract, without `UseCFTypes`).
        let shared = unsafe { &*(info as *const Shared) };
        let paths = paths as *const *const c_char;
        let mut seen = shared.seen.lock().unwrap_or_else(|e| e.into_inner());
        for i in 0..n {
            let (path, flag) = unsafe { (CStr::from_ptr(*paths.add(i)).to_string_lossy().into_owned(), *flags.add(i)) };
            if flag & LOST != 0 {
                seen.lost = Some("the system dropped or renumbered events");
            }
            if flag & HISTORY_DONE != 0 {
                seen.history_done = true;
                continue;
            }
            if path == shared.cookie {
                seen.cookie = true;
                continue;
            }
            seen.paths.push((path, flag));
        }
        shared.ready.notify_all();
    }

    pub(super) fn volume(path: &Path) -> Option<[u8; 16]> {
        use std::os::unix::fs::MetadataExt as _;
        let dev = std::fs::metadata(path).ok()?.dev() as i32;
        // SAFETY: a CFUUID is returned owned, or null.
        unsafe {
            let uuid = FSEventsCopyUUIDForDevice(dev);
            if uuid.is_null() {
                return None;
            }
            let bytes = CFUUIDGetUUIDBytes(uuid).bytes;
            CFRelease(uuid);
            Some(bytes)
        }
    }

    pub(super) fn since(roots: &[PathBuf], cursor: Option<u64>, cookie_dir: &Path) -> Since {
        // The id now, before anything is asked: a change from here on is the next replay's.
        // SAFETY: no arguments; returns the latest id.
        let now = unsafe { FSEventsGetCurrentEventId() };
        let Some(cursor) = cursor else { return Since::Lost { cursor: Some(now), why: "no earlier cursor" } };
        if cursor > now {
            return Since::Lost { cursor: Some(now), why: "a cursor ahead of the journal" };
        }
        // Records are kept by the path the caller gave; events come by the real path.
        let Ok(cookie_dir) = std::fs::canonicalize(cookie_dir) else { return Since::Lost { cursor: Some(now), why: "no cookie directory" } };
        let real: Vec<(PathBuf, PathBuf)> = roots.iter().filter_map(|r| Some((std::fs::canonicalize(r).ok()?, r.clone()))).collect();
        if real.len() != roots.len() {
            return Since::Lost { cursor: Some(now), why: "a root is not readable" };
        }
        let cookie = cookie_dir.join(format!("rerail-cookie-{}-{}", std::process::id(), now));
        let shared = Arc::new(Shared { seen: Mutex::new(Seen::default()), ready: Condvar::new(), cookie: cookie.to_string_lossy().into_owned() });
        let mut watched: Vec<CString> = real.iter().map(|(r, _)| CString::new(r.to_string_lossy().as_bytes()).unwrap_or_default()).collect();
        watched.push(CString::new(cookie_dir.to_string_lossy().as_bytes()).unwrap_or_default());
        // SAFETY: the strings and array are released below; the stream holds its own references while it lives.
        let (stream, queue) = unsafe {
            let strings: Vec<CFRef> = watched.iter().map(|s| CFStringCreateWithCString(std::ptr::null(), s.as_ptr(), UTF8)).collect();
            let array = CFArrayCreate(std::ptr::null(), strings.as_ptr(), strings.len() as CFIndex, &kCFTypeArrayCallBacks);
            for s in strings {
                CFRelease(s);
            }
            let mut context = Context { version: 0, info: Arc::as_ptr(&shared) as *mut c_void, retain: std::ptr::null(), release: std::ptr::null(), copy_description: std::ptr::null() };
            let stream = FSEventStreamCreate(std::ptr::null(), callback, &mut context, array, cursor, 0.0, NO_DEFER | FILE_EVENTS);
            CFRelease(array);
            if stream.is_null() {
                return Since::Lost { cursor: Some(now), why: "the journal refused the stream" };
            }
            let queue = dispatch_queue_create(c"rerail.journal".as_ptr(), std::ptr::null());
            FSEventStreamSetDispatchQueue(stream, queue);
            if FSEventStreamStart(stream) == 0 {
                FSEventStreamInvalidate(stream);
                FSEventStreamRelease(stream);
                dispatch_release(queue);
                return Since::Lost { cursor: Some(now), why: "the journal did not start" };
            }
            (stream, queue)
        };
        // The cookie: its event comes after every change made before it.
        let wrote = std::fs::write(&cookie, b"").is_ok();
        let deadline = Instant::now() + WAIT;
        let mut seen = shared.seen.lock().unwrap_or_else(|e| e.into_inner());
        while wrote && !(seen.cookie && seen.history_done) && seen.lost.is_none() {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            seen = shared.ready.wait_timeout(seen, left).unwrap_or_else(|e| e.into_inner()).0;
        }
        let done = seen.cookie && seen.history_done;
        let lost = seen.lost;
        let paths = std::mem::take(&mut seen.paths);
        drop(seen);
        // SAFETY: stopped and invalidated before release; the callback no longer runs once `FSEventStreamInvalidate`
        // returns, so `shared` may go after.
        unsafe {
            FSEventStreamStop(stream);
            FSEventStreamInvalidate(stream);
            FSEventStreamRelease(stream);
            dispatch_release(queue);
        }
        let _ = std::fs::remove_file(&cookie);
        if let Some(why) = lost {
            return Since::Lost { cursor: Some(now), why };
        }
        if !done {
            return Since::Lost { cursor: Some(now), why: if wrote { "the cookie's event did not arrive" } else { "no cookie could be written" } };
        }
        // A directory to scan again is changed as a whole: the caller drops what it knows under it.
        let mut out: Vec<PathBuf> = Vec::new();
        for (path, flag) in paths {
            let path = PathBuf::from(path.trim_end_matches('/'));
            let Some((real_root, root)) = real.iter().find(|(r, _)| path.starts_with(r)) else { continue };
            let under = path.strip_prefix(real_root).map(|rest| if rest.as_os_str().is_empty() { root.clone() } else { root.join(rest) }).unwrap_or(path.clone());
            let _ = flag & MUST_SCAN;
            out.push(under);
        }
        out.sort();
        out.dedup();
        Since::Changed { paths: out, cursor: now }
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    /// The journal names a file written after the cursor; once every earlier event is in, a replay from a later
    /// cursor names nothing. A write just before a cursor may be named too (its event's id came after the cursor):
    /// a path looked at again, never one missed.
    #[test]
    fn the_journal_names_what_changed_since_its_cursor() {
        let base = std::env::temp_dir().join(format!("rerail-journal-{}", std::process::id()));
        let (root, cookies) = (base.join("root"), base.join("cookies"));
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::create_dir_all(&cookies).unwrap();
        std::fs::write(root.join("sub/kept.py"), "x = 1\n").unwrap();
        let Since::Lost { cursor: Some(first), .. } = since(std::slice::from_ref(&root), None, &cookies) else { panic!("no cursor without one") };
        std::fs::write(root.join("sub/edited.py"), "y = 2\n").unwrap();
        let Since::Changed { paths, cursor } = since(std::slice::from_ref(&root), Some(first), &cookies) else { panic!("a replay") };
        assert!(paths.contains(&root.join("sub/edited.py")), "the edit is named: {paths:?}");
        assert!(cursor >= first);
        std::thread::sleep(std::time::Duration::from_millis(500));
        let Since::Changed { cursor: settled, .. } = since(std::slice::from_ref(&root), Some(cursor), &cookies) else { panic!("a replay") };
        match since(std::slice::from_ref(&root), Some(settled), &cookies) {
            Since::Changed { paths, .. } => assert!(paths.is_empty(), "nothing since: {paths:?}"),
            other => panic!("{other:?}"),
        }
        let _ = std::fs::remove_dir_all(&base);
    }
}
