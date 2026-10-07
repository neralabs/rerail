//! Per-node values: what a literal, or an expression folded from literals, evaluates to. A value
//! is `Unknown` whenever the engine cannot be sure, so every predicate over values is three-valued.

pub(crate) mod ecma;
pub(crate) mod key;
pub(crate) mod names;
pub(crate) mod php;
pub(crate) mod php_compare;
pub(crate) mod php_structure;
pub(crate) mod python;
pub(crate) mod structure;
pub(crate) mod text_regex;

use crate::policy_dsl::language::GrammarDialect;
use crate::policy_dsl::tape::TapeDoc;
use ast_grep_core::tree_sitter::StrDoc;
use ast_grep_core::{Doc, Node};
pub(crate) use num_bigint::BigInt;
use num_traits::{FromPrimitive, Signed, ToPrimitive, Zero};
use std::cmp::Ordering;
use std::ops::Range;

/// Nesting a decode follows before it gives up (`Unknown`).
pub(crate) const MAX_DEPTH: usize = 256;
/// Bytes a decoded string or bytes value may hold: UTF-8 for Python `str`, two per UTF-16 code
/// unit for JavaScript strings. An operation that would build a longer one is `Unknown` before it
/// allocates.
pub(crate) const MAX_STRING_BYTES: usize = 64 * 1024;
/// Elements a tuple or array built by an operation (`(1,) * n`, a spread) may hold.
pub(crate) const MAX_ELEMENTS: usize = 4096;
/// Bits a Python `int` may hold, sign aside (`2 ** 4095` does, `2 ** 4096` does not). A literal or
/// an operation past it is `Unknown`; an operation checks before it computes (`2 ** 10**9`).
pub(crate) const MAX_INT_BITS: u64 = 4096;

/// A node of a file, by its byte range and kind: a binding's site, value or annotation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct NodeRef {
    pub(crate) start: u32,
    pub(crate) end: u32,
    pub(crate) kind: u16,
}

impl NodeRef {
    pub(crate) fn of<D: Doc>(node: &Node<'_, D>) -> Self {
        let range = node.range();
        Self {
            start: range.start as u32,
            end: range.end as u32,
            kind: node.kind_id(),
        }
    }

    #[cfg(test)]
    pub(crate) fn range(&self) -> Range<usize> {
        self.start as usize..self.end as usize
    }

