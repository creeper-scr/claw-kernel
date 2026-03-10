//! Helper utilities for EventTrigger support (G-08).
//!
//! Provides pattern matching, condition evaluation, and message template
//! rendering for the `trigger.add_event` IPC endpoint.

use std::borrow::Cow;

use claw_runtime::events::Event;

/// Returns a canonical dot-separated event type name for the given event.
///
/// For [`Event::Custom`] events the `event_type` field is returned directly,
/// enabling user-defined event name hierarchies (e.g. `"data.ready"`).
pub(crate) fn event_type_name(event: &Event) -> Cow<'static, str> {
    match event {
        Event::AgentStarted { .. }                  => "agent.started".into(),
        Event::AgentStopped { .. }                  => "agent.stopped".into(),
        Event::LlmRequestStarted { .. }             => "llm.request.started".into(),
        Event::LlmRequestCompleted { .. }           => "llm.request.completed".into(),
        Event::MessageReceived { .. }               => "message.received".into(),
        Event::A2A(_)                               => "a2a".into(),
        Event::ToolCalled { .. }                    => "tool.called".into(),
        Event::ToolResult { .. }                    => "tool.result".into(),
        Event::ContextWindowApproachingLimit { .. } => "context.window.limit".into(),
        Event::MemoryArchiveComplete { .. }         => "memory.archive.complete".into(),
        Event::ModeChanged { .. }                   => "mode.changed".into(),
        Event::Extension(_)                         => "extension".into(),
        Event::AgentRestarted { .. }                => "agent.restarted".into(),
        Event::AgentFailed { .. }                   => "agent.failed".into(),
        Event::Shutdown                             => "shutdown".into(),
        Event::Custom { event_type, .. }            => event_type.clone().into(),
        // #[non_exhaustive]: future event variants default to "unknown"
        _ => "unknown".into(),
    }
}

/// Converts a glob-style event pattern into a compiled [`regex::Regex`].
///
/// Supported wildcards:
/// - `*`  — matches any sequence of characters (including `.`)
/// - `?`  — matches exactly one character
///
/// All other regex metacharacters are escaped, so literal dots in event
/// names (e.g. `"agent.started"`) are safe to use directly.
pub(crate) fn build_event_pattern_regex(pattern: &str) -> Result<regex::Regex, regex::Error> {
    let mut s = String::with_capacity(pattern.len() + 4);
    s.push('^');
    for ch in pattern.chars() {
        match ch {
            '*' => s.push_str(".*"),
            '?' => s.push('.'),
            // Escape regex metacharacters so literal dots etc. work correctly.
            '.' | '+' | '(' | ')' | '[' | ']' | '{' | '}' | '^' | '$' | '|' | '\\' => {
                s.push('\\');
                s.push(ch);
            }
            _ => s.push(ch),
        }
    }
    s.push('$');
    regex::Regex::new(&s)
}

/// Returns `true` if the serialised event satisfies `condition`.
///
/// The condition object supports a simple equality check:
/// ```json
/// { "field": "agent_id", "equals": "agent-001" }
/// ```
///
/// `field` is a dot-separated JSON path into the serialised event value.
/// If the condition has no `"field"` key the function always returns `true`.
pub(crate) fn condition_matches(event: &Event, condition: &serde_json::Value) -> bool {
    let field = match condition.get("field").and_then(|v| v.as_str()) {
        Some(f) => f,
        None => return true,
    };
    let expected = match condition.get("equals") {
        Some(v) => v,
        None => return true,
    };

    let event_json = match serde_json::to_value(event) {
        Ok(v) => v,
        Err(_) => return false,
    };

    // Navigate a simple dot-separated path inside the serialised event.
    let mut current = &event_json;
    for part in field.split('.') {
        match current.get(part) {
            Some(v) => current = v,
            None => return false,
        }
    }
    current == expected
}

/// Renders a message template by substituting `{event.type}` with the
/// canonical event type name.
pub(crate) fn render_template(template: &str, event: &Event) -> String {
    let name = event_type_name(event);
    template.replace("{event.type}", name.as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;
    use claw_runtime::agent_types::AgentId;

    #[test]
    fn test_event_type_name_builtin() {
        let id = AgentId::new("a1");
        assert_eq!(event_type_name(&Event::AgentStarted { agent_id: id.clone() }), "agent.started");
        assert_eq!(event_type_name(&Event::AgentStopped { agent_id: id, reason: "done".into() }), "agent.stopped");
        assert_eq!(event_type_name(&Event::Shutdown), "shutdown");
    }

    #[test]
    fn test_event_type_name_custom() {
        let ev = Event::Custom {
            event_type: "data.ready".to_string(),
            data: serde_json::Value::Null,
        };
        assert_eq!(event_type_name(&ev), "data.ready");
    }

    #[test]
    fn test_build_event_pattern_regex_exact() {
        let re = build_event_pattern_regex("agent.started").unwrap();
        assert!(re.is_match("agent.started"));
        assert!(!re.is_match("agent.stopped"));
    }

    #[test]
    fn test_build_event_pattern_regex_wildcard() {
        let re = build_event_pattern_regex("agent.*").unwrap();
        assert!(re.is_match("agent.started"));
        assert!(re.is_match("agent.stopped"));
        assert!(!re.is_match("llm.request.started"));
    }

    #[test]
    fn test_build_event_pattern_regex_double_wildcard() {
        let re = build_event_pattern_regex("*").unwrap();
        assert!(re.is_match("agent.started"));
        assert!(re.is_match("data.ready"));
    }

    #[test]
    fn test_condition_matches_no_field() {
        let id = AgentId::new("a1");
        let ev = Event::AgentStarted { agent_id: id };
        assert!(condition_matches(&ev, &serde_json::json!({})));
    }

    #[test]
    fn test_render_template() {
        let id = AgentId::new("a1");
        let ev = Event::AgentStarted { agent_id: id };
        let out = render_template("Event: {event.type} received", &ev);
        assert_eq!(out, "Event: agent.started received");
    }

    #[test]
    fn test_render_template_no_placeholder() {
        let id = AgentId::new("a1");
        let ev = Event::AgentStarted { agent_id: id };
        let out = render_template("no substitution here", &ev);
        assert_eq!(out, "no substitution here");
    }
}
