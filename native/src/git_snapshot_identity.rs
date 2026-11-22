//! Clone identity for history and worktree identity for ref and index requests.
use rerail_store::Canonical as _;
use sha2::{Digest as _, Sha256};
use std::fs;
use std::io::{self, Read as _};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct Identity {
    pub repository: String,
    pub view: String,
}

/// Open directory handles outlive a rename and prevent reuse of a deleted directory's file number.
#[derive(Debug)]
pub(super) struct Directories(Vec<PinnedDirectory>);

#[derive(Debug)]
struct PinnedDirectory {
    path: PathBuf,
    #[cfg(not(target_family = "wasm"))]
    original: same_file::Handle,
    #[cfg(target_family = "wasm")]
    original: fs::File,
}

impl Directories {
    pub(super) fn open(repo: &gix::Repository) -> io::Result<Self> {
        let mut dirs = Vec::new();
        for path in [repo.git_dir(), repo.common_dir()] {
            if dirs.iter().any(|d: &PinnedDirectory| d.path == path) {
                continue;
            }
            #[cfg(not(target_family = "wasm"))]
            let original = same_file::Handle::from_path(path)?;
            #[cfg(target_family = "wasm")]
            let original = fs::File::open(path)?;
            dirs.push(PinnedDirectory {
                path: path.to_owned(),
                original,
            });
        }
        Ok(Self(dirs))
    }

    pub(super) fn check(&self) -> io::Result<()> {
        for dir in &self.0 {
            let current = dir.is_current().map_err(|err| {
                if err.kind() == io::ErrorKind::NotFound {
                    io::Error::other(format!("Git directory {} is unavailable: {err}", dir.path.display()))
                } else {
                    err
                }
            })?;
            if !current {
                return Err(io::Error::other(
                    "Git directory changed since the source view was opened",
                ));
            }
        }
        Ok(())
    }
}

impl PinnedDirectory {
    fn is_current(&self) -> io::Result<bool> {
        #[cfg(not(target_family = "wasm"))]
        {
            Ok(same_file::Handle::from_path(&self.path)? == self.original)
        }
        #[cfg(target_os = "wasi")]
        {
            use std::os::wasi::fs::MetadataExt as _;
            let original = self.original.metadata()?;
            let current = fs::metadata(&self.path)?;
            Ok((original.dev(), original.ino()) == (current.dev(), current.ino()))
        }
        #[cfg(all(target_family = "wasm", not(target_os = "wasi")))]
        {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "the host cannot check a Git directory identity",
            ))
        }
    }
}

fn digest(bytes: &[u8]) -> String {
    crate::content_store::observe::hex(&Sha256::digest(bytes).into())
}

fn valid(id: &str) -> bool {
    let bytes = id.as_bytes();
    (1..=100).contains(&bytes.len())
        && bytes[0].is_ascii_alphanumeric()
        && bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || b"._:-".contains(b))
}

fn read_id(path: &Path) -> io::Result<Option<String>> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt as _;
        options.custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = match options.open(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        result => result?,
    };
    let metadata = file.metadata()?;
    if metadata.is_file() && metadata.len() <= 128 {
        let mut value = String::new();
        file.take(129).read_to_string(&mut value)?;
        if value.len() <= 128 && valid(value.trim()) {
            return Ok(Some(value.trim().to_owned()));
        }
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        format!("invalid Rerail repository identity at {}", path.display()),
    ))
}

