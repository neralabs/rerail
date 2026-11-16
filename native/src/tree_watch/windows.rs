//! Windows: one directory handle per watched root, read by `ReadDirectoryChangesW` over its whole
//! tree, every handle on one I/O completion port served by one thread, `rerail-tree-watch`.
//!
//! A plain handle on a folder makes renaming, moving or deleting it, or any folder above it, fail
//! ("access denied"). Each handle therefore holds a read-handle oplock: such a rename breaks it,
//! the thread closes the handle at once (which lets the rename go on) and the root's watch ends.
//! The oplock also breaks on a change among the root's own entries, which ends the watch the same
//! way. A root the file system grants no oplock on, a network path, a mapped remote drive and a
//! WSL path are not watched. An overflow of the 64 KiB buffer (`ERROR_NOTIFY_ENUM_DIR`, or a read
//! that returns nothing) makes the root `lost`.
//!
//! The host's calls run on the thread, which answers each before the host goes on: `unwatch`
//! returns with the handle closed, so `git worktree remove` and renames work at once.

use super::Backend;
use super::notices::Notices;
use crate::panic_boundary::contain_panic;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{SyncSender, sync_channel};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_IO_PENDING, ERROR_NOTIFY_ENUM_DIR, GetLastError, HANDLE,
    INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ACTION_MODIFIED, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OVERLAPPED,
    FILE_LIST_DIRECTORY, FILE_NOTIFY_CHANGE_ATTRIBUTES, FILE_NOTIFY_CHANGE_CREATION,
    FILE_NOTIFY_CHANGE_DIR_NAME, FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE,
    FILE_NOTIFY_CHANGE_SIZE, FILE_NOTIFY_INFORMATION, FILE_SHARE_DELETE, FILE_SHARE_READ,
    FILE_SHARE_WRITE, GetDriveTypeW, GetVolumePathNameW, OPEN_EXISTING, ReadDirectoryChangesW,
};
use windows_sys::Win32::System::IO::{
    CreateIoCompletionPort, DeviceIoControl, GetQueuedCompletionStatus, OVERLAPPED,
    PostQueuedCompletionStatus,
};
use windows_sys::Win32::System::Ioctl::{
    FSCTL_REQUEST_OPLOCK, OPLOCK_LEVEL_CACHE_HANDLE, OPLOCK_LEVEL_CACHE_READ,
    REQUEST_OPLOCK_CURRENT_VERSION, REQUEST_OPLOCK_INPUT_BUFFER, REQUEST_OPLOCK_INPUT_FLAG_REQUEST,
    REQUEST_OPLOCK_OUTPUT_BUFFER,
};
use windows_sys::Win32::System::Threading::INFINITE;

const BUFFER_BYTES: usize = 64 * 1024;
const FILTER: u32 = FILE_NOTIFY_CHANGE_FILE_NAME
    | FILE_NOTIFY_CHANGE_DIR_NAME
    | FILE_NOTIFY_CHANGE_SIZE
    | FILE_NOTIFY_CHANGE_LAST_WRITE
    | FILE_NOTIFY_CHANGE_CREATION
    | FILE_NOTIFY_CHANGE_ATTRIBUTES;
/// `GetDriveTypeW`'s answer for a network drive.
const DRIVE_REMOTE: u32 = 4;
/// The completion key that wakes the thread for the host's calls; roots' keys start at 1.
const WAKE: usize = 0;
/// How long `close` waits for the cancelled reads of closed handles to complete.
const DRAIN_MILLISECONDS: u32 = 5_000;

/// A call of the host's for the thread, with where to answer it.
enum Command {
    Watch(String, SyncSender<bool>),
    Unwatch(String, SyncSender<bool>),
    #[cfg(test)]
    Roots(SyncSender<usize>),
    Stop,
}

