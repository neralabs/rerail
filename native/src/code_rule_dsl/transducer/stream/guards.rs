//! G0: the structural guards (`docs/parser-generators/09-compile-and-lower.md` §12.3).
//!
//! They read the stream tree's source as Rust syntax (`syn`), not as text, and check its shape:
//!
//! - G0.1: the file layout is simdjson's;
//! - G0.2: the API is simdjson's;
//! - G0.3: intrinsics appear only in the per-architecture layer;
//! - G0.4: the layer has no loops or branches;
//! - G0.5: nothing above the layer touches a byte or a lane;
//! - G0.6: `Simd8x64` checks its register count at compile time;
//! - G0.7: every method of the layer is `#[inline(always)]`;
//! - G0.8: `Simd8x64` cannot be copied.
//!
//! `syn` does not expand macros, so a macro could hide a loop. G0.4 and G0.5 therefore report
//! every macro but a short list whose expansion has none.
//!
//! Each guard is a function from source to violations. The tests run it on the tree, and on
//! snippets that break its rule, to show that it reports them.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use syn::visit::Visit;

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/src/code_rule_dsl/transducer/stream");

/// The gates whose files must exist (G0.1) and whose API must be complete (G0.2).
const CLAIMED: &[&str] = &["G1"];

/// §12.2: simdjson's path, the Rust path under `stream/`, and the gate that adds it.
const LAYOUT: &[(&str, &str, &str)] = &[
    ("include/simdjson/arm64/simd.h", "arm64/simd.rs", "G1"),
    ("include/simdjson/arm64/bitmask.h", "arm64/bitmask.rs", "G1"),
    ("include/simdjson/arm64/bitmanipulation.h", "arm64/bitmanipulation.rs", "G1"),
    ("include/simdjson/internal/simdprune_tables.h", "internal/simdprune_tables.rs", "G1"),
    ("src/generic/json_character_block.h", "generic/json_character_block.rs", "G2"),
    ("src/arm64.cpp", "arm64.rs", "G2"),
    ("src/generic/stage1/buf_block_reader.h", "generic/stage1/buf_block_reader.rs", "G2"),
    ("src/generic/stage1/json_escape_scanner.h", "generic/stage1/json_escape_scanner.rs", "G2"),
    ("src/generic/stage1/json_string_scanner.h", "generic/stage1/json_string_scanner.rs", "G2"),
    ("src/generic/stage1/json_scanner.h", "generic/stage1/json_scanner.rs", "G2"),
    ("src/generic/stage1/json_structural_indexer.h", "generic/stage1/json_structural_indexer.rs", "G2"),
];

/// The primitive layer: per-architecture files, where G0.4 and G0.7 apply.
const LAYER: &[&str] = &["arm64/simd.rs", "arm64/bitmask.rs", "arm64/bitmanipulation.rs"];

/// Where intrinsics may be named (G0.3).
const INTRINSICS_UNDER: &str = "arm64/";

/// Not above the layer (G0.5): the layer itself, and simdjson's tables, built by `const fn`.
const BELOW: &[&str] = &["arm64/", "internal/"];

/// G0.4's allow-list: a layer function, the one construct it may use, and why (as in simdjson).
const LAYER_ALLOW: &[(&str, &str, &str)] = &[
    ("compress", "index", "simdjson reads thintable_epi8 and BitsSetTable256mul2 by mask byte"),
    ("compress_halves", "index", "simdjson reads thintable_epi8 by mask byte"),
    ("prev", "match", "vextq_u8 takes 16 - N as a constant; the match on const N folds away"),
];

/// G0.5's allow-list: a function above the layer that may loop, and why. simdjson's own loops.
const ABOVE_ALLOW: &[(&str, &str)] = &[
    ("index", "json_structural_indexer::index: the loop over steps of the input"),
    ("write", "bit_indexer::write: the tail past 24 set bits"),
];

/// Macros the layer may use: their expansion has no loop or branch.
const LAYER_MACROS: &[&str] = &["`assert!`"];

/// Macros above the layer may use.
const ABOVE_MACROS: &[&str] = &["`assert!`", "`debug_assert!`", "`assert_eq!`", "`debug_assert_eq!`", "`unreachable!`", "`panic!`"];

