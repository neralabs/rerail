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

/// What changed under the roots since a cursor. A cursor is the journal's own text: an FSEvents event id, or a
/// Watchman clock.
#[derive(Debug)]
pub enum Since<C = String> {
    /// The paths that changed (each a file or a directory, as the root's caller names it), and the cursor to ask
    /// from next time.
    Changed { paths: Vec<PathBuf>, cursor: C },
    /// The journal cannot say: everything may have changed. Next time, ask from `cursor`.
    Lost { cursor: Option<C>, why: &'static str },
}

/// The journal a process asks: macOS's FSEvents (the system's, no daemon), else Watchman where it is installed (a
/// daemon outside rerail, its history kept while it runs), else none. `RERAIL_JOURNAL` = `fsevents`, `watchman` or
/// `off` chooses; `RERAIL_WATCHMAN` names the Watchman binary.
enum Backend {
    #[cfg(target_os = "macos")]
    FsEvents,
    Watchman(PathBuf),
    Off,
}

fn backend() -> Backend {
    let watchman = || std::env::var_os("RERAIL_WATCHMAN").map(PathBuf::from).or_else(|| on_path("watchman")).map(Backend::Watchman);
    match std::env::var("RERAIL_JOURNAL").unwrap_or_default().as_str() {
        "off" => Backend::Off,
        "watchman" => watchman().unwrap_or(Backend::Off),
        #[cfg(target_os = "macos")]
        _ => Backend::FsEvents,
        #[cfg(not(target_os = "macos"))]
        _ => watchman().unwrap_or(Backend::Off),
    }
}

fn on_path(name: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?).map(|d| d.join(name)).find(|p| p.is_file())
}

/// The identity a cursor is of: an FSEvents id is of its volume; a Watchman clock names its own instance, so one
/// identity serves (and keeps its cursors apart from FSEvents').
#[cfg_attr(not(target_os = "macos"), allow(unused_variables))]
pub fn volume(path: &Path) -> Option<[u8; 16]> {
    match backend() {
        #[cfg(target_os = "macos")]
        Backend::FsEvents => fsevents::volume(path),
        Backend::Watchman(_) => Some(*b"rerail-watchman\0"),
        Backend::Off => None,
    }
}

/// A new outermost run begins: a question asked after this is answered with every change made before it (each kept
/// stream waits for a cookie once, `fsevents::sync`). Runs inside it share its answer.
pub fn next_epoch() {
    #[cfg(target_os = "macos")]
    fsevents::next_epoch();
}

/// Starts, for the process, a stream for the roots the last runs watched, each from its stored cursor (`roots`:
/// `(root, cursor)`): their replays run beside each other and beside the process's start, before a run asks
/// (`since`). A root under another is the outer one's stream's, from the older of their cursors.
pub fn prepare(roots: &[(PathBuf, String)], cookie_dir: &Path) {
    match backend() {
        #[cfg(target_os = "macos")]
        Backend::FsEvents => {
            let roots: Vec<(PathBuf, u64)> = roots.iter().filter_map(|(r, c)| Some((r.clone(), c.parse().ok()?))).collect();
            fsevents::prepare(&roots, cookie_dir);
        }
        _ => {
            let _ = (roots, cookie_dir);
        }
    }
}

/// What changed under `roots` (each a directory, by the path the caller keeps its records under) since `cursor`
/// (`None`: no earlier cursor), the journal synchronised by a cookie written in `cookie_dir` (Watchman syncs with its
/// own).
pub fn since(roots: &[PathBuf], cursor: Option<&str>, cookie_dir: &Path) -> Since {
    let at = std::time::Instant::now();
    let r = since_backend(roots, cursor, cookie_dir);
    if std::env::var_os("RERAIL_JOURNAL_TRACE").is_some() {
        let what = match &r { Since::Changed { paths, .. } => format!("{} paths", paths.len()), Since::Lost { why, .. } => format!("lost: {why}") };
        eprintln!("journal: {:.3} s, {} roots ({}), {what}", at.elapsed().as_secs_f64(), roots.len(), roots.first().map_or(String::new(), |r| r.display().to_string()));
    }
    r
}

