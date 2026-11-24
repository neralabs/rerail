//! `#[derive(Archive)]` (decision 0628 §4): for a struct or an enum, its archived view, a `repr(C)` record (a
//! `repr(u32)` tagged record for an enum) whose fields are its fields' archived views, and the encoder that writes
//! it into a section (`rerail_section::Writer`): each field's out-of-line data first, then the record.
//!
//! `#[archive(derive(Debug, PartialEq, …))]` on the type derives those traits for its archived view.
//! `#[archive(flat)]` on a field archives it as `rerail_section::Flat`: a `Plain` value as itself, a list of them as
//! a `RelList` of themselves.

use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as Tokens};
use quote::{format_ident, quote};
use syn::{Data, DeriveInput, Fields, Ident, Index, parse_macro_input};

#[proc_macro_derive(Archive, attributes(archive))]
pub fn derive_archive(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match expand(&input) {
        Ok(t) => t.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

/// The traits `#[archive(derive(…))]` names, and whether `#[archive(decode)]` asks for a `Decode`.
fn derives(input: &DeriveInput) -> syn::Result<(Vec<syn::Path>, bool)> {
    let mut out = Vec::new();
    let mut decode = false;
    for attr in &input.attrs {
        if !attr.path().is_ident("archive") {
            continue;
        }
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("derive") {
                meta.parse_nested_meta(|d| {
                    out.push(d.path);
                    Ok(())
                })
            } else if meta.path.is_ident("decode") {
                decode = true;
                Ok(())
            } else {
                Err(meta.error("expected `derive(…)` or `decode`"))
            }
        })?;
    }
    Ok((out, decode))
}