/// §12.4: a type, the impl keys that may define its methods, and the names it must have.
fn api() -> Vec<(&'static str, Vec<&'static str>, Vec<&'static str>)> {
    let base = ["bitor", "bitand", "bitxor", "bit_andnot", "not", "bitor_assign", "bitand_assign", "bitxor_assign", "eq", "prev"];
    let mut b = vec!["splat", "to_bitmask", "to_bitmask64", "any"];
    b.extend(base);
    let mut u = vec![
        "splat", "zero", "load", "new", "repeat_16", "store", "saturating_add", "saturating_sub", "add", "sub", "add_assign", "sub_assign",
        "max_val", "min_val", "max_val_with", "min_val_with", "le", "ge", "lt", "gt", "gt_bits", "lt_bits", "any_bits_set",
        "any_bits_set_anywhere", "any_bits_set_anywhere_in", "shr", "shl", "lookup_16", "lookup_16_values", "to_bitmask64", "compress",
        "compress_halves", "apply_lookup_16_to",
    ];
    u.extend(base);
    let i = vec![
        "splat", "zero", "load", "new", "repeat_16", "store", "from_u8", "to_u8", "add", "sub", "add_assign", "sub_assign", "max_val",
        "min_val", "gt", "lt", "eq", "prev", "lookup_16", "lookup_16_values", "apply_lookup_16_to",
    ];
    vec![
        ("Simd8<bool>", vec!["Simd8<bool>", "Simd8<T>"], b),
        ("Simd8<u8>", vec!["Simd8<u8>", "Simd8<T>"], u),
        ("Simd8<i8>", vec!["Simd8<i8>"], i),
        (
            "Simd8x64",
            vec!["Simd8x64<T>", "Simd8x64<u8>", "Simd8x64<bool>"],
            vec!["NUM_CHUNKS", "get", "new", "load", "store", "reduce_or", "compress", "to_bitmask", "eq", "lteq"],
        ),
        ("bitmask", vec!["mod bitmask"], vec!["prefix_xor"]),
        (
            "bitmanipulation",
            vec!["mod bitmanipulation"],
            vec!["trailing_zeroes", "clear_lowest_bit", "leading_zeroes", "count_ones", "reverse_bits", "zero_leading_bit"],
        ),
        ("simdprune_tables", vec!["mod simdprune_tables"], vec!["THINTABLE_EPI8", "BITS_SET_TABLE_256_MUL2", "PSHUFB_COMBINE_TABLE"]),
    ]
}

// ---- reading the tree ----

/// Every `.rs` file under `stream/`, by path relative to it, except tests and this file.
fn tree() -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let mut stack = vec![Path::new(ROOT).to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "rs") {
                let rel = p.strip_prefix(ROOT).unwrap().to_string_lossy().replace('\\', "/");
                if !is_test_file(&rel) {
                    out.insert(rel, std::fs::read_to_string(&p).unwrap());
                }
            }
        }
    }
    out
}

fn is_test_file(rel: &str) -> bool {
    rel == "guards.rs" || rel.ends_with("_tests.rs")
}

fn parse(rel: &str, src: &str) -> syn::File {
    let mut f = syn::parse_file(src).unwrap_or_else(|e| panic!("{rel}: {e}"));
    f.items.retain(|i| !is_cfg_test(item_attrs(i)));
    f
}

fn item_attrs(i: &syn::Item) -> &[syn::Attribute] {
    match i {
        syn::Item::Mod(m) => &m.attrs,
        syn::Item::Fn(f) => &f.attrs,
        syn::Item::Impl(x) => &x.attrs,
        syn::Item::Use(u) => &u.attrs,
        _ => &[],
    }
}

fn is_cfg_test(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| a.path().is_ident("cfg") && a.meta.require_list().is_ok_and(|l| l.tokens.to_string() == "test"))
}

fn stem(rel: &str) -> &str {
    rel.rsplit('/').next().unwrap().trim_end_matches(".rs")
}

/// `Simd8<u8>`, `Simd8x64<T>` … from an impl's self type.
fn type_key(t: &syn::Type) -> String {
    let syn::Type::Path(p) = t else { return "?".into() };
    let seg = p.path.segments.last().unwrap();
    let mut s = seg.ident.to_string();
    if let syn::PathArguments::AngleBracketed(a) = &seg.arguments {
        let args: Vec<String> = a
            .args
            .iter()
            .map(|g| match g {
                syn::GenericArgument::Type(t) => type_key(t),
                _ => "?".into(),
            })
            .collect();
        s = format!("{s}<{}>", args.join(","));
    }
    s
}

