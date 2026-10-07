//! Detached process creation that hands the child only the handles it is given.
//!
//! Windows `CreateProcess` with handle inheritance enabled (how Node and libuv spawn) gives the child every
//! inheritable handle in the parent, not just its stdio. A launcher written in Rust, such as `just`, leaves its own
//! stdout inheritable, so a long-lived daemon started below it kept the caller's output pipe open: `just rerail`
//! with captured output never finished while the daemon ran. `PROC_THREAD_ATTRIBUTE_HANDLE_LIST` limits
//! inheritance to NUL stdin and the log file.

use napi::{Error, Result, Status};
use napi_derive::napi;
use std::collections::HashMap;

/// Quote one argument for the Microsoft C runtime command-line parser `node.exe` uses.
#[cfg_attr(not(windows), allow(dead_code))]
fn quote_windows_argument(argument: &str, line: &mut String) {
    if !argument.is_empty() && !argument.contains([' ', '\t', '\n', '\u{b}', '"']) {
        line.push_str(argument);
        return;
    }
    line.push('"');
    let mut backslashes = 0usize;
    for character in argument.chars() {
        match character {
            '\\' => backslashes += 1,
            '"' => {
                line.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                line.push('"');
                backslashes = 0;
            }
            _ => {
                line.extend(std::iter::repeat_n('\\', backslashes));
                line.push(character);
                backslashes = 0;
            }
        }
    }
    line.extend(std::iter::repeat_n('\\', backslashes * 2));
    line.push('"');
}

#[cfg_attr(not(windows), allow(dead_code))]
fn windows_command_line(program: &str, args: &[String]) -> String {
    let mut line = String::new();
    quote_windows_argument(program, &mut line);
    for argument in args {
        line.push(' ');
        quote_windows_argument(argument, &mut line);
    }
    line
}

/// Start `program` detached with NUL stdin, stdout and stderr appended to `log_path`, and no other inherited
/// handle. Returns the child's process id.
#[napi]
pub fn spawn_detached(
    program: String,
    args: Vec<String>,
    cwd: Option<String>,
    env: HashMap<String, String>,
    log_path: String,
) -> Result<u32> {
    #[cfg(windows)]
    {
        windows::spawn_detached(&program, &args, cwd.as_deref(), &env, &log_path)
    }
    #[cfg(not(windows))]
    {
        let _ = (program, args, cwd, env, log_path);
        Err(Error::new(
            Status::GenericFailure,
            "spawnDetached is only needed on Windows; use child_process.spawn elsewhere",
        ))
    }
}

#[cfg(windows)]
mod windows {
    use super::{Error, HashMap, Result, Status, windows_command_line};
    use std::ffi::OsStr;
    use std::mem::{size_of, zeroed};
    use std::os::windows::ffi::OsStrExt;
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Foundation::{
        CloseHandle, GENERIC_READ, GetLastError, HANDLE, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_APPEND_DATA, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_DELETE, FILE_SHARE_READ,
        FILE_SHARE_WRITE, OPEN_ALWAYS, OPEN_EXISTING,
    };
    use windows_sys::Win32::System::Threading::{
        CREATE_NEW_PROCESS_GROUP, CREATE_UNICODE_ENVIRONMENT, CreateProcessW, DETACHED_PROCESS,
        DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT,
        InitializeProcThreadAttributeList, LPPROC_THREAD_ATTRIBUTE_LIST,
        PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROCESS_INFORMATION, STARTF_USESTDHANDLES,
        STARTUPINFOEXW, UpdateProcThreadAttribute,
    };

    fn wide(value: &str) -> Vec<u16> {
        OsStr::new(value).encode_wide().chain(Some(0)).collect()
    }

    fn os_error(action: &str) -> Error {
        // SAFETY: reads the calling thread's last-error value.
        let code = unsafe { GetLastError() };
        Error::new(
            Status::GenericFailure,
            format!(
                "{action} failed: {}",
                std::io::Error::from_raw_os_error(code as i32)
            ),
        )
    }

    /// Closes the wrapped handle when dropped.
    struct OwnedHandle(HANDLE);

    impl Drop for OwnedHandle {
        fn drop(&mut self) {
            // SAFETY: the handle was returned open by the Win32 API and is closed exactly once.
            unsafe { CloseHandle(self.0) };
        }
    }

    fn open_inheritable(path: &str, access: u32, disposition: u32) -> Result<OwnedHandle> {
        let attributes = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: null_mut(),
            bInheritHandle: 1,
        };
        let path = wide(path);
        // SAFETY: `path` is NUL-terminated and `attributes` outlives the call.
        let handle = unsafe {
            CreateFileW(
                path.as_ptr(),
                access,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                &attributes,
                disposition,
                FILE_ATTRIBUTE_NORMAL,
                null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(os_error("CreateFileW"));
        }
        Ok(OwnedHandle(handle))
    }

