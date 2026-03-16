use crate::error::AgentError;
use crate::protocol::*;
use crate::tool::Tool;

/// 传输层抽象 — 隔离 MQTT 实现细节，便于测试。
pub trait Transport {
    fn send_llm_request(&mut self, req: &LlmRequest) -> Result<(), String>;
    fn recv_llm_response(&mut self, timeout_secs: u32) -> Result<LlmResponse, String>;
}

/// Agent 配置
pub struct AgentConfig {
    pub device_id: String,
    pub chip_model: String,
    pub max_iterations: u32,
    pub llm_timeout_secs: u32,
    pub history_max_rounds: usize,
    pub system_prompt: String,
}

/// 运行一次 agent 循环：从用户输入到最终文本输出。
/// 可能涉及多轮 LLM 请求（工具调用 → 结果 → 再请求）。
pub fn run_agent_loop(
    config: &AgentConfig,
    transport: &mut dyn Transport,
    tools: &mut Vec<Box<dyn Tool>>,
    user_input: &str,
) -> Result<String, AgentError> {
    // 构建初始消息历史
    let mut messages = vec![
        Message {
            role: "system".into(),
            content: config.system_prompt.clone(),
            tool_calls: vec![],
            tool_call_id: None,
        },
        Message {
            role: "user".into(),
            content: user_input.into(),
            tool_calls: vec![],
            tool_call_id: None,
        },
    ];

    // 构建工具定义列表
    let tool_defs: Vec<ToolDefinition> = tools
        .iter()
        .map(|t| ToolDefinition {
            r#type: "function".into(),
            function: FunctionDefinition {
                name: t.name().into(),
                description: t.description().into(),
                parameters: serde_json::from_str(t.parameters_schema())
                    .unwrap_or(serde_json::Value::Object(Default::default())),
            },
        })
        .collect();

    let mut request_id: u32 = 0;

    for _iteration in 0..config.max_iterations {
        request_id += 1;
        let req = LlmRequest {
            id: format!("req-{request_id:03}"),
            messages: messages.clone(),
            tools: tool_defs.clone(),
            max_tokens: 256,
        };

        transport
            .send_llm_request(&req)
            .map_err(AgentError::MqttError)?;

        let resp = transport
            .recv_llm_response(config.llm_timeout_secs)
            .map_err(|_| AgentError::LlmTimeout)?;

        // 纯文本响应 — 任务完成
        if resp.tool_calls.is_empty() {
            return Ok(resp.content.unwrap_or_default());
        }

        // 有工具调用 — 顺序执行所有工具
        // 先追加 assistant 消息（包含 tool_calls，OpenAI 协议要求）
        messages.push(Message {
            role: "assistant".into(),
            content: resp.content.clone().unwrap_or_default(),
            tool_calls: resp.tool_calls.clone(),
            tool_call_id: None,
        });

        for tool_call in &resp.tool_calls {
            let result = execute_tool_call(tools, tool_call);
            messages.push(Message {
                role: "tool".into(),
                content: result,
                tool_calls: vec![],
                tool_call_id: Some(tool_call.id.clone()),
            });
        }

        // 裁剪历史长度
        trim_history(&mut messages, config.history_max_rounds);
    }

    Err(AgentError::MaxIterationsReached(config.max_iterations))
}