    /// The node this names in `root`'s tree: the outermost node of that range and kind, first in
    /// preorder. Siblings may overlap (a Blade tape's template tree and its embedded `program`,
    /// a marker and the loop it opens), so a descent that finds no such node tries the next
    /// sibling that also covers the range. Siblings are read only up to the one the descent
    /// takes, and past it only when it fails: a lookup in a class of a thousand methods reads
    /// the methods before the one holding the node, not all of them.
    pub(crate) fn find<'r, D: Doc>(&self, root: Node<'r, D>) -> Option<Node<'r, D>> {
        let (start, end) = (self.start as usize, self.end as usize);
        let covers = |child: &Node<'r, D>| {
            let range = child.range();
            range.start <= start && end <= range.end && range.start < range.end
        };
        // The first child of `node` from position `from` on that covers the range, and where.
        let covering = |node: &Node<'r, D>, from: usize| {
            node.children()
                .enumerate()
                .skip(from)
                .find(|(_, child)| covers(child))
        };
        // The nodes the descent passed through, and the position of the next child to try there.
        let mut pending: Vec<(Node<'r, D>, usize)> = Vec::new();
        let mut node = root;
        loop {
            let range = node.range();
            if range.start == start && range.end == end && node.kind_id() == self.kind {
                return Some(node);
            }
            let mut next = covering(&node, 0);
            let mut parent = node;
            node = loop {
                match next {
                    Some((at, child)) => {
                        pending.push((parent, at + 1));
                        break child;
                    }
                    None => {
                        let (up, from) = pending.pop()?;
                        next = covering(&up, from);
                        parent = up;
                    }
                }
            };
        }
    }

    /// `find` in `root`'s tree, searched from `near`, a node of the same tree. In a tree whose
    /// siblings never overlap (every grammar's but Blade's template), the nodes covering a nonempty
    /// range form one path from the root, so the descent starts at the topmost node of `near`'s
    /// nearest covering ancestor's range instead of reading every sibling before that path from
    /// the root down: a read in a class of a thousand methods reads its own method's statements.
    pub(crate) fn find_near<'r, D: Doc>(
        &self,
        root: Node<'r, D>,
        near: &Node<'r, D>,
    ) -> Option<Node<'r, D>> {
        let (start, end) = (self.start as usize, self.end as usize);
        // A file's tree, native or taped; a parsed literal's tree has no dialect to vouch for it.
        let disjoint = || {
            let doc: &dyn std::any::Any = root.get_doc();
            let dialect = match doc.downcast_ref::<StrDoc<GrammarDialect>>() {
                Some(doc) => Some(*doc.get_lang()),
                None => doc.downcast_ref::<TapeDoc>().map(|doc| *doc.get_lang()),
            };
            dialect.is_some_and(|dialect| dialect != GrammarDialect::Blade)
        };
        if start >= end || root.parent().is_some() || !disjoint() {
            return self.find(root);
        }
        let covers = |node: &Node<'r, D>| {
            let range = node.range();
            range.start <= start && end <= range.end
        };
        let mut from = near.clone();
        while !covers(&from) {
            from = match from.parent() {
                Some(parent) => parent,
                None => return self.find(root),
            };
        }
        // An ancestor of the same range comes first in preorder.
        while let Some(parent) = from
            .parent()
            .filter(|parent| parent.range() == from.range())
        {
            from = parent;
        }
        self.find(from)
    }
}

/// Binding hops one decode follows (`value.follow`); a name past them is `Unknown`.
pub(crate) const MAX_HOPS: usize = 16;

/// Whether the name at a range is an unshadowed builtin or global: `Some(false)` when a file-local
/// binding shadows it, `None` when that is not known.
pub(crate) type GlobalLookup<'a> = dyn FnMut(Range<usize>, &str) -> Option<bool> + 'a;

/// The value a name at a range is bound to, when its binding carries one (`value.follow`).
pub(crate) type BindingLookup<'a> = dyn FnMut(Range<usize>, &str) -> Option<Followed> + 'a;

/// The value node of the constant a node fetches (a PHP named or class constant), where the file
/// declares it (`value.follow`).
pub(crate) type ConstantLookup<'a> = dyn FnMut(NodeRef) -> Option<NodeRef> + 'a;

/// A binding's value node, whether a reference mutates a property of the value (TypeScript), and
/// whether the value is an element of the site's value (a destructured or unpacked name) rather
/// than all of it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Followed {
    pub(crate) value: NodeRef,
    pub(crate) mutated: bool,
    pub(crate) destructured: bool,
}

/// One decode's limits: its nesting and the caller's step budget. Each node decoded spends a step;
/// a spent budget stops the decode (the caller decides what that costs the rule). Builtin names
/// (`undefined`, `list`) are values only where `global` says they are not shadowed; other names
/// are values only through `binding`, at most `MAX_HOPS` times per decode.
pub(crate) struct Context<'a> {
    depth: usize,
    spend: &'a mut dyn FnMut() -> bool,
    global: Option<&'a mut GlobalLookup<'a>>,
    binding: Option<&'a mut BindingLookup<'a>>,
    constant: Option<&'a mut ConstantLookup<'a>>,
    hops: usize,
    exhausted: bool,
}

impl<'a> Context<'a> {
    /// A context where no name is known to be a builtin (structures read no names' values).
    pub(crate) fn new(spend: &'a mut dyn FnMut() -> bool) -> Self {
        Self {
            depth: 0,
            spend,
            global: None,
            binding: None,
            constant: None,
            hops: 0,
            exhausted: false,
        }
    }

