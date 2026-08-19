//! `#[harness]` — turns a module of plain functions into a loadable harness.
//!
//! The exports an ELF needs are ceremony: an entry point that keeps the linker
//! from discarding the image, a heap handshake, a description the host reads at
//! registration, and dispatch from a tool index back to a function. None of it
//! is the author's problem, so none of it is in their source.

use proc_macro::TokenStream;
use quote::quote;
use syn::{
    Expr, ExprLit, Item, ItemFn, ItemMod, Lit, LitStr, Token, parse::Parse, parse_macro_input,
};

mod harnesses;
mod schema;

/// Declare a set of system harnesses, generating one side of the ABI.
///
/// ```ignore
/// berm_lang::harnesses!(guest {
///     namespace = "berm";
///     mod fs {
///         /// Read a file whole.
///         fn read(path: &str) -> Vec<u8>;
///     }
/// });
/// ```
///
/// `guest` emits the stub a harness calls; `host` emits what a host registers
/// under. Crates that can share a declaration file pass a path instead of a
/// block, so the framing one side builds and the other reads has one source.
#[proc_macro]
pub fn harnesses(input: TokenStream) -> TokenStream {
    parse_macro_input!(input as harnesses::Declaration)
        .expand()
        .into()
}

/// Default size of the argument and result buffers, in bytes.
///
/// This is paid on every invocation, not once: the buffers live in `.bss`,
/// which the host zeroes each time it instantiates the guest. Measured against
/// the reference guest, 64 KiB costs a few microseconds per call over 16 KiB, and 4 KiB
/// buys nothing back. `buffer = N` overrides it for a harness that needs room.
const DEFAULT_BUFFER: usize = 16 * 1024;

/// JSON Schema for a tool that declares no parameters.
const DEFAULT_PARAMETERS: &str = r#"{"type":"object"}"#;

/// Prefix on every tool's exported symbol, so a tool called `init` or
/// `describe` cannot collide with the exports the ABI reserves.
const TOOL_PREFIX: &str = "berm_tool_";

struct Config {
    buffer: usize,
    usage: String,
    /// Kept only to re-emit as an `include_str!`, so editing the file
    /// rebuilds the harness.
    usage_file: Option<syn::LitStr>,
}

impl Parse for Config {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let mut buffer = DEFAULT_BUFFER;
        let mut usage = String::new();
        let mut usage_file = None;

        while !input.is_empty() {
            let key: syn::Ident = input.parse()?;
            input.parse::<Token![=]>()?;

            match key.to_string().as_str() {
                "buffer" => {
                    let size: syn::LitInt = input.parse()?;
                    buffer = size.base10_parse()?;
                }
                // A path rather than the text, because usage runs to
                // paragraphs and `include_str!` cannot help here: a proc
                // macro sees the unexpanded call, never the file. So the
                // macro reads it, and the expansion carries an
                // `include_str!` of its own purely so cargo rebuilds when
                // the file changes.
                "usage_file" => {
                    let path: syn::LitStr = input.parse()?;
                    let root = std::env::var("CARGO_MANIFEST_DIR").map_err(|_| {
                        syn::Error::new(key.span(), "usage_file needs CARGO_MANIFEST_DIR")
                    })?;
                    let full = std::path::Path::new(&root).join(path.value());
                    usage = std::fs::read_to_string(&full).map_err(|e| {
                        syn::Error::new(path.span(), format!("{}: {e}", full.display()))
                    })?;
                    usage_file = Some(path);
                }
                other => {
                    return Err(syn::Error::new(
                        key.span(),
                        format!("unknown argument: {other} (expected `buffer` or `usage_file`)"),
                    ));
                }
            }

            if !input.is_empty() {
                input.parse::<Token![,]>()?;
            }
        }

        Ok(Config {
            buffer,
            usage,
            usage_file,
        })
    }
}

/// One exported tool, as the description will see it.
struct Tool {
    ident: syn::Ident,
    name: String,
    description: String,
    parameters: String,
    args: Args,
}

/// Where a tool's schema comes from. The handler always receives the raw
/// blob — parsing is the author's choice, and not every harness wants a JSON
/// parser linked into it.
enum Args {
    /// No declared shape; the schema is an open object.
    Raw,
    /// A struct declared beside the tool, read for its fields.
    Struct(syn::Ident),
}

