//! Finite State Machine for the Agent Loop.
//!
//! Provides strict state management with validated transitions and broadcast notifications.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Current state of the agent loop execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash, Default)]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
    /// Initial state, ready to start.
    #[default]
    Idle,
    /// Actively processing (checking conditions, preparing requests).
    Running,
    /// Waiting for LLM response.
    AwaitingLLM,
    /// Executing tool calls.
    ToolExecuting,
    /// Paused by user interrupt, can resume.
    Paused,
    /// Successfully completed.
    Completed,
    /// Error occurred.
    Error,
}

impl AgentState {
    /// Returns a human-readable description of the state.
    pub fn description(&self) -> &'static str {
        match self {
            AgentState::Idle => "Agent is idle, ready to start",
            AgentState::Running => "Agent is actively processing",
            AgentState::AwaitingLLM => "Agent is waiting for LLM response",
            AgentState::ToolExecuting => "Agent is executing tools",
            AgentState::Paused => "Agent is paused",
            AgentState::Completed => "Agent has completed successfully",
            AgentState::Error => "Agent encountered an error",
        }
    }
}

/// Events that can trigger state transitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[serde(rename_all = "snake_case")]
pub enum StateEvent {
    /// Start or resume the agent loop.
    Start,
    /// LLM request has been sent.
    LLMRequestSent,
    /// LLM response received.
    LLMResponseReceived,
    /// Tool calls are required.
    ToolsRequired,
    /// All tool calls completed.
    ToolsCompleted,
    /// User requested to pause.
    UserInterrupt,
    /// Stop condition met (max turns, token budget, etc.).
    StopConditionMet,
    /// Error occurred during execution.
    Error,
    /// Reset the agent to initial state.
    Reset,
}

/// Result of a state transition attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransitionResult {
    /// Transition succeeded, returns the new state.
    Success(AgentState),
    /// Transition is invalid, includes details about why.
    Invalid {
        /// Current state when transition was attempted.
        from: AgentState,
        /// Event that was attempted.
        event: StateEvent,
        /// List of events that would be valid from this state.
        allowed: Vec<StateEvent>,
    },
}

/// Type alias for state transition hook callbacks.
pub type TransitionHook = Box<dyn Fn(&AgentState, &AgentState) + Send + Sync>;

/// Finite State Machine for agent loop state management.
///
/// Validates all state transitions and maintains the transition rules.
/// Supports transition hooks for observing state changes.
pub struct StateMachine {
    /// Current state of the machine.
    current: AgentState,
    /// Transition table: (from_state, event) -> to_state
    transitions: HashMap<(AgentState, StateEvent), AgentState>,
    /// Allowed events per state for error reporting.
    allowed_events: HashMap<AgentState, Vec<StateEvent>>,
    /// Hooks called on every state transition.
    transition_hooks: Vec<TransitionHook>,
}

// Manual Debug implementation that skips transition_hooks
impl std::fmt::Debug for StateMachine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StateMachine")
            .field("current", &self.current)
            .field("transitions", &self.transitions)
            .field("allowed_events", &self.allowed_events)
            .field("hook_count", &self.transition_hooks.len())
            .finish()
    }
}

// Manual Clone implementation that skips transition_hooks
impl Clone for StateMachine {
    fn clone(&self) -> Self {
        Self {
            current: self.current,
            transitions: self.transitions.clone(),
            allowed_events: self.allowed_events.clone(),
            transition_hooks: Vec::new(), // Hooks are not cloned
        }
    }
}

impl StateMachine {
    /// Creates a new StateMachine in the Idle state with all transitions configured.
    pub fn new() -> Self {
        let mut machine = Self {
            current: AgentState::Idle,
            transitions: HashMap::new(),
            allowed_events: HashMap::new(),
            transition_hooks: Vec::new(),
        };
        machine.build_transition_table();
        machine
    }

    /// Registers a hook that will be called on every state transition.
    ///
    /// The hook receives `(old_state, new_state)` as parameters.
    /// Hooks are called in registration order after a successful transition.
    pub fn on_transition<F>(&mut self, hook: F)
    where
        F: Fn(&AgentState, &AgentState) + Send + Sync + 'static,
    {
        self.transition_hooks.push(Box::new(hook));
    }

    /// Returns the number of registered transition hooks.
    pub fn hook_count(&self) -> usize {
        self.transition_hooks.len()
    }

    /// Clears all transition hooks.
    pub fn clear_hooks(&mut self) {
        self.transition_hooks.clear();
    }

    /// Returns the current state.
    pub fn current_state(&self) -> AgentState {
        self.current
    }

