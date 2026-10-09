//! Rust pack feasibility measurements (syn 2.0.119, proc-macro2 1.0.107 with span locations).
//!
//! `rerail-rust-feasibility scan <root> <out.jsonl>`: one JSON object per `.rs` file under `<root>` (skipping
//! `target/` and `.git/`), with byte offsets into the file:
//! - `idents`: `[offset, kind]` for identifiers in reference position, `kind` one of `use`, `path_single`,
//!   `path_prefix`, `path_last`, `path_qself`, `macro_name`, `attr`, `derive`, `method`, `field`, `field_init`,
//!   `field_shorthand`, `field_pat`, `field_pat_shorthand`, `pat_ident`, `assoc_binding`; prefixed `m:` when the
//!   identifier sits in a macro invocation whose token tree re-parses as items, statements or comma-separated
//!   expressions (what a syntax-only engine could recover by treating the macro as transparent). Path segments
//!   after the first carry a third element, the offset of the previous segment (`[offset, kind, previous]`).
//! - `calls`: `[offset, kind]` for callees, `call_path` (`f(..)`, `a::b(..)`, at the last segment), `call_other`
//!   (any other callee expression) or `call_method` (`.m(..)`), `m:`-prefixed likewise.
//! - `opaque`/`parsed`/`rules`: byte ranges of macro token trees that did not / did re-parse, and `macro_rules!`
//!   bodies. `strs`: string literals inside re-parsed token trees (where `format!` inline arguments live).
//! - `fns`: byte ranges of fn items (with signature). `cfg`: byte ranges of items, statements, expressions,
//!   fields, variants and arms carrying `#[cfg(..)]` (code rust-analyzer may index as inactive).
//! - `consts`: `{site, name, class, refs, macro}` for every constant-expression site (see `classify`).
//! - `attrs`: lint-level attributes found by a token-level scan (also inside macro token trees).
//!
//! `rerail-rust-feasibility scip <index.scip> <out.jsonl>`: decodes a SCIP index (protobuf, decoded by hand
//! here to keep the dependency set small) into one JSON object per document
//! `{path, encoding, occ: [[start line, start char, end line, end char, symbol, roles]], syms: [[symbol, kind]]}`
//! with symbols interned, then a last line `{symbols: [...], external: [[symbol, kind]]}`.

use proc_macro2::{Delimiter, Spacing, Span, TokenStream, TokenTree};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use syn::parse::Parser;
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::visit::{self, Visit};
use syn::{Attribute, Expr, Item, Member, UnOp, UseTree};

fn start(span: Span) -> usize {
    span.byte_range().start
}

fn range(span: Span) -> [usize; 2] {
    let r = span.byte_range();
    [r.start, r.end]
}

#[derive(Default)]
struct Scan {
    depth: u32,
    idents: Vec<(usize, String, Option<usize>)>,
    calls: Vec<(usize, String)>,
    opaque: Vec<[usize; 2]>,
    parsed: Vec<[usize; 2]>,
    rules: Vec<[usize; 2]>,
    fns: Vec<[usize; 2]>,
    cfg: Vec<[usize; 2]>,
    strs: Vec<[usize; 2]>,
    consts: Vec<Value>,
}

impl Scan {
    fn kind(&self, kind: &str) -> String {
        if self.depth > 0 { format!("m:{kind}") } else { kind.to_string() }
    }

    fn ident(&mut self, span: Span, kind: &str) {
        self.ident_after(span, kind, None);
    }

    fn ident_after(&mut self, span: Span, kind: &str, previous: Option<usize>) {
        let kind = self.kind(kind);
        self.idents.push((start(span), kind, previous));
    }

    fn call(&mut self, span: Span, kind: &str) {
        let kind = self.kind(kind);
        self.calls.push((start(span), kind));
    }

    fn cfg(&mut self, attrs: &[Attribute], span: Span) {
        if attrs.iter().any(|a| a.path().is_ident("cfg")) {
            self.cfg.push(range(span));
        }
    }

    fn segments(&mut self, path: &syn::Path, qself: bool, single_kind: Option<&str>) {
        let n = path.segments.len();
        let mut previous = None;
        for (i, seg) in path.segments.iter().enumerate() {
            let kind = match single_kind {
                Some(k) => k,
                None if qself => "path_qself",
                None if n == 1 && path.leading_colon.is_none() => "path_single",
                None if i + 1 < n => "path_prefix",
                None => "path_last",
            };
            self.ident_after(seg.ident.span(), kind, previous);
            previous = Some(start(seg.ident.span()));
            if single_kind.is_none() {
                visit::visit_path_arguments(self, &seg.arguments);
            }
        }
    }