    pub(crate) fn with_globals(
        spend: &'a mut dyn FnMut() -> bool,
        global: &'a mut GlobalLookup<'a>,
    ) -> Self {
        Self {
            depth: 0,
            spend,
            global: Some(global),
            binding: None,
            constant: None,
            hops: 0,
            exhausted: false,
        }
    }

    /// Follows names through their bindings' values (`value.follow`).
    pub(crate) fn following(mut self, binding: &'a mut BindingLookup<'a>) -> Self {
        self.binding = Some(binding);
        self
    }

    /// Follows constants to their declarations' values (`value.follow`).
    pub(crate) fn following_constants(mut self, constant: &'a mut ConstantLookup<'a>) -> Self {
        self.constant = Some(constant);
        self
    }

    /// Whether `name` at `range` is the unshadowed builtin or global.
    fn is_global(&mut self, range: Range<usize>, name: &str) -> bool {
        self.global
            .as_mut()
            .is_some_and(|global| global(range, name) == Some(true))
    }

    /// The value node the name at `range` is bound to, spending one of the decode's hops.
    fn follow(&mut self, range: Range<usize>, name: &str) -> Option<Followed> {
        let binding = self.binding.as_mut()?;
        if self.hops >= MAX_HOPS {
            return None;
        }
        self.hops += 1;
        binding(range, name)
    }

    /// The value node of the constant `node` fetches, spending one of the decode's hops.
    fn constant(&mut self, node: NodeRef) -> Option<NodeRef> {
        let constant = self.constant.as_mut()?;
        if self.hops >= MAX_HOPS {
            return None;
        }
        self.hops += 1;
        constant(node)
    }

    /// Whether the budget ran out during this decode.
    pub(crate) fn exhausted(&self) -> bool {
        self.exhausted
    }

    fn enter(&mut self) -> bool {
        if self.exhausted || self.depth >= MAX_DEPTH {
            return false;
        }
        if !(self.spend)() {
            self.exhausted = true;
            return false;
        }
        self.depth += 1;
        true
    }

    fn leave(&mut self) {
        self.depth -= 1;
    }
}

/// The language whose semantics a value follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Lang {
    Python,
    Js,
    Php,
}

/// A container whose elements are not all known: its kind, its length when known, and whether it
/// is empty when that is known even though its length is not (`{**a, 1: 2}` is not).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ContainerKind {
    /// Python `tuple` with an element that is not known.
    Tuple,
    List,
    Set,
    Dict,
    /// JavaScript array with an element that is not known.
    Array,
    /// JavaScript object literal.
    Object,
    /// PHP array with a key or an element that is not known.
    PhpArray,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Value {
    Unknown,
    /// Python `None`, JavaScript `null`.
    None,
    /// JavaScript `undefined`.
    Undefined,
    /// Python `...`.
    Ellipsis,
    Bool(bool),
    /// A Python `int` of at most `MAX_INT_BITS` bits.
    Int(BigInt),
    /// A Python `float` or a JavaScript number.
    Float(f64),
    /// A Python `complex`: real and imaginary parts.
    Complex(f64, f64),
    /// A JavaScript BigInt that fits in 128 bits.
    BigInt(i128),
    /// A Python `str` without lone surrogates.
    Str(String),
    /// A JavaScript string, as UTF-16 code units.
    Utf16(Vec<u16>),
    Bytes(Vec<u8>),
    /// A Python tuple whose elements are all known.
    Tuple(Vec<Value>),
    /// A JavaScript array whose elements are all known (holes are `undefined`).
    Array(Vec<Value>),
    /// A JavaScript RegExp object, as a regex literal evaluates to (`getStaticValue`'s `new
    /// RegExp(pattern, flags)`): an object, truthy, and a fresh one at each evaluation. It holds
    /// what it converts to (`String(re)`, `/source/flags` with the flags in `flags`' order), as
    /// UTF-16 code units.
    RegExp(Vec<u16>),
    /// A PHP string: bytes.
    PhpStr(Vec<u8>),
    /// A PHP array whose keys and values are all known: an ordered map, in insertion order.
    PhpArray(Vec<(php::Key, Value)>),
    Container {
        kind: ContainerKind,
        len: Option<usize>,
        /// Whether it has an element; known for a literal with one element that is not a splat.
        nonempty: Option<bool>,
    },
}

