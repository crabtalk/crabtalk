//! Tests for default_context_limit prefix matching.

use crabtalk_core::model::default_context_limit;

#[test]
fn claude_family() {
    assert_eq!(default_context_limit("claude-3-sonnet"), 200_000);
    assert_eq!(default_context_limit("claude-3-opus"), 200_000);
}

#[test]
fn gpt4o_family() {
    assert_eq!(default_context_limit("gpt-4o"), 128_000);
    assert_eq!(default_context_limit("gpt-4o-mini"), 128_000);
}

#[test]
fn gpt4_turbo() {
    assert_eq!(default_context_limit("gpt-4-turbo"), 128_000);
}

#[test]
fn gpt4_non_turbo() {
    assert_eq!(default_context_limit("gpt-4"), 8_192);
}

#[test]
fn gpt35() {
    assert_eq!(default_context_limit("gpt-3.5-turbo"), 16_385);
}

#[test]
fn o_series() {
    assert_eq!(default_context_limit("o1-preview"), 200_000);
    assert_eq!(default_context_limit("o3-mini"), 200_000);
    assert_eq!(default_context_limit("o4-mini"), 200_000);
}

#[test]
fn grok() {
    assert_eq!(default_context_limit("grok-2"), 131_072);
}

#[test]
fn qwen() {
    assert_eq!(default_context_limit("qwen-72b"), 32_768);
    assert_eq!(default_context_limit("qwq-32b"), 32_768);
}

#[test]
fn kimi_moonshot() {
    assert_eq!(default_context_limit("kimi-k1"), 128_000);
    assert_eq!(default_context_limit("moonshot-v1"), 128_000);
}

#[test]
fn unknown_model_default() {
    assert_eq!(default_context_limit("llama-3"), 8_192);
    assert_eq!(default_context_limit("unknown"), 8_192);
}
