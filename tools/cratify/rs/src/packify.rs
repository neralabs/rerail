//! `cratify packify <pack-methods.tsv> <file>... [--write]`: the core's static adapter dispatch, rewritten to the
//! registered language packs.
//!
//! - `with_adapter!(family, A => body)` becomes `{ let pack = language_pack::pack(family); body }`;
//! - a function generic over an adapter trait (`fn f<S: ScopeRules, D: Doc>(…)`) loses the parameter and takes
//!   `pack: &dyn language_pack::LanguagePack` first;
//! - in either, `A::LABEL`, `<<A as Adapter>::Scope as ScopeRules>::SUPPORTED` become `pack.label()`,
//!   `pack.scope_supported()`; a method call `pack.m(…)`, or `language_pack::m(pack, …)` where the table says
//!   the method is a free function (generic over the document, or taking `impl` arguments);
//! - a call `f::<A, D>(…)` to such a function drops the adapter arguments and passes `pack` first.
//!
//! Prints each edit, and each use of an adapter type it leaves (UNHANDLED); `--write` applies the edits.
use std::collections::HashMap;

use proc_macro2::Span;
use syn::spanned::Spanned;
use syn::visit::Visit;

const TRAITS: &[&str] =
    &["Adapter", "SyntaxRules", "ScopeRules", "ValueRules", "ArgumentRules", "CallRules", "FormatArgRules"];
const MODULE: &str = "language_pack";

struct Table(HashMap<(String, String), (String, String)>);

impl Table {
    fn load(path: &str) -> Self {
        let text = std::fs::read_to_string(path).expect("table");
        let mut map = HashMap::new();
        for line in text.lines() {
            let f: Vec<&str> = line.split('\t').collect();
            map.insert((f[0].to_owned(), f[1].to_owned()), (f[2].to_owned(), f[3].to_owned()));
        }
        Table(map)
    }
    /// The (kind, method) of `item` in `tr`; an Adapter-bound name also answers SyntaxRules' items (every pack
    /// is both).
    fn get(&self, tr: &str, item: &str) -> Option<(String, String)> {
        let mut found = self.0.get(&(tr.to_owned(), item.to_owned())).cloned();
        if found.is_none() && tr == "Adapter" {
            found = self.0.get(&("SyntaxRules".to_owned(), item.to_owned())).cloned();
        }
        found
    }
}

struct Edit {
    start: usize,
    end: usize,
    text: String,
    what: &'static str,
}

struct Packify<'a> {
    src: &'a str,
    lines: Vec<usize>,
    table: &'a Table,
    /// Adapter-typed names in scope: name -> the trait it is bound by.
    env: Vec<HashMap<String, String>>,
    edits: Vec<Edit>,
    unhandled: Vec<String>,
    /// Ranges edits already account for (an ident inside one is not a leftover).
    covered: Vec<(usize, usize)>,
    /// The macro being visited is a `let … else` initializer: its block needs parentheses.
    let_else: Option<Span>,
}