    /// Attempts to transition to a new state based on the event.
    ///
    /// Returns `TransitionResult::Success(new_state)` if the transition is valid,
    /// or `TransitionResult::Invalid` with details about allowed transitions.
    ///
    /// On successful transition, all registered hooks are called with
    /// `(old_state, new_state)` before returning.
    pub fn transition(&mut self, event: StateEvent) -> TransitionResult {
        if let Some(&new_state) = self.transitions.get(&(self.current, event)) {
            let old_state = self.current;
            self.current = new_state;
            tracing::debug!(
                "State transition: {:?} + {:?} -> {:?}",
                old_state,
                event,
                new_state
            );
            // Trigger all transition hooks
            for hook in &self.transition_hooks {
                hook(&old_state, &new_state);
            }
            TransitionResult::Success(new_state)
        } else {
            let allowed = self
                .allowed_events
                .get(&self.current)
                .cloned()
                .unwrap_or_default();
            TransitionResult::Invalid {
                from: self.current,
                event,
                allowed,
            }
        }
    }

    /// Returns true if the given event would result in a valid transition from current state.
    pub fn can_transition(&self, event: StateEvent) -> bool {
        self.transitions.contains_key(&(self.current, event))
    }

    /// Returns the list of events that are valid from the current state.
    pub fn valid_events(&self) -> Vec<StateEvent> {
        self.allowed_events
            .get(&self.current)
            .cloned()
            .unwrap_or_default()
    }

    /// Resets the state machine to Idle state.
    pub fn reset(&mut self) {
        self.current = AgentState::Idle;
    }

    /// Builds the complete transition table according to the specification:
    ///
    /// - Idle: Start → Running, Reset → Idle
    /// - Running: LLMRequestSent → AwaitingLLM, StopConditionMet → Completed, Error → Error, UserInterrupt → Paused
    /// - AwaitingLLM: LLMResponseReceived → Running/ToolExecuting, Error → Error, UserInterrupt → Paused
    /// - ToolExecuting: ToolsCompleted → Running, Error → Error, UserInterrupt → Paused
    /// - Paused: Start → Running, Reset → Idle
    /// - Completed/Error: Reset → Idle
    fn build_transition_table(&mut self) {
        // Idle state
        self.add_transition(AgentState::Idle, StateEvent::Start, AgentState::Running);
        self.add_transition(AgentState::Idle, StateEvent::Reset, AgentState::Idle);

        // Running state
        self.add_transition(
            AgentState::Running,
            StateEvent::LLMRequestSent,
            AgentState::AwaitingLLM,
        );
        self.add_transition(
            AgentState::Running,
            StateEvent::StopConditionMet,
            AgentState::Completed,
        );
        self.add_transition(AgentState::Running, StateEvent::Error, AgentState::Error);
        self.add_transition(
            AgentState::Running,
            StateEvent::UserInterrupt,
            AgentState::Paused,
        );

        // AwaitingLLM state - note: LLMResponseReceived can go to Running (no tools) or ToolExecuting
        // The state machine allows both possibilities, actual determination happens in agent logic
        self.add_transition(
            AgentState::AwaitingLLM,
            StateEvent::LLMResponseReceived,
            AgentState::Running,
        );
        self.add_transition(
            AgentState::AwaitingLLM,
            StateEvent::ToolsRequired,
            AgentState::ToolExecuting,
        );
        self.add_transition(
            AgentState::AwaitingLLM,
            StateEvent::Error,
            AgentState::Error,
        );
        self.add_transition(
            AgentState::AwaitingLLM,
            StateEvent::UserInterrupt,
            AgentState::Paused,
        );

        // ToolExecuting state
        self.add_transition(
            AgentState::ToolExecuting,
            StateEvent::ToolsCompleted,
            AgentState::Running,
        );
        self.add_transition(
            AgentState::ToolExecuting,
            StateEvent::Error,
            AgentState::Error,
        );
        self.add_transition(
            AgentState::ToolExecuting,
            StateEvent::UserInterrupt,
            AgentState::Paused,
        );

        // Paused state
        self.add_transition(AgentState::Paused, StateEvent::Start, AgentState::Running);
        self.add_transition(AgentState::Paused, StateEvent::Reset, AgentState::Idle);

        // Completed state
        self.add_transition(AgentState::Completed, StateEvent::Reset, AgentState::Idle);

        // Error state
        self.add_transition(AgentState::Error, StateEvent::Reset, AgentState::Idle);
    }

    fn add_transition(&mut self, from: AgentState, event: StateEvent, to: AgentState) {
        self.transitions.insert((from, event), to);

        // Track allowed events for error reporting
        self.allowed_events.entry(from).or_default().push(event);
    }
}

