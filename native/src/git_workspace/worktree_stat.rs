//! Whether a clean worktree file still holds the bytes Git compared with its blob, without a read:
//! its stat against the stat Git recorded and, on Windows, NTFS's change time, which that stat
//! lacks; and, once a read hashed a file's bytes equal to its blob after its change time moved,
//! what restores remember of it in the content store.

use super::RESTORE_STAT;
use crate::content_store::{ContentStore, HashKey, VerifiedWorktreeFile, worktree_path_key};
use std::collections::HashMap;
use std::path::Path;

/// A reading of the clock this system stamps a file's changes with, in 100 ns units since 1601:
/// every change after the reading gets a change time no earlier than it. On Windows the system
/// time as of the clock's last tick (`GetSystemTimeAsFileTime`), which NTFS and ReFS stamp a
/// change with, so changes within one tick (up to 15.6 ms) share one change time. `None` where no
/// change time is kept beside the stat (`WorktreeStat`): no stamp is compared with a reading.
pub fn file_clock() -> Option<i64> {
    platform::file_clock()
}

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
        let change = change.with_facts(times, attributes);
        Self { metadata, change }
    }
}

/// How many queries beyond the stat's own call this thread's proofs made since the last call: the
/// file id, the volume's protocol and its file system.
#[cfg(all(test, windows))]
pub(super) fn queries_made() -> usize {
    platform::QUERIES.replace(0)
}

/// Makes the file id query answer nothing on this thread, as a volume that gives no file id, or
/// answer again.
#[cfg(all(test, windows))]
pub(super) fn file_ids_answer(answer: bool) {
    platform::NO_FILE_ID.set(!answer);
}

/// Makes every file this thread's proofs ask about answer as one on a network share, or as one on
/// this machine again.
#[cfg(all(test, windows))]
pub(super) fn files_are_on_a_share(answer: bool) {
    platform::ON_A_SHARE.set(answer);
}

/// Makes the file clock read `reading` on this thread, as on a tick these cases cannot time, or
/// read the system's clock again (`None`).
#[cfg(all(test, windows))]
pub(super) fn file_clock_reads(reading: Option<i64>) {
    platform::CLOCK.set(reading);
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

    /// Whether every change after the file clock read `clock` (`file_clock`) moves this change
    /// time: each gets a time no earlier than the reading, so only a change time older than it is
    /// no later change's. One equal to the reading may be shared by a write in the same tick.
    fn older_than(&self, clock: Option<i64>) -> bool {
        clock.is_some_and(|clock| self.changed < clock)
    }
}

/// How a clean file whose stat equals the stat Git recorded is shown to hold its blob's bytes.
pub(super) enum Proof {
    /// Its stat, its change time or what a restore remembered shows it.
    Held,
    /// Only a read can tell: a hash equal to the blob is remembered under the stamp, if any.
    Read(Option<ChangeStamp>),
}

/// What restores remember of a worktree's clean files whose change time moved after their last
/// write: rows of a relation in the content store's frames, read when the first such file needs
/// them and written once by `save`. A restore where no change time moved reads and writes no row.
pub struct VerifiedFiles<'a> {
    store: Option<&'a ContentStore>,
    worktree: String,
    /// The remembered files, by the key of their repository path (`worktree_path_key`).
    known: Option<HashMap<i64, VerifiedWorktreeFile>>,
    learned: Vec<VerifiedWorktreeFile>,
    /// Whether each volume, by its serial number, keeps a change time to trust: asked once per
    /// volume, since every file on it gives the same answer; under `None`, once for the files
    /// that give no identity.
    volumes: HashMap<Option<u64>, bool>,
}

/// The worktree and each volume's answer: the store gives no `Debug`, and the files are many.
impl std::fmt::Debug for VerifiedFiles<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("VerifiedFiles")
            .field("worktree", &self.worktree)
            .field("volumes", &self.volumes)
            .finish_non_exhaustive()
    }
}

impl<'a> VerifiedFiles<'a> {
    /// The memory of the worktree whose root is `worktree`, kept in `store`.
    pub fn new(store: Option<&'a ContentStore>, worktree: &Path) -> Self {
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
        // A file with no identity is remembered by none: where change times count, a read decides.
        let stamp = change.stamp();
        if !self.trusts(stamp.map(|stamp| stamp.volume), change)
            || stamp.is_some_and(|stamp| self.remembers(path, &stamp, blob))
        {
            return Proof::Held;
        }
        Proof::Read(stamp)
    }

