//! The C tape lowered to the kernel tape (decision 0567 §2.2): C's declarations read for their types and layouts,
//! each function's body written as the kernel tape's nodes, every conversion explicit, every lvalue an address.
//!
//! An expression's type is found before its node is written (`ty`, kept by C node), since a node is written before
//! its operands: a conversion's `Cast` is opened, then its operand written inside it.

use std::collections::HashMap;

use crate::Error;
use crate::ctape::C;
use crate::k::{Builder, Builtin, Kind, M, Op, SYNCS};
use crate::ty::{I32, I64, Struct, Ty, U32, U64, lay_out};

/// A function of the program.
#[derive(Clone, Debug)]
pub struct Func {
    pub name: String,
    pub ret: Ty,
    pub params: Vec<Ty>,
    /// Its node in the kernel tape, once its body is written.
    pub node: Option<usize>,
    pub kernel: bool,
    /// Its parameters' names (a kernel's, for its launch).
    pub names: Vec<String>,
}

/// The program lowered: its kernel tape, and what its launches read (each function's signature, each structure's
/// layout).
pub struct Program {
    pub tape: crate::k::Tape,
    pub funcs: Vec<Func>,
    pub structs: Vec<Struct>,
    /// Each function's stack: its frame and its deepest callee's (no kernel recurses).
    pub stacks: Vec<u32>,
}

impl Program {
    /// The kernel named so.
    pub fn kernel(&self, name: &str) -> Option<u32> {
        self.funcs.iter().position(|f| f.kernel && f.name == name).map(|i| i as u32)
    }
}

#[derive(Clone, Copy, Debug)]
enum Place {
    Local(u32),
    Shared(u32),
    Global(u32),
}

#[derive(Clone, Debug)]
struct Var {
    ty: Ty,
    place: Place,
}

struct L<'a> {
    c: C<'a>,
    src: &'a crate::Source,
    b: Builder,
    structs: Vec<Struct>,
    tags: HashMap<String, u32>,
    typedefs: HashMap<String, Ty>,
    funcs: Vec<Func>,
    fnames: HashMap<String, Vec<u32>>,
    globals: HashMap<String, Var>,
    data: Vec<u8>,
    scopes: Vec<HashMap<String, Var>>,
    /// The function being written: its frame's next free byte, its frame's size, its threadgroup memory's, its
    /// return type.
    at: u32,
    frame: u32,
    shared: u32,
    ret: Ty,
    tys: HashMap<usize, Ty>,
}

type R<T> = Result<T, Error>;

/// The program of `source`'s C tape.
pub fn lower(source: &crate::Source, tape: &rerail_c_lean::Tape) -> R<Program> {
    let mut l = L {
        c: C::new(tape, &source.expanded.text),
        src: source,
        b: Builder::default(),
        structs: Vec::new(),
        tags: HashMap::new(),
        typedefs: HashMap::new(),
        funcs: Vec::new(),
        fnames: HashMap::new(),
        globals: HashMap::new(),
        data: Vec::new(),
        scopes: Vec::new(),
        at: 0,
        frame: 0,
        shared: 0,
        ret: Ty::Void,
        tys: HashMap::new(),
    };
    let root = l.b.open(Kind::Program, &[0]);
    for d in l.c.named_children(l.c.root()) {
        l.top(d)?;
    }
    let data = std::mem::take(&mut l.data);
    let mut words = vec![data.len() as u32];
    words.extend(data.chunks(4).map(|c| {
        let mut w = [0u8; 4];
        w[..c.len()].copy_from_slice(c);
        u32::from_le_bytes(w)
    }));
    l.b.leaf(Kind::Data, &words);
    l.b.set_word(root, 0, l.funcs.len() as u32);
    l.b.close(root);
    let stacks = cut(&mut l.b, &l.funcs)?;
    Ok(Program { tape: l.b.finish(), funcs: l.funcs, structs: l.structs, stacks })
}

