//! phpdoc-parser 2.3.5's `Lexer`: one anchored, case-insensitive PCRE alternation over bytes,
//! tried in order at each offset, which stops at the first offset no token matches.

/// The token types, numbered as `Lexer::TOKEN_*`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(super) enum T {
    Reference = 0,
    Union = 1,
    Intersection = 2,
    Nullable = 3,
    OpenParentheses = 4,
    CloseParentheses = 5,
    OpenAngleBracket = 6,
    CloseAngleBracket = 7,
    OpenSquareBracket = 8,
    CloseSquareBracket = 9,
    Comma = 10,
    Variadic = 11,
    DoubleColon = 12,
    DoubleArrow = 13,
    Equal = 14,
    OpenPhpdoc = 15,
    ClosePhpdoc = 16,
    PhpdocTag = 17,
    DoctrineTag = 18,
    Float = 19,
    Integer = 20,
    SingleQuotedString = 21,
    DoubleQuotedString = 22,
    DoctrineAnnotationString = 23,
    Identifier = 24,
    ThisVariable = 25,
    Variable = 26,
    HorizontalWs = 27,
    PhpdocEol = 28,
    Other = 29,
    End = 30,
    Colon = 31,
    Wildcard = 32,
    OpenCurlyBracket = 33,
    CloseCurlyBracket = 34,
    Negated = 35,
    Arrow = 36,
    Comment = 37,
}

impl T {
    /// `Lexer::TOKEN_LABELS`, as parse errors name what they expected.
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Reference | Self::Intersection => "'&'",
            Self::Union => "'|'",
            Self::Nullable => "'?'",
            Self::Negated => "'!'",
            Self::OpenParentheses => "'('",
            Self::CloseParentheses => "')'",
            Self::OpenAngleBracket => "'<'",
            Self::CloseAngleBracket => "'>'",
            Self::OpenSquareBracket => "'['",
            Self::CloseSquareBracket => "']'",
            Self::OpenCurlyBracket => "'{'",
            Self::CloseCurlyBracket => "'}'",
            Self::Comma => "','",
            Self::Comment => "'//'",
            Self::Colon => "':'",
            Self::Variadic => "'...'",
            Self::DoubleColon => "'::'",
            Self::DoubleArrow => "'=>'",
            Self::Arrow => "'->'",
            Self::Equal => "'='",
            Self::OpenPhpdoc => "'/**'",
            Self::ClosePhpdoc => "'*/'",
            Self::PhpdocTag => "TOKEN_PHPDOC_TAG",
            Self::DoctrineTag => "TOKEN_DOCTRINE_TAG",
            Self::PhpdocEol => "TOKEN_PHPDOC_EOL",
            Self::Float => "TOKEN_FLOAT",
            Self::Integer => "TOKEN_INTEGER",
            Self::SingleQuotedString => "TOKEN_SINGLE_QUOTED_STRING",
            Self::DoubleQuotedString => "TOKEN_DOUBLE_QUOTED_STRING",
            Self::DoctrineAnnotationString => "TOKEN_DOCTRINE_ANNOTATION_STRING",
            Self::Identifier => "type",
            Self::ThisVariable => "'$this'",
            Self::Variable => "variable",
            Self::HorizontalWs => "TOKEN_HORIZONTAL_WS",
            Self::Other => "TOKEN_OTHER",
            Self::End => "TOKEN_END",
            Self::Wildcard => "*",
        }
    }
}

/// A token: its type, its bytes `start..end` and its line (from 1).
#[derive(Clone, Copy, Debug)]
pub(super) struct Token {
    pub(super) ty: T,
    pub(super) start: usize,
    pub(super) end: usize,
    pub(super) line: u32,
}

/// PCRE's `\s` without UTF mode: space, `\t`, `\n`, `\v`, `\f`, `\r`.
fn space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

/// `[a-z_\x80-\xFF]` under `i`.
fn name_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_' || byte >= 0x80
}

/// `[0-9a-z_\x80-\xFF]` under `i`.
fn name_byte(byte: u8) -> bool {
    name_start(byte) || byte.is_ascii_digit()
}

