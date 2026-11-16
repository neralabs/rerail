//! Windows: one directory handle per watched root, read by `ReadDirectoryChangesW` over its whole
//! tree, every handle on one I/O completion port served by one thread, `rerail-tree-watch`.
//!
//! A plain handle on a folder makes renaming, moving or deleting it, or any folder above it, fail
//! ("access denied"). Each handle therefore holds a read-handle oplock on its folder, and the
//! thread acts on each break as what broke it needs:
//! - A change among the folder's entries (a file at its top written, created or deleted, a folder
//!   at its top changed) breaks it to none: the thread acknowledges the break, which the change
//!   waits for, and takes the oplock again; the handle and its read go on. (In that moment a
//!   rename of the folder or one above it would fail as it would with no oplock.) What cannot go
//!   at once (an acknowledgment while the last is in flight, an oplock not granted while another
//!   holder's break runs) is asked again `RENEW_RETRY` later, the handle and its read kept, up to
//!   `RENEWS` asks; then the handle is let go as for a rename, below.
//! - A rename or deletion of the folder or one above it breaks its handle part and waits for the
//!   handle: the thread closes it at once, which lets that go on, and opens the path again
//!   `REOPEN_DELAY` later. The same folder there (its volume and file id) is watched on and `lost`,
//!   since what changed while no handle was open is unknown; none there, or another folder, ends
//!   the watch. A root reopened `REOPENS` times within `REOPEN_WINDOW` ends at the next, so a
//!   folder renamed over and over is read whole rather than chased.
//! - A break that keeps the handle part is acknowledged at its new level, and the handle stays.
//!
//! A root the file system grants no oplock or file id on, a network path, a mapped remote drive
//! and a WSL path are not watched. An overflow of the 64 KiB buffer (`ERROR_NOTIFY_ENUM_DIR`, or a
//! read that returns nothing) makes the root `lost`.
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
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_IO_PENDING, ERROR_NOTIFY_ENUM_DIR, GetLastError, HANDLE,
    INVALID_HANDLE_VALUE, WAIT_TIMEOUT,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ACTION_MODIFIED, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OVERLAPPED,
    FILE_ID_INFO, FILE_LIST_DIRECTORY, FILE_NOTIFY_CHANGE_ATTRIBUTES, FILE_NOTIFY_CHANGE_CREATION,
    FILE_NOTIFY_CHANGE_DIR_NAME, FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE,
    FILE_NOTIFY_CHANGE_SIZE, FILE_NOTIFY_INFORMATION, FILE_SHARE_DELETE, FILE_SHARE_READ,
    FILE_SHARE_WRITE, FileIdInfo, GetDriveTypeW, GetFileInformationByHandleEx, GetVolumePathNameW,
    OPEN_EXISTING, ReadDirectoryChangesW,
};
use windows_sys::Win32::System::IO::{
    CreateIoCompletionPort, DeviceIoControl, GetQueuedCompletionStatus, OVERLAPPED,
    PostQueuedCompletionStatus,
};
use windows_sys::Win32::System::Ioctl::{
    FSCTL_REQUEST_OPLOCK, OPLOCK_LEVEL_CACHE_HANDLE, OPLOCK_LEVEL_CACHE_READ,
    REQUEST_OPLOCK_CURRENT_VERSION, REQUEST_OPLOCK_INPUT_BUFFER, REQUEST_OPLOCK_INPUT_FLAG_ACK,
    REQUEST_OPLOCK_INPUT_FLAG_REQUEST, REQUEST_OPLOCK_OUTPUT_BUFFER,
    REQUEST_OPLOCK_OUTPUT_FLAG_ACK_REQUIRED,
};
use windows_sys::Win32::System::Threading::INFINITE;

const BUFFER_BYTES: usize = 64 * 1024;
const FILTER: u32 = FILE_NOTIFY_CHANGE_FILE_NAME
    | FILE_NOTIFY_CHANGE_DIR_NAME
    | FILE_NOTIFY_CHANGE_SIZE
    | FILE_NOTIFY_CHANGE_LAST_WRITE
    | FILE_NOTIFY_CHANGE_CREATION
    | FILE_NOTIFY_CHANGE_ATTRIBUTES;