fn since_backend(roots: &[PathBuf], cursor: Option<&str>, cookie_dir: &Path) -> Since {
    match backend() {
        #[cfg(target_os = "macos")]
        Backend::FsEvents => match fsevents::since(roots, cursor.and_then(|c| c.parse().ok()), cookie_dir) {
            Since::Changed { paths, cursor } => Since::Changed { paths, cursor: cursor.to_string() },
            Since::Lost { cursor, why } => Since::Lost { cursor: cursor.map(|c| c.to_string()), why },
        },
        Backend::Watchman(bin) => {
            let _ = cookie_dir;
            watchman::since(&bin, roots, cursor)
        }
        Backend::Off => Since::Lost { cursor: None, why: "no journal on this system" },
    }
}

/// Watchman's JSON protocol over its command line (`watchman -j`): `watch-project` the root, then `query` with
/// `since` the clock kept. Watchman's `sync_timeout` waits for its own cookie, so every change before the query is
/// in its answer. A fresh instance (Watchman started again, or the root first watched) has no history: lost.
mod watchman {
    use super::Since;
    use serde_json::{Value, json};
    use std::io::Write as _;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};

    fn call(bin: &Path, command: &Value) -> Option<Value> {
        let mut child = Command::new(bin).args(["-j", "--no-pretty"]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
        child.stdin.take()?.write_all(command.to_string().as_bytes()).ok()?;
        let out = child.wait_with_output().ok()?;
        let answer: Value = serde_json::from_slice(&out.stdout).ok()?;
        (out.status.success() && answer.get("error").is_none()).then_some(answer)
    }

    pub(super) fn since(bin: &Path, roots: &[PathBuf], cursor: Option<&str>) -> Since {
        // A clock is of one watch: one root a query.
        let [root] = roots else { return Since::Lost { cursor: None, why: "Watchman is asked one root at a time" } };
        let Ok(real) = std::fs::canonicalize(root) else { return Since::Lost { cursor: None, why: "a root is not readable" } };
        let Some(watched) = call(bin, &json!(["watch-project", real])) else { return Since::Lost { cursor: None, why: "Watchman did not answer" } };
        let Some(watch) = watched.get("watch").and_then(Value::as_str) else { return Since::Lost { cursor: None, why: "Watchman named no watch" } };
        let relative = watched.get("relative_path").and_then(Value::as_str);
        let Some(cursor) = cursor else {
            let clock = call(bin, &json!(["clock", watch, {"sync_timeout": 2000}])).and_then(|c| c.get("clock")?.as_str().map(str::to_owned));
            return Since::Lost { cursor: clock, why: "no earlier cursor" };
        };
        let mut query = json!({"since": cursor, "fields": ["name"], "sync_timeout": 2000, "empty_on_fresh_instance": true});
        if let Some(relative) = relative {
            query["relative_root"] = json!(relative);
        }
        let Some(answer) = call(bin, &json!(["query", watch, query])) else { return Since::Lost { cursor: None, why: "Watchman did not answer the query" } };
        let clock = answer.get("clock").and_then(Value::as_str).map(str::to_owned);
        let Some(clock) = clock else { return Since::Lost { cursor: None, why: "Watchman gave no clock" } };
        if answer.get("is_fresh_instance").and_then(Value::as_bool).unwrap_or(true) {
            return Since::Lost { cursor: Some(clock), why: "Watchman has no history from the cursor (a fresh instance)" };
        }
        let names = answer.get("files").and_then(Value::as_array).cloned().unwrap_or_default();
        let mut paths: Vec<PathBuf> = names.iter().filter_map(Value::as_str).map(|n| root.join(n)).collect();
        paths.sort();
        paths.dedup();
        Since::Changed { paths, cursor: clock }
    }

    #[cfg(test)]
    mod tests {
        use super::super::Since;

        /// Against a stand-in for Watchman that answers as its protocol documents: a cursor's first ask is lost and
        /// gives a clock; a query since a clock names the files relative to the root's `relative_root`; a fresh
        /// instance is lost.
        #[test]
        fn watchman_answers_are_read_as_its_protocol_says() {
            let base = std::env::temp_dir().join(format!("rerail-watchman-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&base);
            let root = base.join("repo/sub");
            std::fs::create_dir_all(&root).unwrap();
            let bin = base.join("watchman");
            let real = std::fs::canonicalize(base.join("repo")).unwrap();
            let fresh = base.join("fresh");
            std::fs::write(
                &bin,
                format!(
                    "#!/bin/sh\nread -r line\ncase \"$line\" in\n  *watch-project*) echo '{{\"watch\": \"{w}\", \"relative_path\": \"sub\"}}' ;;\n  *clock*) echo '{{\"clock\": \"c:1:1\"}}' ;;\n  *relative_root*sub*) if [ -e {f} ]; then echo '{{\"clock\": \"c:9:1\", \"is_fresh_instance\": true, \"files\": []}}'; else echo '{{\"clock\": \"c:1:2\", \"is_fresh_instance\": false, \"files\": [\"a.py\", \"pkg/b.py\"]}}'; fi ;;\n  *) echo '{{\"error\": \"no\"}}' ;;\nesac\n",
                    w = real.display(),
                    f = fresh.display()
                ),
            )
            .unwrap();
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
            let roots = [root.clone()];
            let Since::Lost { cursor: Some(first), .. } = super::since(&bin, &roots, None) else { panic!("no cursor without one") };
            assert_eq!(first, "c:1:1");
            let Since::Changed { paths, cursor } = super::since(&bin, &roots, Some(&first)) else { panic!("a query") };
            assert_eq!((paths, cursor.as_str()), (vec![root.join("a.py"), root.join("pkg/b.py")], "c:1:2"));
            std::fs::write(&fresh, "").unwrap();
            let Since::Lost { cursor: Some(again), .. } = super::since(&bin, &roots, Some(&cursor)) else { panic!("a fresh instance is lost") };
            assert_eq!(again, "c:9:1");
            let _ = std::fs::remove_dir_all(&base);
        }
    }
}

#[cfg(target_os = "macos")]
mod fsevents {
    type Since = super::Since<u64>;
    use std::ffi::{CStr, CString, c_char, c_void};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
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

    /// The stream's own reference to `Shared` (its context's `retain`), given back when FSEvents frees the stream:
    /// what a callback reads lives as long as the stream, not only as long as `since`.
    extern "C" fn retain(info: *const c_void) -> *const c_void {
        // SAFETY: `info` is an `Arc<Shared>`'s pointer, and the caller holds a reference while the stream is made.
        unsafe { Arc::increment_strong_count(info as *const Shared) };
        info
    }

    /// The stream's reference given back when FSEvents frees the stream (its context's `release`).
    extern "C" fn release(info: *const c_void) {
        // SAFETY: balances the one `retain` took.
        unsafe { Arc::decrement_strong_count(info as *const Shared) };
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

    /// How many streams a process keeps (one per root not under another's): the oldest unused goes first.
    const STREAMS: usize = 16;
    /// How many events a stream keeps before it says it lost track (and is replaced by a fresh replay).
    const MAX_EVENTS: usize = 1 << 21;

    /// What a stream has been told: every event since the id it was started from, with its id.
    #[derive(Default)]
    struct Seen {
        events: Vec<(u64, String)>,
        history_done: bool,
        /// The cookies seen, with their events' ids.
        cookies: std::collections::HashMap<String, u64>,
        lost: Option<&'static str>,
    }

    struct Shared {
        seen: Mutex<Seen>,
        ready: Condvar,
        /// The cookies' prefix, `<cookie dir>/rerail-cookie-<pid>-`: an event under it is a cookie, not a change.
        cookie: String,
    }

    extern "C" fn callback(_: Stream, info: *mut c_void, n: usize, paths: *mut c_void, flags: *const u32, ids: *const u64) {
        // SAFETY: `info` is the `Shared` the stream holds a reference to (`retain`), and the arrays hold `n` entries
        // for the callback's duration (FSEvents' contract, without `UseCFTypes`).
        let shared = unsafe { &*(info as *const Shared) };
        let paths = paths as *const *const c_char;
        let mut seen = shared.seen.lock().unwrap_or_else(|e| e.into_inner());
        for i in 0..n {
            let (path, flag, id) = unsafe { (CStr::from_ptr(*paths.add(i)).to_string_lossy().into_owned(), *flags.add(i), *ids.add(i)) };
            if flag & LOST != 0 {
                seen.lost = Some("the system dropped or renumbered events");
            }
            if flag & HISTORY_DONE != 0 {
                seen.history_done = true;
                continue;
            }
            if path.starts_with(&shared.cookie) {
                seen.cookies.entry(path).or_insert(id);
                continue;
            }
            seen.events.push((id, path));
        }
        if seen.events.len() > MAX_EVENTS {
            seen.lost = Some("more changes than a stream keeps");
            seen.events = Vec::new();
        }
        shared.ready.notify_all();
    }

    /// A stream the process keeps: started from the oldest cursor asked of its roots, its history replayed once,
    /// then told every change as it comes. A later question under its roots is answered from what it was told, once
    /// a cookie shows it has been told everything up to the question's run (`sync`).
    struct Live {
        stream: Stream,
        queue: *mut c_void,
        shared: Arc<Shared>,
        /// Its roots, canonical, and the cookie directory it watches with them.
        roots: Vec<PathBuf>,
        cookie_dir: PathBuf,
        /// The id it was started from: it holds every event after it.
        from: u64,
        /// The run it was last synchronised for (`EPOCH`), and its cookie's event's id: every change before it has an
        /// earlier id, and was told before it.
        synced: Option<(u64, u64)>,
        used: u64,
    }

    // SAFETY: the stream and queue are FSEvents' and libdispatch's handles, usable from any thread; they are only
    // touched under `LIVE`'s lock.
    unsafe impl Send for Live {}

    impl Drop for Live {
        fn drop(&mut self) {
            // SAFETY: stopped and invalidated before release; the stream holds its own reference to `shared`
            // (`retain`) until FSEvents frees it, so no callback reads freed memory whenever it runs.
            unsafe {
                FSEventStreamStop(self.stream);
                FSEventStreamInvalidate(self.stream);
                FSEventStreamRelease(self.stream);
                dispatch_release(self.queue);
            }
        }
    }

    static LIVE: Mutex<Vec<Live>> = Mutex::new(Vec::new());
    /// The outermost run under way (`next_epoch`): a stream is synchronised once for it.
    static EPOCH: AtomicU64 = AtomicU64::new(0);
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    /// A new outermost run: a question after this waits for a cookie of its own, once per stream.
    pub(super) fn next_epoch() {
        EPOCH.fetch_add(1, Ordering::Relaxed);
    }

    fn start(roots: &[PathBuf], cookie_dir: &Path, from: u64) -> Result<Live, &'static str> {
        let prefix = cookie_dir.join(format!("rerail-cookie-{}-", std::process::id()));
        let shared = Arc::new(Shared { seen: Mutex::new(Seen::default()), ready: Condvar::new(), cookie: prefix.to_string_lossy().into_owned() });
        let mut watched: Vec<CString> = roots.iter().map(|r| CString::new(r.to_string_lossy().as_bytes()).unwrap_or_default()).collect();
        watched.push(CString::new(cookie_dir.to_string_lossy().as_bytes()).unwrap_or_default());
        // SAFETY: the strings and array are released below; the stream holds its own references while it lives.
        unsafe {
            let strings: Vec<CFRef> = watched.iter().map(|s| CFStringCreateWithCString(std::ptr::null(), s.as_ptr(), UTF8)).collect();
            let array = CFArrayCreate(std::ptr::null(), strings.as_ptr(), strings.len() as CFIndex, &kCFTypeArrayCallBacks);
            for s in strings {
                CFRelease(s);
            }
            let mut context = Context { version: 0, info: Arc::as_ptr(&shared) as *mut c_void, retain: retain as *const c_void, release: release as *const c_void, copy_description: std::ptr::null() };
            let stream = FSEventStreamCreate(std::ptr::null(), callback, &mut context, array, from, 0.0, NO_DEFER | FILE_EVENTS);
            CFRelease(array);
            if stream.is_null() {
                return Err("the journal refused the stream");
            }
            let queue = dispatch_queue_create(c"rerail.journal".as_ptr(), std::ptr::null());
            FSEventStreamSetDispatchQueue(stream, queue);
            if FSEventStreamStart(stream) == 0 {
                FSEventStreamInvalidate(stream);
                FSEventStreamRelease(stream);
                dispatch_release(queue);
                return Err("the journal did not start");
            }
            Ok(Live { stream, queue, shared, roots: roots.to_vec(), cookie_dir: cookie_dir.to_path_buf(), from, synced: None, used: 0 })
        }
    }

    /// Waits, once for the run (`EPOCH`), until `live` has been told every change made before now: a cookie written
    /// now has a later id than each, and comes after them. The cookie's id: what it has been told up to it is
    /// complete, so a replay from it misses nothing (and no id need be asked of the system).
    fn sync(live: &mut Live, epoch: u64) -> Result<u64, &'static str> {
        if let Some((e, now)) = live.synced
            && e == epoch
        {
            return Ok(now);
        }
        let cookie = format!("{}{}", live.shared.cookie, COUNTER.fetch_add(1, Ordering::Relaxed));
        if std::fs::write(&cookie, b"").is_err() {
            return Err("no cookie could be written");
        }
        let deadline = Instant::now() + WAIT;
        let mut seen = live.shared.seen.lock().unwrap_or_else(|e| e.into_inner());
        while !(seen.cookies.contains_key(&cookie) && seen.history_done) && seen.lost.is_none() {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            seen = live.shared.ready.wait_timeout(seen, left).unwrap_or_else(|e| e.into_inner()).0;
        }
        let done = seen.history_done.then(|| seen.cookies.get(&cookie).copied()).flatten();
        seen.cookies.clear();
        let lost = seen.lost;
        drop(seen);
        let _ = std::fs::remove_file(&cookie);
        if let Some(why) = lost {
            return Err(why);
        }
        let Some(id) = done else { return Err("the cookie's event did not arrive") };
        live.synced = Some((epoch, id));
        Ok(id)
    }

    pub(super) fn volume(path: &Path) -> Option<[u8; 16]> {
        use std::os::unix::fs::MetadataExt as _;
        let dev = std::fs::metadata(path).ok()?.dev() as i32;
        // Asked once a root by the observer, which keeps it with the root (`Observer::volumes`).
        uuid_of(dev)
    }

    fn uuid_of(dev: i32) -> Option<[u8; 16]> {
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

    pub(super) fn prepare(roots: &[(PathBuf, u64)], cookie_dir: &Path) {
        // SAFETY: no arguments; returns the latest id.
        let now = unsafe { FSEventsGetCurrentEventId() };
        let Ok(cookie_dir) = std::fs::canonicalize(cookie_dir) else { return };
        let mut real: Vec<(PathBuf, u64)> = roots.iter().filter(|(_, c)| *c <= now).filter_map(|(r, c)| Some((std::fs::canonicalize(r).ok()?, *c))).collect();
        real.sort();
        // Outer roots first (a path sorts before those under it): each takes the cursors of the roots under it.
        let mut groups: Vec<(PathBuf, u64)> = Vec::new();
        for (r, c) in real {
            match groups.last_mut() {
                Some((g, from)) if r.starts_with(&*g) => *from = (*from).min(c),
                _ => groups.push((r, c)),
            }
        }
        let mut all = LIVE.lock().unwrap_or_else(|e| e.into_inner());
        for (root, from) in groups {
            if all.len() >= STREAMS {
                break;
            }
            if all.iter().any(|l| l.cookie_dir == cookie_dir && l.from <= from && l.roots.iter().any(|lr| root.starts_with(lr))) {
                continue;
            }
            if let Ok(live) = start(std::slice::from_ref(&root), &cookie_dir, from) {
                all.push(live);
            }
        }
    }

    pub(super) fn since(roots: &[PathBuf], cursor: Option<u64>, cookie_dir: &Path) -> Since {
        // The id now, asked of the system only where no stream's cookie gives one (it waits on `fseventsd`): a change
        // after it is the next replay's.
        // SAFETY: no arguments; returns the latest id.
        let now = || unsafe { FSEventsGetCurrentEventId() };
        let Some(cursor) = cursor else { return Since::Lost { cursor: Some(now()), why: "no earlier cursor" } };
        // Records are kept by the path the caller gave; events come by the real path.
        let Ok(cookie_dir) = std::fs::canonicalize(cookie_dir) else { return Since::Lost { cursor: Some(now()), why: "no cookie directory" } };
        let real: Vec<(PathBuf, PathBuf)> = roots.iter().filter_map(|r| Some((std::fs::canonicalize(r).ok()?, r.clone()))).collect();
        if real.len() != roots.len() {
            return Since::Lost { cursor: Some(now()), why: "a root is not readable" };
        }
        let epoch = EPOCH.load(Ordering::Relaxed);
        let mut all = LIVE.lock().unwrap_or_else(|e| e.into_inner());
        // A stream that lost track answers nothing more: a question under its roots replays afresh.
        all.retain(|l| l.shared.seen.lock().unwrap_or_else(|e| e.into_inner()).lost.is_none());
        // The stream that holds every change under the roots since the cursor, else one started for them from it.
        let held = all.iter().position(|l| l.cookie_dir == cookie_dir && l.from <= cursor && real.iter().all(|(r, _)| l.roots.iter().any(|lr| r.starts_with(lr))));
        let at = match held {
            Some(at) => at,
            None => {
                let id_now = now();
                if cursor > id_now {
                    return Since::Lost { cursor: Some(id_now), why: "a cursor ahead of the journal" };
                }
                if all.len() >= STREAMS
                    && let Some(old) = all.iter().enumerate().min_by_key(|(_, l)| l.used).map(|(i, _)| i)
                {
                    all.remove(old);
                }
                let mut asked: Vec<PathBuf> = real.iter().map(|(r, _)| r.clone()).collect();
                asked.sort();
                asked.dedup_by(|b, a| b.starts_with(&*a));
                match start(&asked, &cookie_dir, cursor) {
                    Ok(live) => {
                        all.push(live);
                        all.len() - 1
                    }
                    Err(why) => return Since::Lost { cursor: Some(id_now), why },
                }
            }
        };
        all[at].used = COUNTER.fetch_add(1, Ordering::Relaxed);
        let t = Instant::now();
        let fresh = all[at].synced.is_none_or(|(e, _)| e != epoch);
        let r = sync(&mut all[at], epoch);
        if std::env::var_os("RERAIL_JOURNAL_TRACE").is_some() {
            let n = all[at].shared.seen.lock().map(|s| s.events.len()).unwrap_or(0);
            eprintln!("journal stream {at} of {}: {}{}, epoch {epoch}, {:.3} s, {n} events kept, from {} (cursor {cursor})", all.len(), if held.is_some() { "kept" } else { "new" }, if fresh { ", cookie" } else { "" }, t.elapsed().as_secs_f64(), all[at].from);
        }
        let synced = match r {
            Ok(synced) => synced,
            Err(why) => {
                all.remove(at);
                return Since::Lost { cursor: Some(now()), why };
            }
        };
        // A directory to scan again is changed as a whole: the caller drops what it knows under it.
        let mut out: Vec<PathBuf> = Vec::new();
        {
            let seen = all[at].shared.seen.lock().unwrap_or_else(|e| e.into_inner());
            for (id, path) in seen.events.iter().filter(|(id, _)| *id > cursor) {
                let _ = id;
                let path = Path::new(path.trim_end_matches('/'));
                let Some((real_root, root)) = real.iter().find(|(r, _)| path.starts_with(r)) else { continue };
                let under = path.strip_prefix(real_root).map(|rest| if rest.as_os_str().is_empty() { root.clone() } else { root.join(rest) }).unwrap_or_else(|_| path.to_path_buf());
                out.push(under);
            }
        }
        out.sort();
        out.dedup();
        Since::Changed { paths: out, cursor: synced.max(cursor) }
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::Since;
    use super::fsevents::next_epoch;
    use super::fsevents::since;
    use std::path::PathBuf;

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
        next_epoch();
        let Since::Changed { paths, cursor } = since(std::slice::from_ref(&root), Some(first), &cookies) else { panic!("a replay") };
        assert!(paths.contains(&root.join("sub/edited.py")), "the edit is named: {paths:?}");
        assert!(cursor >= first);
        std::thread::sleep(std::time::Duration::from_millis(500));
        next_epoch();
        let Since::Changed { cursor: settled, .. } = since(std::slice::from_ref(&root), Some(cursor), &cookies) else { panic!("a replay") };
        next_epoch();
        match since(std::slice::from_ref(&root), Some(settled), &cookies) {
            Since::Changed { paths, .. } => assert!(paths.is_empty(), "nothing since: {paths:?}"),
            other => panic!("{other:?}"),
        }
        // A root under one asked is answered by the same stream, kept for the process: a change made since the
        // run's question is told to the next run.
        std::fs::write(root.join("sub/later.py"), "z = 3\n").unwrap();
        next_epoch();
        let Since::Changed { paths, .. } = since(&[root.join("sub")], Some(settled), &cookies) else { panic!("a replay") };
        assert!(paths.contains(&root.join("sub/later.py")), "the next run is told: {paths:?}");
        let _ = std::fs::remove_dir_all(&base);
    }
}
