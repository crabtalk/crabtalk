//! One declaration, two expansions — `harnesses!` from the guest, `host!` from
//! the host.
//!
//! A system harness is native host code behind a name. Both ends of that name
//! have to agree on the framing, and hand-writing them is how they drift: the
//! guest builds `[path]` and the host reads `fields[0]`, in different crates,
//! kept in step by memory. Here they come from one grammar.

use crate::docs;
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{
    Ident, LitStr, Token,
    parse::{Parse, ParseStream},
    punctuated::Punctuated,
    token,
};

/// Which side is being generated. Chosen by the macro the declaration was
/// written under, not by the declaration — the crate you reach it through is
/// the side you are on, so the wrong one is not spellable.
#[derive(Clone, Copy, PartialEq)]
pub enum Side {
    Guest,
    Host,
}

pub struct Declaration {
    namespace: String,
    modules: Vec<Module>,
}

struct Module {
    docs: String,
    name: Ident,
    calls: Vec<Call>,
}

struct Call {
    docs: String,
    name: Ident,
    params: Vec<Param>,
    /// The trailing `&[(&str, &str)]`, flattened into pairs of fields.
    tail: Option<Param>,
    reply: Reply,
}

struct Param {
    name: Ident,
    ty: Ty,
}

/// What a call answers with.
enum Reply {
    /// No fields.
    Nothing,
    /// One field, staged raw — the bytes are the whole reply.
    One(Ty),
    /// Two or more, framed. A struct is generated to name them.
    Many(Vec<Param>),
}

/// The closed set of field types.
///
/// Closed on purpose: every system harness that exists is served by it, and
/// anything richer frames its own payload, which is what `protocol` does with
/// protobuf.
#[derive(Clone, Copy, PartialEq)]
enum Ty {
    /// `&str` out, `String` back.
    Text,
    /// `&[u8]` out, `Vec<u8>` back.
    Bytes,
    /// Decimal text on the wire.
    Int(u8),
    /// `&[(&str, &str)]`, only as the trailing parameter.
    Pairs,
}

impl Parse for Declaration {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let key: Ident = input.parse()?;
        if key != "namespace" {
            return Err(syn::Error::new(
                key.span(),
                "a declaration opens with `namespace = \"...\";`",
            ));
        }
        input.parse::<Token![=]>()?;
        let namespace: LitStr = input.parse()?;
        input.parse::<Token![;]>()?;

        let mut modules = Vec::new();
        while !input.is_empty() {
            modules.push(input.parse()?);
        }
        if modules.is_empty() {
            return Err(syn::Error::new(
                namespace.span(),
                "a declaration needs at least one `mod`",
            ));
        }

        Ok(Self {
            namespace: namespace.value(),
            modules,
        })
    }
}

impl Parse for Module {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let attrs = input.call(syn::Attribute::parse_outer)?;
        input.parse::<Token![mod]>()?;
        let name: Ident = input.parse()?;

        let inner;
        syn::braced!(inner in input);
        let mut calls = Vec::new();
        while !inner.is_empty() {
            calls.push(inner.parse()?);
        }
        if calls.is_empty() {
            return Err(syn::Error::new(name.span(), "a harness needs a call"));
        }

        Ok(Self {
            docs: docs(&attrs),
            name,
            calls,
        })
    }
}

impl Parse for Call {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let attrs = input.call(syn::Attribute::parse_outer)?;
        input.parse::<Token![fn]>()?;
        let name: Ident = input.parse()?;

        let inner;
        syn::parenthesized!(inner in input);
        let declared = inner.parse_terminated(Param::parse, Token![,])?;

        // Only the last parameter may be pairs: the flattening has no
        // terminator, so anything after it would be indistinguishable from more
        // pairs once the fields are on the wire.
        let mut params: Vec<Param> = declared.into_iter().collect();
        let tail = match params.iter().position(|p| p.ty == Ty::Pairs) {
            Some(at) if at + 1 == params.len() => params.pop(),
            Some(at) => {
                return Err(syn::Error::new(
                    params[at].name.span(),
                    "`&[(&str, &str)]` has to be the last parameter — it flattens into \
                     fields, so nothing can follow it",
                ));
            }
            None => None,
        };