/// What the thread's I/O writes into while it is in flight; boxed, so it stays put.
struct Io {
    read: OVERLAPPED,
    oplock: OVERLAPPED,
    oplock_in: REQUEST_OPLOCK_INPUT_BUFFER,
    oplock_out: REQUEST_OPLOCK_OUTPUT_BUFFER,
    /// `ReadDirectoryChangesW` wants a DWORD-aligned buffer.
    buffer: Vec<u32>,
}

/// One root's handle and its two operations: the read and the oplock. Each flag holds while that
/// operation's completion is still to come, so its OVERLAPPED stays put until then.
struct Watch {
    root: String,
    handle: HANDLE,
    io: Box<Io>,
    reading: bool,
    oplocked: bool,
}

impl Watch {
    fn open(root: String, handle: HANDLE) -> Self {
        let io = Box::new(Io {
            read: OVERLAPPED::default(),
            oplock: OVERLAPPED::default(),
            oplock_in: REQUEST_OPLOCK_INPUT_BUFFER {
                StructureVersion: REQUEST_OPLOCK_CURRENT_VERSION as u16,
                StructureLength: size_of::<REQUEST_OPLOCK_INPUT_BUFFER>() as u16,
                RequestedOplockLevel: OPLOCK_LEVEL_CACHE_READ | OPLOCK_LEVEL_CACHE_HANDLE,
                Flags: REQUEST_OPLOCK_INPUT_FLAG_REQUEST,
            },
            oplock_out: REQUEST_OPLOCK_OUTPUT_BUFFER {
                StructureVersion: REQUEST_OPLOCK_CURRENT_VERSION as u16,
                StructureLength: size_of::<REQUEST_OPLOCK_OUTPUT_BUFFER>() as u16,
                ..REQUEST_OPLOCK_OUTPUT_BUFFER::default()
            },
            buffer: vec![0; BUFFER_BYTES / 4],
        });
        Self {
            root,
            handle,
            io,
            reading: false,
            oplocked: false,
        }
    }

    /// Whether a completion of either operation is still to come.
    fn pending(&self) -> bool {
        self.reading || self.oplocked
    }

    fn open_handle(&self) -> bool {
        self.handle != INVALID_HANDLE_VALUE
    }

    /// Binds the handle to `port` under `key`, takes the oplock and starts the first read; true
    /// when all three hold. Otherwise the handle is closed.
    fn start(&mut self, port: HANDLE, key: usize) -> bool {
        // SAFETY: this watch's handle, bound to the thread's port under a fresh key.
        let bound = !unsafe { CreateIoCompletionPort(self.handle, port, key, 0) }.is_null();
        let started = bound && self.request_oplock() && self.read();
        if !started {
            self.close_handle();
        }
        started
    }

    /// Requests the read-handle oplock; true when it is granted (its completion is the break).
    fn request_oplock(&mut self) -> bool {
        let io = &mut *self.io;
        // SAFETY: the boxed buffers and OVERLAPPED stay put until the completion is dequeued.
        let done = unsafe {
            DeviceIoControl(
                self.handle,
                FSCTL_REQUEST_OPLOCK,
                (&raw const io.oplock_in).cast(),
                size_of::<REQUEST_OPLOCK_INPUT_BUFFER>() as u32,
                (&raw mut io.oplock_out).cast(),
                size_of::<REQUEST_OPLOCK_OUTPUT_BUFFER>() as u32,
                std::ptr::null_mut(),
                &mut io.oplock,
            )
        } != 0;
        self.oplocked = completion_comes(done);
        // One that finished at once holds no oplock.
        !done && self.oplocked
    }

    /// Starts the next read of the tree's changes.
    fn read(&mut self) -> bool {
        let io = &mut *self.io;
        // SAFETY: the boxed buffer and OVERLAPPED stay put until the completion is dequeued.
        let done = unsafe {
            ReadDirectoryChangesW(
                self.handle,
                io.buffer.as_mut_ptr().cast(),
                BUFFER_BYTES as u32,
                1,
                FILTER,
                std::ptr::null_mut(),
                &mut io.read,
                None,
            )
        } != 0;
        self.reading = completion_comes(done);
        self.reading
    }

