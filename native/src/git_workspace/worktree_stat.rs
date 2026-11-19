//! Whether a clean worktree file still holds the bytes Git compared with its blob, without a read:
//! its stat against the stat Git recorded and, on Windows, NTFS's change time, which that stat
//! lacks; and, once a read hashed a file's bytes equal to its blob after its change time moved,
//! what restores remember of it in the content store.

use super::RESTORE_STAT;
use crate::content_store::{ContentStore, HashKey, VerifiedWorktreeFile};
use std::collections::HashMap;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// How far a change time can trail the write it records, in 100 ns units: NTFS and ReFS stamp a
/// write with the system time of the last clock tick, at most 15.6 ms old. A change time older
/// than this before a read began cannot be the stamp of a write after that read.
const CHANGE_TIME_LAG: i64 = 10_000_000;

/// A worktree file's stat, as `lstat` gives it without following a link, with what this system
/// keeps beside it.
pub(super) struct WorktreeStat {
    pub(super) metadata: gix::index::fs::Metadata,
    change: platform::Change,
}

impl WorktreeStat {
    pub(super) fn read(path: &Path) -> Option<Self> {
        let (metadata, change) = platform::read(path)?;
        Some(Self { metadata, change })
    }

    /// Whether the stat alone equals `recorded`, as Git compares them (`RESTORE_STAT`).
    pub(super) fn matches(&self, recorded: &gix::index::entry::Stat) -> bool {
        gix::index::entry::Stat::from_fs(&self.metadata)
            .is_ok_and(|current| recorded.matches(&current, RESTORE_STAT))
    }

    /// Whether the file still holds the bytes Git compared with the blob when it recorded
    /// `recorded`, with no read: it has exactly that stat and nothing changed it after its last
    /// write.
    #[cfg(test)]
    pub(super) fn proves(&self, recorded: &gix::index::entry::Stat) -> bool {
        self.change.unchanged_since_written() && self.matches(recorded)
    }

    /// The file's identity and change time, where this system keeps them.
    #[cfg(all(test, windows))]
    pub(super) fn stamp(&self) -> Option<ChangeStamp> {
        self.change.stamp()
    }

    /// The stat of the file at `path`, with `times` (change, then modification) and `attributes`
    /// in place of its own: what a volume or a provider these cases cannot make gives.
    #[cfg(all(test, windows))]
    pub(super) fn as_if(path: &Path, times: Option<(i64, i64)>, attributes: u32) -> Self {
        let (metadata, change) = platform::read(path).unwrap();
        let change = change.as_if(times, attributes);
        Self { metadata, change }
    }
}

/// How many queries beyond the stat's own call this thread's proofs made since the last call: the
/// file id, the volume's protocol and its file system.
#[cfg(all(test, windows))]
pub(super) fn queries_made() -> usize {
    platform::QUERIES.replace(0)
}

/// A file's identity and change time: every write through any of its names, every time set and
/// every change of its permissions moves the change time, and a file a rename put in its place has
/// another identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ChangeStamp {
    volume: u64,
    file_id: [u8; 16],
    pub(super) changed: i64,
}

impl ChangeStamp {
    fn of(file: &VerifiedWorktreeFile) -> Self {
        Self {
            volume: file.volume,
            file_id: file.file_id,
            changed: file.changed,
        }
    }

    /// Whether the change time is older than `read_from`, when a read of the file began, by more
    /// than `CHANGE_TIME_LAG`: a write after that read then moves it.
    fn older_than(&self, read_from: SystemTime) -> bool {
        file_time(read_from).is_some_and(|read_from| self.changed < read_from - CHANGE_TIME_LAG)
    }
}

/// `time` in Windows file time units: 100 ns intervals since 1601.
fn file_time(time: SystemTime) -> Option<i64> {
    let since_1970 = time.duration_since(UNIX_EPOCH).ok()?;
    i64::try_from(since_1970.as_nanos() / 100)
        .ok()?
        .checked_add(116_444_736_000_000_000)
}