/// The oplock each handle holds: read and handle caching, the most a folder is granted.
const READ_HANDLE: u32 = OPLOCK_LEVEL_CACHE_READ | OPLOCK_LEVEL_CACHE_HANDLE;
/// `GetDriveTypeW`'s answer for a network drive.
const DRIVE_REMOTE: u32 = 4;
/// The completion key that wakes the thread for the host's calls; roots' keys start at 1.
const WAKE: usize = 0;
/// How long `close` waits for the cancelled reads of closed handles to complete.
const DRAIN_MILLISECONDS: u32 = 5_000;
/// How long after letting go of a root's handle the thread opens its path again: time for the
/// rename or deletion that broke the oplock to finish.
const REOPEN_DELAY: Duration = Duration::from_millis(200);
/// How soon the thread tries again to reopen a root whose closed handle's cancelled operations
/// have not completed yet.
const REOPEN_RETRY: Duration = Duration::from_millis(20);
/// How many reopens of one root `REOPEN_WINDOW` allows; the next ends its watch.
const REOPENS: usize = 8;
const REOPEN_WINDOW: Duration = Duration::from_secs(60);
/// How soon the thread asks again to take back an oplock after a break to none, when the
/// acknowledgment or the request could not go.
const RENEW_RETRY: Duration = Duration::from_millis(20);
/// How many times it asks; then it lets go of the handle and opens the path again.
const RENEWS: u32 = 10;

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
    /// The oplock held: its completion is the oplock's break.
    oplock: OVERLAPPED,
    oplock_in: REQUEST_OPLOCK_INPUT_BUFFER,
    oplock_out: REQUEST_OPLOCK_OUTPUT_BUFFER,
    /// The acknowledgment of a break to none, which holds no oplock.
    release: OVERLAPPED,
    release_in: REQUEST_OPLOCK_INPUT_BUFFER,
    release_out: REQUEST_OPLOCK_OUTPUT_BUFFER,
    /// `ReadDirectoryChangesW` wants a DWORD-aligned buffer.
    buffer: Vec<u32>,
}

impl Io {
    fn new(buffer_words: usize) -> Box<Self> {
        Box::new(Self {
            read: OVERLAPPED::default(),
            oplock: OVERLAPPED::default(),
            oplock_in: REQUEST_OPLOCK_INPUT_BUFFER::default(),
            oplock_out: REQUEST_OPLOCK_OUTPUT_BUFFER::default(),
            release: OVERLAPPED::default(),
            release_in: REQUEST_OPLOCK_INPUT_BUFFER::default(),
            release_out: REQUEST_OPLOCK_OUTPUT_BUFFER::default(),
            buffer: vec![0; buffer_words],
        })
    }
}

/// One of a watch's operations, told by its OVERLAPPED in [`Io`].
enum Operation {
    Read,
    Oplock,
    Release,
}

/// A folder as the file system knows it: a rename keeps it, a new folder at the path is another.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Identity {
    volume: u64,
    file: [u8; 16],
}

/// An oplock to take back after a break to none, asked for again at `at`.
#[derive(Debug, Clone, Copy)]
struct Renewal {
    at: Instant,
    /// The break's acknowledgment has still to go.
    acknowledge: bool,
    /// How many times it was asked for.
    tries: u32,
}

/// One root's handle and its operations: the read, the oplock and the acknowledgment of a break to
/// none. Each flag holds while that operation's completion is still to come, so its OVERLAPPED
/// stays put until then.
struct Watch {
    root: String,
    identity: Identity,
    handle: HANDLE,
    io: Box<Io>,
    reading: bool,
    oplocked: bool,
    releasing: bool,
    /// When the oplock a break to none took is to be asked for again.
    renewal: Option<Renewal>,
    /// When the handle, let go for a rename or deletion, is to be opened again.
    reopen_at: Option<Instant>,
    /// When the handle was opened again, within the last `REOPEN_WINDOW`.
    reopens: VecDeque<Instant>,
}

impl Watch {
    fn open(root: String, identity: Identity, handle: HANDLE) -> Self {
        Self {
            root,
            identity,
            handle,
            io: Io::new(BUFFER_BYTES / 4),
            reading: false,
            oplocked: false,
            releasing: false,
            renewal: None,
            reopen_at: None,
            reopens: VecDeque::new(),
        }
    }

    /// Whether a completion of any operation is still to come.
    fn pending(&self) -> bool {
        self.reading || self.oplocked || self.releasing
    }

