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
    use onig::{EncodedBytes, MatchParam, Regex, RegexOptions, SearchOptions, Syntax, SyntaxBehavior, SyntaxOperator};

    static SYNTAX: std::sync::LazyLock<Syntax> = std::sync::LazyLock::new(|| {
        use SyntaxOperator as Op;
        let mut syntax = *Syntax::posix_extended();
        syntax.enable_behavior(SyntaxBehavior::SYNTAX_BEHAVIOR_BACKSLASH_ESCAPE_IN_CC);
        syntax.disable_operators(Op::SYNTAX_OPERATOR_ESC_CONTROL_CHARS);
        syntax.enable_operators(Op::SYNTAX_OPERATOR_ESC_AZ_BUF_ANCHOR | Op::SYNTAX_OPERATOR_ESC_W_WORD |
            Op::SYNTAX_OPERATOR_ESC_B_WORD_BOUND | Op::SYNTAX_OPERATOR_ESC_LTGT_WORD_BEGIN_END |
            Op::SYNTAX_OPERATOR_ESC_S_WHITE_SPACE);
        syntax
    });

    /// Git for Windows uses single-byte collating elements. Quote those bytes in the
    /// destination class and keep character classes such as [[:digit:]] intact.
    fn bracket(pattern: &[u8], out: &mut Vec<u8>) -> Result<usize, String> {
        let mut at = 1 + usize::from(pattern.get(1) == Some(&b'^'));
        at += usize::from(pattern.get(at) == Some(&b']'));
        out.extend_from_slice(&pattern[..at]);
        while let Some(&byte) = pattern.get(at) {
            at += 1;
            if byte == b'[' && let Some(&mark @ (b':' | b'.' | b'=')) = pattern.get(at) {
                let name = &pattern[at + 1..];
                let end = name.windows(2).position(|pair| pair == [mark, b']'])
                    .ok_or("unterminated bracket element")?;
                if mark == b':' {
                    out.extend_from_slice(&pattern[at - 1..at + end + 3]);
                } else {
                    let [literal] = name[..end] else { return Err("invalid collating element".into()) };
                    if b"[]-^\\".contains(&literal) { out.push(b'\\'); }
                    out.push(literal);
                }
                at += end + 3;
                continue;
            }
            if byte == b'\\' { out.push(b'\\'); } // A literal backslash in Git's bracket expressions.
            out.push(byte);
            if byte == b']' { break; }
        }
        Ok(at)
    }

    fn interval_bounds(rest: &[u8]) -> Result<(), String> {
        let mut count = 0u32;
        for &byte in rest {
            if byte == b',' { count = 0; continue; }
            if !byte.is_ascii_digit() { break; }
            count = count * 10 + u32::from(byte - b'0');
            if count > 32767 { return Err("Git repetition count exceeds 32767".into()); }
        }
        Ok(())
    }

    fn extended_pattern(mut pattern: &[u8]) -> Result<Vec<u8>, String> {
        if pattern.contains(&0) { return Err("Git message pattern contains a NUL byte".into()); }
        let mut out = Vec::with_capacity(pattern.len());
        while let Some((&byte, rest)) = pattern.split_first() {
            match byte {
                b'[' => {
                    let end = bracket(pattern, &mut out)?;
                    pattern = &pattern[end..];
                    continue;
                }
                b'{' => { interval_bounds(rest)?; out.push(byte); }
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
        Ok(out)
    }

    pub struct Pattern(Regex);

    impl Pattern {
        pub fn new(pattern: &[u8]) -> Result<Self, String> {
            Regex::with_options_and_encoding(EncodedBytes::ascii(&extended_pattern(pattern)?), RegexOptions::REGEX_OPTION_NONE, &SYNTAX)
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
    assert!(portable::Pattern::new(b"a{99999}").is_err());
    // Expected answers from Git for Windows v2.50.1.windows.1's bundled matcher.
    for (pattern, answers) in [
        (br"(echo) \1".as_slice(), [false, true, false]),
        (br"\bcode\s+dd", [false, false, true]),
        (br"[]$]", [false, false, false]),
        (br"[[:alpha:]$]", [true, true, true]),
        (br"[[.e.]]", [true, true, true]),
        (br"[[=e=]]", [true, true, true]),
    ] {
        let regex = portable::Pattern::new(pattern).unwrap();
        for (message, answer) in messages.iter().zip(answers) {
            assert_eq!(regex.matches(message).unwrap(), answer, "{pattern:?} in {message:?}");
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