/// Declare a harness from a module of tool functions.
///
/// ```ignore
/// #[harness]
/// mod tools {
///     use berm_lang::{Failed, Out};
///
///     /// Echo the argument blob back.
///     pub fn echo(args: &[u8], out: &mut Out) -> Result<(), Failed> {
///         out.write(args);
///         Ok(())
///     }
/// }
/// ```
///
/// Every `pub fn` in the module becomes a tool. Its doc comment is the
/// description the model reads, and `#[params("…")]` carries a JSON Schema for
/// its arguments.
#[proc_macro_attribute]
pub fn harness(args: TokenStream, item: TokenStream) -> TokenStream {
    let config = parse_macro_input!(args as Config);
    let mut module = parse_macro_input!(item as ItemMod);

    let tools = match collect(&mut module) {
        Ok(tools) => tools,
        Err(error) => return error.to_compile_error().into(),
    };

    let description = describe(&config, &tools);
    let description_len = description.len();
    let description_bytes =
        syn::LitByteStr::new(description.as_bytes(), proc_macro2::Span::call_site());
    let module_ident = &module.ident;
    let buffer = config.buffer;

    // One export per tool, so the host resolves it the way it resolves any
    // other symbol. An index would couple the two sides by declaration order
    // for no gain — rvtime looks entries up by name already.
    let exports = tools.iter().map(|tool| {
        let ident = &tool.ident;
        let symbol = syn::Ident::new(&format!("{TOOL_PREFIX}{}", tool.name), ident.span());
        let doc = format!("Tool `{}`. {}", tool.name, tool.description);

        quote! {
            #[doc = #doc]
            #[unsafe(no_mangle)]
            pub extern "C" fn #symbol() -> ::berm_lang::Buf {
                let arguments = unsafe { &mut *::core::ptr::addr_of_mut!(_CRABTALK_ARGS) };
                let length = ::berm_lang::abi::read_args(arguments);
                if length > _CRABTALK_BUFFER {
                    return ::berm_lang::abi::fail(b"arguments exceeded the input buffer");
                }
                let arguments = &arguments[..length];

                let buffer = unsafe { &mut *::core::ptr::addr_of_mut!(_CRABTALK_OUT) };
                let mut out = ::berm_lang::Out::new(buffer);

                match #module_ident::#ident(arguments, &mut out) {
                    Ok(()) if out.overflowed() => {
                        ::berm_lang::abi::fail(b"result exceeded the output buffer")
                    }
                    Ok(()) => out.finish(),
                    Err(::berm_lang::Failed) => {
                        ::berm_lang::abi::fail(out.written())
                    }
                }
            }
        }
    });

    let anchors = tools.iter().map(|tool| {
        let symbol = syn::Ident::new(&format!("{TOOL_PREFIX}{}", tool.name), tool.ident.span());
        quote! { ^ #symbol as *const () as u64 }
    });

    // The macro read the usage file itself, which cargo cannot see. This
    // makes the dependency visible so editing it rebuilds the harness.
    let usage_rebuild = config.usage_file.as_ref().map(|path| {
        quote! {
            const _CRABTALK_USAGE: &str =
                include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/", #path));
        }
    });

    quote! {
        #module

        #usage_rebuild

        const _CRABTALK_BUFFER: usize = #buffer;
        static mut _CRABTALK_ARGS: [u8; _CRABTALK_BUFFER] = [0; _CRABTALK_BUFFER];
        static mut _CRABTALK_OUT: [u8; _CRABTALK_BUFFER] = [0; _CRABTALK_BUFFER];

        /// What this harness is, as a section rather than an export: the host
        /// reads it out of the ELF without compiling or running anything, so
        /// learning what a harness claims never means executing it.
        #[used]
        #[cfg_attr(target_arch = "riscv64", unsafe(link_section = ".berm.abi"))]
        static _CRABTALK_ABI: [u8; #description_len] = *#description_bytes;

        /// A harness is a binary, and off the guest's target a binary needs a
        /// `main` — which is what lets `cargo test` build one natively.
        #[cfg(not(target_arch = "riscv64"))]
        fn main() {}

        /// The ELF entry point. Never called: it exists so `--gc-sections`
        /// keeps the exports, which nothing else in the image references.
        /// Off the guest's target the C runtime owns `_start`, and defining a
        /// second one fails the native test link on ELF platforms.
        #[cfg(target_arch = "riscv64")]
        #[unsafe(no_mangle)]
        pub extern "C" fn _start() {
            static mut ANCHOR: u64 = 0;
            unsafe {
                ::core::ptr::write_volatile(
                    &raw mut ANCHOR,
                    _CRABTALK_ABI.as_ptr() as u64
                        #(#anchors)*,
                );
            }
        }


        #(#exports)*
    }
    .into()
}

/// Pull the tools out of the module, along with the structs that describe
/// their arguments, stripping the attributes that only exist to be read here.
fn collect(module: &mut ItemMod) -> syn::Result<Vec<Tool>> {
    let Some((_, items)) = module.content.as_mut() else {
        return Err(syn::Error::new_spanned(
            &module.ident,
            "#[harness] needs a module with a body, not a `mod foo;` declaration",
        ));
    };

    let mut tools = Vec::new();
    for item in items.iter_mut() {
        let Item::Fn(function) = item else { continue };
        if !matches!(function.vis, syn::Visibility::Public(_)) {
            continue;
        }
        tools.push(tool(function)?);
    }

    if tools.is_empty() {
        return Err(syn::Error::new_spanned(
            &module.ident,
            "a harness needs at least one `pub fn` to expose as a tool",
        ));
    }

    // Second pass: a tool taking a struct gets its schema from that struct's
    // fields, and the struct gets the derive that lets it be deserialized. The
    // author writes neither.
    for item in items.iter_mut() {
        let Item::Struct(item) = item else { continue };
        let Some(tool) = tools
            .iter_mut()
            .find(|tool| matches!(&tool.args, Args::Struct(ty) if ty == &item.ident))
        else {
            continue;
        };
        tool.parameters = schema::object(item)?;
        // The struct is an interface declaration: read for its shape, never
        // constructed. Saying so beats an author silencing the warning.
        item.attrs.push(syn::parse_quote!(#[allow(dead_code)]));
        // Reached through the SDK rather than named directly, so a harness
        // depends on one crate and cannot pick a serde the derive disagrees
        // with. This is why the author writes neither line.
        item.attrs
            .push(syn::parse_quote!(#[derive(::berm_lang::serde::Deserialize)]));
        item.attrs
            .push(syn::parse_quote!(#[serde(crate = "::berm_lang::serde")]));
    }

    for tool in &tools {
        if let Args::Struct(ty) = &tool.args
            && tool.parameters == DEFAULT_PARAMETERS
        {
            return Err(syn::Error::new_spanned(
                ty,
                format!(
                    "`{ty}` is not declared in this module — a tool's argument struct has to live \
                     beside it so the schema can be derived from its fields"
                ),
            ));
        }
    }

    Ok(tools)
}

fn tool(function: &mut ItemFn) -> syn::Result<Tool> {
    let description = docs(&function.attrs);
    let args = args_of(function)?;
    let mut parameters = None;

    for attribute in &function.attrs {
        if attribute.path().is_ident("params") {
            parameters = Some(attribute.parse_args::<LitStr>()?.value());
        }
    }

    // `params` exists for this macro to read; leaving it would reach the
    // compiler as an unknown attribute.
    function.attrs.retain(|a| !a.path().is_ident("params"));

    if description.is_empty() {
        return Err(syn::Error::new_spanned(
            &function.sig.ident,
            "a tool needs a doc comment — it is what the model reads to decide whether to call it",
        ));
    }

    Ok(Tool {
        ident: function.sig.ident.clone(),
        name: function.sig.ident.to_string(),
        description,
        args,
        parameters: parameters.unwrap_or_else(|| DEFAULT_PARAMETERS.to_owned()),
    })
}

/// Where a tool's schema comes from: `#[args(Shape)]` names a struct declared
/// beside it. The handler still receives bytes.
fn args_of(function: &mut ItemFn) -> syn::Result<Args> {
    let mut declared = None;
    for attribute in &function.attrs {
        if attribute.path().is_ident("args") {
            declared = Some(attribute.parse_args::<syn::Ident>()?);
        }
    }
    function.attrs.retain(|a| !a.path().is_ident("args"));

    match function.sig.inputs.first() {
        Some(syn::FnArg::Typed(first)) if matches!(&*first.ty, syn::Type::Reference(r) if matches!(&*r.elem, syn::Type::Slice(_))) =>
            {}
        _ => {
            return Err(syn::Error::new_spanned(
                &function.sig,
                "a tool takes the argument blob as `&[u8]` and an `&mut Out` to write its result \
                 into — declare its shape with #[args(Struct)] if it has one",
            ));
        }
    }

    Ok(declared.map_or(Args::Raw, Args::Struct))
}

/// Join a doc comment into one line — the description is a JSON string.
pub(crate) fn docs(attrs: &[syn::Attribute]) -> String {
    let mut description = String::new();
    for attribute in attrs {
        if attribute.path().is_ident("doc")
            && let syn::Meta::NameValue(pair) = &attribute.meta
            && let Expr::Lit(ExprLit {
                lit: Lit::Str(line),
                ..
            }) = &pair.value
        {
            if !description.is_empty() {
                description.push(' ');
            }
            description.push_str(line.value().trim());
        }
    }
    description.trim().to_owned()
}

/// Build the description JSON at compile time.
fn describe(config: &Config, tools: &[Tool]) -> String {
    let tools = tools
        .iter()
        .map(|t| {
            format!(
                r#"{{"name":"{}","description":"{}","parameters":{}}}"#,
                escape(&t.name),
                escape(&t.description),
                t.parameters,
            )
        })
        .collect::<Vec<_>>()
        .join(",");

    format!(
        r#"{{"abi_version":0,"tools":[{tools}],"usage":"{}"}}"#,
        escape(&config.usage),
    )
}

/// Minimal JSON string escaping — doc comments are prose and can carry
/// anything, and a stray quote would produce a description the host cannot
/// parse at all.
pub(crate) fn escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            c if (c as u32) < 0x20 => escaped.push_str(&format!("\\u{:04x}", c as u32)),
            c => escaped.push(c),
        }
    }
    escaped
}