// ---- G0.1 layout ----

fn check_layout(files: &BTreeSet<String>, claimed: &[&str]) -> Vec<String> {
    LAYOUT
        .iter()
        .filter(|(_, _, g)| claimed.contains(g))
        .filter(|(_, rust, _)| !files.contains(*rust))
        .map(|(c, rust, g)| format!("G0.1 {g}: {rust} (simdjson's {c}) is missing"))
        .collect()
}

// ---- G0.2 API ----

/// For each impl key (`Simd8<u8>`, or `mod bitmask` for free items), the names it defines.
fn defined(files: &BTreeMap<String, syn::File>) -> BTreeMap<String, BTreeSet<String>> {
    let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (rel, f) in files {
        let module = format!("mod {}", stem(rel));
        for item in &f.items {
            match item {
                syn::Item::Impl(i) => {
                    let names = out.entry(type_key(&i.self_ty)).or_default();
                    for it in &i.items {
                        match it {
                            syn::ImplItem::Fn(m) => names.insert(m.sig.ident.to_string()),
                            syn::ImplItem::Const(c) => names.insert(c.ident.to_string()),
                            _ => false,
                        };
                    }
                }
                syn::Item::Fn(x) => {
                    out.entry(module.clone()).or_default().insert(x.sig.ident.to_string());
                }
                syn::Item::Const(c) => {
                    out.entry(module.clone()).or_default().insert(c.ident.to_string());
                }
                syn::Item::Static(s) => {
                    out.entry(module.clone()).or_default().insert(s.ident.to_string());
                }
                _ => {}
            }
        }
    }
    out
}

fn check_api(files: &BTreeMap<String, syn::File>) -> Vec<String> {
    let have = defined(files);
    let mut out = vec![];
    for (ty, keys, names) in api() {
        for n in names {
            if !keys.iter().any(|k| have.get(*k).is_some_and(|s| s.contains(n))) {
                out.push(format!("G0.2 {ty}: no `{n}` (simdjson's API, §12.4)"));
            }
        }
    }
    out
}

// ---- G0.3 intrinsics ----

#[derive(Default)]
struct ArchPaths(Vec<String>);

fn names_arch(t: &syn::UseTree) -> bool {
    match t {
        syn::UseTree::Path(n) => n.ident == "arch",
        syn::UseTree::Name(n) => n.ident == "arch",
        syn::UseTree::Rename(n) => n.ident == "arch",
        syn::UseTree::Group(g) => g.items.iter().any(names_arch),
        syn::UseTree::Glob(_) => false,
    }
}

impl<'a> Visit<'a> for ArchPaths {
    fn visit_path(&mut self, p: &'a syn::Path) {
        let segs: Vec<String> = p.segments.iter().map(|s| s.ident.to_string()).collect();
        if segs.windows(2).any(|w| (w[0] == "core" || w[0] == "std") && w[1] == "arch") {
            self.0.push(segs.join("::"));
        }
        syn::visit::visit_path(self, p);
    }
    fn visit_use_path(&mut self, u: &'a syn::UsePath) {
        if (u.ident == "core" || u.ident == "std") && names_arch(&u.tree) {
            self.0.push(format!("use {}::arch", u.ident));
        }
        syn::visit::visit_use_path(self, u);
    }
}

fn check_intrinsics(rel: &str, f: &syn::File) -> Vec<String> {
    if rel.starts_with(INTRINSICS_UNDER) {
        return vec![];
    }
    let mut v = ArchPaths::default();
    v.visit_file(f);
    v.0.into_iter().map(|p| format!("G0.3 {rel}: names `{p}`; intrinsics belong under stream/{INTRINSICS_UNDER}")).collect()
}

// ---- G0.4 and G0.5: what a function body may contain ----

/// Iterator methods: a loop over elements under another name.
const ITERATOR_CALLS: &[&str] = &[
    "iter", "iter_mut", "into_iter", "map", "fold", "for_each", "zip", "enumerate", "chunks", "chunks_exact", "windows", "bytes", "filter",
    "collect", "sum", "position", "rev", "flat_map", "filter_map", "try_fold", "scan",
];