        let reply = if input.peek(Token![->]) {
            input.parse::<Token![->]>()?;
            input.parse()?
        } else {
            Reply::Nothing
        };
        input.parse::<Token![;]>()?;

        Ok(Self {
            docs: docs(&attrs),
            name,
            params,
            tail,
            reply,
        })
    }
}

impl Parse for Param {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let name: Ident = input.parse()?;
        input.parse::<Token![:]>()?;
        Ok(Self {
            name,
            ty: input.parse()?,
        })
    }
}

impl Parse for Reply {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        // A tuple carries names so the generated struct has fields rather than
        // `.0` and `.1`, which say nothing at the call site.
        if input.peek(token::Paren) {
            let inner;
            syn::parenthesized!(inner in input);
            let fields: Punctuated<Param, Token![,]> =
                inner.parse_terminated(Param::parse, Token![,])?;
            let fields: Vec<Param> = fields.into_iter().collect();
            if fields.len() < 2 {
                return Err(syn::Error::new(
                    fields
                        .first()
                        .map(|f| f.name.span())
                        .unwrap_or_else(proc_macro2::Span::call_site),
                    "a reply of one field is written without parentheses",
                ));
            }
            for field in &fields {
                if field.ty == Ty::Pairs {
                    return Err(syn::Error::new(
                        field.name.span(),
                        "a reply field cannot be pairs",
                    ));
                }
            }
            return Ok(Self::Many(fields));
        }
        Ok(Self::One(input.parse()?))
    }
}

impl Parse for Ty {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let ty: syn::Type = input.parse()?;
        let text = quote!(#ty).to_string().replace(' ', "");
        Ok(match text.as_str() {
            "&str" | "String" => Ty::Text,
            "&[u8]" | "Vec<u8>" => Ty::Bytes,
            "u16" => Ty::Int(16),
            "u32" => Ty::Int(32),
            "u64" => Ty::Int(64),
            "&[(&str,&str)]" => Ty::Pairs,
            other => {
                return Err(syn::Error::new_spanned(
                    &ty,
                    format!(
                        "unsupported type: {other} (expected `&str`, `&[u8]`, `String`, \
                         `Vec<u8>`, `u16`, `u32`, `u64`, or `&[(&str, &str)]`)"
                    ),
                ));
            }
        })
    }
}

