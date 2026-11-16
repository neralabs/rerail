//! Whether paths belong to the user this process runs as, by the rule Git applies on Windows before it takes a
//! repository.
//!
//! Git refuses a repository whose worktree, `.git` file or Git folder another account owns, unless `safe.directory`
//! names it. Node reads no owner on Windows, so the catalog asks here before it places a linked worktree from its files
//! instead of starting Git; every answer but "owned" sends it to Git.

use napi_derive::napi;

/// Whether every path's owner is the user this process runs as, or the Administrators group while this process's token
/// is a member of it: the two owners Git for Windows takes as the user's own (`is_path_owned_by_current_sid` in its
/// `compat/mingw.c`), and a Git this process starts runs with the same token. False when an owner cannot be read, and
/// on every platform but Windows, where the caller compares `lstat`'s owner itself.
#[napi(catch_unwind)]
pub fn paths_owned_by_current_user(paths: Vec<String>) -> bool {
    #[cfg(windows)]
    {
        !paths.is_empty()
            && paths
                .iter()
                .all(|path| windows::owned_by_current_user(path))
    }
    #[cfg(not(windows))]
    {
        let _ = paths;
        false
    }
}

#[cfg(windows)]
mod windows {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::ptr::null_mut;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::Security::{
        CheckTokenMembership, EqualSid, GetFileSecurityW, GetSecurityDescriptorOwner,
        GetTokenInformation, IsValidSid, IsWellKnownSid, OWNER_SECURITY_INFORMATION, PSID,
        TOKEN_QUERY, TOKEN_USER, TokenUser, WinBuiltinAdministratorsSid,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    pub(super) fn owned_by_current_user(path: &str) -> bool {
        let Some(descriptor) = security_descriptor(path) else {
            return false;
        };
        let Some(owner) = owner_of(&descriptor) else {
            return false;
        };
        is_current_user(owner) || is_administrators_member(owner)
    }

    /// `path`'s security descriptor holding its owner, self-relative, in a buffer aligned for it; None when it cannot
    /// be read (no such path, no right to read it) or the path holds a NUL, which would end it early.
    fn security_descriptor(path: &str) -> Option<Vec<u64>> {
        if path.contains('\0') {
            return None;
        }
        let wide: Vec<u16> = OsStr::new(path).encode_wide().chain(Some(0)).collect();
        let mut needed = 0u32;
        // SAFETY: a null buffer of length 0 asks only for the size, which the call writes to `needed`.
        unsafe {
            GetFileSecurityW(
                wide.as_ptr(),
                OWNER_SECURITY_INFORMATION,
                null_mut(),
                0,
                &mut needed,
            )
        };
        if needed == 0 {
            return None;
        }
        let mut buffer = vec![0u64; (needed as usize).div_ceil(8)];
        // SAFETY: `buffer` holds at least `needed` bytes, aligned for a descriptor; `wide` ends with a NUL.
        let read = unsafe {
            GetFileSecurityW(
                wide.as_ptr(),
                OWNER_SECURITY_INFORMATION,
                buffer.as_mut_ptr().cast(),
                needed,
                &mut needed,
            )
        };
        (read != 0).then_some(buffer)
    }

    /// The owner a descriptor `security_descriptor` read names, pointing inside it; None when it names no valid one.
    fn owner_of(descriptor: &[u64]) -> Option<PSID> {
        let mut owner: PSID = null_mut();
        let mut defaulted = 0;
        // SAFETY: `descriptor` is the self-relative descriptor GetFileSecurityW wrote; the call only reads it.
        let read = unsafe {
            GetSecurityDescriptorOwner(
                descriptor.as_ptr().cast_mut().cast(),
                &mut owner,
                &mut defaulted,
            )
        };
        // SAFETY: a non-null `owner` points at the SID inside `descriptor`, which outlives this call.
        (read != 0 && !owner.is_null() && unsafe { IsValidSid(owner) } != 0).then_some(owner)
    }

    /// Whether `owner` is the user this process's token names (`TokenUser`), as Git compares them.
    fn is_current_user(owner: PSID) -> bool {
        let Some(token_user) = token_user() else {
            return false;
        };
        // SAFETY: `token_user` holds the TOKEN_USER GetTokenInformation wrote; its SID points inside the same buffer.
        let user = unsafe { (*token_user.as_ptr().cast::<TOKEN_USER>()).User.Sid };
        // SAFETY: both SIDs are valid and live for the call: `owner` in its descriptor, `user` in `token_user`.
        unsafe { IsValidSid(user) != 0 && EqualSid(owner, user) != 0 }
    }

    /// This process's TOKEN_USER, in a buffer aligned for it.
    fn token_user() -> Option<Vec<u64>> {
        let mut token: HANDLE = null_mut();
        // SAFETY: the pseudo handle of this process; on success the call writes an open token handle to `token`.
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
            return None;
        }
        let user = token_information(token);
        // SAFETY: `token` was opened above and is closed once.
        unsafe { CloseHandle(token) };
        user
    }

    fn token_information(token: HANDLE) -> Option<Vec<u64>> {
        let mut needed = 0u32;
        // SAFETY: a null buffer of length 0 asks only for the size, which the call writes to `needed`.
        unsafe { GetTokenInformation(token, TokenUser, null_mut(), 0, &mut needed) };
        if needed == 0 {
            return None;
        }
        let mut buffer = vec![0u64; (needed as usize).div_ceil(8)];
        // SAFETY: `buffer` holds at least `needed` bytes, aligned for a TOKEN_USER and the SID after it.
        let read = unsafe {
            GetTokenInformation(
                token,
                TokenUser,
                buffer.as_mut_ptr().cast(),
                needed,
                &mut needed,
            )
        };
        (read != 0).then_some(buffer)
    }

    /// Whether `owner` is the Administrators group and this process's token counts as a member (an elevated
    /// administrator's), which Git takes as the user's own too. A null token handle checks the calling thread's token,
    /// as Git's own check does.
    fn is_administrators_member(owner: PSID) -> bool {
        let mut member = 0;
        // SAFETY: `owner` is a valid SID inside its descriptor, which outlives the call.
        unsafe {
            IsWellKnownSid(owner, WinBuiltinAdministratorsSid) != 0
                && CheckTokenMembership(null_mut(), owner, &mut member) != 0
                && member != 0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::paths_owned_by_current_user;

    #[test]
    fn no_path_is_owned() {
        assert!(!paths_owned_by_current_user(Vec::new()));
    }

    /// Outside Windows the catalog compares `lstat`'s owner itself, so the engine answers no.
    #[cfg(not(windows))]
    #[test]
    fn outside_windows_the_owner_is_left_to_the_caller() {
        let folder = std::env::temp_dir().to_string_lossy().into_owned();
        assert!(!paths_owned_by_current_user(vec![folder]));
    }

    #[cfg(windows)]
    #[test]
    fn a_folder_this_process_made_is_owned_and_one_that_is_gone_is_not() {
        let folder = std::env::temp_dir().join(format!(
            "rerail-path-owner-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&folder).unwrap();
        let made = folder.to_string_lossy().into_owned();
        let gone = folder.join("gone").to_string_lossy().into_owned();
        let with_nul = format!("{made}\0x");

        assert!(paths_owned_by_current_user(vec![made.clone()]));
        assert!(!paths_owned_by_current_user(vec![gone.clone()]));
        assert!(!paths_owned_by_current_user(vec![made, gone]));
        assert!(!paths_owned_by_current_user(vec![with_nul]));

        std::fs::remove_dir_all(folder).unwrap();
    }
}