impl Default for StateMachine {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_state_machine_new_starts_idle() {
        let sm = StateMachine::new();
        assert_eq!(sm.current_state(), AgentState::Idle);
    }

    #[test]
    fn test_idle_start_to_running() {
        let mut sm = StateMachine::new();
        let result = sm.transition(StateEvent::Start);
        assert!(matches!(
            result,
            TransitionResult::Success(AgentState::Running)
        ));
        assert_eq!(sm.current_state(), AgentState::Running);
    }

    #[test]
    fn test_idle_reset_stays_idle() {
        let mut sm = StateMachine::new();
        let result = sm.transition(StateEvent::Reset);
        assert!(matches!(
            result,
            TransitionResult::Success(AgentState::Idle)
        ));
        assert_eq!(sm.current_state(), AgentState::Idle);
    }

    #[test]
    fn test_running_to_awaiting_llm() {
        let mut sm = StateMachine::new();
        sm.transition(StateEvent::Start);
        let result = sm.transition(StateEvent::LLMRequestSent);
        assert!(matches!(
            result,
            TransitionResult::Success(AgentState::AwaitingLLM)
        ));
    }

    #[test]
    fn test_running_to_completed_on_stop() {
        let mut sm = StateMachine::new();
        sm.transition(StateEvent::Start);
        let result = sm.transition(StateEvent::StopConditionMet);
        assert!(matches!(
            result,
            TransitionResult::Success(AgentState::Completed)
        ));
    }

    #[test]
    fn test_running_to_paused() {
        let mut sm = StateMachine::new();
        sm.transition(StateEvent::Start);
        let result = sm.transition(StateEvent::UserInterrupt);
        assert!(matches!(
            result,
            TransitionResult::Success(AgentState::Paused)
        ));
    }

    #[test]
    fn test_running_to_error() {
        let mut sm = StateMachine::new();
        sm.transition(StateEvent::Start);
        let result = sm.transition(StateEvent::Error);
        assert!(matches!(
            result,
            TransitionResult::Success(AgentState::Error)
        ));
    }

    #[test]
    fn test_awaiting_llm_response_to_running() {
        let mut sm = StateMachine::new();
        sm.transition(StateEvent::Start);
        sm.transition(StateEvent::LLMRequestSent);
        let result = sm.transition(StateEvent::LLMResponseReceived);
        assert!(matches!(
            result,
            TransitionResult::Success(AgentState::Running)
        ));
    }

    #[test]
    fn test_awaiting_llm_to_tool_executing() {
        let mut sm = StateMachine::new();
        sm.transition(StateEvent::Start);
        sm.transition(StateEvent::LLMRequestSent);
        let result = sm.transition(StateEvent::ToolsRequired);
        assert!(matches!(
            result,
            TransitionResult::Success(AgentState::ToolExecuting)
        ));
    }

    #[test]
    fn test_tool_executing_to_running() {
        let mut sm = StateMachine::new();
        sm.transition(StateEvent::Start);
        sm.transition(StateEvent::LLMRequestSent);
        sm.transition(StateEvent::ToolsRequired);
        let result = sm.transition(StateEvent::ToolsCompleted);
        assert!(matches!(
            result,
            TransitionResult::Success(AgentState::Running)
        ));
    }

    #[test]
    fn test_paused_to_running() {
        let mut sm = StateMachine::new();
        sm.transition(StateEvent::Start);
        sm.transition(StateEvent::UserInterrupt);
        let result = sm.transition(StateEvent::Start);
        assert!(matches!(
            result,
            TransitionResult::Success(AgentState::Running)
        ));
    }

    #[test]
    fn test_paused_to_idle() {
        let mut sm = StateMachine::new();
        sm.transition(StateEvent::Start);
        sm.transition(StateEvent::UserInterrupt);
        let result = sm.transition(StateEvent::Reset);
        assert!(matches!(
            result,
            TransitionResult::Success(AgentState::Idle)
        ));
    }

    #[test]
    fn test_completed_to_idle() {
        let mut sm = StateMachine::new();
        sm.transition(StateEvent::Start);
        sm.transition(StateEvent::StopConditionMet);
        assert_eq!(sm.current_state(), AgentState::Completed);
        let result = sm.transition(StateEvent::Reset);
        assert!(matches!(
            result,
            TransitionResult::Success(AgentState::Idle)
        ));
    }

    #[test]
    fn test_error_to_idle() {
        let mut sm = StateMachine::new();
        sm.transition(StateEvent::Start);
        sm.transition(StateEvent::Error);
        let result = sm.transition(StateEvent::Reset);
        assert!(matches!(
            result,
            TransitionResult::Success(AgentState::Idle)
        ));
    }

