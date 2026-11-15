//! Tag names as tree-sitter-html's scanner reads them (`tag.h`): each name it knows is a type of
//! its own, any other is `CUSTOM` (told apart by its name). The scanner's rules over the types:
//! which elements are void, and which may not contain which (an implicit end tag). tree-sitter-vue's
//! scanner reads tags by the same `tag.h`, so `rerail-vue-lean` reads them here too.

/// The names the scanner knows, upper case, sorted (a name's index is its type, from 1; 0 is
/// `CUSTOM`). `CUSTOM` itself is the scanner's name for its custom type.
const NAMES: &[&str] = &[
    "A", "ABBR", "ADDRESS", "AREA", "ARTICLE", "ASIDE", "AUDIO", "B", "BASE", "BASEFONT", "BDI", "BDO", "BGSOUND",
    "BLOCKQUOTE", "BODY", "BR", "BUTTON", "CANVAS", "CAPTION", "CITE", "CODE", "COL", "COLGROUP", "COMMAND", "CUSTOM",
    "DATA", "DATALIST", "DD", "DEL", "DETAILS", "DFN", "DIALOG", "DIV", "DL", "DT", "EM", "EMBED", "FIELDSET",
    "FIGCAPTION", "FIGURE", "FOOTER", "FORM", "FRAME", "H1", "H2", "H3", "H4", "H5", "H6", "HEAD", "HEADER", "HGROUP",
    "HR", "HTML", "I", "IFRAME", "IMAGE", "IMG", "INPUT", "INS", "ISINDEX", "KBD", "KEYGEN", "LABEL", "LEGEND", "LI",
    "LINK", "MAIN", "MAP", "MARK", "MATH", "MENU", "MENUITEM", "META", "METER", "NAV", "NEXTID", "NOSCRIPT", "OBJECT",
    "OL", "OPTGROUP", "OPTION", "OUTPUT", "P", "PARAM", "PICTURE", "PRE", "PROGRESS", "Q", "RB", "RP", "RT", "RTC",
    "RUBY", "S", "SAMP", "SCRIPT", "SECTION", "SELECT", "SLOT", "SMALL", "SOURCE", "SPAN", "STRONG", "STYLE", "SUB",
    "SUMMARY", "SUP", "SVG", "TABLE", "TBODY", "TD", "TEMPLATE", "TEXTAREA", "TFOOT", "TH", "THEAD", "TIME", "TITLE",
    "TR", "TRACK", "U", "UL", "VAR", "VIDEO", "WBR",
];

/// The void elements (`tag_is_void`: the types before `END_OF_VOID_TAGS`).
const VOID: &[&str] = &[
    "AREA", "BASE", "BASEFONT", "BGSOUND", "BR", "COL", "COMMAND", "EMBED", "FRAME", "HR", "IMAGE", "IMG", "INPUT",
    "ISINDEX", "KEYGEN", "LINK", "MENUITEM", "META", "NEXTID", "PARAM", "SOURCE", "TRACK", "WBR",
];

/// `TAG_TYPES_NOT_ALLOWED_IN_PARAGRAPHS`.
const NOT_IN_P: &[&str] = &[
    "ADDRESS", "ARTICLE", "ASIDE", "BLOCKQUOTE", "DETAILS", "DIV", "DL", "FIELDSET", "FIGCAPTION", "FIGURE", "FOOTER",
    "FORM", "H1", "H2", "H3", "H4", "H5", "H6", "HEADER", "HR", "MAIN", "NAV", "OL", "P", "PRE", "SECTION",
];

/// A name packed into a `u128`, upper case, one byte a character: the key `NAMES` is searched by.
/// `None` where it is longer than 16 bytes (no known name is).
#[inline]
fn pack(name: &[u8]) -> Option<u128> {
    if name.len() > 16 {
        return None;
    }
    let mut k = 0u128;
    for &c in name {
        k = k << 8 | u128::from(c.to_ascii_uppercase());
    }
    Some(k | (name.len() as u128) << 124)
}

const fn pack_const(name: &str) -> u128 {
    let b = name.as_bytes();
    let mut k = 0u128;
    let mut i = 0;
    while i < b.len() {
        k = k << 8 | b[i] as u128;
        i += 1;
    }
    k | (b.len() as u128) << 124
}

const N: usize = NAMES.len();

/// `NAMES`' keys, sorted by key.
static KEYS: [(u128, u8); N] = {
    let mut t = [(0u128, 0u8); N];
    let mut i = 0;
    while i < N {
        t[i] = (pack_const(NAMES[i]), i as u8 + 1);
        i += 1;
    }
    // Insertion sort, at compile time.
    let mut i = 1;
    while i < N {
        let mut j = i;
        while j > 0 && t[j - 1].0 > t[j].0 {
            let x = t[j];
            t[j] = t[j - 1];
            t[j - 1] = x;
            j -= 1;
        }
        i += 1;
    }
    t
};