    fn open_handle(&self) -> bool {
        self.handle != INVALID_HANDLE_VALUE
    }

    /// Which of this watch's operations `overlapped` belongs to, now no longer in flight; none for
    /// another OVERLAPPED or an operation that was not in flight.
    fn finished(&mut self, overlapped: *mut OVERLAPPED) -> Option<Operation> {
        let (operation, in_flight) = if std::ptr::eq(overlapped, &raw mut self.io.oplock) {
            (Operation::Oplock, &mut self.oplocked)
        } else if std::ptr::eq(overlapped, &raw mut self.io.read) {
            (Operation::Read, &mut self.reading)
        } else if std::ptr::eq(overlapped, &raw mut self.io.release) {
            (Operation::Release, &mut self.releasing)
        } else {
            return None;
        };
        std::mem::replace(in_flight, false).then_some(operation)
    }

    /// Binds the handle to `port` under `key`, takes the oplock and starts the first read; true
    /// when all three hold. Otherwise the handle is closed.
    fn start(&mut self, port: HANDLE, key: usize) -> bool {
        // SAFETY: this watch's handle, bound to the thread's port under its key.
        let bound = !unsafe { CreateIoCompletionPort(self.handle, port, key, 0) }.is_null();
        let started =
            bound && self.oplock(REQUEST_OPLOCK_INPUT_FLAG_REQUEST, READ_HANDLE) && self.read();
        if !started {
            self.close_handle();
        }
        started
    }

    /// Requests the oplock at `level` (`flags` `REQUEST_OPLOCK_INPUT_FLAG_REQUEST`) or
    /// acknowledges its break to `level` (`REQUEST_OPLOCK_INPUT_FLAG_ACK`), which holds it on; true
    /// when an oplock is then held (its completion is the next break).
    fn oplock(&mut self, flags: u32, level: u32) -> bool {
        let io = &mut *self.io;
        // SAFETY: the boxed buffers and OVERLAPPED stay put until the completion is dequeued.
        let done = unsafe {
            request_oplock(
                self.handle,
                flags,
                level,
                &mut io.oplock_in,
                &mut io.oplock_out,
                &mut io.oplock,
            )
        };
        self.oplocked = completion_comes(done);
        // One that finished at once holds no oplock.
        !done && self.oplocked
    }

    /// Acknowledges a break to none if it waits, then takes the oplock again, after `tries` asks.
    /// What cannot go now is asked again `RENEW_RETRY` after `now`, the handle and its read kept;
    /// false once `RENEWS` asks failed, or when a request finished at once, whose completion
    /// tells why and whose OVERLAPPED another request would share.
    fn renew(&mut self, acknowledge: bool, tries: u32, now: Instant) -> bool {
        self.renewal = None;
        let acknowledged = !acknowledge || self.release();
        if acknowledged && self.oplock(REQUEST_OPLOCK_INPUT_FLAG_REQUEST, READ_HANDLE) {
            return true;
        }
        if self.oplocked || tries + 1 >= RENEWS {
            return false;
        }
        self.renewal = Some(Renewal {
            at: now + RENEW_RETRY,
            acknowledge: !acknowledged,
            tries: tries + 1,
        });
        true
    }

    /// Lets go of the handle so whatever broke the oplock goes on; it is opened again
    /// `REOPEN_DELAY` after `now`.
    fn let_go(&mut self, now: Instant) {
        self.close_handle();
        self.renewal = None;
        self.reopen_at = Some(now + REOPEN_DELAY);
    }

    /// When the thread is next to act on this watch: a renewal, or a reopen.
    fn due(&self) -> Option<Instant> {
        self.renewal
            .map(|renewal| renewal.at)
            .into_iter()
            .chain(self.reopen_at)
            .min()
    }