/// Constructs found in function bodies: the enclosing function, the kind, and what it is.
#[derive(Default)]
struct Body {
    fns: Vec<String>,
    found: Vec<(String, &'static str, String)>,
}

impl Body {
    fn hit(&mut self, kind: &'static str, what: String) {
        let f = self.fns.last().cloned().unwrap_or_else(|| "<item>".into());
        self.found.push((f, kind, what));
    }
}

/// A constant index (`chunks[0]`, `chunks[IDX]`) reads a register, not a lane.
fn constant_index(e: &syn::Expr) -> bool {
    match e {
        syn::Expr::Lit(l) => matches!(l.lit, syn::Lit::Int(_)),
        syn::Expr::Path(p) => {
            p.path.get_ident().is_some_and(|i| i.to_string().chars().all(|c| c.is_ascii_uppercase() || c == '_' || c.is_ascii_digit()))
        }
        syn::Expr::Paren(p) => constant_index(&p.expr),
        _ => false,
    }
}

impl<'a> Visit<'a> for Body {
    fn visit_item_fn(&mut self, f: &'a syn::ItemFn) {
        if is_cfg_test(&f.attrs) {
            return;
        }
        self.fns.push(f.sig.ident.to_string());
        syn::visit::visit_item_fn(self, f);
        self.fns.pop();
    }
    fn visit_impl_item_fn(&mut self, f: &'a syn::ImplItemFn) {
        self.fns.push(f.sig.ident.to_string());
        syn::visit::visit_impl_item_fn(self, f);
        self.fns.pop();
    }
    fn visit_item_mod(&mut self, m: &'a syn::ItemMod) {
        if !is_cfg_test(&m.attrs) {
            syn::visit::visit_item_mod(self, m);
        }
    }
    fn visit_expr_for_loop(&mut self, e: &'a syn::ExprForLoop) {
        self.hit("loop", "`for`".into());
        syn::visit::visit_expr_for_loop(self, e);
    }
    fn visit_expr_while(&mut self, e: &'a syn::ExprWhile) {
        self.hit("loop", "`while`".into());
        syn::visit::visit_expr_while(self, e);
    }
    fn visit_expr_loop(&mut self, e: &'a syn::ExprLoop) {
        self.hit("loop", "`loop`".into());
        syn::visit::visit_expr_loop(self, e);
    }
    fn visit_expr_if(&mut self, e: &'a syn::ExprIf) {
        self.hit("if", "`if`".into());
        syn::visit::visit_expr_if(self, e);
    }
    fn visit_expr_match(&mut self, e: &'a syn::ExprMatch) {
        self.hit("match", "`match`".into());
        syn::visit::visit_expr_match(self, e);
    }
    fn visit_expr_index(&mut self, e: &'a syn::ExprIndex) {
        if !constant_index(&e.index) {
            self.hit("index", "an index by a runtime value".into());
        }
        syn::visit::visit_expr_index(self, e);
    }
    fn visit_expr_range(&mut self, e: &'a syn::ExprRange) {
        self.hit("range", "a range".into());
        syn::visit::visit_expr_range(self, e);
    }
    fn visit_expr_closure(&mut self, e: &'a syn::ExprClosure) {
        self.hit("closure", "a closure".into());
        syn::visit::visit_expr_closure(self, e);
    }
    fn visit_expr_method_call(&mut self, e: &'a syn::ExprMethodCall) {
        let m = e.method.to_string();
        if ITERATOR_CALLS.contains(&m.as_str()) {
            self.hit("iterator", format!("`.{m}()`"));
        }
        syn::visit::visit_expr_method_call(self, e);
    }
    fn visit_expr_path(&mut self, e: &'a syn::ExprPath) {
        if e.path.segments.last().is_some_and(|s| s.ident == "from_fn") {
            self.hit("iterator", "`from_fn`".into());
        }
        syn::visit::visit_expr_path(self, e);
    }
    fn visit_macro(&mut self, m: &'a syn::Macro) {
        let name = m.path.segments.last().map(|s| s.ident.to_string()).unwrap_or_default();
        self.hit("macro", format!("`{name}!`"));
        syn::visit::visit_macro(self, m);
    }
    fn visit_item_macro(&mut self, m: &'a syn::ItemMacro) {
        if m.mac.path.is_ident("macro_rules") {
            self.hit("macro", "a `macro_rules!` definition".into());
            return;
        }
        syn::visit::visit_item_macro(self, m);
    }
    fn visit_type_array(&mut self, t: &'a syn::TypeArray) {
        if let syn::Type::Path(p) = &*t.elem {
            if p.path.get_ident().is_some_and(|i| i == "u8" || i == "i8" || i == "bool") {
                self.hit("bytes", "an array of bytes".into());
            }
        }
        syn::visit::visit_type_array(self, t);
    }
}

fn body(f: &syn::File) -> Vec<(String, &'static str, String)> {
    let mut b = Body::default();
    b.visit_file(f);
    b.found
}

/// G0.4: the layer is straight-line intrinsics.
fn check_layer(rel: &str, f: &syn::File) -> Vec<String> {
    body(f)
        .into_iter()
        .filter(|(_, kind, _)| *kind != "bytes")
        .filter(|(_, kind, what)| !(*kind == "macro" && LAYER_MACROS.contains(&what.as_str())))
        .filter(|(func, kind, _)| !LAYER_ALLOW.iter().any(|(n, k, _)| n == func && k == kind))
        .map(|(func, _, what)| format!("G0.4 {rel}: `{func}` has {what}; the layer is straight-line intrinsics"))
        .collect()
}

/// G0.5: above the layer, no byte arrays, no lane indexing, no loops but simdjson's.
fn check_above(rel: &str, f: &syn::File) -> Vec<String> {
    body(f)
        .into_iter()
        .filter(|(func, kind, _)| !(*kind == "loop" && ABOVE_ALLOW.iter().any(|(n, _)| n == func)))
        .filter(|(_, kind, _)| *kind != "if" && *kind != "match")
        .filter(|(_, kind, what)| !(*kind == "macro" && ABOVE_MACROS.contains(&what.as_str())))
        .map(|(func, _, what)| format!("G0.5 {rel}: `{func}` has {what}; above the layer, work goes through Simd8x64 and u64 masks"))
        .collect()
}

// ---- G0.6, G0.7, G0.8 ----

/// G0.6: a module-level `const _: () = assert!(… NUM_CHUNKS == 4 …)`.
fn check_num_chunks(rel: &str, f: &syn::File) -> Vec<String> {
    let ok = f.items.iter().any(|i| {
        let syn::Item::Const(c) = i else { return false };
        let syn::Expr::Macro(m) = &*c.expr else { return false };
        let t = m.mac.tokens.to_string();
        c.ident == "_" && m.mac.path.is_ident("assert") && t.contains("NUM_CHUNKS") && t.contains("== 4")
    });
    if ok { vec![] } else { vec![format!("G0.6 {rel}: no `const _: () = assert!(… NUM_CHUNKS == 4)`, simdjson's static_assert")] }
}

fn inline_always(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| a.path().is_ident("inline") && a.meta.require_list().is_ok_and(|l| l.tokens.to_string() == "always"))
}

