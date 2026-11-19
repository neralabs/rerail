//! Windows storage observations for loose Git objects and pack files.

use std::path::Path;
use sha2::{Digest as _, Sha256};
use windows_sys::Win32::Storage::FileSystem::{BY_HANDLE_FILE_INFORMATION, FILE_BASIC_INFO};

pub(super) struct StorageMetadata {
    file: BY_HANDLE_FILE_INFORMATION,
    basic: FILE_BASIC_INFO,
}

pub(super) fn read_metadata(path: &Path) -> std::io::Result<StorageMetadata> {
    use std::os::windows::{fs::OpenOptionsExt as _, io::AsRawHandle as _};
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_READ_ATTRIBUTES, FileBasicInfo, GetFileInformationByHandle, GetFileInformationByHandleEx,
    };
    // Allow directory handles as well. The default share mode permits readers, writers and renames.
    let handle = std::fs::OpenOptions::new().access_mode(FILE_READ_ATTRIBUTES).custom_flags(FILE_FLAG_BACKUP_SEMANTICS).open(path)?;
    let mut file = BY_HANDLE_FILE_INFORMATION::default();
    let mut basic = FILE_BASIC_INFO::default();
    // Both buffers have the layout and size of the information requested. The handle stays open for both calls.
    if unsafe { GetFileInformationByHandle(handle.as_raw_handle(), &mut file) } == 0
        || unsafe { GetFileInformationByHandleEx(handle.as_raw_handle(), FileBasicInfo, (&raw mut basic).cast(), size_of::<FILE_BASIC_INFO>() as u32) } == 0
    {
        return Err(std::io::Error::last_os_error());
    }
    if basic.ChangeTime == 0 {
        return Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "filesystem supplied no file change time"));
    }
    Ok(StorageMetadata { file, basic })
}

impl StorageMetadata {
    pub(super) fn hash(&self, digest: &mut Sha256) {
        digest.update(self.file.dwVolumeSerialNumber.to_le_bytes());
        digest.update(self.file.nFileIndexHigh.to_le_bytes());
        digest.update(self.file.nFileIndexLow.to_le_bytes());
        digest.update(self.file.nFileSizeHigh.to_le_bytes());
        digest.update(self.file.nFileSizeLow.to_le_bytes());
        digest.update(self.basic.CreationTime.to_le_bytes());
        digest.update(self.basic.LastWriteTime.to_le_bytes());
        digest.update(self.basic.ChangeTime.to_le_bytes());
    }
}