/// 在工具列表中查找并执行指定的工具调用
fn execute_tool_call(tools: &mut Vec<Box<dyn Tool>>, tool_call: &ToolCall) -> String {
    for tool in tools.iter_mut() {
        if tool.name() == tool_call.name {
            let params =
                serde_json::to_string(&tool_call.arguments).unwrap_or_else(|_| "{}".into());
            return match tool.execute(&params) {
                Ok(result) => result,
                Err(e) => format!(r#"{{"error":"{}"}}"#, e),
            };
        }
    }
    format!(r#"{{"error":"tool '{}' not found"}}"#, tool_call.name)
}

/// 保留 system prompt + 最近 N 轮对话，丢弃中间的旧消息
fn trim_history(messages: &mut Vec<Message>, max_rounds: usize) {
    // 每轮最多 3 条消息（user + assistant + tool），加上 1 条 system
    let max_messages = 1 + max_rounds * 3;
    if messages.len() > max_messages {
        // 保留第一条 system 消息和最后 max_messages - 1 条
        let system = messages[0].clone();
        let keep_start = messages.len() - (max_messages - 1);
        let mut trimmed = vec![system];
        trimmed.extend_from_slice(&messages[keep_start..]);
        *messages = trimmed;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ToolError;
    use crate::tool::Tool;

    struct MockGpioWrite;
    impl Tool for MockGpioWrite {
        fn name(&self) -> &str {
            "gpio_write_living_room_light"
        }
        fn description(&self) -> &str {
            "Write GPIO pin for living room light"
        }
        fn parameters_schema(&self) -> &str {
            r#"{"type":"object","properties":{"pin":{"type":"string"},"value":{"type":"boolean"}},"required":["pin","value"]}"#
        }
        fn execute(&mut self, params: &str) -> Result<String, ToolError> {
            let v: serde_json::Value = serde_json::from_str(params)
                .map_err(|e| ToolError::InvalidParams(e.to_string()))?;
            let pin = v["pin"].as_str().unwrap_or("unknown");
            let value = v["value"].as_bool().unwrap_or(false);
            Ok(format!(
                r#"{{"success":true,"pin":"{pin}","value":{value}}}"#
            ))
        }
    }

    /// Mock transport that returns a text-only response
    struct TextOnlyTransport;
    impl Transport for TextOnlyTransport {
        fn send_llm_request(&mut self, _req: &LlmRequest) -> Result<(), String> {
            Ok(())
        }
        fn recv_llm_response(&mut self, _timeout_secs: u32) -> Result<LlmResponse, String> {
            Ok(LlmResponse {
                id: "req-001".into(),
                content: Some("好的，灯已打开".into()),
                tool_calls: vec![],
                finish_reason: "stop".into(),
            })
        }
    }

    /// Mock transport that returns a tool_call then a text response
    struct ToolCallTransport {
        call_count: u32,
    }
    impl Transport for ToolCallTransport {
        fn send_llm_request(&mut self, _req: &LlmRequest) -> Result<(), String> {
            Ok(())
        }
        fn recv_llm_response(&mut self, _timeout_secs: u32) -> Result<LlmResponse, String> {
            self.call_count += 1;
            if self.call_count == 1 {
                Ok(LlmResponse {
                    id: "req-001".into(),
                    content: None,
                    tool_calls: vec![ToolCall {
                        id: "call_001".into(),
                        name: "gpio_write_living_room_light".into(),
                        arguments: serde_json::json!({"pin": "living_room_light", "value": true}),
                    }],
                    finish_reason: "tool_calls".into(),
                })
            } else {
                Ok(LlmResponse {
                    id: "req-001".into(),
                    content: Some("客厅灯已打开".into()),
                    tool_calls: vec![],
                    finish_reason: "stop".into(),
                })
            }
        }
    }

    /// Mock transport that always times out
    struct TimeoutTransport;
    impl Transport for TimeoutTransport {
        fn send_llm_request(&mut self, _req: &LlmRequest) -> Result<(), String> {
            Ok(())
        }
        fn recv_llm_response(&mut self, _timeout_secs: u32) -> Result<LlmResponse, String> {
            Err("timeout".into())
        }
    }

    fn make_config() -> AgentConfig {
        AgentConfig {
            device_id: "test-device".into(),
            chip_model: "esp32s3".into(),
            max_iterations: 5,
            llm_timeout_secs: 30,
            history_max_rounds: 10,
            system_prompt: "You are a test agent.".into(),
        }
    }

    #[test]
    fn agent_text_only_response() {
        let mut tools: Vec<Box<dyn Tool>> = vec![Box::new(MockGpioWrite)];
        let mut transport = TextOnlyTransport;
        let config = make_config();
        let result = run_agent_loop(&config, &mut transport, &mut tools, "hello");
        assert!(result.is_ok());
        let output = result.unwrap();
        assert!(output.contains("灯已打开"));
    }

    #[test]
    fn agent_tool_call_then_text() {
        let mut tools: Vec<Box<dyn Tool>> = vec![Box::new(MockGpioWrite)];
        let mut transport = ToolCallTransport { call_count: 0 };
        let config = make_config();
        let result = run_agent_loop(&config, &mut transport, &mut tools, "开灯");
        assert!(result.is_ok());
        let output = result.unwrap();
        assert!(output.contains("客厅灯已打开"));
    }

    #[test]
    fn agent_timeout_error() {
        let mut tools: Vec<Box<dyn Tool>> = vec![];
        let mut transport = TimeoutTransport;
        let config = make_config();
        let result = run_agent_loop(&config, &mut transport, &mut tools, "hello");
        assert!(result.is_err());
    }
}