/// How a clean file whose stat equals the stat Git recorded is shown to hold its blob's bytes.
pub(super) enum Proof {
    /// Its stat, its change time or what a restore remembered shows it.
    Held,
    /// Only a read can tell: a hash equal to the blob is remembered under the stamp, if any.
    Read(Option<ChangeStamp>),
}

/// What restores remember of a worktree's clean files whose change time moved after their last
/// write: the content store's rows, read when the first such file needs them and written once by
/// `save`. A restore where no change time moved reads and writes no row.
pub struct VerifiedFiles<'a> {
    store: Option<&'a ContentStore>,
    worktree: String,
    known: Option<HashMap<String, VerifiedWorktreeFile>>,
    learned: Vec<VerifiedWorktreeFile>,
    /// Whether each volume, by its serial number, keeps a change time to trust: asked once per
    /// volume, since every file on it gives the same answer.
    volumes: HashMap<u64, bool>,
}

impl<'a> VerifiedFiles<'a> {
    /// The memory of the worktree whose root is `worktree`, kept in `store`.
    pub(super) fn new(store: Option<&'a ContentStore>, worktree: &Path) -> Self {
        Self {
            store,
            worktree: worktree.to_string_lossy().into_owned(),
            known: None,
            learned: Vec::new(),
            volumes: HashMap::new(),
        }
    }

    /// How a file whose stat equals the stat Git recorded is shown to hold `blob`'s bytes. Where
    /// the volume keeps no change time to trust (FAT, exFAT, a network share, a file that refused
    /// the handle), or the file's data is not all on this disk, the stat decides, as it does for
    /// Git.
    pub(super) fn proof(&mut self, path: &str, blob: HashKey, current: &WorktreeStat) -> Proof {
        let change = &current.change;
        if change.unchanged_since_written() || change.leaves_the_stat_to_decide() {
            return Proof::Held;
        }
        let Some(stamp) = change.stamp() else {
            // No identity to remember the file by: where change times count, a read decides.
            return if change.volume_keeps_change_times() {
                Proof::Read(None)
            } else {
                Proof::Held
            };
        };
        if !self.trusts(stamp.volume, change) || self.remembers(path, &stamp, blob) {
            return Proof::Held;
        }
        Proof::Read(Some(stamp))
    }

    /// Whether the volume whose serial number is `volume` keeps a change time to trust.
    fn trusts(&mut self, volume: u64, change: &platform::Change) -> bool {
        *self
            .volumes
            .entry(volume)
            .or_insert_with(|| change.volume_keeps_change_times())
    }

    /// Remembers that the file at `path` held `blob`'s bytes under `stamp`, read from `read_from`
    /// on, where its change time is older than that read by more than `CHANGE_TIME_LAG`.
    pub(super) fn learn(
        &mut self,
        path: &str,
        stamp: Option<ChangeStamp>,
        blob: HashKey,
        read_from: SystemTime,
    ) {
        let Some(stamp) = stamp.filter(|stamp| stamp.older_than(read_from)) else {
            return;
        };
        let file = VerifiedWorktreeFile {
            path: path.to_owned(),
            volume: stamp.volume,
            file_id: stamp.file_id,
            changed: stamp.changed,
            blob,
        };
        self.known().insert(path.to_owned(), file.clone());
        self.learned.push(file);
    }

    /// How many files this memory learned: each one a read whose hash equalled its blob.
    #[cfg(all(test, windows))]
    pub(super) fn learned(&self) -> usize {
        self.learned.len()
    }

    /// Writes what this restore learned to the store, in one write that never waits: a busy store
    /// keeps nothing, which costs the next restore a read of each file.
    pub fn save(self) {
        let (Some(store), false) = (self.store, self.learned.is_empty()) else {
            return;
        };
        let Ok(now) = crate::content_store::unix_seconds() else {
            return;
        };
        let _ = store.remember_verified_worktree_files(&self.worktree, &self.learned, now);
    }