    fn close_handle(&mut self) {
        if self.open_handle() {
            // SAFETY: this watch's handle, closed once; its operations complete as cancelled.
            unsafe { CloseHandle(self.handle) };
            self.handle = INVALID_HANDLE_VALUE;
        }
    }

    /// The bytes the last read returned.
    fn returned(&self, bytes: u32) -> &[u8] {
        let length = (bytes as usize).min(BUFFER_BYTES);
        // SAFETY: the buffer holds `BUFFER_BYTES` initialized bytes.
        unsafe { std::slice::from_raw_parts(self.io.buffer.as_ptr().cast::<u8>(), length) }
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        self.close_handle();
        if self.pending() {
            // The kernel may still write into it; leak it rather than free it under the kernel.
            std::mem::forget(std::mem::replace(
                &mut self.io,
                Box::new(Io {
                    read: OVERLAPPED::default(),
                    oplock: OVERLAPPED::default(),
                    oplock_in: REQUEST_OPLOCK_INPUT_BUFFER::default(),
                    oplock_out: REQUEST_OPLOCK_OUTPUT_BUFFER::default(),
                    buffer: Vec::new(),
                }),
            ));
        }
    }
}

/// Whether an overlapped call that returned `done` posts a completion to the port it is in flight
/// for: it finished at once, or is pending. One that failed at once finished in the kernel before
/// it returned; a completion it may still post is not counted, and is told apart by its
/// OVERLAPPED, whose flag is clear.
fn completion_comes(done: bool) -> bool {
    // SAFETY: reads the calling thread's last error.
    done || unsafe { GetLastError() } == ERROR_IO_PENDING
}

/// The wide, NUL-terminated form of `text`.
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// `root` with `\` separators, prefixed `\\?\` when it is a drive path, so no length limit applies.
fn long_path(root: &str) -> String {
    let path = root.replace('/', "\\");
    let drive = path.as_bytes().get(1) == Some(&b':');
    if drive {
        format!("\\\\?\\{path}")
    } else {
        path
    }
}

/// Whether `root` is on another machine: a UNC path (WSL's `\\wsl$` and `\\wsl.localhost`
/// included), a device path, or a drive Windows maps to a network share. A path whose volume
/// cannot be told is taken as local: opening it decides.
fn remote(root: &str) -> bool {
    let path = root.replace('/', "\\");
    let lower = path.to_ascii_lowercase();
    if lower.starts_with("\\\\?\\unc\\") {
        return true;
    }
    if lower.starts_with("\\\\") && !lower.starts_with("\\\\?\\") {
        return true;
    }
    let mut volume = vec![0_u16; 1024];
    // SAFETY: a NUL-terminated path and a buffer of the length passed.
    let found = unsafe {
        GetVolumePathNameW(
            wide(&path).as_ptr(),
            volume.as_mut_ptr(),
            volume.len() as u32,
        )
    };
    // SAFETY: a NUL-terminated volume path `GetVolumePathNameW` wrote.
    found != 0 && unsafe { GetDriveTypeW(volume.as_ptr()) } == DRIVE_REMOTE
}

/// A handle on the folder `root` for reading its changes, or none.
fn open_directory(root: &str) -> Option<HANDLE> {
    // SAFETY: a NUL-terminated path; the handle is owned by the caller.
    let handle = unsafe {
        CreateFileW(
            wide(&long_path(root)).as_ptr(),
            FILE_LIST_DIRECTORY,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED,
            std::ptr::null_mut(),
        )
    };
    (handle != INVALID_HANDLE_VALUE).then_some(handle)
}

