use heck::ToKebabCase;
use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{ItemStruct, LitStr, Token, parse::Parse, parse_macro_input};

struct CommandArgs {
    kind: String,
    name: Option<String>,
    label: Option<String>,
}

impl Parse for CommandArgs {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let mut kind = None;
        let mut name = None;
        let mut label = None;

        while !input.is_empty() {
            let ident: syn::Ident = input.parse()?;
            input.parse::<Token![=]>()?;
            let value: LitStr = input.parse()?;

            match ident.to_string().as_str() {
                "kind" => kind = Some(value.value()),
                "name" => name = Some(value.value()),
                "label" => label = Some(value.value()),
                other => {
                    return Err(syn::Error::new(
                        ident.span(),
                        format!("unknown attribute: {other}"),
                    ));
                }
            }

            if !input.is_empty() {
                input.parse::<Token![,]>()?;
            }
        }

        let kind = kind.ok_or_else(|| input.error("missing required attribute: kind"))?;
        Ok(CommandArgs { kind, name, label })
    }
}

#[proc_macro_attribute]
pub fn command(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as CommandArgs);
    let input = parse_macro_input!(item as ItemStruct);

    let struct_name = &input.ident;
    let name = args
        .name
        .unwrap_or_else(|| struct_name.to_string().to_kebab_case());
    let label = args.label.unwrap_or_else(|| format!("ai.crabtalk.{name}"));

    let command_enum = format_ident!("{}Command", struct_name);

    let start_doc = format!("Install and start the {name} service.");
    let stop_doc = format!("Stop and uninstall the {name} service.");
    let run_doc = format!("Run the {name} service directly (used by launchd/systemd).");
    let logs_doc = format!("View {name} service logs.");

    let run_arm = match args.kind.as_str() {
        "mcp" => quote! {
            #command_enum::Run { .. } => {
                crabtalk_command::run_mcp(self).await?
            }
        },
        "client" => quote! {
            #command_enum::Run { .. } => {
                self.run().await?
            }
        },
        _ => {
            return syn::Error::new_spanned(struct_name, "kind must be \"mcp\" or \"client\"")
                .to_compile_error()
                .into();
        }
    };

    let expanded = quote! {
        #input

        impl crabtalk_command::Service for #struct_name {
            fn name(&self) -> &str {
                #name
            }
            fn description(&self) -> &str {
                env!("CARGO_PKG_DESCRIPTION")
            }
            fn label(&self) -> &str {
                #label
            }
        }

        #[derive(Debug, clap::Subcommand)]
        pub enum #command_enum {
            #[doc = #start_doc]
            Start {
                /// Increase log verbosity (-v = info, -vv = debug, -vvv = trace).
                #[arg(short, long, action = clap::ArgAction::Count)]
                verbose: u8,
            },
            #[doc = #stop_doc]
            Stop,
            #[doc = #run_doc]
            Run {
                /// Increase log verbosity (-v = info, -vv = debug, -vvv = trace).
                #[arg(short, long, action = clap::ArgAction::Count)]
                verbose: u8,
            },
            #[doc = #logs_doc]
            Logs {
                #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
                tail_args: Vec<String>,
            },
        }

        impl #struct_name {
            pub async fn exec(
                &self,
                action: #command_enum,
            ) -> crabtalk_command::anyhow::Result<()> {
                use crabtalk_command::Service as _;
                match action {
                    #command_enum::Start { verbose } => self.start(verbose)?,
                    #command_enum::Stop => self.stop()?,
                    #run_arm
                    #command_enum::Logs { tail_args } => {
                        self.logs(&tail_args)?
                    }
                }
                Ok(())
            }
        }

        impl #command_enum {
            /// Init tracing, build a tokio runtime, and run the command.
            pub fn start(self, svc: #struct_name) {
                let verbose = match &self {
                    Self::Run { verbose } | Self::Start { verbose } => *verbose,
                    _ => 0,
                };
                crabtalk_command::run(verbose, move || async move { svc.exec(self).await });
            }
        }
    };

    expanded.into()
}