    /// Acknowledges a break to none, which holds no oplock; true when it was taken.
    fn release(&mut self) -> bool {
        if self.releasing {
            return false;
        }
        let io = &mut *self.io;
        // SAFETY: the boxed buffers and OVERLAPPED stay put until the completion is dequeued.
        let done = unsafe {
            request_oplock(
                self.handle,
                REQUEST_OPLOCK_INPUT_FLAG_ACK,
                0,
                &mut io.release_in,
                &mut io.release_out,
                &mut io.release,
            )
        };
        self.releasing = completion_comes(done);
        self.releasing
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
            std::mem::forget(std::mem::replace(&mut self.io, Io::new(0)));
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

/// Calls `FSCTL_REQUEST_OPLOCK` on `handle` with `flags` and `level`; whether it finished at once.
///
/// # Safety
///
/// `input`, `output` and `overlapped` must stay put until the call's completion is dequeued.
unsafe fn request_oplock(
    handle: HANDLE,
    flags: u32,
    level: u32,
    input: &mut REQUEST_OPLOCK_INPUT_BUFFER,
    output: &mut REQUEST_OPLOCK_OUTPUT_BUFFER,
    overlapped: &mut OVERLAPPED,
) -> bool {
    *input = REQUEST_OPLOCK_INPUT_BUFFER {
        StructureVersion: REQUEST_OPLOCK_CURRENT_VERSION as u16,
        StructureLength: size_of::<REQUEST_OPLOCK_INPUT_BUFFER>() as u16,
        RequestedOplockLevel: level,
        Flags: flags,
    };
    *output = REQUEST_OPLOCK_OUTPUT_BUFFER {
        StructureVersion: REQUEST_OPLOCK_CURRENT_VERSION as u16,
        StructureLength: size_of::<REQUEST_OPLOCK_OUTPUT_BUFFER>() as u16,
        ..REQUEST_OPLOCK_OUTPUT_BUFFER::default()
    };
    // SAFETY: the caller keeps the buffers and OVERLAPPED put until the completion is dequeued.
    unsafe {
        DeviceIoControl(
            handle,
            FSCTL_REQUEST_OPLOCK,
            (&raw const *input).cast(),
            size_of::<REQUEST_OPLOCK_INPUT_BUFFER>() as u32,
            (&raw mut *output).cast(),
            size_of::<REQUEST_OPLOCK_OUTPUT_BUFFER>() as u32,
            std::ptr::null_mut(),
            overlapped,
        ) != 0
    }
}

/// What a broken oplock asks of its handle.
#[derive(Debug, PartialEq, Eq)]
enum Break {
    /// The handle part stays and the break waits: acknowledge it at this level.
    Acknowledge(u32),
    /// The folder's entries changed (a break to none): acknowledge the break if it waits, then
    /// take the oplock again.
    Renew { acknowledge: bool },
    /// The handle part goes, keeping the read part (the folder or one above it is renamed or
    /// deleted), or the oplock failed: close the handle.
    LetGo,
}

/// What the oplock's completion with `error` and `out` asks of the handle.
fn oplock_break(error: u32, out: &REQUEST_OPLOCK_OUTPUT_BUFFER) -> Break {
    let level = out.NewOplockLevel;
    let waits = out.Flags & REQUEST_OPLOCK_OUTPUT_FLAG_ACK_REQUIRED != 0;
    if error != 0 {
        Break::LetGo
    } else if level & OPLOCK_LEVEL_CACHE_HANDLE != 0 && waits {
        Break::Acknowledge(level)
    } else if level & OPLOCK_LEVEL_CACHE_HANDLE != 0 || level == 0 {
        Break::Renew { acknowledge: waits }
    } else {
        Break::LetGo
    }
}

/// Records a reopen at `now` in `reopens`; false when `REOPENS` were already made within
/// `REOPEN_WINDOW`.
fn may_reopen(reopens: &mut VecDeque<Instant>, now: Instant) -> bool {
    while reopens
        .front()
        .is_some_and(|at| now.duration_since(*at) >= REOPEN_WINDOW)
    {
        reopens.pop_front();
    }
    if reopens.len() >= REOPENS {
        return false;
    }
    reopens.push_back(now);
    true
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

/// A handle on the folder `root` for reading its changes, with the folder's identity, or none.
fn open_directory(root: &str) -> Option<(HANDLE, Identity)> {
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
    if handle == INVALID_HANDLE_VALUE {
        return None;
    }
    let mut info = FILE_ID_INFO::default();
    // SAFETY: an open handle and a buffer of the size passed.
    let identified = unsafe {
        GetFileInformationByHandleEx(
            handle,
            FileIdInfo,
            (&raw mut info).cast(),
            size_of::<FILE_ID_INFO>() as u32,
        )
    } != 0;
    if !identified {
        // SAFETY: the handle opened above, used nowhere else.
        unsafe { CloseHandle(handle) };
        return None;
    }
    let identity = Identity {
        volume: info.VolumeSerialNumber,
        file: info.FileId.Identifier,
    };
    Some((handle, identity))
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
        let Some((handle, identity)) = (!remote(&root)).then(|| open_directory(&root)).flatten()
        else {
            return false;
        };
        let key = self.next_key;
        self.next_key += 1;
        let mut watch = Watch::open(root, identity, handle);
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
        watch.renewal = None;
        watch.reopen_at = None;
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
    /// call that failed at once) is dropped: the watch goes only once none can still come, and not
    /// while its handle is to be opened again. An acknowledgment's completion asks nothing more.
    fn completed(&mut self, key: usize, overlapped: *mut OVERLAPPED, bytes: u32, error: u32) {
        match self.finished(key, overlapped) {
            Some(Operation::Oplock) => self.oplock_completed(key, error),
            Some(Operation::Read) => self.read_completed(key, bytes, error),
            Some(Operation::Release) | None => {}
        }
    }

    /// Which operation of `key` `overlapped` belongs to, now no longer in flight, while its handle
    /// is open; none for an operation not in flight, or one of a closed handle, whose watch goes
    /// here once no completion can still come and its handle is not to be opened again.
    fn finished(&mut self, key: usize, overlapped: *mut OVERLAPPED) -> Option<Operation> {
        let watch = self.watches.get_mut(&key)?;
        let operation = watch.finished(overlapped)?;
        if watch.open_handle() {
            return Some(operation);
        }
        if !watch.pending() && watch.reopen_at.is_none() {
            self.watches.remove(&key);
        }
        None
    }

    /// `key`'s oplock broke, or failed, with `error`: the handle keeps it as the break asks, or
    /// is let go.
    fn oplock_completed(&mut self, key: usize, error: u32) {
        let Some(watch) = self.watches.get_mut(&key) else {
            return;
        };
        let now = Instant::now();
        let kept = match oplock_break(error, &watch.io.oplock_out) {
            Break::Acknowledge(level) => watch.oplock(REQUEST_OPLOCK_INPUT_FLAG_ACK, level),
            Break::Renew { acknowledge } => watch.renew(acknowledge, 0, now),
            Break::LetGo => false,
        };
        if !kept {
            watch.let_go(now);
        }
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

    /// How long the thread may wait for a completion before an oplock is due to be asked for
    /// again or a handle to be opened again.
    fn wait_milliseconds(&self, now: Instant) -> u32 {
        self.watches
            .values()
            .filter_map(Watch::due)
            .min()
            .map_or(INFINITE, |at| {
                let wait = at.saturating_duration_since(now).as_millis();
                u32::try_from(wait).unwrap_or(INFINITE - 1)
            })
    }

    /// Asks again for each oplock due to be taken back after a break to none.
    fn renew_due(&mut self, now: Instant) {
        for watch in self.watches.values_mut() {
            let Some(renewal) = watch.renewal.filter(|renewal| renewal.at <= now) else {
                continue;
            };
            if !watch.renew(renewal.acknowledge, renewal.tries, now) {
                watch.let_go(now);
            }
        }
    }

    /// Opens again every handle due, once its closed handle's operations have all completed.
    fn reopen_due(&mut self, now: Instant) {
        let due: Vec<usize> = self
            .watches
            .iter()
            .filter(|(_, watch)| watch.reopen_at.is_some_and(|at| at <= now))
            .map(|(key, _)| *key)
            .collect();
        for key in due {
            self.reopen(key, now);
        }
    }

    /// Opens `key`'s root again: the same folder is watched on and lost; none there, another
    /// folder, or one reopen too many ends the watch.
    fn reopen(&mut self, key: usize, now: Instant) {
        let Some(watch) = self.watches.get_mut(&key) else {
            return;
        };
        if watch.pending() {
            // The closed handle's OVERLAPPEDs are still the kernel's.
            watch.reopen_at = Some(now + REOPEN_RETRY);
            return;
        }
        watch.reopen_at = None;
        if !may_reopen(&mut watch.reopens, now) {
            return self.end(key);
        }
        let Some((handle, identity)) = open_directory(&watch.root) else {
            return self.end(key);
        };
        if identity != watch.identity {
            // SAFETY: the handle opened above, used nowhere else.
            unsafe { CloseHandle(handle) };
            return self.end(key);
        }
        watch.handle = handle;
        if !watch.start(self.port, key) {
            return self.end(key);
        }
        self.notices.lost(&watch.root);
    }

    /// Closes every handle and waits for their cancelled operations, so nothing is freed under the
    /// kernel.
    fn shut(&mut self) {
        let keys: Vec<usize> = self.watches.keys().copied().collect();
        for key in keys {
            self.close(key);
        }
        while !self.watches.is_empty() {
            let Dequeued::Completion(key, overlapped, bytes, error) =
                dequeue(self.port, DRAIN_MILLISECONDS)
            else {
                return;
            };
            if !overlapped.is_null() {
                self.completed(key, overlapped, bytes, error);
            }
        }
    }
}

/// What waiting on the port gave.
enum Dequeued {
    /// A completion: its key, OVERLAPPED (null for a wake), bytes and error.
    Completion(usize, *mut OVERLAPPED, u32, u32),
    /// None came in time.
    TimedOut,
    /// The port failed.
    Failed,
}

/// The next completion on `port`, waiting at most `milliseconds`.
fn dequeue(port: HANDLE, milliseconds: u32) -> Dequeued {
    let (mut bytes, mut key, mut overlapped) = (0_u32, 0_usize, std::ptr::null_mut());
    // SAFETY: the port lives until the thread is joined; the out-parameters are locals.
    let done = unsafe {
        GetQueuedCompletionStatus(port, &mut bytes, &mut key, &mut overlapped, milliseconds)
    };
    if done != 0 {
        return Dequeued::Completion(key, overlapped, bytes, 0);
    }
    // SAFETY: reads the calling thread's last error.
    let error = unsafe { GetLastError() };
    match (overlapped.is_null(), error) {
        (false, _) => Dequeued::Completion(key, overlapped, bytes, error),
        (true, WAIT_TIMEOUT) => Dequeued::TimedOut,
        (true, _) => Dequeued::Failed,
    }
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
        match dequeue(shared.port, state.wait_milliseconds(Instant::now())) {
            Dequeued::Completion(key, overlapped, bytes, error) => {
                if !overlapped.is_null() {
                    state.completed(key, overlapped, bytes, error);
                } else if key == WAKE && !run_commands(shared, state) {
                    return;
                }
            }
            Dequeued::TimedOut => {}
            Dequeued::Failed => return,
        }
        let now = Instant::now();
        state.renew_due(now);
        state.reopen_due(now);
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
    use super::{
        Break, Dequeued, Identity, Loop, READ_HANDLE, RENEWS, REOPEN_WINDOW, REOPENS, ReadOutcome,
        WAKE, Watch, dequeue, may_reopen, open_directory, oplock_break, parse, read_outcome,
        remote,
    };
    use crate::tree_watch::TreeWatcher;
    use crate::tree_watch::notices::Notices;
    use crate::tree_watch::tests::{Scratch, names, wait_for_watcher};
    use std::collections::{HashMap, VecDeque};
    use std::sync::Arc;
    use std::time::{Duration, Instant};
    use windows_sys::Win32::Foundation::{
        CloseHandle, ERROR_ACCESS_DENIED, ERROR_NOTIFY_ENUM_DIR, ERROR_OPERATION_ABORTED, HANDLE,
        INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::System::IO::CreateIoCompletionPort;
    use windows_sys::Win32::System::Ioctl::{
        OPLOCK_LEVEL_CACHE_HANDLE, OPLOCK_LEVEL_CACHE_READ, REQUEST_OPLOCK_INPUT_FLAG_REQUEST,
        REQUEST_OPLOCK_OUTPUT_BUFFER, REQUEST_OPLOCK_OUTPUT_FLAG_ACK_REQUIRED,
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
        let mut watch = Watch::open(
            r"C:\r".to_owned(),
            Identity::default(),
            INVALID_HANDLE_VALUE,
        );
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
    fn a_break_that_takes_the_handle_part_lets_it_go_and_one_to_none_renews_the_oplock() {
        let broken = |new, flags| REQUEST_OPLOCK_OUTPUT_BUFFER {
            OriginalOplockLevel: READ_HANDLE,
            NewOplockLevel: new,
            Flags: flags,
            ..REQUEST_OPLOCK_OUTPUT_BUFFER::default()
        };
        let waits = REQUEST_OPLOCK_OUTPUT_FLAG_ACK_REQUIRED;
        // A change among the folder's entries.
        assert_eq!(
            oplock_break(0, &broken(0, waits)),
            Break::Renew { acknowledge: true }
        );
        assert_eq!(
            oplock_break(0, &broken(0, 0)),
            Break::Renew { acknowledge: false }
        );
        // A rename or deletion of the folder or one above it.
        assert_eq!(
            oplock_break(0, &broken(OPLOCK_LEVEL_CACHE_READ, waits)),
            Break::LetGo
        );
        assert_eq!(
            oplock_break(0, &broken(READ_HANDLE, waits)),
            Break::Acknowledge(OPLOCK_LEVEL_CACHE_READ | OPLOCK_LEVEL_CACHE_HANDLE)
        );
        assert_eq!(
            oplock_break(ERROR_ACCESS_DENIED, &broken(0, 0)),
            Break::LetGo
        );
    }

    #[test]
    fn a_break_to_none_that_cannot_renew_yet_keeps_the_handle_until_the_bound() {
        let scratch = Scratch::new("renew-retry");
        scratch.write("checkout/a.txt", "a");
        let root = scratch.path("checkout").to_string_lossy().into_owned();
        let (handle, identity) = open_directory(&root).expect("open the root");
        let mut watch = Watch::open(root.clone(), identity, handle);
        // A break to none that waits for its acknowledgment while the last one is still in flight.
        watch.releasing = true;
        watch.io.oplock_out.Flags = REQUEST_OPLOCK_OUTPUT_FLAG_ACK_REQUIRED;
        let mut state = Loop {
            port: std::ptr::null_mut(),
            notices: Arc::new(Notices::default()),
            watches: HashMap::from([(1, watch)]),
            keys: HashMap::from([(root, 1)]),
            next_key: 2,
        };
        state.oplock_completed(1, 0);
        for asked in 1..RENEWS {
            let watch = &state.watches[&1];
            assert!(
                watch.open_handle() && watch.reopen_at.is_none(),
                "the handle and its read stay after {asked} asks"
            );
            let renewal = watch.renewal.expect("asked for again");
            assert!(renewal.acknowledge);
            state.renew_due(renewal.at);
        }
        let watch = &state.watches[&1];
        assert!(!watch.open_handle() && watch.renewal.is_none() && watch.reopen_at.is_some());
        assert!(watch.reopens.is_empty(), "no reopen counted yet");
    }

    #[test]
    fn a_root_is_reopened_at_most_the_bound_times_within_the_window() {
        let mut reopens = VecDeque::new();
        let start = Instant::now();
        for index in 0..REOPENS {
            assert!(may_reopen(
                &mut reopens,
                start + Duration::from_secs(index as u64)
            ));
        }
        assert!(!may_reopen(
            &mut reopens,
            start + Duration::from_secs(REOPENS as u64)
        ));
        assert!(may_reopen(&mut reopens, start + REOPEN_WINDOW));
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

    /// A watch of `scratch`'s `base/checkout` holding the oplock, its handle on a port of its own.
    fn oplocked(scratch: &Scratch) -> (HANDLE, Watch) {
        let root = scratch.path("base/checkout").to_string_lossy().into_owned();
        // SAFETY: a new port, closed by the caller.
        let port =
            unsafe { CreateIoCompletionPort(INVALID_HANDLE_VALUE, std::ptr::null_mut(), 0, 1) };
        let (handle, identity) = open_directory(&root).expect("open the root");
        let mut watch = Watch::open(root, identity, handle);
        // SAFETY: the watch's handle, bound to the port above.
        unsafe { CreateIoCompletionPort(handle, port, 1, 0) };
        assert!(watch.oplock(REQUEST_OPLOCK_INPUT_FLAG_REQUEST, READ_HANDLE));
        (port, watch)
    }

    /// What `change` (run on a thread of its own, as it may wait for the break) asks of `watch`'s
    /// oplock, with the levels and flags it broke with; none within a few seconds.
    fn break_of(
        port: HANDLE,
        watch: &mut Watch,
        change: impl FnOnce() + Send,
    ) -> Option<(Break, String)> {
        std::thread::scope(|scope| {
            let changing = scope.spawn(change);
            let started = Instant::now();
            let mut broke = None;
            while broke.is_none() && started.elapsed() < Duration::from_secs(5) {
                let Dequeued::Completion(_, overlapped, _, error) = dequeue(port, 100) else {
                    continue;
                };
                if std::ptr::eq(overlapped, &raw mut watch.io.release) {
                    watch.releasing = false;
                } else if std::ptr::eq(overlapped, &raw mut watch.io.oplock) {
                    watch.oplocked = false;
                    let out = &watch.io.oplock_out;
                    let levels = format!(
                        "{} to {}, flags {:#x}",
                        out.OriginalOplockLevel, out.NewOplockLevel, out.Flags
                    );
                    broke = Some((oplock_break(error, out), levels));
                }
            }
            if !matches!(broke, Some((Break::Renew { .. }, _))) {
                // Let a change that waits for the handle go on.
                watch.close_handle();
            }
            changing.join().expect("change");
            broke
        })
    }

    /// The adapter's answer to each break rests on what Windows breaks a folder's oplock with: a
    /// change among its entries breaks it to none, which an acknowledgment and a new request
    /// renew on the same handle; a rename of the folder above breaks its handle part.
    #[test]
    fn a_change_among_a_folders_entries_renews_its_oplock_and_a_rename_above_lets_it_go() {
        let scratch = Scratch::new("oplock-break");
        scratch.write("base/checkout/a.txt", "a");
        scratch.write("base/checkout/src/a.txt", "a");
        let (port, mut watch) = oplocked(&scratch);
        for round in 0..2 {
            let written = break_of(port, &mut watch, || {
                scratch.write("base/checkout/a.txt", &format!("written {round}"));
            });
            let Some((Break::Renew { acknowledge }, levels)) = written else {
                panic!("a write at the top: {written:?}");
            };
            assert!(
                (!acknowledge || watch.release())
                    && watch.oplock(REQUEST_OPLOCK_INPUT_FLAG_REQUEST, READ_HANDLE),
                "the oplock renews after a break of {levels}"
            );
        }
        let renamed = break_of(port, &mut watch, || {
            std::fs::rename(scratch.path("base"), scratch.path("moved")).expect("rename");
        });
        assert!(
            matches!(renamed, Some((Break::LetGo, _))),
            "a rename above: {renamed:?}"
        );
        // SAFETY: the port `oplocked` created; its one handle is closed.
        unsafe { CloseHandle(port) };
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

    /// Renames `scratch`'s `checkout` away and back, then waits for the notice it makes.
    fn rename_away_and_back(watcher: &mut TreeWatcher, scratch: &Scratch, root: &str) -> bool {
        std::fs::rename(scratch.path("checkout"), scratch.path("moved")).expect("rename away");
        std::fs::rename(scratch.path("moved"), scratch.path("checkout")).expect("rename back");
        let notice = wait_for_watcher(watcher, root, |notice| notice.lost || notice.ended);
        assert!(notice.lost != notice.ended, "{notice:?}");
        notice.ended
    }

    #[test]
    fn a_root_renamed_away_and_back_before_it_is_opened_again_is_lost_and_still_watched() {
        let scratch = Scratch::new("rename-back");
        scratch.write("checkout/src/a.txt", "a");
        let root = scratch.path("checkout").to_string_lossy().into_owned();
        let mut watcher = TreeWatcher::new();
        assert!(watcher.watch(root.clone(), vec![String::new()]).unwrap());
        assert!(!rename_away_and_back(&mut watcher, &scratch, &root));
        assert_eq!(watcher.held_roots(), 1);
        scratch.write("checkout/src/a.txt", "b");
        let notice = wait_for_watcher(&mut watcher, &root, |notice| {
            names(&notice.changed, "src/a.txt")
        });
        assert!(!notice.ended && !notice.lost, "{notice:?}");
        watcher.close().unwrap();
    }

    #[test]
    fn a_root_renamed_away_and_back_more_often_than_the_bound_ends() {
        let scratch = Scratch::new("rename-bound");
        scratch.write("checkout/a.txt", "a");
        let root = scratch.path("checkout").to_string_lossy().into_owned();
        let mut watcher = TreeWatcher::new();
        assert!(watcher.watch(root.clone(), vec![String::new()]).unwrap());
        for _ in 0..REOPENS {
            assert!(!rename_away_and_back(&mut watcher, &scratch, &root));
        }
        assert!(rename_away_and_back(&mut watcher, &scratch, &root));
        assert_eq!(watcher.held_roots(), 0);
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