/// A tag's type: a known name's index from 1, or 0 (`CUSTOM`, the name `CUSTOM` included).
pub fn type_of(name: &[u8]) -> u8 {
    let Some(k) = pack(name) else { return 0 };
    match KEYS.binary_search_by(|e| e.0.cmp(&k)) {
        Ok(i) if KEYS[i].1 != CUSTOM => KEYS[i].1,
        _ => 0,
    }
}

const fn index(name: &str) -> u8 {
    let mut i = 0;
    while i < N {
        if eq(NAMES[i], name) {
            return i as u8 + 1;
        }
        i += 1;
    }
    panic!("a known name");
}

const fn eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

const fn set(names: &[&str]) -> [bool; N + 1] {
    let mut t = [false; N + 1];
    let mut i = 0;
    while i < names.len() {
        t[index(names[i]) as usize] = true;
        i += 1;
    }
    t
}

static IS_VOID: [bool; N + 1] = set(VOID);
static IS_NOT_IN_P: [bool; N + 1] = set(NOT_IN_P);

const CUSTOM: u8 = index("CUSTOM");
pub const SCRIPT: u8 = index("SCRIPT");

/// The scanner's serialized state with no tags open: its two counts (`serialize`).
pub const SERIALIZED_EMPTY: usize = 4;

/// What an open tag of type `ty` (0: custom) with a name of `len` bytes adds to the scanner's
/// serialized state: a byte a known tag; a type, a length and the name a custom one.
pub fn serialized_size(ty: u8, len: usize) -> usize {
    if ty == 0 { 2 + len } else { 1 }
}

/// Whether the scanner, its open tags taking `size` bytes serialized, serializes one more (type
/// `ty`, a name of `len` bytes) whole. Past `TREE_SITTER_SERIALIZATION_BUFFER_SIZE` (1024) its
/// state is cut, and the tags past it come back nameless (an end tag of one is an error); a
/// custom name past 255 bytes is cut too.
pub fn serializes(size: usize, ty: u8, len: usize) -> bool {
    (ty != 0 || len <= 255) && size + serialized_size(ty, len) < 1024
}
pub const STYLE: u8 = index("STYLE");
pub const TEMPLATE: u8 = index("TEMPLATE");
const HTML: u8 = index("HTML");
const HEAD: u8 = index("HEAD");
const BODY: u8 = index("BODY");
const LI: u8 = index("LI");
const DT: u8 = index("DT");
const DD: u8 = index("DD");
const P: u8 = index("P");
const COLGROUP: u8 = index("COLGROUP");
const COL: u8 = index("COL");
const RB: u8 = index("RB");
const RT: u8 = index("RT");
const RP: u8 = index("RP");
const OPTGROUP: u8 = index("OPTGROUP");
const TR: u8 = index("TR");
const TD: u8 = index("TD");
const TH: u8 = index("TH");

pub fn is_void(t: u8) -> bool {
    IS_VOID[usize::from(t)]
}

/// Whether an element of type `parent` may contain one of type `child` (`tag_can_contain`).
pub fn can_contain(parent: u8, child: u8) -> bool {
    match parent {
        LI => child != LI,
        DT | DD => child != DT && child != DD,
        P => !IS_NOT_IN_P[usize::from(child)],
        COLGROUP => child == COL,
        RB | RT | RP => child != RB && child != RT && child != RP,
        OPTGROUP => child != OPTGROUP,
        TR => child != TR,
        TD | TH => child != TD && child != TH && child != TR,
        _ => true,
    }
}

/// Whether an element of type `t` ends at the text's end (`scan_implicit_end_tag` there).
pub fn ends_at_eof(t: u8) -> bool {
    is_void(t) || !can_contain(t, 0) || matches!(t, HTML | HEAD | BODY)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_have_their_types() {
        assert_eq!(NAMES.len(), 126, "tag.h's TAG_TYPES_BY_TAG_NAME, CUSTOM included");
        assert!(NAMES.windows(2).all(|w| w[0] < w[1]));
        for (i, n) in NAMES.iter().enumerate() {
            let want = if *n == "CUSTOM" { 0 } else { i as u8 + 1 };
            assert_eq!(type_of(n.as_bytes()), want, "{n}");
            assert_eq!(type_of(n.to_ascii_lowercase().as_bytes()), want, "{n}");
        }
        assert_eq!(type_of(b"my-element"), 0);
        assert_eq!(type_of(b"averyveryverylongname"), 0);
        assert!(is_void(type_of(b"br")) && !is_void(type_of(b"div")) && !is_void(0));
        assert!(!can_contain(P, type_of(b"div")) && can_contain(P, type_of(b"span")));
        assert!(ends_at_eof(COLGROUP) && ends_at_eof(BODY) && !ends_at_eof(type_of(b"div")) && !ends_at_eof(0));
    }
}