    fn constant(&mut self, site: &str, expr: &Expr) {
        self.named_constant(site, None, expr);
    }

    /// `refs` lists the last segment of every SCREAMING_CASE or one-letter path the initializer mentions, so the
    /// census can follow references to other constants by name.
    fn named_constant(&mut self, site: &str, name: Option<&syn::Ident>, expr: &Expr) {
        let mut f = Features::default();
        features(expr, &mut f);
        let class = classify(&f);
        let name = name.map(|n| n.to_string());
        self.consts.push(json!({"site": site, "name": name, "class": class, "refs": f.refs, "macro": self.depth > 0}));
    }
}

#[derive(Default)]
struct Features {
    lit: bool,
    arith: bool,
    const_ref: bool,
    aggregate: bool,
    call: bool,
    mac: bool,
    other: bool,
    refs: Vec<String>,
}

fn screaming(name: &str) -> bool {
    name.chars().any(|c| c.is_ascii_uppercase()) && !name.chars().any(|c| c.is_ascii_lowercase())
}

fn constructor(name: &str) -> bool {
    name.chars().next().is_some_and(|c| c.is_ascii_uppercase()) && !screaming(name)
}

fn features(expr: &Expr, f: &mut Features) {
    match expr {
        Expr::Lit(_) => f.lit = true,
        Expr::Paren(e) => features(&e.expr, f),
        Expr::Group(e) => features(&e.expr, f),
        Expr::Unary(e) => {
            if matches!(e.op, UnOp::Deref(_)) { f.other = true } else { f.arith = true }
            features(&e.expr, f);
        }
        Expr::Binary(e) => {
            f.arith = true;
            features(&e.left, f);
            features(&e.right, f);
        }
        Expr::Cast(e) => {
            f.arith = true;
            features(&e.expr, f);
        }
        Expr::Path(e) => {
            let last = e.path.segments.last().map(|s| s.ident.to_string()).unwrap_or_default();
            if screaming(&last) || last.len() == 1 {
                f.const_ref = true;
                f.refs.push(last);
            } else {
                f.aggregate = true
            }
        }
        Expr::Call(e) => {
            match &*e.func {
                Expr::Path(p) if p.path.segments.last().is_some_and(|s| constructor(&s.ident.to_string())) => {
                    f.aggregate = true
                }
                _ => f.call = true,
            }
            for a in &e.args {
                features(a, f);
            }
        }
        Expr::MethodCall(e) => {
            f.call = true;
            features(&e.receiver, f);
            for a in &e.args {
                features(a, f);
            }
        }
        Expr::Struct(e) => {
            f.aggregate = true;
            for field in &e.fields {
                features(&field.expr, f);
            }
            if let Some(rest) = &e.rest {
                features(rest, f);
            }
        }
        Expr::Tuple(e) => {
            f.aggregate = true;
            for a in &e.elems {
                features(a, f);
            }
        }
        Expr::Array(e) => {
            f.aggregate = true;
            for a in &e.elems {
                features(a, f);
            }
        }
        Expr::Repeat(e) => {
            f.aggregate = true;
            features(&e.expr, f);
            features(&e.len, f);
        }
        Expr::Reference(e) => features(&e.expr, f),
        Expr::Macro(_) => f.mac = true,
        _ => f.other = true,
    }
}

/// Hardest feature wins: `other` (blocks, if/match, closures, field/index access, deref) > `macro` > `const_fn_call`
/// > `aggregate` (struct/tuple/array/repeat literal, tuple-struct or variant constructor, unit variant path) >
/// `const_ref` (a SCREAMING_CASE or one-letter path, mixed with literals and arithmetic) > `literal_arith` (unary,
/// binary and `as` over literals only) > `literal`.
fn classify(f: &Features) -> &'static str {
    if f.other {
        "other"
    } else if f.mac {
        "macro"
    } else if f.call {
        "const_fn_call"
    } else if f.aggregate {
        "aggregate"
    } else if f.const_ref {
        "const_ref"
    } else if f.arith {
        "literal_arith"
    } else if f.lit {
        "literal"
    } else {
        "other"
    }
}