/// `Lexer::tokenize`: the tokens, then `TOKEN_END`.
pub(super) fn tokenize(text: &str) -> Vec<Token> {
    let s = text.as_bytes();
    let mut tokens = Vec::new();
    let mut at = 0;
    let mut line = 1;
    while at < s.len() {
        let Some((ty, end)) = token(s, at) else {
            break;
        };
        tokens.push(Token {
            ty,
            start: at,
            end,
            line,
        });
        if ty == T::PhpdocEol {
            line += 1;
        }
        at = end;
    }
    tokens.push(Token {
        ty: T::End,
        start: at,
        end: at,
        line,
    });
    tokens
}

/// The first alternative matching at `at`, and where it ends.
fn token(s: &[u8], at: usize) -> Option<(T, usize)> {
    let get = |i: usize| s.get(i).copied();
    let run = |mut i: usize, ok: &dyn Fn(u8) -> bool| {
        while s.get(i).is_some_and(|b| ok(*b)) {
            i += 1;
        }
        i
    };
    let starts = |i: usize, lit: &[u8]| s.get(i..).is_some_and(|rest| rest.starts_with(lit));
    let first = s[at];
    // TOKEN_HORIZONTAL_WS
    if matches!(first, b'\t' | b' ') {
        return Some((T::HorizontalWs, run(at, &|b| matches!(b, b'\t' | b' '))));
    }
    // TOKEN_IDENTIFIER: `(?:[\\]?+[a-z_\x80-\xFF][0-9a-z_\x80-\xFF-]*+)++`
    {
        let mut i = at;
        loop {
            let segment = if get(i) == Some(b'\\') { i + 1 } else { i };
            if !get(segment).is_some_and(name_start) {
                break;
            }
            i = run(segment + 1, &|b| name_byte(b) || b == b'-');
        }
        if i > at {
            return Some((T::Identifier, i));
        }
    }
    if first == b'$' {
        // TOKEN_THIS_VARIABLE: `\$this(?![0-9a-z_\x80-\xFF])`
        if s.get(at + 1..at + 5)
            .is_some_and(|word| word.eq_ignore_ascii_case(b"this"))
            && !get(at + 5).is_some_and(name_byte)
        {
            return Some((T::ThisVariable, at + 5));
        }
        // TOKEN_VARIABLE
        if get(at + 1).is_some_and(name_start) {
            return Some((T::Variable, run(at + 2, &name_byte)));
        }
    }
    // TOKEN_REFERENCE: `&(?=\s*+(?:[,=)]|\.\.\.|(?:\$(?!this(?![0-9a-z_\x80-\xFF])))))`
    if first == b'&' {
        let i = run(at + 1, &space);
        let reference = match get(i) {
            Some(b',' | b'=' | b')') => true,
            Some(b'.') => starts(i, b"..."),
            Some(b'$') => {
                !s.get(i + 1..i + 5)
                    .is_some_and(|word| word.eq_ignore_ascii_case(b"this"))
                    || get(i + 5).is_some_and(name_byte)
            }
            _ => false,
        };
        return Some((
            if reference {
                T::Reference
            } else {
                T::Intersection
            },
            at + 1,
        ));
    }
    let single = match first {
        b'|' => Some(T::Union),
        b'?' => Some(T::Nullable),
        b'!' => Some(T::Negated),
        b'(' => Some(T::OpenParentheses),
        b')' => Some(T::CloseParentheses),
        b'<' => Some(T::OpenAngleBracket),
        b'>' => Some(T::CloseAngleBracket),
        b'[' => Some(T::OpenSquareBracket),
        b']' => Some(T::CloseSquareBracket),
        b'{' => Some(T::OpenCurlyBracket),
        b'}' => Some(T::CloseCurlyBracket),
        b',' => Some(T::Comma),
        _ => None,
    };
    if let Some(ty) = single {
        return Some((ty, at + 1));
    }
    // TOKEN_COMMENT: `\/\/[^\r\n]*(?=\n|\r|\*/)`, backtracking to the last `*/` of a line the
    // text ends in.
    if starts(at, b"//") {
        let line_end = run(at + 2, &|b| b != b'\r' && b != b'\n');
        if line_end < s.len() {
            return Some((T::Comment, line_end));
        }
        if let Some(close) = (at + 2..line_end).rev().find(|i| starts(*i, b"*/")) {
            return Some((T::Comment, close));
        }
    }
    for (lit, ty) in [
        (&b"..."[..], T::Variadic),
        (b"::", T::DoubleColon),
        (b"=>", T::DoubleArrow),
        (b"->", T::Arrow),
        (b"=", T::Equal),
        (b":", T::Colon),
    ] {
        if starts(at, lit) {
            return Some((ty, at + lit.len()));
        }
    }
    // TOKEN_OPEN_PHPDOC: `/\*\*(?=\s)\x20?+`
    if starts(at, b"/**") && get(at + 3).is_some_and(space) {
        let end = if get(at + 3) == Some(b' ') {
            at + 4
        } else {
            at + 3
        };
        return Some((T::OpenPhpdoc, end));
    }
    if starts(at, b"*/") {
        return Some((T::ClosePhpdoc, at + 2));
    }
    if first == b'@' {
        if let Some(end) = phpdoc_tag(s, at) {
            return Some((T::PhpdocTag, end));
        }
        if let Some(end) = doctrine_tag(s, at) {
            return Some((T::DoctrineTag, end));
        }
    }
    // TOKEN_PHPDOC_EOL: `\r?+\n[\x09\x20]*+(?:\*(?!/)\x20?+)?`
    {
        let newline = if first == b'\r' { at + 1 } else { at };
        if get(newline) == Some(b'\n') {
            let mut end = run(newline + 1, &|b| matches!(b, b'\t' | b' '));
            if get(end) == Some(b'*') && get(end + 1) != Some(b'/') {
                end += 1;
                if get(end) == Some(b' ') {
                    end += 1;
                }
            }
            return Some((T::PhpdocEol, end));
        }
    }
    if let Some(end) = float(s, at) {
        return Some((T::Float, end));
    }
    if let Some(end) = integer(s, at) {
        return Some((T::Integer, end));
    }
    // TOKEN_SINGLE_QUOTED_STRING, TOKEN_DOUBLE_QUOTED_STRING:
    // `'(?:\\[^\r\n]|[^'\r\n\\])*+'`.
    if first == b'\'' || first == b'"' {
        let mut i = at + 1;
        loop {
            match get(i) {
                Some(b'\\') if get(i + 1).is_some_and(|b| b != b'\r' && b != b'\n') => i += 2,
                Some(b) if b != first && b != b'\r' && b != b'\n' && b != b'\\' => i += 1,
                _ => break,
            }
        }
        if get(i) == Some(first) {
            return Some((
                if first == b'\'' {
                    T::SingleQuotedString
                } else {
                    T::DoubleQuotedString
                },
                i + 1,
            ));
        }
    }
    // TOKEN_DOCTRINE_ANNOTATION_STRING: `"(?:""|[^"])*+"`
    if first == b'"' {
        let mut i = at + 1;
        loop {
            if starts(i, b"\"\"") {
                i += 2;
            } else if get(i).is_some_and(|b| b != b'"') {
                i += 1;
            } else {
                break;
            }
        }
        if get(i) == Some(b'"') {
            return Some((T::DoctrineAnnotationString, i + 1));
        }
    }
    if first == b'*' {
        return Some((T::Wildcard, at + 1));
    }
    // TOKEN_OTHER: `(?:(?!\*/)[^\s])++`
    let mut i = at;
    while get(i).is_some_and(|b| !space(b)) && !starts(i, b"*/") {
        i += 1;
    }
    (i > at).then_some((T::Other, i))
}