/// Calls `entry` with each record of a read's `bytes`: the path (relative, `/`-separated) and
/// whether the entry's name changed (created, deleted or renamed) rather than its content.
fn parse(bytes: &[u8], mut entry: impl FnMut(String, bool)) {
    let header = std::mem::offset_of!(FILE_NOTIFY_INFORMATION, FileName);
    let mut offset = 0;
    while offset + header <= bytes.len() {
        // SAFETY: a whole header lies at `offset`; it is read without assuming alignment.
        let record = unsafe {
            std::ptr::read_unaligned(bytes.as_ptr().add(offset).cast::<FILE_NOTIFY_INFORMATION>())
        };
        let start = offset + header;
        let end = (start + record.FileNameLength as usize).min(bytes.len());
        let units: Vec<u16> = bytes[start..end]
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        let path = String::from_utf16_lossy(&units).replace('\\', "/");
        entry(path, record.Action != FILE_ACTION_MODIFIED);
        if record.NextEntryOffset == 0 {
            return;
        }
        offset += record.NextEntryOffset as usize;
    }
}

/// What a completed read means for its root.
#[derive(Debug, PartialEq, Eq)]
enum ReadOutcome {
    /// Records to report, then read on.
    Records,
    /// The buffer overflowed: the root is lost, then read on.
    Overflowed,
    /// The read failed: the watch ends.
    Failed,
}

fn read_outcome(error: u32, bytes: u32) -> ReadOutcome {
    match (error, bytes) {
        (ERROR_NOTIFY_ENUM_DIR, _) | (0, 0) => ReadOutcome::Overflowed,
        (0, _) => ReadOutcome::Records,
        _ => ReadOutcome::Failed,
    }
}

/// The thread's state: every root's watch, by completion key and by root.
struct Loop {
    port: HANDLE,
    notices: Arc<Notices>,
    watches: HashMap<usize, Watch>,
    keys: HashMap<String, usize>,
    next_key: usize,
}

impl Loop {
    fn watch(&mut self, root: String) -> bool {
        if self.keys.contains_key(&root) {
            return true;
        }
        let Some(handle) = (!remote(&root)).then(|| open_directory(&root)).flatten() else {
            return false;
        };
        let key = self.next_key;
        self.next_key += 1;
        let mut watch = Watch::open(root, handle);
        let started = watch.start(self.port, key);
        if started {
            self.keys.insert(watch.root.clone(), key);
        }
        // A watch that did not start is kept until the completions its calls posted come.
        if watch.pending() {
            self.watches.insert(key, watch);
        }
        started
    }

    /// Closes `key`'s handle; its watch goes once its cancelled operations complete.
    fn close(&mut self, key: usize) {
        let Some(watch) = self.watches.get_mut(&key) else {
            return;
        };
        watch.close_handle();
        self.keys.remove(&watch.root);
        if !watch.pending() {
            self.watches.remove(&key);
        }
    }

    fn unwatch(&mut self, root: &str) {
        if let Some(key) = self.keys.get(root).copied() {
            self.close(key);
        }
    }

    /// Ends `key`'s watch: the host reads its root whole and watches it again.
    fn end(&mut self, key: usize) {
        if let Some(watch) = self.watches.get(&key) {
            self.notices.ended(&watch.root);
        }
        self.close(key);
    }

    /// One operation of `key` completed. A completion of an operation not in flight (posted by a
    /// call that failed at once) is dropped: the watch goes only once neither can still come.
    fn completed(&mut self, key: usize, overlapped: *mut OVERLAPPED, bytes: u32, error: u32) {
        let Some(watch) = self.watches.get_mut(&key) else {
            return;
        };
        let oplock = std::ptr::eq(overlapped, &raw mut watch.io.oplock);
        let in_flight = if oplock {
            &mut watch.oplocked
        } else if std::ptr::eq(overlapped, &raw mut watch.io.read) {
            &mut watch.reading
        } else {
            return;
        };
        if !std::mem::replace(in_flight, false) {
            return;
        }
        if !watch.open_handle() {
            if !watch.pending() {
                self.watches.remove(&key);
            }
            return;
        }
        if oplock {
            // The oplock broke: let go of the handle so whatever broke it goes on.
            self.end(key);
            return;
        }
        self.read_completed(key, bytes, error);
    }