fn item_attrs(item: &Item) -> &[Attribute] {
    match item {
        Item::Const(i) => &i.attrs,
        Item::Enum(i) => &i.attrs,
        Item::ExternCrate(i) => &i.attrs,
        Item::Fn(i) => &i.attrs,
        Item::ForeignMod(i) => &i.attrs,
        Item::Impl(i) => &i.attrs,
        Item::Macro(i) => &i.attrs,
        Item::Mod(i) => &i.attrs,
        Item::Static(i) => &i.attrs,
        Item::Struct(i) => &i.attrs,
        Item::Trait(i) => &i.attrs,
        Item::TraitAlias(i) => &i.attrs,
        Item::Type(i) => &i.attrs,
        Item::Union(i) => &i.attrs,
        Item::Use(i) => &i.attrs,
        _ => &[],
    }
}

fn expr_attrs(expr: &Expr) -> &[Attribute] {
    match expr {
        Expr::Array(e) => &e.attrs,
        Expr::Assign(e) => &e.attrs,
        Expr::Async(e) => &e.attrs,
        Expr::Await(e) => &e.attrs,
        Expr::Binary(e) => &e.attrs,
        Expr::Block(e) => &e.attrs,
        Expr::Break(e) => &e.attrs,
        Expr::Call(e) => &e.attrs,
        Expr::Cast(e) => &e.attrs,
        Expr::Closure(e) => &e.attrs,
        Expr::Const(e) => &e.attrs,
        Expr::Continue(e) => &e.attrs,
        Expr::Field(e) => &e.attrs,
        Expr::ForLoop(e) => &e.attrs,
        Expr::Group(e) => &e.attrs,
        Expr::If(e) => &e.attrs,
        Expr::Index(e) => &e.attrs,
        Expr::Infer(e) => &e.attrs,
        Expr::Let(e) => &e.attrs,
        Expr::Lit(e) => &e.attrs,
        Expr::Loop(e) => &e.attrs,
        Expr::Macro(e) => &e.attrs,
        Expr::Match(e) => &e.attrs,
        Expr::MethodCall(e) => &e.attrs,
        Expr::Paren(e) => &e.attrs,
        Expr::Path(e) => &e.attrs,
        Expr::Range(e) => &e.attrs,
        Expr::RawAddr(e) => &e.attrs,
        Expr::Reference(e) => &e.attrs,
        Expr::Repeat(e) => &e.attrs,
        Expr::Return(e) => &e.attrs,
        Expr::Struct(e) => &e.attrs,
        Expr::Try(e) => &e.attrs,
        Expr::TryBlock(e) => &e.attrs,
        Expr::Tuple(e) => &e.attrs,
        Expr::Unary(e) => &e.attrs,
        Expr::Unsafe(e) => &e.attrs,
        Expr::While(e) => &e.attrs,
        Expr::Yield(e) => &e.attrs,
        _ => &[],
    }
}

impl<'ast> Visit<'ast> for Scan {
    fn visit_file(&mut self, file: &'ast syn::File) {
        // `#![cfg(..)]` at the top of a source file switches the whole file.
        if self.depth == 0 && file.attrs.iter().any(|a| a.path().is_ident("cfg")) {
            self.cfg.push([0, usize::MAX]);
        }
        visit::visit_file(self, file);
    }

    fn visit_item(&mut self, item: &'ast Item) {
        self.cfg(item_attrs(item), item.span());
        visit::visit_item(self, item);
    }

    fn visit_impl_item(&mut self, item: &'ast syn::ImplItem) {
        let attrs: &[Attribute] = match item {
            syn::ImplItem::Const(i) => &i.attrs,
            syn::ImplItem::Fn(i) => &i.attrs,
            syn::ImplItem::Type(i) => &i.attrs,
            syn::ImplItem::Macro(i) => &i.attrs,
            _ => &[],
        };
        self.cfg(attrs, item.span());
        visit::visit_impl_item(self, item);
    }

    fn visit_trait_item(&mut self, item: &'ast syn::TraitItem) {
        let attrs: &[Attribute] = match item {
            syn::TraitItem::Const(i) => &i.attrs,
            syn::TraitItem::Fn(i) => &i.attrs,
            syn::TraitItem::Type(i) => &i.attrs,
            syn::TraitItem::Macro(i) => &i.attrs,
            _ => &[],
        };
        self.cfg(attrs, item.span());
        visit::visit_trait_item(self, item);
    }

    fn visit_local(&mut self, local: &'ast syn::Local) {
        self.cfg(&local.attrs, local.span());
        visit::visit_local(self, local);
    }

    fn visit_stmt_macro(&mut self, mac: &'ast syn::StmtMacro) {
        self.cfg(&mac.attrs, mac.span());
        visit::visit_stmt_macro(self, mac);
    }