fn directory_seed(common: &Path) -> Option<String> {
    let metadata = fs::metadata(common).ok()?;
    #[cfg(unix)]
    let (dev, ino) = {
        use std::os::unix::fs::MetadataExt as _;
        (metadata.dev(), metadata.ino())
    };
    #[cfg(windows)]
    let (dev, ino) = super::storage::windows::read_metadata(common)
        .ok()?
        .directory_id();
    #[cfg(target_os = "wasi")]
    let (dev, ino) = {
        use std::os::wasi::fs::MetadataExt as _;
        (metadata.dev(), metadata.ino())
    };
    #[cfg(not(any(unix, windows, target_os = "wasi")))]
    let (dev, ino) = (0u64, 0u64);
    let birth = metadata
        .created()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_nanos();
    if birth == 0 || ino == 0 {
        return None;
    }
    // Keep the byte format in sync with directoryCloneId in repository-identity.ts.
    Some(format!(
        "git:fs-{}",
        digest(format!("rerail.git-directory/v1\0{dev}/{ino}/{birth}").as_bytes())
    ))
}

#[cfg(not(target_family = "wasm"))]
fn publish(path: &Path, seed: Option<&str>) -> io::Result<String> {
    use std::io::Write as _;
    let common = path.parent().expect("identity in Git common directory");
    let mut file = tempfile::Builder::new()
        .prefix("rerail-repository-id.")
        .rand_bytes(32)
        .tempfile_in(common)?;
    let id = seed.map(str::to_owned).unwrap_or_else(|| {
        format!(
            "git:{}",
            digest(
                file.path()
                    .file_name()
                    .expect("temporary file")
                    .as_encoded_bytes()
            )
        )
    });
    writeln!(file, "{id}")?;
    match file.persist_noclobber(path) {
        Ok(_) => Ok(id),
        Err(e) if e.error.kind() == io::ErrorKind::AlreadyExists => read_id(path)?.ok_or(e.error),
        Err(e) => Err(e.error),
    }
}

fn clone_id(repo: &gix::Repository, publish_id: bool) -> io::Result<String> {
    let common = repo.common_dir();
    let path = common.join("rerail-repository-id");
    if let Some(id) = read_id(&path)? {
        return Ok(id);
    }
    let legacy = common.join(".rerail");
    let old_bare = if legacy.symlink_metadata().is_ok_and(|m| m.is_dir())
        && gix::open(common).is_ok_and(|r| r.is_bare())
    {
        read_id(&legacy.join("repository-id"))?
    } else {
        None
    };
    let configured = || {
        repo.config_snapshot()
            .plumbing()
            .string_filter("rerail.repository-id", |m| {
                m.source == gix::config::Source::Local
            })
            .and_then(|s| {
                std::str::from_utf8(&s)
                    .ok()
                    .map(str::trim)
                    .filter(|s| valid(s))
                    .map(str::to_owned)
            })
    };
    let seed = old_bare
        .or_else(configured)
        .or_else(|| directory_seed(common));
    #[cfg(not(target_family = "wasm"))]
    if publish_id || seed.is_none() {
        return match publish(&path, seed.as_deref()) {
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::PermissionDenied | io::ErrorKind::ReadOnlyFilesystem
                ) =>
            {
                read_id(&path)?.or(seed).ok_or_else(|| {
                    io::Error::new(
                        e.kind(),
                        format!("read-only Git directory has no stable repository identity: {e}"),
                    )
                })
            }
            result => result,
        };
    }
    #[cfg(target_family = "wasm")]
    let _ = publish_id;
    seed.ok_or_else(|| io::Error::other("no stable Git directory identity is available"))
}

impl Identity {
    pub fn of(repo: &gix::Repository, publish_id: bool) -> io::Result<Self> {
        let id = clone_id(repo, publish_id)?;
        Ok(Self::from_id(&id, &repo.git_dir().canonical()?))
    }

    fn from_id(id: &str, admin: &Path) -> Self {
        let repository = format!("git-clone:{}", digest(id.as_bytes()));
        let mut bytes = id.as_bytes().to_vec();
        bytes.push(0);
        if let Some(physical) = directory_seed(admin) {
            bytes.extend_from_slice(physical.as_bytes());
        } else {
            bytes.extend_from_slice(admin.as_os_str().as_encoded_bytes());
        }
        Self {
            repository,
            view: format!("git-view:{}", digest(&bytes)),
        }
    }
}
