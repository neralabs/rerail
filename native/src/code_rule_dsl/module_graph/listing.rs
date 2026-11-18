//! A folder's entries as the server graph's disk walk lists them (`walk`): each one's name and
//! kind and, for a file, the `Facts` its cached summary's `Stamp` comes from. On Windows one call
//! per folder page gives all of it (`FileIdBothDirectoryInfo`), NTFS's change time and file id
//! included, which a rewrite that puts the modification time back still moves; where the folder
//! cannot be listed that way, `read_dir` lists it as it does elsewhere. Elsewhere `read_dir`, and a
//! file's `lstat` once the walk asks for its facts. Either way a folder is read as its entries are
//! taken.

#[cfg(not(windows))]
use std::ffi::OsString;
#[cfg(not(windows))]
use std::io;
#[cfg(not(windows))]
use std::path::Path;

/// What the walk makes of an entry: a link of any kind is `Other`, as `std::fs::FileType` reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Folder,
    File,
    Other,
}

impl Kind {
    fn of(folder: bool, file: bool) -> Self {
        if folder {
            Self::Folder
        } else if file {
            Self::File
        } else {
            Self::Other
        }
    }
}

/// A file's length and what its `Stamp` stands on: its `lstat`.
#[cfg(not(windows))]
pub(super) type Facts = std::fs::Metadata;

/// One entry of a folder, as `read_dir` gives it.
#[cfg(not(windows))]
pub(super) struct Entry(std::fs::DirEntry);

#[cfg(not(windows))]
impl Entry {
    pub(super) fn name(&self) -> OsString {
        self.0.file_name()
    }

    pub(super) fn kind(&self) -> io::Result<Kind> {
        self.0
            .file_type()
            .map(|kind| Kind::of(kind.is_dir(), kind.is_file()))
    }

    pub(super) fn facts(&self) -> io::Result<Facts> {
        self.0.metadata()
    }
}

/// The entries of `directory`, in no order.
#[cfg(not(windows))]
pub(super) fn list(directory: &Path) -> io::Result<impl Iterator<Item = io::Result<Entry>>> {
    Ok(std::fs::read_dir(directory)?.map(|entry| entry.map(Entry)))
}

#[cfg(all(test, windows))]
pub(super) use windows::PAGES_READ;
#[cfg(windows)]
pub(super) use windows::{Entry, Facts, list};

