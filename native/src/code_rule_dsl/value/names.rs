//! Unicode character names as Python's `\N{...}` escape reads them (CPython 3.13, Unicode 15.1.0):
//! stored names and aliases case-insensitively, CJK unified ideographs and Hangul syllables by
//! their derived names, in upper case only. Named sequences are not characters and do not resolve.
//! `names/names.bin` comes from `scripts/generate/value/names.py`, which checks every name against CPython.

use std::cell::Cell;
use std::cmp::Ordering;
use std::sync::LazyLock;

const TABLE_BYTES: &[u8] = include_bytes!("names/names.bin");
const MAGIC: &[u8] = b"TNAMES01";
const LFS_POINTER: &[u8] = b"version https://git-lfs";
const CJK: &str = "CJK UNIFIED IDEOGRAPH-";
const HANGUL: &str = "HANGUL SYLLABLE ";

/// The failure a clause reports where it read a `\N{...}` escape the malformed table could not
/// answer (`take_unavailable`). `native/build.rs` refuses to build with such a table, so only a
/// table corrupted past its header gets here.
pub(crate) const TABLE_UNAVAILABLE: &str = "the engine's Unicode names table (names.bin) is malformed: rebuild the engine after `git lfs install && git lfs pull`";

thread_local! {
    /// Whether a lookup on this thread found the table malformed since `take_unavailable` last
    /// cleared it.
    static UNAVAILABLE: Cell<bool> = const { Cell::new(false) };
}

/// Whether a `\N{...}` lookup on this thread found the table malformed since the last call: the
/// name read as unknown, so the clause reading it has no verdict of its own.
pub(crate) fn take_unavailable() -> bool {
    UNAVAILABLE.replace(false)
}

/// The table's parts: the Unicode version, the CJK ranges, and the name buckets.
struct Table<'a> {
    version: &'a str,
    ranges: &'a [u8],
    buckets: &'a [u8],
    entries: &'a [u8],
}

fn u16_at(bytes: &[u8], at: usize) -> usize {
    usize::from(u16::from_le_bytes([bytes[at], bytes[at + 1]]))
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

/// The next `len` bytes at `at`, which then moves past them.
fn take<'a>(bytes: &'a [u8], at: &mut usize, len: usize) -> Result<&'a [u8], String> {
    let end = at
        .checked_add(len)
        .filter(|end| *end <= bytes.len())
        .ok_or_else(|| format!("names.bin is truncated at byte {at}"))?;
    let taken = &bytes[*at..end];
    *at = end;
    Ok(taken)
}

/// Reads and checks the whole table: its header, and that every bucket starts an entry and every
/// entry lies within the table, so the lookups below index it without bounds failures.
fn parse(bytes: &[u8]) -> Result<Table<'_>, String> {
    if bytes.starts_with(LFS_POINTER) {
        return Err("names.bin is a Git LFS pointer, not the names table".to_owned());
    }
    if !bytes.starts_with(MAGIC) {
        return Err("names.bin has no TNAMES01 header".to_owned());
    }
    let mut at = MAGIC.len();
    let version_len = usize::from(take(bytes, &mut at, 1)?[0]);
    let version = std::str::from_utf8(take(bytes, &mut at, version_len)?)
        .map_err(|_| "names.bin's Unicode version is not text".to_owned())?;
    let range_count = u16_at(take(bytes, &mut at, 2)?, 0);
    let ranges = take(bytes, &mut at, 8 * range_count)?;
    let bucket_count = u32_at(take(bytes, &mut at, 8)?, 4) as usize;
    let buckets = take(bytes, &mut at, bucket_count.saturating_mul(4))?;
    let entries = &bytes[at..];
    let mut bucket = 0;
    let mut at = 0;
    while at < entries.len() {
        if bucket < bucket_count && u32_at(buckets, bucket * 4) as usize == at {
            bucket += 1;
        }
        let len = usize::from(take(entries, &mut at, 2)?[1]);
        take(entries, &mut at, len + 3)?;
    }
    if bucket != bucket_count {
        return Err("names.bin's buckets do not start its entries".to_owned());
    }
    Ok(Table {
        version,
        ranges,
        buckets,
        entries,
    })
}

/// The embedded table, read once per process.
static TABLE: LazyLock<Result<Table<'static>, String>> = LazyLock::new(|| parse(TABLE_BYTES));

/// The Unicode version the table was generated from.
#[cfg(test)]
pub(crate) fn unicode_version() -> &'static str {
    TABLE.as_ref().expect("names table").version
}

/// The character `\N{name}` names, or `None` where CPython raises a SyntaxError. A malformed table
/// answers `None` for every name it would hold, and marks the thread (`take_unavailable`).
pub(crate) fn lookup(name: &str) -> Option<char> {
    lookup_in(&TABLE, name)
}