impl Value {
    /// A Python `int`, or `Unknown` past `MAX_INT_BITS`.
    pub(crate) fn int(value: impl Into<BigInt>) -> Self {
        let value = value.into();
        if value.bits() > MAX_INT_BITS {
            Self::Unknown
        } else {
            Self::Int(value)
        }
    }

    pub(crate) fn is_known(&self) -> bool {
        !matches!(self, Self::Unknown)
    }

    /// Whether the whole value is known: not `Unknown`, and not a container with unknown elements.
    pub(crate) fn is_constant(&self) -> bool {
        !matches!(self, Self::Unknown | Self::Container { .. })
    }

    /// Truthiness: Python's `bool(v)`, JavaScript's ToBoolean (objects and arrays are truthy even
    /// when empty), PHP's `(bool)`. `None` when it is not known.
    pub(crate) fn truthy(&self, lang: Lang) -> Option<bool> {
        if lang == Lang::Php {
            return php::truthy(self);
        }
        Some(match self {
            Self::Unknown => return None,
            Self::None | Self::Undefined => false,
            Self::Ellipsis => true,
            Self::Bool(value) => *value,
            Self::Int(value) => !value.is_zero(),
            Self::BigInt(value) => *value != 0,
            // NaN is truthy in Python, falsy in JavaScript.
            Self::Float(value) => *value != 0.0 && (lang == Lang::Python || !value.is_nan()),
            Self::Complex(real, imag) => *real != 0.0 || *imag != 0.0,
            Self::Str(value) => !value.is_empty(),
            Self::Utf16(value) => !value.is_empty(),
            Self::Bytes(value) => !value.is_empty(),
            Self::Tuple(value) => !value.is_empty(),
            Self::Array(_)
            | Self::RegExp(_)
            | Self::Container {
                kind: ContainerKind::Array | ContainerKind::Object,
                ..
            } => true,
            Self::Container { nonempty, .. } => return *nonempty,
            Self::PhpStr(_) | Self::PhpArray(_) => return None,
        })
    }