    /// `KEY=value\0...\0\0`, sorted case-insensitively as CreateProcess expects.
    fn environment_block(env: &HashMap<String, String>) -> Vec<u16> {
        let mut entries: Vec<(&String, &String)> = env.iter().collect();
        entries.sort_by_key(|(key, _)| key.to_uppercase());
        let mut block = Vec::new();
        for (key, value) in entries {
            block.extend(OsStr::new(&format!("{key}={value}")).encode_wide());
            block.push(0);
        }
        if block.is_empty() {
            block.push(0);
        }
        block.push(0);
        block
    }

    /// Owns an initialized attribute list buffer.
    struct AttributeList(Vec<u8>);

    impl AttributeList {
        fn new() -> Result<Self> {
            let mut size = 0usize;
            // SAFETY: a null list with size 0 only reports the required buffer size.
            unsafe { InitializeProcThreadAttributeList(null_mut(), 1, 0, &mut size) };
            let mut buffer = vec![0u8; size];
            // SAFETY: `buffer` holds `size` bytes as requested above.
            if unsafe {
                InitializeProcThreadAttributeList(buffer.as_mut_ptr().cast(), 1, 0, &mut size)
            } == 0
            {
                return Err(os_error("InitializeProcThreadAttributeList"));
            }
            Ok(Self(buffer))
        }

        fn as_ptr(&mut self) -> LPPROC_THREAD_ATTRIBUTE_LIST {
            self.0.as_mut_ptr().cast()
        }
    }

    impl Drop for AttributeList {
        fn drop(&mut self) {
            // SAFETY: the list was initialized in `new` and is deleted once.
            unsafe { DeleteProcThreadAttributeList(self.as_ptr()) };
        }
    }

    pub(super) fn spawn_detached(
        program: &str,
        args: &[String],
        cwd: Option<&str>,
        env: &HashMap<String, String>,
        log_path: &str,
    ) -> Result<u32> {
        let stdin = open_inheritable("NUL", GENERIC_READ, OPEN_EXISTING)?;
        let log = open_inheritable(log_path, FILE_APPEND_DATA, OPEN_ALWAYS)?;
        let inherited: [HANDLE; 2] = [stdin.0, log.0];
        let mut attributes = AttributeList::new()?;
        // SAFETY: `inherited` outlives CreateProcessW below, and the size matches the array.
        if unsafe {
            UpdateProcThreadAttribute(
                attributes.as_ptr(),
                0,
                PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                inherited.as_ptr().cast(),
                size_of::<[HANDLE; 2]>(),
                null_mut(),
                null(),
            )
        } == 0
        {
            return Err(os_error("UpdateProcThreadAttribute"));
        }

        // SAFETY: STARTUPINFOEXW is plain data; zero is its documented initial state.
        let mut startup: STARTUPINFOEXW = unsafe { zeroed() };
        startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
        startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        startup.StartupInfo.hStdInput = stdin.0;
        startup.StartupInfo.hStdOutput = log.0;
        startup.StartupInfo.hStdError = log.0;
        startup.lpAttributeList = attributes.as_ptr();

        let mut command_line = wide(&windows_command_line(program, args));
        let application = wide(program);
        let directory = cwd.map(wide);
        let environment = environment_block(env);
        // SAFETY: PROCESS_INFORMATION is plain data filled by CreateProcessW.
        let mut process: PROCESS_INFORMATION = unsafe { zeroed() };
        // SAFETY: every pointer is NUL-terminated or a valid struct that outlives the call; the command line
        // buffer is mutable as CreateProcessW requires.
        let created = unsafe {
            CreateProcessW(
                application.as_ptr(),
                command_line.as_mut_ptr(),
                null(),
                null(),
                1,
                EXTENDED_STARTUPINFO_PRESENT
                    | CREATE_UNICODE_ENVIRONMENT
                    | DETACHED_PROCESS
                    | CREATE_NEW_PROCESS_GROUP,
                environment.as_ptr().cast(),
                directory.as_ref().map_or(null(), |path| path.as_ptr()),
                &startup.StartupInfo,
                &mut process,
            )
        };
        if created == 0 {
            return Err(os_error("CreateProcessW"));
        }
        drop(OwnedHandle(process.hThread));
        drop(OwnedHandle(process.hProcess));
        Ok(process.dwProcessId)
    }
}

#[cfg(test)]
mod tests {
    use super::windows_command_line;

    #[test]
    fn quotes_windows_arguments_for_the_c_runtime_parser() {
        let args = [
            r"C:\Users\A B\.rerail\dist\daemon.js".to_owned(),
            "--database".to_owned(),
            r#"C:\work\say "hi"\x"#.to_owned(),
            r"C:\trailing\".to_owned(),
            String::new(),
        ];
        assert_eq!(
            windows_command_line(r"C:\Program Files\nodejs\node.exe", &args),
            r#""C:\Program Files\nodejs\node.exe" "C:\Users\A B\.rerail\dist\daemon.js" --database "C:\work\say \"hi\"\x" C:\trailing\ """#,
        );
    }
}