impl Packify<'_> {
    fn pos(&self, s: Span) -> (usize, usize) {
        (crate::byte(self.src, &self.lines, s.start()), crate::byte(self.src, &self.lines, s.end()))
    }
    fn text(&self, s: Span) -> &str {
        let (a, b) = self.pos(s);
        &self.src[a..b]
    }
    fn edit(&mut self, start: usize, end: usize, text: String, what: &'static str) {
        self.covered.push((start, end));
        self.edits.push(Edit { start, end, text, what });
    }
    fn bound(&self, name: &str) -> Option<String> {
        self.env.iter().rev().find_map(|e| e.get(name).cloned())
    }
    fn line(&self, s: Span) -> usize {
        s.start().line
    }
    fn unhandled(&mut self, s: Span, why: &str) {
        let line = self.line(s);
        let text: String = self.text(s).split_whitespace().collect::<Vec<_>>().join(" ");
        self.unhandled.push(format!("{line}\t{why}\t{}", &text[..text.len().min(160)]));
        let (a, b) = self.pos(s);
        self.covered.push((a, b));
    }

    /// The adapter name a type denotes and the associated rules it names: `A` -> (A, None), `<A as
    /// Adapter>::Scope` or `A::Scope` -> (A, Some(Scope)).
    fn adapter_type(&self, ty: &syn::Type) -> Option<(String, Option<String>)> {
        let syn::Type::Path(tp) = ty else { return None };
        let segs: Vec<String> = tp.path.segments.iter().map(|s| s.ident.to_string()).collect();
        match &tp.qself {
            None => match segs.as_slice() {
                [a] if self.bound(a).is_some() => Some((a.clone(), None)),
                [a, assoc] if self.bound(a).is_some() => Some((a.clone(), Some(assoc.clone()))),
                _ => None,
            },
            Some(q) => {
                let (a, None) = self.adapter_type(&q.ty)? else { return None };
                // `<A as Adapter>::Scope`: the segments are the trait path then the associated type.
                (segs.len() == q.position + 1).then(|| (a, Some(segs[q.position].clone())))
            }
        }
    }

    /// (trait, item) an expression path names, when it is an adapter item.
    fn classify(&self, qself: &Option<syn::QSelf>, path: &syn::Path) -> Option<(String, String)> {
        let segs: Vec<String> = path.segments.iter().map(|s| s.ident.to_string()).collect();
        match qself {
            None => match segs.as_slice() {
                [a, item] => self.bound(a).map(|tr| (tr, item.clone())),
                _ => None,
            },
            Some(q) => {
                let (a, assoc) = self.adapter_type(&q.ty)?;
                let tr = segs.get(q.position.checked_sub(1)?)?.clone();
                let item = segs.get(q.position)?.clone();
                if segs.len() != q.position + 1 || !TRAITS.contains(&tr.as_str()) {
                    return None;
                }
                let base = self.bound(&a)?;
                match assoc {
                    None => Some((if base == "Adapter" { tr } else { base }, item)),
                    Some(_) => Some((tr, item)),
                }
            }
        }
    }

    fn mentions_adapter(&self, ty: &syn::Type) -> bool {
        self.adapter_type(ty).is_some()
    }

    /// A turbofish naming adapter types: the edit leaving the other arguments, or none.
    fn turbofish(&mut self, args: &syn::AngleBracketedGenericArguments) -> bool {
        let keep: Vec<String> = args
            .args
            .iter()
            .filter(|a| !matches!(a, syn::GenericArgument::Type(t) if self.mentions_adapter(t)))
            .map(|a| self.text(a.span()).to_owned())
            .collect();
        if keep.len() == args.args.len() {
            return false;
        }
        let (start, _) = self.pos(args.colon2_token.map_or(args.lt_token.span, |c| c.spans[0]));
        let (_, end) = self.pos(args.gt_token.span);
        let text = if keep.is_empty() { String::new() } else { format!("::<{}>", keep.join(", ")) };
        self.edit(start, end, text, "turbofish");
        true
    }

    /// `pack` as the first argument of the call whose parentheses are `paren`.
    fn pass_pack(&mut self, paren: &syn::token::Paren, empty: bool) {
        let (_, at) = self.pos(paren.span.open());
        self.edit(at, at, if empty { "pack".to_owned() } else { "pack, ".to_owned() }, "pass pack");
    }

    /// A function's adapter parameters, removed; `pack` added first (after the receiver).
    fn signature(&mut self, sig: &syn::Signature) -> HashMap<String, String> {
        let mut env = HashMap::new();
        let params: Vec<&syn::GenericParam> = sig.generics.params.iter().collect();
        let mut drop = Vec::new();
        for (i, p) in params.iter().enumerate() {
            let syn::GenericParam::Type(tp) = p else { continue };
            let tr = tp.bounds.iter().find_map(|b| match b {
                syn::TypeParamBound::Trait(t) => {
                    let n = t.path.segments.last()?.ident.to_string();
                    TRAITS.contains(&n.as_str()).then_some(n)
                }
                _ => None,
            });
            if let Some(tr) = tr {
                env.insert(tp.ident.to_string(), tr);
                drop.push(i);
            }
        }
        if env.is_empty() {
            return env;
        }
        if let Some(w) = &sig.generics.where_clause {
            let text = self.text(w.span()).to_owned();
            if env.keys().any(|k| text.split(|c: char| !c.is_alphanumeric() && c != '_').any(|t| t == k)) {
                self.unhandled(w.span(), "where clause names an adapter parameter");
            }
        }
        if drop.len() == params.len() {
            let (s, _) = self.pos(sig.generics.lt_token.unwrap().span);
            let (_, e) = self.pos(sig.generics.gt_token.unwrap().span);
            self.edit(s, e, String::new(), "drop generics");
        } else {
            for &i in &drop {
                let (s, e) = self.pos(params[i].span());
                if i + 1 < params.len() && !drop.contains(&(i + 1)) {
                    let (next, _) = self.pos(params[i + 1].span());
                    self.edit(s, next, String::new(), "drop generic");
                } else if i + 1 < params.len() {
                    let (next, _) = self.pos(params[i + 1].span());
                    self.edit(s, next, String::new(), "drop generic");
                } else {
                    let (_, prev) = self.pos(params[i - 1].span());
                    self.edit(prev, e, String::new(), "drop generic");
                }
            }
        }
        let ty = format!("pack: &dyn {MODULE}::LanguagePack");
        let inputs: Vec<&syn::FnArg> = sig.inputs.iter().collect();
        match inputs.as_slice() {
            [] => {
                let (_, at) = self.pos(sig.paren_token.span.open());
                self.edit(at, at, ty, "take pack");
            }
            [syn::FnArg::Receiver(r)] => {
                let (_, at) = self.pos(r.span());
                self.edit(at, at, format!(", {ty}"), "take pack");
            }
            [syn::FnArg::Receiver(_), second, ..] => {
                let (at, _) = self.pos(second.span());
                self.edit(at, at, format!("{ty}, "), "take pack");
            }
            [first, ..] => {
                let (at, _) = self.pos(first.span());
                self.edit(at, at, format!("{ty}, "), "take pack");
            }
        }
        env
    }

    fn function(&mut self, sig: &syn::Signature, body: &syn::Block) {
        let env = self.signature(sig);
        let pushed = !env.is_empty();
        if pushed {
            self.env.push(env);
        }
        self.visit_block(body);
        if pushed {
            self.env.pop();
        }
    }

    /// `with_adapter!(family, A => body)`.
    fn with_adapter(&mut self, mac: &syn::Macro) -> bool {
        if mac.path.segments.last().is_none_or(|s| s.ident != "with_adapter") {
            return false;
        }
        struct Args {
            family: syn::Expr,
            name: syn::Ident,
            body: syn::Expr,
        }
        impl syn::parse::Parse for Args {
            fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
                let family = input.parse()?;
                input.parse::<syn::Token![,]>()?;
                let name = input.parse()?;
                input.parse::<syn::Token![=>]>()?;
                let body = input.parse()?;
                Ok(Args { family, name, body })
            }
        }
        let Ok(args) = mac.parse_body::<Args>() else {
            self.unhandled(mac.span(), "with_adapter! arguments do not parse");
            return true;
        };
        let (start, _) = self.pos(mac.path.span());
        let (_, end) = self.pos(mac.delimiter.span().close());
        let (body_start, body_end) = self.pos(args.body.span());
        let family = self.text(args.family.span()).to_owned();
        let paren = self.let_else.is_some_and(|s| self.pos(s).0 == start);
        let (open, close) = if paren { ("({", "})") } else { ("{", "}") };
        self.edit(start, body_start, format!("{open} let pack = {MODULE}::pack({family}); "), "with_adapter");
        self.edit(body_end, end, format!(" {close}"), "with_adapter");
        self.env.push(HashMap::from([(args.name.to_string(), "Adapter".to_owned())]));
        self.visit_expr(&args.body);
        self.env.pop();
        true
    }

    /// A call of an adapter item: rewritten, and true.
    fn adapter_call(&mut self, func: &syn::Expr, paren: &syn::token::Paren, empty: bool) -> bool {
        let syn::Expr::Path(p) = func else { return false };
        let Some((tr, item)) = self.classify(&p.qself, &p.path) else { return false };
        let Some((kind, method)) = self.table.get(&tr, &item) else {
            self.unhandled(p.span(), &format!("no pack method for {tr}::{item}"));
            return true;
        };
        let (s, e) = self.pos(p.span());
        let fish = match &p.path.segments.last().unwrap().arguments {
            syn::PathArguments::AngleBracketed(a) => self.text(a.span()).to_owned(),
            _ => String::new(),
        };
        match kind.as_str() {
            "fn" => self.edit(s, e, format!("pack.{method}{fish}"), "method"),
            "free" => {
                self.edit(s, e, format!("{MODULE}::{method}{fish}"), "free function");
                self.pass_pack(paren, empty);
            }
            _ => self.unhandled(p.span(), "a constant called"),
        }
        true
    }
}

