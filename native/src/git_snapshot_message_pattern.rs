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

    pub fn matches(&self, message: &[u8]) -> Result<bool, String> {
        let message = message.split(|byte| *byte == 0).next().unwrap_or_default();
        let message = std::ffi::CString::new(message).expect("message was cut at the first NUL");
        match unsafe { libc::regexec(&self.0, message.as_ptr(), 0, std::ptr::null_mut(), 0) } {
            0 => Ok(true),
            libc::REG_NOMATCH => Ok(false),
            status => Err(format!("regexec failed with status {status}")),
        }
    }
}

#[cfg(unix)]
impl Drop for MessagePattern {
    fn drop(&mut self) {
        unsafe { libc::regfree(&mut self.0) };
    }
}

#[cfg(windows)]
pub use portable::Pattern as MessagePattern;

#[cfg(any(windows, all(test, unix)))]
mod portable {
    use onig::{EncodedBytes, MatchParam, Regex, RegexOptions, SearchOptions, Syntax, SyntaxOperator};

    static SYNTAX: std::sync::LazyLock<Syntax> = std::sync::LazyLock::new(|| {
        use SyntaxOperator as Op;
        let mut syntax = *Syntax::posix_extended();
        syntax.disable_operators(Op::SYNTAX_OPERATOR_ESC_CONTROL_CHARS);
        syntax.enable_operators(Op::SYNTAX_OPERATOR_ESC_AZ_BUF_ANCHOR | Op::SYNTAX_OPERATOR_ESC_W_WORD |
            Op::SYNTAX_OPERATOR_ESC_B_WORD_BOUND | Op::SYNTAX_OPERATOR_ESC_LTGT_WORD_BEGIN_END |
            Op::SYNTAX_OPERATOR_ESC_S_WHITE_SPACE);
        syntax
    });

    /// Copy a bracket expression verbatim. Its first member can be ']', and a class such as
    /// [[:digit:]] has its own closing bracket. The compiler diagnoses incomplete expressions.
    fn bracket_len(pattern: &[u8]) -> usize {
        let mut at = 1 + usize::from(pattern.get(1) == Some(&b'^'));
        at += usize::from(pattern.get(at) == Some(&b']'));
        while let Some(&byte) = pattern.get(at) {
            at += 1;
            if byte == b']' { break; }
            if byte == b'[' && let Some(&mark @ (b':' | b'.' | b'=')) = pattern.get(at) {
                if let Some(end) = pattern[at + 1..].windows(2).position(|pair| pair == [mark, b']']) {
                    at += end + 3;
                }
            }
        }
        at
    }

    fn anchors(mut pattern: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(pattern.len());
        while let Some((&byte, rest)) = pattern.split_first() {
            match byte {
                b'[' => {
                    let end = bracket_len(pattern);
                    out.extend_from_slice(&pattern[..end]);
                    pattern = &pattern[end..];
                    continue;
                }
                b'$' => out.extend_from_slice(br"\z"), // Git requires the actual end, even after a final newline.
                b'\\' if !rest.is_empty() => {
                    let escaped = rest[0];
                    match escaped {
                        b'A' | b'Z' | b'z' => out.push(escaped), // Literal in Git's ERE dialect.
                        b'`' => out.extend_from_slice(br"\A"),
                        b'\'' => out.extend_from_slice(br"\z"),
                        _ => out.extend_from_slice(&[byte, escaped]),
                    }
                    pattern = &rest[1..];
                    continue;
                }
                _ => out.push(byte),
            }
            pattern = rest;
        }
        out
    }

    pub struct Pattern(Regex);

    impl Pattern {
        pub fn new(pattern: &[u8]) -> Result<Self, String> {
            Regex::with_options_and_encoding(EncodedBytes::ascii(&anchors(pattern)), RegexOptions::REGEX_OPTION_NONE, &SYNTAX)
                .map(Self).map_err(|error| error.to_string())
        }

        pub fn matches(&self, message: &[u8]) -> Result<bool, String> {
            let bytes = message.split(|byte| *byte == 0).next().unwrap_or_default();
            self.0.search_with_param(EncodedBytes::ascii(bytes), 0, bytes.len(), SearchOptions::SEARCH_OPTION_NONE, None, MatchParam::default())
                .map(|found| found.is_some()).map_err(|error| error.to_string())
        }
    }
}

#[cfg(all(test, unix))]
#[test]
fn windows_message_patterns_match_git_extended_expressions() {
    let messages: &[&[u8]] = &[b"seed\n", b"code 12 echo echo\n\nsecond paragraph\n", b"code dd\xff\0hidden"];
    for pattern in [b"^(seed|code)".as_slice(), b"seed$", b"[[:digit:]]{2}", b"echo.*paragraph", b"^second paragraph", br"\d+", b"(?i)SEED", b"[", b"hidden", b"\xff"] {
        let native = MessagePattern::new(pattern);
        let portable = portable::Pattern::new(pattern);
        assert_eq!(native.is_ok(), portable.is_ok(), "{pattern:?}");
        if let (Ok(native), Ok(portable)) = (native, portable) {
            for message in messages {
                assert_eq!(native.matches(message).unwrap(), portable.matches(message).unwrap(), "{pattern:?} in {message:?}");
            }
        }
    }
}

#[cfg(not(any(unix, windows)))]
pub struct MessagePattern(regex::bytes::Regex);

#[cfg(not(any(unix, windows)))]
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

    pub fn matches(&self, message: &[u8]) -> Result<bool, String> {
        Ok(self.0.is_match(message.split(|byte| *byte == 0).next().unwrap_or_default()))
    }
}