/// G0.7: every function of the layer is `#[inline(always)]`.
fn check_inline(rel: &str, f: &syn::File) -> Vec<String> {
    let mut out = vec![];
    for item in &f.items {
        match item {
            syn::Item::Fn(x) if !inline_always(&x.attrs) => out.push(x.sig.ident.to_string()),
            syn::Item::Impl(i) => {
                for it in &i.items {
                    if let syn::ImplItem::Fn(m) = it {
                        if !inline_always(&m.attrs) {
                            out.push(format!("{}::{}", type_key(&i.self_ty), m.sig.ident));
                        }
                    }
                }
            }
            _ => {}
        }
    }
    out.into_iter().map(|n| format!("G0.7 {rel}: `{n}` is not #[inline(always)] (simdjson_inline)")).collect()
}

/// G0.8: `Simd8x64` derives and implements neither `Clone` nor `Copy`.
fn check_no_copies(rel: &str, f: &syn::File) -> Vec<String> {
    let mut out = vec![];
    for item in &f.items {
        match item {
            syn::Item::Struct(s) if s.ident == "Simd8x64" => {
                for a in &s.attrs {
                    if a.path().is_ident("derive") {
                        let t = a.meta.require_list().map(|l| l.tokens.to_string()).unwrap_or_default();
                        if t.split(',').any(|d| matches!(d.trim(), "Clone" | "Copy")) {
                            out.push(format!("G0.8 {rel}: Simd8x64 derives {t}; simdjson deletes its copy constructor"));
                        }
                    }
                }
            }
            syn::Item::Impl(i) if type_key(&i.self_ty).starts_with("Simd8x64") => {
                if let Some((_, p, _)) = &i.trait_ {
                    let n = p.segments.last().unwrap().ident.to_string();
                    if n == "Clone" || n == "Copy" {
                        out.push(format!("G0.8 {rel}: Simd8x64 implements {n}; simdjson deletes its copy constructor"));
                    }
                }
            }
            _ => {}
        }
    }
    out
}