/// `@(?:[a-z][a-z0-9-\\]+:)?[a-z][a-z0-9-\\]*+`
fn phpdoc_tag(s: &[u8], at: usize) -> Option<usize> {
    let class = |b: &u8| b.is_ascii_alphanumeric() || *b == b'-' || *b == b'\\';
    let rest = |from: usize| from + s[from..].iter().take_while(|b| class(b)).count();
    let letter = |i: usize| s.get(i).is_some_and(u8::is_ascii_alphabetic);
    if letter(at + 1) {
        let prefix = rest(at + 2);
        if prefix > at + 2 && s.get(prefix) == Some(&b':') && letter(prefix + 1) {
            return Some(rest(prefix + 2));
        }
        return Some(rest(at + 2));
    }
    None
}

/// `@[a-z_\\][a-z0-9_\:\\]*[a-z_][a-z0-9_]*`
fn doctrine_tag(s: &[u8], at: usize) -> Option<usize> {
    let head = |b: u8| b.is_ascii_alphabetic() || b == b'_';
    if !s.get(at + 1).is_some_and(|b| head(*b) || *b == b'\\') {
        return None;
    }
    let body = at
        + 2
        + s[at + 2..]
            .iter()
            .take_while(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b':' | b'\\'))
            .count();
    let last = (at + 2..body).rev().find(|i| head(s[*i]))?;
    Some(
        last + 1
            + s[last + 1..]
                .iter()
                .take_while(|b| b.is_ascii_alphanumeric() || **b == b'_')
                .count(),
    )
}