fn lookup_in(table: &Result<Table<'_>, String>, name: &str) -> Option<char> {
    // Hangul syllables' names are derived from their jamo alone.
    if let Some(jamo) = name.strip_prefix(HANGUL) {
        return syllable(jamo);
    }
    let Ok(table) = table else {
        UNAVAILABLE.set(true);
        return None;
    };
    debug_assert!(!table.version.is_empty());
    if let Some(hex) = name.strip_prefix(CJK) {
        return ideograph(table, hex);
    }
    if !name.is_ascii() {
        return None;
    }
    stored(table, name.to_ascii_uppercase().as_bytes())
}

/// `CJK UNIFIED IDEOGRAPH-XXXX`: four or five upper-case hex digits, within a range.
fn ideograph(table: &Table, hex: &str) -> Option<char> {
    if !matches!(hex.len(), 4 | 5)
        || !hex
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'A'..=b'F').contains(&b))
    {
        return None;
    }
    let code = u32::from_str_radix(hex, 16).ok()?;
    let within = table
        .ranges
        .chunks_exact(8)
        .any(|range| (u32_at(range, 0)..=u32_at(range, 4)).contains(&code));
    within.then(|| char::from_u32(code)).flatten()
}

/// The Hangul jamo short names (leading, vowel, trailing), as CPython's `hangul_syllables`.
const LEADING: [&str; 19] = [
    "G", "GG", "N", "D", "DD", "R", "M", "B", "BB", "S", "SS", "", "J", "JJ", "C", "K", "T", "P",
    "H",
];
const VOWEL: [&str; 21] = [
    "A", "AE", "YA", "YAE", "EO", "E", "YEO", "YE", "O", "WA", "WAE", "OE", "YO", "U", "WEO", "WE",
    "WI", "YU", "EU", "YI", "I",
];
const TRAILING: [&str; 28] = [
    "", "G", "GG", "GS", "N", "NJ", "NH", "D", "L", "LG", "LM", "LB", "LS", "LT", "LP", "LH", "M",
    "B", "BS", "S", "SS", "NG", "J", "C", "K", "T", "P", "H",
];

/// CPython's `find_syllable`: the longest short name the text starts with, and its index.
fn longest(text: &str, names: &[&str]) -> Option<(usize, usize)> {
    names
        .iter()
        .enumerate()
        .filter(|(_, name)| text.starts_with(**name))
        .max_by_key(|(index, name)| (name.len(), std::cmp::Reverse(*index)))
        .map(|(index, name)| (index, name.len()))
}

/// `HANGUL SYLLABLE` and its jamo, each the longest match in turn, with nothing left over.
fn syllable(text: &str) -> Option<char> {
    let (leading, used) = longest(text, &LEADING)?;
    let text = &text[used..];
    let (vowel, used) = longest(text, &VOWEL)?;
    let text = &text[used..];
    let (trailing, used) = longest(text, &TRAILING)?;
    if used != text.len() {
        return None;
    }
    let index = (leading * VOWEL.len() + vowel) * TRAILING.len() + trailing;
    char::from_u32(0xAC00 + u32::try_from(index).ok()?)
}