    fn read_completed(&mut self, key: usize, bytes: u32, error: u32) {
        let Some(watch) = self.watches.get_mut(&key) else {
            return;
        };
        match read_outcome(error, bytes) {
            ReadOutcome::Records => {
                let notices = &self.notices;
                let root = &watch.root;
                parse(watch.returned(bytes), |path, renamed| match renamed {
                    true => notices.renamed(root, path),
                    false => notices.changed(root, path),
                });
            }
            ReadOutcome::Overflowed => self.notices.lost(&watch.root),
            ReadOutcome::Failed => return self.end(key),
        }
        if !watch.read() {
            self.end(key);
        }
    }

    /// Closes every handle and waits for their cancelled operations, so nothing is freed under the
    /// kernel.
    fn shut(&mut self) {
        let keys: Vec<usize> = self.watches.keys().copied().collect();
        for key in keys {
            self.close(key);
        }
        while !self.watches.is_empty() {
            let Some((key, overlapped, bytes, error)) = dequeue(self.port, DRAIN_MILLISECONDS)
            else {
                return;
            };
            if !overlapped.is_null() {
                self.completed(key, overlapped, bytes, error);
            }
        }
    }
}

/// The next completion on `port`: its key, OVERLAPPED (null for a wake), bytes and error. None
/// when none came in `milliseconds` or the port failed.
fn dequeue(port: HANDLE, milliseconds: u32) -> Option<(usize, *mut OVERLAPPED, u32, u32)> {
    let (mut bytes, mut key, mut overlapped) = (0_u32, 0_usize, std::ptr::null_mut());
    // SAFETY: the port lives until the thread is joined; the out-parameters are locals.
    let done = unsafe {
        GetQueuedCompletionStatus(port, &mut bytes, &mut key, &mut overlapped, milliseconds)
    };
    // SAFETY: reads the calling thread's last error.
    let error = if done == 0 {
        unsafe { GetLastError() }
    } else {
        0
    };
    (done != 0 || !overlapped.is_null()).then_some((key, overlapped, bytes, error))
}

/// What the host's thread and the watch thread share.
struct Shared {
    port: HANDLE,
    commands: Mutex<VecDeque<Command>>,
    /// The thread has stopped: no call reaches it any more.
    stopped: AtomicBool,
}

// SAFETY: the port handle is used from both threads, which Windows allows for completion ports.
unsafe impl Send for Shared {}
// SAFETY: as above; the queue is behind a mutex.
unsafe impl Sync for Shared {}

impl Shared {
    fn commands(&self) -> MutexGuard<'_, VecDeque<Command>> {
        self.commands.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Queues `command` and wakes the thread; false once the thread has stopped.
    fn send(&self, command: Command) -> bool {
        let mut commands = self.commands();
        if self.stopped.load(Ordering::Acquire) {
            return false;
        }
        commands.push_back(command);
        // SAFETY: the port lives until the thread is joined.
        unsafe { PostQueuedCompletionStatus(self.port, 0, WAKE, std::ptr::null()) != 0 }
    }

    /// Runs `command` (made with the answer's sender) on the thread and waits for its answer;
    /// the default (false) when the thread is gone.
    fn call<T: Default>(&self, command: impl FnOnce(SyncSender<T>) -> Command) -> T {
        let (answer, answered) = sync_channel(1);
        if !self.send(command(answer)) {
            return T::default();
        }
        answered.recv().unwrap_or_default()
    }
}

/// The thread: serves completions and the host's calls until stopped. A panic ends every watch.
fn run(shared: &Shared, notices: &Arc<Notices>) {
    let mut state = Loop {
        port: shared.port,
        notices: Arc::clone(notices),
        watches: HashMap::new(),
        keys: HashMap::new(),
        next_key: WAKE + 1,
    };
    let served = contain_panic("tree watch", || {
        serve(shared, &mut state);
        Ok(())
    });
    if served.is_err() {
        notices.ended_all();
    }
    state.shut();
    shared.stopped.store(true, Ordering::Release);
    // Answers no call left in the queue: dropping their senders answers false.
    shared.commands().clear();
}