// ---- all guards over a tree ----

fn check_tree(src: &BTreeMap<String, String>, claimed: &[&str]) -> Vec<String> {
    let files: BTreeMap<String, syn::File> = src.iter().map(|(r, s)| (r.clone(), parse(r, s))).collect();
    let names: BTreeSet<String> = files.keys().cloned().collect();
    let mut out = check_layout(&names, claimed);
    if claimed.contains(&"G1") {
        out.extend(check_api(&files));
    }
    for (rel, f) in &files {
        out.extend(check_intrinsics(rel, f));
        if LAYER.contains(&rel.as_str()) {
            out.extend(check_layer(rel, f));
            out.extend(check_inline(rel, f));
            out.extend(check_no_copies(rel, f));
        } else if !BELOW.iter().any(|b| rel.starts_with(b)) {
            out.extend(check_above(rel, f));
        }
    }
    if let Some(f) = files.get("arm64/simd.rs") {
        out.extend(check_num_chunks("arm64/simd.rs", f));
    }
    out
}

fn report(v: Vec<String>) {
    assert!(v.is_empty(), "{} structural violations:\n{}", v.len(), v.join("\n"));
}

#[test]
fn g0_tree_holds() {
    report(check_tree(&tree(), CLAIMED));
}

// ---- each guard catches what it forbids ----

fn one(rel: &str, src: &str) -> BTreeMap<String, String> {
    BTreeMap::from([(rel.to_string(), src.to_string())])
}

fn caught(v: &[String], code: &str, what: &str) {
    assert!(v.iter().any(|m| m.starts_with(code) && m.contains(what)), "{code} did not report {what:?}: {v:#?}");
}

#[test]
fn g0_1_reports_a_missing_file() {
    let v = check_layout(&BTreeSet::from(["arm64/simd.rs".to_string()]), &["G1"]);
    caught(&v, "G0.1", "arm64/bitmask.rs");
    assert!(!v.iter().any(|m| m.contains("arm64/simd.rs ")), "{v:#?}");
    assert!(check_layout(&BTreeSet::new(), &[]).is_empty());
}

#[test]
fn g0_2_reports_a_missing_method() {
    let f = parse("arm64/simd.rs", "impl Simd8<u8> { fn splat() {} } impl<T> Simd8<T> { fn bitor() {} }");
    let v = check_api(&BTreeMap::from([("arm64/simd.rs".to_string(), f)]));
    caught(&v, "G0.2 Simd8<u8>", "`saturating_sub`");
    caught(&v, "G0.2 Simd8<i8>", "`eq`");
    assert!(!v.iter().any(|m| m.starts_with("G0.2 Simd8<u8>") && (m.contains("`splat`") || m.contains("`bitor`"))), "{v:#?}");
    // The generic base impl does not stand in for Simd8<i8>.
    caught(&v, "G0.2 Simd8<i8>", "`splat`");
}

#[test]
fn g0_3_reports_intrinsics_outside_the_layer() {
    for src in [
        "use core::arch::aarch64::*;",
        "use std::arch::aarch64::vqtbl1q_u8;",
        "fn f() { core::arch::aarch64::vdupq_n_u8(0); }",
        "use core::{arch, mem};",
        "use core::{mem, arch::aarch64};",
    ] {
        caught(&check_tree(&one("generic/x.rs", src), &[]), "G0.3", "arch");
        assert!(check_tree(&one("arm64/x.rs", src), &[]).iter().all(|m| !m.starts_with("G0.3")), "{src}");
    }
}