    /// The type's name: Python's `type(v).__name__`, for JavaScript `typeof` refined with `null`,
    /// `array` and `regexp`, PHP's `get_debug_type`.
    pub(crate) fn type_name(&self, lang: Lang) -> Option<&'static str> {
        if lang == Lang::Php {
            return php::type_name(self);
        }
        Some(match (self, lang) {
            (Self::Unknown, _) => return None,
            (Self::None, Lang::Python) => "NoneType",
            (Self::None, _) => "null",
            (Self::Undefined, _) => "undefined",
            (Self::Ellipsis, _) => "ellipsis",
            (Self::Bool(_), Lang::Python) => "bool",
            (Self::Bool(_), _) => "boolean",
            (Self::Int(_), _) => "int",
            (Self::Float(_), Lang::Python) => "float",
            (Self::Float(_), _) => "number",
            (Self::Complex(..), _) => "complex",
            (Self::BigInt(_), _) => "bigint",
            (Self::Str(_), _) => "str",
            (Self::Utf16(_), _) => "string",
            (Self::Bytes(_), _) => "bytes",
            (Self::Tuple(_), _) => "tuple",
            (Self::Array(_), _) => "array",
            (Self::RegExp(_), _) => "regexp",
            (Self::PhpStr(_) | Self::PhpArray(_), _) => return None,
            (Self::Container { kind, .. }, _) => match kind {
                ContainerKind::Tuple => "tuple",
                ContainerKind::List => "list",
                ContainerKind::Set => "set",
                ContainerKind::Dict => "dict",
                ContainerKind::Array | ContainerKind::PhpArray => "array",
                ContainerKind::Object => "object",
            },
        })
    }

    /// Whether the value is of the named type (`type_name`). A RegExp is also an `object`: its
    /// `typeof`, which `regexp` refines without replacing (unlike `null` and `array`).
    pub(crate) fn is_type(&self, lang: Lang, name: &str) -> bool {
        self.type_name(lang)
            .is_some_and(|own| own == name || (own == "regexp" && name == "object"))
    }

    /// Equality in the language's own sense: Python `==`, JavaScript `Object.is`, PHP `===`.
    /// `None` when it depends on elements that are not known.
    pub(crate) fn equals(&self, other: &Self, lang: Lang) -> Option<bool> {
        match lang {
            Lang::Python => python_equals(self, other),
            Lang::Js => js_same_value(self, other),
            Lang::Php => php::identical(self, other),
        }
    }

    /// Order in the language's own sense: numbers numerically, Python strings by code point and
    /// JavaScript strings by UTF-16 code unit, bytes and tuples lexicographically, PHP values as
    /// PHP 8's `<=>` compares them. `Some(None)` when the two do not order (different types, NaN,
    /// PHP arrays neither of which PHP finds smaller); `None` when it is not known.
    pub(crate) fn order(&self, other: &Self, lang: Lang) -> Option<Option<Ordering>> {
        if lang == Lang::Php {
            return php::order(self, other);
        }
        if !self.is_known() || !other.is_known() {
            return None;
        }
        if let (Some(left), Some(right)) = (Number::of(self, lang), Number::of(other, lang)) {
            return Some(left.order(&right));
        }
        Some(match (self, other) {
            (Self::Str(left), Self::Str(right)) => Some(left.cmp(right)),
            (Self::Utf16(left), Self::Utf16(right)) => Some(left.cmp(right)),
            (Self::Bytes(left), Self::Bytes(right)) => Some(left.cmp(right)),
            (Self::Tuple(left), Self::Tuple(right)) if lang == Lang::Python => {
                // Python compares the first unequal pair, then the lengths.
                for (left, right) in left.iter().zip(right) {
                    if python_equals(left, right)? {
                        continue;
                    }
                    return left.order(right, lang);
                }
                Some(left.len().cmp(&right.len()))
            }
            (Self::Container { .. }, _) | (_, Self::Container { .. }) => return None,
            _ => None,
        })
    }
}

/// A number for comparison: exact integers, floats, or (Python) complex numbers.
enum Number {
    Int(BigInt),
    Float(f64),
    Complex(f64, f64),
}

impl Number {
    fn of(value: &Value, lang: Lang) -> Option<Self> {
        match (value, lang) {
            (Value::Bool(value), Lang::Python) => Some(Self::Int(BigInt::from(u8::from(*value)))),
            (Value::Int(value), _) => Some(Self::Int(value.clone())),
            (Value::BigInt(value), _) => Some(Self::Int(BigInt::from(*value))),
            (Value::Float(value), _) => Some(Self::Float(*value)),
            (Value::Complex(real, imag), _) => Some(Self::Complex(*real, *imag)),
            _ => None,
        }
    }

    fn equals(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Complex(real, imag), other) | (other, Self::Complex(real, imag)) => {
                match other {
                    Self::Complex(other_real, other_imag) => {
                        real == other_real && imag == other_imag
                    }
                    other => {
                        *imag == 0.0 && Self::Float(*real).order(other) == Some(Ordering::Equal)
                    }
                }
            }
            _ => self.order(other) == Some(Ordering::Equal),
        }
    }

    /// Exact order, as Python compares `int` with `float`; complex numbers do not order.
    fn order(&self, other: &Self) -> Option<Ordering> {
        match (self, other) {
            (Self::Int(left), Self::Int(right)) => Some(left.cmp(right)),
            (Self::Float(left), Self::Float(right)) => left.partial_cmp(right),
            (Self::Int(left), Self::Float(right)) => int_float_order(left, *right),
            (Self::Float(left), Self::Int(right)) => {
                int_float_order(right, *left).map(Ordering::reverse)
            }
            _ => None,
        }
    }
}