fn serve(shared: &Shared, state: &mut Loop) {
    loop {
        let Some((key, overlapped, bytes, error)) = dequeue(shared.port, INFINITE) else {
            return;
        };
        if !overlapped.is_null() {
            state.completed(key, overlapped, bytes, error);
        } else if key == WAKE && !run_commands(shared, state) {
            return;
        }
    }
}

/// Runs the host's queued calls; false on `Stop`.
fn run_commands(shared: &Shared, state: &mut Loop) -> bool {
    loop {
        let Some(command) = shared.commands().pop_front() else {
            return true;
        };
        match command {
            Command::Watch(root, answer) => {
                let _ = answer.send(state.watch(root));
            }
            Command::Unwatch(root, answer) => {
                state.unwatch(&root);
                let _ = answer.send(true);
            }
            #[cfg(test)]
            Command::Roots(answer) => {
                let _ = answer.send(state.keys.len());
            }
            Command::Stop => return false,
        }
    }
}

pub(super) struct DirectoryChanges {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

impl DirectoryChanges {
    pub(super) fn open(notices: Arc<Notices>) -> Option<Self> {
        // SAFETY: creates a new completion port, owned by this backend.
        let port =
            unsafe { CreateIoCompletionPort(INVALID_HANDLE_VALUE, std::ptr::null_mut(), 0, 1) };
        if port.is_null() {
            return None;
        }
        let shared = Arc::new(Shared {
            port,
            commands: Mutex::new(VecDeque::new()),
            stopped: AtomicBool::new(false),
        });
        let thread_shared = Arc::clone(&shared);
        let spawned = std::thread::Builder::new()
            .name("rerail-tree-watch".to_owned())
            .spawn(move || run(&thread_shared, &notices));
        let Ok(thread) = spawned else {
            // SAFETY: the port created above, which no thread uses.
            unsafe { CloseHandle(port) };
            return None;
        };
        Some(Self {
            shared,
            thread: Some(thread),
        })
    }
}

impl Backend for DirectoryChanges {
    fn watch(&mut self, root: &str, _folders: &[String]) -> bool {
        self.shared
            .call(|answer| Command::Watch(root.to_owned(), answer))
    }

    fn unwatch(&mut self, root: &str) {
        self.shared
            .call(|answer| Command::Unwatch(root.to_owned(), answer));
    }

    fn close(&mut self) {
        let Some(thread) = self.thread.take() else {
            return;
        };
        self.shared.send(Command::Stop);
        let _ = thread.join();
        // SAFETY: the port, which the joined thread no longer uses.
        unsafe { CloseHandle(self.shared.port) };
    }

    #[cfg(test)]
    fn roots(&self) -> usize {
        self.shared.call(Command::Roots)
    }
}

impl Drop for DirectoryChanges {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::{Loop, ReadOutcome, WAKE, Watch, parse, read_outcome, remote};
    use crate::tree_watch::TreeWatcher;
    use crate::tree_watch::notices::Notices;
    use crate::tree_watch::tests::{Scratch, wait_for_watcher};
    use std::collections::HashMap;
    use std::sync::Arc;
    use windows_sys::Win32::Foundation::{
        ERROR_ACCESS_DENIED, ERROR_NOTIFY_ENUM_DIR, ERROR_OPERATION_ABORTED, INVALID_HANDLE_VALUE,
    };

    #[test]
    fn a_completion_posted_by_a_call_that_failed_at_once_frees_nothing_still_in_flight() {
        let mut state = Loop {
            port: std::ptr::null_mut(),
            notices: Arc::new(Notices::default()),
            watches: HashMap::new(),
            keys: HashMap::new(),
            next_key: WAKE + 1,
        };
        // The oplock was granted, then the read failed at once and the handle was closed.
        let mut watch = Watch::open(r"C:\r".to_owned(), INVALID_HANDLE_VALUE);
        watch.oplocked = true;
        state.watches.insert(1, watch);
        let watch = state.watches.get_mut(&1).expect("watch");
        let (read, oplock) = (&raw mut watch.io.read, &raw mut watch.io.oplock);
        // The failed read's completion comes first: the cancelled oplock's is still to come.
        state.completed(1, read, 0, ERROR_OPERATION_ABORTED);
        assert!(
            state.watches.contains_key(&1),
            "the oplock's OVERLAPPED stays put"
        );
        state.completed(1, oplock, 0, ERROR_OPERATION_ABORTED);
        assert!(state.watches.is_empty());
    }