    fn remembers(&mut self, path: &str, stamp: &ChangeStamp, blob: HashKey) -> bool {
        self.known()
            .get(path)
            .is_some_and(|file| file.blob == blob && ChangeStamp::of(file) == *stamp)
    }

    /// The remembered files, read from the store when first needed; a store that cannot be read
    /// remembers none, which costs a read of each file.
    fn known(&mut self) -> &mut HashMap<String, VerifiedWorktreeFile> {
        let (store, worktree) = (self.store, &self.worktree);
        self.known.get_or_insert_with(|| {
            store
                .and_then(|store| store.verified_worktree_files(worktree).ok())
                .unwrap_or_default()
                .into_iter()
                .map(|file| (file.path.clone(), file))
                .collect()
        })
    }
}

/// Whether a clean file's stat, equal to the stat Git recorded, decides where its read failed:
/// for every reason but the file being gone or grown past its blob's length, which are facts
/// about its bytes. Its permissions, another process holding or locking it, a cloud placeholder
/// its provider cannot fetch, a drive not ready: the stat's answer stands, as it does for Git.
pub(super) fn stat_decides(error: &std::io::Error) -> bool {
    !matches!(
        error.kind(),
        std::io::ErrorKind::NotFound | std::io::ErrorKind::FileTooLarge
    )
}

/// Whether a volume whose file system is `name` keeps a change time that every write moves, at
/// 100 ns: NTFS and ReFS do; FAT and exFAT keep none.
#[cfg(any(windows, test))]
fn keeps_change_times(name: &str) -> bool {
    matches!(name, "NTFS" | "ReFS")
}

/// The attributes of a file whose data is not all on this disk: a cloud file placeholder, whose
/// provider downloads its data when it is read (`FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS`) or opened
/// (`FILE_ATTRIBUTE_RECALL_ON_OPEN`), and a file moved to offline storage
/// (`FILE_ATTRIBUTE_OFFLINE`).
#[cfg(windows)]
const NOT_ALL_ON_DISK: u32 = 0x0040_0000 | 0x0004_0000 | 0x0000_1000;

/// Whether a file with these attributes has data a read would bring back to this disk. Freeing a
/// placeholder's space moves its change time and keeps its stat, so its stat decides, as it does
/// for Git: a read would download every freed file.
#[cfg(windows)]
pub(crate) fn recalls_data(attributes: u32) -> bool {
    attributes & NOT_ALL_ON_DISK != 0
}

#[cfg(not(windows))]
mod platform {
    use super::ChangeStamp;
    use std::path::Path;

    /// The Unix stat holds the change time, which every write moves, so nothing is kept beside it.
    pub(super) struct Change;

    pub(super) fn read(path: &Path) -> Option<(gix::index::fs::Metadata, Change)> {
        Some((
            gix::index::fs::Metadata::from_path_no_follow(path).ok()?,
            Change,
        ))
    }

    impl Change {
        pub(super) fn unchanged_since_written(&self) -> bool {
            true
        }

        pub(super) fn leaves_the_stat_to_decide(&self) -> bool {
            true
        }

        pub(super) fn volume_keeps_change_times(&self) -> bool {
            false
        }

        pub(super) fn stamp(&self) -> Option<ChangeStamp> {
            None
        }
    }
}

