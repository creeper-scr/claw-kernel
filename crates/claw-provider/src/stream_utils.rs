//! SSE 流处理公共工具函数。

use bytes::Bytes;
use futures::{Stream, StreamExt};

use crate::error::ProviderError;
use crate::traits::MessageFormat;
use crate::types::Delta;

/// Parse a single Server-Sent Events (SSE) line and extract the data payload.
///
/// Strips the mandatory `"data: "` prefix defined by the SSE specification and
/// returns the remainder of the line.  Lines that do not carry a data payload
/// (comments starting with `":"`, blank lines, `"event:"` / `"id:"` / `"retry:"`
/// field lines) are silently ignored and `None` is returned.
///
/// The `"[DONE]"` sentinel that OpenAI-compatible APIs append at the end of a
/// stream is **not** filtered here; callers are responsible for detecting
/// `Some("[DONE]")` and stopping iteration.
///
/// # Arguments
///
/// * `line` - A single line from an SSE byte stream.  Leading/trailing
///   whitespace is trimmed before matching.
///
/// # Returns
///
/// * `Some(data)` – the substring after `"data: "` if the trimmed line begins
///   with that prefix.
/// * `None` – for any other line (comments, empty lines, non-data fields).
///
/// # Examples
///
/// ```rust,ignore
/// use claw_provider::stream_utils::parse_sse_line;
///
/// assert_eq!(parse_sse_line("data: hello"), Some("hello"));
/// assert_eq!(parse_sse_line("data: [DONE]"), Some("[DONE]"));
/// assert_eq!(parse_sse_line(": comment"), None);
/// assert_eq!(parse_sse_line(""), None);
/// assert_eq!(parse_sse_line("event: message"), None);
/// ```
pub fn parse_sse_line(line: &str) -> Option<&str> {
    let line = line.trim();
    if line.starts_with("data: ") {
        Some(&line["data: ".len()..])
    } else {
        None
    }
}

/// Parse a raw SSE byte stream into a stream of [`Delta`] values.
///
/// Each byte chunk delivered by the HTTP response body may contain multiple
/// newline-separated SSE lines.  This function flattens those lines into
/// individual [`Delta`] items by delegating chunk parsing to the
/// [`MessageFormat`] implementation supplied via the `Format` type parameter.
///
/// This helper is provider-agnostic: any backend that speaks the standard
/// line-delimited SSE protocol (Anthropic, OpenAI, DeepSeek, Moonshot, …)
/// can pass its `Format` implementation and reuse this logic.
///
/// # Arguments
///
/// * `byte_stream` – An async stream of `Result<Bytes, ProviderError>` items,
///   typically obtained from a streaming HTTP response body.
///
/// # Returns
///
/// An `impl Stream<Item = Result<Delta, ProviderError>>` that yields one
/// [`Delta`] per successfully parsed SSE data line and propagates transport
/// or parse errors as `Err` items.  Empty lines and lines that produce no
/// delta (e.g. the `[DONE]` sentinel) are silently dropped.
///
/// # Examples
///
/// ```rust,ignore
/// use futures::StreamExt;
/// use claw_provider::stream_utils::parse_sse_stream;
/// use claw_provider::anthropic::AnthropicFormat;
///
/// let byte_stream = /* ... reqwest streaming body ... */;
/// let mut delta_stream = parse_sse_stream::<AnthropicFormat>(byte_stream);
///
/// while let Some(result) = delta_stream.next().await {
///     match result {
///         Ok(delta) => println!("token: {:?}", delta),
///         Err(e) => eprintln!("stream error: {}", e),
///     }
/// }
/// ```
pub fn parse_sse_stream<Format>(
    byte_stream: impl Stream<Item = Result<Bytes, ProviderError>> + Send + 'static,
) -> impl Stream<Item = Result<Delta, ProviderError>> + Send + 'static
where
    Format: MessageFormat + 'static,
{
    byte_stream.flat_map(|chunk_result| {
        let deltas: Vec<Result<Delta, ProviderError>> = match chunk_result {
            Err(e) => vec![Err(e)],
            Ok(bytes) => {
                let text = String::from_utf8_lossy(&bytes);
                text.lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                    .filter_map(|line| {
                        match Format::parse_stream_chunk(line.as_bytes()) {
                            Ok(Some(delta)) => Some(Ok(delta)),
                            Ok(None) => None,
                            Err(e) => Some(Err(ProviderError::Other(e.to_string()))),
                        }
                    })
                    .collect()
            }
        };
        futures::stream::iter(deltas)
    })
}