impl<'ast> Visit<'ast> for Packify<'_> {
    fn visit_item_fn(&mut self, f: &'ast syn::ItemFn) {
        self.function(&f.sig, &f.block);
    }
    fn visit_impl_item_fn(&mut self, f: &'ast syn::ImplItemFn) {
        self.function(&f.sig, &f.block);
    }
    fn visit_expr_macro(&mut self, m: &'ast syn::ExprMacro) {
        if !self.with_adapter(&m.mac) {
            self.visit_macro(&m.mac);
        }
    }
    fn visit_stmt_macro(&mut self, m: &'ast syn::StmtMacro) {
        if !self.with_adapter(&m.mac) {
            self.visit_macro(&m.mac);
        }
    }
    fn visit_macro(&mut self, m: &'ast syn::Macro) {
        // Another macro's tokens: read as comma-separated expressions where they parse (format!, assert!, vec!).
        if self.env.is_empty() {
            return;
        }
        let parsed = m.parse_body_with(syn::punctuated::Punctuated::<syn::Expr, syn::Token![,]>::parse_terminated);
        match parsed {
            Ok(exprs) => exprs.iter().for_each(|e| self.visit_expr(e)),
            Err(_) => {
                let names: Vec<String> = self.env.iter().flat_map(|e| e.keys().cloned()).collect();
                if m.tokens.clone().into_iter().any(|t| names.iter().any(|n| t.to_string().contains(n.as_str()))) {
                    self.unhandled(m.span(), "a macro's tokens name an adapter");
                }
            }
        }
    }
    fn visit_expr_call(&mut self, c: &'ast syn::ExprCall) {
        if !self.env.is_empty() {
            if self.adapter_call(&c.func, &c.paren_token, c.args.is_empty()) {
                c.args.iter().for_each(|a| self.visit_expr(a));
                return;
            }
            if let syn::Expr::Path(p) = &*c.func
                && p.qself.is_none()
                && let syn::PathArguments::AngleBracketed(a) = &p.path.segments.last().unwrap().arguments
                && self.turbofish(a)
            {
                self.pass_pack(&c.paren_token, c.args.is_empty());
                c.args.iter().for_each(|a| self.visit_expr(a));
                return;
            }
        }
        syn::visit::visit_expr_call(self, c);
    }
    fn visit_expr_method_call(&mut self, c: &'ast syn::ExprMethodCall) {
        if !self.env.is_empty()
            && let Some(a) = &c.turbofish
            && self.turbofish(a)
        {
            self.pass_pack(&c.paren_token, c.args.is_empty());
            self.visit_expr(&c.receiver);
            c.args.iter().for_each(|a| self.visit_expr(a));
            return;
        }
        syn::visit::visit_expr_method_call(self, c);
    }
    fn visit_expr_path(&mut self, p: &'ast syn::ExprPath) {
        if !self.env.is_empty()
            && let Some((tr, item)) = self.classify(&p.qself, &p.path)
        {
            match self.table.get(&tr, &item) {
                Some((kind, method)) if kind == "const" => {
                    let (s, e) = self.pos(p.span());
                    self.edit(s, e, format!("pack.{method}()"), "constant");
                }
                Some(_) => self.unhandled(p.span(), "an adapter function as a value"),
                None => self.unhandled(p.span(), &format!("no pack method for {tr}::{item}")),
            }
            return;
        }
        syn::visit::visit_expr_path(self, p);
    }
    fn visit_local(&mut self, l: &'ast syn::Local) {
        if let Some(init) = &l.init
            && init.diverge.is_some()
        {
            self.let_else = Some(init.expr.span());
        }
        syn::visit::visit_local(self, l);
    }
    fn visit_item_type(&mut self, t: &'ast syn::ItemType) {
        // `type S = <A as Adapter>::Scope;` in a body: S is the rules, by name; the alias goes.
        if let Some((_, Some(assoc))) = self.adapter_type(&t.ty) {
            let tr = match assoc.as_str() {
                "Scope" => "ScopeRules",
                "Value" => "ValueRules",
                "Arguments" => "ArgumentRules",
                "Calls" => "CallRules",
                "FormatArgs" => "FormatArgRules",
                _ => {
                    self.unhandled(t.span(), "an alias of an unknown associated type");
                    return;
                }
            };
            let (s, e) = self.pos(t.span());
            self.edit(s, e, String::new(), "drop alias");
            self.env.last_mut().unwrap().insert(t.ident.to_string(), tr.to_owned());
            return;
        }
        syn::visit::visit_item_type(self, t);
    }
    fn visit_ident(&mut self, i: &'ast proc_macro2::Ident) {
        if self.bound(&i.to_string()).is_some() {
            let (a, b) = self.pos(i.span());
            if !self.covered.iter().any(|&(s, e)| s <= a && b <= e) {
                self.unhandled(i.span(), "an adapter type left");
            }
        }
    }
}