#[test]
fn g0_4_reports_loops_branches_and_lane_access_in_the_layer() {
    let cases = [
        ("fn f() { for l in 0..64 {} }", "`for`"),
        ("fn f() { let mut i = 0; while i < 64 { i += 1; } }", "`while`"),
        ("fn f() { loop {} }", "`loop`"),
        ("fn f(x: u8) -> u8 { if x > 3 { 1 } else { 0 } }", "`if`"),
        ("fn f(x: u8) -> u8 { match x { 0 => 1, _ => 0 } }", "`match`"),
        ("fn f(a: &[u8], l: usize) -> u8 { a[l] }", "runtime value"),
        ("fn bits(f: impl Fn(usize) -> bool) {} fn g() { bits(|l| l > 1); }", "closure"),
        ("fn f(a: &[u8]) -> u32 { a.iter().map(|&b| b as u32).sum() }", "`.iter()`"),
        ("fn f() -> [u8; 4] { core::array::from_fn(|i| i as u8) }", "`from_fn`"),
        ("macro_rules! m { () => { for _ in 0..3 {} } }", "macro_rules"),
        ("fn f() { m!(); }", "`m!`"),
    ];
    for (src, what) in cases {
        caught(&check_tree(&one("arm64/simd.rs", &format!("#[inline(always)] {src}")), &[]), "G0.4", what);
    }
    // simdjson's own forms pass: constant chunk indexes, the allow-list, `assert!`.
    let ok = "impl<T> Simd8x64<T> { #[inline(always)] fn reduce_or(&self) -> u8 { self.chunks[0] | self.chunks[IDX] } \
              #[inline(always)] fn compress(&self, m: u16) -> u64 { THINTABLE_EPI8[m as usize] } \
              #[inline(always)] fn prev<const N: i32>(&self) -> u8 { const { assert!(N >= 1) }; match N { 1 => 1, _ => 2 } } }";
    let v = check_tree(&one("arm64/simd.rs", ok), &[]);
    assert!(v.iter().all(|m| !m.starts_with("G0.4")), "{v:#?}");
}

#[test]
fn g0_5_reports_byte_and_lane_code_above_the_layer() {
    let cases = [
        ("fn classify(b: &[u8; 64]) {}", "array of bytes"),
        ("fn f(a: &[u64], b: usize) -> u64 { a[b] }", "runtime value"),
        ("fn f() { for b in 0..4 {} }", "`for`"),
        ("fn f(m: &[u64]) -> u64 { m.iter().fold(0, |a, b| a | b) }", "`.iter()`"),
        ("fn f() { lanes!(x); }", "`lanes!`"),
    ];
    for (src, what) in cases {
        caught(&check_tree(&one("generic/stage1/json_scanner.rs", src), &[]), "G0.5", what);
    }
    // simdjson's loops are allowed by function name; the layer and tables are not "above".
    assert!(check_tree(&one("generic/stage1/json_structural_indexer.rs", "fn index() { while true {} }"), &[]).is_empty());
    let table = "const fn t() -> [u8; 4] { let mut i = 0; while i < 4 { i += 1; } [0; 4] }";
    assert!(check_tree(&one("internal/simdprune_tables.rs", table), &[]).is_empty());
}

#[test]
fn g0_6_reports_a_missing_register_count_assertion() {
    caught(&check_tree(&one("arm64/simd.rs", ""), &[]), "G0.6", "NUM_CHUNKS");
    let ok = "const _: () = assert!(Simd8x64::<u8>::NUM_CHUNKS == 4, \"four registers per block\");";
    assert!(check_tree(&one("arm64/simd.rs", ok), &[]).is_empty());
}

#[test]
fn g0_7_reports_a_layer_function_not_inlined() {
    let v = check_tree(&one("arm64/bitmask.rs", "#[inline] pub fn prefix_xor(m: u64) -> u64 { m }"), &[]);
    caught(&v, "G0.7", "prefix_xor");
    assert!(check_tree(&one("arm64/bitmask.rs", "#[inline(always)] pub fn prefix_xor(m: u64) -> u64 { m }"), &[]).is_empty());
}

#[test]
fn g0_8_reports_a_copyable_block() {
    let a = "#[derive(Clone, Debug)] pub struct Simd8x64<T> { chunks: [T; 4] }";
    caught(&check_tree(&one("arm64/bitmask.rs", a), &[]), "G0.8", "derives");
    let b = "pub struct Simd8x64<T> { chunks: [T; 4] } impl<T> Copy for Simd8x64<T> {}";
    caught(&check_tree(&one("arm64/bitmask.rs", b), &[]), "G0.8", "Copy");
}