/// The end of `(?:_[0-9]++)*` style groups of `digit` from `i`.
fn groups(s: &[u8], mut i: usize, digit: fn(u8) -> bool) -> usize {
    while s.get(i) == Some(&b'_') && s.get(i + 1).is_some_and(|b| digit(*b)) {
        i += 1;
        while s.get(i).is_some_and(|b| digit(*b)) {
            i += 1;
        }
    }
    i
}

fn digits(s: &[u8], mut i: usize, digit: fn(u8) -> bool) -> usize {
    while s.get(i).is_some_and(|b| digit(*b)) {
        i += 1;
    }
    i
}

fn decimal(b: u8) -> bool {
    b.is_ascii_digit()
}

/// `(?:e[+\-]?[0-9]++(?:_[0-9]++)*)`: its end from `i`, if it matches.
fn exponent(s: &[u8], i: usize) -> Option<usize> {
    if !s.get(i).is_some_and(|b| b.eq_ignore_ascii_case(&b'e')) {
        return None;
    }
    let mut j = i + 1;
    if matches!(s.get(j), Some(b'+' | b'-')) {
        j += 1;
    }
    let end = digits(s, j, decimal);
    (end > j).then(|| groups(s, end, decimal))
}

/// `[0-9]++(?:_[0-9]++)*` (or `[0-9]*+` when `empty`): its end from `i`.
fn mantissa(s: &[u8], i: usize, empty: bool) -> Option<usize> {
    let end = digits(s, i, decimal);
    (empty || end > i).then(|| groups(s, end, decimal))
}

/// TOKEN_FLOAT.
fn float(s: &[u8], at: usize) -> Option<usize> {
    let body = |i: usize| -> Option<usize> {
        // `[0-9]++(?:_[0-9]++)*\.[0-9]*+(?:_[0-9]++)*(?:e...)?`
        if let Some(m) = mantissa(s, i, false)
            && s.get(m) == Some(&b'.')
        {
            let f = groups(s, digits(s, m + 1, decimal), decimal);
            return Some(exponent(s, f).unwrap_or(f));
        }
        // `[0-9]*+(?:_[0-9]++)*\.[0-9]++(?:_[0-9]++)*(?:e...)?`
        if let Some(m) = mantissa(s, i, true)
            && s.get(m) == Some(&b'.')
            && let Some(f) = mantissa(s, m + 1, false)
        {
            return Some(exponent(s, f).unwrap_or(f));
        }
        // `[0-9]++(?:_[0-9]++)*e[+\-]?[0-9]++(?:_[0-9]++)*`
        mantissa(s, i, false).and_then(|m| exponent(s, m))
    };
    if matches!(s.get(at), Some(b'+' | b'-'))
        && let Some(end) = body(at + 1)
    {
        return Some(end);
    }
    body(at)
}

/// Whether a byte is a digit of a radix.
type Digit = fn(u8) -> bool;

/// TOKEN_INTEGER.
fn integer(s: &[u8], at: usize) -> Option<usize> {
    let body = |i: usize| -> Option<usize> {
        if s.get(i) == Some(&b'0') {
            let prefixed: [(u8, Digit); 3] = [
                (b'b', |b| matches!(b, b'0' | b'1')),
                (b'o', |b| matches!(b, b'0'..=b'7')),
                (b'x', |b| b.is_ascii_hexdigit()),
            ];
            for (letter, digit) in prefixed {
                if s.get(i + 1)
                    .is_some_and(|b| b.eq_ignore_ascii_case(&letter))
                {
                    let end = digits(s, i + 2, digit);
                    if end > i + 2 {
                        return Some(groups(s, end, digit));
                    }
                }
            }
        }
        mantissa(s, i, false)
    };
    if matches!(s.get(at), Some(b'+' | b'-'))
        && let Some(end) = body(at + 1)
    {
        return Some(end);
    }
    body(at)
}