pub(crate) fn main(args: &[String]) {
    let write = args.iter().any(|a| a == "--write");
    let table = Table::load(&args[0]);
    let (mut sites, mut edits_n, mut left) = (0, 0, 0);
    for file in args[1..].iter().filter(|a| *a != "--write") {
        let src = std::fs::read_to_string(file).expect(file);
        let Ok(ast) = syn::parse_file(&src) else {
            println!("{file}\tUNPARSED");
            continue;
        };
        let mut p = Packify {
            src: &src,
            lines: crate::offsets(&src),
            table: &table,
            env: Vec::new(),
            edits: Vec::new(),
            unhandled: Vec::new(),
            covered: Vec::new(),
            let_else: None,
        };
        p.visit_file(&ast);
        if p.edits.is_empty() && p.unhandled.is_empty() {
            continue;
        }
        sites += p.edits.iter().filter(|e| e.what == "with_adapter").count() / 2;
        edits_n += p.edits.len();
        left += p.unhandled.len();
        for u in &p.unhandled {
            println!("{file}:{u}\tUNHANDLED");
        }
        let mut edits = std::mem::take(&mut p.edits);
        edits.sort_by_key(|e| (e.start, e.end));
        for w in edits.windows(2) {
            if w[1].start < w[0].end {
                panic!("{file}: overlapping edits at {}..{} ({}) and {}..{} ({})", w[0].start, w[0].end, w[0].what, w[1].start, w[1].end, w[1].what);
            }
        }
        if !write || edits.is_empty() {
            continue;
        }
        let mut out = src.clone();
        for e in edits.iter().rev() {
            out.replace_range(e.start..e.end, &e.text);
        }
        // The module, imported after the file's last top-level `use`.
        let import = format!("use crate::code_rule_dsl::language::adapter::pack as {MODULE};\n");
        if !out.contains(&import) {
            let at = ast
                .items
                .iter()
                .filter(|i| matches!(i, syn::Item::Use(_)))
                .last()
                .map(|u| crate::byte(&src, &crate::offsets(&src), u.span().end()))
                .map(|end| {
                    // the same position in `out`: edits are all after the imports
                    let nl = src[end..].find('\n').map_or(src.len(), |n| end + n + 1);
                    let shift: isize = edits.iter().filter(|e| e.end <= nl).map(|e| e.text.len() as isize - (e.end - e.start) as isize).sum();
                    (nl as isize + shift) as usize
                })
                .unwrap_or(0);
            out.insert_str(at, &import);
        }
        std::fs::write(file, out).expect("write");
    }
    println!("with_adapter! sites {sites}, edits {edits_n}, unhandled {left}");
}