/// A stored name or alias, already in upper case: a binary search over each bucket's first name,
/// then a scan of that bucket.
fn stored(table: &Table, name: &[u8]) -> Option<char> {
    let bucket_count = table.buckets.len() / 4;
    let first_name = |bucket: usize| {
        let at = u32_at(table.buckets, bucket * 4) as usize;
        let len = usize::from(table.entries[at + 1]);
        &table.entries[at + 2..at + 2 + len]
    };
    // The last bucket whose first name is at most `name`.
    let mut low = 0;
    let mut high = bucket_count;
    while low < high {
        let middle = (low + high) / 2;
        if first_name(middle).cmp(name) == Ordering::Greater {
            high = middle;
        } else {
            low = middle + 1;
        }
    }
    let bucket = low.checked_sub(1)?;
    let mut at = u32_at(table.buckets, bucket * 4) as usize;
    let end = if bucket + 1 < bucket_count {
        u32_at(table.buckets, (bucket + 1) * 4) as usize
    } else {
        table.entries.len()
    };
    let mut current = Vec::with_capacity(64);
    while at < end {
        let shared = usize::from(table.entries[at]);
        let len = usize::from(table.entries[at + 1]);
        current.truncate(shared);
        current.extend_from_slice(&table.entries[at + 2..at + 2 + len]);
        at += 2 + len;
        let code = u32::from(table.entries[at])
            | u32::from(table.entries[at + 1]) << 8
            | u32::from(table.entries[at + 2]) << 16;
        at += 3;
        match current.as_slice().cmp(name) {
            Ordering::Equal => return char::from_u32(code),
            Ordering::Greater => return None,
            Ordering::Less => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_resolve_as_cpython_3_13_reads_them() {
        assert_eq!(unicode_version(), "15.1.0");
        let cases = [
            ("LATIN SMALL LETTER A", Some('a')),
            ("latin small letter a", Some('a')),
            ("Latin Small Letter A", Some('a')),
            ("LATIN SMALL LETTER A ", None),
            ("LATIN  SMALL LETTER A", None),
            ("", None),
            ("BULLET", Some('\u{2022}')),
            ("SPACE", Some(' ')),
            ("NULL", Some('\0')),
            ("EOL", Some('\n')),
            ("BOM", Some('\u{FEFF}')),
            ("latin capital letter gha", Some('\u{01A2}')),
            ("KEYCAP NUMBER SIGN", None),
            ("CJK UNIFIED IDEOGRAPH-4E00", Some('\u{4E00}')),
            ("CJK UNIFIED IDEOGRAPH-04E00", Some('\u{4E00}')),
            ("CJK UNIFIED IDEOGRAPH-4e00", None),
            ("cjk unified ideograph-4E00", None),
            ("CJK UNIFIED IDEOGRAPH-2EBF0", Some('\u{2EBF0}')),
            ("CJK UNIFIED IDEOGRAPH-2EBEF", None),
            ("CJK COMPATIBILITY IDEOGRAPH-F900", Some('\u{F900}')),
            ("TANGUT IDEOGRAPH-17000", None),
            ("HANGUL SYLLABLE GA", Some('\u{AC00}')),
            ("HANGUL SYLLABLE GAG", Some('\u{AC01}')),
            ("HANGUL SYLLABLE A", Some('\u{C544}')),
            ("HANGUL SYLLABLE HIH", Some('\u{D7A3}')),
            ("hangul syllable GA", None),
            ("HANGUL SYLLABLE ga", None),
            ("HANGUL SYLLABLE GAX", None),
            ("TIBETAN MARK TSA -PHRU", Some('\u{0F39}')),
            ("ZERO WIDTH SPACE", Some('\u{200B}')),
            ("\u{e9}", None),
        ];
        for (name, expected) in cases {
            assert_eq!(lookup(name), expected, "{name:?}");
        }
    }

    #[test]
    fn every_hangul_syllable_resolves_by_its_own_name() {
        let mut code = 0xAC00;
        for leading in LEADING {
            for vowel in VOWEL {
                for trailing in TRAILING {
                    let name = format!("{HANGUL}{leading}{vowel}{trailing}");
                    assert_eq!(lookup(&name), char::from_u32(code), "{name}");
                    code += 1;
                }
            }
        }
        assert_eq!(code, 0xD7A4);
    }

    #[test]
    fn every_stored_name_resolves() {
        let table = TABLE.as_ref().expect("names table");
        let mut current = Vec::new();
        let mut at = 0;
        let mut count = 0;
        while at < table.entries.len() {
            let shared = usize::from(table.entries[at]);
            let len = usize::from(table.entries[at + 1]);
            current.truncate(shared);
            current.extend_from_slice(&table.entries[at + 2..at + 2 + len]);
            at += 2 + len + 3;
            let name = String::from_utf8(current.clone()).expect("ASCII");
            assert!(lookup(&name).is_some(), "{name}");
            count += 1;
        }
        assert_eq!(count, 35_301);
    }

    #[test]
    fn a_malformed_table_is_an_error_not_a_panic() {
        let pointer = b"version https://git-lfs.github.com/spec/v1\noid sha256:4d7a\nsize 123\n";
        assert!(parse(pointer).is_err_and(|error| error.contains("Git LFS pointer")));
        assert!(parse(b"").is_err());
        assert!(parse(b"not a table").is_err());
        assert!(parse(MAGIC).is_err());
        // A real header cut short anywhere, entries included, never reads past the end.
        for cut in [9, 20, TABLE_BYTES.len() / 2, TABLE_BYTES.len() - 1] {
            assert!(parse(&TABLE_BYTES[..cut]).is_err(), "cut at {cut}");
        }
        assert!(parse(TABLE_BYTES).is_ok());
    }

    #[test]
    fn lookups_on_a_malformed_table_mark_the_thread() {
        let broken = parse(b"version https://git-lfs.github.com/spec/v1\n");
        take_unavailable();
        // Hangul syllables need no table.
        assert_eq!(lookup_in(&broken, "HANGUL SYLLABLE GA"), Some('\u{AC00}'));
        assert!(!take_unavailable());
        assert_eq!(lookup_in(&broken, "LATIN SMALL LETTER A"), None);
        assert!(take_unavailable());
        assert!(!take_unavailable());
        assert_eq!(lookup_in(&broken, "CJK UNIFIED IDEOGRAPH-4E00"), None);
        assert!(take_unavailable());
        assert_eq!(lookup("LATIN SMALL LETTER A"), Some('a'));
        assert!(!take_unavailable());
    }
}