    fn visit_expr(&mut self, expr: &'ast Expr) {
        self.cfg(expr_attrs(expr), expr.span());
        visit::visit_expr(self, expr);
    }

    fn visit_field(&mut self, field: &'ast syn::Field) {
        self.cfg(&field.attrs, field.span());
        visit::visit_field(self, field);
    }

    fn visit_variant(&mut self, variant: &'ast syn::Variant) {
        self.cfg(&variant.attrs, variant.span());
        if let Some((_, e)) = &variant.discriminant {
            self.constant("enum_discriminant", e);
        }
        visit::visit_variant(self, variant);
    }

    fn visit_arm(&mut self, arm: &'ast syn::Arm) {
        self.cfg(&arm.attrs, arm.span());
        visit::visit_arm(self, arm);
    }

    fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
        self.fns.push(range(item.span()));
        visit::visit_item_fn(self, item);
    }

    fn visit_impl_item_fn(&mut self, item: &'ast syn::ImplItemFn) {
        self.fns.push(range(item.span()));
        visit::visit_impl_item_fn(self, item);
    }

    fn visit_trait_item_fn(&mut self, item: &'ast syn::TraitItemFn) {
        self.fns.push(range(item.span()));
        visit::visit_trait_item_fn(self, item);
    }

    fn visit_item_const(&mut self, item: &'ast syn::ItemConst) {
        self.named_constant("const", Some(&item.ident), &item.expr);
        visit::visit_item_const(self, item);
    }

    fn visit_item_static(&mut self, item: &'ast syn::ItemStatic) {
        self.named_constant("static", Some(&item.ident), &item.expr);
        visit::visit_item_static(self, item);
    }

    fn visit_impl_item_const(&mut self, item: &'ast syn::ImplItemConst) {
        self.named_constant("assoc_const", Some(&item.ident), &item.expr);
        visit::visit_impl_item_const(self, item);
    }

    fn visit_trait_item_const(&mut self, item: &'ast syn::TraitItemConst) {
        if let Some((_, e)) = &item.default {
            self.named_constant("trait_const_default", Some(&item.ident), e);
        }
        visit::visit_trait_item_const(self, item);
    }

    fn visit_type_array(&mut self, ty: &'ast syn::TypeArray) {
        self.constant("array_len", &ty.len);
        visit::visit_type_array(self, ty);
    }

    fn visit_expr_repeat(&mut self, e: &'ast syn::ExprRepeat) {
        self.constant("repeat_len", &e.len);
        visit::visit_expr_repeat(self, e);
    }

    fn visit_generic_argument(&mut self, arg: &'ast syn::GenericArgument) {
        if let syn::GenericArgument::Const(e) = arg {
            self.constant("const_arg", e);
        }
        visit::visit_generic_argument(self, arg);
    }

    fn visit_const_param(&mut self, param: &'ast syn::ConstParam) {
        self.consts.push(json!({"site": "const_param", "class": "none", "macro": self.depth > 0}));
        if let Some(e) = &param.default {
            self.constant("const_param_default", e);
        }
        visit::visit_const_param(self, param);
    }

    fn visit_use_tree(&mut self, tree: &'ast UseTree) {
        match tree {
            UseTree::Path(p) => self.ident(p.ident.span(), "use"),
            UseTree::Name(n) => self.ident(n.ident.span(), "use"),
            UseTree::Rename(n) => self.ident(n.ident.span(), "use"),
            _ => {}
        }
        visit::visit_use_tree(self, tree);
    }

    fn visit_path(&mut self, path: &'ast syn::Path) {
        self.segments(path, false, None);
    }

    fn visit_expr_path(&mut self, e: &'ast syn::ExprPath) {
        if let Some(q) = &e.qself {
            self.visit_qself(q);
        }
        self.segments(&e.path, e.qself.is_some(), None);
    }

    fn visit_type_path(&mut self, t: &'ast syn::TypePath) {
        if let Some(q) = &t.qself {
            self.visit_qself(q);
        }
        self.segments(&t.path, t.qself.is_some(), None);
    }

    fn visit_attribute(&mut self, attr: &'ast Attribute) {
        self.segments(attr.path(), false, Some("attr"));
        if attr.path().is_ident("derive") {
            let parser = Punctuated::<syn::Path, syn::Token![,]>::parse_terminated;
            if let Ok(paths) = attr.parse_args_with(parser) {
                for p in &paths {
                    self.segments(p, false, Some("derive"));
                }
            }
        }
    }

    fn visit_pat_ident(&mut self, p: &'ast syn::PatIdent) {
        self.ident(p.ident.span(), "pat_ident");
        visit::visit_pat_ident(self, p);
    }

    fn visit_field_value(&mut self, fv: &'ast syn::FieldValue) {
        self.cfg(&fv.attrs, fv.span());
        for a in &fv.attrs {
            self.visit_attribute(a);
        }
        if let Member::Named(id) = &fv.member {
            if fv.colon_token.is_none() {
                self.ident(id.span(), "field_shorthand");
                return;
            }
            self.ident(id.span(), "field_init");
        }
        self.visit_expr(&fv.expr);
    }

    fn visit_field_pat(&mut self, fp: &'ast syn::FieldPat) {
        for a in &fp.attrs {
            self.visit_attribute(a);
        }
        if let Member::Named(id) = &fp.member {
            if fp.colon_token.is_none() {
                self.ident(id.span(), "field_pat_shorthand");
                return;
            }
            self.ident(id.span(), "field_pat");
        }
        self.visit_pat(&fp.pat);
    }

    fn visit_expr_field(&mut self, e: &'ast syn::ExprField) {
        if let Member::Named(id) = &e.member {
            self.ident(id.span(), "field");
        }
        visit::visit_expr_field(self, e);
    }

    fn visit_expr_method_call(&mut self, e: &'ast syn::ExprMethodCall) {
        self.ident(e.method.span(), "method");
        self.call(e.method.span(), "call_method");
        visit::visit_expr_method_call(self, e);
    }

    fn visit_expr_call(&mut self, e: &'ast syn::ExprCall) {
        match &*e.func {
            Expr::Path(p) if p.qself.is_none() => {
                let last = p.path.segments.last().map(|s| s.ident.span()).unwrap_or_else(|| p.span());
                self.call(last, "call_path");
            }
            Expr::Path(p) => {
                let last = p.path.segments.last().map(|s| s.ident.span()).unwrap_or_else(|| p.span());
                self.call(last, "call_qself");
            }
            other => self.call(other.span(), "call_other"),
        }
        visit::visit_expr_call(self, e);
    }

    fn visit_assoc_type(&mut self, a: &'ast syn::AssocType) {
        self.ident(a.ident.span(), "assoc_binding");
        visit::visit_assoc_type(self, a);
    }

    fn visit_assoc_const(&mut self, a: &'ast syn::AssocConst) {
        self.ident(a.ident.span(), "assoc_binding");
        visit::visit_assoc_const(self, a);
    }

    fn visit_constraint(&mut self, c: &'ast syn::Constraint) {
        self.ident(c.ident.span(), "assoc_binding");
        visit::visit_constraint(self, c);
    }

    fn visit_lit_str(&mut self, s: &'ast syn::LitStr) {
        if self.depth > 0 {
            self.strs.push(range(s.span()));
        }
    }

    fn visit_item_macro(&mut self, item: &'ast syn::ItemMacro) {
        if item.mac.path.is_ident("macro_rules") {
            self.rules.push(range(item.mac.delimiter.span().join()));
            return;
        }
        visit::visit_item_macro(self, item);
    }

    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        let tt = range(mac.delimiter.span().join());
        if mac.path.is_ident("macro_rules") {
            self.rules.push(tt);
            return;
        }
        self.segments(&mac.path, false, Some("macro_name"));
        if mac.tokens.is_empty() {
            return;
        }
        self.depth += 1;
        let tokens = mac.tokens.clone();
        if let Ok(file) = syn::parse2::<syn::File>(tokens.clone())
            && !file.items.is_empty()
            && !file.items.iter().any(|i| matches!(i, Item::Verbatim(_)))
        {
            self.parsed.push(tt);
            self.visit_file(&file);
        } else if let Ok(stmts) = syn::Block::parse_within.parse2(tokens.clone()) {
            self.parsed.push(tt);
            for s in &stmts {
                self.visit_stmt(s);
            }
        } else if let Ok(exprs) = Punctuated::<Expr, syn::Token![,]>::parse_terminated.parse2(tokens) {
            self.parsed.push(tt);
            for e in &exprs {
                self.visit_expr(e);
            }
        } else {
            self.opaque.push(tt);
        }
        self.depth -= 1;
    }
}