impl Ty {
    /// How a parameter is written into a request field.
    fn encode(&self, name: &Ident) -> TokenStream {
        match self {
            Ty::Text => quote!(#name.as_bytes()),
            Ty::Bytes => quote!(#name),
            Ty::Int(_) => quote!(#name.to_string().as_bytes()),
            Ty::Pairs => unreachable!("pairs are flattened, never a single field"),
        }
    }

    /// The owned type a reply field lands in.
    fn owned(&self) -> TokenStream {
        match self {
            Ty::Text => quote!(::alloc::string::String),
            Ty::Bytes => quote!(::alloc::vec::Vec<u8>),
            Ty::Int(16) => quote!(u16),
            Ty::Int(32) => quote!(u32),
            _ => quote!(u64),
        }
    }

    /// The owned type a host implementation answers with. [`Ty::owned`]'s
    /// counterpart: the guest reaches these through `alloc`, a host through
    /// std.
    fn host_owned(&self) -> TokenStream {
        match self {
            Ty::Text => quote!(::std::string::String),
            Ty::Bytes => quote!(::std::vec::Vec<u8>),
            Ty::Int(16) => quote!(u16),
            Ty::Int(32) => quote!(u32),
            _ => quote!(u64),
        }
    }

    /// The borrowed type a parameter takes.
    fn borrowed(&self) -> TokenStream {
        match self {
            Ty::Text => quote!(&str),
            Ty::Bytes => quote!(&[u8]),
            Ty::Int(16) => quote!(u16),
            Ty::Int(32) => quote!(u32),
            Ty::Int(_) => quote!(u64),
            Ty::Pairs => quote!(&[(&str, &str)]),
        }
    }

    /// Read one reply field back out of `bytes`.
    fn decode(&self, bytes: TokenStream) -> TokenStream {
        match self {
            Ty::Text => quote!(::alloc::string::String::from_utf8_lossy(#bytes).into_owned()),
            Ty::Bytes => quote!(#bytes.to_vec()),
            Ty::Int(_) => quote! {
                ::core::str::from_utf8(#bytes)
                    .ok()
                    .and_then(|text| text.parse().ok())
                    .ok_or(::alloc::string::String::from(
                        "the host sent a number this harness cannot read",
                    ))?
            },
            Ty::Pairs => unreachable!("a reply is never pairs"),
        }
    }
}

impl Declaration {
    pub fn expand(&self, side: Side) -> TokenStream {
        let modules = self.modules.iter().map(|module| {
            let name = &module.name;
            let docs = doc(&module.docs);
            let calls = module.calls.iter().map(|call| match side {
                Side::Guest => self.guest(module, call),
                Side::Host => self.host(module, call),
            });
            quote! {
                #docs
                pub mod #name {
                    #(#calls)*
                }
            }
        });
        quote!(#(#modules)*)
    }

    /// The stub a harness calls.
    fn guest(&self, module: &Module, call: &Call) -> TokenStream {
        let wire = format!("{}.{}.{}", self.namespace, module.name, call.name);
        let ident = &call.name;
        let docs = doc(&call.docs);
        let konst = konst(call);

        let args = call.params.iter().chain(&call.tail).map(|p| {
            let name = &p.name;
            let ty = p.ty.borrowed();
            quote!(#name: #ty)
        });
        let fields = call.params.iter().map(|p| p.ty.encode(&p.name));

        // Fixed fields first, then the tail flattened in place, so the host
        // sees one sequence and never has to know where the split was.
        let build = match &call.tail {
            None => quote!(let request = ::berm_lang::abi::wire::request(&[#(#fields),*]);),
            Some(tail) => {
                let tail = &tail.name;
                quote! {
                    let mut request = ::berm_lang::abi::wire::request(&[#(#fields),*]);
                    for (key, value) in #tail {
                        ::berm_lang::abi::wire::field(&mut request, key.as_bytes());
                        ::berm_lang::abi::wire::field(&mut request, value.as_bytes());
                    }
                }
            }
        };

        let (reply, ret, body) = match &call.reply {
            Reply::Nothing => (
                quote!(),
                quote!(()),
                quote!(::berm_lang::abi::host::call(#konst, &request).map(|_| ())),
            ),
            // Staged raw: one blob needs no framing to be told apart from the
            // rest of itself, and `fs::read` would pay four bytes per file for
            // the privilege.
            Reply::One(Ty::Bytes) => (
                quote!(),
                quote!(::alloc::vec::Vec<u8>),
                quote!(::berm_lang::abi::host::call(#konst, &request)),
            ),
            Reply::One(ty) => {
                let owned = ty.owned();
                let decode = ty.decode(quote!(&reply[..]));
                (
                    quote!(),
                    owned,
                    quote! {
                        let reply = ::berm_lang::abi::host::call(#konst, &request)?;
                        ::core::result::Result::Ok(#decode)
                    },
                )
            }
            Reply::Many(fields) => {
                let ty = format_ident!("{}", pascal(&call.name.to_string()));
                let count = fields.len();
                let declared = fields.iter().map(|f| {
                    let name = &f.name;
                    let owned = f.ty.owned();
                    quote!(pub #name: #owned)
                });
                let reads = fields.iter().enumerate().map(|(at, f)| {
                    let name = &f.name;
                    let value = f.ty.decode(quote!(parts[#at]));
                    quote!(#name: #value)
                });
                (
                    quote! {
                        #docs
                        pub struct #ty { #(#declared),* }
                    },
                    quote!(#ty),
                    quote! {
                        let reply = ::berm_lang::abi::host::call(#konst, &request)?;
                        let ::core::option::Option::Some(parts) =
                            ::berm_lang::abi::wire::fields(&reply)
                        else {
                            return ::core::result::Result::Err(::alloc::string::String::from(
                                "the host framed a reply this harness cannot read",
                            ));
                        };
                        if parts.len() != #count {
                            return ::core::result::Result::Err(::alloc::string::String::from(
                                "the host's reply has the wrong number of fields",
                            ));
                        }
                        ::core::result::Result::Ok(#ty { #(#reads),* })
                    },
                )
            }
        };

        quote! {
            #reply

            #docs
            pub const #konst: u64 = ::berm_lang::abi::hash(#wire);

            #docs
            pub fn #ident(#(#args),*)
                -> ::core::result::Result<#ret, ::alloc::string::String>
            {
                #build
                #body
            }
        }
    }

    /// The constructor a host serves a name with.
    ///
    /// It takes the implementation and returns the registrable `Harness`, so
    /// the fields the guest built are read back by generated code rather than
    /// by an index written out per call — which is where a system harness
    /// actually goes wrong.
    fn host(&self, module: &Module, call: &Call) -> TokenStream {
        let wire = format!("{}.{}.{}", self.namespace, module.name, call.name);
        let ident = &call.name;
        let docs = doc(&call.docs);
        let konst = konst(call);
        let fixed = call.params.len();

        let takes = call
            .params
            .iter()
            .chain(&call.tail)
            .map(|p| p.ty.borrowed())
            .collect::<Vec<_>>();

        let reads = call.params.iter().enumerate().map(|(at, p)| {
            let binding = &p.name;
            let what = p.name.to_string();
            let value = match p.ty {
                Ty::Text => quote!(::berm::wire::text(&fields, #at, #what)?),
                Ty::Bytes => quote!(fields[#at]),
                Ty::Int(_) => {
                    let ty = p.ty.borrowed();
                    quote! {
                        ::core::str::from_utf8(fields[#at])
                            .ok()
                            .and_then(|text| text.parse::<#ty>().ok())
                            .ok_or_else(|| {
                                ::berm::anyhow::anyhow!(concat!(#what, " is not a number"))
                            })?
                    }
                }
                Ty::Pairs => unreachable!("pairs are the tail, read below"),
            };
            quote!(let #binding = #value;)
        });

        // The tail has no terminator, so its arity is the only thing that says
        // it is well formed: everything past the fixed fields is a key and a
        // value, and an odd count means one of them is missing.
        let (arity, pairs, names) = match &call.tail {
            None => (
                quote! {
                    if fields.len() != #fixed {
                        ::berm::anyhow::bail!(
                            concat!(#wire, " takes ", stringify!(#fixed), " fields, the request has {}"),
                            fields.len()
                        );
                    }
                },
                quote!(),
                call.params.iter().map(|p| &p.name).collect::<Vec<_>>(),
            ),
            Some(tail) => {
                let binding = &tail.name;
                let what = tail.name.to_string();
                let mut names = call.params.iter().map(|p| &p.name).collect::<Vec<_>>();
                names.push(binding);
                (
                    quote! {
                        if fields.len() < #fixed {
                            ::berm::anyhow::bail!(
                                concat!(#wire, " takes at least ", stringify!(#fixed), " fields, the request has {}"),
                                fields.len()
                            );
                        }
                    },
                    quote! {
                        let trailing = &fields[#fixed..];
                        if trailing.len() % 2 != 0 {
                            ::berm::anyhow::bail!(concat!(#what, " has a key with no value"));
                        }
                        let mut #binding = ::std::vec::Vec::with_capacity(trailing.len() / 2);
                        for pair in trailing.chunks(2) {
                            #binding.push((
                                ::core::str::from_utf8(pair[0])?,
                                ::core::str::from_utf8(pair[1])?,
                            ));
                        }
                        let #binding = &#binding[..];
                    },
                    names,
                )
            }
        };

        let (returns, reply, answer) = match &call.reply {
            Reply::Nothing => (
                quote!(()),
                quote!(),
                quote! {
                    serve(#(#names),*)?;
                    ::core::result::Result::Ok(::std::vec::Vec::new())
                },
            ),
            // Staged raw, exactly as the guest reads it back.
            Reply::One(Ty::Bytes) => (
                quote!(::std::vec::Vec<u8>),
                quote!(),
                quote!(serve(#(#names),*)),
            ),
            Reply::One(ty) => {
                let owned = ty.host_owned();
                let into = match ty {
                    Ty::Text => quote!(.into_bytes()),
                    _ => quote!(.to_string().into_bytes()),
                };
                (
                    owned,
                    quote!(),
                    quote!(::core::result::Result::Ok(serve(#(#names),*)? #into)),
                )
            }
            Reply::Many(fields) => {
                let ty = format_ident!("{}", pascal(&call.name.to_string()));
                let declared = fields.iter().map(|f| {
                    let name = &f.name;
                    let owned = f.ty.host_owned();
                    quote!(pub #name: #owned)
                });
                // An int field needs somewhere to live as text for as long as
                // the frame borrows it, which is what these bindings are.
                let staged = fields.iter().map(|f| {
                    let name = &f.name;
                    match f.ty {
                        Ty::Int(_) => quote!(let #name = reply.#name.to_string();),
                        _ => quote!(),
                    }
                });
                let borrowed = fields.iter().map(|f| {
                    let name = &f.name;
                    match f.ty {
                        Ty::Text => quote!(reply.#name.as_bytes()),
                        Ty::Bytes => quote!(&reply.#name[..]),
                        Ty::Int(_) => quote!(#name.as_bytes()),
                        Ty::Pairs => unreachable!("a reply is never pairs"),
                    }
                });
                (
                    quote!(#ty),
                    quote! {
                        #docs
                        pub struct #ty { #(#declared),* }
                    },
                    quote! {
                        let reply = serve(#(#names),*)?;
                        #(#staged)*
                        ::core::result::Result::Ok(::berm::wire::frame(&[#(#borrowed),*]))
                    },
                )
            }
        };

        quote! {
            #reply

            #docs
            pub const #konst: &str = #wire;

            #docs
            pub fn #ident(
                serve: impl Fn(#(#takes),*) -> ::berm::anyhow::Result<#returns>
                    + Send + Sync + 'static,
            ) -> ::berm::Harness {
                ::berm::Harness {
                    name: ::std::string::String::from(#konst),
                    call: ::std::sync::Arc::new(move |request: &[u8]| {
                        let fields = ::berm::wire::fields(request)?;
                        #arity
                        #(#reads)*
                        #pairs
                        #answer
                    }),
                }
            }
        }
    }
}

/// `read` -> `READ`, the constant carrying its wire name.
fn konst(call: &Call) -> Ident {
    format_ident!("{}", call.name.to_string().to_uppercase())
}

/// A doc comment, as an attribute the expansion can carry.
fn doc(text: &str) -> TokenStream {
    if text.is_empty() {
        return quote!();
    }
    let text = format!(" {text}");
    quote!(#[doc = #text])
}

/// `fetch` -> `Fetch`, for the struct a multi-field reply lands in.
fn pascal(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut upper = true;
    for c in name.chars() {
        if c == '_' {
            upper = true;
        } else if upper {
            out.extend(c.to_uppercase());
            upper = false;
        } else {
            out.push(c);
        }
    }
    out
}