    #[test]
    fn test_invalid_transition() {
        let mut sm = StateMachine::new();
        // Cannot go directly from Idle to Completed
        let result = sm.transition(StateEvent::StopConditionMet);
        assert!(matches!(
            result,
            TransitionResult::Invalid {
                from: AgentState::Idle,
                event: StateEvent::StopConditionMet,
                ..
            }
        ));
    }

    #[test]
    fn test_can_transition_check() {
        let sm = StateMachine::new();
        assert!(sm.can_transition(StateEvent::Start));
        assert!(!sm.can_transition(StateEvent::StopConditionMet));
    }

    #[test]
    fn test_valid_events_idle() {
        let sm = StateMachine::new();
        let events = sm.valid_events();
        assert!(events.contains(&StateEvent::Start));
        assert!(events.contains(&StateEvent::Reset));
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn test_reset_method() {
        let mut sm = StateMachine::new();
        sm.transition(StateEvent::Start);
        sm.transition(StateEvent::LLMRequestSent);
        sm.reset();
        assert_eq!(sm.current_state(), AgentState::Idle);
    }

    #[test]
    fn test_state_descriptions() {
        assert!(!AgentState::Idle.description().is_empty());
        assert!(!AgentState::Running.description().is_empty());
        assert!(!AgentState::AwaitingLLM.description().is_empty());
        assert!(!AgentState::ToolExecuting.description().is_empty());
        assert!(!AgentState::Paused.description().is_empty());
        assert!(!AgentState::Completed.description().is_empty());
        assert!(!AgentState::Error.description().is_empty());
    }

    #[test]
    fn test_full_workflow_no_tools() {
        let mut sm = StateMachine::new();

        // Idle -> Running
        assert!(matches!(
            sm.transition(StateEvent::Start),
            TransitionResult::Success(AgentState::Running)
        ));

        // Running -> AwaitingLLM
        assert!(matches!(
            sm.transition(StateEvent::LLMRequestSent),
            TransitionResult::Success(AgentState::AwaitingLLM)
        ));

        // AwaitingLLM -> Running (no tools needed)
        assert!(matches!(
            sm.transition(StateEvent::LLMResponseReceived),
            TransitionResult::Success(AgentState::Running)
        ));

        // Running -> Completed
        assert!(matches!(
            sm.transition(StateEvent::StopConditionMet),
            TransitionResult::Success(AgentState::Completed)
        ));

        // Completed -> Idle
        assert!(matches!(
            sm.transition(StateEvent::Reset),
            TransitionResult::Success(AgentState::Idle)
        ));
    }

    #[test]
    fn test_full_workflow_with_tools() {
        let mut sm = StateMachine::new();

        // Idle -> Running
        sm.transition(StateEvent::Start);

        // Running -> AwaitingLLM
        sm.transition(StateEvent::LLMRequestSent);

        // AwaitingLLM -> ToolExecuting (tools needed)
        assert!(matches!(
            sm.transition(StateEvent::ToolsRequired),
            TransitionResult::Success(AgentState::ToolExecuting)
        ));

        // ToolExecuting -> Running
        assert!(matches!(
            sm.transition(StateEvent::ToolsCompleted),
            TransitionResult::Success(AgentState::Running)
        ));

        // Continue with another LLM request
        sm.transition(StateEvent::LLMRequestSent);
        sm.transition(StateEvent::LLMResponseReceived);
        assert!(matches!(
            sm.transition(StateEvent::StopConditionMet),
            TransitionResult::Success(AgentState::Completed)
        ));
    }

    #[test]
    fn test_pause_and_resume() {
        let mut sm = StateMachine::new();

        // Start and go to AwaitingLLM
        sm.transition(StateEvent::Start);
        sm.transition(StateEvent::LLMRequestSent);
        assert_eq!(sm.current_state(), AgentState::AwaitingLLM);

        // Pause
        assert!(matches!(
            sm.transition(StateEvent::UserInterrupt),
            TransitionResult::Success(AgentState::Paused)
        ));

        // Resume
        assert!(matches!(
            sm.transition(StateEvent::Start),
            TransitionResult::Success(AgentState::Running)
        ));
    }

    #[test]
    fn test_pause_from_tool_executing() {
        let mut sm = StateMachine::new();

        sm.transition(StateEvent::Start);
        sm.transition(StateEvent::LLMRequestSent);
        sm.transition(StateEvent::ToolsRequired);
        assert_eq!(sm.current_state(), AgentState::ToolExecuting);

        let result = sm.transition(StateEvent::UserInterrupt);
        assert!(matches!(
            result,
            TransitionResult::Success(AgentState::Paused)
        ));
    }
}