    /// Whether the volume whose serial number is `volume` keeps a change time to trust; `None`
    /// stands for the volume of the files that gave no identity.
    fn trusts(&mut self, volume: Option<u64>, change: &platform::Change) -> bool {
        *self
            .volumes
            .entry(volume)
            .or_insert_with(|| change.volume_keeps_change_times())
    }

    /// Remembers that the file at `path` held `blob`'s bytes under `stamp`, read after the file
    /// clock read `clock`, where the stamp's change time is older than that reading
    /// (`ChangeStamp::older_than`): a write the read missed then moved it.
    pub(super) fn learn(
        &mut self,
        path: &str,
        stamp: Option<ChangeStamp>,
        blob: HashKey,
        clock: Option<i64>,
    ) {
        let Some(stamp) = stamp.filter(|stamp| stamp.older_than(clock)) else {
            return;
        };
        let file = VerifiedWorktreeFile {
            path: worktree_path_key(path),
            volume: stamp.volume,
            file_id: stamp.file_id,
            changed: stamp.changed,
            blob,
        };
        self.known().insert(file.path, file.clone());
        self.learned.push(file);
    }

    /// How many files this memory learned: each one a read whose hash equalled its blob.
    #[cfg(all(test, windows))]
    pub(super) fn learned(&self) -> usize {
        self.learned.len()
    }

