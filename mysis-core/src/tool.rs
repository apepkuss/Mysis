use crate::error::ToolError;

/// 工具 trait — 所有硬件操作和扩展功能的统一接口。
/// 参考 ZeroClaw 的 trait-driven 设计。
pub trait Tool {
    /// 工具名称，如 "gpio_write"
    fn name(&self) -> &str;

    /// 工具描述，用于 LLM 理解工具用途
    fn description(&self) -> &str;

    /// JSON Schema 格式的参数定义（OpenAI 兼容）
    fn parameters_schema(&self) -> &str;

    /// 执行工具。同步阻塞，在 Agent 主任务线程上顺序执行。
    /// 输入：JSON 字符串格式的参数
    /// 输出：JSON 字符串格式的结果
    fn execute(&mut self, params: &str) -> Result<String, ToolError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ToolError;

    struct MockTool;

    impl Tool for MockTool {
        fn name(&self) -> &str {
            "mock_tool"
        }
        fn description(&self) -> &str {
            "A mock tool for testing"
        }
        fn parameters_schema(&self) -> &str {
            r#"{"type":"object","properties":{"value":{"type":"boolean"}},"required":["value"]}"#
        }
        fn execute(&mut self, params: &str) -> Result<String, ToolError> {
            let v: serde_json::Value = serde_json::from_str(params)
                .map_err(|e| ToolError::InvalidParams(e.to_string()))?;
            let value = v["value"]
                .as_bool()
                .ok_or_else(|| ToolError::InvalidParams("missing 'value'".into()))?;
            Ok(format!(r#"{{"success":true,"value":{value}}}"#))
        }
    }

    #[test]
    fn mock_tool_metadata() {
        let tool = MockTool;
        assert_eq!(tool.name(), "mock_tool");
        assert!(!tool.description().is_empty());
        assert!(tool.parameters_schema().contains("object"));
    }

    #[test]
    fn mock_tool_execute_ok() {
        let mut tool = MockTool;
        let result = tool.execute(r#"{"value": true}"#).unwrap();
        assert!(result.contains(r#""success":true"#));
    }

    #[test]
    fn mock_tool_execute_invalid_params() {
        let mut tool = MockTool;
        let result = tool.execute("not json");
        assert!(result.is_err());
    }
}