/// `int` against `float` without rounding either.
pub(crate) fn int_float_order(int: &BigInt, float: f64) -> Option<Ordering> {
    if float.is_nan() {
        return None;
    }
    if float.is_infinite() {
        return Some(if float > 0.0 {
            Ordering::Less
        } else {
            Ordering::Greater
        });
    }
    let whole = float.trunc();
    // A finite float's integral part converts exactly.
    let whole_int = BigInt::from_f64(whole)?;
    Some(int.cmp(&whole_int).then_with(|| {
        let fraction = float - whole;
        if fraction > 0.0 {
            Ordering::Less
        } else if fraction < 0.0 {
            Ordering::Greater
        } else {
            Ordering::Equal
        }
    }))
}

/// An `int` as the nearest `float` (ties to even), as CPython's `float(int)`; `None` where that
/// overflows (`OverflowError`).
pub(crate) fn int_to_float(int: &BigInt) -> Option<f64> {
    let magnitude = int.abs().to_f64().filter(|value| value.is_finite())?;
    Some(if int.is_negative() {
        -magnitude
    } else {
        magnitude
    })
}

fn python_equals(left: &Value, right: &Value) -> Option<bool> {
    if let (Some(left), Some(right)) = (
        Number::of(left, Lang::Python),
        Number::of(right, Lang::Python),
    ) {
        return Some(left.equals(&right));
    }
    Some(match (left, right) {
        (Value::Unknown, _) | (_, Value::Unknown) => return None,
        (Value::None, Value::None) | (Value::Ellipsis, Value::Ellipsis) => true,
        (Value::Str(left), Value::Str(right)) => left == right,
        (Value::Bytes(left), Value::Bytes(right)) => left == right,
        (Value::Tuple(left), Value::Tuple(right)) => {
            if left.len() != right.len() {
                return Some(false);
            }
            let mut all = Some(true);
            for (left, right) in left.iter().zip(right) {
                match python_equals(left, right) {
                    Some(false) => return Some(false),
                    Some(true) => {}
                    None => all = None,
                }
            }
            return all;
        }
        // Containers equal only their own kind; a `tuple` literal with an unknown element may
        // still equal a known tuple.
        (
            Value::Container {
                kind: ContainerKind::Tuple,
                len,
                ..
            },
            Value::Tuple(elements),
        )
        | (
            Value::Tuple(elements),
            Value::Container {
                kind: ContainerKind::Tuple,
                len,
                ..
            },
        ) => {
            return if len.is_some_and(|len| len != elements.len()) {
                Some(false)
            } else {
                None
            };
        }
        (
            Value::Container {
                kind: left_kind,
                len: left_len,
                ..
            },
            Value::Container {
                kind: right_kind,
                len: right_len,
                ..
            },
        ) => {
            if left_kind != right_kind {
                return Some(false);
            }
            return match (left_len, right_len) {
                (Some(left), Some(right)) if left != right => Some(false),
                _ => None,
            };
        }
        _ => false,
    })
}

/// `Object.is`: numbers by value except that NaN is NaN and `0` is not `-0`; strings by code
/// units; two literals of an object or array are two objects.
fn js_same_value(left: &Value, right: &Value) -> Option<bool> {
    Some(match (left, right) {
        (Value::Unknown, _) | (_, Value::Unknown) => return None,
        (Value::None, Value::None) | (Value::Undefined, Value::Undefined) => true,
        (Value::Bool(left), Value::Bool(right)) => left == right,
        (Value::Float(left), Value::Float(right)) => {
            (left.is_nan() && right.is_nan()) || left.to_bits() == right.to_bits()
        }
        (Value::BigInt(left), Value::BigInt(right)) => left == right,
        (Value::Utf16(left), Value::Utf16(right)) => left == right,
        _ => false,
    })
}

#[cfg(test)]
mod eslint_rules;
#[cfg(test)]
mod rules;
#[cfg(test)]
pub(crate) mod tests;
