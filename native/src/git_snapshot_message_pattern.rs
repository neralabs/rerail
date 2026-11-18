//! Commit message search uses Git's extended regular expressions.

#[cfg(unix)]
pub struct MessagePattern(libc::regex_t);

#[cfg(unix)]
impl MessagePattern {
    pub fn new(pattern: &[u8]) -> Result<Self, String> {
        use std::ffi::CString;
        use std::mem::MaybeUninit;

        let pattern = CString::new(pattern).map_err(|error| error.to_string())?;
        let mut regex = MaybeUninit::uninit();
        // regcomp initializes the regex on success. It performs all cleanup on failure.
        let status = unsafe { libc::regcomp(regex.as_mut_ptr(), pattern.as_ptr(), libc::REG_EXTENDED | libc::REG_NOSUB) };
        if status != 0 {
            let length = unsafe { libc::regerror(status, regex.as_ptr(), std::ptr::null_mut(), 0) };
            let mut detail = vec![0; length];
            unsafe { libc::regerror(status, regex.as_ptr(), detail.as_mut_ptr().cast(), detail.len()) };
            detail.pop(); // regerror includes the trailing NUL.
            return Err(String::from_utf8_lossy(&detail).into_owned());
        }
        Ok(Self(unsafe { regex.assume_init() }))
    }

    pub fn matches(&self, message: &[u8]) -> bool {
        let message = message.split(|byte| *byte == 0).next().unwrap_or_default();
        let message = std::ffi::CString::new(message).expect("message was cut at the first NUL");
        unsafe { libc::regexec(&self.0, message.as_ptr(), 0, std::ptr::null_mut(), 0) == 0 }
    }
}

#[cfg(unix)]
impl Drop for MessagePattern {
    fn drop(&mut self) {
        unsafe { libc::regfree(&mut self.0) };
    }
}

#[cfg(not(unix))]
pub struct MessagePattern(regex::bytes::Regex);

#[cfg(not(unix))]
impl MessagePattern {
    pub fn new(pattern: &[u8]) -> Result<Self, String> {
        let pattern = std::str::from_utf8(pattern).map_err(|error| error.to_string())?;
        regex::bytes::RegexBuilder::new(pattern)
            .unicode(false)
            .dot_matches_new_line(true)
            .build()
            .map(Self)
            .map_err(|error| error.to_string())
    }

    pub fn matches(&self, message: &[u8]) -> bool {
        self.0.is_match(message.split(|byte| *byte == 0).next().unwrap_or_default())
    }
}