/// The cut (decision 0567 §2.3): each node whose subtree reaches a sync point flagged `SYNCS`, a call by whether its
/// function's body does; and each function's stack, its frame and its deepest callee's.
fn cut(b: &mut Builder, funcs: &[Func]) -> R<Vec<u32>> {
    let n = b.len();
    let kind = |b: &Builder, i: usize| Kind::of((b.key(i) & 0x3ff) as u16);
    // Which functions sync, to a fixed point (a call may come before its callee's body).
    let mut fsync = vec![false; funcs.len()];
    let bodies: Vec<(usize, usize)> = funcs.iter().filter_map(|f| f.node).map(|at| (at, b.word(at, 0) as usize)).collect();
    loop {
        let mut changed = false;
        for &(at, id) in &bodies {
            if fsync[id] {
                continue;
            }
            let s = (at..at + b.size(at)).any(|i| match kind(b, i) {
                Kind::Builtin => Builtin::of(b.word(i, 0)).syncs(),
                Kind::Call => fsync[b.word(i, 0) as usize],
                _ => false,
            });
            if s {
                fsync[id] = true;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let mut flag = vec![false; n];
    for i in (0..n).rev() {
        let mut s = match kind(b, i) {
            Kind::Builtin => Builtin::of(b.word(i, 0)).syncs(),
            Kind::Call => fsync[b.word(i, 0) as usize],
            _ => false,
        };
        let (mut at, end) = (i + 1, i + b.size(i));
        while at < end && !s {
            s |= flag[at];
            at += b.size(at);
        }
        flag[i] = s && kind(b, i) != Kind::Func && kind(b, i) != Kind::Program;
    }
    for (i, f) in flag.iter().enumerate() {
        if *f {
            b.set_field(i, SYNCS);
        }
    }
    // Stacks, callees first: a function's calls are of functions declared before it, or its own body's later ones;
    // no kernel recurses, so a depth-first walk ends.
    let mut stacks = vec![u32::MAX; funcs.len()];
    fn stack(b: &Builder, funcs: &[Func], id: usize, stacks: &mut [u32], on: &mut Vec<usize>) -> R<u32> {
        if stacks[id] != u32::MAX {
            return Ok(stacks[id]);
        }
        let Some(at) = funcs[id].node else { return Ok(0) };
        if on.contains(&id) {
            return Err(Error(format!("{} recurses: a kernel's functions do not", funcs[id].name)));
        }
        on.push(id);
        let mut deepest = 0;
        for i in at..at + b.size(at) {
            if Kind::of((b.key(i) & 0x3ff) as u16) == Kind::Call {
                deepest = deepest.max(stack(b, funcs, b.word(i, 0) as usize, stacks, on)?);
            }
        }
        on.pop();
        stacks[id] = b.word(at, 1).next_multiple_of(16) + deepest;
        Ok(stacks[id])
    }
    for id in 0..funcs.len() {
        stack(b, funcs, id, &mut stacks, &mut Vec::new())?;
    }
    Ok(stacks)
}

/// A C integer or floating constant's type and value.
fn literal(text: &str) -> Option<(Ty, u64)> {
    let t = text.to_ascii_lowercase();
    let hex = t.starts_with("0x");
    let float = !hex && (t.contains('.') || t.contains('e') || t.ends_with('f'));
    if float {
        let (body, f32) = match t.strip_suffix('f') {
            Some(b) => (b, true),
            None => (t.as_str(), false),
        };
        let v: f64 = body.parse().ok()?;
        return Some(if f32 { (Ty::Float(32), u64::from((v as f32).to_bits())) } else { (Ty::Float(64), v.to_bits()) });
    }
    let body = t.trim_end_matches(['u', 'l']);
    let suffix = &t[body.len()..];
    let (unsigned, long) = (suffix.contains('u'), suffix.contains('l'));
    let v = if hex {
        u64::from_str_radix(&body[2..], 16).ok()?
    } else if body.len() > 1 && body.starts_with('0') {
        u64::from_str_radix(&body[1..], 8).ok()?
    } else {
        body.parse().ok()?
    };
    let decimal = !hex && !(body.len() > 1 && body.starts_with('0'));
    let fits = |t: &Ty| match t {
        Ty::Int { bits, signed } => {
            let max = if *signed { (1u64 << (bits - 1)) - 1 } else if *bits == 64 { u64::MAX } else { (1u64 << bits) - 1 };
            v <= max
        }
        _ => false,
    };
    let candidates: &[Ty] = match (unsigned, long, decimal) {
        (false, false, true) => &[I32, I64],
        (false, false, false) => &[I32, U32, I64, U64],
        (true, false, _) => &[U32, U64],
        (false, true, true) => &[I64],
        (false, true, false) => &[I64, U64],
        (true, true, _) => &[U64],
    };
    let ty = candidates.iter().find(|t| fits(t)).cloned().unwrap_or(U64);
    Some((ty, v))
}

/// A binary operator's operation.
fn binop(s: &str) -> Option<Op> {
    Some(match s {
        "+" => Op::Add,
        "-" => Op::Sub,
        "*" => Op::Mul,
        "/" => Op::Div,
        "%" => Op::Rem,
        "<<" => Op::Shl,
        ">>" => Op::Shr,
        "&" => Op::And,
        "|" => Op::Or,
        "^" => Op::Xor,
        "==" => Op::Eq,
        "!=" => Op::Ne,
        "<" => Op::Lt,
        "<=" => Op::Le,
        ">" => Op::Gt,
        ">=" => Op::Ge,
        _ => return None,
    })
}

fn words_of(m: M, v: u64) -> [u32; 3] {
    [m.0, v as u32, (v >> 32) as u32]
}

fn decay(t: Ty) -> Ty {
    match t {
        Ty::Array(e, _) => Ty::Ptr(e),
        t => t,
    }
}

/// OpenCL's builtins' fixed results.
fn as_target(name: &str) -> Ty {
    match name {
        "as_int" => I32,
        "as_uint" => U32,
        "as_long" => I64,
        "as_ulong" => U64,
        "as_short" => Ty::Int { bits: 16, signed: true },
        "as_ushort" => Ty::Int { bits: 16, signed: false },
        "as_char" => Ty::Int { bits: 8, signed: true },
        "as_uchar" => Ty::Int { bits: 8, signed: false },
        "as_double" => Ty::Float(64),
        _ => Ty::Float(32),
    }
}

impl L<'_> {
    fn err<T>(&self, at: usize, what: impl std::fmt::Display) -> R<T> {
        let text: String = self.c.text(at).chars().take(80).collect();
        Err(Error(format!("{what}, at {} ({}): {text:?}", self.src.place(self.c.start(at) as u32), self.c.kind(at))))
    }

    fn m(&self, t: &Ty) -> M {
        t.m(&self.structs)
    }

    fn size(&self, t: &Ty) -> u64 {
        t.size(&self.structs)
    }

    fn named(&self, i: usize) -> Vec<usize> {
        self.c.named_children(i)
    }

    /// Node `i`'s one expression child (a parenthesized expression's).
    fn inner(&self, i: usize) -> R<usize> {
        match self.named(i).as_slice() {
            [x] => Ok(*x),
            _ => self.err(i, "an expression of one operand"),
        }
    }

    fn has_attr(&self, i: usize, attr: &str) -> bool {
        self.c.children(i).into_iter().any(|c| self.c.kind(c) == "attribute_specifier" && self.c.text(c).contains(attr))
    }

    fn is_static(&self, i: usize) -> bool {
        self.c.children(i).into_iter().any(|c| self.c.kind(c) == "storage_class_specifier" && self.c.text(c) == "static")
    }

    // Types.

    /// The type node `d`'s specifier (its `type` field) names.
    fn typed(&mut self, d: usize, missing: &str) -> R<Ty> {
        match self.c.child(d, "type") {
            Some(x) => self.base(x),
            None => self.err(d, missing),
        }
    }

    /// The type a type specifier names.
    fn base(&mut self, t: usize) -> R<Ty> {
        match self.c.kind(t) {
            "primitive_type" => Ok(match self.c.text(t) {
                "void" => Ty::Void,
                "bool" | "_Bool" => Ty::Bool,
                "char" | "int8_t" => Ty::Int { bits: 8, signed: true },
                "uint8_t" => Ty::Int { bits: 8, signed: false },
                "short" | "int16_t" => Ty::Int { bits: 16, signed: true },
                "uint16_t" => Ty::Int { bits: 16, signed: false },
                "int" | "int32_t" => I32,
                "uint32_t" => U32,
                "long" | "int64_t" | "intptr_t" | "ptrdiff_t" | "ssize_t" => I64,
                "uint64_t" | "size_t" | "uintptr_t" => U64,
                "float" => Ty::Float(32),
                "double" => Ty::Float(64),
                n => return self.err(t, format!("the type {n}")),
            }),
            "sized_type_specifier" => {
                let s = self.c.text(t);
                let unsigned = s.contains("unsigned");
                let bits = if s.contains("long") {
                    64
                } else if s.contains("short") {
                    16
                } else if s.contains("char") {
                    8
                } else {
                    32
                };
                Ok(Ty::Int { bits, signed: !unsigned })
            }
            "type_identifier" => match self.typedefs.get(self.c.text(t)) {
                Some(t) => Ok(t.clone()),
                None => self.err(t, "an unknown type"),
            },
            "struct_specifier" | "union_specifier" => self.record(t),
            k => self.err(t, format!("a type specifier ({k})")),
        }
    }

    /// A structure or union: its tag's, defined here if it has a body.
    fn record(&mut self, t: usize) -> R<Ty> {
        let union = self.c.kind(t) == "union_specifier";
        let name = self.c.child(t, "name").map(|n| self.c.text(n).to_string());
        let id = match name.as_ref().and_then(|n| self.tags.get(n)) {
            Some(&id) => id,
            None => {
                self.structs.push(Struct { name: name.clone().unwrap_or_default(), union, ..Default::default() });
                let id = (self.structs.len() - 1) as u32;
                if let Some(n) = name {
                    self.tags.insert(n, id);
                }
                id
            }
        };
        if let Some(body) = self.c.child(t, "body") {
            let mut fields = Vec::new();
            for f in self.named(body) {
                if self.c.kind(f) != "field_declaration" {
                    continue;
                }
                let ty = self.typed(f, "a field with no type")?;
                for d in self.c.children_in(f, "declarator") {
                    let (n, t) = self.declarator(d, ty.clone())?;
                    fields.push((n, t, 0));
                }
            }
            let mut s = Struct { name: self.structs[id as usize].name.clone(), union, fields, ..Default::default() };
            lay_out(&mut s, &self.structs);
            self.structs[id as usize] = s;
        }
        Ok(Ty::Struct(id))
    }

    /// A declarator's name and type, from the type its specifiers give (`base`).
    fn declarator(&mut self, d: usize, base: Ty) -> R<(String, Ty)> {
        match self.c.kind(d) {
            "identifier" | "field_identifier" | "type_identifier" => Ok((self.c.text(d).to_string(), base)),
            "pointer_declarator" | "abstract_pointer_declarator" => match self.c.child(d, "declarator") {
                Some(x) => self.declarator(x, Ty::Ptr(Box::new(base))),
                None => Ok((String::new(), Ty::Ptr(Box::new(base)))),
            },
            "array_declarator" | "abstract_array_declarator" => {
                let n = match self.c.child(d, "size") {
                    Some(s) => self.constant(s)?,
                    None => 0,
                };
                let t = Ty::Array(Box::new(base), n);
                match self.c.child(d, "declarator") {
                    Some(x) => self.declarator(x, t),
                    None => Ok((String::new(), t)),
                }
            }
            "parenthesized_declarator" | "abstract_parenthesized_declarator" => {
                let x = self.inner(d)?;
                self.declarator(x, base)
            }
            "function_declarator" => {
                let x = self.c.child(d, "declarator").map_or_else(|| self.err(d, "a function with no name"), Ok)?;
                self.declarator(x, base)
            }
            "init_declarator" => {
                let x = self.c.child(d, "declarator").map_or_else(|| self.err(d, "a declarator"), Ok)?;
                self.declarator(x, base)
            }
            k => self.err(d, format!("a declarator ({k})")),
        }
    }

    /// The function declarator inside declarator `d`, and the type its result has from `base`.
    fn function_declarator(&mut self, d: usize, base: Ty) -> R<Option<(usize, Ty)>> {
        match self.c.kind(d) {
            "function_declarator" => Ok(Some((d, base))),
            "pointer_declarator" => match self.c.child(d, "declarator") {
                Some(x) => self.function_declarator(x, Ty::Ptr(Box::new(base))),
                None => Ok(None),
            },
            "parenthesized_declarator" => {
                let x = self.inner(d)?;
                self.function_declarator(x, base)
            }
            _ => Ok(None),
        }
    }

    /// A type descriptor's type (a cast's, `sizeof`'s).
    fn descriptor(&mut self, t: usize) -> R<Ty> {
        let base = self.typed(t, "a type with no specifier")?;
        match self.c.child(t, "declarator") {
            Some(d) => Ok(self.declarator(d, base)?.1),
            None => Ok(base),
        }
    }

    /// A function declarator's parameters: their names and types (arrays as pointers).
    fn params(&mut self, f: usize) -> R<Vec<(String, Ty)>> {
        let Some(list) = self.c.child(f, "parameters") else { return Ok(Vec::new()) };
        let mut out = Vec::new();
        for p in self.named(list) {
            if self.c.kind(p) != "parameter_declaration" {
                return self.err(p, "a parameter");
            }
            let base = self.typed(p, "a parameter with no type")?;
            let (n, t) = match self.c.child(p, "declarator") {
                Some(d) => self.declarator(d, base)?,
                None => (String::new(), base),
            };
            if t == Ty::Void {
                continue;
            }
            out.push((n, decay(t)));
        }
        Ok(out)
    }

    // The file's scope.

    fn top(&mut self, d: usize) -> R<()> {
        match self.c.kind(d) {
            "function_definition" => self.function(d),
            "declaration" => self.top_declaration(d),
            "type_definition" => self.typedef(d),
            "struct_specifier" | "union_specifier" => self.record(d).map(|_| ()),
            ";" | "comment" => Ok(()),
            k => self.err(d, format!("a declaration at the file's scope ({k})")),
        }
    }

    fn typedef(&mut self, d: usize) -> R<()> {
        let base = self.typed(d, "a typedef with no type")?;
        for x in self.c.children_in(d, "declarator") {
            let (n, t) = self.declarator(x, base.clone())?;
            self.typedefs.insert(n, t);
        }
        Ok(())
    }

    /// A function declared: its index, the one already declared with its parameters if any.
    fn declare(&mut self, name: &str, ret: Ty, params: Vec<(String, Ty)>, kernel: bool) -> u32 {
        let types: Vec<Ty> = params.iter().map(|p| p.1.clone()).collect();
        if let Some(ids) = self.fnames.get(name) {
            if let Some(&id) = ids.iter().find(|&&id| self.funcs[id as usize].params == types) {
                let f = &mut self.funcs[id as usize];
                f.kernel |= kernel;
                f.names = params.into_iter().map(|p| p.0).collect();
                return id;
            }
        }
        self.funcs.push(Func { name: name.to_string(), ret, params: types, node: None, kernel, names: params.into_iter().map(|p| p.0).collect() });
        let id = (self.funcs.len() - 1) as u32;
        self.fnames.entry(name.to_string()).or_default().push(id);
        id
    }

    fn top_declaration(&mut self, d: usize) -> R<()> {
        let base = self.typed(d, "a declaration with no type")?;
        for x in self.c.children_in(d, "declarator") {
            if let Some((f, ret)) = self.function_declarator(x, base.clone())? {
                let (name, _) = self.declarator(f, Ty::Void)?;
                let params = self.params(f)?;
                self.declare(&name, ret, params, false);
                continue;
            }
            self.global(x, base.clone(), d)?;
        }
        Ok(())
    }

    /// A variable of static storage: its bytes in the program's constant memory, from its initializer.
    fn global(&mut self, x: usize, base: Ty, d: usize) -> R<Var> {
        let (name, mut ty) = self.declarator(x, base)?;
        let init = if self.c.kind(x) == "init_declarator" { self.c.child(x, "value") } else { None };
        if let (Ty::Array(e, 0), Some(i)) = (&ty, init) {
            let n = match self.c.kind(i) {
                "string_literal" => self.string(i)?.len() as u64 + 1,
                "initializer_list" => self.named(i).len() as u64,
                _ => 0,
            };
            ty = Ty::Array(e.clone(), n);
        }
        let (size, _) = ty.layout(&self.structs);
        let at = self.data.len().next_multiple_of(16);
        self.data.resize(at + size as usize, 0);
        if let Some(i) = init {
            self.constant_init(i, &ty, at)?;
        }
        if !self.c.children(d).into_iter().any(|c| self.c.kind(c) == "type_qualifier" && self.c.text(c) == "const") {
            // A kernel's memory is its buffers' and its threadgroup's: a variable of static storage is a constant.
            return self.err(d, "a variable of static storage that is not const");
        }
        let v = Var { ty, place: Place::Global(at as u32) };
        if self.scopes.is_empty() {
            self.globals.insert(name, v.clone());
        } else {
            self.scopes.last_mut().expect("a scope").insert(name, v.clone());
        }
        Ok(v)
    }

    /// A string literal's bytes, its escapes read.
    fn string(&self, s: usize) -> R<Vec<u8>> {
        let mut out = Vec::new();
        let t = self.c.text(s);
        let body = t.strip_prefix('"').and_then(|t| t.strip_suffix('"')).map_or_else(|| self.err(s, "a string"), Ok)?;
        let b = body.as_bytes();
        let mut i = 0;
        while i < b.len() {
            if b[i] == b'\\' && i + 1 < b.len() {
                i += 1;
                out.push(match b[i] {
                    b'n' => b'\n',
                    b't' => b'\t',
                    b'0' => 0,
                    b'r' => b'\r',
                    c => c,
                });
            } else {
                out.push(b[i]);
            }
            i += 1;
        }
        Ok(out)
    }

    /// The constant `i` written into the program's constant memory at `at`, as type `ty`.
    fn constant_init(&mut self, i: usize, ty: &Ty, at: usize) -> R<()> {
        match (self.c.kind(i), ty) {
            ("string_literal", Ty::Array(..)) => {
                let s = self.string(i)?;
                self.data[at..at + s.len()].copy_from_slice(&s);
                Ok(())
            }
            ("initializer_list", Ty::Array(e, _)) => {
                let es = self.size(e) as usize;
                for (k, x) in self.named(i).into_iter().enumerate() {
                    self.constant_init(x, &e.clone(), at + k * es)?;
                }
                Ok(())
            }
            ("initializer_list", Ty::Struct(s)) => {
                let fields = self.structs[*s as usize].fields.clone();
                for (x, f) in self.named(i).into_iter().zip(fields) {
                    self.constant_init(x, &f.1, at + f.2 as usize)?;
                }
                Ok(())
            }
            _ => {
                let (t, v) = self.constant_typed(i)?;
                let v = crate::ops::cast(self.m(&t), self.m(ty), v);
                let n = self.size(ty) as usize;
                self.data[at..at + n].copy_from_slice(&v.to_le_bytes()[..n]);
                Ok(())
            }
        }
    }

    /// An integer constant expression's value (an array's size).
    fn constant(&mut self, e: usize) -> R<u64> {
        let (t, v) = self.constant_typed(e)?;
        Ok(crate::ops::norm(self.m(&t), v))
    }

    /// A constant expression's type and value, by the operations the kernel tape's are (`ops`).
    fn constant_typed(&mut self, e: usize) -> R<(Ty, u64)> {
        match self.c.kind(e) {
            "number_literal" => literal(self.c.text(e)).map_or_else(|| self.err(e, "a number"), Ok),
            "char_literal" => Ok((I32, self.char_literal(e)?)),
            "true" => Ok((I32, 1)),
            "false" => Ok((I32, 0)),
            "parenthesized_expression" => {
                let x = self.inner(e)?;
                self.constant_typed(x)
            }
            "cast_expression" => {
                let to = self.descriptor(self.c.child(e, "type").expect("a cast's type"))?;
                let (t, v) = self.constant_typed(self.c.child(e, "value").expect("a cast's value"))?;
                Ok((to.clone(), crate::ops::cast(self.m(&t), self.m(&to), v)))
            }
            "sizeof_expression" => Ok((U64, self.sizeof(e)?)),
            "unary_expression" => {
                let op = self.c.text(self.c.child(e, "operator").expect("an operator"));
                let (t, v) = self.constant_typed(self.c.child(e, "argument").expect("an operand"))?;
                let p = t.promoted();
                let v = crate::ops::cast(self.m(&t), self.m(&p), v);
                Ok(match op {
                    "-" => (p.clone(), crate::ops::un(Op::Neg, self.m(&p), v)),
                    "~" => (p.clone(), crate::ops::un(Op::Not, self.m(&p), v)),
                    "!" => (I32, crate::ops::un(Op::LNot, self.m(&p), v)),
                    _ => (p, v),
                })
            }
            "binary_expression" => {
                let op = self.c.text(self.c.child(e, "operator").expect("an operator")).to_string();
                let (lt, lv) = self.constant_typed(self.c.child(e, "left").expect("a left operand"))?;
                let (rt, rv) = self.constant_typed(self.c.child(e, "right").expect("a right operand"))?;
                if op == "&&" || op == "||" {
                    let (a, b) = (crate::ops::truth(self.m(&lt), lv), crate::ops::truth(self.m(&rt), rv));
                    return Ok((I32, if op == "&&" { a & b } else { a | b }));
                }
                let Some(o) = binop(&op) else { return self.err(e, "an operator") };
                let t = if matches!(o, Op::Shl | Op::Shr) { lt.promoted() } else { Ty::common(&lt, &rt) };
                let m = self.m(&t);
                let (a, b) = (crate::ops::cast(self.m(&lt), m, lv), crate::ops::cast(self.m(&rt), m, rv));
                let v = crate::ops::bin(o, m, a, b).map_err(|w| Error(format!("{w} in a constant")))?;
                Ok((if o.compares() { I32 } else { t }, v))
            }
            "conditional_expression" => {
                let (ct, cv) = self.constant_typed(self.c.child(e, "condition").expect("a condition"))?;
                let pick = if crate::ops::truth(self.m(&ct), cv) != 0 { "consequence" } else { "alternative" };
                self.constant_typed(self.c.child(e, pick).expect("a branch"))
            }
            _ => self.err(e, "a constant expression"),
        }
    }

    fn char_literal(&self, e: usize) -> R<u64> {
        let t = self.c.text(e);
        let b = t.as_bytes();
        Ok(match b {
            [b'\'', b'\\', c, b'\''] => u64::from(match c {
                b'n' => b'\n',
                b't' => b'\t',
                b'0' => 0,
                b'r' => b'\r',
                c => *c,
            }),
            [b'\'', c, b'\''] => u64::from(*c),
            _ => return self.err(e, "a character"),
        })
    }

    fn sizeof(&mut self, e: usize) -> R<u64> {
        if let Some(t) = self.c.child(e, "type") {
            let t = self.descriptor(t)?;
            return Ok(self.size(&t));
        }
        let x = self.c.child(e, "value").map_or_else(|| self.err(e, "sizeof of nothing"), Ok)?;
        let x = if self.c.kind(x) == "parenthesized_expression" { self.inner(x)? } else { x };
        // `sizeof(T)` of a typedef's name reads as an expression to C's grammar.
        if self.c.kind(x) == "identifier" && self.lookup(self.c.text(x)).is_none() {
            if let Some(t) = self.typedefs.get(self.c.text(x)) {
                return Ok(self.size(&t.clone()));
            }
        }
        let t = self.ty(x)?;
        Ok(self.size(&t))
    }

    // Functions.

    fn function(&mut self, d: usize) -> R<()> {
        let base = self.typed(d, "a function with no type")?;
        let decl = self.c.child(d, "declarator").map_or_else(|| self.err(d, "a function with no declarator"), Ok)?;
        let Some((f, ret)) = self.function_declarator(decl, base)? else { return self.err(d, "a function's declarator") };
        let (name, _) = self.declarator(f, Ty::Void)?;
        let params = self.params(f)?;
        let kernel = self.has_attr(d, "ukl_kernel");
        let id = self.declare(&name, ret.clone(), params.clone(), kernel);
        let body = self.c.child(d, "body").map_or_else(|| self.err(d, "a function with no body"), Ok)?;
        // [id, frame bytes, threadgroup bytes, return type, parameters, (offset, type) a parameter]
        let mut words = vec![id, 0, 0, self.m(&ret).0, params.len() as u32];
        self.scopes.push(HashMap::new());
        (self.at, self.frame, self.shared, self.ret) = (0, 0, 0, ret);
        for (n, t) in &params {
            let at = self.alloc(t);
            words.extend([at, self.m(t).0]);
            self.scopes.last_mut().expect("a scope").insert(n.clone(), Var { ty: t.clone(), place: Place::Local(at) });
        }
        let node = self.b.open(Kind::Func, &words);
        self.stmt(body)?;
        self.b.close(node);
        self.b.set_word(node, 1, self.frame);
        self.b.set_word(node, 2, self.shared);
        self.scopes.pop();
        if self.funcs[id as usize].node.is_some() {
            return self.err(d, "a function defined twice");
        }
        self.funcs[id as usize].node = Some(node);
        Ok(())
    }

    /// A place in the frame for a value of `t`.
    fn alloc(&mut self, t: &Ty) -> u32 {
        let (size, align) = t.layout(&self.structs);
        let at = self.at.next_multiple_of(align.clamp(8, 16) as u32);
        self.at = at + size.max(1) as u32;
        self.frame = self.frame.max(self.at);
        at
    }

    fn lookup(&self, name: &str) -> Option<&Var> {
        self.scopes.iter().rev().find_map(|s| s.get(name)).or_else(|| self.globals.get(name))
    }

    // Statements.

    fn scoped(&mut self, f: impl FnOnce(&mut Self) -> R<()>) -> R<()> {
        let at = self.at;
        self.scopes.push(HashMap::new());
        let r = f(self);
        self.scopes.pop();
        self.at = at;
        r
    }

    fn stmt(&mut self, s: usize) -> R<()> {
        match self.c.kind(s) {
            "compound_statement" => {
                let n = self.b.open(Kind::Block, &[]);
                self.scoped(|l| {
                    for x in l.named(s) {
                        l.stmt(x)?;
                    }
                    Ok(())
                })?;
                self.b.close(n);
            }
            "declaration" => self.local(s)?,
            "type_definition" => self.typedef(s)?,
            "expression_statement" => match self.named(s).as_slice() {
                [] => {
                    self.b.leaf(Kind::Empty, &[]);
                }
                [x] => {
                    let n = self.b.open(Kind::Expr, &[]);
                    self.rv(*x)?;
                    self.b.close(n);
                }
                _ => return self.err(s, "an expression statement"),
            },
            "if_statement" => {
                let n = self.b.open(Kind::If, &[]);
                self.cond(self.c.child(s, "condition").expect("a condition"))?;
                self.stmt(self.c.child(s, "consequence").expect("a consequence"))?;
                if let Some(e) = self.c.child(s, "alternative") {
                    let x = self.inner(e)?;
                    self.stmt(x)?;
                }
                self.b.close(n);
            }
            "while_statement" => {
                let n = self.b.open(Kind::While, &[]);
                self.cond(self.c.child(s, "condition").expect("a condition"))?;
                self.scoped(|l| l.stmt(l.c.child(s, "body").expect("a body")))?;
                self.b.close(n);
            }
            "do_statement" => {
                let n = self.b.open(Kind::Do, &[]);
                self.scoped(|l| l.stmt(l.c.child(s, "body").expect("a body")))?;
                self.cond(self.c.child(s, "condition").expect("a condition"))?;
                self.b.close(n);
            }
            "for_statement" => {
                let n = self.b.open(Kind::Block, &[]);
                self.scoped(|l| {
                    for i in l.c.children_in(s, "initializer") {
                        if l.c.kind(i) == "declaration" {
                            l.local(i)?;
                        } else {
                            let e = l.b.open(Kind::Expr, &[]);
                            l.rv(i)?;
                            l.b.close(e);
                        }
                    }
                    let f = l.b.open(Kind::For, &[]);
                    match l.c.child(s, "condition") {
                        Some(c) => l.cond(c)?,
                        None => {
                            l.b.leaf(Kind::Const, &words_of(M::B, 1));
                        }
                    }
                    l.scoped(|l| l.stmt(l.c.child(s, "body").expect("a body")))?;
                    match l.c.child(s, "update") {
                        Some(u) => {
                            let e = l.b.open(Kind::Expr, &[]);
                            l.rv(u)?;
                            l.b.close(e);
                        }
                        None => {
                            l.b.leaf(Kind::Empty, &[]);
                        }
                    }
                    l.b.close(f);
                    Ok(())
                })?;
                self.b.close(n);
            }
            "return_statement" => match self.named(s).as_slice() {
                [] => {
                    self.b.leaf(Kind::Return, &[0]);
                }
                [x] => {
                    let n = self.b.open(Kind::Return, &[1]);
                    let ret = self.ret.clone();
                    if ret == Ty::Void {
                        self.rv(*x)?;
                    } else {
                        self.rv_as(*x, &ret)?;
                    }
                    self.b.close(n);
                }
                _ => return self.err(s, "a return"),
            },
            "break_statement" => {
                self.b.leaf(Kind::Break, &[]);
            }
            "continue_statement" => {
                self.b.leaf(Kind::Continue, &[]);
            }
            ";" | "comment" => {
                self.b.leaf(Kind::Empty, &[]);
            }
            k => return self.err(s, format!("a statement ({k})")),
        }
        Ok(())
    }

    /// A declaration in a function: each variable's place, and its initializer's stores.
    fn local(&mut self, d: usize) -> R<()> {
        let base = self.typed(d, "a declaration with no type")?;
        let block = self.b.open(Kind::Block, &[]);
        for x in self.c.children_in(d, "declarator") {
            if self.function_declarator(x, base.clone())?.is_some() {
                return self.err(x, "a function declared in a function");
            }
            if self.is_static(d) {
                self.global(x, base.clone(), d)?;
                continue;
            }
            let (name, ty) = self.declarator(x, base.clone())?;
            if self.has_attr(d, "ukl_local") {
                // A threadgroup's own storage (a `SHARED` row's).
                let (size, align) = ty.layout(&self.structs);
                let at = self.shared.next_multiple_of(align.max(16) as u32);
                self.shared = at + size as u32;
                self.scopes.last_mut().expect("a scope").insert(name, Var { ty, place: Place::Shared(at) });
                continue;
            }
            let at = self.alloc(&ty);
            let init = if self.c.kind(x) == "init_declarator" { self.c.child(x, "value") } else { None };
            self.scopes.last_mut().expect("a scope").insert(name, Var { ty: ty.clone(), place: Place::Local(at) });
            if let Some(i) = init {
                let e = self.b.open(Kind::Expr, &[]);
                self.init(i, &ty, at)?;
                self.b.close(e);
            }
        }
        self.b.close(block);
        Ok(())
    }

    /// The initializer `i` of a variable of `ty` at frame offset `at`, as one expression.
    fn init(&mut self, i: usize, ty: &Ty, at: u32) -> R<()> {
        if self.c.kind(i) != "initializer_list" {
            return self.assign_to(|l| l.b.leaf(Kind::Local, &[at]), ty, i);
        }
        // `{...}`: zero, then each element, as a comma's operands.
        let elements: Vec<(Ty, u32)> = match ty {
            Ty::Array(e, n) => (0..*n).map(|k| ((**e).clone(), at + (k * self.size(e)) as u32)).collect(),
            Ty::Struct(s) => self.structs[*s as usize].fields.iter().map(|f| (f.1.clone(), at + f.2 as u32)).collect(),
            _ => return self.err(i, "an initializer list for a scalar"),
        };
        let items = self.named(i);
        let c = self.b.open(Kind::Comma, &[]);
        let z = self.b.open(Kind::Zero, &[M::VOID.0, self.size(ty) as u32]);
        self.b.leaf(Kind::Local, &[at]);
        self.b.close(z);
        let n = self.b.open(Kind::Block, &[]);
        for (x, (t, a)) in items.into_iter().zip(elements) {
            let e = self.b.open(Kind::Expr, &[]);
            self.init(x, &t, a)?;
            self.b.close(e);
        }
        self.b.close(n);
        self.b.close(c);
        Ok(())
    }

    /// An assignment of `value` to a `ty` at the address `place` writes.
    fn assign_to(&mut self, place: impl FnOnce(&mut Self) -> usize, ty: &Ty, value: usize) -> R<()> {
        if ty.m(&self.structs).is_agg() {
            let n = self.b.open(Kind::Copy, &[self.size(ty) as u32]);
            place(self);
            self.rv(value)?;
            self.b.close(n);
        } else {
            let n = self.b.open(Kind::Store, &[self.m(ty).0]);
            place(self);
            self.rv_as(value, ty)?;
            self.b.close(n);
        }
        Ok(())
    }

    // Expressions' types.

    fn ty(&mut self, e: usize) -> R<Ty> {
        if let Some(t) = self.tys.get(&e) {
            return Ok(t.clone());
        }
        let t = self.ty_of(e)?;
        self.tys.insert(e, t.clone());
        Ok(t)
    }

    fn field_of(&self, e: usize, s: &Ty) -> R<(Ty, u64)> {
        let name = self.c.text(self.c.child(e, "field").expect("a field"));
        let Ty::Struct(k) = s else { return self.err(e, format!("a field of a {s:?}")) };
        match self.structs[*k as usize].fields.iter().find(|f| f.0 == name) {
            Some(f) => Ok((f.1.clone(), f.2)),
            None => self.err(e, format!("no field {name}")),
        }
    }

    /// Node `e`'s operator's text.
    fn operator(&self, e: usize) -> &str {
        self.c.child(e, "operator").map_or("", |o| self.c.text(o))
    }

    fn ty_of(&mut self, e: usize) -> R<Ty> {
        Ok(match self.c.kind(e) {
            "number_literal" => literal(self.c.text(e)).map_or_else(|| self.err(e, "a number"), Ok)?.0,
            "char_literal" | "true" | "false" => I32,
            "string_literal" => Ty::Array(Box::new(Ty::Int { bits: 8, signed: true }), self.string(e)?.len() as u64 + 1),
            "identifier" => match self.lookup(self.c.text(e)) {
                Some(v) => v.ty.clone(),
                None if self.c.text(e).starts_with("CLK_") => U32,
                None => return self.err(e, "an unknown name"),
            },
            "parenthesized_expression" => {
                let x = self.inner(e)?;
                self.ty(x)?
            }
            "binary_expression" => {
                let op = self.operator(e).to_string();
                let l = decay(self.ty(self.c.child(e, "left").expect("a left operand"))?);
                let r = decay(self.ty(self.c.child(e, "right").expect("a right operand"))?);
                match op.as_str() {
                    "&&" | "||" | "==" | "!=" | "<" | "<=" | ">" | ">=" => I32,
                    "<<" | ">>" => l.promoted(),
                    "+" | "-" if l.is_ptr() && r.is_ptr() => I64,
                    "+" | "-" if l.is_ptr() => l,
                    "+" if r.is_ptr() => r,
                    _ => Ty::common(&l, &r),
                }
            }
            "unary_expression" => {
                let a = decay(self.ty(self.c.child(e, "argument").expect("an operand"))?);
                if self.operator(e) == "!" { I32 } else { a.promoted() }
            }
            "pointer_expression" => {
                let a = self.ty(self.c.child(e, "argument").expect("an operand"))?;
                if self.operator(e) == "&" {
                    Ty::Ptr(Box::new(a))
                } else {
                    match decay(a).pointee() {
                        Some(t) => t.clone(),
                        None => return self.err(e, "a dereference of a non-pointer"),
                    }
                }
            }
            "cast_expression" => self.descriptor(self.c.child(e, "type").expect("a cast's type"))?,
            "sizeof_expression" => U64,
            "subscript_expression" => {
                let a = decay(self.ty(self.c.child(e, "argument").expect("an array"))?);
                match a.pointee() {
                    Some(t) => t.clone(),
                    None => return self.err(e, "a subscript of a non-pointer"),
                }
            }
            "field_expression" => {
                let a = self.ty(self.c.child(e, "argument").expect("a structure"))?;
                let s = if self.operator(e) == "->" {
                    match decay(a).pointee() {
                        Some(t) => t.clone(),
                        None => return self.err(e, "-> of a non-pointer"),
                    }
                } else {
                    a
                };
                if s == Ty::Uint4 { U32 } else { self.field_of(e, &s)?.0 }
            }
            "assignment_expression" => self.ty(self.c.child(e, "left").expect("an lvalue"))?,
            "update_expression" => decay(self.ty(self.c.child(e, "argument").expect("an operand"))?),
            "conditional_expression" => {
                let a = decay(self.ty(self.c.child(e, "consequence").expect("a branch"))?);
                let b = decay(self.ty(self.c.child(e, "alternative").expect("a branch"))?);
                if a.is_arith() && b.is_arith() {
                    Ty::common(&a, &b)
                } else if a.is_ptr() {
                    a
                } else {
                    b
                }
            }
            "comma_expression" => self.ty(self.c.child(e, "right").expect("a right operand"))?,
            "call_expression" => self.call_ty(e)?.0,
            k => return self.err(e, format!("an expression ({k})")),
        })
    }

    /// A call's result type, and what it calls: a function of the program, or a builtin with its operands' types.
    fn call_ty(&mut self, e: usize) -> R<(Ty, Callee)> {
        let f = self.c.child(e, "function").expect("a callee");
        if self.c.kind(f) != "identifier" {
            return self.err(f, "a call of a function by pointer");
        }
        let name = self.c.text(f).to_string();
        let args = self.named(self.c.child(e, "arguments").expect("arguments"));
        let mut tys = Vec::with_capacity(args.len());
        for a in &args {
            let t = decay(self.ty(*a)?);
            tys.push(t);
        }
        if let Some(ids) = self.fnames.get(&name).cloned() {
            let id = self.overload(e, &ids, &tys)?;
            return Ok((self.funcs[id as usize].ret.clone(), Callee::Func(id)));
        }
        let Some(b) = Builtin::named(&name) else { return self.err(f, "a call of an unknown function") };
        let arg = |k: usize| tys.get(k).cloned().unwrap_or(U32);
        let (ret, params) = match b {
            Builtin::LocalId | Builtin::LocalSize | Builtin::GlobalId | Builtin::GlobalSize | Builtin::GroupId | Builtin::NumGroups => (U64, vec![U32]),
            Builtin::SubGroupLocalId => (U32, vec![]),
            Builtin::Barrier | Builtin::MemFence | Builtin::SubGroupBarrier => (Ty::Void, vec![U32]),
            Builtin::SubGroupBallot => (Ty::Uint4, vec![I32]),
            Builtin::SubGroupShuffle | Builtin::SubGroupShuffleUp | Builtin::SubGroupShuffleDown | Builtin::SubGroupShuffleXor => {
                let t = arg(0).promoted();
                (t.clone(), vec![t, U32])
            }
            Builtin::AtomicCmpxchg => {
                let t = arg(0).pointee().cloned().unwrap_or(U32);
                (t.clone(), vec![arg(0), t.clone(), t])
            }
            Builtin::AtomicInc | Builtin::AtomicDec => {
                let t = arg(0).pointee().cloned().unwrap_or(U32);
                (t, vec![arg(0)])
            }
            Builtin::AtomicAdd | Builtin::AtomicSub | Builtin::AtomicAnd | Builtin::AtomicOr | Builtin::AtomicXor | Builtin::AtomicXchg | Builtin::AtomicMin | Builtin::AtomicMax => {
                let t = arg(0).pointee().cloned().unwrap_or(U32);
                (t.clone(), vec![arg(0), t])
            }
            Builtin::Popcount | Builtin::Clz => (arg(0), vec![arg(0)]),
            Builtin::MulHi | Builtin::Min | Builtin::Max => {
                let t = Ty::common(&arg(0), &arg(1));
                (t.clone(), vec![t.clone(), t])
            }
            Builtin::As => (as_target(&name), vec![arg(0)]),
        };
        if params.len() != args.len() {
            return self.err(e, format!("{name} takes {} arguments", params.len()));
        }
        Ok((ret, Callee::Builtin(b, params)))
    }

    /// The overload of `ids` that `args` call (clang's `overloadable`): every argument converts, the fewest
    /// conversions that are not exact or a promotion.
    fn overload(&self, e: usize, ids: &[u32], args: &[Ty]) -> R<u32> {
        let rank = |p: &Ty, a: &Ty| -> Option<u32> {
            if p == a {
                Some(0)
            } else if a.is_int() && &a.promoted() == p {
                Some(1)
            } else if p.is_arith() && a.is_arith() {
                Some(2)
            } else if p.is_ptr() && (a.is_ptr() || a.is_int()) {
                Some(if p.pointee() == a.pointee() || matches!(p.pointee(), Some(Ty::Void)) { 0 } else { 3 })
            } else if p == a {
                Some(0)
            } else {
                None
            }
        };
        let mut best: Option<(Vec<u32>, u32)> = None;
        let mut tie = false;
        for &id in ids {
            let f = &self.funcs[id as usize];
            if f.params.len() != args.len() {
                continue;
            }
            let Some(mut r): Option<Vec<u32>> = f.params.iter().zip(args).map(|(p, a)| rank(p, a)).collect() else { continue };
            r.sort_unstable_by(|a, b| b.cmp(a));
            match &best {
                Some((b, _)) if *b < r => {}
                Some((b, _)) if *b == r => tie = true,
                _ => {
                    best = Some((r, id));
                    tie = false;
                }
            }
        }
        match best {
            Some((_, id)) if !tie || ids.len() == 1 => Ok(id),
            Some(_) => self.err(e, format!("an ambiguous call ({args:?})")),
            None => self.err(e, format!("no overload takes {args:?}")),
        }
    }

    // Expressions' nodes.

    /// A condition: `e` as a `bool`.
    fn cond(&mut self, e: usize) -> R<()> {
        let e = if self.c.kind(e) == "parenthesized_expression" { self.inner(e)? } else { e };
        self.rv_as(e, &Ty::Bool)
    }

    /// `e`'s value converted to `to`.
    fn rv_as(&mut self, e: usize, to: &Ty) -> R<()> {
        let from = decay(self.ty(e)?);
        let (fm, tm) = (self.m(&from), self.m(to));
        if fm == tm || *to == Ty::Void || from.m(&self.structs).is_agg() {
            self.rv(e)?;
            return Ok(());
        }
        let n = self.b.open(Kind::Cast, &[fm.0, tm.0]);
        self.rv(e)?;
        self.b.close(n);
        Ok(())
    }

    fn konst(&mut self, t: &Ty, v: u64) {
        let m = self.m(t);
        self.b.leaf(Kind::Const, &words_of(m, crate::ops::norm(m, v)));
    }

    /// `i` (an integer) converted to a pointer's width and scaled by `size`: a pointer's offset.
    fn offset(&mut self, i: usize, size: u64) -> R<()> {
        if size == 1 {
            return self.rv_as(i, &Ty::Ptr(Box::new(Ty::Void)));
        }
        let n = self.b.open(Kind::Bin, &[Op::Mul as u32, M::PTR.0]);
        self.rv_as(i, &Ty::Ptr(Box::new(Ty::Void)))?;
        self.b.leaf(Kind::Const, &words_of(M::PTR, size));
        self.b.close(n);
        Ok(())
    }

    fn place(&mut self, p: Place) {
        match p {
            Place::Local(at) => self.b.leaf(Kind::Local, &[at]),
            Place::Shared(at) => self.b.leaf(Kind::Shared, &[at]),
            Place::Global(at) => self.b.leaf(Kind::Global, &[at]),
        };
    }

    /// `e`'s address (an lvalue's), and its type.
    fn lv(&mut self, e: usize) -> R<Ty> {
        let t = self.ty(e)?;
        match self.c.kind(e) {
            "identifier" => match self.lookup(self.c.text(e)).cloned() {
                Some(v) => self.place(v.place),
                None => return self.err(e, "an unknown name"),
            },
            "parenthesized_expression" => {
                let x = self.inner(e)?;
                self.lv(x)?;
            }
            "subscript_expression" => {
                let size = self.size(&t);
                let n = self.b.open(Kind::Bin, &[Op::Add as u32, M::PTR.0]);
                self.rv(self.c.child(e, "argument").expect("an array"))?;
                self.offset(self.c.child(e, "index").expect("an index"), size)?;
                self.b.close(n);
            }
            "pointer_expression" if self.operator(e) == "*" => {
                self.rv(self.c.child(e, "argument").expect("an operand"))?;
            }
            "field_expression" => {
                let a = self.c.child(e, "argument").expect("a structure");
                let at = self.ty(a)?;
                let s = if self.operator(e) == "->" { decay(at).pointee().cloned().unwrap_or(Ty::Void) } else { at };
                let (_, off) = self.field_of(e, &s)?;
                if off == 0 {
                    self.rv(a)?;
                } else {
                    let n = self.b.open(Kind::Bin, &[Op::Add as u32, M::PTR.0]);
                    self.rv(a)?;
                    self.b.leaf(Kind::Const, &words_of(M::PTR, off));
                    self.b.close(n);
                }
            }
            "string_literal" => {
                let s = self.string(e)?;
                let at = self.data.len().next_multiple_of(16);
                self.data.resize(at, 0);
                self.data.extend_from_slice(&s);
                self.data.push(0);
                self.b.leaf(Kind::Global, &[at as u32]);
            }
            _ => return self.err(e, "an lvalue"),
        }
        Ok(t)
    }

    /// `e`'s value written: its node, typed as `ty(e)` decayed (an aggregate's value its address).
    fn rv(&mut self, e: usize) -> R<()> {
        let t = self.ty(e)?;
        let m = self.m(&t);
        match self.c.kind(e) {
            "number_literal" => {
                let (_, v) = literal(self.c.text(e)).expect("a number");
                self.b.leaf(Kind::Const, &words_of(m, v));
            }
            "char_literal" => {
                let v = self.char_literal(e)?;
                self.konst(&I32, v);
            }
            "true" => self.konst(&I32, 1),
            "false" => self.konst(&I32, 0),
            "identifier" if self.lookup(self.c.text(e)).is_none() => {
                // OpenCL's fence flags: no fence on the CPU's coherent memory.
                let v = match self.c.text(e) {
                    "CLK_LOCAL_MEM_FENCE" => 1,
                    "CLK_GLOBAL_MEM_FENCE" => 2,
                    _ => 0,
                };
                self.konst(&U32, v);
            }
            "identifier" | "subscript_expression" | "string_literal" => self.load(e, &t)?,
            "pointer_expression" if self.operator(e) == "*" => self.load(e, &t)?,
            "pointer_expression" => {
                self.lv(self.c.child(e, "argument").expect("an operand"))?;
            }
            "field_expression" => {
                let a = self.c.child(e, "argument").expect("a structure");
                if self.ty(a)? == Ty::Uint4 {
                    // A ballot's mask: its first word, the others zero at 32 lanes.
                    if self.c.text(self.c.child(e, "field").expect("a field")) == "x" {
                        self.rv(a)?;
                    } else {
                        let n = self.b.open(Kind::Comma, &[]);
                        self.rv(a)?;
                        self.konst(&U32, 0);
                        self.b.close(n);
                    }
                } else {
                    self.load(e, &t)?;
                }
            }
            "parenthesized_expression" => {
                let x = self.inner(e)?;
                self.rv(x)?;
            }
            "cast_expression" => {
                let v = self.c.child(e, "value").expect("a cast's value");
                self.rv_as(v, &t)?;
            }
            "sizeof_expression" => {
                let v = self.sizeof(e)?;
                self.konst(&U64, v);
            }
            "binary_expression" => self.binary(e, &t)?,
            "unary_expression" => {
                let a = self.c.child(e, "argument").expect("an operand");
                match self.operator(e) {
                    "!" => {
                        let n = self.b.open(Kind::Un, &[Op::LNot as u32, M::B.0]);
                        self.rv_as(a, &Ty::Bool)?;
                        self.b.close(n);
                    }
                    "+" => self.rv_as(a, &t)?,
                    op => {
                        let o = if op == "-" { Op::Neg } else { Op::Not };
                        let n = self.b.open(Kind::Un, &[o as u32, m.0]);
                        self.rv_as(a, &t)?;
                        self.b.close(n);
                    }
                }
            }
            "conditional_expression" => {
                let n = self.b.open(Kind::Cond, &[m.0]);
                self.cond(self.c.child(e, "condition").expect("a condition"))?;
                self.rv_as(self.c.child(e, "consequence").expect("a branch"), &t)?;
                self.rv_as(self.c.child(e, "alternative").expect("a branch"), &t)?;
                self.b.close(n);
            }
            "comma_expression" => {
                let n = self.b.open(Kind::Comma, &[]);
                self.rv(self.c.child(e, "left").expect("a left operand"))?;
                self.rv(self.c.child(e, "right").expect("a right operand"))?;
                self.b.close(n);
            }
            "assignment_expression" => self.assignment(e, &t)?,
            "update_expression" => {
                let a = self.c.child(e, "argument").expect("an operand");
                let op = if self.operator(e) == "++" { Op::Add } else { Op::Sub };
                let post = self.c.children(e).first().is_some_and(|&c| c == a);
                let (opt, one) = if t.is_ptr() { (t.clone(), self.size(t.pointee().expect("a pointee"))) } else { (t.promoted(), 1) };
                let n = self.b.open(Kind::Update, &[op as u32, m.0, self.m(&opt).0, u32::from(post)]);
                self.lv(a)?;
                self.konst(&opt, one);
                self.b.close(n);
            }
            "call_expression" => self.call(e, &t)?,
            k => return self.err(e, format!("an expression ({k})")),
        }
        Ok(())
    }

    /// An lvalue's value: a load, or an aggregate's (or array's) address.
    fn load(&mut self, e: usize, t: &Ty) -> R<()> {
        if self.m(t).is_agg() {
            self.lv(e)?;
            return Ok(());
        }
        let n = self.b.open(Kind::Load, &[self.m(t).0]);
        self.lv(e)?;
        self.b.close(n);
        Ok(())
    }

    fn binary(&mut self, e: usize, t: &Ty) -> R<()> {
        let op = self.operator(e).to_string();
        let (l, r) = (self.c.child(e, "left").expect("a left operand"), self.c.child(e, "right").expect("a right operand"));
        if op == "&&" || op == "||" {
            let n = self.b.open(Kind::Logic, &[u32::from(op == "&&")]);
            self.rv_as(l, &Ty::Bool)?;
            self.rv_as(r, &Ty::Bool)?;
            self.b.close(n);
            return Ok(());
        }
        let Some(o) = binop(&op) else { return self.err(e, "an operator") };
        let (lt, rt) = (decay(self.ty(l)?), decay(self.ty(r)?));
        if (o == Op::Add || o == Op::Sub) && (lt.is_ptr() || rt.is_ptr()) {
            if lt.is_ptr() && rt.is_ptr() {
                let size = self.size(lt.pointee().expect("a pointee")).max(1);
                let d = self.b.open(Kind::Bin, &[Op::Div as u32, M::I64.0]);
                let n = self.b.open(Kind::Bin, &[Op::Sub as u32, M::I64.0]);
                self.rv_as(l, &I64)?;
                self.rv_as(r, &I64)?;
                self.b.close(n);
                self.b.leaf(Kind::Const, &words_of(M::I64, size));
                self.b.close(d);
                return Ok(());
            }
            let n = self.b.open(Kind::Bin, &[o as u32, M::PTR.0]);
            if lt.is_ptr() {
                let size = self.size(lt.pointee().expect("a pointee"));
                self.rv(l)?;
                self.offset(r, size)?;
            } else {
                let size = self.size(rt.pointee().expect("a pointee"));
                self.offset(l, size)?;
                self.rv(r)?;
            }
            self.b.close(n);
            return Ok(());
        }
        let ot = if o.compares() {
            if lt.is_ptr() || rt.is_ptr() { Ty::Ptr(Box::new(Ty::Void)) } else { Ty::common(&lt, &rt) }
        } else {
            t.clone()
        };
        let n = self.b.open(Kind::Bin, &[o as u32, self.m(&ot).0]);
        self.rv_as(l, &ot)?;
        self.rv_as(r, &ot)?;
        self.b.close(n);
        Ok(())
    }

    fn assignment(&mut self, e: usize, t: &Ty) -> R<()> {
        let (l, r) = (self.c.child(e, "left").expect("an lvalue"), self.c.child(e, "right").expect("a value"));
        let op = self.operator(e).to_string();
        if op == "=" {
            if self.m(t).is_agg() {
                let n = self.b.open(Kind::Copy, &[self.size(t) as u32]);
                self.lv(l)?;
                self.rv(r)?;
                self.b.close(n);
            } else {
                let n = self.b.open(Kind::Store, &[self.m(t).0]);
                self.lv(l)?;
                self.rv_as(r, t)?;
                self.b.close(n);
            }
            return Ok(());
        }
        let Some(o) = binop(&op[..op.len() - 1]) else { return self.err(e, "an assignment's operator") };
        let rt = decay(self.ty(r)?);
        let m = self.m(t);
        if t.is_ptr() {
            let size = self.size(t.pointee().expect("a pointee"));
            let n = self.b.open(Kind::Update, &[o as u32, m.0, M::PTR.0, 0]);
            self.lv(l)?;
            self.offset(r, size)?;
            self.b.close(n);
            return Ok(());
        }
        let ot = if matches!(o, Op::Shl | Op::Shr) { t.promoted() } else { Ty::common(t, &rt) };
        let n = self.b.open(Kind::Update, &[o as u32, m.0, self.m(&ot).0, 0]);
        self.lv(l)?;
        self.rv_as(r, &ot)?;
        self.b.close(n);
        Ok(())
    }

    fn call(&mut self, e: usize, t: &Ty) -> R<()> {
        let (_, callee) = self.call_ty(e)?;
        let args = self.named(self.c.child(e, "arguments").expect("arguments"));
        match callee {
            Callee::Func(id) => {
                // An aggregate's result is copied to the caller's frame, its value that place.
                let temp = if self.m(t).is_agg() { self.alloc(&t.clone()) } else { 0 };
                let n = self.b.open(Kind::Call, &[id, self.m(t).0, temp]);
                let params = self.funcs[id as usize].params.clone();
                for (a, p) in args.into_iter().zip(params) {
                    self.rv_as(a, &p)?;
                }
                self.b.close(n);
            }
            Callee::Builtin(b, params) => {
                let ret = if b == Builtin::SubGroupBallot { M::U32 } else { self.m(t) };
                let mut words = vec![b as u32, ret.0, params.len() as u32];
                words.extend(params.iter().map(|p| self.m(p).0));
                let n = self.b.open(Kind::Builtin, &words);
                for (a, p) in args.into_iter().zip(params) {
                    self.rv_as(a, &p)?;
                }
                self.b.close(n);
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
enum Callee {
    Func(u32),
    Builtin(Builtin, Vec<Ty>),
}