// ---- token-level lint attribute scan ----

#[derive(Clone, Copy)]
struct Ctx {
    top: bool,
    rules: bool,
    tt: bool,
}

const LEVELS: [&str; 5] = ["allow", "expect", "warn", "deny", "forbid"];

fn is_punct(t: Option<&TokenTree>, c: char) -> bool {
    matches!(t, Some(TokenTree::Punct(p)) if p.as_char() == c)
}

fn bang(t: Option<&TokenTree>) -> bool {
    matches!(t, Some(TokenTree::Punct(p)) if p.as_char() == '!' && p.spacing() == Spacing::Alone)
}

fn target_after(toks: &[TokenTree], mut k: usize) -> &'static str {
    loop {
        // Further attributes on the same node.
        if is_punct(toks.get(k), '#') {
            if matches!(toks.get(k + 1), Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Bracket) {
                k += 2;
                continue;
            }
        }
        if matches!(toks.get(k), Some(TokenTree::Ident(i)) if i == "pub") {
            k += 1;
            if matches!(toks.get(k), Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Parenthesis) {
                k += 1;
            }
            continue;
        }
        break;
    }
    let ident = |j: usize| match toks.get(j) {
        Some(TokenTree::Ident(i)) => Some(i.to_string()),
        _ => None,
    };
    let Some(first) = ident(k) else {
        return match toks.get(k) {
            Some(TokenTree::Punct(p)) if p.as_char() == '|' => "closure",
            Some(_) => "expr",
            None => "end",
        };
    };
    // Qualifiers before an item keyword.
    let mut j = k;
    while let Some(w) = ident(j) {
        match w.as_str() {
            "unsafe" | "async" | "extern" | "default" | "safe" => {
                j += 1;
                if matches!(toks.get(j), Some(TokenTree::Literal(_))) {
                    j += 1;
                }
            }
            "const" if matches!(ident(j + 1).as_deref(), Some("fn" | "unsafe" | "async" | "extern")) => j += 1,
            _ => break,
        }
    }
    if let Some(w) = ident(j) {
        match w.as_str() {
            "fn" => return "fn",
            "impl" => return "impl",
            "mod" => return "mod",
            "struct" | "enum" | "union" => return "adt",
            "trait" => return "trait",
            "const" | "static" => return "const_static",
            "type" => return "type_alias",
            "use" => return "use",
            "crate" if first == "extern" => return "extern_crate",
            "let" => return "let",
            "macro_rules" => return "macro_rules",
            "match" | "if" | "loop" | "while" | "for" | "return" | "unsafe" => return "expr",
            "move" => return "closure",
            _ => {}
        }
        if j != k {
            return "item_other";
        }
    }
    // `path::to::mac!` macro invocation.
    let mut m = k;
    loop {
        if !matches!(toks.get(m), Some(TokenTree::Ident(_))) {
            break;
        }
        m += 1;
        if bang(toks.get(m)) {
            return "macro_call";
        }
        if is_punct(toks.get(m), ':') && is_punct(toks.get(m + 1), ':') {
            m += 2;
            continue;
        }
        break;
    }
    if is_punct(toks.get(k + 1), ':') && !is_punct(toks.get(k + 2), ':') {
        return "field";
    }
    if matches!(toks.get(k + 1), Some(TokenTree::Punct(p)) if p.as_char() == ',') {
        return "variant";
    }
    if matches!(toks.get(k + 1), Some(TokenTree::Group(_)) | None) {
        return "variant_or_expr";
    }
    if is_punct(toks.get(k + 1), '=') && is_punct(toks.get(k + 2), '>') {
        return "match_arm";
    }
    "expr"
}