#[cfg(windows)]
mod windows {
    use super::super::Stamp;
    use super::Kind;
    use std::ffi::OsString;
    use std::io;
    use std::os::windows::ffi::OsStringExt;
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    use std::path::Path;
    use std::time::{Duration, SystemTime};
    use windows_sys::Win32::Foundation::ERROR_NO_MORE_FILES;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS,
        FILE_ID_BOTH_DIR_INFO, FILE_INFO_BY_HANDLE_CLASS, FILE_LIST_DIRECTORY,
        FileIdBothDirectoryInfo, FileIdBothDirectoryRestartInfo, GetFileInformationByHandleEx,
    };

    /// A reparse tag with this bit names another file (a symbolic link, a junction): std's
    /// `FileType` reads such an entry as a link.
    const NAME_SURROGATE: u32 = 0x2000_0000;
    /// A file time's count of 100 ns intervals from 1601 to 1970.
    const UNIX_EPOCH_TICKS: i64 = 116_444_736_000_000_000;
    /// One page of a folder's listing, in 8-byte words: its entries start 8-byte aligned.
    const PAGE_WORDS: usize = 8192;

    #[cfg(test)]
    thread_local! {
        /// How many pages this thread's listings asked their file system for.
        pub(in super::super) static PAGES_READ: std::cell::Cell<usize> =
            const { std::cell::Cell::new(0) };
    }

    /// A file's length and `Stamp`, read with its folder's listing.
    #[derive(Clone, Copy, Debug)]
    pub(in super::super) struct Facts {
        length: u64,
        stamp: Option<Stamp>,
    }

    impl Facts {
        pub(in super::super) fn len(&self) -> u64 {
            self.length
        }

        pub(in super::super) fn stamp(&self) -> Option<Stamp> {
            self.stamp
        }
    }

    /// One entry of a folder, read whole with the folder's listing.
    pub(in super::super) struct Entry {
        name: OsString,
        kind: Kind,
        facts: Facts,
    }

    impl Entry {
        pub(in super::super) fn name(&self) -> OsString {
            self.name.clone()
        }

        pub(in super::super) fn kind(&self) -> io::Result<Kind> {
            Ok(self.kind)
        }

        pub(in super::super) fn facts(&self) -> io::Result<Facts> {
            Ok(self.facts)
        }
    }

    /// The entries of `directory`, in no order, read a page at a time as they are taken: a walk
    /// that stops at its entry bound reads no page past the one that holds it.
    pub(in super::super) fn list(directory: &Path) -> io::Result<Listing> {
        or_read_dir(directory, Pages::open(directory))
    }

    /// The listing of `directory` from `paged`, its handle's pages; where they failed, `read_dir`
    /// lists it, as for a file system that cannot list a folder by `FileIdBothDirectoryInfo`, and
    /// gives the answer it gives on its own. A folder gone is gone either way.
    fn or_read_dir(directory: &Path, paged: io::Result<Pages>) -> io::Result<Listing> {
        match paged {
            Err(error) if error.kind() != io::ErrorKind::NotFound => {
                std::fs::read_dir(directory).map(Listing::ReadDir)
            }
            paged => paged.map(Listing::Paged),
        }
    }

    /// A folder's entries, as `list` reads them.
    pub(in super::super) enum Listing {
        Paged(Pages),
        ReadDir(std::fs::ReadDir),
    }

    impl Iterator for Listing {
        type Item = io::Result<Entry>;

        fn next(&mut self) -> Option<Self::Item> {
            match self {
                Self::Paged(pages) => pages.next(),
                Self::ReadDir(entries) => entries
                    .next()
                    .map(|entry| entry.and_then(|entry| from_dir_entry(&entry))),
            }
        }
    }

    /// A folder's entries from its handle, one page of `FileIdBothDirectoryInfo` at a time.
    pub(in super::super) struct Pages {
        folder: std::fs::File,
        page: Vec<u64>,
        /// The entries of the page read last that are not taken yet.
        entries: Vec<Entry>,
        /// Whether a page may follow the one read last.
        more: bool,
    }

    impl Pages {
        /// Opens `directory` and reads its first page.
        fn open(directory: &Path) -> io::Result<Self> {
            let folder = std::fs::OpenOptions::new()
                .access_mode(FILE_LIST_DIRECTORY)
                .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
                .open(directory)?;
            let mut pages = Self {
                folder,
                page: vec![0u64; PAGE_WORDS],
                entries: Vec::new(),
                more: true,
            };
            pages.read(FileIdBothDirectoryRestartInfo)?;
            Ok(pages)
        }

        /// Reads the next page (`class` names the first) into `entries`; `more` turns false
        /// once none is left.
        fn read(&mut self, class: FILE_INFO_BY_HANDLE_CLASS) -> io::Result<()> {
            self.more = read_page(&self.folder, class, &mut self.page)?;
            if self.more {
                entries_of(&self.page, &mut self.entries);
            }
            Ok(())
        }
    }

    impl Iterator for Pages {
        type Item = io::Result<Entry>;

        /// The next entry, from the page read last while it has one left; a page that fails ends
        /// the listing with its error.
        fn next(&mut self) -> Option<Self::Item> {
            while self.entries.is_empty() && self.more {
                if let Err(error) = self.read(FileIdBothDirectoryInfo) {
                    self.more = false;
                    return Some(Err(error));
                }
            }
            self.entries.pop().map(Ok)
        }
    }

    /// The next page of `folder`'s entries into `page`; `false` once none is left.
    fn read_page(
        folder: &std::fs::File,
        class: FILE_INFO_BY_HANDLE_CLASS,
        page: &mut [u64],
    ) -> io::Result<bool> {
        #[cfg(test)]
        PAGES_READ.with(|read| read.set(read.get() + 1));
        // SAFETY: an open folder handle and a buffer of the size given, alive across the call.
        let read = unsafe {
            GetFileInformationByHandleEx(
                folder.as_raw_handle(),
                class,
                page.as_mut_ptr().cast(),
                std::mem::size_of_val(page) as u32,
            )
        };
        if read != 0 {
            return Ok(true);
        }
        let error = io::Error::last_os_error();
        match error.raw_os_error() {
            Some(code) if code == ERROR_NO_MORE_FILES as i32 => Ok(false),
            _ => Err(error),
        }
    }

    /// Adds the entries of one page `read_page` filled to `listed`, all but `.` and `..`.
    fn entries_of(page: &[u64], listed: &mut Vec<Entry>) {
        let base = page.as_ptr().cast::<u8>();
        let mut offset = 0;
        loop {
            // SAFETY: the call wrote whole entries, each 8-byte aligned at `offset` in the page.
            let info = unsafe { &*base.add(offset).cast::<FILE_ID_BOTH_DIR_INFO>() };
            // SAFETY: the name follows the entry's fixed fields, `FileNameLength` bytes long.
            let name = unsafe {
                std::slice::from_raw_parts(
                    base.add(offset + std::mem::offset_of!(FILE_ID_BOTH_DIR_INFO, FileName))
                        .cast::<u16>(),
                    info.FileNameLength as usize / 2,
                )
            };
            if name != [u16::from(b'.')] && name != [u16::from(b'.'); 2] {
                listed.push(entry_of(OsString::from_wide(name), info));
            }
            if info.NextEntryOffset == 0 {
                return;
            }
            offset += info.NextEntryOffset as usize;
        }
    }

    fn entry_of(name: OsString, info: &FILE_ID_BOTH_DIR_INFO) -> Entry {
        let length = u64::try_from(info.EndOfFile).unwrap_or(0);
        let stamp = system_time(info.LastWriteTime).map(|modified| Stamp {
            modified,
            length,
            changed: info.ChangeTime,
            file: info.FileId,
        });
        Entry {
            name,
            kind: kind_of(info.FileAttributes, info.EaSize),
            facts: Facts { length, stamp },
        }
    }

    /// The kind std's `FileType` gives an entry with these attributes: the listing puts a
    /// reparse point's tag in `EaSize`.
    fn kind_of(attributes: u32, tag: u32) -> Kind {
        let link = attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 && tag & NAME_SURROGATE != 0;
        let folder = attributes & FILE_ATTRIBUTE_DIRECTORY != 0;
        Kind::of(!link && folder, !link && !folder)
    }

    /// A file time (100 ns intervals since 1601) as a `SystemTime`; `None` past its range.
    fn system_time(ticks: i64) -> Option<SystemTime> {
        let since = ticks.checked_sub(UNIX_EPOCH_TICKS)?;
        let span = Duration::from_nanos(since.unsigned_abs().checked_mul(100)?);
        if since < 0 {
            SystemTime::UNIX_EPOCH.checked_sub(span)
        } else {
            SystemTime::UNIX_EPOCH.checked_add(span)
        }
    }

    /// An entry `read_dir` gives, for a file system that cannot list a folder by
    /// `FileIdBothDirectoryInfo`: its stamp holds no change time or file id, as `read_dir` gives
    /// none.
    fn from_dir_entry(entry: &std::fs::DirEntry) -> io::Result<Entry> {
        let kind = entry.file_type()?;
        let metadata = entry.metadata()?;
        let stamp = metadata.modified().ok().map(|modified| Stamp {
            modified,
            length: metadata.len(),
            changed: 0,
            file: 0,
        });
        Ok(Entry {
            name: entry.file_name(),
            kind: Kind::of(kind.is_dir(), kind.is_file()),
            facts: Facts {
                length: metadata.len(),
                stamp,
            },
        })
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use windows_sys::Win32::Foundation::ERROR_INVALID_FUNCTION;

        /// Where the handle's pages fail but for a folder gone, `read_dir` lists the folder: some
        /// file systems answer a listing class they do not know with `ERROR_INVALID_FUNCTION`.
        #[test]
        fn a_folder_whose_pages_fail_is_listed_by_read_dir_unless_it_is_gone() {
            let folder = std::env::temp_dir().join(format!(
                "rerail-listing-fallback-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(folder.join("a")).unwrap();
            std::fs::write(folder.join("b.ts"), "").unwrap();
            let refused = io::Error::from_raw_os_error(ERROR_INVALID_FUNCTION as i32);
            let mut names = or_read_dir(&folder, Err(refused))
                .unwrap()
                .map(|entry| entry.unwrap().name())
                .collect::<Vec<_>>();
            names.sort();
            assert_eq!(names, ["a", "b.ts"]);
            let gone = io::Error::from(io::ErrorKind::NotFound);
            assert!(
                or_read_dir(&folder, Err(gone))
                    .is_err_and(|error| error.kind() == io::ErrorKind::NotFound)
            );
            std::fs::remove_dir_all(&folder).unwrap();
        }
    }
}
