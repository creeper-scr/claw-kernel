//! Summarizer implementations for conversation history.

use crate::error::AgentError;
use crate::traits::Summarizer;
use async_trait::async_trait;
use claw_provider::types::Message;

/// A simple summarizer that concatenates message previews.
///
/// This is a fast, local implementation that doesn't require LLM calls.
/// Suitable for simple use cases where exact summarization isn't critical.
pub struct SimpleSummarizer {
    /// Maximum length of the summary in characters.
    pub max_length: usize,
}

impl SimpleSummarizer {
    pub fn new(max_length: usize) -> Self {
        Self { max_length }
    }

    pub fn with_max_length(mut self, length: usize) -> Self {
        self.max_length = length;
        self
    }
}

impl Default for SimpleSummarizer {
    fn default() -> Self {
        Self::new(500)
    }
}

#[async_trait]
impl Summarizer for SimpleSummarizer {
    async fn summarize(&self, messages: &[Message]) -> Result<String, AgentError> {
        if messages.is_empty() {
            return Ok("No messages to summarize.".to_string());
        }

        let mut summary = String::new();
        summary.push_str("Summary of conversation:\n");

        for (i, msg) in messages.iter().enumerate().take(5) {
            let preview: String = msg
                .content
                .chars()
                .take(100)
                .collect();
            let preview = if msg.content.len() > 100 {
                format!("{}...", preview)
            } else {
                preview
            };
            
            summary.push_str(&format!(
                "[{}] {}: {}\n",
                i + 1,
                format!("{:?}", msg.role),
                preview
            ));
        }

        if messages.len() > 5 {
            summary.push_str(&format!("\n... and {} more messages", messages.len() - 5));
        }

        if summary.len() > self.max_length {
            summary.truncate(self.max_length);
            summary.push_str("...");
        }

        Ok(summary)
    }
}

/// A summarizer that uses an LLM to generate summaries.
///
/// This provides higher quality summaries but requires an LLM provider
/// and incurs additional latency/cost.
pub struct LlmSummarizer<P> {
    provider: P,
    model: String,
    max_summary_tokens: u32,
}

impl<P> LlmSummarizer<P> {
    pub fn new(provider: P, model: impl Into<String>) -> Self {
        Self {
            provider,
            model: model.into(),
            max_summary_tokens: 256,
        }
    }

    pub fn with_max_tokens(mut self, tokens: u32) -> Self {
        self.max_summary_tokens = tokens;
        self
    }
}

// Note: LLM-based summarizer requires integration with claw-provider.
// This is a placeholder implementation that would need the provider trait.
// For now, we only provide the SimpleSummarizer as a working implementation.

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_simple_summarizer_empty() {
        let summarizer = SimpleSummarizer::new(500);
        let result = summarizer.summarize(&[]).await.unwrap();
        assert_eq!(result, "No messages to summarize.");
    }

    #[tokio::test]
    async fn test_simple_summarizer_single_message() {
        let summarizer = SimpleSummarizer::new(500);
        let messages = vec![Message::user("Hello, how are you?")];
        let result = summarizer.summarize(&messages).await.unwrap();
        assert!(result.contains("Summary of conversation"));
        assert!(result.contains("Hello, how are you?"));
    }

    #[tokio::test]
    async fn test_simple_summarizer_multiple_messages() {
        let summarizer = SimpleSummarizer::new(500);
        let messages = vec![
            Message::user("First message"),
            Message::assistant("Second message"),
            Message::user("Third message"),
        ];
        let result = summarizer.summarize(&messages).await.unwrap();
        assert!(result.contains("First message"));
        assert!(result.contains("Second message"));
        assert!(result.contains("Third message"));
    }

    #[tokio::test]
    async fn test_simple_summarizer_truncates_long_content() {
        let summarizer = SimpleSummarizer::new(500);
        let long_content = "a".repeat(200);
        let messages = vec![Message::user(&long_content)];
        let result = summarizer.summarize(&messages).await.unwrap();
        assert!(result.contains("..."));
        assert!(result.len() < 250); // Should be truncated
    }

    #[tokio::test]
    async fn test_simple_summarizer_more_than_five() {
        let summarizer = SimpleSummarizer::new(500);
        let messages: Vec<Message> = (0..10)
            .map(|i| Message::user(&format!("Message {}", i)))
            .collect();
        let result = summarizer.summarize(&messages).await.unwrap();
        assert!(result.contains("and 5 more messages"));
    }

    #[tokio::test]
    async fn test_simple_summarizer_respects_max_length() {
        let summarizer = SimpleSummarizer::new(50);
        let messages: Vec<Message> = (0..10)
            .map(|i| Message::user(&format!("Message number {} with some content", i)))
            .collect();
        let result = summarizer.summarize(&messages).await.unwrap();
        assert!(result.len() <= 60); // Allow some buffer
        assert!(result.ends_with("..."));
    }
}