fn split_commas(ts: TokenStream) -> Vec<Vec<TokenTree>> {
    let mut parts = vec![Vec::new()];
    for t in ts {
        if matches!(&t, TokenTree::Punct(p) if p.as_char() == ',') {
            parts.push(Vec::new());
        } else {
            parts.last_mut().unwrap().push(t);
        }
    }
    parts.retain(|p| !p.is_empty());
    parts
}

#[allow(clippy::too_many_arguments)]
fn lint_attr(toks: &[TokenTree], inner: bool, cfg_pred: Option<&str>, target: &str, ctx: Ctx, at: usize, out: &mut Vec<Value>) {
    let Some(TokenTree::Ident(name)) = toks.first() else { return };
    let name = name.to_string();
    let Some(TokenTree::Group(args)) = toks.get(1) else { return };
    if name == "cfg_attr" {
        let parts = split_commas(args.stream());
        let Some((pred, rest)) = parts.split_first() else { return };
        let pred: String = pred.iter().cloned().collect::<TokenStream>().to_string();
        for p in rest {
            lint_attr(p, inner, Some(&pred), target, ctx, at, out);
        }
        return;
    }
    if !LEVELS.contains(&name.as_str()) {
        return;
    }
    let mut lints = Vec::new();
    let mut reason = false;
    for part in split_commas(args.stream()) {
        if matches!(part.first(), Some(TokenTree::Ident(i)) if i == "reason") && is_punct(part.get(1), '=') {
            reason = true;
            continue;
        }
        lints.push(part.iter().map(|t| t.to_string()).collect::<String>());
    }
    out.push(json!({
        "at": at, "inner": inner, "level": name, "lints": lints, "reason": reason,
        "cfg_attr": cfg_pred, "target": target, "in_macro_rules": ctx.rules, "in_macro_tt": ctx.tt,
    }));
}

