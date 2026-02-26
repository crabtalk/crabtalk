//! Shared helpers for runtime examples.

#![allow(dead_code)]

use deepseek::DeepSeek;
use llm::LLM;
use walrus_runtime::{DEFAULT_COMPACT_PROMPT, DEFAULT_FLUSH_PROMPT, Hook, Memory, prelude::*};

/// Example hook wiring DeepSeek as the LLM provider.
pub struct ExampleHook;

impl Hook for ExampleHook {
    type Provider = DeepSeek;
    type Memory = InMemory;

    fn compact() -> &'static str {
        DEFAULT_COMPACT_PROMPT
    }

    fn flush() -> &'static str {
        DEFAULT_FLUSH_PROMPT
    }
}

/// Initialize tracing with env-filter support.
pub fn init_tracing() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
}

/// Load DEEPSEEK_API_KEY from .env file, falling back to environment.
pub fn load_api_key() -> String {
    let _ = dotenvy::dotenv();
    std::env::var("DEEPSEEK_API_KEY").expect("DEEPSEEK_API_KEY must be set")
}

/// Build a default Runtime with DeepSeek provider and InMemory.
pub fn build_runtime() -> Runtime<ExampleHook> {
    let key = load_api_key();
    let provider = DeepSeek::new(llm::Client::new(), &key).expect("failed to create provider");
    Runtime::new(General::default(), provider, InMemory::new())
}

/// Simple REPL loop: read lines from stdin, stream to agent.
pub async fn repl<H: Hook + 'static>(runtime: &mut Runtime<H>, agent: &str) {
    use futures_util::StreamExt;
    use std::io::{BufRead, Write};

    loop {
        print!("> ");
        std::io::stdout().flush().unwrap();
        let mut input = String::new();
        if std::io::stdin().lock().read_line(&mut input).unwrap() == 0 {
            break;
        }
        let input = input.trim();
        if input.is_empty() || input == "exit" || input == "quit" {
            break;
        }
        let mut stream = std::pin::pin!(runtime.stream_to(agent, Message::user(input)));
        while let Some(result) = stream.next().await {
            match result {
                Ok(chunk) => {
                    if let Some(delta) = chunk.content() {
                        print!("{delta}");
                        std::io::stdout().flush().ok();
                    }
                }
                Err(e) => {
                    eprintln!("\nError: {e}");
                    break;
                }
            }
        }
        println!();
    }
}

/// REPL loop that prints memory entries after each exchange.
pub async fn repl_with_memory<H: Hook + 'static>(runtime: &mut Runtime<H>, agent: &str) {
    use futures_util::StreamExt;
    use std::io::{BufRead, Write};

    loop {
        print!("> ");
        std::io::stdout().flush().unwrap();
        let mut input = String::new();
        if std::io::stdin().lock().read_line(&mut input).unwrap() == 0 {
            break;
        }
        let input = input.trim();
        if input.is_empty() || input == "exit" || input == "quit" {
            break;
        }
        {
            let mut stream = std::pin::pin!(runtime.stream_to(agent, Message::user(input)));
            while let Some(result) = stream.next().await {
                match result {
                    Ok(chunk) => {
                        if let Some(delta) = chunk.content() {
                            print!("{delta}");
                            std::io::stdout().flush().ok();
                        }
                    }
                    Err(e) => {
                        eprintln!("\nError: {e}");
                        break;
                    }
                }
            }
            println!();
        }

        // Print current memory state (stream dropped, borrow released).
        let entries = runtime.memory().entries();
        if entries.is_empty() {
            println!("[Memory: empty]");
        } else {
            println!("[Memory: {} entries]", entries.len());
            for (key, value) in &entries {
                let display = if value.len() > 60 {
                    // UTF-8 safe truncation.
                    let end = value
                        .char_indices()
                        .take_while(|&(i, _)| i <= 57)
                        .last()
                        .map(|(i, c)| i + c.len_utf8())
                        .unwrap_or(0);
                    format!("{}...", &value[..end])
                } else {
                    value.clone()
                };
                println!("  {key} = {display}");
            }
        }
    }
}