/// `T { a: decode(__v0), … }`, `T(decode(__v0), …)` or `T`, from the value bindings.
fn rebuild(path: Tokens, kind: &Fields, fs: &[Field]) -> Tokens {
    let items = fs.iter().map(|f| {
        let v = &f.value;
        let d = if f.flat { quote!(::rerail_section::FlatDecode::decode(#v)) } else { quote!(::rerail_section::Decode::decode(#v, __n)) };
        match &f.member {
            syn::Member::Named(n) => quote!(#n: #d),
            syn::Member::Unnamed(_) => d,
        }
    });
    match kind {
        Fields::Named(_) => quote!(#path { #(#items,)* }),
        Fields::Unnamed(_) => quote!(#path ( #(#items,)* )),
        Fields::Unit => path,
    }
}

/// A field's name in the record (`a`, or `0` for a tuple field) and the names its value and resolver are bound to.
struct Field {
    member: syn::Member,
    vis: syn::Visibility,
    ty: syn::Type,
    value: Ident,
    resolver: Ident,
    /// `#[archive(flat)]`: through `Flat`, not `Archive`.
    flat: bool,
}

impl Field {
    fn tr(&self) -> Tokens {
        if self.flat { quote!(::rerail_section::Flat) } else { quote!(::rerail_section::Archive) }
    }
}

fn flat(attrs: &[syn::Attribute]) -> bool {
    let mut flat = false;
    for a in attrs.iter().filter(|a| a.path().is_ident("archive")) {
        let _ = a.parse_nested_meta(|m| {
            if m.path.is_ident("flat") {
                flat = true;
            }
            Ok(())
        });
    }
    flat
}

fn fields(f: &Fields) -> Vec<Field> {
    f.iter()
        .enumerate()
        .map(|(i, f)| Field {
            flat: flat(&f.attrs),
            member: match &f.ident {
                Some(id) => syn::Member::Named(id.clone()),
                None => syn::Member::Unnamed(Index { index: i as u32, span: Span::call_site() }),
            },
            vis: f.vis.clone(),
            ty: f.ty.clone(),
            value: format_ident!("__v{}", i),
            resolver: format_ident!("__r{}", i),
        })
        .collect()
}

/// A record's field list over `map(field)`: `{ a: X, … }`, `(X, …)` or nothing, `pub` kept where `public`.
fn shape(kind: &Fields, fs: &[Field], public: bool, map: impl Fn(&Field) -> Tokens) -> Tokens {
    let items = fs.iter().map(|f| {
        let ty = map(f);
        let vis = if public { f.vis.clone() } else { syn::Visibility::Inherited };
        match &f.member {
            syn::Member::Named(n) => quote!(#vis #n: #ty),
            syn::Member::Unnamed(_) => quote!(#vis #ty),
        }
    });
    match kind {
        Fields::Named(_) => quote!({ #(#items,)* }),
        Fields::Unnamed(_) => quote!(( #(#items,)* )),
        Fields::Unit => quote!(),
    }
}

/// A pattern binding each field to `bind(field)`: `{ a: __v0, … }`, `(__v0, …)` or nothing.
fn pattern(kind: &Fields, fs: &[Field], bind: impl Fn(&Field) -> &Ident) -> Tokens {
    let items = fs.iter().map(|f| {
        let b = bind(f);
        match &f.member {
            syn::Member::Named(n) => quote!(#n: #b),
            syn::Member::Unnamed(_) => quote!(#b),
        }
    });
    match kind {
        Fields::Named(_) => quote!({ #(#items,)* }),
        Fields::Unnamed(_) => quote!(( #(#items,)* )),
        Fields::Unit => quote!(),
    }
}

fn archived(f: &Field) -> Tokens {
    let (ty, tr) = (&f.ty, f.tr());
    quote!(<#ty as #tr>::Archived)
}

fn resolver(f: &Field) -> Tokens {
    let (ty, tr) = (&f.ty, f.tr());
    quote!(<#ty as #tr>::Resolver)
}

/// Writes each field's record at its place in `*out` (a pointer to a record with the fields), from its value and
/// resolver bindings.
fn resolve_fields(fs: &[Field]) -> Tokens {
    let each = fs.iter().map(|f| {
        let (m, v, r, tr) = (&f.member, &f.value, &f.resolver, f.tr());
        quote! {
            let __p = ::core::ptr::addr_of_mut!((*out).#m);
            #tr::resolve(#v, at + (__p as usize - out as usize), #r, __p);
        }
    });
    quote!(#(#each)*)
}

fn expand(input: &DeriveInput) -> syn::Result<Tokens> {
    if !input.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(&input.generics, "Archive: a generic type is not supported"));
    }
    let name = &input.ident;
    let vis = &input.vis;
    let view = format_ident!("Archived{}", name);
    let res = format_ident!("{}Resolver", name);
    let (derives, decode) = derives(input)?;
    let doc = format!("`{name}`'s archived view (`rerail_section`): read where its section lies.");
    match &input.data {
        Data::Struct(s) => {
            let fs = fields(&s.fields);
            let view_body = shape(&s.fields, &fs, true, archived);
            let res_body = shape(&s.fields, &fs, false, resolver);
            let semi = if matches!(s.fields, Fields::Named(_)) { quote!() } else { quote!(;) };
            let writes = fs.iter().map(|f| {
                let (m, v, tr) = (&f.member, &f.value, f.tr());
                quote!(let #v = #tr::write_out(&self.#m, w);)
            });
            let make = pattern(&s.fields, &fs, |f| &f.value);
            let take_v = pattern(&s.fields, &fs, |f| &f.value);
            let take_r = pattern(&s.fields, &fs, |f| &f.resolver);
            let body = resolve_fields(&fs);
            let decoder = if decode {
                let take = pattern(&s.fields, &fs, |f| &f.value);
                let make = rebuild(quote!(#name), &s.fields, &fs);
                quote! {
                    impl ::rerail_section::Decode for #name {
                        #[allow(unused_variables)]
                        fn decode(a: &#view, __n: &::rerail_section::Names<'_>) -> Self {
                            let #view #take = a;
                            #make
                        }
                    }
                }
            } else {
                quote!()
            };
            Ok(quote! {
                #decoder

                #[doc = #doc]
                #[repr(C)]
                #[derive(#(#derives),*)]
                #vis struct #view #view_body #semi

                #[doc(hidden)]
                #[allow(dead_code)]
                #vis struct #res #res_body #semi

                // SAFETY: the view is `repr(C)` over its fields' views, each written by its own `resolve`.
                unsafe impl ::rerail_section::Archive for #name {
                    type Archived = #view;
                    type Resolver = #res;
                    #[allow(unused_variables)]
                    fn write_out(&self, w: &mut ::rerail_section::Writer) -> #res {
                        #(#writes)*
                        #res #make
                    }
                    #[allow(unused_variables, clippy::unused_unit)]
                    unsafe fn resolve(&self, at: usize, r: #res, out: *mut #view) {
                        let #name #take_v = self;
                        let #res #take_r = r;
                        unsafe { #body }
                    }
                }
            })
        }
        Data::Enum(e) => {
            if e.variants.len() > u32::MAX as usize {
                return Err(syn::Error::new_spanned(name, "Archive: too many variants"));
            }
            let mut view_vs = Vec::new();
            let mut res_vs = Vec::new();
            let mut layouts = Vec::new();
            let mut writes = Vec::new();
            let mut resolves = Vec::new();
            let mut decodes = Vec::new();
            for (tag, v) in e.variants.iter().enumerate() {
                let vn = &v.ident;
                let fs = fields(&v.fields);
                let view_body = shape(&v.fields, &fs, false, archived);
                let res_body = shape(&v.fields, &fs, false, resolver);
                view_vs.push(quote!(#vn #view_body));
                res_vs.push(quote!(#vn #res_body));
                // The variant's record in the `repr(u32)` view: its tag, then its fields, as `repr(C)` lays them.
                let layout = format_ident!("__{}{}", view, vn);
                let lfields = fs.iter().map(|f| {
                    let ty = archived(f);
                    let m = match &f.member {
                        syn::Member::Named(n) => n.clone(),
                        syn::Member::Unnamed(i) => format_ident!("_{}", i.index),
                    };
                    quote!(#m: #ty)
                });
                layouts.push(quote! {
                    #[repr(C)]
                    #[allow(non_camel_case_types, dead_code)]
                    struct #layout { __tag: u32, #(#lfields,)* }
                });
                let take_v = pattern(&v.fields, &fs, |f| &f.value);
                let take_r = pattern(&v.fields, &fs, |f| &f.resolver);
                let each_write = fs.iter().map(|f| {
                    let (val, tr) = (&f.value, f.tr());
                    quote!(let #val = #tr::write_out(#val, w);)
                });
                let make = pattern(&v.fields, &fs, |f| &f.value);
                writes.push(quote! {
                    #name::#vn #take_v => { #(#each_write)* #res::#vn #make }
                });
                let each_resolve = fs.iter().map(|f| {
                    let (val, r, tr) = (&f.value, &f.resolver, f.tr());
                    let m = match &f.member {
                        syn::Member::Named(n) => n.clone(),
                        syn::Member::Unnamed(i) => format_ident!("_{}", i.index),
                    };
                    quote! {
                        let __p = ::core::ptr::addr_of_mut!((*out).#m);
                        #tr::resolve(#val, at + (__p as usize - out as usize), #r, __p);
                    }
                });
                let make = rebuild(quote!(#name::#vn), &v.fields, &fs);
                decodes.push(quote!(#view::#vn #take_v => #make,));
                let tag = tag as u32;
                resolves.push(quote! {
                    (#name::#vn #take_v, #res::#vn #take_r) => {
                        let out = out as *mut #layout;
                        ::core::ptr::addr_of_mut!((*out).__tag).write(#tag);
                        #(#each_resolve)*
                    }
                });
            }
            let decoder = if decode {
                quote! {
                    impl ::rerail_section::Decode for #name {
                        #[allow(unused_variables)]
                        fn decode(a: &#view, __n: &::rerail_section::Names<'_>) -> Self {
                            match a { #(#decodes)* }
                        }
                    }
                }
            } else {
                quote!()
            };
            Ok(quote! {
                #decoder

                #[doc = #doc]
                #[repr(u32)]
                #[derive(#(#derives),*)]
                #vis enum #view { #(#view_vs,)* }

                #[doc(hidden)]
                #[allow(dead_code)]
                #vis enum #res { #(#res_vs,)* }

                const _: () = {
                    #(#layouts)*

                    // SAFETY: a `repr(u32)` enum is a union of `repr(C)` records, each its tag and then its
                    // variant's fields (the reference's "primitive representation"): `resolve` writes one of those.
                    unsafe impl ::rerail_section::Archive for #name {
                        type Archived = #view;
                        type Resolver = #res;
                        #[allow(unused_variables)]
                        fn write_out(&self, w: &mut ::rerail_section::Writer) -> #res {
                            match self { #(#writes)* }
                        }
                        #[allow(unused_variables, unreachable_patterns)]
                        unsafe fn resolve(&self, at: usize, r: #res, out: *mut #view) {
                            unsafe {
                                match (self, r) {
                                    #(#resolves)*
                                    _ => unreachable!("a resolver of another variant"),
                                }
                            }
                        }
                    }
                };
            })
        }
        Data::Union(_) => Err(syn::Error::new_spanned(name, "Archive: a union is not supported")),
    }
}