fn scan_attrs(ts: TokenStream, ctx: Ctx, out: &mut Vec<Value>) {
    let toks: Vec<TokenTree> = ts.into_iter().collect();
    let mut i = 0;
    while i < toks.len() {
        match &toks[i] {
            TokenTree::Punct(p) if p.as_char() == '#' => {
                let inner = is_punct(toks.get(i + 1), '!');
                let j = if inner { i + 2 } else { i + 1 };
                if let Some(TokenTree::Group(g)) = toks.get(j)
                    && g.delimiter() == Delimiter::Bracket
                {
                    let target = if inner {
                        if ctx.top { "crate" } else { "inner" }
                    } else {
                        target_after(&toks, j + 1)
                    };
                    let body: Vec<TokenTree> = g.stream().into_iter().collect();
                    lint_attr(&body, inner, None, target, ctx, start(p.span()), out);
                    i = j + 1;
                    continue;
                }
            }
            TokenTree::Ident(id) if id == "macro_rules" && bang(toks.get(i + 1)) => {
                if let Some(TokenTree::Group(g)) = toks.get(i + 3) {
                    scan_attrs(g.stream(), Ctx { top: false, rules: true, ..ctx }, out);
                    i += 4;
                    continue;
                }
            }
            TokenTree::Ident(_) if bang(toks.get(i + 1)) => {
                if let Some(TokenTree::Group(g)) = toks.get(i + 2) {
                    scan_attrs(g.stream(), Ctx { top: false, tt: true, ..ctx }, out);
                    i += 3;
                    continue;
                }
            }
            TokenTree::Group(g) => scan_attrs(g.stream(), Ctx { top: false, ..ctx }, out),
            _ => {}
        }
        i += 1;
    }
}

// ---- scan driver ----

fn rust_files(root: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<_> = std::fs::read_dir(root).unwrap().filter_map(Result::ok).collect();
    entries.sort_by_key(|e| e.path());
    for e in entries {
        let p = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        if p.is_dir() {
            if name != "target" && name != ".git" {
                rust_files(&p, out);
            }
        } else if name.ends_with(".rs") {
            out.push(p);
        }
    }
}

fn scan(root: &Path, out: &Path) {
    let mut files = Vec::new();
    rust_files(root, &mut files);
    let mut w = std::io::BufWriter::new(std::fs::File::create(out).unwrap());
    for path in files {
        proc_macro2::extra::invalidate_current_thread_spans();
        let rel = path.strip_prefix(root).unwrap().to_string_lossy().to_string();
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        let mut s = Scan::default();
        let mut attrs = Vec::new();
        let lexed = match text.parse::<TokenStream>() {
            Ok(ts) => {
                scan_attrs(ts, Ctx { top: true, rules: false, tt: false }, &mut attrs);
                true
            }
            Err(_) => false,
        };
        // Spans from the lexing pass above and the parse below live in separate files of proc-macro2's source
        // map; byte ranges are relative to each, so both count from the start of this file.
        let parsed = match syn::parse_file(&text) {
            Ok(file) => {
                s.visit_file(&file);
                Value::Null
            }
            Err(e) => {
                let lc = e.span().start();
                json!(format!("{}:{}: {}", lc.line, lc.column + 1, e))
            }
        };
        let pairs = |v: &Vec<(usize, String)>| v.iter().map(|(a, k)| json!([a, k])).collect::<Vec<_>>();
        let idents: Vec<Value> = s
            .idents
            .iter()
            .map(|(a, k, p)| match p {
                Some(p) => json!([a, k, p]),
                None => json!([a, k]),
            })
            .collect();
        let line = json!({
            "path": rel, "lexed": lexed, "error": parsed, "bytes": text.len(),
            "idents": idents, "calls": pairs(&s.calls), "opaque": s.opaque, "parsed": s.parsed,
            "rules": s.rules, "fns": s.fns, "cfg": s.cfg, "strs": s.strs, "consts": s.consts, "attrs": attrs,
        });
        writeln!(w, "{line}").unwrap();
    }
}