#[cfg(windows)]
mod platform {
    use super::{ChangeStamp, keeps_change_times};
    use std::fs::File;
    use std::os::windows::io::AsRawHandle;
    use std::path::Path;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_BASIC_INFO, FILE_ID_INFO, FileBasicInfo, FileIdInfo, FileRemoteProtocolInfo,
        GetFileInformationByHandleEx, GetVolumeInformationByHandleW,
    };

    /// NTFS's change time beside the stat, and the attributes handle that answers the file's
    /// identity and volume. The Windows stat holds the creation time where Unix has the change
    /// time (Git for Windows records the same), so a write that keeps the size and puts the
    /// modification time back leaves the stat as it was; the change time, which every write and
    /// every time set moves, shows it.
    pub(super) struct Change {
        file: Option<File>,
        /// The change time and the modification time, where the handle answered them.
        times: Option<(i64, i64)>,
        /// The file's attributes, as the same call answered them; none where it did not.
        attributes: u32,
    }

    /// One handle answers the stat and the change time, opened as `lstat` opens a file: a link is
    /// not followed. A denial of the file's read also refuses this handle, so there `lstat` gives
    /// the stat alone, as Git for Windows reads it, and nothing proves the file unchanged since
    /// its last write.
    pub(super) fn read(path: &Path) -> Option<(gix::index::fs::Metadata, Change)> {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES,
        };
        let Ok(file) = std::fs::OpenOptions::new()
            .access_mode(FILE_READ_ATTRIBUTES)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
            .open(path)
        else {
            let metadata = gix::index::fs::Metadata::from_path_no_follow(path).ok()?;
            let change = Change {
                file: None,
                times: None,
                attributes: 0,
            };
            return Some((metadata, change));
        };
        let metadata = gix::index::fs::Metadata::from_file(&file).ok()?;
        let basic = basic_info(&file);
        let times = basic.map(|info| (info.ChangeTime, info.LastWriteTime));
        let attributes = basic.map_or(0, |info| info.FileAttributes);
        let file = Some(file);
        Some((
            metadata,
            Change {
                file,
                times,
                attributes,
            },
        ))
    }

    impl Change {
        /// Whether NTFS's change time equals the modification time: false once anything wrote
        /// the file or set one of its times after that modification time, and where the handle
        /// did not answer.
        pub(super) fn unchanged_since_written(&self) -> bool {
            self.times
                .is_some_and(|(changed, written)| changed == written)
        }

        /// Whether what the stat's own call answered leaves the stat to decide, with no other
        /// query: the file's data is not all on this disk (`recalls_data`).
        pub(super) fn leaves_the_stat_to_decide(&self) -> bool {
            super::recalls_data(self.attributes)
        }

        /// Whether the file's volume is on this machine and keeps a change time every write
        /// moves: a network share's change time is another machine's clock, so no read here can
        /// tell a later write from an earlier one.
        pub(super) fn volume_keeps_change_times(&self) -> bool {
            self.file
                .as_ref()
                .is_some_and(|file| is_local(file) && keeps_change_times(&file_system(file)))
        }

        /// The file's identity and change time, where the handle answers both.
        pub(super) fn stamp(&self) -> Option<ChangeStamp> {
            let (changed, _) = self.times?;
            let (volume, file_id) = identity(self.file.as_ref()?)?;
            Some(ChangeStamp {
                volume,
                file_id,
                changed,
            })
        }
    }

    #[cfg(test)]
    impl Change {
        /// This change with `times` and `attributes` in place of what the file gave.
        pub(super) fn as_if(self, times: Option<(i64, i64)>, attributes: u32) -> Self {
            Self {
                times,
                attributes,
                ..self
            }
        }
    }

    #[cfg(test)]
    thread_local! {
        /// The queries beyond the stat's own call this thread made (`super::queries_made`).
        pub(super) static QUERIES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }

    #[cfg(test)]
    fn queried() {
        QUERIES.set(QUERIES.get() + 1);
    }

    /// The times and attributes NTFS keeps for `file`.
    fn basic_info(file: &File) -> Option<FILE_BASIC_INFO> {
        let mut info = FILE_BASIC_INFO::default();
        // SAFETY: an open handle, and a buffer of the size given that lives across the call.
        let read = unsafe {
            GetFileInformationByHandleEx(
                file.as_raw_handle(),
                FileBasicInfo,
                (&raw mut info).cast(),
                std::mem::size_of::<FILE_BASIC_INFO>() as u32,
            )
        };
        (read != 0).then_some(info)
    }

    /// The serial number of `file`'s volume and its 128-bit id there, as NTFS and ReFS give them.
    fn identity(file: &File) -> Option<(u64, [u8; 16])> {
        #[cfg(test)]
        queried();
        let mut info = FILE_ID_INFO::default();
        // SAFETY: an open handle, and a buffer of the size given that lives across the call.
        let read = unsafe {
            GetFileInformationByHandleEx(
                file.as_raw_handle(),
                FileIdInfo,
                (&raw mut info).cast(),
                std::mem::size_of::<FILE_ID_INFO>() as u32,
            )
        };
        (read != 0).then_some((info.VolumeSerialNumber, info.FileId.Identifier))
    }

    /// Whether `file` is on this machine: only a file on a network share answers the protocol it
    /// is reached by.
    fn is_local(file: &File) -> bool {
        #[cfg(test)]
        queried();
        // Larger than `FILE_REMOTE_PROTOCOL_INFO`, and aligned as it is.
        let mut info = [0_u64; 32];
        // SAFETY: an open handle, and a buffer of the size given that lives across the call.
        let remote = unsafe {
            GetFileInformationByHandleEx(
                file.as_raw_handle(),
                FileRemoteProtocolInfo,
                info.as_mut_ptr().cast(),
                std::mem::size_of_val(&info) as u32,
            )
        };
        remote == 0
    }

    /// The name of the file system that holds `file`, such as `NTFS`; empty where it is not told.
    fn file_system(file: &File) -> String {
        #[cfg(test)]
        queried();
        let mut name = [0_u16; 32];
        // SAFETY: an open handle; every other out-parameter is null, and the name buffer has the
        // length given.
        let read = unsafe {
            GetVolumeInformationByHandleW(
                file.as_raw_handle(),
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                name.as_mut_ptr(),
                name.len() as u32,
            )
        };
        let length = name
            .iter()
            .position(|&unit| unit == 0)
            .unwrap_or(name.len());
        if read == 0 {
            return String::new();
        }
        String::from_utf16_lossy(&name[..length])
    }
}

