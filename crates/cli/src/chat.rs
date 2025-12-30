//! Chat command

use super::Config;
use crate::agents::{AgentKind, Anto};
use anyhow::Result;
use clap::{Args, ValueEnum};
use cydonia::{Agent, Chat, Client, DeepSeek, LLM, Message, StreamChunk};
use futures_util::StreamExt;
use std::{
    fmt::{Display, Formatter},
    io::{BufRead, Write},
};

/// Chat command arguments
#[derive(Debug, Args)]
pub struct ChatCmd {
    /// The model provider to use
    #[arg(short, long, default_value = "deepseek")]
    pub model: Model,

    /// The agent to use for the chat
    #[arg(short, long)]
    pub agent: Option<AgentKind>,

    /// Whether to enable thinking
    #[arg(short, long)]
    pub think: bool,

    /// The message to send (if empty, starts interactive mode)
    pub message: Option<String>,
}

impl ChatCmd {
    /// Run the chat command
    pub async fn run(&self, stream: bool) -> Result<()> {
        let mut config = Config::load()?;
        let key = config
            .key
            .get(&self.model.to_string())
            .ok_or_else(|| anyhow::anyhow!("missing {:?} API key in config", self.model))?;
        let provider = match self.model {
            Model::Deepseek => DeepSeek::new(Client::new(), key)?,
        };

        // override the think flag in the config
        config.config.think = self.think;

        // run the chat
        match self.agent {
            Some(AgentKind::Anto) => {
                let mut chat = provider.chat(config.config().clone()).system(Anto);
                self.run_chat(&mut chat, stream).await
            }
            None => {
                let mut chat = provider.chat(config.config().clone());
                self.run_chat(&mut chat, stream).await
            }
        }
    }

    async fn run_chat<A>(&self, chat: &mut Chat<DeepSeek, A>, stream: bool) -> Result<()>
    where
        A: Agent<Chunk = StreamChunk>,
    {
        if let Some(msg) = &self.message {
            Self::send(chat, Message::user(msg), stream).await?;
        } else {
            let stdin = std::io::stdin();
            let mut stdout = std::io::stdout();
            loop {
                print!("> ");
                stdout.flush()?;

                let mut input = String::new();
                if stdin.lock().read_line(&mut input)? == 0 {
                    break;
                }

                let input = input.trim();
                if input.is_empty() {
                    continue;
                }
                if input == "/quit" || input == "/exit" {
                    break;
                }

                Self::send(chat, Message::user(input), stream).await?;
            }
        }

        Ok(())
    }

    async fn send<A>(chat: &mut Chat<DeepSeek, A>, message: Message, stream: bool) -> Result<()>
    where
        A: Agent<Chunk = StreamChunk>,
    {
        if stream {
            let mut response_content = String::new();
            let mut reasoning = false;
            let mut stream = std::pin::pin!(chat.stream(message));
            while let Some(Ok(chunk)) = stream.next().await {
                if let Some(content) = chunk.content() {
                    if reasoning {
                        println!("\n\n\nCONTENT");
                        reasoning = false;
                    }
                    print!("{content}");
                    response_content.push_str(content);
                }

                if let Some(reasoning_content) = chunk.reasoning_content() {
                    if !reasoning {
                        println!("REASONING");
                        reasoning = true;
                    }
                    print!("{reasoning_content}");
                    response_content.push_str(reasoning_content);
                }
            }
            println!();
        } else {
            let response = chat.send(message).await?;
            if let Some(reasoning_content) = response.reasoning() {
                println!("REASONING\n{reasoning_content}");
            }

            if let Some(content) = response.content() {
                println!("\n\nCONTENT\n{content}");
            }
        }
        Ok(())
    }
}

/// Available model providers
#[derive(Debug, Clone, Copy, Default, ValueEnum)]
pub enum Model {
    /// DeepSeek model
    #[default]
    Deepseek,
}

impl Display for Model {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Model::Deepseek => write!(f, "deepseek"),
        }
    }
}