    /// Hands what this restore learned to the store's writer, which writes it behind the restore
    /// as it writes every relation's rows: a write that fails keeps nothing, which costs the next
    /// restore a read of each file.
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
            .get(&worktree_path_key(path))
            .is_some_and(|file| file.blob == blob && ChangeStamp::of(file) == *stamp)
    }

    /// The remembered files, read from the store when first needed: those remembered within the
    /// last 30 days. A store or a clock that cannot be read remembers none, which costs a read of
    /// each file.
    fn known(&mut self) -> &mut HashMap<i64, VerifiedWorktreeFile> {
        let (store, worktree) = (self.store, &self.worktree);
        self.known.get_or_insert_with(|| {
            let now = crate::content_store::unix_seconds().ok();
            store
                .zip(now)
                .and_then(|(store, now)| store.verified_worktree_files(worktree, now).ok())
                .unwrap_or_default()
                .into_iter()
                .map(|file| (file.path, file))
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

/// Whether a file whose volume answered the protocol query (`FileRemoteProtocolInfo`) as
/// `answered` says, or refused it with `refusal`, is on this machine: a network share answers the
/// protocol it is reached by, and a local NTFS or ReFS volume (a Dev Drive is ReFS) refuses the
/// query as a parameter it does not take (`ERROR_INVALID_PARAMETER`, 87), for a file, a folder and
/// the volume's root alike. Any other refusal cannot tell, so the stat decides, as on a share, and
/// no file is remembered.
#[cfg(any(windows, test))]
fn on_this_machine(answered: bool, refusal: &std::io::Error) -> bool {
    !answered && refusal.raw_os_error() == Some(87)
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
pub fn recalls_data(attributes: u32) -> bool {
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

    /// No change time is kept beside the stat here, so no reading is compared with one.
    pub(super) fn file_clock() -> Option<i64> {
        None
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
        /// query: it gave no change time, or a change time of 0, which a volume that keeps none
        /// gives (FAT and exFAT), or the file's data is not all on this disk (`recalls_data`).
        pub(super) fn leaves_the_stat_to_decide(&self) -> bool {
            self.times.is_none_or(|(changed, _)| changed == 0)
                || super::recalls_data(self.attributes)
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
        pub(super) fn with_facts(self, times: Option<(i64, i64)>, attributes: u32) -> Self {
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
        /// Whether the file id query answers nothing on this thread, as on a volume that gives
        /// no file id.
        pub(super) static NO_FILE_ID: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
        /// Whether every file answers as one on a network share on this thread.
        pub(super) static ON_A_SHARE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
        /// The file clock's reading on this thread, where a case sets one (`super::file_clock_reads`).
        pub(super) static CLOCK: std::cell::Cell<Option<i64>> = const { std::cell::Cell::new(None) };
    }

    /// The system time as of the clock's last tick (`GetSystemTimeAsFileTime`), which NTFS and
    /// ReFS stamp a change with: a change after this reading gets a time no earlier than it,
    /// whether the file system takes this clock or a finer one.
    pub(super) fn file_clock() -> Option<i64> {
        #[cfg(test)]
        if let Some(reading) = CLOCK.get() {
            return Some(reading);
        }
        let mut now = windows_sys::Win32::Foundation::FILETIME::default();
        // SAFETY: a buffer of the type the call fills, which lives across the call.
        unsafe {
            windows_sys::Win32::System::SystemInformation::GetSystemTimeAsFileTime(&raw mut now)
        };
        Some((i64::from(now.dwHighDateTime) << 32) | i64::from(now.dwLowDateTime))
    }

    /// Counts a query; `answer`, what this thread's case makes it answer in place of the file.
    #[cfg(test)]
    fn queried(answer: bool) -> bool {
        QUERIES.set(QUERIES.get() + 1);
        answer
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
        if queried(NO_FILE_ID.get()) {
            return None;
        }
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

    /// Whether `file` is on this machine (`super::on_this_machine`).
    fn is_local(file: &File) -> bool {
        #[cfg(test)]
        if queried(ON_A_SHARE.get()) {
            return false;
        }
        // Larger than `FILE_REMOTE_PROTOCOL_INFO`, and aligned as it is.
        let mut info = [0_u64; 32];
        // SAFETY: an open handle, and a buffer of the size given that lives across the call.
        let answered = unsafe {
            GetFileInformationByHandleEx(
                file.as_raw_handle(),
                FileRemoteProtocolInfo,
                info.as_mut_ptr().cast(),
                std::mem::size_of_val(&info) as u32,
            )
        } != 0;
        super::on_this_machine(answered, &std::io::Error::last_os_error())
    }

    /// The name of the file system that holds `file`, such as `NTFS`; empty where it is not told.
    fn file_system(file: &File) -> String {
        #[cfg(test)]
        queried(false);
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
    use super::{
        ChangeStamp, HashKey, VerifiedFiles, keeps_change_times, on_this_machine, stat_decides,
    };
    use std::io::{Error, ErrorKind};
    use std::path::Path;

    /// A read's hash is remembered under a change time older than the file clock's reading taken
    /// before the read, since every later write gets a time no earlier than that reading. A change
    /// time on the reading itself, or past it, may be a later write's too (a write in the same
    /// tick of the clock), and with no reading nothing is remembered.
    #[test]
    fn only_a_change_time_older_than_the_file_clock_before_the_read_is_remembered() {
        let changed = 134_000_000_000_000_000;
        let stamp = ChangeStamp {
            volume: 7,
            file_id: [1; 16],
            changed,
        };
        let blob = HashKey::git_sha1([7; 20]);
        let mut memory = VerifiedFiles::new(None, Path::new("worktree"));
        for clock in [None, Some(changed - 1), Some(changed)] {
            memory.learn("a.py", Some(stamp), blob, clock);
            assert!(memory.learned.is_empty(), "{clock:?}");
        }
        memory.learn("a.py", Some(stamp), blob, Some(changed + 1));
        assert_eq!(
            memory.learned.len(),
            1,
            "a change time older than the reading"
        );
    }

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

    /// A file is on this machine only where its volume refuses the protocol query as a local
    /// NTFS or ReFS volume does (87, a Dev Drive's too): a share answers it, and a volume that
    /// refuses it for another reason (a function it lacks, a denial, a redirector that does not
    /// take the query) cannot tell.
    #[test]
    fn only_a_refusal_of_the_protocol_query_as_ntfs_and_refs_give_it_is_this_machine() {
        let invalid_parameter = Error::from_raw_os_error(87);
        assert!(on_this_machine(false, &invalid_parameter));
        assert!(!on_this_machine(true, &invalid_parameter), "answered");
        for code in [1, 5, 50, 120] {
            let refusal = Error::from_raw_os_error(code);
            assert!(!on_this_machine(false, &refusal), "{code}");
        }
    }
}