    #[test]
    fn an_overflow_loses_the_root_and_an_error_ends_it() {
        assert_eq!(
            read_outcome(ERROR_NOTIFY_ENUM_DIR, 0),
            ReadOutcome::Overflowed
        );
        assert_eq!(read_outcome(0, 0), ReadOutcome::Overflowed);
        assert_eq!(read_outcome(0, 24), ReadOutcome::Records);
        assert_eq!(read_outcome(ERROR_ACCESS_DENIED, 0), ReadOutcome::Failed);
    }

    #[test]
    fn records_are_parsed_with_slash_separated_paths() {
        let mut bytes = Vec::new();
        for (action, name, last) in [(3_u32, "src\\a.ts", false), (1, "b", true)] {
            let units: Vec<u16> = name.encode_utf16().collect();
            let length = 12 + units.len() * 2;
            let next = if last { 0 } else { length.next_multiple_of(4) };
            bytes.extend_from_slice(&(next as u32).to_le_bytes());
            bytes.extend_from_slice(&action.to_le_bytes());
            bytes.extend_from_slice(&((units.len() * 2) as u32).to_le_bytes());
            units
                .iter()
                .for_each(|unit| bytes.extend_from_slice(&unit.to_le_bytes()));
            bytes.resize(bytes.len().next_multiple_of(4), 0);
        }
        let mut seen = Vec::new();
        parse(&bytes, |path, renamed| seen.push((path, renamed)));
        assert_eq!(
            seen,
            vec![("src/a.ts".to_owned(), false), ("b".to_owned(), true)]
        );
    }

    #[test]
    fn network_and_wsl_paths_are_remote() {
        assert!(remote(r"\\server\share\repo"));
        assert!(remote(r"\\wsl$\Ubuntu\home\me\repo"));
        assert!(remote(r"\\wsl.localhost\Ubuntu\home\me\repo"));
        assert!(remote(r"\\?\UNC\server\share\repo"));
        assert!(!remote(&std::env::temp_dir().to_string_lossy()));
        let mut watcher = TreeWatcher::new();
        assert!(
            !watcher
                .watch(r"\\wsl$\Ubuntu\home".to_owned(), vec![String::new()])
                .unwrap()
        );
    }

    #[test]
    fn renaming_the_folder_above_a_watched_root_succeeds_and_ends_the_watch() {
        let scratch = Scratch::new("parent-rename");
        scratch.write("checkout/a.txt", "a");
        let parent = scratch.path("");
        let root = scratch.path("checkout").to_string_lossy().into_owned();
        let mut watcher = TreeWatcher::new();
        assert!(watcher.watch(root.clone(), vec![String::new()]).unwrap());
        let moved = parent.with_extension("moved");
        std::fs::rename(&parent, &moved).expect("the folder above a watched root renames");
        let notice = wait_for_watcher(&mut watcher, &root, |notice| notice.ended);
        assert!(notice.ended);
        std::fs::rename(&moved, &parent).expect("rename back");
        watcher.close().unwrap();
    }

    #[test]
    fn an_unwatched_root_can_be_removed_at_once() {
        let scratch = Scratch::new("unwatch-remove");
        scratch.write("checkout/a.txt", "a");
        let root = scratch.path("checkout");
        let mut watcher = TreeWatcher::new();
        let root_text = root.to_string_lossy().into_owned();
        assert!(
            watcher
                .watch(root_text.clone(), vec![String::new()])
                .unwrap()
        );
        watcher.unwatch(root_text).unwrap();
        std::fs::remove_dir_all(&root).expect("an unwatched root is removed");
        watcher.close().unwrap();
    }
}
