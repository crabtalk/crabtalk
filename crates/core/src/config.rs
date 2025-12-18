//! Configuration for a chat

/// Chat configuration
#[derive(Debug, Clone)]
pub struct Config {
    /// The model to use
    pub model: &'static str,

    /// Whether to enable thinking
    pub think: bool,

    /// The frequency penalty of the model
    pub frequency: i8,

    /// Whether to response in JSON
    pub json: bool,

    /// Whether to return the log probabilities
    pub logprobs: bool,

    /// The presence penalty of the model
    pub presence: i8,

    /// Whether to stream the response
    pub stream: bool,

    /// The temperature of the model
    pub temperature: f32,

    /// The top probability of the model
    pub top_p: f32,

    /// The number of top log probabilities to return
    pub top_logprobs: usize,

    /// The number of max tokens to generate
    pub tokens: usize,

    /// Whether to return the usage information
    pub usage: bool,
}
