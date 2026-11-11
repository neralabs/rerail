//! Byte-exact item spans for the crate split's codemods (tools/cratify).
//!
//! `cratify items <file>...` prints, per top-level item (and per item inside an inline `mod x { }`), one line:
//!   file  start  end  kind  name  [path of inline mods]
//! `start` includes the item's doc comments and attributes; `end` is exclusive. An impl's name is
//! `Type` or `Trait for Type`. Macro invocations at item level are listed as `macro name!`.
mod packify;
use std::fmt::Write as _;
use syn::spanned::Spanned;

fn offsets(src: &str) -> Vec<usize> {
    let mut v = vec![0];
    for (i, b) in src.bytes().enumerate() {
        if b == b'\n' {
            v.push(i + 1);
        }
    }
    v
}

/// proc-macro2 columns count chars; the byte offset of (line, char column).
fn byte(src: &str, lines: &[usize], lc: proc_macro2::LineColumn) -> usize {
    let start = lines[lc.line - 1];
    src[start..].char_indices().nth(lc.column).map_or(src.len(), |(i, _)| start + i)
}

fn ty(t: &syn::Type) -> String {
    quote::quote!(#t).to_string().replace(' ', "")
}

fn describe(item: &syn::Item) -> (&'static str, String) {
    use syn::Item::*;
    match item {
        Const(i) => ("const", i.ident.to_string()),
        Enum(i) => ("enum", i.ident.to_string()),
        ExternCrate(i) => ("extern_crate", i.ident.to_string()),
        Fn(i) => ("fn", i.sig.ident.to_string()),
        ForeignMod(_) => ("extern", String::new()),
        Impl(i) => (
            "impl",
            match &i.trait_ {
                Some((_, p, _)) => format!("{} for {}", quote::quote!(#p).to_string().replace(' ', ""), ty(&i.self_ty)),
                None => ty(&i.self_ty),
            },
        ),
        Macro(i) => ("macro", format!("{}!{}", i.mac.path.segments.last().map(|s| s.ident.to_string()).unwrap_or_default(), i.ident.as_ref().map(|x| format!(" {x}")).unwrap_or_default())),
        Mod(i) => ("mod", i.ident.to_string()),
        Static(i) => ("static", i.ident.to_string()),
        Struct(i) => ("struct", i.ident.to_string()),
        Trait(i) => ("trait", i.ident.to_string()),
        TraitAlias(i) => ("trait_alias", i.ident.to_string()),
        Type(i) => ("type", i.ident.to_string()),
        Union(i) => ("union", i.ident.to_string()),
        Use(i) => ("use", quote::quote!(#i).to_string().replace(' ', "")),
        _ => ("other", String::new()),
    }
}

fn attrs(item: &syn::Item) -> &[syn::Attribute] {
    use syn::Item::*;
    match item {
        Const(i) => &i.attrs, Enum(i) => &i.attrs, ExternCrate(i) => &i.attrs, Fn(i) => &i.attrs,
        ForeignMod(i) => &i.attrs, Impl(i) => &i.attrs, Macro(i) => &i.attrs, Mod(i) => &i.attrs,
        Static(i) => &i.attrs, Struct(i) => &i.attrs, Trait(i) => &i.attrs, TraitAlias(i) => &i.attrs,
        Type(i) => &i.attrs, Union(i) => &i.attrs, Use(i) => &i.attrs, _ => &[],
    }
}

fn walk(file: &str, src: &str, lines: &[usize], items: &[syn::Item], path: &str, out: &mut String) {
    for item in items {
        let span = item.span();
        let mut start = byte(src, lines, span.start());
        for a in attrs(item) {
            start = start.min(byte(src, lines, a.span().start()));
        }
        let end = byte(src, lines, span.end());
        let (kind, name) = describe(item);
        let cfg = attrs(item)
            .iter()
            .filter(|a| a.path().is_ident("cfg"))
            .map(|a| quote::quote!(#a).to_string().replace(' ', ""))
            .collect::<Vec<_>>()
            .join("");
        writeln!(out, "{file}\t{start}\t{end}\t{kind}\t{name}\t{path}\t{cfg}").unwrap();
        if let syn::Item::Mod(m) = item
            && let Some((_, inner)) = &m.content
        {
            let p = if path.is_empty() { m.ident.to_string() } else { format!("{path}::{}", m.ident) };
            walk(file, src, lines, inner, &p, out);
        }
    }
}


/// One module of the tree: its path, its file (an inline module has its parent's), how its parent declares it.
struct Module {
    path: String,
    file: std::path::PathBuf,
    /// The declaration: in which file, its byte span (attributes included), visibility, cfg and path attributes.
    decl_file: std::path::PathBuf,
    decl: (usize, usize),
    vis: String,
    cfg: String,
    path_attr: Option<String>,
    inline: bool,
}

fn attr_str(attrs: &[syn::Attribute], name: &str) -> Option<String> {
    attrs.iter().find(|a| a.path().is_ident(name)).and_then(|a| match &a.meta {
        syn::Meta::NameValue(nv) => match &nv.value {
            syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(s), .. }) => Some(s.value()),
            _ => None,
        },
        _ => None,
    })
}

fn cfgs(attrs: &[syn::Attribute]) -> String {
    attrs
        .iter()
        .filter(|a| a.path().is_ident("cfg") || a.path().is_ident("cfg_attr"))
        .map(|a| quote::quote!(#a).to_string().replace(' ', ""))
        .collect::<Vec<_>>()
        .join("")
}

/// The modules declared in `items`, which sit in `file` at module `path`. `dir` is where a child file without
/// `#[path]` is looked for; `path_dir` is what a `#[path]` is relative to.
fn modules(file: &std::path::Path, src: &str, lines: &[usize], items: &[syn::Item], path: &str, dir: &std::path::Path, path_dir: &std::path::Path, out: &mut Vec<Module>) {
    for item in items {
        let syn::Item::Mod(m) = item else { continue };
        let name = m.ident.to_string().trim_start_matches("r#").to_owned();
        let child = format!("{path}::{name}");
        let span = item.span();
        let mut start = byte(src, lines, span.start());
        for a in &m.attrs {
            start = start.min(byte(src, lines, a.span().start()));
        }
        let end = byte(src, lines, span.end());
        let vis = match &m.vis {
            syn::Visibility::Inherited => String::new(),
            v => quote::quote!(#v).to_string().replace(' ', "").replace("pub(in", "pub(in "),
        };
        let path_attr = attr_str(&m.attrs, "path");
        let cfg = cfgs(&m.attrs);
        if let Some((_, inner)) = &m.content {
            // An inline module: its children are in a directory named after it (or its `#[path]`).
            let sub = match &path_attr { Some(p) => path_dir.join(p), None => dir.join(&name) };
            out.push(Module { path: child.clone(), file: file.to_owned(), decl_file: file.to_owned(), decl: (start, end), vis, cfg, path_attr, inline: true });
            modules(file, src, lines, inner, &child, &sub, &sub, out);
            continue;
        }
        let (target, mod_rs) = match &path_attr {
            // A file loaded by `#[path]` is read as a mod-rs file: its children sit beside it.
            Some(p) => (path_dir.join(p), true),
            None => {
                let a = dir.join(format!("{name}.rs"));
                let b = dir.join(&name).join("mod.rs");
                if a.exists() { (a, false) } else { (b, true) }
            }
        };
        out.push(Module { path: child.clone(), file: target.clone(), decl_file: file.to_owned(), decl: (start, end), vis, cfg, path_attr, inline: false });
        load(&target, &child, mod_rs, out);
    }
}

fn load(file: &std::path::Path, path: &str, mod_rs: bool, out: &mut Vec<Module>) {
    let Ok(src) = std::fs::read_to_string(file) else {
        eprintln!("missing {}", file.display());
        return;
    };
    let parsed = match syn::parse_file(&src) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("{}: {e}", file.display());
            return;
        }
    };
    let here = file.parent().unwrap().to_owned();
    let stem = file.file_stem().unwrap().to_string_lossy().into_owned();
    // A mod-rs file's children are beside it; another file's are in the directory named after it. `#[path]`s are
    // relative to the file's own directory either way.
    let dir = if mod_rs { here.clone() } else { here.join(&stem) };
    modules(file, &src, &offsets(&src), &parsed.items, path, &dir, &here, out);
}

/// `test` atoms of cfg predicates: the attributes `#[cfg(..)]` and `#[cfg_attr(<pred>, ..)]` (its predicate only), and
/// `cfg!(..)`. Each: (start, end) of the ident, the attribute's (or macro's) span, and the mod it gates, if any.
struct CfgTests<'a> {
    src: &'a str,
    lines: Vec<usize>,
    mods: Vec<(usize, String)>,
    out: Vec<(usize, usize, usize, usize)>,
}

impl CfgTests<'_> {
    fn span(&self, s: proc_macro2::Span) -> (usize, usize) {
        (byte(self.src, &self.lines, s.start()), byte(self.src, &self.lines, s.end()))
    }

    fn scan(&mut self, tokens: proc_macro2::TokenStream, whole: (usize, usize), predicate_only: bool) {
        let mut prev_eq = false;
        let mut iter = tokens.into_iter().peekable();
        while let Some(t) = iter.next() {
            match &t {
                proc_macro2::TokenTree::Punct(p) if p.as_char() == ',' && predicate_only => return,
                proc_macro2::TokenTree::Ident(i) if i == "test" && !prev_eq => {
                    let next_is_eq = matches!(iter.peek(), Some(proc_macro2::TokenTree::Punct(p)) if p.as_char() == '=');
                    if !next_is_eq {
                        let (s, e) = self.span(i.span());
                        self.out.push((s, e, whole.0, whole.1));
                    }
                }
                proc_macro2::TokenTree::Group(g) => self.scan(g.stream(), whole, false),
                _ => {}
            }
            prev_eq = matches!(&t, proc_macro2::TokenTree::Punct(p) if p.as_char() == '=');
        }
    }
}

impl<'ast> syn::visit::Visit<'ast> for CfgTests<'_> {
    fn visit_attribute(&mut self, a: &'ast syn::Attribute) {
        let whole = self.span(a.span());
        if let syn::Meta::List(list) = &a.meta {
            if list.path.is_ident("cfg") {
                self.scan(list.tokens.clone(), whole, false);
            } else if list.path.is_ident("cfg_attr") {
                self.scan(list.tokens.clone(), whole, true);
            }
        }
    }

    fn visit_macro(&mut self, m: &'ast syn::Macro) {
        if m.path.is_ident("cfg") {
            let whole = self.span(m.span());
            self.scan(m.tokens.clone(), whole, false);
        }
        syn::visit::visit_macro(self, m);
    }

    fn visit_item_mod(&mut self, m: &'ast syn::ItemMod) {
        for a in &m.attrs {
            let (s, _) = self.span(a.span());
            self.mods.push((s, m.ident.to_string()));
        }
        syn::visit::visit_item_mod(self, m);
    }
}

/// `cratify traits <file>...`: per trait in the files (top level), one JSON line: its name, supertraits, and items
/// (consts with type and whether defaulted, associated types with bounds, fns with generics, receiver, inputs as
/// (pattern, type), output, whether defaulted). Types are token strings, as written.
fn traits_json(src: &str) -> Vec<String> {
    fn q<T: quote::ToTokens>(t: &T) -> String {
        t.to_token_stream().to_string()
    }
    fn esc(s: &str) -> String {
        let mut o = String::from("\"");
        for c in s.chars() {
            match c {
                '"' => o.push_str("\\\""),
                '\\' => o.push_str("\\\\"),
                '\n' => o.push_str("\\n"),
                c => o.push(c),
            }
        }
        o.push('"');
        o
    }
    let f = syn::parse_file(src).expect("parses");
    let mut out = Vec::new();
    for item in &f.items {
        let syn::Item::Trait(t) = item else { continue };
        let mut items = Vec::new();
        for ti in &t.items {
            match ti {
                syn::TraitItem::Const(c) => items.push(format!(
                    "{{\"kind\":\"const\",\"name\":{},\"ty\":{},\"default\":{}}}",
                    esc(&c.ident.to_string()), esc(&q(&c.ty)), c.default.as_ref().map_or("null".to_owned(), |(_, e)| esc(&q(e)))
                )),
                syn::TraitItem::Type(ty) => items.push(format!(
                    "{{\"kind\":\"type\",\"name\":{},\"bounds\":{}}}",
                    esc(&ty.ident.to_string()), esc(&q(&ty.bounds))
                )),
                syn::TraitItem::Fn(m) => {
                    let sig = &m.sig;
                    let cfg: Vec<String> =
                        m.attrs.iter().filter(|a| a.path().is_ident("cfg")).map(|a| esc(&q(a))).collect();
                    let mut inputs = Vec::new();
                    let mut receiver = "null".to_owned();
                    for a in &sig.inputs {
                        match a {
                            syn::FnArg::Receiver(r) => receiver = esc(&q(r)),
                            syn::FnArg::Typed(p) => inputs.push(format!("[{},{}]", esc(&q(&p.pat)), esc(&q(&p.ty)))),
                        }
                    }
                    let output = match &sig.output {
                        syn::ReturnType::Default => "null".to_owned(),
                        syn::ReturnType::Type(_, t) => esc(&q(t)),
                    };
                    let generics: Vec<String> = sig.generics.params.iter().map(|g| esc(&q(g))).collect();
                    items.push(format!(
                        "{{\"kind\":\"fn\",\"name\":{},\"generics\":[{}],\"where\":{},\"receiver\":{},\"inputs\":[{}],\"output\":{},\"default\":{},\"cfg\":[{}]}}",
                        esc(&sig.ident.to_string()), generics.join(","),
                        sig.generics.where_clause.as_ref().map_or("null".to_owned(), |w| esc(&q(w))),
                        receiver, inputs.join(","), output, m.default.is_some(), cfg.join(",")
                    ));
                }
                _ => {}
            }
        }
        out.push(format!(
            "{{\"trait\":{},\"supertraits\":{},\"items\":[{}]}}",
            esc(&t.ident.to_string()), esc(&q(&t.supertraits)), items.join(",")
        ));
    }
    out
}

/// `cratify uses <file>`: each name the file's top-level `use` items bring in, and its path as written: `name\tpath`
/// (a glob is `*\tpath`).
fn use_leaves(tree: &syn::UseTree, prefix: &str, out: &mut Vec<(String, String)>) {
    let join = |p: &str, s: &str| if p.is_empty() { s.to_owned() } else { format!("{p}::{s}") };
    match tree {
        syn::UseTree::Path(p) => use_leaves(&p.tree, &join(prefix, &p.ident.to_string()), out),
        syn::UseTree::Name(n) => {
            let name = n.ident.to_string();
            if name == "self" {
                out.push((prefix.rsplit("::").next().unwrap_or(prefix).to_owned(), prefix.to_owned()));
            } else {
                out.push((name.clone(), join(prefix, &name)));
            }
        }
        syn::UseTree::Rename(r) => out.push((r.rename.to_string(), join(prefix, &r.ident.to_string()))),
        syn::UseTree::Glob(_) => out.push(("*".to_owned(), prefix.to_owned())),
        syn::UseTree::Group(g) => g.items.iter().for_each(|t| use_leaves(t, prefix, out)),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("items") => {
            let mut out = String::new();
            let mut bad = 0;
            for file in &args[2..] {
                let src = std::fs::read_to_string(file).expect(file);
                match syn::parse_file(&src) {
                    Ok(f) => walk(file, &src, &offsets(&src), &f.items, "", &mut out),
                    Err(e) => {
                        eprintln!("{file}: {e}");
                        bad += 1;
                    }
                }
            }
            print!("{out}");
            if bad > 0 {
                std::process::exit(1);
            }
        }
        Some("tree") => {
            // `cratify tree <lib.rs>`: every module from the crate root, one line each.
            let root = std::path::PathBuf::from(&args[2]);
            let mut out = Vec::new();
            load(&root, "crate", true, &mut out);
            for m in out {
                println!(
                    "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                    m.path,
                    m.file.display(),
                    m.decl_file.display(),
                    m.decl.0,
                    m.decl.1,
                    m.vis,
                    m.cfg,
                    m.path_attr.unwrap_or_default(),
                    if m.inline { "inline" } else { "file" }
                );
            }
        }
        Some("packify") => packify::main(&args[2..]),
        Some("uses") => {
            let src = std::fs::read_to_string(&args[2]).expect("file");
            let f = syn::parse_file(&src).expect("parses");
            for item in &f.items {
                if let syn::Item::Use(u) = item {
                    let mut out = Vec::new();
                    use_leaves(&u.tree, if u.leading_colon.is_some() { "::" } else { "" }, &mut out);
                    for (n, p) in out {
                        println!("{n}\t{p}");
                    }
                }
            }
        }
        Some("traits") => {
            for file in &args[2..] {
                let src = std::fs::read_to_string(file).expect(file);
                for line in traits_json(&src) {
                    println!("{line}");
                }
            }
        }
        Some("cfgtest") => {
            // `cratify cfgtest <file>...`: per `test` atom of a cfg predicate: file, start, end, the attribute's start
            // and end, and `mod <name>` when the attribute gates a module (else `-`).
            for file in &args[2..] {
                let src = std::fs::read_to_string(file).expect(file);
                let f = syn::parse_file(&src).unwrap_or_else(|e| panic!("{file}: {e}"));
                let mut v = CfgTests { src: &src, lines: offsets(&src), mods: Vec::new(), out: Vec::new() };
                syn::visit::Visit::visit_file(&mut v, &f);
                for (s, e, a, b) in &v.out {
                    let gated = v.mods.iter().find(|(at, _)| at == a).map_or("-".to_owned(), |(_, n)| format!("mod {n}"));
                    println!("{file}\t{s}\t{e}\t{a}\t{b}\t{gated}");
                }
            }
        }
        _ => {
            eprintln!("usage: cratify items <file>... | cratify tree <lib.rs> | cratify cfgtest <file>...");
            std::process::exit(2);
        }
    }
}
