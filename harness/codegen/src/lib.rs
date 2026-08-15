//! `#[harness]` — turns a module of plain functions into a loadable harness.
//!
//! The exports an ELF needs are ceremony: an entry point that keeps the linker
//! from discarding the image, a heap handshake, a description the host reads at
//! registration, and dispatch from a tool index back to a function. None of it
//! is the author's problem, so none of it is in their source.

use proc_macro::TokenStream;
use quote::quote;
use syn::{
    Expr, ExprArray, ExprLit, Item, ItemFn, ItemMod, Lit, LitStr, Token, parse::Parse,
    parse_macro_input,
};

/// Default size of the argument and result buffers, in bytes.
///
/// This is paid on every invocation, not once: the buffers live in `.bss`,
/// which the host zeroes each time it instantiates the guest. Measured against
/// the spike, 64 KiB costs a few microseconds per call over 16 KiB, and 4 KiB
/// buys nothing back. `buffer = N` overrides it for a harness that needs room.
const DEFAULT_BUFFER: usize = 16 * 1024;

/// JSON Schema for a tool that declares no parameters.
const DEFAULT_PARAMETERS: &str = r#"{"type":"object"}"#;

/// Prefix on every tool's exported symbol, so a tool called `init` or
/// `describe` cannot collide with the exports the ABI reserves.
const TOOL_PREFIX: &str = "crabtalk_tool_";

struct Args {
    capabilities: Vec<String>,
    buffer: usize,
}

impl Parse for Args {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let mut capabilities = Vec::new();
        let mut buffer = DEFAULT_BUFFER;

        while !input.is_empty() {
            let key: syn::Ident = input.parse()?;
            input.parse::<Token![=]>()?;

            match key.to_string().as_str() {
                "capabilities" => {
                    let array: ExprArray = input.parse()?;
                    for element in array.elems {
                        match element {
                            Expr::Lit(ExprLit {
                                lit: Lit::Str(text),
                                ..
                            }) => capabilities.push(text.value()),
                            other => {
                                return Err(syn::Error::new_spanned(
                                    other,
                                    "a capability is a string literal",
                                ));
                            }
                        }
                    }
                }
                "buffer" => {
                    let size: syn::LitInt = input.parse()?;
                    buffer = size.base10_parse()?;
                }
                other => {
                    return Err(syn::Error::new(
                        key.span(),
                        format!("unknown argument: {other} (expected `capabilities` or `buffer`)"),
                    ));
                }
            }

            if !input.is_empty() {
                input.parse::<Token![,]>()?;
            }
        }

        Ok(Args {
            capabilities,
            buffer,
        })
    }
}

/// One exported tool, as the description will see it.
struct Tool {
    ident: syn::Ident,
    name: String,
    description: String,
    parameters: String,
}

/// Declare a harness from a module of tool functions.
///
/// ```ignore
/// #[harness(capabilities = ["log"])]
/// mod tools {
///     use crabtalk_harness_sdk::{Failed, Out};
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
    let args = parse_macro_input!(args as Args);
    let mut module = parse_macro_input!(item as ItemMod);

    let tools = match collect(&mut module) {
        Ok(tools) => tools,
        Err(error) => return error.to_compile_error().into(),
    };

    let description = describe(&args, &tools);
    let module_ident = &module.ident;
    let buffer = args.buffer;

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
            pub extern "C" fn #symbol() -> ::crabtalk_harness_sdk::Buf {
                let arguments = unsafe { &mut *::core::ptr::addr_of_mut!(_CRABTALK_ARGS) };
                let length = ::crabtalk_harness_sdk::read_args(arguments);
                if length > _CRABTALK_BUFFER {
                    return ::crabtalk_harness_sdk::fail(b"arguments exceeded the input buffer");
                }
                let arguments = &arguments[..length];

                let buffer = unsafe { &mut *::core::ptr::addr_of_mut!(_CRABTALK_OUT) };
                let mut out = ::crabtalk_harness_sdk::Out::new(buffer);

                match #module_ident::#ident(arguments, &mut out) {
                    Ok(()) if out.overflowed() => {
                        ::crabtalk_harness_sdk::fail(b"result exceeded the output buffer")
                    }
                    Ok(()) => out.finish(),
                    Err(::crabtalk_harness_sdk::Failed) => {
                        ::crabtalk_harness_sdk::fail(out.written())
                    }
                }
            }
        }
    });

    let anchors = tools.iter().map(|tool| {
        let symbol = syn::Ident::new(&format!("{TOOL_PREFIX}{}", tool.name), tool.ident.span());
        quote! { ^ #symbol as *const () as u64 }
    });

    quote! {
        #module

        const _CRABTALK_BUFFER: usize = #buffer;
        static mut _CRABTALK_ARGS: [u8; _CRABTALK_BUFFER] = [0; _CRABTALK_BUFFER];
        static mut _CRABTALK_OUT: [u8; _CRABTALK_BUFFER] = [0; _CRABTALK_BUFFER];

        /// What this harness is. Built at compile time, so reading it costs
        /// the host one call and the guest no work.
        const _CRABTALK_DESCRIPTION: &str = #description;

        /// The ELF entry point. Never called: it exists so `--gc-sections`
        /// keeps the exports, which nothing else in the image references.
        #[unsafe(no_mangle)]
        pub extern "C" fn _start() {
            static mut ANCHOR: u64 = 0;
            unsafe {
                ::core::ptr::write_volatile(
                    &raw mut ANCHOR,
                    describe as *const () as u64
                        ^ init as *const () as u64
                        #(#anchors)*,
                );
            }
        }

        /// Hands the guest the heap the host committed for it.
        #[unsafe(no_mangle)]
        pub extern "C" fn init(start: u64, size: u64) -> u64 {
            ::crabtalk_harness_sdk::init_heap(start, size);
            0
        }

        /// ABI version, tools, and the capabilities this harness wants.
        #[unsafe(no_mangle)]
        pub extern "C" fn describe() -> ::crabtalk_harness_sdk::Buf {
            ::crabtalk_harness_sdk::Buf::new(_CRABTALK_DESCRIPTION.as_bytes())
        }

        #(#exports)*
    }
    .into()
}

/// Pull the tools out of the module, stripping the attributes that only exist
/// to be read here.
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

    Ok(tools)
}

fn tool(function: &mut ItemFn) -> syn::Result<Tool> {
    let mut description = String::new();
    let mut parameters = None;

    for attribute in &function.attrs {
        if attribute.path().is_ident("doc") {
            if let syn::Meta::NameValue(pair) = &attribute.meta
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
        } else if attribute.path().is_ident("params") {
            parameters = Some(attribute.parse_args::<LitStr>()?.value());
        }
    }

    // `params` exists for this macro to read; leaving it would reach the
    // compiler as an unknown attribute.
    function.attrs.retain(|a| !a.path().is_ident("params"));

    let description = description.trim().to_owned();
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
        parameters: parameters.unwrap_or_else(|| DEFAULT_PARAMETERS.to_owned()),
    })
}

/// Build the description JSON at compile time.
fn describe(args: &Args, tools: &[Tool]) -> String {
    let capabilities = args
        .capabilities
        .iter()
        .map(|c| format!("\"{}\"", escape(c)))
        .collect::<Vec<_>>()
        .join(",");

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

    format!(r#"{{"abi_version":0,"capabilities":[{capabilities}],"tools":[{tools}]}}"#)
}

/// Minimal JSON string escaping — doc comments are prose and can carry
/// anything, and a stray quote would produce a description the host cannot
/// parse at all.
fn escape(text: &str) -> String {
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