#[cfg(test)]
mod tests {
    use super::{keeps_change_times, stat_decides};
    use std::io::{Error, ErrorKind};

    /// A read that failed for any reason but the file being gone or grown leaves the stat's
    /// answer: its permissions, another process holding or locking it (Windows' sharing and lock
    /// violations, 32 and 33), a cloud placeholder whose provider is not running (362) or cannot
    /// fetch it offline (389), a drive not ready (21).
    #[test]
    fn a_read_error_leaves_the_stat_to_decide_unless_the_file_is_gone_or_grown() {
        for error in [
            Error::from(ErrorKind::PermissionDenied),
            Error::from_raw_os_error(32),
            Error::from_raw_os_error(33),
            Error::from_raw_os_error(362),
            Error::from_raw_os_error(389),
            Error::from_raw_os_error(21),
            Error::other("a cloud file the provider cannot fetch"),
        ] {
            assert!(stat_decides(&error), "{error}");
        }
        for error in [
            Error::from(ErrorKind::NotFound),
            Error::from(ErrorKind::FileTooLarge),
        ] {
            assert!(!stat_decides(&error), "{error}");
        }
    }

    /// NTFS and ReFS keep a change time every write moves; FAT and exFAT keep none, so their
    /// clean files keep the stat's answer, as Git gives it.
    #[test]
    fn only_ntfs_and_refs_keep_a_change_time_to_trust() {
        assert!(keeps_change_times("NTFS"));
        assert!(keeps_change_times("ReFS"));
        for name in ["FAT", "FAT32", "exFAT", "CDFS", "UDF", ""] {
            assert!(!keeps_change_times(name), "{name}");
        }
    }
}