// ---- SCIP decoding ----

struct Pb<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> Pb<'a> {
    fn varint(&mut self) -> u64 {
        let mut v = 0u64;
        let mut shift = 0;
        loop {
            let byte = self.b[self.i];
            self.i += 1;
            v |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return v;
            }
            shift += 7;
        }
    }

    /// Next field as (number, wire type, length-delimited payload or empty, varint value).
    fn field(&mut self) -> Option<(u64, u64, &'a [u8], u64)> {
        if self.i >= self.b.len() {
            return None;
        }
        let key = self.varint();
        let (num, wt) = (key >> 3, key & 7);
        match wt {
            0 => {
                let v = self.varint();
                Some((num, wt, &[], v))
            }
            1 => {
                self.i += 8;
                Some((num, wt, &[], 0))
            }
            2 => {
                let len = self.varint() as usize;
                let s = &self.b[self.i..self.i + len];
                self.i += len;
                Some((num, wt, s, 0))
            }
            5 => {
                self.i += 4;
                Some((num, wt, &[], 0))
            }
            _ => panic!("unsupported wire type {wt}"),
        }
    }
}

struct Interner {
    ids: HashMap<String, usize>,
    names: Vec<String>,
}

impl Interner {
    fn id(&mut self, s: &[u8]) -> usize {
        let s = String::from_utf8_lossy(s).to_string();
        if let Some(&i) = self.ids.get(&s) {
            return i;
        }
        self.names.push(s.clone());
        self.ids.insert(s, self.names.len() - 1);
        self.names.len() - 1
    }
}

fn symbol_info(b: &[u8], syms: &mut Interner) -> Value {
    let mut p = Pb { b, i: 0 };
    let (mut sym, mut kind) = (0usize, 0u64);
    while let Some((num, _, s, v)) = p.field() {
        match num {
            1 => sym = syms.id(s),
            5 => kind = v,
            _ => {}
        }
    }
    json!([sym, kind])
}

fn scip(input: &Path, out: &Path) {
    let data = std::fs::read(input).unwrap();
    let mut syms = Interner { ids: HashMap::new(), names: Vec::new() };
    let mut w = std::io::BufWriter::new(std::fs::File::create(out).unwrap());
    let mut external = Vec::new();
    let mut index = Pb { b: &data, i: 0 };
    while let Some((num, _, s, _)) = index.field() {
        match num {
            2 => {
                let mut doc = Pb { b: s, i: 0 };
                let (mut path, mut encoding) = (String::new(), 0u64);
                let (mut occ, mut infos) = (Vec::new(), Vec::new());
                while let Some((dn, _, ds, dv)) = doc.field() {
                    match dn {
                        1 => path = String::from_utf8_lossy(ds).to_string(),
                        2 => {
                            let mut o = Pb { b: ds, i: 0 };
                            let (mut rng, mut sym, mut roles) = (Vec::new(), 0usize, 0u64);
                            while let Some((on, owt, os, ov)) = o.field() {
                                match (on, owt) {
                                    (1, 2) => {
                                        let mut r = Pb { b: os, i: 0 };
                                        while r.i < os.len() {
                                            rng.push(r.varint());
                                        }
                                    }
                                    (1, 0) => rng.push(ov),
                                    (2, _) => sym = syms.id(os),
                                    (3, _) => roles = ov,
                                    _ => {}
                                }
                            }
                            let (sl, sc, el, ec) = if rng.len() == 3 {
                                (rng[0], rng[1], rng[0], rng[2])
                            } else {
                                (rng[0], rng[1], rng[2], rng[3])
                            };
                            occ.push(json!([sl, sc, el, ec, sym, roles]));
                        }
                        3 => infos.push(symbol_info(ds, &mut syms)),
                        6 => encoding = dv,
                        _ => {}
                    }
                }
                writeln!(w, "{}", json!({"path": path, "encoding": encoding, "occ": occ, "syms": infos})).unwrap();
            }
            3 => external.push(symbol_info(s, &mut syms)),
            _ => {}
        }
    }
    writeln!(w, "{}", json!({"symbols": syms.names, "external": external})).unwrap();
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("scan") => scan(Path::new(&args[2]), Path::new(&args[3])),
        Some("scip") => scip(Path::new(&args[2]), Path::new(&args[3])),
        _ => {
            eprintln!("usage: rerail-rust-feasibility (scan <root> | scip <index.scip>) <out.jsonl>");
            std::process::exit(2);
        }
    }
}
