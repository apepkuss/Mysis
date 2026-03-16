use std::fmt;

#[derive(Debug, Clone)]
pub enum ToolError {
    ExecutionFailed(String),
    InvalidParams(String),
    Unavailable(String),
}

impl fmt::Display for ToolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ExecutionFailed(msg) => write!(f, "tool execution failed: {msg}"),
            Self::InvalidParams(msg) => write!(f, "invalid tool params: {msg}"),
            Self::Unavailable(msg) => write!(f, "tool unavailable: {msg}"),
        }
    }
}

impl std::error::Error for ToolError {}

#[derive(Debug, Clone)]
pub enum AgentError {
    MaxIterationsReached(u32),
    LlmTimeout,
    LlmInvalidResponse(String),
    MqttError(String),
}

impl fmt::Display for AgentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MaxIterationsReached(n) => write!(f, "agent reached max iterations: {n}"),
            Self::LlmTimeout => write!(f, "LLM request timed out"),
            Self::LlmInvalidResponse(msg) => write!(f, "invalid LLM response: {msg}"),
            Self::MqttError(msg) => write!(f, "MQTT error: {msg}"),
        }
    }
}

impl std::error::Error for AgentError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_error_display() {
        let err = ToolError::ExecutionFailed("gpio timeout".into());
        assert_eq!(err.to_string(), "tool execution failed: gpio timeout");
    }

    #[test]
    fn agent_error_display() {
        let err = AgentError::MaxIterationsReached(5);
        assert_eq!(err.to_string(), "agent reached max iterations: 5");
    }
}
