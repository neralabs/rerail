//! The macro table's pieces: tokens as the expansion carries them, definitions, the built-in
//! macros, and the identifier interner.

use rerail_c_lean::T;
use rerail_c_lean::pp::P;
use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

/// `Tk::flags`: the first token of its line (`pp::BOL`)...
pub const BOL: u8 = rerail_c_lean::pp::BOL;
/// ...white space before it (`pp::SPACE`)...
pub const SPACE: u8 = rerail_c_lean::pp::SPACE;
/// ...spelled as a digraph...
pub const DIGRAPH: u8 = rerail_c_lean::pp::DIGRAPH;
/// ...an identifier never to be expanded again (C17 §6.10.3.4p2: it named a macro being expanded
/// where it was read)...
pub const NOEXP: u8 = 0x20;
/// ...a `##` of a replacement list: the paste operator (an argument's `##` is none)...
pub const PASTE: u8 = 0x40;
/// ...and a placemarker: an empty argument, until the pastes are done.
pub const PLACEMARKER: u8 = 0x80;

/// The symbol of no identifier.
pub const NO_SYM: u32 = u32::MAX;
/// `Tk::sym` of the end of a token list read in isolation (an argument's pre-expansion, an `#if`'s
/// line): the reader stops there.
pub const STOP: u32 = u32::MAX - 1;

/// A token as the preprocessor carries it: its kind, its spelling (`buf` 0 is the scratch text of
/// pastes and strings; `f + 1` is file `f`'s phase-2 text), its identifier's symbol, and the
/// expansion that made it (`exp`, from 1; 0: read from its file).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tk {
    pub kind: P,
    pub punct: T,
    pub flags: u8,
    pub sym: u32,
    pub buf: u32,
    pub start: u32,
    pub end: u32,
    pub exp: u32,
}

impl Tk {
    /// Whether this is punctuator `t`.
    #[inline]
    pub fn is(&self, t: T) -> bool {
        self.kind == P::Punct && self.punct == t
    }

    /// The end of the text: the file's, or a list read in isolation's (`STOP`).
    pub fn eof(sym: u32) -> Tk {
        Tk { kind: P::Eof, punct: T::Eof, flags: 0, sym, buf: 0, start: 0, end: 0, exp: 0 }
    }

    pub fn is_stop(&self) -> bool {
        self.kind == P::Eof && self.sym == STOP
    }

    pub fn is_placemarker(&self) -> bool {
        self.flags & PLACEMARKER != 0
    }
}

/// A built-in macro.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Builtin {
    Line,
    File,
    FileName,
    BaseFile,
    Counter,
    IncludeLevel,
    Date,
    Time,
    Timestamp,
    /// `_Pragma ( string )`.
    Pragma,
    /// `__has_include`, `__has_include_next`.
    HasInclude { next: bool },
    /// `__has_embed`.
    HasEmbed,
    /// A function-like test answered from the configuration (`__has_feature`, `__has_builtin`,
    /// `__has_attribute`, `__has_cpp_attribute`, `__is_identifier`…), by its name.
    Feature,
}

/// The built-ins, by name; `fnlike` where they take an operand.
pub const BUILTINS: &[(&str, Builtin, bool)] = &[
    ("__LINE__", Builtin::Line, false),
    ("__FILE__", Builtin::File, false),
    ("__FILE_NAME__", Builtin::FileName, false),
    ("__BASE_FILE__", Builtin::BaseFile, false),
    ("__COUNTER__", Builtin::Counter, false),
    ("__INCLUDE_LEVEL__", Builtin::IncludeLevel, false),
    ("__DATE__", Builtin::Date, false),
    ("__TIME__", Builtin::Time, false),
    ("__TIMESTAMP__", Builtin::Timestamp, false),
    ("_Pragma", Builtin::Pragma, true),
    ("__has_include", Builtin::HasInclude { next: false }, true),
    ("__has_include_next", Builtin::HasInclude { next: true }, true),
    ("__has_embed", Builtin::HasEmbed, true),
    ("__has_feature", Builtin::Feature, true),
    ("__has_extension", Builtin::Feature, true),
    ("__has_builtin", Builtin::Feature, true),
    ("__has_attribute", Builtin::Feature, true),
    ("__has_cpp_attribute", Builtin::Feature, true),
    ("__has_c_attribute", Builtin::Feature, true),
    ("__has_declspec_attribute", Builtin::Feature, true),
    ("__has_warning", Builtin::Feature, true),
    ("__is_identifier", Builtin::Feature, true),
    ("__is_target_arch", Builtin::Feature, true),
    ("__is_target_vendor", Builtin::Feature, true),
    ("__is_target_os", Builtin::Feature, true),
    ("__is_target_environment", Builtin::Feature, true),
    ("__is_target_variant_os", Builtin::Feature, true),
    ("__is_target_variant_environment", Builtin::Feature, true),
    ("__has_constexpr_builtin", Builtin::Feature, true),
    ("__building_module", Builtin::Feature, true),
];

/// A macro's definition.
#[derive(Clone, Debug)]
pub struct Def {
    pub name: u32,
    pub fnlike: bool,
    /// The parameters' symbols; a variadic macro's last is its variadic one (`__VA_ARGS__`, or
    /// GNU's name).
    pub params: Vec<u32>,
    pub variadic: bool,
    pub body: Vec<Tk>,
    pub builtin: Option<Builtin>,
    /// Being expanded: an identifier naming it is painted (`NOEXP`).
    pub disabled: bool,
    /// Where it was defined: its file and the `#define`'s phase-2 position.
    pub file: u32,
    pub at: u32,
}

impl Def {
    /// The index of the parameter `sym`.
    #[inline]
    pub fn param(&self, sym: u32) -> Option<usize> {
        if !self.fnlike || sym == NO_SYM {
            return None;
        }
        self.params.iter().position(|&p| p == sym)
    }
}

/// FNV-1a over a word: the interner's hash (no keys come from an adversary here).
#[derive(Default)]
pub struct Fnv(u64);

impl Hasher for Fnv {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        let mut h = if self.0 == 0 { 0xcbf2_9ce4_8422_2325 } else { self.0 };
        for &b in bytes {
            h = (h ^ b as u64).wrapping_mul(0x0100_0000_01b3);
        }
        self.0 = h;
    }
}

/// Identifiers as symbols.
#[derive(Default)]
pub struct Interner {
    map: HashMap<Box<[u8]>, u32, BuildHasherDefault<Fnv>>,
    text: Vec<Box<[u8]>>,
}

impl Interner {
    pub fn get(&mut self, w: &[u8]) -> u32 {
        if let Some(&s) = self.map.get(w) {
            return s;
        }
        let s = self.text.len() as u32;
        self.text.push(w.into());
        self.map.insert(w.into(), s);
        s
    }

    /// Symbol `s`'s text.
    pub fn text(&self, s: u32) -> &[u8] {
        &self.text[s as usize]
    }

    pub fn len(&self) -> usize {
        self.text.len()
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }
}
